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
#define AFSPR_TREE_SCRATCH_SIZE 6144u
#define AFSPR_NO_BLOCK UINT64_MAX
#define AFSPR_NO_CHECKPOINT_SLOT (-1)

#define AFSPR_CAP_PROBE (UINT64_C(1) << 0)
#define AFSPR_CAP_OBJECT_LOOKUP (UINT64_C(1) << 1)
#define AFSPR_CAP_DIRECTORY_ORDINAL (UINT64_C(1) << 2)
#define AFSPR_CAP_FILE_READ (UINT64_C(1) << 3)

#define AFSPR_OBJECT_FLAG_EXTENT_TREE (UINT16_C(1) << 0)

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
    AFSPR_ERR_AMBIGUOUS_CHECKPOINT = -8,
    AFSPR_ERR_NOT_FOUND = -9,
    AFSPR_ERR_NOT_FILE = -10,
    AFSPR_ERR_NOT_DIRECTORY = -11,
    AFSPR_ERR_BUFFER_TOO_SMALL = -12
};

enum afspr_probe_stage {
    AFSPR_STAGE_NONE = 0,
    AFSPR_STAGE_ARGUMENTS = 1,
    AFSPR_STAGE_IDENTIFICATION_READ = 2,
    AFSPR_STAGE_IDENTIFICATION_DECODE = 3,
    AFSPR_STAGE_CHECKPOINT_READ = 4,
    AFSPR_STAGE_CHECKPOINT_DECODE = 5,
    AFSPR_STAGE_CHECKPOINT_SELECTION = 6,
    AFSPR_STAGE_COMPLETE = 7,
    AFSPR_STAGE_TREE_READ = 8,
    AFSPR_STAGE_TREE_DECODE = 9,
    AFSPR_STAGE_TREE_TRAVERSAL = 10,
    AFSPR_STAGE_OBJECT_READ = 11,
    AFSPR_STAGE_OBJECT_DECODE = 12,
    AFSPR_STAGE_DIRECTORY_DECODE = 13,
    AFSPR_STAGE_EXTENT_DECODE = 14,
    AFSPR_STAGE_DATA_READ = 15
};

enum afspr_object_type {
    AFSPR_OBJECT_FILE = 1,
    AFSPR_OBJECT_DIRECTORY = 2,
    AFSPR_OBJECT_SYMLINK = 3,
    AFSPR_OBJECT_INTERNAL = 4
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

struct afspr_timespec {
    int64_t seconds;
    uint32_t nanoseconds;
    uint32_t reserved;
};

struct afspr_object {
    uint32_t abi_version;
    uint32_t type;
    uint64_t object_id;
    uint16_t flags;
    uint16_t reserved16;
    uint32_t link_count;
    uint64_t size_bytes;
    uint64_t allocated_bytes;
    struct afspr_timespec created;
    struct afspr_timespec modified;
    struct afspr_timespec changed;
    uint32_t protection;
    uint32_t reserved32;
    uint64_t content_generation;
    uint64_t data_root;
    uint64_t data_blocks;
};

/*
 * Initial placeholder: no function writes this type. Its layout is retained
 * for source compatibility; new integrations use afspr_directory_entry.
 */
struct afspr_entry {
    uint64_t object_id;
    uint64_t parent_id;
    uint64_t size;
    uint32_t type;
    const uint8_t *name;
    uint16_t name_len;
};

struct afspr_directory_entry {
    uint32_t abi_version;
    uint32_t type_hint;
    uint64_t object_id;
    uint64_t parent_id;
    const uint8_t *name;
    size_t name_len;
};

int afspr_probe(const struct afspr_block_ops *ops,
                const struct afspr_scratch *scratch,
                struct afspr_probe_result *result, size_t result_size);

int afspr_probe_detailed(const struct afspr_block_ops *ops,
                         const struct afspr_scratch *scratch,
                         struct afspr_probe_result *result, size_t result_size,
                         struct afspr_diagnostic *diagnostic,
                         size_t diagnostic_size);

/* Probe and operation outputs are complete only when the call returns OK. */

uint64_t afspr_capabilities(void);

int afspr_lookup_object(const struct afspr_block_ops *ops,
                        const struct afspr_scratch *scratch,
                        const struct afspr_probe_result *volume,
                        uint64_t object_id, struct afspr_object *object,
                        size_t object_size,
                        struct afspr_diagnostic *diagnostic,
                        size_t diagnostic_size);

/*
 * Reads one directory entry by binary-tree ordinal. name points to the
 * caller's name_buffer on success and remains valid until that buffer changes.
 * total_entries is optional. Directory traversal needs
 * AFSPR_TREE_SCRATCH_SIZE bytes so parent key bounds survive the next read.
 * With insufficient name capacity, entry.name is NULL and entry.name_len is
 * the required size even though the call returns BUFFER_TOO_SMALL.
 */
int afspr_directory_entry_at(const struct afspr_block_ops *ops,
                             const struct afspr_scratch *scratch,
                             const struct afspr_probe_result *volume,
                             const struct afspr_object *directory,
                             uint64_t ordinal, void *name_buffer,
                             size_t name_capacity,
                             struct afspr_directory_entry *entry,
                             size_t entry_size, uint64_t *total_entries,
                             struct afspr_diagnostic *diagnostic,
                             size_t diagnostic_size);

/*
 * Reads at most destination_size bytes from the selected checkpoint-root
 * view and reports EOF as zero. The data and scratch buffers must not overlap.
 */
int afspr_read_file(const struct afspr_block_ops *ops,
                    const struct afspr_scratch *scratch,
                    const struct afspr_probe_result *volume,
                    const struct afspr_object *file, uint64_t offset,
                    void *destination, size_t destination_size,
                    size_t *bytes_read,
                    struct afspr_diagnostic *diagnostic,
                    size_t diagnostic_size);

const char *afspr_status_string(int status);
const char *afspr_probe_stage_string(uint32_t stage);

#ifdef __cplusplus
}
#endif

#endif
