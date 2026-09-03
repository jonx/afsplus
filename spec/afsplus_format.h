#ifndef AFSPLUS_FORMAT_H
#define AFSPLUS_FORMAT_H

/*
 * Draft wire-format constants for AFS+.
 *
 * IMPORTANT:
 * These declarations describe field meaning and identifiers.
 * Do not serialize native C structs directly to disk.
 */

#include <stdint.h>

#define AFSP_FORMAT_EPOCH          1u
#define AFSP_DEFAULT_BLOCK_SHIFT   12u
#define AFSP_DEFAULT_BLOCK_SIZE    (1u << AFSP_DEFAULT_BLOCK_SHIFT)

#define AFSP_NAME_MAX_UTF8_BYTES   255u

#define AFSP_OBJECT_INVALID        UINT64_C(0)
#define AFSP_OBJECT_ROOT           UINT64_C(1)
#define AFSP_OBJECT_ORPHAN_DIRECTORY UINT64_C(2) /* ADR-066 */

/* Experimental magic. Freeze before epoch 1. */
#define AFSP_MAGIC_U64 UINT64_C(0x3153554c50534641) /* "AFSPLUS1" LE-ish marker */

enum afsp_feature_class {
    AFSP_FEATURE_COMPAT = 1,
    AFSP_FEATURE_RO_COMPAT = 2,
    AFSP_FEATURE_INCOMPAT = 3
};

/*
 * INCOMPAT feature bits. Version-3 intent records can name durable COW data
 * for writes/truncates of existing files; older namespace-only log readers
 * must reject such volumes rather than mistake an unknown record for a torn
 * tail (ADR-064).
 */
#define AFSP_INCOMPAT_INTENT_LOG              (UINT64_C(1) << 0)
#define AFSP_INCOMPAT_INTENT_LOG_DATA_UPDATES (UINT64_C(1) << 1)

enum afsp_object_type {
    AFSP_OBJECT_FILE = 1,
    AFSP_OBJECT_DIRECTORY = 2,
    AFSP_OBJECT_SYMLINK = 3,
    AFSP_OBJECT_INTERNAL = 4
};

enum afsp_mount_intent {
    AFSP_MOUNT_RW = 1,
    AFSP_MOUNT_READ_ONLY = 2,
    AFSP_MOUNT_NO_CHANGES = 3,
    AFSP_MOUNT_RECOVERY = 4
};

/*
 * Timestamp wire contract:
 *
 * seconds_le is a signed two's-complement 64-bit count of SI seconds since
 * 1970-01-01 00:00:00 UTC (Unix epoch), encoded little-endian.
 * nanoseconds_le is an unsigned little-endian value in 0..999,999,999.
 *
 * No local timezone, daylight-saving state, or Amiga epoch is stored on disk.
 * Host adapters convert to/from their native time representation.
 */
struct afsp_timespec_wire {
    uint8_t seconds_le[8];
    uint8_t nanoseconds_le[4];
    uint8_t reserved[4];
};

/*
 * Extent flags are a versioned namespace. Unknown semantic flags must be
 * handled according to the owning feature's compatibility class.
 * Bit assignments follow the executable prototype and ADR-061: bit 0 marks
 * an allocated-but-unwritten (preallocated) mapping that reads as zeros;
 * bit 1 is the conservative shared marker whose authority is the volume-wide
 * reference tree. A data-checksum association bit stays reserved so adding
 * that feature later does not require redefining the base extent record.
 */
enum afsp_extent_flag {
    AFSP_EXTENT_FLAG_NONE = 0,
    AFSP_EXTENT_FLAG_UNWRITTEN = 1u << 0,
    AFSP_EXTENT_FLAG_SHARED = 1u << 1, /* ADR-061 */
    AFSP_EXTENT_FLAG_DATA_CHECKSUM_PRESENT = 1u << 2 /* reserved feature */
};

/*
 * RO_COMPAT feature bits. An implementation that does not honour
 * shared-extent reference counts (ADR-061) or the persistent orphan
 * lifecycle (ADR-066) must not mount read-write.
 */
#define AFSP_RO_COMPAT_SHARED_EXTENTS (UINT64_C(1) << 0)
#define AFSP_RO_COMPAT_ORPHAN_DIRECTORY (UINT64_C(1) << 1) /* ADR-066 */

/*
 * Object-record flags are a validated namespace: an implementation rejects a
 * record whose flags it does not understand, so a per-file choice stored
 * here cannot be silently dropped. Bit 0 selects the typed extent-map layout
 * for a file's data; bit 1 is the persistent opt-in to ADR-062 private
 * in-place data updates (ADR-065), legal on file objects only and only when
 * the volume carries AFSP_COMPAT_DATA_POLICY.
 */
enum afsp_object_flag {
    AFSP_OBJECT_FLAG_EXTENT_TREE = 1u << 0,
    AFSP_OBJECT_FLAG_DATA_IN_PLACE = 1u << 1 /* ADR-065 */
};

/*
 * COMPAT feature bits. An implementation that ignores the per-file data
 * policy always uses full data COW, which is strictly stronger (ADR-065).
 */
#define AFSP_COMPAT_DATA_POLICY (UINT64_C(1) << 0)

#endif
