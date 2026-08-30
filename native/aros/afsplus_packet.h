/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_AROS_PACKET_H
#define AFSPLUS_AROS_PACKET_H

/* Native DosPacket translation for the AFS+ AROS C boundary.
 *
 * This layer deliberately performs no Exec or DOS library calls. The handler
 * owns the message loop and supplies public-memory allocation plus a UTC Unix
 * clock. That keeps packet semantics independently compilable and testable.
 */

#include <dos/dos64.h>
#include <stddef.h>
#include <stdint.h>

#include "afsplus_aros.h"

#ifdef __cplusplus
extern "C" {
#endif

#define AFSPLUS_AROS_PACKET_ABI_VERSION UINT32_C(1)

struct AfsplusArosPacketContext;

typedef void *(*AfsplusArosPacketAllocate)(void *context, size_t size);
typedef void (*AfsplusArosPacketFree)(void *context, void *allocation,
    size_t size);
typedef int32_t (*AfsplusArosPacketNow)(void *context,
    int64_t *unix_seconds, uint32_t *nanoseconds);

struct AfsplusArosPacketConfig {
    uint32_t abi_version;
    uint32_t struct_size;
    struct AfsplusAros *filesystem;
    struct MsgPort *handler_port;
    BPTR volume_node;
    void *callback_context;
    AfsplusArosPacketAllocate allocate;
    AfsplusArosPacketFree free;
    AfsplusArosPacketNow now;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(struct AfsplusArosPacketConfig) == 64,
    "AfsplusArosPacketConfig 64-bit ABI drift");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(struct AfsplusArosPacketConfig) == 36,
    "AfsplusArosPacketConfig 32-bit ABI drift");
#endif
#endif

int32_t afsplus_aros_packet_create(
    const struct AfsplusArosPacketConfig *config,
    struct AfsplusArosPacketContext **output);

/* Closes all packet-owned handles and locks, frees the context, and returns
 * the first close/free error. The Rust filesystem remains mounted. */
int32_t afsplus_aros_packet_destroy(
    struct AfsplusArosPacketContext *context);

/* Fills dp_Res1/dp_Res2, or the DosPacket64 result overlay when applicable.
 * It does not reply to dp_Port; the native handler retains message ownership.
 * The function itself returns zero once a non-null packet has been translated,
 * including when the packet's operation reports an ERROR_* in dp_Res2. */
int32_t afsplus_aros_packet_process(
    struct AfsplusArosPacketContext *context, struct DosPacket *packet);

uint32_t afsplus_aros_packet_should_quit(
    const struct AfsplusArosPacketContext *context);

#ifdef __cplusplus
}
#endif

#endif
