/* SPDX-License-Identifier: BSD-2-Clause */

/* AFSPlusInfo <path>: prints the structured report of the AFS+ handler that
 * serves path, as the JSON document of schema afsplus-handler-info. A handler
 * without the extension transport is reported as such with RETURN_WARN. */

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <exec/memory.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <string.h>

#include "../client/afsplus_client.h"

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

    if (argc != 2)
    {
        Printf("usage: AFSPlusInfo <volume or path>\n");
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
