/* SPDX-License-Identifier: BSD-2-Clause */

/* AFSPlusClone <source file> <target directory> <name>: makes the target a
 * clone of the source when both live on one AFS+ volume whose handler can
 * clone, and a byte copy otherwise. The last line says which it was, so a
 * caller never mistakes a copy for shared storage. An existing target is
 * never replaced. */

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <exec/memory.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <string.h>

#include "../client/afsplus_client.h"

#define COPY_BUFFER_BYTES 65536

static int fail(const char *stage)
{
    Printf("AFSPlusClone: %s: error %ld\n", stage, IoErr());
    return RETURN_FAIL;
}

/* Copies into a new file under directory. MODE_NEWFILE would replace an
 * existing target, so its absence is established first. */
static int byte_copy(BPTR source_lock, BPTR directory, CONST_STRPTR name)
{
    UBYTE *buffer;
    BPTR previous;
    BPTR existing;
    BPTR source;
    BPTR target;
    LONG count = 0;
    int status = RETURN_OK;

    previous = CurrentDir(directory);
    existing = Lock(name, SHARED_LOCK);
    if (existing != BNULL)
    {
        UnLock(existing);
        CurrentDir(previous);
        SetIoErr(ERROR_OBJECT_EXISTS);
        return fail("target");
    }
    target = Open(name, MODE_NEWFILE);
    CurrentDir(previous);
    if (target == BNULL)
        return fail("create target");
    /* OpenFromLock consumes the lock it is given. */
    source_lock = DupLock(source_lock);
    source = source_lock != BNULL ? OpenFromLock(source_lock) : BNULL;
    if (source == BNULL)
    {
        if (source_lock != BNULL)
            UnLock(source_lock);
        Close(target);
        return fail("open source");
    }
    buffer = AllocVec(COPY_BUFFER_BYTES, MEMF_ANY);
    if (buffer == NULL)
    {
        SetIoErr(ERROR_NO_FREE_STORE);
        status = fail("buffer");
    }
    while (status == RETURN_OK
        && (count = Read(source, buffer, COPY_BUFFER_BYTES)) > 0)
        if (Write(target, buffer, count) != count)
            status = fail("write target");
    if (status == RETURN_OK && count < 0)
        status = fail("read source");
    if (buffer != NULL)
        FreeVec(buffer);
    Close(source);
    if (!Close(target) && status == RETURN_OK)
        status = fail("close target");
    return status;
}

int main(int argc, char **argv)
{
    BPTR source;
    BPTR directory;
    LONG error;
    int status;

    if (argc != 4 || strpbrk(argv[3], ":/") != NULL)
    {
        Printf("usage: AFSPlusClone <source file> <target directory> "
            "<name>\n");
        return RETURN_ERROR;
    }
    source = Lock((CONST_STRPTR)argv[1], SHARED_LOCK);
    if (source == BNULL)
        return fail("source");
    directory = Lock((CONST_STRPTR)argv[2], SHARED_LOCK);
    if (directory == BNULL)
    {
        UnLock(source);
        return fail("target directory");
    }
    error = afsplus_client_clone_file(source, directory,
        (CONST_STRPTR)argv[3]);
    /* No transport, two volumes, or a volume without shared extents: the
     * portable path. Every other error is the answer. */
    if (error == ERROR_ACTION_NOT_KNOWN
        || error == ERROR_RENAME_ACROSS_DEVICES)
    {
        status = byte_copy(source, directory, (CONST_STRPTR)argv[3]);
        if (status == RETURN_OK)
            Printf("AFSPlusClone: copied\n");
    }
    else if (error != 0)
    {
        SetIoErr(error);
        status = fail("clone");
    }
    else
    {
        status = RETURN_OK;
        Printf("AFSPlusClone: cloned\n");
    }
    UnLock(directory);
    UnLock(source);
    return status;
}
