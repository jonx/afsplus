/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * AFSPlusDriverProbe -- does a block device give the AFS+ handler what it
 * relies on? docs/aros-block-device-contract.md is the contract; each check
 * below is one of its clauses.
 *
 * Usage: AFSPlusDriverProbe <device> <unit> <first-block> <blocks> WRITE [CORRUPT]
 *
 * Blocks are the handler's 4096 bytes. The range is read and kept, written
 * with distinct patterns, checked, and written back as it was; WRITE is the
 * explicit consent to that. CORRUPT flips one byte of the expected pattern,
 * so a harness can prove that a mismatch fails the probe. Every check prints
 * one "[AFSPLUS-DRIVER] PASS name" or "FAIL name ..." line, and the last line
 * is "[AFSPLUS-DRIVER] PASS" or "[AFSPLUS-DRIVER] FAIL".
 */

#include <exec/types.h>
#include <exec/errors.h>
#include <exec/io.h>
#include <exec/memory.h>
#include <devices/newstyle.h>
#include <devices/trackdisk.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#ifndef TD_READ64
#define TD_READ64 (CMD_NONSTD + 15)
#define TD_WRITE64 (CMD_NONSTD + 16)
#endif

#define BLOCK 4096u
#define MAX_BLOCKS 64u
#define MAX_NSD_COMMANDS 256u

struct Probe
{
    struct MsgPort *port;
    struct IOExtTD *request;
    UWORD read_command;
    UWORD write_command;
    uint64_t device_bytes;
    ULONG failures;
};

static void report(struct Probe *probe, BOOL passed, CONST_STRPTR name,
    CONST_STRPTR detail)
{
    if (passed)
        Printf("[AFSPLUS-DRIVER] PASS %s\n", name);
    else
    {
        Printf("[AFSPLUS-DRIVER] FAIL %s %s\n", name, detail);
        probe->failures++;
    }
}

static void prepare(struct IOExtTD *request, UWORD command, APTR data,
    ULONG length, uint64_t offset)
{
    request->iotd_Req.io_Command = command;
    request->iotd_Req.io_Flags = 0;
    request->iotd_Req.io_Data = data;
    request->iotd_Req.io_Length = length;
    request->iotd_Req.io_Offset = (ULONG)offset;
    request->iotd_Req.io_Actual = (ULONG)(offset >> 32);
}

static BOOL transfer(struct Probe *probe, BOOL writing, uint64_t offset,
    UBYTE *buffer)
{
    prepare(probe->request, writing ? probe->write_command : probe->read_command,
        buffer, BLOCK, offset);
    return DoIO((struct IORequest *)probe->request) == 0
        && probe->request->iotd_Req.io_Error == 0
        && probe->request->iotd_Req.io_Actual == BLOCK;
}

static BOOL update(struct Probe *probe)
{
    prepare(probe->request, CMD_UPDATE, NULL, 0, 0);
    return DoIO((struct IORequest *)probe->request) == 0
        && probe->request->iotd_Req.io_Error == 0;
}

static BOOL command_known(struct Probe *probe, UWORD command)
{
    LONG result;

    prepare(probe->request, command, NULL, 0, 0);
    result = DoIO((struct IORequest *)probe->request);
    return result != IOERR_NOCMD && probe->request->iotd_Req.io_Error != IOERR_NOCMD;
}

/* The handler's own choice (afsplus_handler.c, probe_64bit_commands):
 * NSD 64-bit, then TD64, then the 32-bit commands. */
static void choose_commands(struct Probe *probe)
{
    struct NSDeviceQueryResult query;
    ULONG index;

    probe->read_command = CMD_READ;
    probe->write_command = CMD_WRITE;
    memset(&query, 0, sizeof(query));
    prepare(probe->request, NSCMD_DEVICEQUERY, &query, sizeof(query), 0);
    if (DoIO((struct IORequest *)probe->request) == 0
        && probe->request->iotd_Req.io_Error == 0
        && query.SupportedCommands != NULL)
    {
        BOOL read64 = FALSE;
        BOOL write64 = FALSE;

        for (index = 0; index < MAX_NSD_COMMANDS
            && query.SupportedCommands[index] != 0; index++)
        {
            read64 |= query.SupportedCommands[index] == NSCMD_TD_READ64;
            write64 |= query.SupportedCommands[index] == NSCMD_TD_WRITE64;
        }
        if (read64 && write64)
        {
            probe->read_command = NSCMD_TD_READ64;
            probe->write_command = NSCMD_TD_WRITE64;
            Printf("[AFSPLUS-DRIVER] commands NSD 64-bit\n");
            return;
        }
    }
    if (command_known(probe, TD_READ64) && command_known(probe, TD_WRITE64))
    {
        probe->read_command = TD_READ64;
        probe->write_command = TD_WRITE64;
        Printf("[AFSPLUS-DRIVER] commands TD64\n");
        return;
    }
    Printf("[AFSPLUS-DRIVER] commands 32-bit\n");
}

static void fill(UBYTE *buffer, ULONG block, ULONG round)
{
    ULONG index;

    for (index = 0; index < BLOCK; index++)
        buffer[index] = (UBYTE)((block * 131u + round * 17u + index * 7u) ^ (index >> 8));
}

/* The barrier: a CMD_UPDATE queued behind a write may not be answered while
 * the write is still pending. */
static void check_barrier(struct Probe *probe, uint64_t offset, UBYTE *buffer)
{
    struct IOExtTD barrier = *probe->request;
    BOOL write_pending;
    BOOL overtook;

    prepare(probe->request, probe->write_command, buffer, BLOCK, offset);
    prepare(&barrier, CMD_UPDATE, NULL, 0, 0);
    Forbid();
    SendIO((struct IORequest *)probe->request);
    SendIO((struct IORequest *)&barrier);
    write_pending = CheckIO((struct IORequest *)probe->request) == NULL;
    overtook = write_pending && CheckIO((struct IORequest *)&barrier) != NULL;
    Permit();
    WaitIO((struct IORequest *)probe->request);
    WaitIO((struct IORequest *)&barrier);
    if (!write_pending)
        Printf("[AFSPLUS-DRIVER] note barrier: the write completed at once, the order cannot be seen\n");
    report(probe, !overtook && probe->request->iotd_Req.io_Error == 0
        && barrier.iotd_Req.io_Error == 0, "barrier",
        "CMD_UPDATE answered before the write queued ahead of it");
}

int main(int argc, char **argv)
{
    struct Probe probe;
    struct DriveGeometry geometry;
    UBYTE *saved = NULL;
    UBYTE *block = NULL;
    UBYTE *expected = NULL;
    ULONG first;
    ULONG count;
    ULONG index;
    BOOL corrupt;
    BOOL opened = FALSE;
    BOOL restored = TRUE;

    memset(&probe, 0, sizeof(probe));
    if (argc < 6 || argc > 7 || strcmp(argv[5], "WRITE") != 0
        || (argc == 7 && strcmp(argv[6], "CORRUPT") != 0))
    {
        Printf("usage: AFSPlusDriverProbe <device> <unit> <first-block> <blocks> WRITE [CORRUPT]\n");
        return 20;
    }
    first = (ULONG)strtoul(argv[3], NULL, 10);
    count = (ULONG)strtoul(argv[4], NULL, 10);
    corrupt = argc == 7;
    if (count == 0 || count > MAX_BLOCKS)
    {
        Printf("[AFSPLUS-DRIVER] FAIL range between 1 and %lu blocks\n", (ULONG)MAX_BLOCKS);
        return 20;
    }

    probe.port = CreateMsgPort();
    probe.request = probe.port == NULL ? NULL
        : (struct IOExtTD *)CreateIORequest(probe.port, sizeof(struct IOExtTD));
    saved = AllocMem(count * BLOCK, MEMF_PUBLIC);
    block = AllocMem(BLOCK, MEMF_PUBLIC);
    expected = AllocMem(BLOCK, MEMF_PUBLIC);
    if (probe.request == NULL || saved == NULL || block == NULL || expected == NULL)
    {
        Printf("[AFSPLUS-DRIVER] FAIL setup\n");
        probe.failures++;
        goto done;
    }
    opened = OpenDevice((CONST_STRPTR)argv[1], (ULONG)strtoul(argv[2], NULL, 10),
        (struct IORequest *)probe.request, 0) == 0;
    report(&probe, opened, "open", "OpenDevice refused the unit");
    if (!opened)
        goto done;

    choose_commands(&probe);

    memset(&geometry, 0, sizeof(geometry));
    prepare(probe.request, TD_GETGEOMETRY, &geometry, sizeof(geometry), 0);
    if (DoIO((struct IORequest *)probe.request) == 0
        && probe.request->iotd_Req.io_Error == 0 && geometry.dg_SectorSize != 0)
    {
        probe.device_bytes = (uint64_t)geometry.dg_TotalSectors * geometry.dg_SectorSize;
        Printf("[AFSPLUS-DRIVER] geometry sector %lu sectors %lu\n",
            geometry.dg_SectorSize, geometry.dg_TotalSectors);
        report(&probe, BLOCK % geometry.dg_SectorSize == 0, "sector-size",
            "4096 is not a multiple of the sector size");
    }
    else
        report(&probe, FALSE, "geometry", "TD_GETGEOMETRY failed");
    if (probe.device_bytes != 0
        && ((uint64_t)first + count) * BLOCK > probe.device_bytes)
    {
        report(&probe, FALSE, "range", "the range ends beyond the device");
        goto done;
    }

    /* Both are optional: the handler reads an unknown command as a medium
     * present and writable. */
    prepare(probe.request, TD_CHANGESTATE, NULL, 0, 0);
    DoIO((struct IORequest *)probe.request);
    report(&probe, probe.request->iotd_Req.io_Error == IOERR_NOCMD
        || (probe.request->iotd_Req.io_Error == 0
            && probe.request->iotd_Req.io_Actual == 0), "media", "no medium present");
    prepare(probe.request, TD_PROTSTATUS, NULL, 0, 0);
    DoIO((struct IORequest *)probe.request);
    report(&probe, probe.request->iotd_Req.io_Error == IOERR_NOCMD
        || (probe.request->iotd_Req.io_Error == 0
            && probe.request->iotd_Req.io_Actual == 0), "writable", "the medium is write-protected");
    report(&probe, command_known(&probe, CMD_UPDATE), "update-known",
        "CMD_UPDATE is not a known command");

    /* Keep the range as it was. */
    for (index = 0; index < count; index++)
        if (!transfer(&probe, FALSE, ((uint64_t)first + index) * BLOCK, saved + index * BLOCK))
        {
            report(&probe, FALSE, "read-original", "reading the range failed");
            goto done;
        }
    restored = FALSE;

    {
        BOOL written = TRUE;
        BOOL matched = TRUE;

        for (index = 0; index < count; index++)
        {
            fill(block, first + index, 1);
            written &= transfer(&probe, TRUE, ((uint64_t)first + index) * BLOCK, block);
        }
        report(&probe, written, "write", "a 4096-byte write failed or was short");
        report(&probe, update(&probe), "update", "CMD_UPDATE failed");
        for (index = 0; index < count; index++)
        {
            fill(expected, first + index, 1);
            if (corrupt && index == count - 1)
                expected[BLOCK / 2] ^= 0x40;
            matched &= transfer(&probe, FALSE, ((uint64_t)first + index) * BLOCK, block)
                && memcmp(block, expected, BLOCK) == 0;
        }
        report(&probe, matched, "read-back", "a block read back differs from what was written");
    }

    {
        uint64_t offset = (uint64_t)first * BLOCK;
        BOOL last_wins;

        fill(block, first, 2);
        transfer(&probe, TRUE, offset, block);
        fill(block, first, 3);
        transfer(&probe, TRUE, offset, block);
        update(&probe);
        fill(expected, first, 3);
        last_wins = transfer(&probe, FALSE, offset, block)
            && memcmp(block, expected, BLOCK) == 0;
        report(&probe, last_wins, "last-write-wins", "two writes to one block landed out of order");
        fill(block, first, 4);
        check_barrier(&probe, offset, block);
    }

    if (probe.device_bytes != 0)
    {
        BOOL refused = !transfer(&probe, FALSE, probe.device_bytes, block);

        report(&probe, refused, "beyond-end", "a read past the end of the device succeeded");
    }

done:
    if (!restored)
    {
        BOOL back = TRUE;

        for (index = 0; index < count; index++)
            back &= transfer(&probe, TRUE, ((uint64_t)first + index) * BLOCK, saved + index * BLOCK);
        back &= update(&probe);
        report(&probe, back, "restore", "the range could not be written back");
    }
    if (opened)
        CloseDevice((struct IORequest *)probe.request);
    if (probe.request != NULL)
        DeleteIORequest((struct IORequest *)probe.request);
    if (probe.port != NULL)
        DeleteMsgPort(probe.port);
    if (saved != NULL)
        FreeMem(saved, count * BLOCK);
    if (block != NULL)
        FreeMem(block, BLOCK);
    if (expected != NULL)
        FreeMem(expected, BLOCK);
    Printf("[AFSPLUS-DRIVER] %s\n", probe.failures == 0 ? "PASS" : "FAIL");
    return probe.failures == 0 ? 0 : 10;
}
