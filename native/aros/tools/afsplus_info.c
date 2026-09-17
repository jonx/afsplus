/* SPDX-License-Identifier: BSD-2-Clause */

/* AFSPlusInfo <path>: prints the structured report of the AFS+ handler that
 * serves path, as the JSON document of schema afsplus-handler-info. A handler
 * without the extension transport is reported as such with RETURN_WARN.
 *
 * AFSPlusInfo <path> PACKETS prints what the handler has answered since it
 * started, one line per packet type ("packet <type> <count> <failed>") and
 * one per error code ("error <code> <count>"), in decimal. */

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <exec/memory.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <string.h>

#include "../client/afsplus_client.h"

#define COUNT_RECORDS 66

static LONG print_counts(struct MsgPort *port, uint32_t which)
{
    static struct AfsplusExtPacketCount records[COUNT_RECORDS];
    uint32_t stored = 0;
    uint32_t total = 0;
    uint32_t index;
    LONG error = afsplus_client_packet_counts(port, which, records,
        COUNT_RECORDS, &stored, &total);

    for (index = 0; error == 0 && index < stored; index++)
    {
        /* Counts are printed in 32 bits; a handler does not live that long
         * in a test, and a saturated value says so. */
        ULONG count = records[index].count > 0xFFFFFFFFULL
            ? 0xFFFFFFFFUL : (ULONG)records[index].count;
        ULONG failed = records[index].failed > 0xFFFFFFFFULL
            ? 0xFFFFFFFFUL : (ULONG)records[index].failed;

        if (which == AFSPLUS_EXT_COUNT_BY_ACTION)
            Printf("packet %ld %lu %lu\n", (LONG)records[index].key, count,
                failed);
        else
            Printf("error %ld %lu\n", (LONG)records[index].key, count);
    }
    return error;
}

int main(int argc, char **argv)
{
    struct DevProc *process;
    struct MsgPort *port;
    char *document = NULL;
    uint32_t capacity = 0;
    uint32_t required = 0;
    uint32_t revision = 0;
    uint64_t groups = 0;
    LONG error;
    int attempt;

    if (argc != 2 && !(argc == 3 && strcmp(argv[2], "PACKETS") == 0))
    {
        Printf("usage: AFSPlusInfo <volume or path> [PACKETS]\n");
        return RETURN_ERROR;
    }
    process = GetDeviceProc((CONST_STRPTR)argv[1], NULL);
    if (process == NULL)
    {
        Printf("AFSPlusInfo: %s: error %ld\n", argv[1], IoErr());
        return RETURN_ERROR;
    }
    port = process->dvp_Port;
    error = afsplus_client_interface(port, &revision, NULL, &groups);
    if (error == ERROR_ACTION_NOT_KNOWN)
    {
        FreeDeviceProc(process);
        Printf("AFSPlusInfo: %s is not served by an AFS+ handler with the "
            "extension transport\n", argv[1]);
        return RETURN_WARN;
    }
    if (error == 0 && argc == 3)
    {
        error = print_counts(port, AFSPLUS_EXT_COUNT_BY_ACTION);
        if (error == 0)
            error = print_counts(port, AFSPLUS_EXT_COUNT_BY_ERROR);
        FreeDeviceProc(process);
        if (error != 0)
        {
            Printf("AFSPlusInfo: error %ld\n", error);
            return RETURN_FAIL;
        }
        return RETURN_OK;
    }
    /* The document can grow between the sizing call and the read. */
    for (attempt = 0; error == 0 && attempt < 4; attempt++)
    {
        error = afsplus_client_info_json(port, document, capacity,
            &required);
        if (error != 0 || (required != 0 && required <= capacity))
            break;
        if (document != NULL)
            FreeVec(document);
        capacity = required + 256;
        document = AllocVec(capacity + 1, MEMF_ANY | MEMF_CLEAR);
        if (document == NULL)
            error = ERROR_NO_FREE_STORE;
    }
    FreeDeviceProc(process);
    if (error == 0 && (document == NULL || required > capacity))
        error = ERROR_OBJECT_TOO_LARGE;
    if (error != 0)
    {
        if (document != NULL)
            FreeVec(document);
        Printf("AFSPlusInfo: error %ld\n", error);
        return RETURN_FAIL;
    }
    document[required] = 0;
    PutStr((CONST_STRPTR)document);
    PutStr((CONST_STRPTR)"\n");
    FreeVec(document);
    return RETURN_OK;
}
