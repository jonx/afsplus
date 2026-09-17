/* SPDX-License-Identifier: BSD-2-Clause */

/* AFSPlusClone <source file> <target directory> <name>: makes the target a
 * clone of the source when both live on one AFS+ volume whose handler can
 * clone, and a byte copy otherwise. The last line says which it was, so a
 * caller never mistakes a copy for shared storage. An existing target is
 * never replaced, and a failed attempt leaves nothing behind. */

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>

#include <string.h>

#include "../client/afsplus_client.h"
#include "../client/afsplus_copy.h"

static int fail(const char *stage)
{
    Printf("AFSPlusClone: %s: error %ld\n", stage, IoErr());
    return RETURN_FAIL;
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
    /* No transport, a handler or volume that cannot clone, or two volumes:
     * the portable path. Every other error is the answer. */
    if (error == ERROR_ACTION_NOT_KNOWN || error == ERROR_NOT_IMPLEMENTED
        || error == ERROR_RENAME_ACROSS_DEVICES)
    {
        error = afsplus_copy_file(source, directory, (CONST_STRPTR)argv[3]);
        if (error == 0)
        {
            status = RETURN_OK;
            Printf("AFSPlusClone: copied\n");
        }
        else
        {
            SetIoErr(error);
            status = fail("copy");
        }
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
