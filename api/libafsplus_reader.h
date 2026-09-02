/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef LIBAFSPLUS_READER_H
#define LIBAFSPLUS_READER_H

/*
 * Minimal read-only profile.
 *
 * The implementation should be usable without threads and should permit
 * caller-supplied scratch buffers for constrained systems.
 */

#include <stdint.h>
#include <stddef.h>

#define AFSPR_ABI_VERSION 1u
#define AFSPR_LABEL_CAPACITY 65u
#define AFSPR_MIN_SCRATCH_SIZE 4096u
#define AFSPR_NO_BLOCK UINT64_MAX
#define AFSPR_NO_CHECKPOINT_SLOT (-1)

#ifdef __cplusplus
extern "C" {
#endif

enum afspr_status {
    AFSPR_NOT_CHECKED = 1,
    AFSPR_OK = 0,
    AFSPR_ERR_INVALID_ARGUMENT = -1,
    AFSPR_ERR_ABI = -2,
    AFSPR_ERR_IO = -3,
    AFSPR_ERR_SCRATCH_TOO_SMALL = -4,
    AFSPR_ERR_UNSUPPORTED = -5,
    AFSPR_ERR_CORRUPT = -6,
    AFSPR_ERR_NO_CHECKPOINT = -7,
    AFSPR_ERR_AMBIGUOUS_CHECKPOINT = -8
};

enum afspr_probe_stage {
    AFSPR_STAGE_NONE = 0,
    AFSPR_STAGE_ARGUMENTS = 1,
    AFSPR_STAGE_IDENTIFICATION_READ = 2,
    AFSPR_STAGE_IDENTIFICATION_DECODE = 3,
    AFSPR_STAGE_CHECKPOINT_READ = 4,
    AFSPR_STAGE_CHECKPOINT_DECODE = 5,
    AFSPR_STAGE_CHECKPOINT_SELECTION = 6,
    AFSPR_STAGE_COMPLETE = 7
};

/*
 * The reader owns no storage and assumes no host API. The callback reads
 * exactly count logical blocks into dst and returns zero on success.
 */
struct afspr_block_ops {
    uint32_t abi_version;
    uint32_t struct_size;
    void *ctx;
    int (*read_blocks)(void *ctx, uint64_t first_block, uint32_t count,
                       void *dst);
    uint64_t block_count;
    uint32_t block_size;
};

struct afspr_scratch {
    void *buffer;
    size_t size;
};

/*
 * Result of bounds-first identification and structural checkpoint selection.
 * This does not replay the intent log or validate reachable metadata roots.
 */
struct afspr_probe_result {
    uint32_t abi_version;
    uint32_t identification_version;
    uint8_t uuid[16];
    uint8_t block_shift;
    uint8_t checksum_algorithm;
    uint16_t log_slots;
    uint32_t region_size;
    uint64_t total_blocks;
    uint64_t metadata_start;
    uint64_t compat_features;
    uint64_t ro_compat_features;
    uint64_t incompat_features;
    uint8_t name_key_algorithm;
    uint8_t unicode_version[3];
    char label[AFSPR_LABEL_CAPACITY];

    uint8_t selected_checkpoint;
    uint8_t valid_checkpoint_mask;
    uint8_t reserved[6];
    uint64_t generation;
    uint64_t object_map_block;
    uint64_t allocation_root_block;
    uint64_t reclaim_root_block;
    uint64_t next_object_id;
    uint64_t committed_tx_id;
    uint64_t free_blocks_total;
    uint64_t shared_extent_root_block;
};

/*
 * Optional machine-readable context for integration and recovery tools.
 * block is AFSPR_NO_BLOCK and checkpoint_slot is
 * AFSPR_NO_CHECKPOINT_SLOT when that coordinate does not apply. The per-slot
 * statuses describe structural decoding; a later selection error may leave
 * both as AFSPR_OK.
 */
struct afspr_diagnostic {
    uint32_t abi_version;
    int32_t status;
    uint32_t stage;
    int32_t checkpoint_slot;
    uint64_t block;
    int32_t checkpoint_status[2];
};

struct afspr_entry {
    uint64_t object_id;
    uint64_t parent_id;
    uint64_t size;
    uint32_t type;
    const uint8_t *name;
    uint16_t name_len;
};

int afspr_probe(const struct afspr_block_ops *ops,
                const struct afspr_scratch *scratch,
                struct afspr_probe_result *result, size_t result_size);

int afspr_probe_detailed(const struct afspr_block_ops *ops,
                         const struct afspr_scratch *scratch,
                         struct afspr_probe_result *result, size_t result_size,
                         struct afspr_diagnostic *diagnostic,
                         size_t diagnostic_size);

/* result is complete only when a probe returns AFSPR_OK. */

const char *afspr_status_string(int status);
const char *afspr_probe_stage_string(uint32_t stage);

#ifdef __cplusplus
}
#endif

#endif
