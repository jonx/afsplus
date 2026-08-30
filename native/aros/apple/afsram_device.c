/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * Writable view of an AFS+ payload appended to MacAROS's retained FAT image.
 * This is a QEMU/pre-hardware transport, not a persistence claim.
 */

#include <aros/apple/startup.h>
#include <aros/libcall.h>
#include <aros/symbolsets.h>

#include <devices/newstyle.h>
#include <devices/trackdisk.h>
#include <exec/errors.h>
#include <exec/io.h>
#include <exec/memory.h>
#define SysBase base->sys_base
#include <proto/exec.h>
#define KernelBase base->kernel_base
#include <proto/kernel.h>

#include <stdint.h>

#include "afsram_device.h"
#include "afsram_format.h"
#include LC_LIBDEFS_FILE

#define AFSRAM_TRACK_SECTORS UINT32_C(128)

static const UWORD supported_commands[] = {
    CMD_CLEAR,
    CMD_FLUSH,
    CMD_READ,
    CMD_UPDATE,
    CMD_WRITE,
    TD_CHANGENUM,
    TD_CHANGESTATE,
    TD_FORMAT,
    TD_GETDRIVETYPE,
    TD_GETGEOMETRY,
    TD_GETNUMTRACKS,
    TD_MOTOR,
    TD_PROTSTATUS,
    NSCMD_DEVICEQUERY,
    0
};

static int AfsRam_Init(LIBBASETYPEPTR base)
{
    struct AfsplusAfsRamPayload payload;
    intptr_t image_start;
    intptr_t image_size;

    base->kernel_base = OpenResource("kernel.resource");
    if (base->kernel_base == NULL)
        return FALSE;
    image_start = KrnGetSystemAttr(AROS_APPLE_KATTR_BOOT_IMAGE_START);
    image_size = KrnGetSystemAttr(AROS_APPLE_KATTR_BOOT_IMAGE_SIZE);
    if (image_start <= 0 || image_size <= 0 ||
        !afsplus_afsram_locate((const uint8_t *)(uintptr_t)image_start,
            (uint64_t)image_size, &payload))
        return FALSE;
    if ((uintptr_t)image_start > UINTPTR_MAX - (uintptr_t)payload.offset)
        return FALSE;
    base->payload_start = (uintptr_t)image_start + (uintptr_t)payload.offset;
    base->payload_size = payload.size;
    base->ready = TRUE;
    return TRUE;
}

static int AfsRam_Open(
    LIBBASETYPEPTR base, struct IOExtTD *request, ULONG unit, ULONG flags)
{
    (void)flags;
    if (!base->ready || unit != 0 ||
        request->iotd_Req.io_Message.mn_Length < sizeof(*request)) {
        request->iotd_Req.io_Error = IOERR_OPENFAIL;
        return FALSE;
    }
    request->iotd_Req.io_Unit = &base->unit;
    request->iotd_Req.io_Error = 0;
    return TRUE;
}

static int AfsRam_Close(LIBBASETYPEPTR base, struct IOExtTD *request)
{
    (void)base;
    request->iotd_Req.io_Unit = NULL;
    return TRUE;
}

ADD2INITLIB(AfsRam_Init, 0)
ADD2OPENDEV(AfsRam_Open, 0)
ADD2CLOSEDEV(AfsRam_Close, 0)

static void finish_request(
    struct AfsRamBase *base, struct IORequest *request)
{
    request->io_Message.mn_Node.ln_Type = NT_MESSAGE;
    if ((request->io_Flags & IOF_QUICK) == 0)
        ReplyMsg(&request->io_Message);
}

static int valid_transfer(
    const struct AfsRamBase *base, const struct IOStdReq *request)
{
    return request->io_Data != NULL &&
        ((uint64_t)request->io_Offset &
            (AFSPLUS_AFSRAM_SECTOR_SIZE - 1U)) == 0 &&
        ((uint64_t)request->io_Length &
            (AFSPLUS_AFSRAM_SECTOR_SIZE - 1U)) == 0 &&
        (uint64_t)request->io_Offset <= base->payload_size &&
        (uint64_t)request->io_Length <=
            base->payload_size - (uint64_t)request->io_Offset;
}

AROS_LH1(void, AfsRam_BeginIO,
    AROS_LHA(struct IOExtTD *, request, A1),
    struct AfsRamBase *, base, 5, AfsRam)
{
    struct IOStdReq *io = &request->iotd_Req;

    AROS_LIBFUNC_INIT
    io->io_Error = 0;
    io->io_Actual = 0;
    switch (io->io_Command) {
    case CMD_READ:
        if (io->io_Length == 0) {
            break;
        } else if (!valid_transfer(base, io)) {
            io->io_Error = IOERR_BADADDRESS;
        } else {
            CopyMem((const uint8_t *)base->payload_start + io->io_Offset,
                io->io_Data, io->io_Length);
            io->io_Actual = io->io_Length;
        }
        break;
    case CMD_WRITE:
    case TD_FORMAT:
        if (io->io_Length == 0) {
            break;
        } else if (!valid_transfer(base, io)) {
            io->io_Error = IOERR_BADADDRESS;
        } else {
            CopyMem(io->io_Data,
                (uint8_t *)base->payload_start + io->io_Offset,
                io->io_Length);
            io->io_Actual = io->io_Length;
        }
        break;
    case TD_PROTSTATUS:
    case TD_CHANGENUM:
    case TD_CHANGESTATE:
        io->io_Actual = 0;
        break;
    case TD_GETDRIVETYPE:
        io->io_Actual = DRIVE_NEWSTYLE;
        break;
    case TD_GETNUMTRACKS:
        io->io_Actual = (ULONG)(base->payload_size /
            (AFSPLUS_AFSRAM_SECTOR_SIZE * AFSRAM_TRACK_SECTORS));
        break;
    case TD_GETGEOMETRY:
        if (io->io_Data == NULL ||
            io->io_Length < sizeof(struct DriveGeometry)) {
            io->io_Error = IOERR_BADLENGTH;
        } else {
            struct DriveGeometry *geometry = io->io_Data;

            geometry->dg_SectorSize = AFSPLUS_AFSRAM_SECTOR_SIZE;
            geometry->dg_TotalSectors =
                (ULONG)(base->payload_size / AFSPLUS_AFSRAM_SECTOR_SIZE);
            geometry->dg_Cylinders = geometry->dg_TotalSectors /
                AFSRAM_TRACK_SECTORS;
            geometry->dg_CylSectors = AFSRAM_TRACK_SECTORS;
            geometry->dg_Heads = 1;
            geometry->dg_TrackSectors = AFSRAM_TRACK_SECTORS;
            geometry->dg_BufMemType = MEMF_PUBLIC;
            geometry->dg_DeviceType = DG_DIRECT_ACCESS;
            geometry->dg_Flags = 0;
            io->io_Actual = sizeof(*geometry);
        }
        break;
    case NSCMD_DEVICEQUERY:
        if (io->io_Data == NULL ||
            io->io_Length < sizeof(struct NSDeviceQueryResult)) {
            io->io_Error = IOERR_BADLENGTH;
        } else {
            struct NSDeviceQueryResult *query = io->io_Data;

            query->DevQueryFormat = 0;
            query->SizeAvailable = sizeof(*query);
            query->DeviceType = NSDEVTYPE_TRACKDISK;
            query->DeviceSubType = 0;
            query->SupportedCommands = (UWORD *)supported_commands;
            io->io_Actual = sizeof(*query);
        }
        break;
    case CMD_CLEAR:
    case CMD_FLUSH:
    case CMD_UPDATE:
    case TD_MOTOR:
        break;
    default:
        io->io_Error = IOERR_NOCMD;
        break;
    }
    finish_request(base, (struct IORequest *)io);
    AROS_LIBFUNC_EXIT
}

AROS_LH1(LONG, AfsRam_AbortIO,
    AROS_LHA(struct IORequest *, request, A1),
    struct AfsRamBase *, base, 6, AfsRam)
{
    AROS_LIBFUNC_INIT
    (void)base;
    (void)request;
    return IOERR_NOCMD;
    AROS_LIBFUNC_EXIT
}
