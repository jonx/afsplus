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

int main(int argc, char **argv)
{
    struct FileInfoBlock *fib;
    int status;

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
        Printf("[AFSPLUS-DOS] PASS setters/comment/softlink/exall/"
            "fromlock/changemode/records/attributes/notify/relabel\n");
    return status;
}
