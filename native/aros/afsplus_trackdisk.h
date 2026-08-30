/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_AROS_TRACKDISK_H
#define AFSPLUS_AROS_TRACKDISK_H

#include <dos/dos.h>
#include <stddef.h>
#include <stdint.h>

#include "afsplus_aros.h"
#include "debug_observability.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Version 2 adds the optional fixed-size activity sink. */
#define AFSPLUS_AROS_TRACKDISK_ABI_VERSION UINT32_C(2)
#define AFSPLUS_AROS_ALPHA0_BLOCK_SIZE UINT32_C(4096)

typedef int32_t (*AfsplusArosTrackdiskTransfer)(void *context,
    uint32_t command, uint64_t byte_offset, void *buffer, uint32_t length,
    uint32_t writing);
typedef int32_t (*AfsplusArosTrackdiskSync)(void *context);

struct AfsplusArosTrackdiskConfig {
    uint32_t abi_version;
    uint32_t struct_size;
    void *context;
    AfsplusArosTrackdiskTransfer transfer;
    AfsplusArosTrackdiskSync sync;
    uint64_t partition_start_bytes;
    uint64_t partition_length_bytes;
    /* Smallest byte-addressable transfer unit reported by DosEnvec. */
    uint32_t device_block_size;
    /* AFS+ logical block size; Alpha-0 currently requires exactly 4096. */
    uint32_t logical_block_size;
    uint32_t read_command;
    uint32_t write_command;
    uint32_t supports_64bit_offsets;
    uint32_t read_only;
    /* Optional fixed-size activity events for a virtual drive LED. */
    struct afsp_io_activity_sink activity;
};

struct AfsplusArosTrackdisk {
    void *context;
    AfsplusArosTrackdiskTransfer transfer;
    AfsplusArosTrackdiskSync sync;
    uint64_t partition_start_bytes;
    uint64_t partition_length_bytes;
    uint64_t total_blocks;
    uint32_t logical_block_size;
    uint32_t read_command;
    uint32_t write_command;
    uint32_t supports_64bit_offsets;
    uint32_t read_only;
    struct afsp_io_activity_sink activity;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(struct AfsplusArosTrackdiskConfig) == 96,
    "AfsplusArosTrackdiskConfig 64-bit ABI drift");
_Static_assert(sizeof(struct AfsplusArosTrackdisk) == 96,
    "AfsplusArosTrackdisk 64-bit ABI drift");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(struct AfsplusArosTrackdiskConfig) == 72,
    "AfsplusArosTrackdiskConfig 32-bit ABI drift");
_Static_assert(sizeof(struct AfsplusArosTrackdisk) == 68,
    "AfsplusArosTrackdisk 32-bit ABI drift");
#endif
#endif

/* Converts the inclusive cylinder range in a DosEnvec into an exact byte
 * viewport. size_block_longwords follows de_SizeBlock. The caller passes
 * de_LowCyl, de_HighCyl, de_Surfaces and de_BlocksPerTrack after rejecting
 * negative signed values. */
int32_t afsplus_aros_trackdisk_geometry(uint64_t low_cylinder,
    uint64_t high_cylinder, uint64_t surfaces,
    uint64_t blocks_per_track, uint64_t size_block_longwords,
    uint64_t *partition_start_bytes, uint64_t *partition_length_bytes);

/* Initializes caller-owned state and the callback device consumed by
 * afsplus_aros_mount(). Both must remain alive for the whole mount. */
int32_t afsplus_aros_trackdisk_init(
    const struct AfsplusArosTrackdiskConfig *config,
    struct AfsplusArosTrackdisk *trackdisk,
    struct AfsplusArosDevice *device);

#ifdef __cplusplus
}
#endif

#endif
