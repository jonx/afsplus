/* SPDX-License-Identifier: BSD-2-Clause */

/* Target-side probe of the DOS semantics beyond the Alpha-0 slice, through a
 * real dos.library: metadata setters, the comment, soft links, ExAll,
 * OpenFromLock, ChangeMode, record locks, notification and Relabel. It works
 * in one drawer and removes it, so the volume's namespace is as it found it.
 *
 * With the argument HOLD it instead ends a notification request while one of
 * its messages is still unreplied, never replies, and exits: the state in
 * which a following dismount has to succeed. A request that is still
 * registered keeps the handler alive by design, so it is ended first.
 *
 * STEADY <rounds> repeats one fixed set of operations and reports the free
 * memory after each round, so a handler that keeps something per operation
 * shows as a falling line instead of a flat one.
 *
 * RECORD-HOLDER and RECORD-WAITER run as two tasks: the holder keeps a record
 * for three seconds, the waiter asks for it with a ten-second timeout and must
 * get it from the release, neither at once nor by expiry. */

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <dos/exall.h>
#include <dos/notify.h>
#include <dos/record.h>
#include <exec/memory.h>
#include <exec/ports.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <string.h>

#include "../client/afsplus_client.h"

#ifndef AFSPLUS_PROBE_VOLUME
#define AFSPLUS_PROBE_VOLUME "AFSPLUS19"
#endif

#define DRAWER AFSPLUS_PROBE_VOLUME ":dosprobe"
#define NOTE DRAWER "/note"
#define ALIAS DRAWER "/alias"
#define PAGES DRAWER "/pages"
#define HELD AFSPLUS_PROBE_VOLUME ":dosprobe.held"
#define RECORDS AFSPLUS_PROBE_VOLUME ":dosprobe.records"
#define TEMPORARY_LABEL "AFSPlusRelabelled"

static const UBYTE content[] = "0123456789abcdef";

static int fail(const char *stage, SIPTR result)
{
    Printf("[AFSPLUS-DOS] FAIL %s result %ld error %ld\n",
        stage, (LONG)result, IoErr());
    return RETURN_FAIL;
}

static int write_file(CONST_STRPTR path, LONG mode)
{
    BPTR file = Open(path, mode);
    LONG written;

    if (file == BNULL)
        return 0;
    written = Write(file, content, sizeof(content) - 1);
    return Close(file) && written == (LONG)(sizeof(content) - 1);
}

static int examine_path(CONST_STRPTR path, struct FileInfoBlock *fib)
{
    BPTR lock = Lock(path, SHARED_LOCK);
    LONG ok;

    if (lock == BNULL)
        return 0;
    ok = Examine(lock, fib);
    UnLock(lock);
    return ok != DOSFALSE;
}

/* Waits up to two seconds for one message on port. */
static struct Message *await_message(struct MsgPort *port)
{
    struct Message *message;
    int tries;

    for (tries = 0; tries < 20; tries++)
    {
        message = GetMsg(port);
        if (message != NULL)
            return message;
        Delay(5);
    }
    return NULL;
}

static int probe_metadata(struct FileInfoBlock *fib)
{
    struct DateStamp stamp;
    UBYTE long_comment[81];

    if (!SetProtection(NOTE, FIBF_SCRIPT | FIBF_ARCHIVE))
        return fail("SetProtection", DOSFALSE);
    stamp.ds_Days = 5000;
    stamp.ds_Minute = 61;
    stamp.ds_Tick = 100;
    if (!SetFileDate(NOTE, &stamp))
        return fail("SetFileDate", DOSFALSE);
    if (!SetComment(NOTE, "draft two"))
        return fail("SetComment", DOSFALSE);
    if (!examine_path(NOTE, fib))
        return fail("Examine after setters", DOSFALSE);
    if (fib->fib_Protection != (LONG)(FIBF_SCRIPT | FIBF_ARCHIVE))
        return fail("protection readback", fib->fib_Protection);
    if (fib->fib_Date.ds_Days != 5000 || fib->fib_Date.ds_Minute != 61
        || fib->fib_Date.ds_Tick != 100)
        return fail("date readback", fib->fib_Date.ds_Tick);
    if (strcmp((const char *)fib->fib_Comment, "draft two") != 0)
        return fail("comment readback", fib->fib_Comment[0]);

    /* Eighty characters do not fit fib_Comment: refused, comment kept. */
    memset(long_comment, 'k', 80);
    long_comment[80] = 0;
    if (SetComment(NOTE, long_comment))
        return fail("oversized comment accepted", DOSTRUE);
    if (IoErr() != ERROR_COMMENT_TOO_BIG)
        return fail("oversized comment error", DOSFALSE);
    long_comment[79] = 0;
    if (!SetComment(NOTE, long_comment))
        return fail("79-character comment", DOSFALSE);
    if (!examine_path(NOTE, fib)
        || strcmp((const char *)fib->fib_Comment,
            (const char *)long_comment) != 0)
        return fail("79-character readback", DOSFALSE);
    if (!SetComment(NOTE, "kept"))
        return fail("SetComment kept", DOSFALSE);
    return RETURN_OK;
}

static int probe_soft_link(void)
{
    UBYTE target[64];
    UBYTE readback[sizeof(content)];
    struct DevProc *process;
    BPTR file;
    LONG length;

    if (!MakeLink(ALIAS, (SIPTR)"note", LINK_SOFT))
        return fail("MakeLink soft", DOSFALSE);
    /* dos.library resolves the link after ERROR_IS_SOFT_LINK. */
    file = Open(ALIAS, MODE_OLDFILE);
    if (file == BNULL)
        return fail("Open through soft link", DOSFALSE);
    memset(readback, 0, sizeof(readback));
    length = Read(file, readback, sizeof(readback));
    Close(file);
    if (length != (LONG)(sizeof(content) - 1)
        || memcmp(readback, content, sizeof(content) - 1) != 0)
        return fail("read through soft link", length);

    process = GetDeviceProc(ALIAS, NULL);
    if (process == NULL)
        return fail("GetDeviceProc", DOSFALSE);
    memset(target, 0, sizeof(target));
    length = ReadLink(process->dvp_Port, process->dvp_Lock,
        "dosprobe/alias", target, sizeof(target));
    FreeDeviceProc(process);
    if (length < 0)
        return fail("ReadLink", length);
    if (strstr((const char *)target, "note") == NULL)
        return fail("ReadLink target", length);
    return RETURN_OK;
}

static int probe_exall(void)
{
    /* Small on purpose: one ED_COMMENT record with its strings fits, two do
     * not, so the second entry needs a continuation. */
    union { UBYTE bytes[104]; struct ExAllData align; } buffer;
    struct ExAllControl *control;
    struct ExAllData *entry;
    BPTR lock;
    LONG more;
    ULONG seen_note = 0;
    ULONG seen_alias = 0;
    ULONG total = 0;
    ULONG calls = 0;

    lock = Lock(DRAWER, SHARED_LOCK);
    if (lock == BNULL)
        return fail("Lock drawer", DOSFALSE);
    control = AllocDosObject(DOS_EXALLCONTROL, NULL);
    if (control == NULL)
    {
        UnLock(lock);
        return fail("AllocDosObject", DOSFALSE);
    }
    control->eac_LastKey = 0;
    do
    {
        more = ExAll(lock, &buffer.align, sizeof(buffer), ED_COMMENT,
            control);
        calls++;
        if (!more && IoErr() != ERROR_NO_MORE_ENTRIES)
        {
            FreeDosObject(DOS_EXALLCONTROL, control);
            UnLock(lock);
            return fail("ExAll", (SIPTR)calls);
        }
        entry = control->eac_Entries != 0 ? &buffer.align : NULL;
        for (; entry != NULL; entry = entry->ed_Next)
        {
            total++;
            if (strcmp((const char *)entry->ed_Name, "note") == 0
                && strcmp((const char *)entry->ed_Comment, "kept") == 0
                && entry->ed_Type < 0
                && entry->ed_Size == sizeof(content) - 1
                && entry->ed_Prot == (FIBF_SCRIPT | FIBF_ARCHIVE))
                seen_note++;
            if (strcmp((const char *)entry->ed_Name, "alias") == 0
                && entry->ed_Type == ST_SOFTLINK
                && entry->ed_Comment[0] == 0)
                seen_alias++;
        }
    }
    while (more);
    FreeDosObject(DOS_EXALLCONTROL, control);
    UnLock(lock);
    if (total != 2 || seen_note != 1 || seen_alias != 1)
        return fail("ExAll entries", (SIPTR)total);
    Printf("[AFSPLUS-DOS] ExAll calls %lu\n", calls);
    if (calls < 2)
        return fail("ExAll continuation not exercised", (SIPTR)calls);
    return RETURN_OK;
}

/*
 * ExAll across three pages, with the directory changing between them.
 *
 * The contract the AFS+ resume states: entries ordered after the last one
 * returned come back exactly once, and an entry created before that position
 * during the enumeration does not come back at all. One page cannot show
 * that; three, with a create and a delete in the middle, can.
 */
/* PAGES "/pN" for one digit. */
static const char *page_entry(char *name, ULONG digit)
{
    static const char prefix[] = PAGES "/p";

    memcpy(name, prefix, sizeof(prefix) - 1);
    name[sizeof(prefix) - 1] = (char)('0' + digit);
    name[sizeof(prefix)] = 0;
    return name;
}

static int probe_exall_pages(void)
{
    /* ED_NAME records with two-character names: three fit, a fourth does
     * not, so nine entries need at least three calls. */
    union { UBYTE bytes[80]; struct ExAllData align; } buffer;
    struct ExAllControl *control;
    struct ExAllData *entry;
    BPTR drawer;
    LONG more;
    ULONG calls = 0;
    ULONG seen[10];
    ULONG seen_before = 0;
    ULONG seen_after = 0;
    ULONG duplicates = 0;
    ULONG unexpected = 0;
    ULONG index;
    int status = RETURN_OK;

    drawer = CreateDir(PAGES);
    if (drawer == BNULL)
        return fail("CreateDir pages", DOSFALSE);
    UnLock(drawer);
    for (index = 1; index <= 9; index++)
    {
        char name[sizeof(PAGES) + 3];

        seen[index] = 0;
        if (!write_file(page_entry(name, index), MODE_NEWFILE))
            return fail("create page entry", (SIPTR)index);
    }
    drawer = Lock(PAGES, SHARED_LOCK);
    if (drawer == BNULL)
        return fail("Lock pages", DOSFALSE);
    control = AllocDosObject(DOS_EXALLCONTROL, NULL);
    if (control == NULL)
    {
        UnLock(drawer);
        return fail("AllocDosObject pages", DOSFALSE);
    }
    control->eac_LastKey = 0;

    do
    {
        more = ExAll(drawer, &buffer.align, sizeof(buffer), ED_NAME, control);
        calls++;
        if (!more && IoErr() != ERROR_NO_MORE_ENTRIES)
        {
            status = fail("ExAll pages", (SIPTR)calls);
            break;
        }
        entry = control->eac_Entries != 0 ? &buffer.align : NULL;
        for (; entry != NULL; entry = entry->ed_Next)
        {
            const char *name = (const char *)entry->ed_Name;

            if (name[0] == 'p' && name[1] >= '1' && name[1] <= '9'
                && name[2] == 0)
            {
                index = (ULONG)(name[1] - '0');
                if (seen[index]++)
                    duplicates++;
            }
            else if (strcmp(name, "p0") == 0)
                seen_before++;
            else if (strcmp(name, "pz") == 0)
                seen_after++;
            else
                unexpected++;
        }

        /* After the first page only: p9 has certainly not been returned yet,
         * p0 sorts before everything returned so far, and pz after
         * everything. */
        if (calls == 1 && status == RETURN_OK)
        {
            if (control->eac_Entries == 0 || !more)
            {
                status = fail("first page held everything",
                    (SIPTR)control->eac_Entries);
                break;
            }
            if (!DeleteFile(PAGES "/p9"))
            {
                status = fail("delete during enumeration", DOSFALSE);
                break;
            }
            if (!write_file(PAGES "/p0", MODE_NEWFILE)
                || !write_file(PAGES "/pz", MODE_NEWFILE))
            {
                status = fail("create during enumeration", DOSFALSE);
                break;
            }
        }
    }
    while (more);
    FreeDosObject(DOS_EXALLCONTROL, control);
    UnLock(drawer);

    if (status == RETURN_OK)
    {
        for (index = 1; index <= 8 && status == RETURN_OK; index++)
            if (seen[index] != 1)
                status = fail("entry not returned exactly once",
                    (SIPTR)index);
    }
    if (status == RETURN_OK && duplicates != 0)
        status = fail("an entry came back twice", (SIPTR)duplicates);
    if (status == RETURN_OK && seen[9] != 0)
        status = fail("a deleted entry was returned", (SIPTR)seen[9]);
    if (status == RETURN_OK && seen_before != 0)
        status = fail("an entry created behind the cursor was returned",
            (SIPTR)seen_before);
    if (status == RETURN_OK && seen_after != 1)
        status = fail("the entry created ahead of the cursor was not"
            " returned once", (SIPTR)seen_after);
    if (status == RETURN_OK && unexpected != 0)
        status = fail("an entry nobody created was returned",
            (SIPTR)unexpected);
    if (status == RETURN_OK && calls < 3)
        status = fail("the walk did not take three pages", (SIPTR)calls);
    if (status == RETURN_OK)
        Printf("[AFSPLUS-DOS] ExAll pages %lu entries 10\n", calls);

    /* The drawer goes, whatever happened, so a rerun starts clean. */
    for (index = 0; index <= 9; index++)
    {
        char name[sizeof(PAGES) + 3];

        DeleteFile(page_entry(name, index));
    }
    DeleteFile(PAGES "/pz");
    if (!DeleteFile(PAGES) && status == RETURN_OK)
        status = fail("remove pages drawer", DOSFALSE);
    return status;
}

/*
 * The paged walk by object ID, which only the extension packet reaches. It
 * runs over a nine-entry drawer and holds the same rule as the ExAll
 * continuation: the position is the last name returned, so an entry created
 * ahead of it comes back and the walk never loses its place.
 */
static int probe_dir_walk(void)
{
    UBYTE records[AFSPLUS_AROS_DIR_RECORD_MAX * 2];
    BPTR drawer;
    uint64_t walk = 0;
    ULONG index;
    ULONG seen[10];
    ULONG seen_after = 0;
    ULONG duplicates = 0;
    ULONG calls = 0;
    uint32_t eof = 0;
    LONG error;
    int status = RETURN_OK;

    drawer = CreateDir(PAGES);
    if (drawer == BNULL)
        return fail("CreateDir walk", DOSFALSE);
    UnLock(drawer);
    for (index = 1; index <= 9; index++)
    {
        char name[sizeof(PAGES) + 3];

        seen[index] = 0;
        if (!write_file(page_entry(name, index), MODE_NEWFILE))
            return fail("create walk entry", (SIPTR)index);
    }
    drawer = Lock(PAGES, SHARED_LOCK);
    if (drawer == BNULL)
        return fail("Lock walk", DOSFALSE);

    error = afsplus_client_dir_open(drawer, &walk);
    if (error != 0)
    {
        UnLock(drawer);
        return fail("dir_open", error);
    }
    while (status == RETURN_OK && !eof)
    {
        uint32_t count = 0;
        uint32_t at = 0;

        error = afsplus_client_dir_read(drawer, walk, records,
            sizeof(records), 64, &count, &eof);
        calls++;
        if (error != 0)
        {
            status = fail("dir_read", error);
            break;
        }
        if (count == 0 && !eof)
        {
            status = fail("a page with nothing in it", (SIPTR)calls);
            break;
        }
        for (index = 0; index < count; index++)
        {
            struct AfsplusArosDirEntry record;
            const char *name;

            memcpy(&record, records + at, sizeof(record));
            name = (const char *)(records + at + sizeof(record));
            at += record.record_length;
            if (record.name_length == 2 && name[0] == 'p'
                && name[1] >= '1' && name[1] <= '9')
            {
                ULONG which = (ULONG)(name[1] - '0');

                if (seen[which]++)
                    duplicates++;
            }
            else if (record.name_length == 2 && name[0] == 'p'
                && name[1] == 'z')
                seen_after++;
            else
                status = fail("an entry nobody created", (SIPTR)at);
        }
        /* After the first page, one entry ahead of the position: it must
         * come back, and the walk must not lose its place. */
        if (calls == 1 && status == RETURN_OK && !eof)
        {
            if (!write_file(PAGES "/pz", MODE_NEWFILE))
                status = fail("create during the walk", DOSFALSE);
        }
    }
    error = afsplus_client_dir_close(drawer, walk);
    if (status == RETURN_OK && error != 0)
        status = fail("dir_close", error);
    /* A closed walk is gone: reading it again is an error, not a second
     * view of the directory. */
    if (status == RETURN_OK)
    {
        uint32_t count = 0;
        uint32_t ignored = 0;

        if (afsplus_client_dir_read(drawer, walk, records, sizeof(records),
                64, &count, &ignored) == 0)
            status = fail("a closed walk still read", (SIPTR)count);
    }
    UnLock(drawer);

    for (index = 1; index <= 9 && status == RETURN_OK; index++)
        if (seen[index] != 1)
            status = fail("walk entry not returned exactly once",
                (SIPTR)index);
    if (status == RETURN_OK && duplicates != 0)
        status = fail("the walk returned an entry twice", (SIPTR)duplicates);
    if (status == RETURN_OK && seen_after != 1)
        status = fail("the entry created ahead of the walk was not returned"
            " once", (SIPTR)seen_after);
    if (status == RETURN_OK && calls < 2)
        status = fail("the walk took one page", (SIPTR)calls);
    if (status == RETURN_OK)
        Printf("[AFSPLUS-DOS] dir walk pages %lu entries 10\n", calls);

    for (index = 1; index <= 9; index++)
    {
        char name[sizeof(PAGES) + 3];

        DeleteFile(page_entry(name, index));
    }
    DeleteFile(PAGES "/pz");
    if (!DeleteFile(PAGES) && status == RETURN_OK)
        status = fail("remove walk drawer", DOSFALSE);
    return status;
}

/* The v2 watch: a name that does not exist yet under the drawer, taken
 * before, after and again after its creation, and gone once removed. */
static int probe_watch(void)
{
    BPTR drawer;
    struct MsgPort *port;
    uint64_t watch = 0;
    uint32_t changed = 7;
    LONG error;
    int status = RETURN_OK;

    drawer = Lock(DRAWER, SHARED_LOCK);
    if (drawer == BNULL)
        return fail("Lock watch drawer", DOSFALSE);
    port = afsplus_client_lock_port(drawer);
    error = afsplus_client_watch_add(drawer, (CONST_STRPTR)"watched", &watch);
    if (error != 0)
    {
        UnLock(drawer);
        return fail("watch_add", error);
    }
    error = afsplus_client_watch_take(port, watch, &changed);
    if (error != 0)
        status = fail("watch_take", error);
    else if (changed != 0)
        status = fail("a watch changed before anything did", (SIPTR)changed);
    if (status == RETURN_OK && !write_file(DRAWER "/watched", MODE_NEWFILE))
        status = fail("create watched", DOSFALSE);
    if (status == RETURN_OK
        && ((error = afsplus_client_watch_take(port, watch, &changed)) != 0
            || changed != 1))
        status = fail("the created name was not seen", error ? error
            : (SIPTR)changed);
    if (status == RETURN_OK
        && ((error = afsplus_client_watch_take(port, watch, &changed)) != 0
            || changed != 0))
        status = fail("a take did not clear", error ? error : (SIPTR)changed);
    error = afsplus_client_watch_remove(port, watch);
    if (status == RETURN_OK && error != 0)
        status = fail("watch_remove", error);
    if (status == RETURN_OK
        && afsplus_client_watch_take(port, watch, &changed)
            != ERROR_OBJECT_NOT_FOUND)
        status = fail("a removed watch still answered", DOSTRUE);
    UnLock(drawer);
    DeleteFile(DRAWER "/watched");
    if (status == RETURN_OK)
        Printf("[AFSPLUS-DOS] v2 watch taken 1 then 0, removed\n");
    return status;
}

static int probe_handles(void)
{
    UBYTE readback[4];
    BPTR lock;
    BPTR file;
    BPTR second;

    lock = Lock(NOTE, SHARED_LOCK);
    if (lock == BNULL)
        return fail("Lock note", DOSFALSE);
    file = OpenFromLock(lock);
    if (file == BNULL)
    {
        UnLock(lock);
        return fail("OpenFromLock", DOSFALSE);
    }
    if (Read(file, readback, 4) != 4 || memcmp(readback, content, 4) != 0)
    {
        Close(file);
        return fail("read after OpenFromLock", DOSFALSE);
    }

    /* Exclusive through ChangeMode: a second open is refused; shared again:
     * it succeeds. */
    if (!ChangeMode(CHANGE_FH, file, EXCLUSIVE_LOCK))
    {
        Close(file);
        return fail("ChangeMode exclusive", DOSFALSE);
    }
    second = Open(NOTE, MODE_OLDFILE);
    if (second != BNULL)
    {
        Close(second);
        Close(file);
        return fail("open beside exclusive handle", DOSTRUE);
    }
    if (IoErr() != ERROR_OBJECT_IN_USE)
    {
        Close(file);
        return fail("exclusive handle error", DOSFALSE);
    }
    if (!ChangeMode(CHANGE_FH, file, SHARED_LOCK))
    {
        Close(file);
        return fail("ChangeMode shared", DOSFALSE);
    }
    second = Open(NOTE, MODE_OLDFILE);
    if (second == BNULL)
    {
        Close(file);
        return fail("open beside shared handle", DOSFALSE);
    }

    /* Record locks: another handle collides at once, a waiting mode times
     * out, and the owner's own overlapping request is reported as found. */
    if (!LockRecord(file, 0, 10, REC_EXCLUSIVE_IMMED, 0))
    {
        Close(second);
        Close(file);
        return fail("LockRecord", DOSFALSE);
    }
    if (LockRecord(second, 5, 10, REC_EXCLUSIVE_IMMED, 0)
        || IoErr() != ERROR_LOCK_COLLISION)
    {
        Close(second);
        Close(file);
        return fail("record collision", DOSFALSE);
    }
    if (LockRecord(second, 5, 10, REC_EXCLUSIVE, 5)
        || IoErr() != ERROR_LOCK_TIMEOUT)
    {
        Close(second);
        Close(file);
        return fail("record timeout", DOSFALSE);
    }
    if (LockRecord(second, 10, 10, REC_EXCLUSIVE_IMMED, 0) == DOSFALSE)
    {
        Close(second);
        Close(file);
        return fail("adjacent record", DOSFALSE);
    }
    {
        LONG own = LockRecord(file, 5, 2, REC_EXCLUSIVE_IMMED, 0);

        Printf("[AFSPLUS-DOS] self-overlap result %ld error %ld\n",
            own, own ? 0 : IoErr());
        if (own)
            UnLockRecord(file, 5, 2);
    }
    if (UnLockRecord(second, 0, 10) || IoErr() != ERROR_RECORD_NOT_LOCKED)
    {
        Close(second);
        Close(file);
        return fail("foreign record unlock", DOSFALSE);
    }
    if (!UnLockRecord(file, 0, 10) || !UnLockRecord(second, 10, 10))
    {
        Close(second);
        Close(file);
        return fail("UnLockRecord", DOSFALSE);
    }
    if (!Close(second) || !Close(file))
        return fail("close handles", DOSFALSE);

    /* A locked object is not deletable. */
    lock = Lock(NOTE, SHARED_LOCK);
    if (lock == BNULL)
        return fail("Lock before delete", DOSFALSE);
    if (DeleteFile(NOTE) || IoErr() != ERROR_OBJECT_IN_USE)
    {
        UnLock(lock);
        return fail("delete of a locked object", DOSFALSE);
    }
    UnLock(lock);
    return RETURN_OK;
}

/* Extended attributes through the extension packet: the classic namespaces
 * are written, the preserved ones refused, sizes are probed, and one
 * attribute stays on the volume root for the host to find in the image. */
static int probe_attributes(void)
{
    UBYTE value[16];
    char names[64];
    uint32_t required = 0;
    BPTR lock = Lock(NOTE, SHARED_LOCK);
    LONG error;

    if (lock == BNULL)
        return fail("Lock for attributes", DOSFALSE);
    error = afsplus_client_set_attribute(lock, "user.kind", "text", 4,
        AFSPLUS_AROS_ATTRIBUTE_CREATE);
    if (error == 0)
        error = afsplus_client_set_attribute(lock, "user.kind", "x", 1,
            AFSPLUS_AROS_ATTRIBUTE_CREATE) == ERROR_OBJECT_EXISTS ? 0 : 1;
    if (error == 0)
        error = afsplus_client_get_attribute(lock, "user.kind", NULL, 0,
            &required);
    if (error == 0 && required != 4)
        error = 2;
    memset(value, 0, sizeof(value));
    if (error == 0)
        error = afsplus_client_get_attribute(lock, "user.kind", value,
            sizeof(value), &required);
    if (error == 0 && memcmp(value, "text", 5) != 0)
        error = 3;
    memset(names, 0x7e, sizeof(names));
    if (error == 0)
        error = afsplus_client_list_attributes(lock, names, sizeof(names),
            &required);
    if (error == 0 && (required != 10 || memcmp(names, "user.kind", 10) != 0))
        error = 4;
    if (error == 0
        && afsplus_client_set_attribute(lock, "security.probe", "x", 1,
            AFSPLUS_AROS_ATTRIBUTE_UPSERT) != ERROR_WRITE_PROTECTED)
        error = 5;
    if (error == 0)
        error = afsplus_client_set_attribute(lock, "user.kind", NULL, 0,
            AFSPLUS_AROS_ATTRIBUTE_REMOVE);
    if (error == 0
        && afsplus_client_get_attribute(lock, "user.kind", NULL, 0,
            &required) != ERROR_OBJECT_NOT_FOUND)
        error = 6;
    UnLock(lock);
    if (error != 0)
        return fail("attributes", error);

    lock = Lock(AFSPLUS_PROBE_VOLUME ":", SHARED_LOCK);
    if (lock == BNULL)
        return fail("Lock root for attributes", DOSFALSE);
    error = afsplus_client_set_attribute(lock, "aros.probe", "kept", 4,
        AFSPLUS_AROS_ATTRIBUTE_UPSERT);
    UnLock(lock);
    if (error != 0)
        return fail("root attribute", error);
    return RETURN_OK;
}

static int start_notify(struct NotifyRequest *request, struct MsgPort *port,
    STRPTR path)
{
    memset(request, 0, sizeof(*request));
    request->nr_Name = path;
    request->nr_Flags = NRF_SEND_MESSAGE | NRF_WAIT_REPLY;
    request->nr_stuff.nr_Msg.nr_Port = port;
    return StartNotify(request) != DOSFALSE;
}

static int probe_notify(void)
{
    struct NotifyRequest request;
    struct MsgPort *port = CreateMsgPort();
    struct Message *first;
    struct Message *second;
    int status = RETURN_OK;

    if (port == NULL)
        return fail("CreateMsgPort", DOSFALSE);
    if (!start_notify(&request, port, NOTE))
    {
        DeleteMsgPort(port);
        return fail("StartNotify", DOSFALSE);
    }
    if (!write_file(NOTE, MODE_READWRITE))
        status = fail("first notified write", DOSFALSE);
    first = status == RETURN_OK ? await_message(port) : NULL;
    if (status == RETURN_OK && first == NULL)
        status = fail("first notification", DOSFALSE);

    /* While the first message is unreplied a second change sends nothing;
     * the change is owed and arrives after the reply. */
    if (status == RETURN_OK && !write_file(NOTE, MODE_READWRITE))
        status = fail("second notified write", DOSFALSE);
    if (status == RETURN_OK)
    {
        Delay(25);
        second = GetMsg(port);
        if (second != NULL)
        {
            ReplyMsg(second);
            status = fail("message while one was unreplied", DOSTRUE);
        }
    }
    if (first != NULL)
        ReplyMsg(first);
    if (status == RETURN_OK)
    {
        second = await_message(port);
        if (second == NULL)
            status = fail("owed notification after reply", DOSFALSE);
        else
            ReplyMsg(second);
    }
    EndNotify(&request);
    while ((first = GetMsg(port)) != NULL)
        ReplyMsg(first);
    DeleteMsgPort(port);
    return status;
}

static int probe_relabel(void)
{
    UBYTE original[108];
    UBYTE name[160];
    BPTR lock;
    char *colon;

    lock = Lock(DRAWER, SHARED_LOCK);
    if (lock == BNULL || !NameFromLock(lock, name, sizeof(name)))
    {
        if (lock != BNULL)
            UnLock(lock);
        return fail("NameFromLock", DOSFALSE);
    }
    colon = strchr((char *)name, ':');
    if (colon == NULL || (size_t)(colon - (char *)name) >= sizeof(original))
    {
        UnLock(lock);
        return fail("volume name", DOSFALSE);
    }
    memset(original, 0, sizeof(original));
    memcpy(original, name, (size_t)(colon - (char *)name));

    /* The held lock keeps pointing at the renamed node. */
    if (!Relabel(AFSPLUS_PROBE_VOLUME ":", TEMPORARY_LABEL))
    {
        UnLock(lock);
        return fail("Relabel", DOSFALSE);
    }
    if (!NameFromLock(lock, name, sizeof(name))
        || strcmp((const char *)name, TEMPORARY_LABEL ":dosprobe") != 0)
    {
        Relabel(AFSPLUS_PROBE_VOLUME ":", original);
        UnLock(lock);
        return fail("name after Relabel", DOSFALSE);
    }
    UnLock(lock);
    lock = Lock(TEMPORARY_LABEL ":dosprobe/note", SHARED_LOCK);
    if (lock == BNULL)
    {
        Relabel(AFSPLUS_PROBE_VOLUME ":", original);
        return fail("Lock by new volume name", DOSFALSE);
    }
    UnLock(lock);
    if (!Relabel(AFSPLUS_PROBE_VOLUME ":", original))
        return fail("Relabel back", DOSFALSE);
    return RETURN_OK;
}

/* Ends a WAIT_REPLY request whose one message is never replied. The port
 * stays allocated: the message on it belongs to the handler. */
static int hold_notification(void)
{
    struct NotifyRequest *request = AllocMem(sizeof(*request),
        MEMF_PUBLIC | MEMF_CLEAR);
    struct MsgPort *port = CreateMsgPort();

    if (request == NULL || port == NULL)
        return fail("HOLD allocation", DOSFALSE);
    if (!write_file(HELD, MODE_NEWFILE))
        return fail("HOLD create", DOSFALSE);
    if (!start_notify(request, port, HELD))
        return fail("HOLD StartNotify", DOSFALSE);
    if (!write_file(HELD, MODE_READWRITE))
        return fail("HOLD write", DOSFALSE);
    if (await_message(port) == NULL)
        return fail("HOLD notification", DOSFALSE);
    EndNotify(request);
    if (!DeleteFile(HELD))
        return fail("HOLD remove", DOSFALSE);
    Printf("[AFSPLUS-DOS] HOLD one notification left unreplied\n");
    return RETURN_OK;
}

/* Ticks since midnight, to order what two tasks print. */
static LONG now_ticks(void)
{
    struct DateStamp stamp;

    DateStamp(&stamp);
    return stamp.ds_Minute * 3000 + stamp.ds_Tick;
}

static int hold_record(void)
{
    BPTR file = Open(RECORDS, MODE_READWRITE);
    int tries;

    if (file == BNULL)
        return fail("HOLDER open", DOSFALSE);

    /* The waiter polls the same record and holds it for an instant at a
     * time, so one attempt can lose to it. */
    for (tries = 0; tries < 200; tries++)
    {
        if (LockRecord(file, 0, 10, REC_EXCLUSIVE_IMMED, 0))
            break;
        Delay(1);
    }
    if (tries == 200)
    {
        Close(file);
        return fail("HOLDER LockRecord", DOSFALSE);
    }
    Printf("[AFSPLUS-DOS] HOLDER holds the record at tick %ld\n",
        now_ticks());
    Delay(150);
    if (!UnLockRecord(file, 0, 10))
    {
        Close(file);
        return fail("HOLDER UnLockRecord", DOSFALSE);
    }
    Close(file);
    Printf("[AFSPLUS-DOS] HOLDER released the record at tick %ld\n",
        now_ticks());
    return RETURN_OK;
}

static LONG ticks_between(const struct DateStamp *from,
    const struct DateStamp *to)
{
    return (to->ds_Days - from->ds_Days) * 24 * 60 * 3000
        + (to->ds_Minute - from->ds_Minute) * 3000
        + (to->ds_Tick - from->ds_Tick);
}

static int wait_for_record(void)
{
    struct DateStamp before;
    struct DateStamp after;
    BPTR file = Open(RECORDS, MODE_READWRITE);
    LONG waited;
    int tries;

    if (file == BNULL)
        return fail("WAITER open", DOSFALSE);
    Printf("[AFSPLUS-DOS] WAITER starts polling at tick %ld\n", now_ticks());
    /* Until the holder task has the record, an immediate request succeeds. */
    for (tries = 0; tries < 400; tries++)
    {
        if (!LockRecord(file, 0, 10, REC_EXCLUSIVE_IMMED, 0))
            break;
        UnLockRecord(file, 0, 10);
        Delay(2);
    }
    Printf("[AFSPLUS-DOS] WAITER stops polling at tick %ld after %ld tries\n",
        now_ticks(), (LONG)tries);
    if (tries == 400 || IoErr() != ERROR_LOCK_COLLISION)
    {
        Close(file);
        return fail("WAITER never saw the holder", (SIPTR)tries);
    }
    DateStamp(&before);
    if (!LockRecord(file, 0, 10, REC_EXCLUSIVE, 500))
    {
        Close(file);
        return fail("WAITER LockRecord", DOSFALSE);
    }
    DateStamp(&after);
    waited = ticks_between(&before, &after);
    UnLockRecord(file, 0, 10);
    Close(file);
    DeleteFile(RECORDS);
    /* Granted by the release: it took a while, and far less than the
     * timeout. */
    if (waited < 10 || waited >= 400)
        return fail("WAITER wait length", waited);
    Printf("[AFSPLUS-DOS] RECORDS granted after %ld ticks\n", waited);
    return RETURN_OK;
}

/*
 * One round of the operations a handler is asked for all day, each one
 * paired with what releases it. Nothing here is meant to fail.
 */
/* The negative control of STEADY: "STEADY <rounds> LEAK" keeps one lock on
 * the drawer per round and never gives it back, so the handler's heap grows
 * at every reading and the gate has to say so. */
static int steady_leak;

static int steady_round(void)
{
    struct FileInfoBlock *fib;
    struct NotifyRequest request;
    struct MsgPort *port;
    BPTR lock;
    BPTR file;
    UBYTE buffer[sizeof(content)];

    if (steady_leak && Lock(DRAWER, SHARED_LOCK) == BNULL)
        return fail("STEADY leak lock", DOSFALSE);
    if (!write_file(NOTE, MODE_NEWFILE))
        return fail("STEADY create", DOSFALSE);
    file = Open(NOTE, MODE_OLDFILE);
    if (file == BNULL)
        return fail("STEADY open", DOSFALSE);
    if (Read(file, buffer, sizeof(buffer)) < 0)
    {
        Close(file);
        return fail("STEADY read", DOSFALSE);
    }
    if (!LockRecord(file, 0, 4, REC_EXCLUSIVE_IMMED, 0)
        || !UnLockRecord(file, 0, 4))
    {
        Close(file);
        return fail("STEADY record", DOSFALSE);
    }
    if (!Close(file))
        return fail("STEADY close", DOSFALSE);

    lock = Lock(NOTE, SHARED_LOCK);
    if (lock == BNULL)
        return fail("STEADY lock", DOSFALSE);
    fib = AllocDosObject(DOS_FIB, NULL);
    if (fib == NULL)
    {
        UnLock(lock);
        return fail("STEADY AllocDosObject", DOSFALSE);
    }
    if (!Examine(lock, fib))
    {
        FreeDosObject(DOS_FIB, fib);
        UnLock(lock);
        return fail("STEADY examine", DOSFALSE);
    }
    FreeDosObject(DOS_FIB, fib);
    UnLock(lock);

    /* A watch taken and given back: the table must not grow. */
    port = CreateMsgPort();
    if (port == NULL)
        return fail("STEADY CreateMsgPort", DOSFALSE);
    if (!start_notify(&request, port, NOTE))
    {
        DeleteMsgPort(port);
        return fail("STEADY StartNotify", DOSFALSE);
    }
    EndNotify(&request);
    DeleteMsgPort(port);

    if (!SetComment(NOTE, "round") || !SetProtection(NOTE, FIBF_ARCHIVE))
        return fail("STEADY metadata", DOSFALSE);
    if (!DeleteFile(NOTE))
        return fail("STEADY delete", DOSFALSE);
    return RETURN_OK;
}

/* System free memory moves by a few KiB between two readings with no leak
 * behind it: exec's pools keep or return puddles as the handler's transient
 * allocations change size, and a cleanup that takes 32 orphans in one
 * transaction makes larger ones than 32 cleanups did. Measured with that
 * change: 1,248 and 1,520 bytes lost over 100 rounds, 3,520 gained over
 * 400, the library's own heap flat each time. The heap clause below, at 256
 * bytes, is what finds a leak; this one only catches a gross one. */
#define STEADY_ALLOWANCE 4096
/* The commit path grows by a few dozen bytes over hundreds of rounds, an
 * open finding; three bytes a round over a hundred rounds is past this. */
#define STEADY_HEAP_ALLOWANCE 256

/* What the volume still owes after the flush: deleted files whose blocks
 * are not back yet. Their number is what the heap readings differ by when
 * they differ, so it is printed beside them. */
static ULONG steady_orphans(struct MsgPort *port)
{
    struct AfsplusArosHealth health;

    memset(&health, 0, sizeof(health));
    if (afsplus_client_health(port, &health) != 0)
        return 0;
    return (ULONG)health.pending_orphans;
}

/* The handler library's heap counters, through the transport. */
static int steady_heap(struct MsgPort *port,
    struct AfsplusArosCounters *counters)
{
    LONG error;
    ULONG flushes;

    /* At a durable point, and with nothing left to do: a delayed mount
     * holds a varying amount of uncommitted state between rounds, and one
     * flush cleans a bounded number of the files the rounds deleted; those
     * still pending live in the orphan directory the cache holds decoded,
     * about 24 bytes each, so two readings with different backlogs differ
     * by that and not by a leak. Flushing until none is pending compares
     * like with like. */
    for (flushes = 0; flushes < 64; flushes++)
    {
        if (!DoPkt(port, ACTION_FLUSH, 0, 0, 0, 0, 0))
            return fail("STEADY flush", DOSFALSE);
        if (steady_orphans(port) == 0)
            break;
    }
    memset(counters, 0, sizeof(*counters));
    error = afsplus_client_counters(port, counters);
    if (error != 0)
        return fail("STEADY counters", error);
    if (counters->struct_size < sizeof(*counters))
        return fail("STEADY counters without the heap", counters->struct_size);
    return RETURN_OK;
}

static int probe_steady(const char *rounds_text)
{
    ULONG rounds = 0;
    ULONG round;
    ULONG before;
    ULONG after = 0;
    ULONG orphans_warm = 0;
    ULONG orphans_done = 0;
    ULONG batch;
    uint64_t held[4] = { 0, 0, 0, 0 };
    struct AfsplusArosCounters warm;
    struct AfsplusArosCounters done;
    struct MsgPort *port;
    BPTR drawer;
    int status;

    while (*rounds_text >= '0' && *rounds_text <= '9')
        rounds = rounds * 10 + (ULONG)(*rounds_text++ - '0');
    if (*rounds_text != 0 || rounds == 0 || rounds > 1000)
        return fail("STEADY rounds", (SIPTR)rounds);
    drawer = CreateDir(DRAWER);
    if (drawer == BNULL)
        return fail("STEADY CreateDir", DOSFALSE);
    /* The handler's port, and no lock held while measuring: with nothing
     * open the handler keeps no per-object state, so the heap is exact. */
    port = afsplus_client_lock_port(drawer);
    UnLock(drawer);
    status = RETURN_OK;
    /* Three times as many rounds first as are measured, so one-off
     * allocations of the first use of each operation, and structures
     * growing to their working size, are not read as a leak. The read cache
     * is one of them: a delayed mount leaves each round's delete for idle
     * time, the next round writes to blocks the cache has not held, and the
     * cache fills to its size over the first hundred rounds. Another took a
     * step of 2,176 bytes somewhere between the hundredth and the two
     * hundredth round once a close stopped committing the window, which a
     * warm-up as long as the measurement put between the two readings. */
    for (round = 0; status == RETURN_OK && round < 3 * rounds; round++)
        status = steady_round();
    if (status == RETURN_OK)
        status = steady_heap(port, &warm);
    orphans_warm = steady_orphans(port);
    /* Both readings follow a flush: between flushes a delayed mount holds
     * its open window and the deletes waiting for idle time, which the flush
     * gives back. */
    before = (ULONG)AvailMem(MEMF_ANY);
    /* Four readings, the warm one and three more a batch apart. What the
     * mount holds at a flush point moves by a kilobyte or two with where
     * the window's bound falls among the rounds, up on one machine and
     * down on another with the same code: 1,712 bytes fewer on Hosted and
     * 1,600 more under QEMU, the peak flat on both. A leak grows at every
     * reading; that does not. */
    held[0] = warm.heap_bytes;
    for (batch = 1; status == RETURN_OK && batch < 4; batch++)
    {
        for (round = 0; status == RETURN_OK && round < rounds; round++)
            status = steady_round();
        if (status == RETURN_OK)
            status = steady_heap(port, &done);
        held[batch] = done.heap_bytes;
    }
    orphans_done = steady_orphans(port);
    after = (ULONG)AvailMem(MEMF_ANY);
    DeleteFile(DRAWER);
    if (status != RETURN_OK)
        return status;
    Printf("[AFSPLUS-DOS] STEADY rounds %lu free before %lu after %lu\n",
        rounds, before, after);
    Printf("[AFSPLUS-DOS] STEADY heap before %lu after %lu peak before %lu"
        " after %lu\n", (ULONG)warm.heap_bytes, (ULONG)done.heap_bytes,
        (ULONG)warm.heap_peak_bytes, (ULONG)done.heap_peak_bytes);
    Printf("[AFSPLUS-DOS] STEADY orphans pending before %lu after %lu\n",
        orphans_warm, orphans_done);
    /* The handler library's own view, blind to other tasks: after the
     * warm-up, further rounds hold nothing more and need no more at once,
     * within the allowance, so the peak is a property of the operations and
     * not of how often they run. */
    Printf("[AFSPLUS-DOS] STEADY held %lu %lu %lu %lu\n", (ULONG)held[0],
        (ULONG)held[1], (ULONG)held[2], (ULONG)held[3]);
    /* A leak: the lower of the last two readings above the higher of the
     * first two. Three bytes a round over batches of a hundred is past the
     * allowance; a reading that swings and comes back is not. */
    {
        uint64_t early = held[0] > held[1] ? held[0] : held[1];
        uint64_t late = held[2] < held[3] ? held[2] : held[3];

        if (late > early + STEADY_HEAP_ALLOWANCE)
            return fail("heap held over the rounds", (SIPTR)(late - early));
    }
    if (done.heap_peak_bytes > warm.heap_peak_bytes + STEADY_HEAP_ALLOWANCE)
        return fail("heap peak grew over the rounds",
            (SIPTR)(done.heap_peak_bytes - warm.heap_peak_bytes));
    /* Every operation of a round is paired with what releases it, so a round
     * must cost nothing. Measured on Hosted the two numbers are equal to the
     * byte over 20 and over 100 rounds. The bound is not zero because
     * another task on the system may allocate while this runs; it is small
     * enough that a leak of one allocation per round, which cannot be less
     * than a few bytes, fails a run of fifty rounds or more. */
    if (before > after && before - after > STEADY_ALLOWANCE)
        return fail("memory lost over the rounds",
            (SIPTR)(before - after));
    return RETURN_OK;
}

int main(int argc, char **argv)
{
    struct FileInfoBlock *fib;
    int status;

    if (argc > 2 && strcmp(argv[1], "STEADY") == 0)
    {
        steady_leak = argc > 3 && strcmp(argv[3], "LEAK") == 0;
        return probe_steady(argv[2]);
    }
    if (argc > 1 && strcmp(argv[1], "RECORD-HOLDER") == 0)
        return hold_record();
    if (argc > 1 && strcmp(argv[1], "RECORD-WAITER") == 0)
        return wait_for_record();

    if (argc > 1 && strcmp(argv[1], "HOLD") == 0)
        return hold_notification();

    fib = AllocDosObject(DOS_FIB, NULL);
    if (fib == NULL)
        return fail("AllocDosObject", DOSFALSE);
    {
        BPTR drawer = CreateDir(DRAWER);

        if (drawer == BNULL)
        {
            FreeDosObject(DOS_FIB, fib);
            return fail("CreateDir", DOSFALSE);
        }
        UnLock(drawer);
    }
    status = write_file(NOTE, MODE_NEWFILE)
        ? RETURN_OK : fail("create note", DOSFALSE);
    if (status == RETURN_OK)
        status = probe_metadata(fib);
    if (status == RETURN_OK)
        status = probe_soft_link();
    if (status == RETURN_OK)
        status = probe_exall();
    if (status == RETURN_OK)
        status = probe_exall_pages();
    if (status == RETURN_OK)
        status = probe_dir_walk();
    if (status == RETURN_OK)
        status = probe_watch();
    if (status == RETURN_OK)
        status = probe_handles();
    if (status == RETURN_OK)
        status = probe_attributes();
    if (status == RETURN_OK)
        status = probe_notify();
    if (status == RETURN_OK)
        status = probe_relabel();
    FreeDosObject(DOS_FIB, fib);

    /* The namespace is restored on failure too, so a rerun starts clean. */
    DeleteFile(ALIAS);
    DeleteFile(NOTE);
    if (!DeleteFile(DRAWER) && status == RETURN_OK)
        status = fail("remove drawer", DOSFALSE);
    if (status == RETURN_OK)
        Printf("[AFSPLUS-DOS] PASS setters/comment/softlink/exall/pages/"
            "dirwalk/fromlock/changemode/records/attributes/notify/"
            "relabel\n");
    return status;
}
