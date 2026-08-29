#ifndef AROS_FILESYSTEM_V2_H
#define AROS_FILESYSTEM_V2_H

#include <stdint.h>
#include <stddef.h>

typedef uint64_t FSV2_ObjectId;
typedef int64_t  FSV2_Offset;
typedef uint64_t FSV2_Size;
typedef uint64_t FSV2_Sequence;

enum FSV2_Result {
    FSV2_OK = 0,
    FSV2_ERR_NOT_FOUND,
    FSV2_ERR_NOT_SUPPORTED,
    FSV2_ERR_READ_ONLY,
    FSV2_ERR_INVALID,
    FSV2_ERR_IO,
    FSV2_ERR_CORRUPT,
    FSV2_ERR_RESCAN_REQUIRED,
    FSV2_ERR_STALE,
    FSV2_ERR_NO_SPACE
};

enum FSV2_Capability {
    FSV2_CAP_64BIT_IO          = UINT64_C(1) << 0,
    FSV2_CAP_UTF8_NAMES        = UINT64_C(1) << 1,
    FSV2_CAP_SYMLINKS          = UINT64_C(1) << 2,
    FSV2_CAP_HARDLINKS         = UINT64_C(1) << 3,
    FSV2_CAP_XATTRS            = UINT64_C(1) << 4,
    FSV2_CAP_ATOMIC_REPLACE    = UINT64_C(1) << 5,
    FSV2_CAP_OBJECT_IDS        = UINT64_C(1) << 6,
    FSV2_CAP_FAST_ENUMERATION  = UINT64_C(1) << 7,
    FSV2_CAP_CHANGE_STREAM     = UINT64_C(1) << 8,
    FSV2_CAP_SPARSE            = UINT64_C(1) << 9,
    FSV2_CAP_FSYNC             = UINT64_C(1) << 10
};

struct FSV2_String {
    const uint8_t *bytes;
    size_t length;
    uint32_t encoding;
};

struct FSV2_Stat {
    FSV2_ObjectId object_id;
    FSV2_Size size;
    FSV2_Size allocated_size;
    uint64_t type;
    uint64_t protection;
    int64_t mtime_seconds;
    uint32_t mtime_nanoseconds;
};

struct FSV2_Change {
    FSV2_Sequence sequence;
    FSV2_ObjectId object_id;
    FSV2_ObjectId old_parent;
    FSV2_ObjectId new_parent;
    uint32_t kind;
};

#endif
