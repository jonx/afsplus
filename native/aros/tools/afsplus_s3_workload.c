/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * AFSPlusS3Workload <control> <progress> <root> <copydir> -- the S3 workload
 * that runs from the Startup-Sequence of an AFS+ boot volume and is cut off
 * in the middle of it (tools/check-hosted-aros-s3.sh).
 *
 * What it does is read from <control>, the one line the host writes before
 * the boot:
 *
 *   work <round> <seed> [noflush]   a round of real work
 *   verify <round> <seed>           copy the markers out for the host to judge
 *
 * A work round churns files (write, rename, delete the previous pass's),
 * saves a preference under ENVARC:, then writes two markers into
 * <root>/Markers:
 *
 *   m<round>.flush  written, closed and made durable with ACTION_FLUSH, the
 *                   packet dos.library sends for a whole-volume flush; the
 *                   promise this check holds the handler to.
 *   m<round>.soft   written and closed and nothing more; a cut may keep it or
 *                   lose it, but must never leave it half written.
 *
 * Both hold the same bytes for a given round and seed: a header naming the
 * round and the hash of the payload, 4096 deterministic payload bytes, and a
 * trailer. The host builds the same bytes (tools/s3-markers.py) and compares.
 *
 * With `noflush` the claim comes first and nothing backs it: the line saying
 * the marker is flushed is printed, a second passes, and only then is the
 * marker written, with no flush at all. That is the negative control. It is
 * built this way because on the Hosted boot mount, skipping the flush alone
 * loses nothing: the mount cannot open timer.device while it is being made,
 * so it runs SYNC (afsplus_handler.c, "a volume that cannot delay stays
 * SYNC"), and every packet is already durable when it is answered. A claim
 * with nothing behind it is what a check must catch, and the second of delay
 * is the window every cut of the control falls into, so the control fails on
 * every round instead of on the ones that get unlucky.
 *
 * Every step prints a line to <progress>, unbuffered, so the host can time a
 * cut on a phase instead of on a sleep:
 *
 *   [AFSPLUS-S3] phase <name>
 *   [AFSPLUS-S3] churn <pass> files <n>
 *   [AFSPLUS-S3] marker <round> flushed
 *   [AFSPLUS-S3] marker <round> written
 *   [AFSPLUS-S3] idle <beat>
 *   [AFSPLUS-S3] markers <n> round <r>
 *   [AFSPLUS-S3] PASS, or [AFSPLUS-S3] FAIL <stage> error <n>
 *
 * dos.library and timer.device only: a program that calls posixc does not
 * start on every profile the handler runs on.
 */

#include <devices/timer.h>
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <proto/exec.h>
#include <proto/timer.h>

#include <stdint.h>
#include <string.h>

#include "../client/afsplus_client.h"

#define MARKER_BYTES 4096u
#define MARKER_FRAME 128u
/* One size for every text buffer, so the appenders need only one limit. */
#define TEXT_BYTES 256u
#define CHURN_FILES 32u
#define CHURN_BYTES 8192u
#define CHURN_MIN_US UINT64_C(6000000)
#define CHURN_MAX_PASSES 24u
#define IDLE_BEATS 16u
#define IDLE_TICKS 25u /* Delay() ticks of 1/50 s: half a second a beat */
#define CLAIM_TICKS 50u /* the negative control's empty second */
#define COPY_BYTES 8192u

struct Device *TimerBase;
static struct timerequest timer_request;

static BPTR progress_file;
static UBYTE payload_bytes[MARKER_BYTES];
static UBYTE marker_bytes[MARKER_BYTES + MARKER_FRAME];
static UBYTE churn_bytes[CHURN_BYTES];
static UBYTE copy_bytes[COPY_BYTES];
static char text_line[TEXT_BYTES];

/* --- small formatting, so every line is built as bytes we control --- */

static size_t append_text(char *out, size_t at, const char *text)
{
    while (*text != 0 && at + 1u < TEXT_BYTES)
        out[at++] = *text++;
    out[at] = 0;
    return at;
}

static size_t append_number(char *out, size_t at, uint32_t value)
{
    char digits[11];
    int count = 0;

    do
    {
        digits[count++] = (char)('0' + (char)(value % 10u));
        value /= 10u;
    } while (value != 0);
    while (count > 0 && at + 1u < TEXT_BYTES)
        out[at++] = digits[--count];
    out[at] = 0;
    return at;
}

static size_t append_hex8(char *out, size_t at, uint32_t value)
{
    static const char hex[] = "0123456789abcdef";
    int shift;

    for (shift = 28; shift >= 0 && at + 1u < TEXT_BYTES; shift -= 4)
        out[at++] = hex[(value >> shift) & 0xfu];
    out[at] = 0;
    return at;
}

/* A line reaches the host file now, not when a buffer fills: Write() is the
 * packet, FPuts() would sit in dos.library's buffer until the program ends. */
static void emit(size_t length)
{
    if (progress_file != BNULL)
        Write(progress_file, (APTR)text_line, (LONG)length);
    PutStr((CONST_STRPTR)text_line);
}

static void say_phase(const char *name)
{
    size_t at = append_text(text_line, 0, "[AFSPLUS-S3] phase ");

    at = append_text(text_line, at, name);
    at = append_text(text_line, at, "\n");
    emit(at);
}

/* "[AFSPLUS-S3] <what> <first>" and, when middle is given, " <middle> <second>". */
static void say_count(const char *what, uint32_t first, const char *middle,
    uint32_t second)
{
    size_t at = append_text(text_line, 0, "[AFSPLUS-S3] ");

    at = append_text(text_line, at, what);
    at = append_text(text_line, at, " ");
    at = append_number(text_line, at, first);
    if (middle != NULL)
    {
        at = append_text(text_line, at, " ");
        at = append_text(text_line, at, middle);
        at = append_text(text_line, at, " ");
        at = append_number(text_line, at, second);
    }
    at = append_text(text_line, at, "\n");
    emit(at);
}

/* "[AFSPLUS-S3] marker <round> flushed|written". */
static void say_marker(uint32_t round, const char *state)
{
    size_t at = append_text(text_line, 0, "[AFSPLUS-S3] marker ");

    at = append_number(text_line, at, round);
    at = append_text(text_line, at, " ");
    at = append_text(text_line, at, state);
    at = append_text(text_line, at, "\n");
    emit(at);
}

static void say_line(const char *whole)
{
    size_t at = append_text(text_line, 0, whole);

    at = append_text(text_line, at, "\n");
    emit(at);
}

static int fail(const char *stage)
{
    size_t at = append_text(text_line, 0, "[AFSPLUS-S3] FAIL ");

    at = append_text(text_line, at, stage);
    at = append_text(text_line, at, " error ");
    at = append_number(text_line, at, (uint32_t)IoErr());
    at = append_text(text_line, at, "\n");
    emit(at);
    return RETURN_FAIL;
}

/* --- paths --- */

static void join(char *out, const char *drawer, const char *name)
{
    size_t length = strlen(drawer);

    if (length >= TEXT_BYTES)
        length = TEXT_BYTES - 1u;
    memcpy(out, drawer, length);
    out[length] = 0;
    AddPart((STRPTR)out, (CONST_STRPTR)name, (ULONG)TEXT_BYTES);
}

/* --- the marker bytes, built the same way on the host --- */

static uint32_t next_random(uint32_t *state)
{
    uint32_t value = *state;

    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    *state = value;
    return value;
}

static uint32_t stream_state(uint32_t seed, uint32_t round)
{
    uint32_t state = seed ^ (round * UINT32_C(2654435761))
        ^ UINT32_C(0x9e3779b9);

    return state != 0 ? state : 1u;
}

/* Fills marker_bytes and returns the length. The header carries the round
 * and the FNV-1a hash of the payload, so half a file is recognisable as half
 * a file and never as another round's. */
static uint32_t build_marker(uint32_t seed, uint32_t round)
{
    uint32_t state = stream_state(seed, round);
    uint32_t hash = UINT32_C(2166136261);
    uint32_t at;
    size_t header;
    size_t trailer;

    for (at = 0; at < MARKER_BYTES; at++)
    {
        UBYTE byte = (UBYTE)next_random(&state);

        payload_bytes[at] = byte;
        hash = (hash ^ (uint32_t)byte) * UINT32_C(16777619);
    }
    header = append_text(text_line, 0, "AFSPLUS-S3 marker ");
    header = append_number(text_line, header, round);
    header = append_text(text_line, header, " hash ");
    header = append_hex8(text_line, header, hash);
    header = append_text(text_line, header, "\n");
    memcpy(marker_bytes, text_line, header);
    memcpy(&marker_bytes[header], payload_bytes, MARKER_BYTES);
    trailer = append_text(text_line, 0, "AFSPLUS-S3 end ");
    trailer = append_number(text_line, trailer, round);
    trailer = append_text(text_line, trailer, "\n");
    memcpy(&marker_bytes[header + MARKER_BYTES], text_line, trailer);
    return (uint32_t)header + MARKER_BYTES + (uint32_t)trailer;
}

/* --- the work --- */

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

static int write_whole(const char *path, const UBYTE *bytes, uint32_t length,
    uint32_t pieces)
{
    BPTR file = Open((CONST_STRPTR)path, MODE_NEWFILE);
    uint32_t at = 0;
    int ok = file != BNULL;

    while (ok && at < length)
    {
        uint32_t piece = length - at;

        if (pieces > 1u && piece > length / pieces + 1u)
            piece = length / pieces + 1u;
        ok = Write(file, (APTR)&bytes[at], (LONG)piece) == (LONG)piece;
        at += piece;
    }
    if (file != BNULL && !Close(file))
        ok = 0;
    return ok;
}

static void churn_name(char *out, const char *prefix, uint32_t index,
    const char *suffix)
{
    size_t at = append_text(out, 0, prefix);

    at = append_number(out, at, index);
    (void)append_text(out, at, suffix);
}

static int churn(const char *work, uint32_t seed, uint32_t round)
{
    char path[TEXT_BYTES];
    char other[TEXT_BYTES];
    char name[TEXT_BYTES];
    uint64_t start = now_microseconds();
    uint32_t pass;

    for (pass = 0; pass < CHURN_MAX_PASSES; pass++)
    {
        uint32_t state = stream_state(seed ^ (pass + 1u), round);
        uint32_t index;

        for (index = 0; index < CHURN_FILES; index++)
        {
            uint32_t size = next_random(&state) % CHURN_BYTES + 1u;
            uint32_t at;

            for (at = 0; at < size; at += 8u)
                churn_bytes[at] = (UBYTE)next_random(&state);
            churn_name(name, "w", index, "");
            join(path, work, name);
            churn_name(name, "w", index, ".dat");
            join(other, work, name);
            /* The file of the pass, and of the round, before: a delayed-
             * commit mount leaves the deletes for idle time, which is one of
             * the moments the cut schedule aims at. */
            (void)DeleteFile((CONST_STRPTR)other);
            if (!write_whole(path, churn_bytes, size, 1u))
                return fail("churn write");
            if (!Rename((CONST_STRPTR)path, (CONST_STRPTR)other))
                return fail("churn rename");
        }
        say_count("churn", pass, "files", CHURN_FILES);
        if (now_microseconds() - start >= CHURN_MIN_US)
            break;
    }
    return RETURN_OK;
}

static int save_preference(uint32_t round)
{
    size_t length = append_text(text_line, 0, "AFSPlusS3 round ");

    length = append_number(text_line, length, round);
    length = append_text(text_line, length, "\n");
    if (!write_whole("ENVARC:AFSPlusS3", (const UBYTE *)text_line,
            (uint32_t)length, 1u))
        return fail("save the preference");
    return RETURN_OK;
}

static BPTR open_drawer(const char *path)
{
    BPTR lock = Lock((CONST_STRPTR)path, SHARED_LOCK);

    if (lock == BNULL)
        lock = CreateDir((CONST_STRPTR)path);
    return lock;
}

static int work_round(const char *root, uint32_t round, uint32_t seed,
    int noflush)
{
    char markers[TEXT_BYTES];
    char work[TEXT_BYTES];
    char path[TEXT_BYTES];
    char name[TEXT_BYTES];
    uint32_t length;
    uint32_t beat;
    BPTR lock;
    struct MsgPort *port;
    int status;

    join(markers, root, "Markers");
    join(work, root, "Work");
    lock = open_drawer(root);
    if (lock == BNULL)
        return fail("the run drawer");
    port = afsplus_client_lock_port(lock);
    UnLock(lock);
    if (port == NULL)
        return fail("the run drawer names no handler");
    lock = open_drawer(markers);
    if (lock == BNULL)
        return fail("the marker drawer");
    UnLock(lock);
    lock = open_drawer(work);
    if (lock == BNULL)
        return fail("the work drawer");
    UnLock(lock);

    say_phase("churn");
    status = churn(work, seed, round);
    if (status != RETURN_OK)
        return status;

    say_phase("prefs");
    status = save_preference(round);
    if (status != RETURN_OK)
        return status;

    say_phase("marker");
    length = build_marker(seed, round);
    churn_name(name, "m", round, ".flush");
    join(path, markers, name);
    /* The line the host times the "after-flush" cut on. */
    if (noflush)
    {
        say_marker(round, "flushed");
        Delay(CLAIM_TICKS);
        if (!write_whole(path, marker_bytes, length, 4u))
            return fail("write the flushed marker");
    }
    else
    {
        if (!write_whole(path, marker_bytes, length, 4u))
            return fail("write the flushed marker");
        /* ACTION_FLUSH: the whole-volume flush of dos.library. The handler
         * has committed and the device barrier has passed when it answers. */
        if (!DoPkt(port, ACTION_FLUSH, 0, 0, 0, 0, 0))
            return fail("flush the volume");
        say_marker(round, "flushed");
    }

    churn_name(name, "m", round, ".soft");
    join(path, markers, name);
    if (!write_whole(path, marker_bytes, length, 1u))
        return fail("write the soft marker");
    say_marker(round, "written");

    say_phase("idle");
    for (beat = 0; beat < IDLE_BEATS; beat++)
    {
        Delay(IDLE_TICKS);
        say_count("idle", beat, NULL, 0);
    }
    say_phase("worked");
    return RETURN_OK;
}

static int copy_out(const char *from, const char *to)
{
    BPTR in = Open((CONST_STRPTR)from, MODE_OLDFILE);
    BPTR out;
    int ok;

    if (in == BNULL)
        return 0;
    out = Open((CONST_STRPTR)to, MODE_NEWFILE);
    if (out == BNULL)
    {
        Close(in);
        return 0;
    }
    ok = 1;
    for (;;)
    {
        LONG got = Read(in, copy_bytes, (LONG)COPY_BYTES);

        if (got < 0)
        {
            ok = 0;
            break;
        }
        if (got == 0)
            break;
        if (Write(out, copy_bytes, got) != got)
        {
            ok = 0;
            break;
        }
    }
    if (!Close(out))
        ok = 0;
    Close(in);
    return ok;
}

static int verify_round(const char *root, const char *copydir, uint32_t round)
{
    char markers[TEXT_BYTES];
    char from[TEXT_BYTES];
    char to[TEXT_BYTES];
    struct FileInfoBlock *fib;
    BPTR lock;
    BPTR made;
    uint32_t copied = 0;
    int ok;

    join(markers, root, "Markers");
    made = CreateDir((CONST_STRPTR)copydir);
    if (made != BNULL)
        UnLock(made);
    say_phase("verify");
    lock = Lock((CONST_STRPTR)markers, SHARED_LOCK);
    if (lock == BNULL)
    {
        /* No marker drawer at all is a fact for the host, not a crash: the
         * first round has not written one yet. */
        say_count("markers", 0, "round", round);
        return RETURN_OK;
    }
    fib = AllocDosObject(DOS_FIB, NULL);
    if (fib == NULL)
    {
        UnLock(lock);
        return fail("AllocDosObject");
    }
    ok = Examine(lock, fib) != DOSFALSE;
    while (ok && ExNext(lock, fib))
    {
        join(from, markers, (const char *)fib->fib_FileName);
        join(to, copydir, (const char *)fib->fib_FileName);
        if (!copy_out(from, to))
        {
            ok = 0;
            break;
        }
        copied++;
    }
    if (ok && IoErr() != ERROR_NO_MORE_ENTRIES)
        ok = 0;
    FreeDosObject(DOS_FIB, fib);
    UnLock(lock);
    if (!ok)
        return fail("copy the markers out");
    join(to, copydir, "envarc-prefs");
    (void)copy_out("ENVARC:AFSPlusS3", to);
    say_count("markers", copied, "round", round);
    return RETURN_OK;
}

/* --- the control file --- */

static const char *word(const char *at, char *out)
{
    size_t length = 0;

    while (*at == ' ' || *at == '\t')
        at++;
    while (*at != 0 && *at != ' ' && *at != '\t' && *at != '\n'
        && *at != '\r' && length + 1u < TEXT_BYTES)
        out[length++] = *at++;
    out[length] = 0;
    return at;
}

static uint32_t number(const char *text, int *ok)
{
    uint32_t value = 0;

    if (*text == 0)
        *ok = 0;
    while (*text != 0)
    {
        if (*text < '0' || *text > '9')
        {
            *ok = 0;
            return 0;
        }
        value = value * 10u + (uint32_t)(*text++ - '0');
    }
    return value;
}

static int run(const char *control, const char *progress, const char *root,
    const char *copydir)
{
    char text[TEXT_BYTES];
    char item[TEXT_BYTES];
    const char *at;
    uint32_t round;
    uint32_t seed;
    int working;
    int noflush = 0;
    int ok = 1;
    LONG got;
    BPTR file = Open((CONST_STRPTR)control, MODE_OLDFILE);

    if (file == BNULL)
        return fail("open the control file");
    got = Read(file, text, (LONG)(TEXT_BYTES - 1u));
    Close(file);
    if (got <= 0)
        return fail("read the control file");
    text[got] = 0;

    progress_file = Open((CONST_STRPTR)progress, MODE_NEWFILE);
    if (progress_file == BNULL)
        return fail("open the progress file");

    at = word(text, item);
    working = strcmp(item, "work") == 0;
    if (!working && strcmp(item, "verify") != 0)
        return fail("the control file names no mode");
    at = word(at, item);
    round = number(item, &ok);
    if (!ok)
        return fail("the control file names no round");
    at = word(at, item);
    seed = number(item, &ok);
    if (!ok)
        return fail("the control file names no seed");
    (void)word(at, item);
    if (strcmp(item, "noflush") == 0)
        noflush = 1;

    if (working)
        return work_round(root, round, seed, noflush);
    return verify_round(root, copydir, round);
}

int main(int argc, char **argv)
{
    int status;

    if (argc != 5)
    {
        Printf("Usage: AFSPlusS3Workload <control> <progress> <root>"
            " <copydir>\n");
        return RETURN_ERROR;
    }
    if (OpenDevice((CONST_STRPTR)TIMERNAME, UNIT_MICROHZ,
            (struct IORequest *)&timer_request, 0) != 0)
    {
        Printf("[AFSPLUS-S3] FAIL open timer.device\n");
        return RETURN_FAIL;
    }
    TimerBase = timer_request.tr_node.io_Device;
    status = run(argv[1], argv[2], argv[3], argv[4]);
    CloseDevice((struct IORequest *)&timer_request);
    if (status == RETURN_OK)
        say_line("[AFSPLUS-S3] PASS");
    if (progress_file != BNULL)
        Close(progress_file);
    return status;
}
