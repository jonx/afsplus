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
#include <stddef.h>
#include <stdint.h>

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
    uint8_t *bounce;
    uintptr_t dma_mask;
    uint32_t bounce_size;
    uint32_t use_dma_mask;
    uint32_t device_open;
    uint32_t volume_registered;
    uint32_t read_only;
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
    static const uint8_t volume_name[] = { 'A', 'F', 'S', '+' };
    struct AfsplusArosTrackdiskConfig trackdisk_config;
    struct AfsplusArosMountConfig mount_config;
    struct AfsplusArosPacketConfig packet_config;
    struct AfsplusArosDiskInfo disk_info;
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

    memset(&mount_config, 0, sizeof(mount_config));
    mount_config.abi_version = AFSPLUS_AROS_ABI_VERSION;
    mount_config.struct_size = sizeof(mount_config);
    mount_config.mount_mode = handler->read_only
        ? AFSPLUS_AROS_MOUNT_READ_ONLY : AFSPLUS_AROS_MOUNT_READ_WRITE;
    mount_config.name_encoding = AFSPLUS_AROS_ENCODING_UTF8;
    mount_config.volume_name = volume_name;
    mount_config.volume_name_length = sizeof(volume_name);
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
    write_volume_node_name(handler->volume_node, volume_name,
        sizeof(volume_name));
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
        (void)afsplus_aros_unmount(handler->filesystem);
        handler->filesystem = NULL;
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

LONG handler(struct ExecBase *SysBase)
{
    struct AfsplusArosHandler *state;
    struct Process *process;
    struct MsgPort *port;
    struct Message *message;
    struct DosPacket *packet;
    struct DosPacket *death_packet = NULL;
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
            | (1UL << state->notify_port->mp_SigBit));
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
    }

    /* Packets that queued up behind ACTION_DIE would wait for ever on a port
     * nobody serves. No lock or file exists at this point, so each of them
     * names the volume only, and the volume is going away. */
    while ((message = GetMsg(port)) != NULL)
    {
        packet = (struct DosPacket *)message->mn_Node.ln_Name;
        if (packet == NULL)
            continue;
        packet->dp_Res1 = DOSFALSE;
        packet->dp_Res2 = ERROR_DEVICE_NOT_MOUNTED;
        reply_packet(port, SysBase, packet);
    }

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
