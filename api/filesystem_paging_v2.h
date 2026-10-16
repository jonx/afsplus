/* Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * Licensed under the AROS Public License (APL) Version 1.1.
 */
#ifndef DOS_FILESYSTEM_PAGING_V2_H
#define DOS_FILESYSTEM_PAGING_V2_H
#include <stdint.h>
#include <stddef.h>

/* Optional filesystem-neutral snapshot transport. dp_Arg1 is a shared
 * request, dp_Res1 DOSTRUE/DOSFALSE, dp_Res2 an ERROR_* value. Unknown
 * handlers answer ERROR_ACTION_NOT_KNOWN. Snapshot support is independent
 * of FSV2_CAP_PAGING: that bit requires the complete shared coherence API.
 * The number is provisional and outside the classic allocated ranges. */
#define ACTION_FILESYSTEM_PAGING_V2 INT32_C(0x46535032)
#define FSV2_PAGING_VERSION 1
#define FSV2_PAGING_SNAPSHOT_OPEN 1
#define FSV2_SNAPSHOT_ALLOW_COPY 1
#define FSV2_SNAPSHOT_REFLINK 1
#define FSV2_SNAPSHOT_RAM 2

struct FSV2_PagingRequest {
    uint32_t struct_size;
    uint16_t version, operation;
    uint32_t flags, reserved;
    /* Native source fh_Arg1; no file position is changed. */
    uint64_t source;
    /* Caller-allocated struct FileHandle. Success supplies fh_Arg1 and
     * fh_Type; caller owns it and closes it with ordinary DOS Close().
     * The handle supports Read/Seek/size/examine and is immutable. */
    union { void *destination; uint64_t destination_bits; };
    uint64_t size;
    uint32_t storage, reserved2;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(struct FSV2_PagingRequest)==48,"FSV2 snapshot packet ABI");
_Static_assert(offsetof(struct FSV2_PagingRequest,size)==32,"FSV2 snapshot size offset");
#endif
#endif
