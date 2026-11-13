/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * Host matrix for afsplus_commit_tick.h: which packets make the handler ask
 * the library what is due, against a model of the library's delayed window.
 */

#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "afsplus_commit_tick.h"

/* The AROS assert, which this host build has no C library for. */
void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL);
    abort();
}

/* afsplus_packet.c's number for the positional write. */
#define STUB_WRITE_AT64 2022

/* The library as the handler sees it: a change opens the window at the
 * time it is made, and a call commits a window that is old enough and says
 * whether one is still open. */
struct Model
{
    uint32_t window_open;
    uint32_t opened_at;
    uint32_t committed_at;
    uint32_t calls;
};

#define MODEL_MAX_AGE 50

static uint32_t model_commit_due(struct Model *model, uint32_t now)
{
    model->calls++;
    if (model->window_open && now - model->opened_at >= MODEL_MAX_AGE)
    {
        model->window_open = 0;
        model->committed_at = now;
    }
    return model->window_open;
}

static void model_packet(struct Model *model, LONG type, uint32_t now)
{
    if (afsplus_commit_tick_reads_only(type))
        return;
    if (!model->window_open)
    {
        model->window_open = 1;
        model->opened_at = now;
    }
}

/* One handler as it was, asking after every batch, and one with the rule,
 * fed the same packets at the same times. */
static void same_commits(const LONG *types, uint32_t count)
{
    struct Model before = { 0, 0, 0, 0 };
    struct Model after = { 0, 0, 0, 0 };
    uint32_t pending = 0;
    uint32_t changed = 1;
    uint32_t quiet = 0;
    uint32_t now;

    for (now = 0; now < count; now++)
    {
        model_packet(&before, types[now], now);
        (void)model_commit_due(&before, now);

        model_packet(&after, types[now], now);
        if (!afsplus_commit_tick_reads_only(types[now]))
            changed = 1;
        if (afsplus_commit_tick_due(pending, changed, &quiet))
        {
            pending = model_commit_due(&after, now);
            changed = 0;
        }
        /* A change waits in both or in neither, since the same moment. */
        assert(before.window_open == after.window_open);
        assert(before.opened_at == after.opened_at);
        assert(before.committed_at == after.committed_at);
        /* While a change waits, every batch asks. */
        if (after.window_open)
            assert(pending);
    }
    assert(after.calls <= before.calls);
}

int main(void)
{
    static const LONG reads[] = {
        ACTION_LOCATE_OBJECT, ACTION_FREE_LOCK, ACTION_COPY_DIR,
        ACTION_COPY_DIR_FH, ACTION_PARENT, ACTION_PARENT_FH,
        ACTION_SAME_LOCK, ACTION_EXAMINE_OBJECT, ACTION_EXAMINE_NEXT,
        ACTION_EXAMINE_FH, ACTION_EXAMINE_ALL, ACTION_EXAMINE_ALL_END,
        ACTION_EXAMINE_OBJECT64, ACTION_EXAMINE_NEXT64, ACTION_EXAMINE_FH64,
        AFSPLUS_COMMIT_TICK_EXAMINE_NAME64, ACTION_FINDINPUT, ACTION_READ,
        AFSPLUS_COMMIT_TICK_READ_AT64, ACTION_SEEK, ACTION_SEEK64,
        ACTION_END, ACTION_IS_FILESYSTEM,
    };
    static const LONG changes[] = {
        ACTION_WRITE, STUB_WRITE_AT64, ACTION_FINDOUTPUT, ACTION_FINDUPDATE,
        ACTION_DELETE_OBJECT, ACTION_RENAME_OBJECT, ACTION_CREATE_DIR,
        ACTION_SET_PROTECT, ACTION_SET_COMMENT, ACTION_SET_DATE,
        ACTION_SET_FILE_SIZE, ACTION_MAKE_LINK, ACTION_FLUSH,
        ACTION_RENAME_DISK, ACTION_FORMAT, ACTION_INHIBIT, ACTION_DIE,
        ACTION_FH_FROM_LOCK, ACTION_LOCK_RECORD, ACTION_SET_OWNER, 0, -1,
        99999,
    };
    LONG mixed[1024];
    uint32_t quiet = 0;
    uint32_t calls = 0;
    uint32_t state = 12345;
    uint32_t index;

    for (index = 0; index < sizeof(reads) / sizeof(reads[0]); index++)
        assert(afsplus_commit_tick_reads_only(reads[index]));
    for (index = 0; index < sizeof(changes) / sizeof(changes[0]); index++)
        assert(!afsplus_commit_tick_reads_only(changes[index]));

    /* A change asks at once and work that remains asks every batch. */
    assert(afsplus_commit_tick_due(0, 1, &quiet) && quiet == 0);
    assert(afsplus_commit_tick_due(1, 0, &quiet) && quiet == 0);
    /* Batches that only read ask once in AFSPLUS_COMMIT_TICK_QUIET. */
    for (index = 0; index < 10 * AFSPLUS_COMMIT_TICK_QUIET; index++)
        calls += afsplus_commit_tick_due(0, 0, &quiet);
    assert(calls == 10);
    /* A change in between starts the count again. */
    for (index = 0; index + 1 < AFSPLUS_COMMIT_TICK_QUIET; index++)
        assert(!afsplus_commit_tick_due(0, 0, &quiet));
    assert(afsplus_commit_tick_due(0, 1, &quiet));
    for (index = 0; index + 1 < AFSPLUS_COMMIT_TICK_QUIET; index++)
        assert(!afsplus_commit_tick_due(0, 0, &quiet));
    assert(afsplus_commit_tick_due(0, 0, &quiet));

    /* Mixed traffic at several densities of change commits when the
     * handler that asked after every batch committed. */
    for (uint32_t density = 1; density <= 512; density *= 2)
    {
        for (index = 0; index < 1024; index++)
        {
            state = state * UINT32_C(1664525) + UINT32_C(1013904223);
            mixed[index] = (state >> 8) % density == 0
                ? changes[(state >> 20)
                    % (sizeof(changes) / sizeof(changes[0]))]
                : reads[(state >> 20) % (sizeof(reads) / sizeof(reads[0]))];
        }
        same_commits(mixed, 1024);
    }
    for (index = 0; index < 1024; index++)
        mixed[index] = ACTION_READ;
    same_commits(mixed, 1024);

    printf("commit-tick: %u read types, %u change types\n",
        (unsigned)(sizeof(reads) / sizeof(reads[0])),
        (unsigned)(sizeof(changes) / sizeof(changes[0])));
    return 0;
}
