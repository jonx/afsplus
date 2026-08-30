#ifndef AFSPLUS_AROS_H
#define AFSPLUS_AROS_H

/* Stable, allocation-free C boundary for the native AROS DOS handler.
 *
 * All functions return 0 on success or an AROS ERROR_* value on failure.
 * The caller owns every buffer and the block-device context. The context and
 * callbacks must remain valid until afsplus_aros_unmount() returns. One AFS+
 * instance is single-task: do not issue concurrent calls for the same pointer.
 */

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AFSPLUS_AROS_ABI_VERSION UINT32_C(1)

#define AFSPLUS_AROS_MOUNT_READ_WRITE UINT32_C(0)
#define AFSPLUS_AROS_MOUNT_READ_ONLY UINT32_C(1)
#define AFSPLUS_AROS_MOUNT_NO_CHANGES UINT32_C(2)
#define AFSPLUS_AROS_MOUNT_RECOVERY UINT32_C(3)

#define AFSPLUS_AROS_ENCODING_UTF8 UINT32_C(0)
#define AFSPLUS_AROS_ENCODING_LATIN1 UINT32_C(1)

#define AFSPLUS_AROS_LOCK_SHARED UINT32_C(0)
#define AFSPLUS_AROS_LOCK_EXCLUSIVE UINT32_C(1)

#define AFSPLUS_AROS_OPEN_OLD_FILE UINT32_C(0)
#define AFSPLUS_AROS_OPEN_NEW_FILE UINT32_C(1)
#define AFSPLUS_AROS_OPEN_READ_WRITE UINT32_C(2)

#define AFSPLUS_AROS_SEEK_BEGINNING UINT32_C(0)
#define AFSPLUS_AROS_SEEK_CURRENT UINT32_C(1)
#define AFSPLUS_AROS_SEEK_END UINT32_C(2)

struct AfsplusAros;

typedef int32_t (*AfsplusArosReadBlock)(void *context, uint64_t lba,
    uint8_t *destination, uint32_t length);
typedef int32_t (*AfsplusArosWriteBlock)(void *context, uint64_t lba,
    const uint8_t *source, uint32_t length);
typedef int32_t (*AfsplusArosFlush)(void *context);

struct AfsplusArosDevice {
    uint32_t abi_version;
    uint32_t struct_size;
    void *context;
    uint32_t block_size;
    uint32_t reserved;
    uint64_t total_blocks;
    AfsplusArosReadBlock read_block;
    AfsplusArosWriteBlock write_block;
    AfsplusArosFlush flush;
};

struct AfsplusArosMountConfig {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t mount_mode;
    uint32_t name_encoding;
    const uint8_t *volume_name;
    uint32_t volume_name_length;
    uint32_t max_file_handles;
    uint32_t max_locks;
    uint32_t max_file_info_name_bytes;
    uint32_t reserved;
};

struct AfsplusArosFileInfo {
    uint64_t disk_key;
    uint64_t size;
    uint64_t blocks;
    int64_t modified_seconds;
    uint64_t object_id;
    int32_t directory_entry_type;
    int32_t entry_type;
    uint32_t protection;
    uint32_t modified_nanoseconds;
    uint32_t name_length;
    uint32_t reserved;
};

struct AfsplusArosDiskInfo {
    uint64_t total_blocks;
    uint64_t used_blocks;
    uint32_t bytes_per_block;
    int32_t disk_type;
    uint32_t write_protected;
    uint32_t in_use;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(struct AfsplusArosFileInfo) == 64,
    "AfsplusArosFileInfo ABI drift");
_Static_assert(sizeof(struct AfsplusArosDiskInfo) == 32,
    "AfsplusArosDiskInfo ABI drift");
#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(struct AfsplusArosDevice) == 56,
    "AfsplusArosDevice 64-bit ABI drift");
_Static_assert(sizeof(struct AfsplusArosMountConfig) == 48,
    "AfsplusArosMountConfig 64-bit ABI drift");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(struct AfsplusArosDevice) == 40,
    "AfsplusArosDevice 32-bit ABI drift");
_Static_assert(sizeof(struct AfsplusArosMountConfig) == 40,
    "AfsplusArosMountConfig 32-bit ABI drift");
#endif
#endif

int32_t afsplus_aros_mount(const struct AfsplusArosDevice *device,
    const struct AfsplusArosMountConfig *config,
    struct AfsplusAros **output);

/* Consumes filesystem exactly once, including when the final flush fails. */
int32_t afsplus_aros_unmount(struct AfsplusAros *filesystem);

/* Lock value 0 is the DOS null lock and denotes the root where accepted. */
int32_t afsplus_aros_locate(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint32_t access, uint64_t *output_lock);
int32_t afsplus_aros_duplicate_lock(struct AfsplusAros *filesystem,
    uint64_t lock, uint64_t *output_lock);
int32_t afsplus_aros_parent_lock(struct AfsplusAros *filesystem,
    uint64_t lock, uint64_t *output_lock);
int32_t afsplus_aros_same_lock(struct AfsplusAros *filesystem,
    uint64_t first_lock, uint64_t second_lock, uint32_t *output_same);
int32_t afsplus_aros_free_lock(struct AfsplusAros *filesystem, uint64_t lock);

int32_t afsplus_aros_open(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint32_t mode, int64_t now_seconds, uint32_t now_nanoseconds,
    uint64_t *output_file);
int32_t afsplus_aros_parent_of_file(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_lock);
int32_t afsplus_aros_close(struct AfsplusAros *filesystem, uint64_t file);

/* A single call is bounded by uint32_t, matching ACTION_READ/ACTION_WRITE. */
int32_t afsplus_aros_read(struct AfsplusAros *filesystem, uint64_t file,
    uint8_t *destination, uint32_t length, uint32_t *output_count);
int32_t afsplus_aros_write(struct AfsplusAros *filesystem, uint64_t file,
    const uint8_t *source, uint32_t length, int64_t now_seconds,
    uint32_t now_nanoseconds, uint32_t *output_count);

int32_t afsplus_aros_seek(struct AfsplusAros *filesystem, uint64_t file,
    int64_t offset, uint32_t mode, uint64_t *output_old_position);
int32_t afsplus_aros_file_position(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_position);
int32_t afsplus_aros_file_size(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_size);
int32_t afsplus_aros_set_file_size(struct AfsplusAros *filesystem,
    uint64_t file, int64_t offset, uint32_t mode, int64_t now_seconds,
    uint32_t now_nanoseconds, uint64_t *output_size);
int32_t afsplus_aros_fsync(struct AfsplusAros *filesystem, uint64_t file);
int32_t afsplus_aros_flush(struct AfsplusAros *filesystem);

int32_t afsplus_aros_create_directory(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    int64_t now_seconds, uint32_t now_nanoseconds, uint64_t *output_lock);
int32_t afsplus_aros_delete_object(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    int64_t now_seconds, uint32_t now_nanoseconds);
int32_t afsplus_aros_rename(struct AfsplusAros *filesystem,
    uint64_t source_base_lock, const uint8_t *source_name,
    uint32_t source_name_length, uint64_t target_base_lock,
    const uint8_t *target_name, uint32_t target_name_length,
    int64_t now_seconds, uint32_t now_nanoseconds);
int32_t afsplus_aros_make_hard_link(struct AfsplusAros *filesystem,
    uint64_t target_base_lock, const uint8_t *target_name,
    uint32_t target_name_length, uint64_t source_lock,
    int64_t now_seconds, uint32_t now_nanoseconds);

/* Pass at least max_file_info_name_bytes bytes. On success name_length bytes
 * have been written to name without a trailing NUL or BCPL length byte. */
int32_t afsplus_aros_examine_lock(struct AfsplusAros *filesystem,
    uint64_t lock, struct AfsplusArosFileInfo *output,
    uint8_t *name, uint32_t name_capacity);
int32_t afsplus_aros_examine_file(struct AfsplusAros *filesystem,
    uint64_t file, struct AfsplusArosFileInfo *output,
    uint8_t *name, uint32_t name_capacity);
int32_t afsplus_aros_examine_next(struct AfsplusAros *filesystem,
    uint64_t lock, struct AfsplusArosFileInfo *output,
    uint8_t *name, uint32_t name_capacity);
int32_t afsplus_aros_rewind_directory(struct AfsplusAros *filesystem,
    uint64_t lock);
int32_t afsplus_aros_disk_info(struct AfsplusAros *filesystem,
    struct AfsplusArosDiskInfo *output);

#ifdef __cplusplus
}
#endif

#endif
