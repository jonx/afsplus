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

#include "debug_observability.h"

#ifdef __cplusplus
extern "C" {
#endif

#define AFSPLUS_AROS_ABI_VERSION UINT32_C(1)

/* Additive growth inside ABI version 1. The revision counts the entry-point
 * groups a library carries; every revision keeps all earlier functions and
 * structure layouts. A caller built against a newer header asks
 * afsplus_aros_interface() before it calls a function of a later group and
 * treats a missing group as ERROR_ACTION_NOT_KNOWN. */
#define AFSPLUS_AROS_INTERFACE_REVISION UINT32_C(17)

#define AFSPLUS_AROS_GROUP_BASE UINT64_C(0x1)
#define AFSPLUS_AROS_GROUP_INTERFACE_QUERY UINT64_C(0x2)
#define AFSPLUS_AROS_GROUP_DOS_METADATA UINT64_C(0x4)
#define AFSPLUS_AROS_GROUP_SOFT_LINKS UINT64_C(0x8)
#define AFSPLUS_AROS_GROUP_API_V2 UINT64_C(0x10)
#define AFSPLUS_AROS_GROUP_NOTIFY UINT64_C(0x20)
#define AFSPLUS_AROS_GROUP_OBSERVE UINT64_C(0x40)
#define AFSPLUS_AROS_GROUP_MANAGE UINT64_C(0x80)
#define AFSPLUS_AROS_GROUP_COUNTERS UINT64_C(0x100)
#define AFSPLUS_AROS_GROUP_DOS_HANDLES UINT64_C(0x200)
#define AFSPLUS_AROS_GROUP_DOS_RECORDS UINT64_C(0x400)
#define AFSPLUS_AROS_GROUP_OBJECT_IDS UINT64_C(0x800)
#define AFSPLUS_AROS_GROUP_EXTENT_MAP UINT64_C(0x1000)
#define AFSPLUS_AROS_GROUP_VOLUME_LABEL UINT64_C(0x2000)
#define AFSPLUS_AROS_GROUP_DOS_COMMENT UINT64_C(0x4000)
#define AFSPLUS_AROS_GROUP_ATTRIBUTES UINT64_C(0x8000)
#define AFSPLUS_AROS_GROUP_CACHE UINT64_C(0x10000)
#define AFSPLUS_AROS_GROUP_COMMIT UINT64_C(0x20000)

/* AfsplusArosExtent.flags. */
#define AFSPLUS_AROS_EXTENT_UNWRITTEN UINT32_C(0x1)

/* AfsplusArosStat.kind and AfsplusArosDirEntry.kind. */
#define AFSPLUS_AROS_KIND_FILE UINT32_C(1)
#define AFSPLUS_AROS_KIND_DIRECTORY UINT32_C(2)
#define AFSPLUS_AROS_KIND_SYMLINK UINT32_C(3)

/* Largest packed directory record: header, 255 name bytes, padding. A
 * dir_read buffer holds at least one of these. */
#define AFSPLUS_AROS_DIR_RECORD_MAX UINT32_C(280)

/* AfsplusArosHealth.flags. Disk-full is counted and is not a degraded state. */
#define AFSPLUS_AROS_HEALTH_DEVICE_ERROR UINT32_C(0x1)
#define AFSPLUS_AROS_HEALTH_CORRUPTION UINT32_C(0x2)
#define AFSPLUS_AROS_HEALTH_REPLAY_PENDING UINT32_C(0x4)
#define AFSPLUS_AROS_HEALTH_INTERNAL_FAULT UINT32_C(0x8)

/* AfsplusArosHealthEvent.kind. Numbers are appended and never reused. The
 * last three describe the volume without failing a call: they carry
 * dos_error 0 and set no flag. CHECKPOINT_FALLBACK says the mount could not
 * use the newer checkpoint of the A/B pair and reads the older one, so the
 * last commit before this mount is not in what the volume shows.
 * RECLAIM_BACKLOG_HIGH is raised once each time retired space waiting to
 * come back crosses a sixteenth of the volume (at least 4096 blocks), and
 * again only after it has fallen below half of that. REGION_FREECOUNT_
 * MISMATCH says the mounted checkpoint's free-block total disagrees with
 * the sum over its own allocation-root region records. */
#define AFSPLUS_AROS_HEALTH_EVENT_DEVICE_ERROR UINT32_C(1)
#define AFSPLUS_AROS_HEALTH_EVENT_CORRUPTION UINT32_C(2)
#define AFSPLUS_AROS_HEALTH_EVENT_NO_SPACE UINT32_C(3)
#define AFSPLUS_AROS_HEALTH_EVENT_INTERNAL_FAULT UINT32_C(4)
#define AFSPLUS_AROS_HEALTH_EVENT_CHECKPOINT_FALLBACK UINT32_C(5)
#define AFSPLUS_AROS_HEALTH_EVENT_RECLAIM_BACKLOG_HIGH UINT32_C(6)
#define AFSPLUS_AROS_HEALTH_EVENT_REGION_FREECOUNT_MISMATCH UINT32_C(7)

/* afsplus_aros_advise effect. */
#define AFSPLUS_AROS_ADVICE_NO_EFFECT UINT32_C(0)

#define AFSPLUS_AROS_MOUNT_READ_WRITE UINT32_C(0)
#define AFSPLUS_AROS_MOUNT_READ_ONLY UINT32_C(1)
#define AFSPLUS_AROS_MOUNT_NO_CHANGES UINT32_C(2)
#define AFSPLUS_AROS_MOUNT_RECOVERY UINT32_C(3)

/* AfsplusArosMountConfig.flags. Undefined bits are refused at mount.
 *
 * SECURITY_DOWNGRADE is the explicit request to let a protection write
 * replace security metadata the container does not hold, where preserving it
 * is impossible; zero refuses such a write with ERROR_WRITE_PROTECTED.
 *
 * STRICT_SECURITY_PROJECTION refuses a protection write on an object that
 * carries an on-disk security descriptor; zero takes the handler default,
 * which applies the write, keeps every descriptor byte and marks the
 * projection as diverged. */
#define AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE UINT32_C(1)
#define AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION UINT32_C(2)

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
    /* Zero length names the volume after its committed label, which is
     * what a handler publishes; a name overrides it for this mount. */
    const uint8_t *volume_name;
    uint32_t volume_name_length;
    uint32_t max_file_handles;
    uint32_t max_locks;
    uint32_t max_file_info_name_bytes;
    uint32_t flags;
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

/* Query structures use one growth rule. The caller stores the size of its
 * own structure in struct_size. The library fills the fields that fit, never
 * writes past that size, and stores the number of bytes it filled. A size
 * below the layout the structure was first published with is refused with
 * ERROR_BAD_NUMBER (115). A structure may grow; these floors never move, so
 * a client built against the first layout works with every later library. */
#define AFSPLUS_AROS_INTERFACE_FIRST_LAYOUT 24
#define AFSPLUS_AROS_CAPABILITIES_FIRST_LAYOUT 64
#define AFSPLUS_AROS_HEALTH_FIRST_LAYOUT 112
#define AFSPLUS_AROS_TRACE_COUNTERS_FIRST_LAYOUT 40
#define AFSPLUS_AROS_COUNTERS_FIRST_LAYOUT 72
#define AFSPLUS_AROS_STAT_FIRST_LAYOUT 88
struct AfsplusArosInterface {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t interface_revision;
    uint32_t reserved;
    uint64_t groups;
};

/* capabilities uses the FSV2_CAP_* identities of filesystem_v2.h. */
struct AfsplusArosCapabilities {
    uint32_t struct_size;
    uint32_t mount_mode;
    uint64_t capabilities;
    uint32_t block_size;
    uint32_t max_name_bytes;
    uint32_t case_sensitive;
    uint8_t unicode_version_major;
    uint8_t unicode_version_minor;
    uint8_t unicode_version_patch;
    uint8_t reserved0;
    uint32_t pending_intent_records;
    uint32_t reserved1;
    uint64_t total_blocks;
    uint64_t free_blocks;
    uint64_t available_blocks;
};

/* Sized query structure; see AfsplusArosInterface for the growth rule. */
struct AfsplusArosHealth {
    uint32_t struct_size;
    uint32_t mount_mode;
    uint32_t flags;
    uint32_t pending_intent_records;
    uint64_t generation;
    uint64_t pending_orphans;
    uint64_t total_blocks;
    uint64_t free_blocks;
    uint64_t available_blocks;
    uint64_t device_errors;
    uint64_t corruption_errors;
    uint64_t no_space_errors;
    uint64_t internal_faults;
    uint64_t events_recorded;
    uint64_t events_dropped;
    /* IoErr() of the most recent event that carries one; an event that
     * failed no call leaves it as it was. */
    int32_t last_error;
    uint32_t reserved;
    /* Appended after the first published layout; a caller that declares
     * AFSPLUS_AROS_HEALTH_FIRST_LAYOUT receives exactly that much. */
    uint64_t checkpoint_fallbacks;
    uint64_t reclaim_backlog_highs;
    uint64_t free_count_mismatches;
};

/* sequence counts every recorded event since mount, so a gap is loss. */
struct AfsplusArosHealthEvent {
    uint64_t sequence;
    uint32_t kind;
    int32_t dos_error;
};

/* Sized query structure. delivered and missed count sink calls; filtered
 * counts events outside the category mask; dropped counts ring overwrites. */
struct AfsplusArosTraceCounters {
    uint32_t struct_size;
    uint32_t attached;
    uint64_t delivered;
    uint64_t missed;
    uint64_t filtered;
    uint64_t dropped;
};

/* Sized query structure. calls counts completed entries on the mounted
 * instance before this one; device_* counts block callbacks. The benchmark
 * contract reads these instead of inferring traffic from elapsed time.
 * heap_bytes is what the library's Rust allocations hold now, heap_peak_bytes
 * the most they held at once since the library started; both are the
 * library's, shared by the instances one copy of it serves, and leave out the
 * handler's own allocations and allocator overhead. cache_blocks is the
 * read cache in force (group CACHE), cache_hits and cache_misses the reads it
 * served and the reads that went to the device, since the mount. */
struct AfsplusArosCounters {
    uint32_t struct_size;
    uint32_t reserved;
    uint64_t calls;
    uint64_t failed_calls;
    uint64_t device_reads;
    uint64_t device_writes;
    uint64_t device_flushes;
    uint64_t device_read_bytes;
    uint64_t device_written_bytes;
    uint64_t device_failures;
    uint64_t heap_bytes;
    uint64_t heap_peak_bytes;
    uint64_t cache_blocks;
    uint64_t cache_hits;
    uint64_t cache_misses;
};

/* Sized query structure; see AfsplusArosInterface for the growth rule. */
struct AfsplusArosStat {
    uint32_t struct_size;
    uint32_t kind;
    uint64_t object_id;
    uint64_t size;
    uint64_t allocated_size;
    uint64_t protection;
    uint32_t links;
    uint32_t reserved0;
    int64_t created_seconds;
    int64_t modified_seconds;
    int64_t changed_seconds;
    uint32_t created_nanoseconds;
    uint32_t modified_nanoseconds;
    uint32_t changed_nanoseconds;
    uint32_t reserved1;
};

/* One mapped piece of a file's byte space; gaps are holes and read as zeros.
 * UNWRITTEN is reserved storage: it reads as zeros and a write into it
 * allocates nothing. */
struct AfsplusArosExtent {
    uint64_t offset;
    uint64_t length;
    uint32_t flags;
    uint32_t reserved;
};

/* One packed record of dir_read. name_length UTF-8 bytes follow without a
 * terminator; the next record starts record_length bytes after this one,
 * 8-byte aligned. */
struct AfsplusArosDirEntry {
    uint64_t object_id;
    uint32_t kind;
    uint32_t name_length;
    uint32_t record_length;
    uint32_t reserved;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(struct AfsplusArosStat) == 88,
    "AfsplusArosStat ABI drift");
_Static_assert(sizeof(struct AfsplusArosExtent) == 24,
    "AfsplusArosExtent ABI drift");
_Static_assert(sizeof(struct AfsplusArosDirEntry) == 24,
    "AfsplusArosDirEntry ABI drift");
_Static_assert(sizeof(struct AfsplusArosCounters) == 112,
    "AfsplusArosCounters ABI drift");
_Static_assert(sizeof(struct AfsplusArosHealth) == 136,
    "AfsplusArosHealth ABI drift");
_Static_assert(sizeof(struct AfsplusArosHealthEvent) == 16,
    "AfsplusArosHealthEvent ABI drift");
_Static_assert(sizeof(struct AfsplusArosTraceCounters) == 40,
    "AfsplusArosTraceCounters ABI drift");
_Static_assert(sizeof(struct afsp_trace_event) == 64,
    "afsp_trace_event ABI drift");
_Static_assert(sizeof(struct AfsplusArosInterface) == 24,
    "AfsplusArosInterface ABI drift");
_Static_assert(sizeof(struct AfsplusArosCapabilities) == 64,
    "AfsplusArosCapabilities ABI drift");
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

/* Callable without a mounted filesystem. */
int32_t afsplus_aros_interface(struct AfsplusArosInterface *output);

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
int32_t afsplus_aros_parent_lock_with_access(struct AfsplusAros *filesystem,
    uint64_t lock, uint32_t access, uint64_t *output_lock);
int32_t afsplus_aros_same_lock(struct AfsplusAros *filesystem,
    uint64_t first_lock, uint64_t second_lock, uint32_t *output_same);
int32_t afsplus_aros_free_lock(struct AfsplusAros *filesystem, uint64_t lock);

int32_t afsplus_aros_open(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint32_t mode, int64_t now_seconds, uint32_t now_nanoseconds,
    uint64_t *output_file);
int32_t afsplus_aros_parent_of_file(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t *output_lock);
int32_t afsplus_aros_lock_from_file(struct AfsplusAros *filesystem,
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

/* Group AFSPLUS_AROS_GROUP_DOS_METADATA. An empty name addresses the object
 * of base_lock itself. protection is the 32-bit DOS word, stored as given. */
int32_t afsplus_aros_set_protection(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint32_t protection, int64_t now_seconds, uint32_t now_nanoseconds);
int32_t afsplus_aros_set_modified(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    int64_t modified_seconds, uint32_t modified_nanoseconds,
    int64_t now_seconds, uint32_t now_nanoseconds);

/* Group AFSPLUS_AROS_GROUP_DOS_HANDLES. open_from_lock turns a lock on a file
 * into a file handle: on success the lock identifier is dead and its shared
 * or exclusive hold continues as the handle's; on failure the lock is
 * untouched. The handle is writable on a read-write mount and never
 * truncates. change_*_mode takes AFSPLUS_AROS_LOCK_*: shared to exclusive
 * needs the caller to be the object's only holder (ERROR_OBJECT_IN_USE
 * otherwise, nothing changed); exclusive to shared always succeeds. */
int32_t afsplus_aros_open_from_lock(struct AfsplusAros *filesystem,
    uint64_t lock, uint64_t *output_file);
int32_t afsplus_aros_change_lock_mode(struct AfsplusAros *filesystem,
    uint64_t lock, uint32_t access);
int32_t afsplus_aros_change_file_mode(struct AfsplusAros *filesystem,
    uint64_t file, uint32_t access);
/* Interface revision 9, same group. Runtime write protection for the mount's
 * lifetime: protecting flushes, then every mutating call answers
 * ERROR_DISK_WRITE_PROTECTED and the volume is not changed at all: flush and
 * close publish only writes accepted before and start no orphan cleanup.
 * Protecting a protected volume succeeds only with the stored key
 * (ERROR_DISK_WRITE_PROTECTED otherwise). Unprotecting needs the stored key,
 * or any key when the stored key is zero; a wrong key is
 * ERROR_INVALID_COMPONENT_NAME and changes nothing. No master key exists. */
int32_t afsplus_aros_set_write_protect(struct AfsplusAros *filesystem,
    uint32_t protect, uint32_t key);

/* Group AFSPLUS_AROS_GROUP_DOS_RECORDS: advisory byte-range record locks of
 * LockRecord, held in memory and owned by a file handle; closing the handle
 * releases them. lock_record never waits: an overlapping range of another
 * handle of the same object, where either side is exclusive, is
 * ERROR_LOCK_COLLISION and the caller decides about waiting. A zero length
 * or a range past 2^64 is ERROR_BAD_NUMBER, a full table
 * ERROR_NO_FREE_STORE. free_record needs the owning handle and the exact
 * range, else ERROR_RECORD_NOT_LOCKED. */
int32_t afsplus_aros_lock_record(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length, uint32_t exclusive);
int32_t afsplus_aros_free_record(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length);

/* Group AFSPLUS_AROS_GROUP_OBJECT_IDS: the object-ID operations of the v2
 * API. Names are UTF-8 whatever the mount's DOS encoding. lookup_id takes no
 * lock and never follows a link. stat_id answers ERROR_OBJECT_NOT_FOUND for a
 * deleted object and for a guessed identifier.
 *
 * dir_open starts a paged walk of the directory of base_lock, independent of
 * the lock and of its ExNext cursor afterwards; the table of walks is
 * bounded. dir_read packs up to max_entries (1 to 64) AfsplusArosDirEntry
 * records; it reads no more entries than the buffer is certain to hold, so
 * capacity is at least AFSPLUS_AROS_DIR_RECORD_MAX (ERROR_BAD_NUMBER
 * otherwise) and none is lost. The position is the last returned name: every
 * entry ordered after it is returned once, whatever was created, deleted or
 * renamed between two calls, and one call is one consistent view. A deleted
 * directory is ERROR_OBJECT_NOT_FOUND. After the end every call stores zero
 * records and output_eof 1. */
int32_t afsplus_aros_lookup_id(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint64_t *output_object_id);
int32_t afsplus_aros_stat_id(struct AfsplusAros *filesystem,
    uint64_t object_id, struct AfsplusArosStat *output);
int32_t afsplus_aros_dir_open(struct AfsplusAros *filesystem,
    uint64_t base_lock, uint64_t *output_dir);
int32_t afsplus_aros_dir_read(struct AfsplusAros *filesystem, uint64_t dir,
    uint8_t *buffer, uint32_t capacity, uint32_t max_entries,
    uint32_t *output_count, uint32_t *output_eof);
int32_t afsplus_aros_dir_close(struct AfsplusAros *filesystem, uint64_t dir);

/* Group AFSPLUS_AROS_GROUP_EXTENT_MAP: the committed mapping of
 * offset..offset+length of an open file, clipped to that range, without
 * physical addresses; what a pager needs to plan faults and block-aligned
 * transfers. Stores up to capacity (1 to 64) extents and their count, found
 * in one tree descent wherever offset lies. output_complete is 1 when every
 * mapping intersecting the range was stored; otherwise query again from
 * output_next_offset. With unpublished writes pending the answer is
 * ERROR_OBJECT_IN_USE and nothing is committed; flush first. */
int32_t afsplus_aros_extent_map(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length,
    struct AfsplusArosExtent *extents, uint32_t capacity,
    uint32_t *output_count, uint32_t *output_complete,
    uint64_t *output_next_offset);

/* Group AFSPLUS_AROS_GROUP_VOLUME_LABEL. The label is the DOS volume name,
 * in the mount's name encoding. volume_label stores the byte count in
 * output_required and leaves a short buffer untouched. set_volume_label
 * relabels in one commit: a power cut leaves the old label or the new one. A
 * name that is empty or contains ':', '/' or NUL is
 * ERROR_INVALID_COMPONENT_NAME; one whose stored UTF-8 form exceeds 64 bytes
 * is ERROR_OBJECT_TOO_LARGE. */
int32_t afsplus_aros_volume_label(struct AfsplusAros *filesystem,
    uint8_t *label, uint32_t capacity, uint32_t *output_required);
int32_t afsplus_aros_set_volume_label(struct AfsplusAros *filesystem,
    const uint8_t *label, uint32_t label_length, int64_t now_seconds,
    uint32_t now_nanoseconds);

/* Group AFSPLUS_AROS_GROUP_DOS_COMMENT. A comment is text in the mount's
 * name encoding, addressed like set_protection: an empty name is the base
 * lock's own object. An empty comment removes the stored one; a comment whose
 * stored form exceeds 255 bytes is ERROR_COMMENT_TOO_BIG (220). comment() fills
 * at most comment_capacity bytes, cut at a character boundary, stores the
 * count in output_length and never fails on the comment's content: a
 * character the encoding lacks reads as '?'. No terminator is written. */
int32_t afsplus_aros_set_comment(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    const uint8_t *comment, uint32_t comment_length,
    int64_t now_seconds, uint32_t now_nanoseconds);
int32_t afsplus_aros_comment(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint8_t *comment, uint32_t comment_capacity, uint32_t *output_length);
/* The same report for an open file, which ACTION_EXAMINE_FH needs. */
int32_t afsplus_aros_file_comment(struct AfsplusAros *filesystem,
    uint64_t file, uint8_t *comment, uint32_t comment_capacity,
    uint32_t *output_length);

/* Group AFSPLUS_AROS_GROUP_CACHE: a read cache of device blocks in front of
 * the block callbacks. Every write reaches the device before the call
 * returns, so the cache changes what is read from the device and nothing it
 * keeps. set_cache_blocks asks for blocks (zero: none), bounded by the volume
 * size and 1 << 20, and stores the bounded size in output_blocks; the cache
 * takes the memory at its next device access and keeps its size when the
 * memory cannot be had. A mount starts without one. */
int32_t afsplus_aros_set_cache_blocks(struct AfsplusAros *filesystem,
    uint32_t blocks, uint32_t *output_blocks);

/* Group AFSPLUS_AROS_GROUP_COMMIT (ADR-121): when changes reach the disk.
 * set_commit_policy with max_age_ms 0 makes every change durable when it
 * returns, as a mount starts. Otherwise changes gather and are committed
 * together once the volume has been idle idle_ms (1 to max_age_ms), once the
 * oldest is max_age_ms old (at most 60000), at the window's bound, or when
 * anything asks for durability: fsync, ACTION_FLUSH, inhibit, dismount. A
 * volume without the intent log's data updates cannot delay and answers
 * ERROR_ACTION_NOT_KNOWN. commit_due commits when now makes the changes due
 * and sets output_pending to 1 while changes still wait: the handler calls it
 * after its packets and on its clock until it answers 0. A crash loses at
 * most the waiting changes, whole and in order. */
int32_t afsplus_aros_set_commit_policy(struct AfsplusAros *filesystem,
    uint32_t max_age_ms, uint32_t idle_ms);
int32_t afsplus_aros_commit_due(struct AfsplusAros *filesystem,
    int64_t now_seconds, uint32_t now_nanoseconds, uint32_t *output_pending);

/* Group AFSPLUS_AROS_GROUP_ATTRIBUTES: extended attributes of the object
 * name under base_lock, an empty name being the base lock's own object.
 * Attribute names carry their namespace ("user.", "aros.", "system.",
 * "security.") and are text in the mount's name encoding, at most 255 bytes
 * stored; values are bytes, at most 65,535, and an object's whole set at
 * most 64 KiB (ERROR_OBJECT_TOO_LARGE). Every namespace is readable; this
 * boundary writes "user." and "aros." and answers ERROR_WRITE_PROTECTED for
 * the two it only preserves. A volume that cannot store attributes answers
 * ERROR_ACTION_NOT_KNOWN and lacks FSV2_CAP_XATTRS.
 *
 * get and list store the size in output_required and fill the buffer only
 * when it fits, so a zero capacity asks for the size. An absent attribute is
 * ERROR_OBJECT_NOT_FOUND. list separates names with a NUL after each, in the
 * volume's byte order of stored names. set writes under mode; CREATE answers
 * ERROR_OBJECT_EXISTS, REPLACE and REMOVE ERROR_OBJECT_NOT_FOUND; REMOVE
 * takes no value. */
#define AFSPLUS_AROS_ATTRIBUTE_UPSERT UINT32_C(0)
#define AFSPLUS_AROS_ATTRIBUTE_CREATE UINT32_C(1)
#define AFSPLUS_AROS_ATTRIBUTE_REPLACE UINT32_C(2)
#define AFSPLUS_AROS_ATTRIBUTE_REMOVE UINT32_C(3)
int32_t afsplus_aros_get_attribute(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    const uint8_t *attribute, uint32_t attribute_length,
    uint8_t *value, uint32_t value_capacity, uint32_t *output_required);
int32_t afsplus_aros_list_attributes(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint8_t *names, uint32_t names_capacity, uint32_t *output_required);
int32_t afsplus_aros_set_attribute(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    const uint8_t *attribute, uint32_t attribute_length,
    const uint8_t *value, uint32_t value_length, uint32_t mode,
    int64_t now_seconds, uint32_t now_nanoseconds);

/* Group AFSPLUS_AROS_GROUP_SOFT_LINKS. The target is an opaque path in the
 * mount's name encoding. Locate and open answer ERROR_IS_SOFT_LINK for a
 * link; the caller substitutes the target and retries. read_soft_link stores
 * the target size in output_required; when it exceeds capacity the buffer is
 * untouched and the call still succeeds. */
int32_t afsplus_aros_make_soft_link(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    const uint8_t *target, uint32_t target_length,
    int64_t now_seconds, uint32_t now_nanoseconds);
int32_t afsplus_aros_read_soft_link(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint8_t *target, uint32_t target_capacity, uint32_t *output_required);

/* Group AFSPLUS_AROS_GROUP_API_V2: filesystem-neutral 64-bit operations on
 * the same locks and file handles as the DOS calls.
 *
 * read_at/write_at take an explicit 64-bit offset and neither use nor move
 * the DOS file position. clone_file and clone_range answer
 * ERROR_ACTION_NOT_KNOWN on a volume without the capability, the signal to
 * fall back to a byte copy. preallocate reserves storage without changing
 * the file size and answers ERROR_OBJECT_TOO_LARGE, with nothing reserved,
 * when one request exceeds the mount's per-request block budget. replace is
 * an atomic rename over an existing target that no lock or handle holds.
 * advise takes an afsplus_access_hint_t value and reports what it changed;
 * no hint alters durability, contents or allocation. */
int32_t afsplus_aros_read_at(struct AfsplusAros *filesystem, uint64_t file,
    uint64_t offset, uint8_t *destination, uint32_t length,
    uint32_t *output_count);
int32_t afsplus_aros_write_at(struct AfsplusAros *filesystem, uint64_t file,
    uint64_t offset, const uint8_t *source, uint32_t length,
    int64_t now_seconds, uint32_t now_nanoseconds, uint32_t *output_count);
int32_t afsplus_aros_clone_file(struct AfsplusAros *filesystem,
    uint64_t source_lock, uint64_t target_base_lock,
    const uint8_t *target_name, uint32_t target_name_length,
    int64_t now_seconds, uint32_t now_nanoseconds);
int32_t afsplus_aros_clone_range(struct AfsplusAros *filesystem,
    uint64_t source_file, uint64_t source_offset, uint64_t target_file,
    uint64_t target_offset, uint64_t length, int64_t now_seconds,
    uint32_t now_nanoseconds);
int32_t afsplus_aros_preallocate(struct AfsplusAros *filesystem,
    uint64_t file, uint64_t offset, uint64_t length, int64_t now_seconds,
    uint32_t now_nanoseconds);
int32_t afsplus_aros_replace(struct AfsplusAros *filesystem,
    uint64_t source_base_lock, const uint8_t *source_name,
    uint32_t source_name_length, uint64_t target_base_lock,
    const uint8_t *target_name, uint32_t target_name_length,
    int64_t now_seconds, uint32_t now_nanoseconds);
int32_t afsplus_aros_advise(struct AfsplusAros *filesystem, uint64_t file,
    uint64_t offset, uint64_t length, uint32_t hint, uint32_t *output_effect);

/* Group AFSPLUS_AROS_GROUP_NOTIFY: a bounded watch table. A watch names an
 * entry under base_lock, which need not exist; an empty name watches the
 * object of base_lock. A directory watch also fires for changes to its
 * entries. Pending state is one flag per watch: changes between two drains
 * are one event. watch_drain stores up to capacity pending identifiers,
 * lowest first; the rest stay pending. */
int32_t afsplus_aros_watch_add(struct AfsplusAros *filesystem,
    uint64_t base_lock, const uint8_t *name, uint32_t name_length,
    uint64_t *output_watch);
int32_t afsplus_aros_watch_remove(struct AfsplusAros *filesystem,
    uint64_t watch);
int32_t afsplus_aros_watch_drain(struct AfsplusAros *filesystem,
    uint64_t *watches, uint32_t capacity, uint32_t *output_count);

/* Group AFSPLUS_AROS_GROUP_OBSERVE. Every failed call on a mounted instance
 * that describes the volume or its device enters the health log.
 *
 * set_trace_sink attaches the core flight recorder to sink->emit, or detaches
 * with NULL. emit runs inside filesystem operations on the handler task: it
 * must not block, call back into this library or unwind; it hands the event
 * to a preallocated queue and returns. category_mask uses AFSP_TRACE_*. */
int32_t afsplus_aros_health(struct AfsplusAros *filesystem,
    struct AfsplusArosHealth *output);
int32_t afsplus_aros_health_events(struct AfsplusAros *filesystem,
    struct AfsplusArosHealthEvent *events, uint32_t capacity,
    uint32_t *output_count);
int32_t afsplus_aros_set_trace_sink(struct AfsplusAros *filesystem,
    const struct afsp_trace_sink *sink);
int32_t afsplus_aros_trace_counters(struct AfsplusAros *filesystem,
    struct AfsplusArosTraceCounters *output);

/* Group AFSPLUS_AROS_GROUP_MANAGE. info_json writes one UTF-8 JSON object
 * without a terminator: schema "afsplus-handler-info", its schema_version,
 * then volume, mount, capabilities, health and handles. output_required
 * receives the byte count; when it exceeds capacity the buffer is untouched
 * and the call still succeeds. A consumer rejects a schema_version it does
 * not know. */
int32_t afsplus_aros_info_json(struct AfsplusAros *filesystem,
    uint8_t *buffer, uint32_t capacity, uint32_t *output_required);

/* Group AFSPLUS_AROS_GROUP_COUNTERS. */
int32_t afsplus_aros_counters(struct AfsplusAros *filesystem,
    struct AfsplusArosCounters *output);

/* Group AFSPLUS_AROS_GROUP_INTERFACE_QUERY. */
int32_t afsplus_aros_capabilities(struct AfsplusAros *filesystem,
    struct AfsplusArosCapabilities *output);

#ifdef __cplusplus
}
#endif

#endif
