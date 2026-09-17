/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_copy.h"

#include <exec/memory.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <stdint.h>
#include <string.h>

#define COPY_BUFFER_BYTES 65536
#define TEMPORARY_ATTEMPTS 16

static LONG last_error(LONG fallback)
{
    LONG error = (LONG)IoErr();

    return error != 0 ? error : fallback;
}

/* ".afsplus-copy-" and eight hexadecimal digits: distinct per task and per
 * attempt, so two copies never share a temporary file. */
static void temporary_name(char *name, uint32_t salt)
{
    static const char digits[] = "0123456789abcdef";
    static const char prefix[] = ".afsplus-copy-";
    uint32_t value = (uint32_t)(uintptr_t)FindTask(NULL) * UINT32_C(2654435761)
        + salt;
    size_t at = sizeof(prefix) - 1;
    int shift;

    memcpy(name, prefix, at);
    for (shift = 28; shift >= 0; shift -= 4)
        name[at++] = digits[(value >> shift) & 15];
    name[at] = 0;
}

/* A new, empty temporary file. MODE_NEWFILE would truncate a file of the
 * same name, so the name is first seen to be free; a name taken in between
 * belongs to a copy of another task, which the task address in the name
 * excludes. */
static BPTR create_temporary(char *name, LONG *error)
{
    uint32_t attempt;

    for (attempt = 0; attempt < TEMPORARY_ATTEMPTS; attempt++)
    {
        BPTR existing;
        BPTR file;

        temporary_name(name, attempt);
        existing = Lock((CONST_STRPTR)name, SHARED_LOCK);
        if (existing != BNULL)
        {
            UnLock(existing);
            continue;
        }
        if (IoErr() != ERROR_OBJECT_NOT_FOUND)
            break;
        file = Open((CONST_STRPTR)name, MODE_NEWFILE);
        if (file != BNULL)
            return file;
        break;
    }
    *error = attempt == TEMPORARY_ATTEMPTS ? ERROR_OBJECT_EXISTS
        : last_error(ERROR_UNKNOWN);
    return BNULL;
}

LONG afsplus_copy_file(BPTR source, BPTR directory, CONST_STRPTR name)
{
    char temporary[32];
    UBYTE *buffer;
    BPTR source_lock;
    BPTR input;
    BPTR output;
    BPTR previous;
    LONG count = 0;
    LONG error = 0;

    if (source == BNULL || name == NULL || name[0] == 0)
        return ERROR_REQUIRED_ARG_MISSING;
    /* The source first: a copy that cannot start creates nothing.
     * OpenFromLock consumes the lock it is given. */
    source_lock = DupLock(source);
    if (source_lock == BNULL)
        return last_error(ERROR_UNKNOWN);
    input = OpenFromLock(source_lock);
    if (input == BNULL)
    {
        error = last_error(ERROR_UNKNOWN);
        UnLock(source_lock);
        SetIoErr(0);
        return error;
    }
    buffer = AllocVec(COPY_BUFFER_BYTES, MEMF_ANY);
    if (buffer == NULL)
    {
        Close(input);
        SetIoErr(0);
        return ERROR_NO_FREE_STORE;
    }

    previous = CurrentDir(directory);
    output = create_temporary(temporary, &error);
    if (output != BNULL)
    {
        while ((count = Read(input, buffer, COPY_BUFFER_BYTES)) > 0)
            if (Write(output, buffer, count) != count)
            {
                error = last_error(ERROR_UNKNOWN);
                break;
            }
        if (error == 0 && count < 0)
            error = last_error(ERROR_UNKNOWN);
        /* A close that fails has not stored the bytes. */
        if (!Close(output) && error == 0)
            error = last_error(ERROR_UNKNOWN);
        /* Rename does not replace: a target that exists, whenever it came
         * into being, makes the copy fail and stays as it is. */
        if (error == 0 && !Rename((CONST_STRPTR)temporary, name))
            error = last_error(ERROR_OBJECT_EXISTS);
        if (error != 0)
            DeleteFile((CONST_STRPTR)temporary);
    }
    CurrentDir(previous);
    FreeVec(buffer);
    Close(input);
    SetIoErr(0);
    return error;
}
