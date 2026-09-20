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
#include <dos/notify.h>
#include <stddef.h>
#include <stdint.h>

#include "afsplus_aros.h"
#include "afsplus_ext_packet.h"

#ifdef __cplusplus
extern "C" {
#endif

#define AFSPLUS_AROS_PACKET_ABI_VERSION UINT32_C(5)

/* afsplus_aros_packet_process kept the packet: no result is stored and the
 * handler must not reply. The packet comes back through the complete
 * callback. Only a configuration with that callback ever sees this value. */
#define AFSPLUS_AROS_PACKET_DEFERRED INT32_C(-1)

struct AfsplusArosPacketContext;

typedef void *(*AfsplusArosPacketAllocate)(void *context, size_t size);
typedef void (*AfsplusArosPacketFree)(void *context, void *allocation,
    size_t size);
typedef int32_t (*AfsplusArosPacketNow)(void *context,
    int64_t *unix_seconds, uint32_t *nanoseconds);
/* Delivers one change notification: a NotifyMessage to nr_Port or a Signal
 * to nr_Task, as nr_Flags selects. The handler owns message memory, the
 * nr_MsgCount bookkeeping and NRF_WAIT_REPLY suppression. Called from
 * afsplus_aros_packet_process after the packet's own result is stored. */
typedef void (*AfsplusArosPacketNotify)(void *context,
    struct NotifyRequest *request);
/* Renames the DOS volume node around the label change on the volume, in
 * phases, so that neither side changes alone. PREPARE takes whatever the
 * handler needs to rename the node (the DosList write lock, room for the
 * name) and may fail with an ERROR_* value, in which case nothing changes.
 * After a successful PREPARE exactly one of COMMIT (the volume took the
 * label: write the node name, release) or ABORT (it refused: release) follows;
 * neither can fail. */
#define AFSPLUS_AROS_RELABEL_PREPARE UINT32_C(0)
#define AFSPLUS_AROS_RELABEL_COMMIT UINT32_C(1)
#define AFSPLUS_AROS_RELABEL_ABORT UINT32_C(2)
typedef int32_t (*AfsplusArosPacketRelabel)(void *context, uint32_t phase,
    const uint8_t *name, uint32_t name_length);

/* Takes up to capacity events from the handler's trace ring, oldest first,
 * and stores in dropped how many it has lost since the mount. Returns the
 * number taken. Without it the extension packet answers
 * ERROR_NOT_IMPLEMENTED for the trace operations, which is what a mount
 * without a ring does. */
typedef uint32_t (*AfsplusArosPacketTraceTake)(void *context,
    struct afsp_trace_event *events, uint32_t capacity, uint64_t *dropped);

/* Hands back a packet that afsplus_aros_packet_process deferred, with its
 * result stored; the handler replies to it. Called from inside
 * afsplus_aros_packet_process, afsplus_aros_packet_elapsed and
 * afsplus_aros_packet_destroy. */
typedef void (*AfsplusArosPacketComplete)(void *context,
    struct DosPacket *packet);

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
    /* Optional. Without it ACTION_ADD_NOTIFY is ERROR_ACTION_NOT_KNOWN. */
    AfsplusArosPacketNotify notify;
    /* Optional. Without it ACTION_RENAME_DISK is ERROR_ACTION_NOT_KNOWN:
     * a volume whose DOS node kept the old name would answer to two names. */
    AfsplusArosPacketRelabel relabel;
    /* Optional. Without it a waiting ACTION_LOCK_RECORD whose range is taken
     * answers ERROR_LOCK_TIMEOUT at once instead of waiting dp_Arg5 ticks. */
    AfsplusArosPacketComplete complete;
    /* Optional; present when the mount asked for a trace ring. */
    AfsplusArosPacketTraceTake trace_take;
};

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(struct AfsplusArosPacketConfig) == 96,
    "AfsplusArosPacketConfig 64-bit ABI drift");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(struct AfsplusArosPacketConfig) == 52,
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

/* 1 while request is registered through ACTION_ADD_NOTIFY. A handler asks
 * this before it touches the NotifyRequest of a returning NotifyMessage: after
 * EndNotify the application owns that memory again and may have freed it. */
uint32_t afsplus_aros_packet_notify_registered(
    const struct AfsplusArosPacketContext *context,
    const struct NotifyRequest *request);

/* Number of deferred packets. While it is nonzero the handler reports the
 * passing of time with afsplus_aros_packet_elapsed, in ticks of 1/50 s; a
 * packet whose dp_Arg5 ticks have passed completes with ERROR_LOCK_TIMEOUT.
 * Deferred packets are retried, oldest first, whenever a record is freed or
 * a file closed. A later immediate request may take a range before an older
 * waiting one is retried: the queue orders waiters, not all requests. */
uint32_t afsplus_aros_packet_waiting(
    const struct AfsplusArosPacketContext *context);
void afsplus_aros_packet_elapsed(struct AfsplusArosPacketContext *context,
    uint32_t ticks);

uint32_t afsplus_aros_packet_should_quit(
    const struct AfsplusArosPacketContext *context);

/* Installs the complete callback on a running context. A handler that could
 * not open its clock at mount calls this the moment it has one, so that
 * waiting record locks wait from then on rather than only after the next
 * mount. */
void afsplus_aros_packet_set_complete(
    struct AfsplusArosPacketContext *context,
    AfsplusArosPacketComplete complete);

/* What AFSPLUS_EXT_COMMIT_POLICY answers: AFSPLUS_EXT_COMMIT_SYNC or
 * AFSPLUS_EXT_COMMIT_DELAYED, the longest a change then waits in seconds,
 * and whether the delayed policy was taken after the mount. The handler
 * calls this once it has applied a policy, and again whenever it changes. */
void afsplus_aros_packet_set_commit_policy(
    struct AfsplusArosPacketContext *context, uint32_t policy,
    uint32_t seconds, uint32_t late);

#ifdef __cplusplus
}
#endif

#endif
