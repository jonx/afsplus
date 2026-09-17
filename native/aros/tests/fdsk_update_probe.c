/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * FDSKUpdateProbe -- regression probe for the fdsk.device write barrier.
 *
 * A filesystem issues CMD_UPDATE after its writes and treats the reply as
 * "everything queued before this point has reached the backing file". The
 * probe queues one CMD_WRITE and one CMD_UPDATE under Forbid(), so the unit
 * process cannot run in between. A device that answers CMD_UPDATE inside
 * BeginIO completes it while the write is still queued: the barrier overtook
 * the data. A correct device leaves both pending and completes them in order.
 *
 * Usage: FDSKUpdateProbe <unit>. The unit must be a writable scratch image;
 * block 0 is overwritten. Prints one "[FDSKUPDATE] PASS" or FAIL line.
 */

#include <exec/types.h>
#include <exec/io.h>
#include <exec/memory.h>
#include <devices/trackdisk.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <stdlib.h>

#define PROBE_BLOCK 512

static struct IOExtTD *create_request(struct MsgPort *port, ULONG unit)
{
    struct IOExtTD *request = (struct IOExtTD *)CreateIORequest(port,
        sizeof(struct IOExtTD));

    if (request == NULL)
        return NULL;
    if (OpenDevice("fdsk.device", unit, (struct IORequest *)request, 0) != 0)
    {
        DeleteIORequest((struct IORequest *)request);
        return NULL;
    }
    return request;
}

int main(int argc, char **argv)
{
    struct MsgPort *port;
    struct IOExtTD *write_request;
    struct IOExtTD update_request;
    UBYTE *block;
    BOOL update_overtook;
    BOOL write_pending;
    int result = 20;

    if (argc != 2)
    {
        Printf("[FDSKUPDATE] FAIL usage: FDSKUpdateProbe <unit>\n");
        return 20;
    }
    port = CreateMsgPort();
    block = AllocMem(PROBE_BLOCK, MEMF_PUBLIC | MEMF_CLEAR);
    write_request = port != NULL ? create_request(port, (ULONG)atol(argv[1]))
        : NULL;
    if (port == NULL || block == NULL || write_request == NULL)
    {
        Printf("[FDSKUPDATE] FAIL setup\n");
        goto cleanup;
    }

    /* Same device and unit, second request for the barrier. */
    update_request = *write_request;

    write_request->iotd_Req.io_Command = CMD_WRITE;
    write_request->iotd_Req.io_Data = block;
    write_request->iotd_Req.io_Length = PROBE_BLOCK;
    write_request->iotd_Req.io_Offset = 0;
    update_request.iotd_Req.io_Command = CMD_UPDATE;
    update_request.iotd_Req.io_Data = NULL;
    update_request.iotd_Req.io_Length = 0;

    Forbid();
    SendIO((struct IORequest *)write_request);
    SendIO((struct IORequest *)&update_request);
    write_pending = CheckIO((struct IORequest *)write_request) == NULL;
    update_overtook = write_pending
        && CheckIO((struct IORequest *)&update_request) != NULL;
    Permit();

    WaitIO((struct IORequest *)write_request);
    WaitIO((struct IORequest *)&update_request);

    if (!write_pending)
        Printf("[FDSKUPDATE] FAIL write completed under Forbid; probe"
            " cannot observe ordering\n");
    else if (update_overtook)
        Printf("[FDSKUPDATE] FAIL CMD_UPDATE replied before the queued"
            " CMD_WRITE\n");
    else if (write_request->iotd_Req.io_Error != 0
        || update_request.iotd_Req.io_Error != 0)
        Printf("[FDSKUPDATE] FAIL io_Error write=%ld update=%ld\n",
            (LONG)write_request->iotd_Req.io_Error,
            (LONG)update_request.iotd_Req.io_Error);
    else
    {
        Printf("[FDSKUPDATE] PASS barrier queued behind the write\n");
        result = 0;
    }

cleanup:
    if (write_request != NULL)
    {
        CloseDevice((struct IORequest *)write_request);
        DeleteIORequest((struct IORequest *)write_request);
    }
    if (block != NULL)
        FreeMem(block, PROBE_BLOCK);
    if (port != NULL)
        DeleteMsgPort(port);
    return result;
}
