/*
 * Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * This file is distributed under the AROS Public License 1.1.
 */
#ifndef DOS_FILESYSTEM_V2_PACKET_H
#define DOS_FILESYSTEM_V2_PACKET_H

#include "filesystem_v2.h"

/* Native filesystem-neutral transport. Unknown handlers must answer
 * ERROR_ACTION_NOT_KNOWN. Recognized but unsupported operations answer
 * ERROR_NOT_IMPLEMENTED. dp_Arg1 points at a request in shared memory;
 * dp_Res1 is DOSTRUE/DOSFALSE and dp_Res2 is an ERROR_* value.
 * Packet number is provisional, outside the classic allocated ranges. */
#define ACTION_FILESYSTEM_V2 INT32_C(0x46535632)
#define FSV2_PACKET_VERSION 1
#define FSV2_OP_CAPABILITIES 1
#define FSV2_OP_STAT 2
#define FSV2_OP_SET_MTIME 3
#define FSV2_OP_REPLACE 4
#define FSV2_FLAG_FH 1
#define FSV2_FLAG_NOFOLLOW 2

struct FSV2_Request {
    uint32_t struct_size;
    uint16_t version;
    uint16_t operation;
    uint32_t flags;
    uint32_t nanoseconds;
    uint64_t capabilities;
    /* Locks as BPTRs; with FH, object[0] is fh_Arg1. */
    uint64_t object[2];
    /* Native NUL-terminated DOS paths, relative to the corresponding lock.
     * NULL path means examine the lock/filehandle itself. */
    union { const char *path0; uint64_t path0_bits; };
    union { const char *path1; uint64_t path1_bits; };
    int64_t seconds;
    struct FSV2_Stat stat;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(struct FSV2_Request) == 136, "FSV2 packet ABI drift");
_Static_assert(offsetof(struct FSV2_Request, stat) == 64, "FSV2 stat offset drift");
#endif

#endif
