/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * Native AROS handler shell for the Mountable Alpha-0 bridge.
 *
 * This file deliberately lives in the AFS+ repository so the driver can ship
 * independently of any AROS source-tree integration. It owns Exec/DOS
 * resources; packet and partition semantics remain in their independently
 * tested modules.
 */

#define USE_INLINE_STDARG

#include <aros/asmcall.h>
#include <aros/debug.h>
#include <aros/stdc/string.h>
#include <exec/types.h>
#include <devices/newstyle.h>
#include <devices/timer.h>
#include <devices/trackdisk.h>
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <dos/filehandler.h>
#include <dos/notify.h>
#include <exec/errors.h>
#include <exec/execbase.h>
#include <exec/memory.h>
#include <libraries/locale.h>
#include <proto/dos.h>
#include <proto/exec.h>
#include <proto/locale.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include "afsplus_claim.h"
#include "afsplus_control.h"
#include "afsplus_packet.h"
#include "afsplus_trackdisk.h"

#ifndef TD_READ64
#define TD_READ64 (CMD_NONSTD + 15)
#define TD_WRITE64 (CMD_NONSTD + 16)
#endif

#define AFSPLUS_UNIX_TO_AMIGA_EPOCH INT64_C(252460800)
#define AFSPLUS_SECONDS_PER_DAY INT64_C(86400)
#define AFSPLUS_SECONDS_PER_MINUTE INT64_C(60)
#define AFSPLUS_TICKS_PER_SECOND UINT32_C(50)
#define AFSPLUS_MAX_NSD_COMMANDS UINT32_C(256)

#ifndef AFSPLUS_AROS_TRACE_STARTUP
#define AFSPLUS_AROS_TRACE_STARTUP 0
#endif

#if AFSPLUS_AROS_TRACE_STARTUP && defined(__aarch64__)
#include <aros/apple/startup.h>
#include <proto/kernel.h>
#endif

/*
 * MacAROS's Rust std port obtains process arguments through these globals.
 * A DOS handler is entered through handler(SysBase), not main(argc, argv), so
 * its argument vector is intentionally empty for the complete process life.
 */
int aros_argc = 0;
char **aros_argv = NULL;

struct AfsplusArosHandler {
    struct ExecBase *SysBase;
    struct DosLibrary *DOSBase;
    struct LocaleBase *LocaleBase;
    struct Locale *locale;
    struct Process *process;
    struct MsgPort *handler_port;
    struct DosList *device_node;
    struct FileSysStartupMsg *startup;
    struct DosEnvec *environment;
    struct MsgPort *device_port;
    struct IOExtTD *device_request;
    struct DosList *volume_node;
    struct AfsplusArosTrackdisk trackdisk;
    struct AfsplusArosDevice device;
    struct AfsplusAros *filesystem;
    struct AfsplusArosPacketContext *packets;
    /* Reply port of the NotifyMessages this handler sent, and how many of
     * them are still out. The port must outlive every one of them. */
    struct MsgPort *notify_port;
    uint32_t notify_outstanding;
    /* timer.device, open only to time waiting record locks. Without it the
     * packet layer gets no complete callback and such locks never wait. */
    struct MsgPort *timer_port;
    struct timerequest *timer_request;
    uint32_t timer_open;
    uint32_t timer_pending;
    uint8_t *bounce;
    uintptr_t dma_mask;
    uint32_t bounce_size;
    uint32_t use_dma_mask;
    uint32_t device_open;
    uint32_t volume_registered;
    uint32_t read_only;
    /* From the DOSDriver Control string. */
    uint32_t mount_flags;
    uint32_t name_encoding;
    /* The preallocated queue the trace sink hands its events to, empty
     * unless the Control string asked for it. See afsplus_aros_trace_emit. */
    struct afsp_trace_event *trace_ring;
    uint32_t trace_capacity;
    uint32_t trace_count;
    uint32_t trace_next;
    uint64_t trace_dropped;
    uint16_t read_command;
    uint16_t write_command;
    uint32_t supports_64bit_offsets;
    const char *startup_stage;
};

#if AFSPLUS_AROS_TRACE_STARTUP && defined(__aarch64__)
static void afsplus_aros_startup_trace(
    struct ExecBase *SysBase, ULONG event)
{
    struct KernelBase *KernelBase = OpenResource(
        (CONST_STRPTR)"kernel.resource");
    intptr_t image_start;
    intptr_t image_size;
    uint8_t *image;
    uint32_t sectors;
    uint64_t header_offset;
    volatile uint32_t *trace;

    if (KernelBase == NULL)
        return;
    image_start = KrnGetSystemAttr(AROS_APPLE_KATTR_BOOT_IMAGE_START);
    image_size = KrnGetSystemAttr(AROS_APPLE_KATTR_BOOT_IMAGE_SIZE);
    if (image_start <= 0 || image_size < 512)
        return;
    image = (uint8_t *)(uintptr_t)image_start;
    sectors = (uint32_t)image[19] | ((uint32_t)image[20] << 8);
    if (sectors == 0) {
        sectors = (uint32_t)image[32] | ((uint32_t)image[33] << 8) |
            ((uint32_t)image[34] << 16) | ((uint32_t)image[35] << 24);
    }
    header_offset = ((uint64_t)sectors * 512U + 4095U) & ~UINT64_C(4095);
    if (sectors == 0 || header_offset > (uint64_t)image_size ||
        UINT64_C(80) > (uint64_t)image_size - header_offset ||
        memcmp(image + header_offset, "AFSPRAM", 7) != 0)
        return;
    trace = (volatile uint32_t *)(image + header_offset + 64U);
    trace[0] = (uint32_t)event;
    trace[1] += 1U;
}
#elif AFSPLUS_AROS_TRACE_STARTUP
static void afsplus_aros_startup_trace(
    struct ExecBase *SysBase, ULONG event)
{
    (void)SysBase;
    bug("[AFSPLUS-EVENT] 0x%08lx\n", (unsigned long)event);
}
#else
#define afsplus_aros_startup_trace(SysBase, event) ((void)0)
#endif

void afsplus_aros_trace_stage(const char *stage)
{
#if AFSPLUS_AROS_TRACE_STARTUP && !defined(__aarch64__)
    bug("[AFSPLUS-STARTUP] %s\n", stage);
#else
    (void)stage;
#endif
}

static void set_startup_stage(struct AfsplusArosHandler *handler,
    const char *stage)
{
#if AFSPLUS_AROS_TRACE_STARTUP && defined(__aarch64__)
    uint32_t hash = UINT32_C(2166136261);
    const unsigned char *cursor = (const unsigned char *)stage;
#endif

    handler->startup_stage = stage;
    afsplus_aros_trace_stage(stage);
#if AFSPLUS_AROS_TRACE_STARTUP && defined(__aarch64__)
    while (*cursor != 0) {
        hash ^= *cursor++;
        hash *= UINT32_C(16777619);
    }
    afsplus_aros_startup_trace(handler->SysBase,
        UINT32_C(0x30000000) | (hash & UINT32_C(0x00ffffff)));
#endif
}

static void reply_packet(struct MsgPort *handler_port,
    struct ExecBase *SysBase, struct DosPacket *packet)
{
    struct MsgPort *reply_port = packet->dp_Port;
    struct Message *message = packet->dp_Link;

    packet->dp_Port = handler_port;
    message->mn_Node.ln_Name = (char *)packet;
    PutMsg(reply_port, message);
}

void afsplus_aros_trace_emit(void *context,
    const struct afsp_trace_event *event);
uint32_t afsplus_aros_trace_take(struct AfsplusArosHandler *handler,
    struct afsp_trace_event *events, uint32_t capacity, uint64_t *dropped);

/* afsplus_packet.h: the trace ring of this handler. */
static uint32_t packet_trace_take(void *context,
    struct afsp_trace_event *events, uint32_t capacity, uint64_t *dropped)
{
    return afsplus_aros_trace_take(context, events, capacity, dropped);
}

/* A packet the packet layer had kept comes back with its result stored. */
static void packet_complete(void *context, struct DosPacket *packet)
{
    struct AfsplusArosHandler *handler = context;

    reply_packet(handler->handler_port, handler->SysBase, packet);
}

/* Waiting record locks are timed in steps of this many 1/50 s ticks, so a
 * wait may last up to one step longer than asked. */
#define AFSPLUS_AROS_WAIT_STEP_TICKS UINT32_C(5)

static void open_wait_timer(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;

    handler->timer_port = CreateMsgPort();
    if (handler->timer_port == NULL)
        return;
    handler->timer_request = (struct timerequest *)CreateIORequest(
        handler->timer_port, sizeof(*handler->timer_request));
    if (handler->timer_request != NULL
        && OpenDevice((CONST_STRPTR)"timer.device", UNIT_VBLANK,
            (struct IORequest *)handler->timer_request, 0) == 0)
    {
        handler->timer_open = 1;
        return;
    }
    if (handler->timer_request != NULL)
        DeleteIORequest((struct IORequest *)handler->timer_request);
    handler->timer_request = NULL;
    DeleteMsgPort(handler->timer_port);
    handler->timer_port = NULL;
}

static void close_wait_timer(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;

    if (handler->timer_port == NULL)
        return;
    if (handler->timer_pending)
    {
        AbortIO((struct IORequest *)handler->timer_request);
        WaitIO((struct IORequest *)handler->timer_request);
        handler->timer_pending = 0;
    }
    if (handler->timer_open)
        CloseDevice((struct IORequest *)handler->timer_request);
    handler->timer_open = 0;
    DeleteIORequest((struct IORequest *)handler->timer_request);
    handler->timer_request = NULL;
    DeleteMsgPort(handler->timer_port);
    handler->timer_port = NULL;
}

/* Collects an expired step and keeps one step running while a packet waits. */
static void run_wait_timer(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;

    if (!handler->timer_open)
        return;
    if (handler->timer_pending
        && CheckIO((struct IORequest *)handler->timer_request) != NULL)
    {
        WaitIO((struct IORequest *)handler->timer_request);
        handler->timer_pending = 0;
        afsplus_aros_packet_elapsed(handler->packets,
            AFSPLUS_AROS_WAIT_STEP_TICKS);
    }
    if (!handler->timer_pending
        && afsplus_aros_packet_waiting(handler->packets) != 0)
    {
        handler->timer_request->tr_node.io_Command = TR_ADDREQUEST;
        handler->timer_request->tr_time.tv_secs = 0;
        handler->timer_request->tr_time.tv_micro =
            AFSPLUS_AROS_WAIT_STEP_TICKS * UINT32_C(20000);
        SendIO((struct IORequest *)handler->timer_request);
        handler->timer_pending = 1;
    }
}

static void *packet_allocate(void *context, size_t size)
{
    struct AfsplusArosHandler *handler = context;
    struct ExecBase *SysBase = handler->SysBase;

    if (size == 0 || size > UINT32_MAX)
        return NULL;
    return AllocMem((ULONG)size, MEMF_PUBLIC | MEMF_CLEAR);
}

static void packet_free(void *context, void *allocation, size_t size)
{
    struct AfsplusArosHandler *handler = context;
    struct ExecBase *SysBase = handler->SysBase;

    if (allocation != NULL && size != 0 && size <= UINT32_MAX)
        FreeMem(allocation, (ULONG)size);
}

/* Delivers one change notification as the AROS FAT handler does: a signal,
 * or a NotifyMessage unless NRF_WAIT_REPLY holds it back while an earlier one
 * is unreplied. A message that cannot be allocated is dropped; the watch
 * fires again on the next change. */
static void packet_notify(void *context, struct NotifyRequest *request)
{
    struct AfsplusArosHandler *handler = context;
    struct ExecBase *SysBase = handler->SysBase;
    struct NotifyMessage *message;

    if ((request->nr_Flags & NRF_SEND_SIGNAL) != 0)
    {
        if (request->nr_stuff.nr_Signal.nr_Task != NULL
            && request->nr_stuff.nr_Signal.nr_SignalNum < 32)
            Signal(request->nr_stuff.nr_Signal.nr_Task,
                1UL << request->nr_stuff.nr_Signal.nr_SignalNum);
        return;
    }
    if ((request->nr_Flags & NRF_SEND_MESSAGE) == 0)
        return;
    if ((request->nr_Flags & NRF_WAIT_REPLY) != 0
        && request->nr_MsgCount > 0)
    {
        /* The change is not dropped: it is owed, and sent when the
         * outstanding message comes back (the AROS RAM handler's rule). */
        request->nr_Flags |= NRF_MAGIC;
        return;
    }
    message = AllocMem(sizeof(*message), MEMF_PUBLIC | MEMF_CLEAR);
    if (message == NULL)
        return;
    message->nm_ExecMessage.mn_ReplyPort = handler->notify_port;
    message->nm_ExecMessage.mn_Length = sizeof(*message);
    message->nm_Class = NOTIFY_CLASS;
    message->nm_Code = NOTIFY_CODE;
    message->nm_NReq = request;
    request->nr_MsgCount++;
    handler->notify_outstanding++;
    PutMsg(request->nr_stuff.nr_Msg.nr_Port, &message->nm_ExecMessage);
}

/* Takes back replied NotifyMessages. After EndNotify the NotifyRequest is the
 * application's again and may be gone, so its count is touched only while
 * the request is still registered. */
static void collect_notify_replies(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;
    struct NotifyMessage *message;

    if (handler->notify_port == NULL)
        return;
    while ((message = (struct NotifyMessage *)GetMsg(handler->notify_port))
        != NULL)
    {
        if (message->nm_Class != NOTIFY_CLASS
            || message->nm_Code != NOTIFY_CODE)
        {
            /* Not ours: its sender waits for a reply like anyone else. */
            ReplyMsg(&message->nm_ExecMessage);
            continue;
        }
        struct NotifyRequest *request = message->nm_NReq;
        uint32_t registered = afsplus_aros_packet_notify_registered(
            handler->packets, request);

        if (registered && request->nr_MsgCount > 0)
            request->nr_MsgCount--;
        if (handler->notify_outstanding > 0)
            handler->notify_outstanding--;
        FreeMem(message, sizeof(*message));
        /* A change that arrived while this message was out is sent now. */
        if (registered && (request->nr_Flags & NRF_MAGIC) != 0)
        {
            request->nr_Flags &= ~NRF_MAGIC;
            packet_notify(handler, request);
        }
    }
}

/* Longest DOS volume name the node buffer is created for. */
#define AFSPLUS_VOLUME_NAME_MAX 107

static void write_volume_node_name(struct DosList *node, const uint8_t *name,
    uint32_t name_length)
{
    memcpy(AROS_BSTR_ADDR(node->dol_Name), name, name_length);
    AROS_BSTR_setstrlen(node->dol_Name, name_length);
}

/* ACTION_RENAME_DISK, DOS side. PREPARE takes the DosList write lock without
 * waiting: the handler is the only task that can serve a lock holder that is
 * itself waiting on this volume, so blocking here could deadlock. A busy list
 * refuses the rename before the volume is touched. */
static int32_t packet_relabel(void *context, uint32_t phase,
    const uint8_t *name, uint32_t name_length)
{
    struct AfsplusArosHandler *handler = context;
    struct DosLibrary *DOSBase = handler->DOSBase;

    switch (phase)
    {
    case AFSPLUS_AROS_RELABEL_PREPARE:
        if (name_length == 0 || name_length > AFSPLUS_VOLUME_NAME_MAX)
            return ERROR_LINE_TOO_LONG;
        if (AttemptLockDosList(LDF_VOLUMES | LDF_WRITE) == NULL)
            return ERROR_OBJECT_IN_USE;
        return 0;
    case AFSPLUS_AROS_RELABEL_COMMIT:
        write_volume_node_name(handler->volume_node, name, name_length);
        UnLockDosList(LDF_VOLUMES | LDF_WRITE);
        return 0;
    default:
        UnLockDosList(LDF_VOLUMES | LDF_WRITE);
        return 0;
    }
}

static int32_t packet_now(void *context, int64_t *unix_seconds,
    uint32_t *nanoseconds)
{
    struct AfsplusArosHandler *handler = context;
    struct DosLibrary *DOSBase = handler->DOSBase;
    struct DateStamp stamp;
    int64_t amiga_seconds;
    int64_t gmt_offset = 0;

    if (unix_seconds == NULL || nanoseconds == NULL)
        return ERROR_BAD_NUMBER;
    DateStamp(&stamp);
    if (stamp.ds_Days < 0 || stamp.ds_Minute < 0 || stamp.ds_Tick < 0)
        return ERROR_BAD_NUMBER;
    if (handler->locale != NULL)
        gmt_offset = (int64_t)handler->locale->loc_GMTOffset
            * AFSPLUS_SECONDS_PER_MINUTE;
    amiga_seconds = (int64_t)stamp.ds_Days * AFSPLUS_SECONDS_PER_DAY
        + (int64_t)stamp.ds_Minute * AFSPLUS_SECONDS_PER_MINUTE
        + (int64_t)stamp.ds_Tick / AFSPLUS_TICKS_PER_SECOND;
    *unix_seconds = amiga_seconds + AFSPLUS_UNIX_TO_AMIGA_EPOCH
        + gmt_offset;
    *nanoseconds = (uint32_t)(stamp.ds_Tick % AFSPLUS_TICKS_PER_SECOND)
        * UINT32_C(20000000);
    return 0;
}

static uint32_t buffer_matches_mask(const struct AfsplusArosHandler *handler,
    const void *buffer, uint32_t length)
{
    uintptr_t first;
    uintptr_t last;

    if (!handler->use_dma_mask)
        return 1;
    if (buffer == NULL || length == 0)
        return 0;
    first = (uintptr_t)buffer;
    if (first > UINTPTR_MAX - (length - 1))
        return 0;
    last = first + length - 1;
    return ((first | last) & ~handler->dma_mask) == 0;
}

static int32_t device_transfer(void *context, uint32_t command,
    uint64_t byte_offset, void *buffer, uint32_t length, uint32_t writing)
{
    struct AfsplusArosHandler *handler = context;
    struct ExecBase *SysBase = handler->SysBase;
    struct IOExtTD *request = handler->device_request;
    void *io_buffer = buffer;
    LONG result;

    if (request == NULL || buffer == NULL || length == 0
        || command > UINT16_MAX
        || (writing && command != handler->write_command)
        || (!writing && command != handler->read_command))
        return ERROR_BAD_NUMBER;
    if (!buffer_matches_mask(handler, buffer, length))
    {
        if (handler->bounce == NULL || length > handler->bounce_size)
            return ERROR_NO_FREE_STORE;
        io_buffer = handler->bounce;
        if (writing)
            memcpy(io_buffer, buffer, length);
    }

    request->iotd_Req.io_Command = (UWORD)command;
    request->iotd_Req.io_Flags = 0;
    request->iotd_Req.io_Data = io_buffer;
    request->iotd_Req.io_Length = length;
    request->iotd_Req.io_Offset = (ULONG)byte_offset;
    request->iotd_Req.io_Actual = (ULONG)(byte_offset >> 32);
    result = DoIO((struct IORequest *)request);
    if (result != 0 || request->iotd_Req.io_Error != 0
        || request->iotd_Req.io_Actual != length)
        return ERROR_NOT_A_DOS_DISK;
    if (!writing && io_buffer != buffer)
        memcpy(buffer, io_buffer, length);
    return 0;
}

static int32_t device_sync(void *context)
{
    struct AfsplusArosHandler *handler = context;
    struct ExecBase *SysBase = handler->SysBase;
    struct IOExtTD *request = handler->device_request;
    LONG result;

    if (request == NULL)
        return ERROR_BAD_NUMBER;
    request->iotd_Req.io_Command = CMD_UPDATE;
    request->iotd_Req.io_Flags = 0;
    request->iotd_Req.io_Data = NULL;
    request->iotd_Req.io_Length = 0;
    request->iotd_Req.io_Offset = 0;
    request->iotd_Req.io_Actual = 0;
    result = DoIO((struct IORequest *)request);
    return result == 0 && request->iotd_Req.io_Error == 0
        ? 0 : ERROR_NOT_A_DOS_DISK;
}

static uint32_t supports_command(const struct NSDeviceQueryResult *query,
    uint16_t command)
{
    const UWORD *commands;
    uint32_t index;

    if (query == NULL || query->SupportedCommands == NULL)
        return 0;
    commands = query->SupportedCommands;
    for (index = 0; index < AFSPLUS_MAX_NSD_COMMANDS; index++)
    {
        if (commands[index] == command)
            return 1;
        if (commands[index] == 0)
            return 0;
    }
    return 0;
}

static uint32_t probe_one_command(struct AfsplusArosHandler *handler,
    uint16_t command)
{
    struct ExecBase *SysBase = handler->SysBase;
    struct IOExtTD *request = handler->device_request;
    LONG result;

    request->iotd_Req.io_Command = command;
    request->iotd_Req.io_Flags = 0;
    request->iotd_Req.io_Data = NULL;
    request->iotd_Req.io_Length = 0;
    request->iotd_Req.io_Offset = 0;
    request->iotd_Req.io_Actual = 0;
    result = DoIO((struct IORequest *)request);
    return result != IOERR_NOCMD && request->iotd_Req.io_Error != IOERR_NOCMD;
}

static void probe_64bit_commands(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;
    struct NSDeviceQueryResult query;
    struct IOExtTD *request = handler->device_request;
    uint32_t nsd_read;
    uint32_t nsd_write;
    LONG result;

    handler->read_command = CMD_READ;
    handler->write_command = CMD_WRITE;
    handler->supports_64bit_offsets = 0;

    memset(&query, 0, sizeof(query));
    request->iotd_Req.io_Command = NSCMD_DEVICEQUERY;
    request->iotd_Req.io_Flags = 0;
    request->iotd_Req.io_Data = &query;
    request->iotd_Req.io_Length = sizeof(query);
    request->iotd_Req.io_Offset = 0;
    request->iotd_Req.io_Actual = 0;
    result = DoIO((struct IORequest *)request);
    if (result == 0 && request->iotd_Req.io_Error == 0
        && query.DevQueryFormat == 0
        && query.SizeAvailable >= (ULONG)sizeof(query)
        && request->iotd_Req.io_Actual >= (ULONG)sizeof(query))
    {
        nsd_read = supports_command(&query, NSCMD_TD_READ64);
        nsd_write = supports_command(&query, NSCMD_TD_WRITE64);
        if (nsd_read && (handler->read_only || nsd_write))
        {
            handler->read_command = NSCMD_TD_READ64;
            handler->write_command = nsd_write ? NSCMD_TD_WRITE64 : 0;
            handler->supports_64bit_offsets = 1;
            return;
        }
    }

    if (probe_one_command(handler, TD_READ64)
        && (handler->read_only || probe_one_command(handler, TD_WRITE64)))
    {
        handler->read_command = TD_READ64;
        handler->write_command = handler->read_only ? 0 : TD_WRITE64;
        handler->supports_64bit_offsets = 1;
    }
}

static uint32_t device_is_write_protected(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;
    struct IOExtTD *request = handler->device_request;

    request->iotd_Req.io_Command = TD_PROTSTATUS;
    request->iotd_Req.io_Flags = 0;
    request->iotd_Req.io_Data = NULL;
    request->iotd_Req.io_Length = 0;
    request->iotd_Req.io_Offset = 0;
    request->iotd_Req.io_Actual = 0;
    if (DoIO((struct IORequest *)request) != 0
        || request->iotd_Req.io_Error != 0)
        return 0;
    return request->iotd_Req.io_Actual != 0;
}

static int32_t require_media(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;
    struct IOExtTD *request = handler->device_request;
    LONG result;

    request->iotd_Req.io_Command = TD_CHANGESTATE;
    request->iotd_Req.io_Flags = 0;
    request->iotd_Req.io_Data = NULL;
    request->iotd_Req.io_Length = 0;
    request->iotd_Req.io_Offset = 0;
    request->iotd_Req.io_Actual = 0;
    result = DoIO((struct IORequest *)request);
    if (result == IOERR_NOCMD
        || request->iotd_Req.io_Error == IOERR_NOCMD)
        return 0;
    if (result != 0 || request->iotd_Req.io_Error != 0)
        return ERROR_DEVICE_NOT_MOUNTED;
    return request->iotd_Req.io_Actual == 0 ? 0 : ERROR_NO_DISK;
}

static int32_t setup_dma_bounce(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;
    struct DosEnvec *environment = handler->environment;
    ULONG memory_flags = MEMF_PUBLIC | MEMF_CLEAR;

    if ((SIPTR)environment->de_TableSize >= DE_MAXTRANSFER
        && environment->de_MaxTransfer != 0
        && environment->de_MaxTransfer < AFSPLUS_AROS_ALPHA0_BLOCK_SIZE)
        return ERROR_OBJECT_TOO_LARGE;
    if ((SIPTR)environment->de_TableSize < DE_MASK
        || environment->de_Mask == 0)
        return 0;

    handler->use_dma_mask = 1;
    handler->dma_mask = (uintptr_t)environment->de_Mask;
    handler->bounce_size = AFSPLUS_AROS_ALPHA0_BLOCK_SIZE;
    if ((SIPTR)environment->de_TableSize >= DE_BUFMEMTYPE)
        memory_flags |= (ULONG)environment->de_BufMemType;
    handler->bounce = AllocMem(handler->bounce_size, memory_flags);
    if (handler->bounce == NULL)
        return ERROR_NO_FREE_STORE;
    if (!buffer_matches_mask(handler, handler->bounce, handler->bounce_size))
        return ERROR_NO_FREE_STORE;
    return 0;
}

static int32_t open_device(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase = handler->SysBase;
    struct FileSysStartupMsg *startup = handler->startup;

    set_startup_stage(handler, "device-unit");
    if (startup->fssm_Unit > UINT32_MAX)
        return ERROR_BAD_NUMBER;
    set_startup_stage(handler, "device-port");
    handler->device_port = CreateMsgPort();
    if (handler->device_port == NULL)
        return ERROR_NO_FREE_STORE;
    set_startup_stage(handler, "device-request");
    handler->device_request = (struct IOExtTD *)CreateIORequest(
        handler->device_port, sizeof(struct IOExtTD));
    if (handler->device_request == NULL)
        return ERROR_NO_FREE_STORE;
    set_startup_stage(handler, "open-device");
    if (OpenDevice(AROS_BSTR_ADDR(startup->fssm_Device),
            (ULONG)startup->fssm_Unit,
            (struct IORequest *)handler->device_request,
            startup->fssm_Flags) != 0)
        return ERROR_DEVICE_NOT_MOUNTED;
    handler->device_open = 1;
    set_startup_stage(handler, "media-state");
    return require_media(handler);
}

static int32_t setup_filesystem(struct AfsplusArosHandler *handler)
{
    uint8_t volume_name[AFSPLUS_VOLUME_NAME_MAX];
    uint32_t volume_name_length = 0;
    struct AfsplusArosTrackdiskConfig trackdisk_config;
    struct AfsplusArosMountConfig mount_config;
    struct AfsplusArosPacketConfig packet_config;
    struct AfsplusArosDiskInfo disk_info;
    struct ExecBase *SysBase = handler->SysBase;
    struct DosLibrary *DOSBase = handler->DOSBase;
    struct DosEnvec *environment = handler->environment;
    struct DateStamp now;
    uint64_t partition_start;
    uint64_t partition_length;
    uint64_t physical_block_size;
    int32_t error;

    set_startup_stage(handler, "geometry-fields");
    if ((SIPTR)environment->de_TableSize < DE_HIGHCYL
        || (SIPTR)environment->de_SizeBlock <= 0
        || (SIPTR)environment->de_Surfaces <= 0
        || (SIPTR)environment->de_BlocksPerTrack <= 0
        || (SIPTR)environment->de_LowCyl < 0
        || (SIPTR)environment->de_HighCyl < 0)
        return ERROR_BAD_NUMBER;
    set_startup_stage(handler, "geometry-bounds");
    error = afsplus_aros_trackdisk_geometry(
        (uint64_t)environment->de_LowCyl,
        (uint64_t)environment->de_HighCyl,
        (uint64_t)environment->de_Surfaces,
        (uint64_t)environment->de_BlocksPerTrack,
        (uint64_t)environment->de_SizeBlock,
        &partition_start, &partition_length);
    if (error != 0)
        return error;
    physical_block_size = (uint64_t)environment->de_SizeBlock * 4;
    if (physical_block_size > UINT32_MAX)
        return ERROR_OBJECT_TOO_LARGE;

    set_startup_stage(handler, "dma-bounce");
    error = setup_dma_bounce(handler);
    if (error != 0)
        return error;
    handler->read_only = device_is_write_protected(handler);
    probe_64bit_commands(handler);

    memset(&trackdisk_config, 0, sizeof(trackdisk_config));
    trackdisk_config.abi_version = AFSPLUS_AROS_TRACKDISK_ABI_VERSION;
    trackdisk_config.struct_size = sizeof(trackdisk_config);
    trackdisk_config.context = handler;
    trackdisk_config.transfer = device_transfer;
    trackdisk_config.sync = handler->read_only ? NULL : device_sync;
    trackdisk_config.partition_start_bytes = partition_start;
    trackdisk_config.partition_length_bytes = partition_length;
    trackdisk_config.device_block_size = (uint32_t)physical_block_size;
    trackdisk_config.logical_block_size = AFSPLUS_AROS_ALPHA0_BLOCK_SIZE;
    trackdisk_config.read_command = handler->read_command;
    trackdisk_config.write_command = handler->write_command;
    trackdisk_config.supports_64bit_offsets =
        handler->supports_64bit_offsets;
    trackdisk_config.read_only = handler->read_only;
    set_startup_stage(handler, "trackdisk-adapter");
    error = afsplus_aros_trackdisk_init(&trackdisk_config,
        &handler->trackdisk, &handler->device);
    if (error != 0)
        return error;

    /* The DOSDriver Control string, the only place a mountlist can name a
     * policy. A string this handler does not understand fails the mount:
     * one that looks applied and is not would be worse than none. */
    set_startup_stage(handler, "control-string");
    {
        struct AfsplusArosControl control;
        const char *text = NULL;
        uint32_t length = 0;
        uint32_t result;

        if ((SIPTR)handler->environment->de_TableSize >= DE_CONTROL
            && handler->environment->de_Control != 0)
        {
            BSTR value = (BSTR)handler->environment->de_Control;

            text = (const char *)AROS_BSTR_ADDR(value);
#ifdef AROS_FAST_BSTR
            length = (uint32_t)strlen(text);
#else
            length = (uint32_t)AROS_BSTR_strlen(value);
#endif
        }
        result = afsplus_control_parse(text, length, &control);
        if (result != AFSPLUS_CONTROL_OK)
        {
            bug("[AFSPLUS] Control string refused (reason %u): %s\n",
                (unsigned)result, text != NULL ? text : "");
            return ERROR_BAD_NUMBER;
        }
        handler->mount_flags = control.mount_flags;
        handler->name_encoding = control.name_encoding;
        handler->trace_capacity = control.trace_events;
    }

    memset(&mount_config, 0, sizeof(mount_config));
    mount_config.abi_version = AFSPLUS_AROS_ABI_VERSION;
    mount_config.struct_size = sizeof(mount_config);
    mount_config.mount_mode = handler->read_only
        ? AFSPLUS_AROS_MOUNT_READ_ONLY : AFSPLUS_AROS_MOUNT_READ_WRITE;
    mount_config.name_encoding = handler->name_encoding;
    mount_config.flags = handler->mount_flags;
    /* No name: the volume is named after its committed label, so a renamed
     * volume comes back under its new name. */
    mount_config.volume_name = NULL;
    mount_config.volume_name_length = 0;
    mount_config.max_file_handles = 1024;
    mount_config.max_locks = 1024;
    mount_config.max_file_info_name_bytes = 107;
    set_startup_stage(handler, "rust-mount");
#if defined(AFSPLUS_AROS_DIAGNOSTIC_STOP_BEFORE_RUST_MOUNT)
    return ERROR_NOT_IMPLEMENTED;
#endif
    error = afsplus_aros_mount(&handler->device, &mount_config,
        &handler->filesystem);
    if (error != 0)
        return error;

    /* The ring exists before the sink can write into it, and the sink is
     * attached only once both are ready. A mount that asked for tracing and
     * cannot have it fails rather than running untraced: the caller asked. */
    if (handler->trace_capacity != 0)
    {
        struct afsp_trace_sink sink;

        set_startup_stage(handler, "trace-ring");
        handler->trace_ring = AllocMem(
            (ULONG)handler->trace_capacity * sizeof(*handler->trace_ring),
            MEMF_PUBLIC | MEMF_CLEAR);
        if (handler->trace_ring == NULL)
            return ERROR_NO_FREE_STORE;
        memset(&sink, 0, sizeof(sink));
        sink.emit = afsplus_aros_trace_emit;
        sink.ctx = handler;
        sink.category_mask = UINT64_MAX;
        error = afsplus_aros_set_trace_sink(handler->filesystem, &sink);
        if (error != 0)
            return error;
    }

    /* The DOSDriver's Buffers become the read cache, in device blocks, as
     * the classic file systems take them; AddBuffers resizes it later
     * (ACTION_MORE_CACHE). */
    set_startup_stage(handler, "read-cache");
    {
        uint32_t granted = 0;
        uint32_t buffers = (SIPTR)handler->environment->de_TableSize
                >= DE_NUMBUFFERS
                && (SIPTR)handler->environment->de_NumBuffers > 0
            ? (uint32_t)handler->environment->de_NumBuffers : 0;

        error = afsplus_aros_set_cache_blocks(handler->filesystem, buffers,
            &granted);
        if (error != 0)
            return error;
    }

    set_startup_stage(handler, "disk-info");
    error = afsplus_aros_disk_info(handler->filesystem, &disk_info);
    if (error != 0)
        return error;
    set_startup_stage(handler, "volume-entry");
    /* Locks carry a pointer to this node, so ACTION_RENAME_DISK renames it
     * in place. MakeDosEntry sizes the name buffer for the name it is given:
     * create the node with the longest name a rename may bring, then write
     * the mount-time name into that buffer. */
    {
        char longest[AFSPLUS_VOLUME_NAME_MAX + 1];

        memset(longest, 'x', AFSPLUS_VOLUME_NAME_MAX);
        longest[AFSPLUS_VOLUME_NAME_MAX] = 0;
        handler->volume_node = MakeDosEntry((STRPTR)longest, DLT_VOLUME);
    }
    if (handler->volume_node == NULL)
        return ERROR_NO_FREE_STORE;
    /* The DOS volume name is the label. A volume without a usable label
     * answers to the name of its device node instead. */
    if (afsplus_aros_volume_label(handler->filesystem, volume_name,
            sizeof(volume_name), &volume_name_length) != 0
        || volume_name_length == 0
        || volume_name_length > sizeof(volume_name))
    {
        const uint8_t *device_name = (const uint8_t *)AROS_BSTR_ADDR(
            handler->device_node->dol_Name);

#ifdef AROS_FAST_BSTR
        volume_name_length = 0;
        while (volume_name_length < sizeof(volume_name)
            && device_name[volume_name_length] != 0)
            volume_name_length++;
#else
        /* A length-prefixed BSTR is not required to carry a terminator. */
        volume_name_length = device_name[-1];
        if (volume_name_length > sizeof(volume_name))
            volume_name_length = sizeof(volume_name);
#endif
        memcpy(volume_name, device_name, volume_name_length);
    }
    write_volume_node_name(handler->volume_node, volume_name,
        volume_name_length);
    handler->volume_node->dol_Task = handler->handler_port;
    handler->volume_node->dol_misc.dol_volume.dol_DiskType =
        (ULONG)disk_info.disk_type;
    DateStamp(&now);
    handler->volume_node->dol_misc.dol_volume.dol_VolumeDate = now;
    set_startup_stage(handler, "volume-register");
    if (!AddDosEntry(handler->volume_node))
        return IoErr() != 0 ? (int32_t)IoErr() : ERROR_OBJECT_EXISTS;
    handler->volume_registered = 1;

    memset(&packet_config, 0, sizeof(packet_config));
    packet_config.abi_version = AFSPLUS_AROS_PACKET_ABI_VERSION;
    packet_config.struct_size = sizeof(packet_config);
    packet_config.filesystem = handler->filesystem;
    packet_config.handler_port = handler->handler_port;
    packet_config.volume_node = MKBADDR(handler->volume_node);
    packet_config.callback_context = handler;
    packet_config.allocate = packet_allocate;
    packet_config.free = packet_free;
    packet_config.now = packet_now;
    handler->notify_port = CreateMsgPort();
    if (handler->notify_port == NULL)
        return ERROR_NO_FREE_STORE;
    packet_config.notify = packet_notify;
    packet_config.relabel = packet_relabel;
    if (handler->trace_ring != NULL)
        packet_config.trace_take = packet_trace_take;
    open_wait_timer(handler);
    if (handler->timer_open)
        packet_config.complete = packet_complete;
    set_startup_stage(handler, "packet-context");
    error = afsplus_aros_packet_create(&packet_config, &handler->packets);
    if (error == 0)
        set_startup_stage(handler, "ready");
    return error;
}

static void cleanup_handler(struct AfsplusArosHandler *handler)
{
    struct ExecBase *SysBase;
    struct DosLibrary *DOSBase;
    struct LocaleBase *LocaleBase;

    if (handler == NULL)
        return;
    SysBase = handler->SysBase;
    DOSBase = handler->DOSBase;
    LocaleBase = handler->LocaleBase;

    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000001));
    if (handler->packets != NULL)
    {
        (void)afsplus_aros_packet_destroy(handler->packets);
        handler->packets = NULL;
    }
    close_wait_timer(handler);
    /* A NotifyMessage can stay out for good: EndNotify takes back only the
     * messages still queued at the application, and one already fetched by
     * an application that crashed or never replies is never returned.
     * Refusing to die for it would make the volume undismountable, and
     * deleting the port would let a late ReplyMsg write into freed memory.
     * So the port, its signal bit and the message are left behind on
     * purpose in that case; they cost a few dozen bytes once. */
    collect_notify_replies(handler);
    if (handler->notify_port != NULL)
    {
        if (handler->notify_outstanding == 0)
            DeleteMsgPort(handler->notify_port);
        else
            /* The port outlives this task: a late reply must queue without
             * signalling a task that no longer exists. */
            handler->notify_port->mp_Flags = PA_IGNORE;
        handler->notify_port = NULL;
    }
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000002));
    if (handler->volume_registered)
    {
        RemDosEntry(handler->volume_node);
        handler->volume_registered = 0;
    }
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000003));
    if (handler->volume_node != NULL)
    {
        FreeDosEntry(handler->volume_node);
        handler->volume_node = NULL;
    }
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000004));
    if (handler->filesystem != NULL)
    {
        /* Detached before the ring goes: the library must not hold a
         * callback into memory this is about to free. */
        if (handler->trace_ring != NULL)
            (void)afsplus_aros_set_trace_sink(handler->filesystem, NULL);
        (void)afsplus_aros_unmount(handler->filesystem);
        handler->filesystem = NULL;
    }
    if (handler->trace_ring != NULL)
    {
        FreeMem(handler->trace_ring,
            (ULONG)handler->trace_capacity * sizeof(*handler->trace_ring));
        handler->trace_ring = NULL;
    }
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000005));
    if (handler->device_open)
    {
        CloseDevice((struct IORequest *)handler->device_request);
        handler->device_open = 0;
    }
    if (handler->device_request != NULL)
        DeleteIORequest((struct IORequest *)handler->device_request);
    if (handler->device_port != NULL)
        DeleteMsgPort(handler->device_port);
    if (handler->bounce != NULL)
        FreeMem(handler->bounce, handler->bounce_size);
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000006));
    if (handler->device_node != NULL)
    {
        if (handler->device_node->dol_Task == handler->handler_port)
            handler->device_node->dol_Task = NULL;
    }
    if (handler->locale != NULL)
        CloseLocale(handler->locale);
    if (handler->LocaleBase != NULL)
        CloseLibrary((struct Library *)handler->LocaleBase);
    if (handler->DOSBase != NULL)
        CloseLibrary((struct Library *)handler->DOSBase);
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000007));
    FreeMem(handler, sizeof(*handler));

    (void)DOSBase;
    (void)LocaleBase;
}

static struct AfsplusArosHandler *initialize_handler(struct ExecBase *SysBase,
    struct Process *process, struct DosPacket *startup_packet, int32_t *error)
{
    struct AfsplusArosHandler *handler;
    struct FileSysStartupMsg *startup;

    handler = AllocMem(sizeof(*handler), MEMF_PUBLIC | MEMF_CLEAR);
    if (handler == NULL)
    {
        *error = ERROR_NO_FREE_STORE;
        return NULL;
    }
    handler->SysBase = SysBase;
    set_startup_stage(handler, "startup-message");
    handler->process = process;
    handler->handler_port = &process->pr_MsgPort;
    handler->device_node = (struct DosList *)BADDR(startup_packet->dp_Arg3);
    startup = (struct FileSysStartupMsg *)BADDR(startup_packet->dp_Arg2);
    handler->startup = startup;
    if (handler->device_node == NULL || startup == NULL
        || startup->fssm_Device == BNULL || startup->fssm_Environ == BNULL)
    {
        *error = ERROR_BAD_NUMBER;
        return handler;
    }
    handler->environment = (struct DosEnvec *)BADDR(startup->fssm_Environ);
    if (handler->environment == NULL)
    {
        *error = ERROR_BAD_NUMBER;
        return handler;
    }

    set_startup_stage(handler, "dos-library");
    handler->DOSBase = (struct DosLibrary *)TaggedOpenLibrary(TAGGEDOPEN_DOS);
    if (handler->DOSBase == NULL)
    {
        *error = ERROR_NO_FREE_STORE;
        return handler;
    }
    handler->LocaleBase = (struct LocaleBase *)OpenLibrary(
        (CONST_STRPTR)"locale.library", 38);
    if (handler->LocaleBase != NULL)
    {
        struct LocaleBase *LocaleBase = handler->LocaleBase;
        handler->locale = OpenLocale(NULL);
    }

    *error = open_device(handler);
    if (*error == 0)
        *error = setup_filesystem(handler);
    return handler;
}

/* Answers what is still queued on a port nobody will serve again. */
static void refuse_queued_packets(struct ExecBase *SysBase,
    struct MsgPort *port)
{
    struct Message *message;

    while ((message = GetMsg(port)) != NULL)
    {
        struct DosPacket *packet =
            (struct DosPacket *)message->mn_Node.ln_Name;

        if (packet == NULL)
            continue;
        packet->dp_Res1 = DOSFALSE;
        packet->dp_Res2 = ERROR_DEVICE_NOT_MOUNTED;
        reply_packet(port, SysBase, packet);
    }
}

/*
 * The trace sink, from a target.
 *
 * The core hands every recorded event to a callback that runs inside a
 * filesystem operation on this task; it may not block, call back into the
 * library or unwind. So it does one thing: it copies the event into a ring
 * this handler preallocated, and counts what it had to drop. A tool drains
 * the ring through the extension packet, which runs between operations.
 *
 * The event's timestamp is filled here. The core has no clock, so it leaves
 * the field zero; the handler has one, and a stamp taken when the event is
 * recorded is the closest a caller can get. It costs a DateStamp per event,
 * which is why the ring exists only when the mount asked for it.
 */
void afsplus_aros_trace_emit(void *context,
    const struct afsp_trace_event *event)
{
    struct AfsplusArosHandler *handler = context;
    struct afsp_trace_event *slot;
    int64_t seconds = 0;
    uint32_t nanoseconds = 0;

    if (handler->trace_ring == NULL || handler->trace_capacity == 0)
        return;
    if (handler->trace_count == handler->trace_capacity)
    {
        /* The oldest event goes: a tool that came late wants what the
         * filesystem did last, and the count says what it missed. */
        handler->trace_dropped++;
        handler->trace_next = (handler->trace_next + 1)
            % handler->trace_capacity;
        handler->trace_count--;
    }
    slot = &handler->trace_ring[(handler->trace_next + handler->trace_count)
        % handler->trace_capacity];
    *slot = *event;
    if (packet_now(handler, &seconds, &nanoseconds) == 0)
        slot->timestamp = (uint64_t)seconds * UINT64_C(1000000000)
            + nanoseconds;
    handler->trace_count++;
}

/* Takes up to capacity events, oldest first, and says how many the ring has
 * dropped since the mount. */
uint32_t afsplus_aros_trace_take(struct AfsplusArosHandler *handler,
    struct afsp_trace_event *events, uint32_t capacity, uint64_t *dropped)
{
    uint32_t taken = 0;

    *dropped = handler->trace_dropped;
    while (taken < capacity && handler->trace_count != 0)
    {
        events[taken++] = handler->trace_ring[handler->trace_next];
        handler->trace_next = (handler->trace_next + 1)
            % handler->trace_capacity;
        handler->trace_count--;
    }
    return taken;
}

/* afsplus_claim.h: the identity of a task, and whether a recorded task is
 * still that task. Called under Forbid(), which is what keeps the lists
 * still. A task that crashed or ended is on neither list; the running task
 * is on neither as well. The address alone is not an identity: exec reissues
 * it, so the unique task ID has to match too. */
uint32_t afsplus_claim_task_id(struct ExecBase *SysBase,
    const struct Task *task)
{
    (void)SysBase;
    if (task == NULL || !(task->tc_Flags & TF_ETASK)
        || task->tc_UnionETask.tc_ETask == NULL)
        return 0;
    return (uint32_t)task->tc_UnionETask.tc_ETask->et_UniqueID;
}

uint32_t afsplus_claim_task_alive(struct ExecBase *SysBase,
    const struct Task *task, uint32_t task_id)
{
    const struct Node *node;
    uint32_t listed = 0;

    if (task == NULL)
        return 0;
    if (task == SysBase->ThisTask)
        listed = 1;
    for (node = SysBase->TaskReady.lh_Head; !listed && node->ln_Succ != NULL;
        node = node->ln_Succ)
        listed = (const struct Task *)node == task;
    for (node = SysBase->TaskWait.lh_Head; !listed && node->ln_Succ != NULL;
        node = node->ln_Succ)
        listed = (const struct Task *)node == task;
    /* Only now is task known to be a task, and safe to read. */
    return listed && task->tc_Node.ln_Type == NT_PROCESS
        && afsplus_claim_task_id(SysBase, task) == task_id;
}

/* The life of an instance that found its medium claimed: see
 * afsplus_claim.h. A packet is not touched again once it has been forwarded;
 * it belongs to the other instance then. The forwarder ends with the
 * instance it serves, after answering what is still queued. */
static LONG forward_to_claimed_instance(struct ExecBase *SysBase,
    struct MsgPort *port, const char *claim_name, uint64_t generation)
{
    struct Task *self = FindTask(NULL);
    struct Message *message;
    uint32_t serving;

    /* Enrolled so that the instance wakes this task when it goes. The answer
     * says whether that instance is still there, slot or no slot. */
    serving = afsplus_claim_enroll(SysBase, claim_name, generation, self, 1);
    while (serving)
    {
        Wait((1UL << port->mp_SigBit) | AFSPLUS_CLAIM_WAKE_SIGNAL);
        /* Woken by the instance's release, or by a packet: either way the
         * instance it serves may be gone. */
        serving = afsplus_claim_enroll(SysBase, claim_name, generation, self,
            1);
        while (serving && (message = GetMsg(port)) != NULL)
        {
            struct DosPacket *packet =
                (struct DosPacket *)message->mn_Node.ln_Name;

            if (packet == NULL)
                continue;
            if (!afsplus_claim_forward(SysBase, claim_name, generation,
                    message))
            {
                packet->dp_Res1 = DOSFALSE;
                packet->dp_Res2 = ERROR_DEVICE_NOT_MOUNTED;
                reply_packet(port, SysBase, packet);
                serving = 0;
            }
        }
    }
    (void)afsplus_claim_enroll(SysBase, claim_name, generation, self, 0);
    refuse_queued_packets(SysBase, port);
    return RETURN_OK;
}

/*
 * A handler that cannot open a library must fail its mount, not hang.
 *
 * The generated entry opens the libraries this module was linked against
 * before it calls handler(). autoinit reports a failure through
 * ___showerror(), which for a process without a console is a requester that
 * waits for a click, and the entry then returned without ever answering the
 * startup packet: the task that made the first access waited for ever,
 * behind a requester or not. This definition replaces autoinit's for the
 * module, since the module's objects are linked before the archive, and
 * writes to the debug log. afsplus-handler-autolibs.patch makes the entry
 * call afsplus_aros_refuse_startup() when the libraries or the init set
 * failed.
 */
void ___showerror(struct ExecBase *SysBase, const char *format, ...)
{
    va_list arguments;

    (void)SysBase;
    va_start(arguments, format);
    bug("[AFSPLUS] startup: ");
    vkprintf(format, arguments);
    bug("\n");
    va_end(arguments);
}

LONG afsplus_aros_refuse_startup(struct ExecBase *SysBase)
{
    struct Process *process = (struct Process *)FindTask(NULL);
    struct MsgPort *port = &process->pr_MsgPort;
    struct Message *message;
    struct DosPacket *packet;

    WaitPort(port);
    message = GetMsg(port);
    if (message == NULL || message->mn_Node.ln_Name == NULL)
        return RETURN_FAIL;
    packet = (struct DosPacket *)message->mn_Node.ln_Name;
    packet->dp_Res1 = DOSFALSE;
    packet->dp_Res2 = ERROR_INVALID_RESIDENT_LIBRARY;
    reply_packet(port, SysBase, packet);
    return RETURN_FAIL;
}

LONG handler(struct ExecBase *SysBase)
{
    struct AfsplusArosHandler *state;
    struct Process *process;
    struct MsgPort *port;
    struct Message *message;
    struct DosPacket *packet;
    struct DosPacket *death_packet = NULL;
    struct AfsplusArosClaim *claim = NULL;
    struct FileSysStartupMsg *claimed_startup;
    char claim_name[AFSPLUS_CLAIM_NAME_BYTES];
    uint64_t held_generation = 0;
    uint32_t claim_result;
    int32_t error = ERROR_NO_FREE_STORE;
    uint32_t quit = 0;

    afsplus_aros_startup_trace(SysBase, UINT32_C(0x30100001));
    process = (struct Process *)FindTask(NULL);
    port = &process->pr_MsgPort;
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x30100002));
    WaitPort(port);
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x30100003));
    message = GetMsg(port);
    if (message == NULL || message->mn_Node.ln_Name == NULL)
        return RETURN_FAIL;
    packet = (struct DosPacket *)message->mn_Node.ln_Name;

    /* The medium is claimed before anything opens it. A startup message
     * that names no device or no partition is left to initialize_handler()
     * to refuse: nothing is opened for it. */
    claimed_startup = (struct FileSysStartupMsg *)BADDR(packet->dp_Arg2);
    if (claimed_startup != NULL && (IPTR)packet->dp_Arg2 >= 64
        && claimed_startup->fssm_Device != BNULL
        && claimed_startup->fssm_Environ != BNULL
        && ((struct DosEnvec *)BADDR(claimed_startup->fssm_Environ))
            ->de_TableSize >= DE_UPPERCYL)
    {
        const struct DosEnvec *environment =
            (const struct DosEnvec *)BADDR(claimed_startup->fssm_Environ);
        struct AfsplusArosClaimKey key;

        key.device = (const uint8_t *)AROS_BSTR_ADDR(
            claimed_startup->fssm_Device);
        /* The length the BSTR form of this build states, as
         * afsplus_packet.c reads it. */
#ifdef AROS_FAST_BSTR
        key.device_length = (uint32_t)strlen((const char *)key.device);
#else
        key.device_length = (uint32_t)AROS_BSTR_strlen(
            claimed_startup->fssm_Device);
#endif
        key.unit = (uint64_t)claimed_startup->fssm_Unit;
        key.flags = (uint64_t)claimed_startup->fssm_Flags;
        key.size_block = (uint64_t)environment->de_SizeBlock;
        key.surfaces = (uint64_t)environment->de_Surfaces;
        key.blocks_per_track = (uint64_t)environment->de_BlocksPerTrack;
        key.low_cylinder = (uint64_t)environment->de_LowCyl;
        key.high_cylinder = (uint64_t)environment->de_HighCyl;
        if (!afsplus_claim_name(claim_name, &key))
            claim_result = AFSPLUS_CLAIM_NAME_TOO_LONG;
        else
            claim_result = afsplus_claim_take(SysBase, claim_name,
                &process->pr_Task, port, &claim, &held_generation);
        if (claim_result == AFSPLUS_CLAIM_ALREADY_HELD)
        {
            bug("[AFSPLUS] %s is served by another instance; forwarding\n",
                claim_name);
            packet->dp_Res1 = DOSTRUE;
            packet->dp_Res2 = 0;
            reply_packet(port, SysBase, packet);
            return forward_to_claimed_instance(SysBase, port, claim_name,
                held_generation);
        }
        if (claim_result != AFSPLUS_CLAIM_TAKEN)
        {
            packet->dp_Res1 = DOSFALSE;
            packet->dp_Res2 = claim_result == AFSPLUS_CLAIM_NO_MEMORY
                ? ERROR_NO_FREE_STORE : ERROR_OBJECT_TOO_LARGE;
            reply_packet(port, SysBase, packet);
            return RETURN_FAIL;
        }
    }

    state = initialize_handler(SysBase, process, packet, &error);
    if (state == NULL || error != 0)
    {
#if AFSPLUS_AROS_TRACE_STARTUP
        afsplus_aros_startup_trace(SysBase,
            UINT32_C(0x3f000000) | ((uint32_t)error & UINT32_C(0x00ffffff)));
#else
        bug("[AFSPLUS] mount failed at %s: error %d\n",
            state != NULL ? state->startup_stage : "handler-allocation",
            (int)error);
#endif
        packet->dp_Res1 = DOSFALSE;
        packet->dp_Res2 = error;
        if (state != NULL)
            cleanup_handler(state);
        /* No forwarder may find this instance any more, and what one has
         * already queued here gets an answer. */
        afsplus_claim_release(SysBase, claim);
        refuse_queued_packets(SysBase, port);
        reply_packet(port, SysBase, packet);
        return RETURN_FAIL;
    }

    afsplus_aros_startup_trace(SysBase, UINT32_C(0x30200001));
    state->device_node->dol_Task = port;
    packet->dp_Res1 = DOSTRUE;
    packet->dp_Res2 = 0;
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x30200002));
    reply_packet(port, SysBase, packet);
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x30200003));

    while (!quit)
    {
        afsplus_aros_startup_trace(SysBase, UINT32_C(0x30200004));
        Wait((1UL << port->mp_SigBit)
            | (1UL << state->notify_port->mp_SigBit)
            | (state->timer_open
                ? 1UL << state->timer_port->mp_SigBit : 0));
        afsplus_aros_startup_trace(SysBase, UINT32_C(0x30200005));
        collect_notify_replies(state);
        while (!quit && (message = GetMsg(port)) != NULL)
        {
            packet = (struct DosPacket *)message->mn_Node.ln_Name;
            if (packet == NULL)
                continue;
            if (packet->dp_Type == ACTION_DIE)
                collect_notify_replies(state);
            afsplus_aros_startup_trace(SysBase, UINT32_C(0x40000000) |
                ((uint32_t)packet->dp_Type & UINT32_C(0x0fffffff)));
            error = afsplus_aros_packet_process(state->packets, packet);
            /* Kept by the packet layer: packet_complete replies later. */
            if (error == AFSPLUS_AROS_PACKET_DEFERRED)
                continue;
            if (error != 0)
                afsplus_aros_startup_trace(SysBase, UINT32_C(0x50000000) |
                    ((uint32_t)error & UINT32_C(0x0fffffff)));
            if (error != 0)
            {
                packet->dp_Res1 = DOSFALSE;
                packet->dp_Res2 = error;
            }
            quit = afsplus_aros_packet_should_quit(state->packets);
            if (quit)
            {
                state->device_node->dol_Task = NULL;
                death_packet = packet;
            }
            else
                reply_packet(port, SysBase, packet);
        }
        if (!quit)
            run_wait_timer(state);
    }

    /* Packets that queued up behind ACTION_DIE would wait for ever on a port
     * nobody serves. No lock or file exists at this point, so each of them
     * names the volume only, and the volume is going away. The claim goes
     * first, so that no forwarder queues another one behind this sweep. */
    afsplus_claim_release(SysBase, claim);
    refuse_queued_packets(SysBase, port);

    /* Keep the ACTION_DIE sender blocked until every handler-owned reference
     * to the device node and backing device is gone. The caller retains the
     * DeviceNode: Mount SHUTDOWN must leave it available for a subsequent
     * Assign DISMOUNT, and a later access may restart this handler from it. */
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000000));
    cleanup_handler(state);
    afsplus_aros_startup_trace(SysBase, UINT32_C(0x60000008));
    if (death_packet != NULL)
        reply_packet(port, SysBase, death_packet);
    return RETURN_OK;
}
