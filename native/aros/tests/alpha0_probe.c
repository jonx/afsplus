/* SPDX-License-Identifier: BSD-2-Clause */

/* Target-side Mountable Alpha-0 operation and durability probe. */

#include <dos/dos.h>
#include <proto/dos.h>

#include <string.h>

#ifndef AFSPLUS_PROBE_VOLUME
#define AFSPLUS_PROBE_VOLUME "AFSPLUS19"
#endif

#ifndef AFSPLUS_PROBE_FUNCTION
#define AFSPLUS_PROBE_FUNCTION main
#endif

#define TEMP_PATH AFSPLUS_PROBE_VOLUME ":alpha0.tmp"
#define FINAL_PATH AFSPLUS_PROBE_VOLUME ":alpha0.from-aros"
#define FOLDED_FINAL_PATH AFSPLUS_PROBE_VOLUME ":ALPHA0.FROM-AROS"
#define SPARSE_OFFSET 8192

static const UBYTE prefix[] = "hello";
static const UBYTE tail[] = "tail";

static int fail(const char *stage, SIPTR result)
{
    Printf("[AFSPLUS-ALPHA0] FAIL %s result %ld error %ld\n",
        stage, result, IoErr());
    return RETURN_FAIL;
}

int AFSPLUS_PROBE_FUNCTION(void)
{
    UBYTE readback[sizeof(prefix)];
    BPTR file;
    BPTR lock;
    LONG result;

    SetIoErr(0);
    lock = Lock(TEMP_PATH, SHARED_LOCK);
    if (lock != BNULL)
    {
        UnLock(lock);
        return fail("temporary-file precondition", DOSFALSE);
    }
    if (IoErr() != ERROR_OBJECT_NOT_FOUND)
        return fail("temporary-file lookup", DOSFALSE);
    SetIoErr(0);
    lock = Lock(FINAL_PATH, SHARED_LOCK);
    if (lock != BNULL)
    {
        UnLock(lock);
        return fail("final-file precondition", DOSFALSE);
    }
    if (IoErr() != ERROR_OBJECT_NOT_FOUND)
        return fail("final-file lookup", DOSFALSE);

    file = Open(TEMP_PATH, MODE_NEWFILE);
    if (file == BNULL)
        return fail("create", DOSFALSE);
    result = Write(file, prefix, sizeof(prefix) - 1);
    if (result != (LONG)(sizeof(prefix) - 1))
    {
        Close(file);
        return fail("write prefix", result);
    }
    if (Seek(file, SPARSE_OFFSET, OFFSET_BEGINNING) < 0)
    {
        Close(file);
        return fail("sparse seek", -1);
    }
    result = Write(file, tail, sizeof(tail) - 1);
    if (result != (LONG)(sizeof(tail) - 1))
    {
        Close(file);
        return fail("write tail", result);
    }
    if (!Flush(file))
    {
        Close(file);
        return fail("fsync before truncate", DOSFALSE);
    }
    result = SetFileSize(file, sizeof(prefix) - 1, OFFSET_BEGINNING);
    if (result != (LONG)(sizeof(prefix) - 1))
    {
        Close(file);
        return fail("truncate", result);
    }
    if (!Flush(file))
    {
        Close(file);
        return fail("fsync after truncate", DOSFALSE);
    }
    if (!Close(file))
        return fail("close", DOSFALSE);

    if (!Rename(TEMP_PATH, FINAL_PATH))
        return fail("rename", DOSFALSE);
    lock = Lock(FOLDED_FINAL_PATH, SHARED_LOCK);
    if (lock == BNULL)
        return fail("case-folded lookup", DOSFALSE);
    UnLock(lock);
    if (!Rename(FINAL_PATH, FOLDED_FINAL_PATH))
        return fail("case-only rename", DOSFALSE);
    file = Open(FINAL_PATH, MODE_OLDFILE);
    if (file == BNULL)
        return fail("reopen", DOSFALSE);
    memset(readback, 0, sizeof(readback));
    result = Read(file, readback, sizeof(readback));
    if (result != (LONG)(sizeof(prefix) - 1)
        || memcmp(readback, prefix, sizeof(prefix) - 1) != 0)
    {
        Close(file);
        return fail("readback", result);
    }
    result = Read(file, readback, 1);
    if (result != 0)
    {
        Close(file);
        return fail("truncated eof", result);
    }
    if (!Close(file))
        return fail("read close", DOSFALSE);

    Printf("[AFSPLUS-ALPHA0] PASS create/read/write/truncate/rename/fsync/casefold\n");
    return RETURN_OK;
}
