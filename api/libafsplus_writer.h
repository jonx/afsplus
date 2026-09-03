/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef LIBAFSPLUS_WRITER_H
#define LIBAFSPLUS_WRITER_H

/*
 * Bounded namespace-mutating slice of the independent portable C path.
 *
 * These APIs append one regular-file namespace operation to the preallocated
 * intent log and flush it. Final-link delete and replacement require the
 * ADR-066 orphan feature; replay moves the victim into bounded cleanup state.
 */

#include "libafsplus_reader.h"

#include <stddef.h>
#include <stdint.h>

#define AFSPW_ABI_VERSION 1u
#define AFSPW_SCRATCH_SIZE AFSPR_INTENT_SCRATCH_SIZE
#define AFSPW_MAX_CACHED_BLOCKS 16u
#define AFSPW_RECOMMENDED_SCRATCH_SIZE                                    \
    (AFSPW_SCRATCH_SIZE + 12u * AFSPR_MIN_SCRATCH_SIZE)
#define AFSPW_CAP_RENAME_FILE_NO_REPLACE (UINT64_C(1) << 0)
#define AFSPW_CAP_DELETE_FILE (UINT64_C(1) << 1)
#define AFSPW_CAP_RENAME_FILE_REPLACE (UINT64_C(1) << 2)
#define AFSPW_CAP_CREATE_EMPTY_FILE (UINT64_C(1) << 3)

#ifdef __cplusplus
extern "C" {
#endif

enum afspw_status {
    AFSPW_ERR_LOG_FULL = -100,
    AFSPW_ERR_DESTINATION_EXISTS = -101,
    AFSPW_ERR_WRITE_UNCERTAIN = -102,
    AFSPW_ERR_DURABILITY_UNCERTAIN = -103,
    AFSPW_ERR_WRITE_FEATURE = -104,
    AFSPW_ERR_OBJECT_ID_EXHAUSTED = -105
};

enum afspw_stage {
    AFSPW_STAGE_NONE = 0,
    AFSPW_STAGE_ARGUMENTS = 1,
    AFSPW_STAGE_PROBE = 2,
    AFSPW_STAGE_INTENT_SCAN = 3,
    AFSPW_STAGE_SOURCE_LOOKUP = 4,
    AFSPW_STAGE_TARGET_LOOKUP = 5,
    AFSPW_STAGE_ENCODE = 6,
    AFSPW_STAGE_RECORD_WRITE = 7,
    AFSPW_STAGE_FLUSH = 8,
    AFSPW_STAGE_COMPLETE = 9,
    AFSPW_STAGE_CREATE_LOOKUP = 10
};

/*
 * The caller owns serialization and supplies complete logical-block I/O.
 * write_blocks and flush return zero only when their operation succeeded.
 * After either callback reports failure, media state is uncertain and the
 * caller must discard cached state and probe again before another mutation.
 */
struct afspw_block_ops {
    uint32_t abi_version;
    uint32_t struct_size;
    void *ctx;
    int (*read_blocks)(void *ctx, uint64_t first_block, uint32_t count,
                       void *dst);
    int (*write_blocks)(void *ctx, uint64_t first_block, uint32_t count,
                        const void *src);
    int (*flush)(void *ctx);
    uint64_t block_count;
    uint32_t block_size;
};

/*
 * AFSPW_SCRATCH_SIZE is the hard minimum and keeps the low-memory path at
 * 8 KiB. Every additional complete 4 KiB block becomes one read-cache entry
 * for the duration of a writer call, up to AFSPW_MAX_CACHED_BLOCKS. The
 * recommended size supplies twelve entries, enough to retain the hot metadata
 * and log working set in the qualification fixture. Cache contents never
 * survive a call.
 */

/*
 * The result records durable-log coordinates and is shared by all namespace
 * calls. Its historical tag is retained to keep ABI 1 source-compatible.
 */
struct afspw_rename_result {
    uint32_t abi_version;
    uint32_t prior_records;
    uint32_t sequence;
    uint32_t log_slot;
    uint64_t base_generation;
    uint64_t log_block;
};

struct afspw_create_result {
    uint32_t abi_version;
    uint32_t prior_records;
    uint32_t sequence;
    uint32_t log_slot;
    uint64_t base_generation;
    uint64_t log_block;
    uint64_t object_id;
};

struct afspw_diagnostic {
    uint32_t abi_version;
    int32_t status;
    uint32_t stage;
    int32_t reader_status;
    uint64_t block;
    uint32_t sequence;
    uint32_t reserved;
};

uint64_t afspw_capabilities(void);

/*
 * Durably create an empty regular file. The identifier is the monotone
 * checkpoint-plus-log allocator watermark returned in result.object_id. The
 * preflight also proves that the highest committed object remains below that
 * watermark. This operation allocates no data block; reference replay
 * allocates only the metadata needed to materialize the record into a
 * checkpoint.
 */
int afspw_create_empty_file(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t parent_id, const void *name, size_t name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_create_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size);

/*
 * Durably rename one regular-file directory entry without replacement.
 *
 * The function freshly probes and semantically scans the durable intent view,
 * verifies the source and absent destination, writes exactly one log block,
 * then invokes one flush. source_name and target_name must remain immutable
 * and must not overlap scratch for the duration of the call. The function
 * owns no memory, retains no pointer and performs no checkpoint mutation.
 *
 * Common argument/read/format failures use AFSPR_ERR_* values. Writer-only
 * failures use AFSPW_ERR_* above. A successful return is AFSPR_OK.
 */
int afspw_rename_file_no_replace(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t source_parent_id, const void *source_name,
    size_t source_name_len, uint64_t target_parent_id,
    const void *target_name, size_t target_name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size);

/*
 * Durably remove one regular-file directory entry. A final link becomes a
 * persistent orphan during Rust/reference replay and is reclaimed later in
 * bounded steps; a non-final hard link is decremented normally.
 */
int afspw_delete_file(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t parent_id, const void *name, size_t name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size);

/*
 * Durably rename one regular file, atomically replacing a regular-file target
 * when present. A final replaced target enters bounded orphan cleanup state.
 */
int afspw_rename_file_replace(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t source_parent_id, const void *source_name,
    size_t source_name_len, uint64_t target_parent_id,
    const void *target_name, size_t target_name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size);

const char *afspw_status_string(int status);
const char *afspw_stage_string(uint32_t stage);

#ifdef __cplusplus
}
#endif

#endif
