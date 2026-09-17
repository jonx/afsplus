/* SPDX-License-Identifier: BSD-2-Clause */

/* AFSPlusInfo <path>: prints the structured report of the AFS+ handler that
 * serves path, as the JSON document of schema afsplus-handler-info. A handler
 * without the extension transport is reported as such with RETURN_WARN.
 *
 * AFSPlusInfo <path> PACKETS prints what the handler has answered since it
 * started, one line per packet type ("packet <type> <count> <failed>") and
 * one per error code ("error <code> <count>"), in decimal.
 *
 * AFSPlusInfo <path> TRACE drains the handler's trace ring, one line per
 * event ("trace <sequence> <timestamp> <category> <event> <object>"), then
 * "trace lost <n>" and the sink's counters. A mount whose Control string did
 * not ask for a ring says so. */

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

#define TRACE_EVENTS 64

static LONG print_trace(struct MsgPort *port)
{
    static struct afsp_trace_event events[TRACE_EVENTS];
    struct AfsplusArosTraceCounters counters;
    uint32_t count = 0;
    uint32_t index;
    uint64_t lost = 0;
    LONG error;

    /* Drain until the ring is empty: one call takes what fits. */
    do
    {
        error = afsplus_client_trace_events(port, events, TRACE_EVENTS,
            &count, &lost);
        /* Printf takes 32-bit words, and three of these fields are 64 bits:
         * the time is split into seconds and nanoseconds, and the object is
         * printed as two halves rather than silently truncated. */
        for (index = 0; error == 0 && index < count; index++)
            Printf("trace %lu %lu.%09lu %lu %lu %08lx%08lx\n",
                (ULONG)events[index].sequence,
                (ULONG)(events[index].timestamp / UINT64_C(1000000000)),
                (ULONG)(events[index].timestamp % UINT64_C(1000000000)),
                (ULONG)events[index].category,
                (ULONG)events[index].event,
                (ULONG)(events[index].object_id >> 32),
                (ULONG)events[index].object_id);
    }
    while (error == 0 && count == TRACE_EVENTS);
    if (error != 0)
        return error;
    Printf("trace lost %lu\n", (ULONG)lost);
    if (lost > UINT64_C(0xFFFFFFFF))
        Printf("trace lost more than a 32-bit count can show\n");
    memset(&counters, 0, sizeof(counters));
    error = afsplus_client_trace_counters(port, &counters);
    if (error == 0)
        Printf("trace counters attached %lu delivered %lu missed %lu"
            " filtered %lu dropped %lu\n",
            (ULONG)counters.attached, (ULONG)counters.delivered,
            (ULONG)counters.missed, (ULONG)counters.filtered,
            (ULONG)counters.dropped);
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

    if (argc != 2 && !(argc == 3 && (strcmp(argv[2], "PACKETS") == 0
        || strcmp(argv[2], "TRACE") == 0)))
    {
        Printf("usage: AFSPlusInfo <volume or path> [PACKETS|TRACE]\n");
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
    if (error == 0 && argc == 3 && strcmp(argv[2], "TRACE") == 0)
    {
        error = print_trace(port);
        FreeDeviceProc(process);
        if (error == ERROR_NOT_IMPLEMENTED)
        {
            Printf("AFSPlusInfo: this mount keeps no trace ring; add"
                " TRACE=<events> to the DOSDriver Control string\n");
            return RETURN_WARN;
        }
        if (error != 0)
        {
            Printf("AFSPlusInfo: error %ld\n", error);
            return RETURN_FAIL;
        }
        return RETURN_OK;
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
        /* A report is never empty; asking again would not change that. */
        if (required == 0)
        {
            error = ERROR_OBJECT_WRONG_TYPE;
            break;
        }
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
