/* SPDX-License-Identifier: BSD-2-Clause */

/* AFSPlusBench <drawer> <seed> [trees]: runs the small-file development-tree
 * workload of the benchmark contract in a new drawer and removes it. Each
 * tree is eight drawers of thirty-two files, sizes and bytes drawn from the
 * seed; every phase runs over all trees before the next begins and is timed
 * once: create, list, read (every byte compared), rename, delete. The clock
 * moves in steps ("clock step_us <n>"), so a phase long against the step
 * needs enough trees (default 10, at most 99). One line per phase ("phase
 * <name> ops <n> us <time>", and when an AFS+ handler serves the drawer
 * also what the phase cost it: "calls <n> flushes <n> writes <n>
 * cache_reads <n>"), and that handler's counters before and after
 * ("counters <when> ..."); another file system says "counters none". The
 * last line is PASS or FAIL.
 *
 * AFSPlusBench FORMAT <device:> <name> formats the volume of a device with
 * the Fast File System, the baseline the same workload runs against. */

#include <devices/timer.h>
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <exec/memory.h>
#include <proto/dos.h>
#include <proto/exec.h>
#include <proto/timer.h>

#include <string.h>

#include "../client/afsplus_client.h"

#define DRAWERS 8
#define FILES_PER_DRAWER 32
#define FILES (DRAWERS * FILES_PER_DRAWER)
#define TREES_DEFAULT 10
#define TREES_MAX 99
#define LARGEST 16384
#define PATH_MAX_BYTES 256

/* timer.device, opened for ReadEClock: the system time moves by the tick,
 * the E-clock by far less. The posixc clock would do, but a program calling
 * posixc does not start on every profile the handler runs on. */
struct Device *TimerBase;
static struct timerequest timer_request;

static UBYTE file_bytes[LARGEST];
static UBYTE read_bytes[LARGEST];

static int fail(const char *stage, LONG detail)
{
    Printf("[AFSPLUS-BENCH] FAIL %s detail %ld error %ld\n", stage, detail,
        IoErr());
    return RETURN_FAIL;
}

static uint32_t next_random(uint32_t *state)
{
    uint32_t value = *state;

    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    *state = value;
    return value;
}

/* The size and first random state of file index, from the seed alone. */
static uint32_t file_state(uint32_t seed, uint32_t index)
{
    uint32_t state = seed ^ (index * UINT32_C(2654435761)) ^ UINT32_C(0x9e3779b9);

    return state != 0 ? state : 1;
}

static uint32_t file_size(uint32_t seed, uint32_t index)
{
    uint32_t state = file_state(seed, index);
    uint32_t value = next_random(&state);

    /* Most files of a source tree are small; one in four is larger. */
    return (value >> 30) == 0 ? value % (LARGEST + 1) : value % 1024;
}

static void fill(uint32_t seed, uint32_t index, UBYTE *bytes, uint32_t size)
{
    uint32_t state = file_state(seed, index) ^ UINT32_C(0x5bd1e995);
    uint32_t at;

    if (state == 0)
        state = 1;
    for (at = 0; at < size; at++)
        bytes[at] = (UBYTE)next_random(&state);
}

static void tree_path(char *out, const char *root, uint32_t tree)
{
    size_t length = strlen(root);

    memcpy(out, root, length);
    out[length++] = '/';
    out[length++] = 't';
    out[length++] = (char)('0' + tree / 10);
    out[length++] = (char)('0' + tree % 10);
    out[length] = 0;
}

/* Drawer number drawer counts across trees: DRAWERS to a tree. */
static void drawer_path(char *out, const char *root, uint32_t drawer)
{
    size_t length;

    tree_path(out, root, drawer / DRAWERS);
    length = strlen(out);
    out[length++] = '/';
    out[length++] = 'd';
    out[length++] = (char)('0' + drawer % DRAWERS);
    out[length] = 0;
}

/* File number index counts across trees: FILES to a tree. */
static void file_path(char *out, const char *root, uint32_t index,
    int renamed)
{
    size_t length;

    drawer_path(out, root, index / FILES_PER_DRAWER);
    length = strlen(out);
    out[length++] = '/';
    out[length++] = 'f';
    out[length++] = (char)('0' + (index % FILES_PER_DRAWER) / 10);
    out[length++] = (char)('0' + (index % FILES_PER_DRAWER) % 10);
    out[length++] = '.';
    out[length++] = renamed ? 'o' : 'c';
    out[length] = 0;
}

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

/* The smallest step the clock takes, from three successive changes. */
static uint64_t clock_step(void)
{
    uint64_t step = UINT64_MAX;
    uint64_t last = now_microseconds();
    int changes = 0;

    while (changes < 3)
    {
        uint64_t now = now_microseconds();

        if (now != last)
        {
            if (now - last < step)
                step = now - last;
            last = now;
            changes++;
        }
    }
    return step;
}

static ULONG clamp(uint64_t value)
{
    return value > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : (ULONG)value;
}

static const char *decimal(char *out, uint64_t value);

/* The handler's counters at a phase boundary, all zero where the volume
 * has none, so that a phase line can carry what the phase cost the device
 * and not only what it cost the clock. */
static struct AfsplusArosCounters phase_counters(struct MsgPort *port)
{
    struct AfsplusArosCounters counters;

    memset(&counters, 0, sizeof(counters));
    if (port != NULL && afsplus_client_counters(port, &counters) != 0)
        memset(&counters, 0, sizeof(counters));
    return counters;
}

static void report_phase(const char *name, ULONG operations, uint64_t start,
    struct MsgPort *port, const struct AfsplusArosCounters *at_start)
{
    struct AfsplusArosCounters now = phase_counters(port);
    char text[4][21];

    if (port == NULL)
    {
        Printf("[AFSPLUS-BENCH] phase %s ops %lu us %lu\n", name, operations,
            clamp(now_microseconds() - start));
        return;
    }
    Printf("[AFSPLUS-BENCH] phase %s ops %lu us %lu calls %s flushes %s"
        " writes %s cache_reads %s\n", name, operations,
        clamp(now_microseconds() - start),
        decimal(text[0], now.calls - at_start->calls),
        decimal(text[1], now.device_flushes - at_start->device_flushes),
        decimal(text[2], now.device_writes - at_start->device_writes),
        decimal(text[3], now.cache_hits + now.cache_misses
            - at_start->cache_hits - at_start->cache_misses));
}

static int write_one(const char *path, const UBYTE *bytes, uint32_t size)
{
    BPTR file = Open((CONST_STRPTR)path, MODE_NEWFILE);
    LONG written;

    if (file == BNULL)
        return 0;
    written = size != 0 ? Write(file, (APTR)bytes, (LONG)size) : 0;
    if (!Close(file))
        return 0;
    return written == (LONG)size;
}

static int read_matches(const char *path, const UBYTE *expected,
    uint32_t size)
{
    BPTR file = Open((CONST_STRPTR)path, MODE_OLDFILE);
    LONG got;

    if (file == BNULL)
        return 0;
    /* One byte more than the file holds: a longer file shows. */
    got = Read(file, read_bytes, (LONG)size + (size < LARGEST ? 1 : 0));
    Close(file);
    return got == (LONG)size && memcmp(read_bytes, expected, size) == 0;
}

static ULONG list_drawer(const char *path, int *ok)
{
    struct FileInfoBlock *fib = AllocDosObject(DOS_FIB, NULL);
    BPTR lock = Lock((CONST_STRPTR)path, SHARED_LOCK);
    ULONG entries = 0;

    *ok = fib != NULL && lock != BNULL && Examine(lock, fib);
    while (*ok && ExNext(lock, fib))
        entries++;
    if (*ok && IoErr() != ERROR_NO_MORE_ENTRIES)
        *ok = 0;
    if (lock != BNULL)
        UnLock(lock);
    if (fib != NULL)
        FreeDosObject(DOS_FIB, fib);
    return entries;
}

/* A counter in full: dos.library formats 32 bits at most. */
static const char *decimal(char *out, uint64_t value)
{
    char digits[21];
    int count = 0;
    int at = 0;

    do
    {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value != 0);
    while (count > 0)
        out[at++] = digits[--count];
    out[at] = 0;
    return out;
}

static void print_counters(const char *when, struct MsgPort *port)
{
    struct AfsplusArosCounters counters;
    char text[12][21];

    memset(&counters, 0, sizeof(counters));
    if (port == NULL || afsplus_client_counters(port, &counters) != 0)
    {
        Printf("[AFSPLUS-BENCH] counters none\n");
        return;
    }
    Printf("[AFSPLUS-BENCH] counters %s calls %s failed %s reads %s"
        " writes %s flushes %s read_bytes %s written_bytes %s"
        " heap %s heap_peak %s cache_blocks %s cache_hits %s cache_misses %s\n",
        when, decimal(text[0], counters.calls),
        decimal(text[1], counters.failed_calls),
        decimal(text[2], counters.device_reads),
        decimal(text[3], counters.device_writes),
        decimal(text[4], counters.device_flushes),
        decimal(text[5], counters.device_read_bytes),
        decimal(text[6], counters.device_written_bytes),
        decimal(text[7], counters.heap_bytes),
        decimal(text[8], counters.heap_peak_bytes),
        decimal(text[9], counters.cache_blocks),
        decimal(text[10], counters.cache_hits),
        decimal(text[11], counters.cache_misses));
}

static uint32_t parse_number(const char *text, int *ok)
{
    uint32_t value = 0;

    *ok = *text != 0;
    while (*ok && *text != 0)
    {
        if (*text < '0' || *text > '9' || value > UINT32_C(429496728))
            *ok = 0;
        else
            value = value * 10 + (uint32_t)(*text++ - '0');
    }
    return value;
}

static int run(const char *root, uint32_t seed, uint32_t trees)
{
    char path[PATH_MAX_BYTES];
    char other[PATH_MAX_BYTES];
    struct MsgPort *port = NULL;
    struct AfsplusArosCounters at_start;
    uint64_t total = 0;
    uint64_t start;
    uint32_t index;
    uint32_t drawers = trees * DRAWERS;
    uint32_t files = trees * FILES;
    BPTR lock;
    int ok = 1;

    if (strlen(root) + 16 > PATH_MAX_BYTES)
        return fail("drawer path too long", (LONG)strlen(root));
    for (index = 0; index < files; index++)
        total += file_size(seed, index);
    Printf("[AFSPLUS-BENCH] clock step_us %lu\n", clamp(clock_step()));
    Printf("[AFSPLUS-BENCH] workload devtree seed %lu trees %lu drawers %lu"
        " files %lu bytes %lu\n", (ULONG)seed, (ULONG)trees, (ULONG)drawers,
        (ULONG)files, clamp(total));

    lock = CreateDir((CONST_STRPTR)root);
    if (lock == BNULL)
        return fail("create the run drawer", 0);
    port = afsplus_client_lock_port(lock);
    UnLock(lock);
    print_counters("before", port);

    start = now_microseconds();
    at_start = phase_counters(port);
    for (index = 0; ok && index < trees; index++)
    {
        tree_path(path, root, index);
        lock = CreateDir((CONST_STRPTR)path);
        if (lock == BNULL)
            return fail("create tree", (LONG)index);
        UnLock(lock);
    }
    for (index = 0; ok && index < drawers; index++)
    {
        drawer_path(path, root, index);
        lock = CreateDir((CONST_STRPTR)path);
        if (lock == BNULL)
            return fail("create drawer", (LONG)index);
        UnLock(lock);
    }
    for (index = 0; ok && index < files; index++)
    {
        uint32_t size = file_size(seed, index);

        fill(seed, index, file_bytes, size);
        file_path(path, root, index, 0);
        if (!write_one(path, file_bytes, size))
            return fail("create file", (LONG)index);
    }
    report_phase("create", trees + drawers + files, start, port, &at_start);

    start = now_microseconds();
    at_start = phase_counters(port);
    for (index = 0; index < drawers; index++)
    {
        ULONG entries;

        drawer_path(path, root, index);
        entries = list_drawer(path, &ok);
        if (!ok)
            return fail("list drawer", (LONG)index);
        if (entries != FILES_PER_DRAWER)
            return fail("entries listed", (LONG)entries);
    }
    report_phase("list", files, start, port, &at_start);

    start = now_microseconds();
    at_start = phase_counters(port);
    for (index = 0; index < files; index++)
    {
        uint32_t size = file_size(seed, index);

        fill(seed, index, file_bytes, size);
        file_path(path, root, index, 0);
        if (!read_matches(path, file_bytes, size))
            return fail("read back", (LONG)index);
    }
    report_phase("read", files, start, port, &at_start);

    start = now_microseconds();
    at_start = phase_counters(port);
    for (index = 0; index < files; index++)
    {
        file_path(path, root, index, 0);
        file_path(other, root, index, 1);
        if (!Rename((CONST_STRPTR)path, (CONST_STRPTR)other))
            return fail("rename", (LONG)index);
    }
    report_phase("rename", files, start, port, &at_start);

    start = now_microseconds();
    at_start = phase_counters(port);
    for (index = 0; index < files; index++)
    {
        file_path(path, root, index, 1);
        if (!DeleteFile((CONST_STRPTR)path))
            return fail("delete file", (LONG)index);
    }
    for (index = 0; index < drawers; index++)
    {
        drawer_path(path, root, index);
        if (!DeleteFile((CONST_STRPTR)path))
            return fail("delete drawer", (LONG)index);
    }
    for (index = 0; index < trees; index++)
    {
        tree_path(path, root, index);
        if (!DeleteFile((CONST_STRPTR)path))
            return fail("delete tree", (LONG)index);
    }
    report_phase("delete", trees + drawers + files, start, port, &at_start);

    if (!DeleteFile((CONST_STRPTR)root))
        return fail("remove the run drawer", 0);
    print_counters("after", port);
    Printf("[AFSPLUS-BENCH] PASS\n");
    return RETURN_OK;
}

static int format_volume(const char *device, const char *name)
{
    int status = RETURN_OK;

    if (!Inhibit((CONST_STRPTR)device, DOSTRUE))
        return fail("inhibit", 0);
    if (!Format((CONST_STRPTR)device, (CONST_STRPTR)name, ID_FFS_DISK))
        status = fail("format", 0);
    if (!Inhibit((CONST_STRPTR)device, DOSFALSE) && status == RETURN_OK)
        status = fail("uninhibit", 0);
    if (status == RETURN_OK)
        Printf("[AFSPLUS-BENCH] formatted %s\n", name);
    return status;
}

int main(int argc, char **argv)
{
    uint32_t seed;
    uint32_t trees = TREES_DEFAULT;
    int ok;
    int status;

    if (argc == 4 && strcmp(argv[1], "FORMAT") == 0)
        return format_volume(argv[2], argv[3]);
    if (argc != 3 && argc != 4)
    {
        Printf("Usage: AFSPlusBench <drawer> <seed> [trees]\n"
            "       AFSPlusBench FORMAT <device:> <name>\n");
        return RETURN_ERROR;
    }
    seed = parse_number(argv[2], &ok);
    if (!ok)
        return fail("seed", 0);
    if (argc == 4)
    {
        trees = parse_number(argv[3], &ok);
        if (!ok || trees == 0 || trees > TREES_MAX)
            return fail("trees", (LONG)trees);
    }
    if (OpenDevice((CONST_STRPTR)TIMERNAME, UNIT_MICROHZ,
            (struct IORequest *)&timer_request, 0) != 0)
        return fail("open timer.device", 0);
    TimerBase = timer_request.tr_node.io_Device;
    status = run(argv[1], seed, trees);
    CloseDevice((struct IORequest *)&timer_request);
    return status;
}
