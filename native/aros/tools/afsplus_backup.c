/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* Local consistent backup capture. The snapshot remains on its source volume. */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <stdio.h>
#include <string.h>
#include "../client/afsplus_client.h"

static void print_id(const char *label, uint64_t id)
{
    char text[21];
    snprintf(text, sizeof text, "%llu", (unsigned long long)id);
    Printf((CONST_STRPTR)"%s%s\n", label, text);
}
int main(int argc, char **argv)
{
    struct DevProc *device;
    LONG error;
    uint64_t id;
    if (argc < 2 || argc > 3 || (argc == 3 && strcmp(argv[2], "LIST") != 0))
    {
        Printf((CONST_STRPTR)"usage: AFSPlusBackup <volume> [LIST]\n"
               "Creates a local snapshot capture; LIST shows retained captures.\n");
        return RETURN_ERROR;
    }
    device = GetDeviceProc((CONST_STRPTR)argv[1], NULL);
    if (device == NULL) return RETURN_FAIL;
    if (argc == 2)
    {
        error = afsplus_client_snapshot_create(device->dvp_Port, &id);
        if (error == 0) print_id("AFSPlusBackup: captured snapshot ", id);
    }
    else
    {
        struct AfsplusArosSnapshotInfo entries[32];
        uint64_t next = 0, previous;
        uint32_t count, i;
        do
        {
            previous = next;
            error = afsplus_client_snapshot_list(device->dvp_Port, previous,
                entries, 32, &count, &next);
            for (i = 0; error == 0 && i < count; i++) print_id("snapshot ", entries[i].id);
            if (error == 0 && next != 0 && next <= previous) error = ERROR_BAD_NUMBER;
        } while (error == 0 && next != 0);
    }
    FreeDeviceProc(device);
    if (error != 0) Printf((CONST_STRPTR)"AFSPlusBackup: error %ld\n", error);
    return error == 0 ? RETURN_OK : RETURN_FAIL;
}
