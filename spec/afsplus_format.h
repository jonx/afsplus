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

/* Experimental magic. Freeze before epoch 1. */
#define AFSP_MAGIC_U64 UINT64_C(0x3153554c50534641) /* "AFSPLUS1" LE-ish marker */

enum afsp_feature_class {
    AFSP_FEATURE_COMPAT = 1,
    AFSP_FEATURE_RO_COMPAT = 2,
    AFSP_FEATURE_INCOMPAT = 3
};

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

struct afsp_timespec_wire {
    uint8_t seconds_le[8];
    uint8_t nanoseconds_le[4];
    uint8_t reserved[4];
};

#endif
