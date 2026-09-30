/* SPDX-License-Identifier: BSD-2-Clause */
/* Query a mounted device without taking a filesystem lock on blank media. */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
    struct MsgPort *port;
    struct InfoData info;
    ULONG expected;
    LONG result;

    if (argc != 3)
        return RETURN_ERROR;
    expected = (ULONG)strtoul(argv[2], NULL, 0);
    port = DeviceProc((CONST_STRPTR)argv[1]);
    if (port == NULL)
    {
        Printf("[FORMAT-STATE] FAIL DeviceProc error=%ld\n", (long)IoErr());
        return RETURN_ERROR;
    }
    memset(&info, 0, sizeof(info));
    result = DoPkt(port, ACTION_DISK_INFO, (SIPTR)MKBADDR(&info), 0, 0, 0, 0);
    Printf("[FORMAT-STATE] result=%ld type=0x%08lx volume=%p expected=0x%08lx %s\n",
        (long)result, (unsigned long)(ULONG)info.id_DiskType,
        BADDR(info.id_VolumeNode), (unsigned long)expected,
        result && (ULONG)info.id_DiskType == expected ? "PASS" : "FAIL");
    return result && (ULONG)info.id_DiskType == expected ? RETURN_OK : RETURN_ERROR;
}
