/* SPDX-License-Identifier: BSD-2-Clause */

/*
 * When the handler asks the library whether delayed changes are due
 * (ADR-121), kept apart from the handler so that a host test can run it.
 *
 * On a mount that delays its commits the handler used to read the clock and
 * call afsplus_aros_commit_due after every batch of packets. A batch that
 * only read cannot have given the library anything to commit, so it asks
 * nothing. The call is made, as it always was, whenever
 *
 *   - the library said at the last call that work remains (changes wait in
 *     the window, or an idle maintenance batch was just done), or
 *   - a packet arrived since the last call that is not known to only read.
 *
 * Every state in which a change waits falls under the first rule, because a
 * change enters the window through a packet of the second rule, and the
 * call after that packet reports it pending until it is committed. The
 * bounds of ADR-121 are therefore the ones the same calls gave before.
 *
 * What a skipped call would have done is idle maintenance only: cleaning
 * deleted files and returning their blocks when the volume has been idle
 * long enough. A volume whose last change committed at once, and which was
 * not yet idle at the call that followed, has such work left while the
 * library reports nothing pending. The third rule covers it: the call is
 * made anyway once every AFSPLUS_COMMIT_TICK_QUIET batches, so that work
 * waits a bounded number of read packets instead of the next change.
 */

#ifndef AFSPLUS_COMMIT_TICK_H
#define AFSPLUS_COMMIT_TICK_H

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <dos/dos64.h>
#include <stdint.h>

/* Quiet batches after which the library is asked whatever happened. */
#define AFSPLUS_COMMIT_TICK_QUIET UINT32_C(64)

/* dos/dos64.h packets, for SDKs that do not name them; afsplus_packet.c
 * carries the same numbers. */
#define AFSPLUS_COMMIT_TICK_READ_AT64 2021
#define AFSPLUS_COMMIT_TICK_EXAMINE_NAME64 2023

/* Whether a packet of this type leaves the volume as it found it: it makes
 * no library call that changes an object, opens the delayed window or
 * creates maintenance work. Anything not named here is taken to change the
 * volume, so a type added to the packet layer is safe until it is listed.
 *
 * ACTION_END is listed although the close of a file that was deleted while
 * open cleans the file up: that commits at once and leaves nothing in the
 * window, and the blocks it frees come back under the third rule above. */
static inline uint32_t afsplus_commit_tick_reads_only(LONG type)
{
    switch (type)
    {
    case ACTION_LOCATE_OBJECT:
    case ACTION_FREE_LOCK:
    case ACTION_COPY_DIR:
    case ACTION_COPY_DIR_FH:
    case ACTION_PARENT:
    case ACTION_PARENT_FH:
    case ACTION_SAME_LOCK:
    case ACTION_EXAMINE_OBJECT:
    case ACTION_EXAMINE_NEXT:
    case ACTION_EXAMINE_FH:
    case ACTION_EXAMINE_ALL:
    case ACTION_EXAMINE_ALL_END:
    case ACTION_EXAMINE_OBJECT64:
    case ACTION_EXAMINE_NEXT64:
    case ACTION_EXAMINE_FH64:
    case AFSPLUS_COMMIT_TICK_EXAMINE_NAME64:
    case ACTION_FINDINPUT:
    case ACTION_READ:
    case AFSPLUS_COMMIT_TICK_READ_AT64:
    case ACTION_SEEK:
    case ACTION_SEEK64:
    case ACTION_END:
    case ACTION_IS_FILESYSTEM:
        return 1;
    default:
        return 0;
    }
}

/* Whether the library is asked now. `pending` is what it answered last,
 * `changed` whether a packet that may change the volume arrived since, and
 * `quiet` counts the batches that asked nothing; it is reset by a call. */
static inline uint32_t afsplus_commit_tick_due(uint32_t pending,
    uint32_t changed, uint32_t *quiet)
{
    if (pending || changed || ++*quiet >= AFSPLUS_COMMIT_TICK_QUIET)
    {
        *quiet = 0;
        return 1;
    }
    return 0;
}

#endif
