/* SPDX-License-Identifier: BSD-2-Clause */

/* Target-side verification after mounting an intent-log crash fixture. */

#include <dos/dos.h>
#include <proto/dos.h>

#include <string.h>

#define HEAD_PATH "AFSPLUS19:HEAD"
#define LOCK_PATH "AFSPLUS19:HEAD.lock"

static int fail(const char *stage, SIPTR result)
{
    Printf("[AFSPLUS-REPLAY] FAIL %s result %ld error %ld\n",
        stage, result, IoErr());
    return RETURN_FAIL;
}

int main(int argc, char **argv)
{
    UBYTE content[4];
    const char *expected;
    BPTR file;
    BPTR lock;
    LONG result;

    if (argc != 2 || (strcmp(argv[1], "old") != 0 && strcmp(argv[1], "new") != 0))
        return fail("usage: AFSPlusReplayProbe old|new", argc);
    expected = argv[1];

    SetIoErr(0);
    lock = Lock(LOCK_PATH, SHARED_LOCK);
    if (lock != BNULL)
    {
        UnLock(lock);
        return fail("temporary name survived", DOSTRUE);
    }
    if (IoErr() != ERROR_OBJECT_NOT_FOUND)
        return fail("temporary-name lookup", DOSFALSE);

    file = Open(HEAD_PATH, MODE_OLDFILE);
    if (file == BNULL)
        return fail("open HEAD", DOSFALSE);
    memset(content, 0, sizeof(content));
    result = Read(file, content, sizeof(content));
    if (result != 3 || memcmp(content, expected, 3) != 0)
    {
        Close(file);
        return fail("HEAD content", result);
    }
    result = Read(file, content, 1);
    if (result != 0)
    {
        Close(file);
        return fail("HEAD eof", result);
    }
    if (!Close(file))
        return fail("close HEAD", DOSFALSE);

    Printf("[AFSPLUS-REPLAY] PASS expected=%s\n", expected);
    return RETURN_OK;
}
