/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * AFSPlusTour <drawer> -- what the AFS+ extension interface gives a program,
 * shown on the volume it runs on.
 *
 *   1. The volume answers the AFS+ query, with its capabilities and health.
 *   2. A 16 MiB file is cloned: the copy exists at once and takes no space
 *      until one side changes. A byte copy of the same file follows for
 *      comparison.
 *   3. A watch on the drawer sees a file appear, without polling.
 *   4. An extended attribute is written and read back.
 *
 * Every step prints a sentence for a person and an "[AFSPLUS-TOUR] <step>
 * PASS" or "FAIL" line for a harness; the files it makes are removed at the
 * end. On a volume that is not AFS+ it says so and stops.
 */

#include <devices/timer.h>
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <exec/memory.h>
#include <proto/dos.h>
#include <proto/exec.h>
#include <proto/timer.h>

#include <stdint.h>
#include <string.h>

#include "../client/afsplus_client.h"

#define TOUR_BYTES (16u * 1024u * 1024u)
#define CHUNK (256u * 1024u)
#define PATH_MAX_TOUR 512

struct Device *TimerBase;
static struct timerequest timer_request;
static ULONG failures;

static uint64_t now_microseconds(void)
{
    struct EClockVal now;
    uint64_t ticks;
    ULONG frequency = ReadEClock(&now);

    ticks = ((uint64_t)now.ev_hi << 32) | now.ev_lo;
    return frequency != 0
        ? ticks / frequency * UINT64_C(1000000)
            + ticks % frequency * UINT64_C(1000000) / frequency
        : 0;
}

static void step(CONST_STRPTR name, BOOL passed)
{
    Printf("[AFSPLUS-TOUR] %s %s\n", name, passed ? "PASS" : "FAIL");
    if (!passed)
        failures++;
}

static void join(char *out, CONST_STRPTR drawer, CONST_STRPTR name)
{
    strncpy(out, (const char *)drawer, PATH_MAX_TOUR - 1);
    out[PATH_MAX_TOUR - 1] = 0;
    AddPart((STRPTR)out, name, PATH_MAX_TOUR);
}

/* Bytes the volume holds in use now. */
static uint64_t used_bytes(BPTR lock)
{
    struct InfoData info;

    if (!Info(lock, &info))
        return 0;
    return (uint64_t)(ULONG)info.id_NumBlocksUsed * (ULONG)info.id_BytesPerBlock;
}

static BOOL write_file(CONST_STRPTR path, UBYTE *chunk)
{
    BPTR file = Open(path, MODE_NEWFILE);
    ULONG written = 0;
    BOOL ok = file != BNULL;

    while (ok && written < TOUR_BYTES)
    {
        ULONG index;

        for (index = 0; index < CHUNK; index += 64)
            chunk[index] = (UBYTE)(written / CHUNK + index);
        ok = Write(file, chunk, CHUNK) == (LONG)CHUNK;
        written += CHUNK;
    }
    if (file != BNULL && !Close(file))
        ok = FALSE;
    return ok;
}

static BOOL copy_file(CONST_STRPTR from, CONST_STRPTR to, UBYTE *chunk)
{
    BPTR source = Open(from, MODE_OLDFILE);
    BPTR target = source != BNULL ? Open(to, MODE_NEWFILE) : BNULL;
    BOOL ok = source != BNULL && target != BNULL;
    LONG got;

    while (ok && (got = Read(source, chunk, CHUNK)) > 0)
        ok = Write(target, chunk, got) == got;
    if (target != BNULL && !Close(target))
        ok = FALSE;
    if (source != BNULL)
        Close(source);
    return ok;
}

static void tour(BPTR drawer, CONST_STRPTR drawer_name, UBYTE *chunk)
{
    char original[PATH_MAX_TOUR];
    char clone[PATH_MAX_TOUR];
    char copy[PATH_MAX_TOUR];
    char watched[PATH_MAX_TOUR];
    struct MsgPort *port = afsplus_client_lock_port(drawer);
    struct AfsplusArosHealth health;
    struct AfsplusArosCapabilities capabilities;
    uint32_t revision = 0;
    uint32_t packet_abi = 0;
    uint64_t groups = 0;

    join(original, drawer_name, "tour-original");
    join(clone, drawer_name, "tour-clone");
    join(copy, drawer_name, "tour-copy");
    join(watched, drawer_name, "tour-new-file");

    /* 1. The volume. */
    if (port == NULL || afsplus_client_interface(port, &revision, &packet_abi, &groups) != 0)
    {
        Printf("%s is not on an AFS+ volume: nothing here answers the AFS+ query.\n",
            drawer_name);
        step("volume", FALSE);
        return;
    }
    memset(&capabilities, 0, sizeof(capabilities));
    memset(&health, 0, sizeof(health));
    if (afsplus_client_capabilities(port, &capabilities) == 0
        && afsplus_client_health(port, &health) == 0)
    {
        Printf("\nThis is an AFS+ volume, interface revision %lu: %lu MiB, %lu MiB free,"
            " checkpoint %lu.\n",
            (ULONG)revision,
            (ULONG)(health.total_blocks * 4096 >> 20),
            (ULONG)(health.available_blocks * 4096 >> 20),
            (ULONG)health.generation);
        Printf("Health: %lu device errors, %lu corruption errors, %lu internal faults.\n",
            (ULONG)health.device_errors, (ULONG)health.corruption_errors,
            (ULONG)health.internal_faults);
        Printf("Notices: %lu checkpoint fallbacks, %lu reclaim backlogs,"
            " %lu free-count mismatches.\n",
            (ULONG)health.checkpoint_fallbacks, (ULONG)health.reclaim_backlog_highs,
            (ULONG)health.free_count_mismatches);
        step("volume", health.corruption_errors == 0 && health.internal_faults == 0);
    }
    else
        step("volume", FALSE);

    /* 2. A clone, then a copy. */
    {
        uint64_t before;
        uint64_t after_clone;
        uint64_t after_copy;
        uint64_t started;
        uint64_t clone_us;
        uint64_t copy_us;
        BPTR source;
        BOOL cloned = FALSE;
        BOOL copied;

        if (!write_file((CONST_STRPTR)original, chunk))
        {
            step("clone", FALSE);
            goto watch;
        }
        before = used_bytes(drawer);
        source = Lock((CONST_STRPTR)original, SHARED_LOCK);
        started = now_microseconds();
        if (source != BNULL)
        {
            cloned = afsplus_client_clone_file(source, drawer,
                (CONST_STRPTR)"tour-clone") == 0;
            UnLock(source);
        }
        clone_us = now_microseconds() - started;
        after_clone = used_bytes(drawer);
        started = now_microseconds();
        copied = copy_file((CONST_STRPTR)original, (CONST_STRPTR)copy, chunk);
        copy_us = now_microseconds() - started;
        after_copy = used_bytes(drawer);
        /* The clock may step by milliseconds; a clone inside one step reads
         * as no time at all, and is said so. */
        if (clone_us == 0)
            Printf("\nA 16 MiB file, cloned: at once, below what the clock can"
                " measure, and %lu KiB more in use.\n",
                (ULONG)((after_clone > before ? after_clone - before : 0) >> 10));
        else
            Printf("\nA 16 MiB file, cloned: %lu ms and %lu KiB more in use.\n",
                (ULONG)((clone_us + 500) / 1000),
                (ULONG)((after_clone > before ? after_clone - before : 0) >> 10));
        Printf("The same file copied byte by byte: %lu ms and %lu KiB more.\n",
            (ULONG)((copy_us + 500) / 1000),
            (ULONG)((after_copy > after_clone ? after_copy - after_clone : 0) >> 10));
        Printf("The clone shares the blocks until one side changes; a write to either"
            " leaves the other as it was.\n");
        step("clone", cloned && copied
            && after_clone - before < TOUR_BYTES / 16
            && after_copy - after_clone >= TOUR_BYTES / 2);
    }

watch:
    /* 3. A watch sees the drawer change. */
    {
        uint64_t watch = 0;
        uint32_t changed = 0;
        BOOL watched_ok = afsplus_client_watch_add(drawer, NULL, &watch) == 0;
        BPTR file;

        if (watched_ok)
        {
            file = Open((CONST_STRPTR)watched, MODE_NEWFILE);
            if (file != BNULL)
                Close(file);
            watched_ok = afsplus_client_watch_take(port, watch, &changed) == 0
                && changed == 1;
            afsplus_client_watch_remove(port, watch);
        }
        Printf("\nA watch on the drawer %s the new file appear, with no polling and no"
            " rescan.\n", watched_ok ? "saw" : "did not see");
        step("watch", watched_ok);
    }

    /* 4. An extended attribute. */
    {
        static const char value[] = "written by AFSPlusTour";
        char back[64];
        uint32_t required = 0;
        BPTR file = Lock((CONST_STRPTR)original, SHARED_LOCK);
        BOOL attribute_ok = file != BNULL
            && afsplus_client_set_attribute(file, (CONST_STRPTR)"user.tour", value,
                (uint32_t)sizeof(value) - 1, AFSPLUS_AROS_ATTRIBUTE_UPSERT) == 0
            && afsplus_client_get_attribute(file, (CONST_STRPTR)"user.tour", back,
                (uint32_t)sizeof(back), &required) == 0
            && required == sizeof(value) - 1
            && memcmp(back, value, sizeof(value) - 1) == 0;

        if (file != BNULL)
            UnLock(file);
        Printf("\nAn extended attribute, user.tour, %s: the file carries data of its own"
            " beside its content, as on Linux and macOS.\n",
            attribute_ok ? "written and read back" : "could not be written");
        step("attribute", attribute_ok);
    }

    DeleteFile((CONST_STRPTR)watched);
    DeleteFile((CONST_STRPTR)copy);
    DeleteFile((CONST_STRPTR)clone);
    DeleteFile((CONST_STRPTR)original);
}

int main(int argc, char **argv)
{
    BPTR drawer;
    UBYTE *chunk;

    if (argc != 2)
    {
        Printf("usage: AFSPlusTour <drawer on an AFS+ volume>\n");
        return RETURN_ERROR;
    }
    if (OpenDevice((CONST_STRPTR)TIMERNAME, UNIT_MICROHZ,
            (struct IORequest *)&timer_request, 0) != 0)
    {
        Printf("AFSPlusTour: timer.device\n");
        return RETURN_FAIL;
    }
    TimerBase = timer_request.tr_node.io_Device;
    drawer = Lock((CONST_STRPTR)argv[1], SHARED_LOCK);
    chunk = AllocMem(CHUNK, MEMF_ANY | MEMF_CLEAR);
    if (drawer == BNULL || chunk == NULL)
    {
        Printf("AFSPlusTour: cannot use %s\n", argv[1]);
        failures++;
    }
    else
        tour(drawer, (CONST_STRPTR)argv[1], chunk);
    if (chunk != NULL)
        FreeMem(chunk, CHUNK);
    if (drawer != BNULL)
        UnLock(drawer);
    CloseDevice((struct IORequest *)&timer_request);
    Printf("[AFSPLUS-TOUR] %s\n", failures == 0 ? "PASS" : "FAIL");
    return failures == 0 ? RETURN_OK : RETURN_WARN;
}
