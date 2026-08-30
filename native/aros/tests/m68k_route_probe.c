/* SPDX-License-Identifier: BSD-2-Clause */

/* Inspect DOS routing without sending a packet to the AFS+ handler. */

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>

int main(void)
{
    const ULONG flags = LDF_DEVICES | LDF_VOLUMES | LDF_READ;
    struct DosList *list = LockDosList(flags);
    struct DosList *device;
    struct DosList *volume;
    struct MsgPort *device_port = NULL;
    struct MsgPort *volume_port = NULL;

    if (list == NULL)
    {
        Printf("[AFSPLUS-ROUTE] FAIL lock-dos-list error %ld\n", IoErr());
        return RETURN_FAIL;
    }
    device = FindDosEntry(list, (CONST_STRPTR)"AFSPLUS19", LDF_DEVICES);
    volume = FindDosEntry(list, (CONST_STRPTR)"AFS+", LDF_VOLUMES);
    if (device != NULL)
        device_port = device->dol_Task;
    if (volume != NULL)
        volume_port = volume->dol_Task;
    UnLockDosList(flags);

    Printf("[AFSPLUS-ROUTE] device %p port %p volume %p port %p\n",
        device, device_port, volume, volume_port);
    if (device == NULL || device_port == NULL || volume == NULL
        || volume_port == NULL || device_port != volume_port)
    {
        Printf("[AFSPLUS-ROUTE] FAIL inconsistent routing\n");
        return RETURN_FAIL;
    }
    Printf("[AFSPLUS-ROUTE] PASS\n");
    return RETURN_OK;
}
