/* SPDX-License-Identifier: BSD-2-Clause */

#include <aros/apple/startup.h>
#include <aros/asmcall.h>
#include <aros/libcall.h>
#include <aros/kernel.h>
#include <devices/trackdisk.h>
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <dos/filehandler.h>
#include <exec/io.h>
#include <exec/memory.h>
#include <exec/resident.h>
#include <proto/arossupport.h>
#include <proto/dos.h>
#include <proto/exec.h>
#include <proto/expansion.h>
#include <proto/kernel.h>
#include <workbench/startup.h>

#include <stddef.h>
#include <stdint.h>

#include "afsram_format.h"

int __nowbsupport = 1;
int __nostdiowin = 1;
struct WBStartup *WBenchMsg;
static BPTR loaded_device_seglist;
static struct Device *loaded_device;

static int string_equal(const char *left, const char *right)
{
    if (left == NULL || right == NULL)
        return 0;
    while (*left != '\0' && *left == *right) {
        ++left;
        ++right;
    }
    return *left == *right;
}

static int bytes_equal(const UBYTE *left, const UBYTE *right, ULONG size)
{
    ULONG index;

    for (index = 0; index < size; ++index) {
        if (left[index] != right[index])
            return 0;
    }
    return 1;
}

static struct Resident *find_seglist_resident(BPTR seglist)
{
    const ULONG resident_size =
        (ULONG)(offsetof(struct Resident, rt_Init) + sizeof(APTR));

    while (seglist != BNULL) {
        UBYTE *address = (UBYTE *)BADDR(seglist) - sizeof(ULONG);
        ULONG size = *(ULONG *)address;

        if (size >= sizeof(BPTR) + sizeof(ULONG)) {
            address += sizeof(BPTR) + sizeof(ULONG);
            size -= sizeof(BPTR) + sizeof(ULONG);
            while (size >= resident_size) {
                struct Resident *resident = (struct Resident *)address;

                if (resident->rt_MatchWord == RTC_MATCHWORD &&
                    resident->rt_MatchTag == resident)
                    return resident;
                address += 2;
                size -= 2;
            }
        }
        seglist = *(BPTR *)BADDR(seglist);
    }
    return NULL;
}

static int load_device(void)
{
    BPTR seglist = LoadSeg("DEVS:afsram.device");
    struct Resident *resident;
    APTR initialized;

    if (seglist == BNULL)
        return 55;
    resident = find_seglist_resident(seglist);
    if (resident == NULL) {
        UnLoadSeg(seglist);
        return 56;
    }
    if (!string_equal(resident->rt_Name, "afsram.device")) {
        UnLoadSeg(seglist);
        return 57;
    }
    Forbid();
    initialized = InitResident(resident, seglist);
    Permit();
    if (initialized == NULL) {
        UnLoadSeg(seglist);
        return 58;
    }
    loaded_device_seglist = seglist;
    return 0;
}

static int unload_device(struct Device *device)
{
    BPTR seglist;

    seglist = AROS_LVO_CALL1(BPTR,
        AROS_LCA(struct Device *, device, D0),
        struct Device *, device, 3,
    );
    if (seglist == BNULL || seglist != loaded_device_seglist)
        return 69;
    UnLoadSeg(seglist);
    loaded_device_seglist = BNULL;
    return 0;
}

static int probe_device(void)
{
    struct MsgPort *port = NULL;
    struct IOExtTD *request = NULL;
    struct DriveGeometry geometry;
    UBYTE first[512];
    UBYTE second[512];
    int opened = 0;
    int result = 0;
    BPTR device_file;

    device_file = Lock("DEVS:afsram.device", SHARED_LOCK);
    if (device_file == BNULL)
        return 59;
    UnLock(device_file);
    result = load_device();
    if (result != 0)
        return result;

    port = CreateMsgPort();
    if (port == NULL)
        return 60;
    request = (struct IOExtTD *)CreateIORequest(port, sizeof(*request));
    if (request == NULL) {
        result = 61;
        goto out;
    }
    if (OpenDevice("afsram.device", 0,
            (struct IORequest *)request, 0) != 0) {
        result = 62;
        goto out;
    }
    opened = 1;
    loaded_device = request->iotd_Req.io_Device;

    request->iotd_Req.io_Command = TD_PROTSTATUS;
    request->iotd_Req.io_Data = NULL;
    request->iotd_Req.io_Length = 0;
    request->iotd_Req.io_Offset = 0;
    if (DoIO((struct IORequest *)request) != 0 ||
        request->iotd_Req.io_Actual != 0) {
        result = 63;
        goto out;
    }

    request->iotd_Req.io_Command = CMD_READ;
    request->iotd_Req.io_Data = first;
    request->iotd_Req.io_Length = sizeof(first);
    request->iotd_Req.io_Offset = 0;
    if (DoIO((struct IORequest *)request) != 0 ||
        request->iotd_Req.io_Actual != sizeof(first) ||
        first[0] != 'A' || first[1] != 'F' ||
        first[2] != 'S' || first[3] != 'I') {
        result = 64;
        goto out;
    }

    request->iotd_Req.io_Command = CMD_WRITE;
    request->iotd_Req.io_Data = first;
    request->iotd_Req.io_Length = sizeof(first);
    request->iotd_Req.io_Offset = 0;
    if (DoIO((struct IORequest *)request) != 0 ||
        request->iotd_Req.io_Actual != sizeof(first)) {
        result = 65;
        goto out;
    }
    request->iotd_Req.io_Command = CMD_UPDATE;
    request->iotd_Req.io_Data = NULL;
    request->iotd_Req.io_Length = 0;
    request->iotd_Req.io_Offset = 0;
    if (DoIO((struct IORequest *)request) != 0) {
        result = 66;
        goto out;
    }

    request->iotd_Req.io_Command = CMD_READ;
    request->iotd_Req.io_Data = second;
    request->iotd_Req.io_Length = sizeof(second);
    request->iotd_Req.io_Offset = 0;
    if (DoIO((struct IORequest *)request) != 0 ||
        request->iotd_Req.io_Actual != sizeof(second) ||
        !bytes_equal(first, second, sizeof(first))) {
        result = 67;
        goto out;
    }

    request->iotd_Req.io_Command = TD_GETGEOMETRY;
    request->iotd_Req.io_Data = &geometry;
    request->iotd_Req.io_Length = sizeof(geometry);
    request->iotd_Req.io_Offset = 0;
    if (DoIO((struct IORequest *)request) != 0 ||
        request->iotd_Req.io_Actual != sizeof(geometry) ||
        geometry.dg_SectorSize != 512 ||
        geometry.dg_TotalSectors != 131072) {
        result = 68;
        goto out;
    }

out:
    if (opened)
        CloseDevice((struct IORequest *)request);
    if (request != NULL)
        DeleteIORequest((struct IORequest *)request);
    DeleteMsgPort(port);
    if (result != 0 && loaded_device != NULL) {
        (void)unload_device(loaded_device);
        loaded_device = NULL;
    }
    return result;
}

#ifndef AFSPLUS_AFSRAM_BLOCK_ONLY
struct RetainedHandlerFile {
    const UBYTE *data;
    ULONG size;
    ULONG position;
};

#ifndef AFSPLUS_AFSRAM_TRACE
#define AFSPLUS_AFSRAM_TRACE 0
#endif

#if AFSPLUS_AFSRAM_TRACE
static volatile ULONG *retained_trace;
static ULONG retained_trace_count;

static void trace_retained_loader(ULONG event, ULONG first, ULONG second)
{
    if (retained_trace != NULL) {
        retained_trace[0] = event;
        retained_trace[1] = first;
        retained_trace[2] = second;
        retained_trace[3] = ++retained_trace_count;
    }
}
#else
#define trace_retained_loader(event, first, second) ((void)0)
#endif

static AROS_UFH4(LONG, retained_read,
    AROS_UFHA(BPTR, file, D1),
    AROS_UFHA(APTR, buffer, D2),
    AROS_UFHA(LONG, length, D3),
    AROS_UFHA(struct DosLibrary *, DOSBase, A6))
{
    struct RetainedHandlerFile *memory = (struct RetainedHandlerFile *)BADDR(file);
    ULONG available;
    ULONG count;

    AROS_USERFUNC_INIT
    (void)DOSBase;
    if (memory == NULL || buffer == NULL || length < 0 ||
        memory->position > memory->size)
        return -1;
    trace_retained_loader(2, memory->position, (ULONG)length);
    available = memory->size - memory->position;
    count = (ULONG)length < available ? (ULONG)length : available;
    if (count != 0)
        CopyMem(memory->data + memory->position, buffer, count);
    memory->position += count;
    return (LONG)count;
    AROS_USERFUNC_EXIT
}

static AROS_UFH4(LONG, retained_seek,
    AROS_UFHA(BPTR, file, D1),
    AROS_UFHA(LONG, position, D2),
    AROS_UFHA(LONG, mode, D3),
    AROS_UFHA(struct DosLibrary *, DOSBase, A6))
{
    struct RetainedHandlerFile *memory = (struct RetainedHandlerFile *)BADDR(file);
    int64_t next;
    ULONG previous;

    AROS_USERFUNC_INIT
    (void)DOSBase;
    if (memory == NULL)
        return -1;
    trace_retained_loader(3, (ULONG)position, (ULONG)mode);
    if (mode == OFFSET_BEGINNING)
        next = position;
    else if (mode == OFFSET_CURRENT)
        next = (int64_t)memory->position + position;
    else if (mode == OFFSET_END)
        next = (int64_t)memory->size + position;
    else
        return -1;
    if (next < 0 || (uint64_t)next > memory->size)
        return -1;
    previous = memory->position;
    memory->position = (ULONG)next;
    return (LONG)(int32_t)previous;
    AROS_USERFUNC_EXIT
}

static AROS_UFH3(APTR, retained_alloc,
    AROS_UFHA(ULONG, length, D0),
    AROS_UFHA(ULONG, flags, D1),
    AROS_UFHA(struct ExecBase *, SysBase, A6))
{
    AROS_USERFUNC_INIT
    trace_retained_loader(4, length, flags);
    if ((flags & MEMF_EXECUTABLE) != 0) {
        struct KernelBase *KernelBase = OpenResource("kernel.resource");
        APTR pages = KernelBase != NULL ?
            KrnAllocPages(NULL, length, flags) : NULL;

        if (pages != NULL)
            return pages;
    }
    return AllocMem(length, flags);
    AROS_USERFUNC_EXIT
}

static AROS_UFH3(void, retained_free,
    AROS_UFHA(APTR, buffer, A1),
    AROS_UFHA(ULONG, length, D0),
    AROS_UFHA(struct ExecBase *, SysBase, A6))
{
    AROS_USERFUNC_INIT
    trace_retained_loader(5, length, (ULONG)(uintptr_t)buffer);
    if (TypeOfMem(buffer) != 0) {
        FreeMem(buffer, length);
    } else {
        struct KernelBase *KernelBase = OpenResource("kernel.resource");

        if (KernelBase != NULL)
            KrnFreePages(buffer, length);
        else
            FreeMem(buffer, length);
    }
    AROS_USERFUNC_EXIT
}

static BPTR load_retained_handler(void)
{
    struct KernelBase *KernelBase = OpenResource("kernel.resource");
    struct AfsplusAfsRamPayload payload;
    struct RetainedHandlerFile file;
    LONG_FUNC functions[] = {
        (LONG_FUNC)retained_read,
        (LONG_FUNC)retained_alloc,
        (LONG_FUNC)retained_free,
        (LONG_FUNC)retained_seek,
    };
    intptr_t image_start;
    intptr_t image_size;

    if (KernelBase == NULL)
        return BNULL;
    image_start = KrnGetSystemAttr(AROS_APPLE_KATTR_BOOT_IMAGE_START);
    image_size = KrnGetSystemAttr(AROS_APPLE_KATTR_BOOT_IMAGE_SIZE);
    if (image_start <= 0 || image_size <= 0 ||
        !afsplus_afsram_locate((const uint8_t *)(uintptr_t)image_start,
            (uint64_t)image_size, &payload) ||
        payload.handler_size > UINT32_MAX)
        return BNULL;
    file.data = (const UBYTE *)(uintptr_t)image_start + payload.handler_offset;
    file.size = (ULONG)payload.handler_size;
    file.position = 0;
#if AFSPLUS_AFSRAM_TRACE
    retained_trace = (volatile ULONG *)(file.data - AFSPLUS_AFSRAM_HEADER_SIZE + 48);
    retained_trace_count = 0;
#endif
    trace_retained_loader(1, file.size, 0);
    {
        BPTR segment = InternalLoadSeg(MKBADDR(&file), BNULL, functions, NULL);

        trace_retained_loader(6, (ULONG)(IPTR)segment, (ULONG)IoErr());
        return segment;
    }
}

#if defined(AFSPLUS_AFSRAM_REPLAY_EXPECTED)
int afsplus_native_replay_probe(const char *expected);
#else
int afsplus_native_alpha_probe(void);
#endif

static int wait_handler_exit(void)
{
    ULONG ticks;

    for (ticks = 0; ticks < 250; ++ticks) {
        struct Task *task;

        Forbid();
        task = FindTask((CONST_STRPTR)"AFSPLUS0");
        Permit();
        if (task == NULL)
            return 1;
        Delay(1);
    }
    return 0;
}

static void free_mount_node(struct DeviceNode *node)
{
    if (node == NULL)
        return;

    /*
     * expansion.library/MakeDosNode() packs the DeviceNode, startup
     * message, environment and both BSTRs into one allocation.  Only the
     * handler name below is supplied separately by this probe.
     */
    if (node->dn_Handler != BNULL)
        FreeVec(BADDR(node->dn_Handler));
    FreeVec(node);
}

static int probe_filesystem(void)
{
    IPTR parameters[4 + DE_BOOTBLOCKS + 1] = {0};
    struct DeviceNode *node;
    struct MsgPort *handler_port;
    BPTR root = BNULL;
    LONG operation_result = -1;
    int result = 0;
    int node_added = 0;
    int node_removed = 0;
    BPTR handler_seglist;

    handler_seglist = load_retained_handler();
    if (handler_seglist == BNULL)
        return 81;
    trace_retained_loader(7, 0, 0);

    parameters[0] = (IPTR)"AFSPLUS0";
    parameters[1] = (IPTR)"afsram.device";
    parameters[2] = 0;
    parameters[3] = 0;
    parameters[4 + DE_TABLESIZE] = DE_BOOTBLOCKS;
    parameters[4 + DE_SIZEBLOCK] = 1024;
    parameters[4 + DE_NUMHEADS] = 1;
    parameters[4 + DE_SECSPERBLOCK] = 1;
    parameters[4 + DE_BLKSPERTRACK] = 1;
    parameters[4 + DE_RESERVEDBLKS] = 0;
    parameters[4 + DE_LOWCYL] = 0;
    parameters[4 + DE_HIGHCYL] = 16383;
    parameters[4 + DE_NUMBUFFERS] = 64;
    parameters[4 + DE_BUFMEMTYPE] = MEMF_PUBLIC;
    parameters[4 + DE_MAXTRANSFER] = 0x7fffffff;
    parameters[4 + DE_MASK] = 0;
    parameters[4 + DE_BOOTPRI] = 0;
    parameters[4 + DE_DOSTYPE] = 0x4146532b;
    parameters[4 + DE_BOOTBLOCKS] = 0;

    node = MakeDosNode(parameters);
    if (node == NULL) {
        UnLoadSeg(handler_seglist);
        return 70;
    }
    node->dn_Handler = CreateBSTR("afsplus-handler");
    node->dn_SegList = handler_seglist;
    node->dn_StackSize = 262144;
    node->dn_Priority = 5;
    node->dn_GlobalVec = (BPTR)-1;
    if (node->dn_Handler == BNULL) {
        result = 71;
        goto out;
    }
    if (!AddDosEntry((struct DosList *)node)) {
        result = 72;
        goto out;
    }
    node_added = 1;
    trace_retained_loader(8, 0, 0);

    root = Lock("AFSPLUS0:", SHARED_LOCK);
    if (root == BNULL) {
        result = 74;
        goto out;
    }
    UnLock(root);
    root = BNULL;
    trace_retained_loader(9, 0, 0);

#if defined(AFSPLUS_AFSRAM_REPLAY_EXPECTED)
    operation_result = afsplus_native_replay_probe(
        AFSPLUS_AFSRAM_REPLAY_EXPECTED);
#else
    operation_result = afsplus_native_alpha_probe();
#endif
    if (operation_result != RETURN_OK) {
        result = 76;
        goto shutdown;
    }
    trace_retained_loader(10, 0, 0);

shutdown:
    trace_retained_loader(11, (ULONG)result, 0);
    handler_port = node->dn_Task;
    if (handler_port == NULL) {
        if (result == 0)
            result = 77;
    } else if (!DoPkt(handler_port, ACTION_INHIBIT, DOSTRUE, 0, 0, 0, 0)) {
        if (result == 0)
            result = 78;
    } else {
        trace_retained_loader(12, 0, 0);
        if (!DoPkt(handler_port, ACTION_DIE, 0, 0, 0, 0, 0)) {
            if (result == 0)
                result = 79;
        } else {
            trace_retained_loader(13, 0, 0);
            if (!wait_handler_exit()) {
                if (result == 0)
                    result = 84;
            } else {
                trace_retained_loader(14, 0, 0);
                node_removed = 1;
            }
        }
    }

out:
    if (root != BNULL)
        UnLock(root);
    if (node_added && !node_removed && node->dn_Task == NULL) {
        if (RemDosEntry((struct DosList *)node))
            node_removed = 1;
        else if (result == 0)
            result = 80;
    }
    if (!node_added || node_removed) {
        trace_retained_loader(15, 0, 0);
        free_mount_node(node);
        trace_retained_loader(16, 0, 0);
        UnLoadSeg(handler_seglist);
        trace_retained_loader(17, 0, 0);
    }
    return result;
}
#endif

int main(int argc, char **argv)
{
    int result;

    if (argc != 4)
        return 30;
    if (argv == NULL || argv[0] == NULL ||
        !string_equal(argv[1], "alpha") ||
        !string_equal(argv[2], "two words") || argv[3] == NULL ||
        argv[4] != NULL)
        return 31;
    result = probe_device();
    if (result == 0) {
#ifndef AFSPLUS_AFSRAM_BLOCK_ONLY
        result = probe_filesystem();
#endif
    }
    if (loaded_device != NULL) {
#ifndef AFSPLUS_AFSRAM_BLOCK_ONLY
        trace_retained_loader(18, 0, 0);
#endif
        if (unload_device(loaded_device) != 0 && result == 0)
            result = 69;
#ifndef AFSPLUS_AFSRAM_BLOCK_ONLY
        trace_retained_loader(19, 0, 0);
#endif
        loaded_device = NULL;
    }
    return result == 0 ? 73 : result;
}
