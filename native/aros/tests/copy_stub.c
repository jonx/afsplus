/* SPDX-License-Identifier: BSD-2-Clause */

/* Host matrix of afsplus_copy_file over a faked directory: what is left
 * behind by a copy that succeeds, cannot start, fails half-way, or finds its
 * target taken at the last moment. */

#include "../client/afsplus_copy.h"

#include <proto/dos.h>
#include <proto/exec.h>

#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL);
    abort();
}

/* One directory of named byte strings. */
#define ENTRIES 8
struct Entry {
    char name[40];
    uint8_t bytes[200000];
    size_t length;
    int used;
};
static struct Entry directory[ENTRIES];
static SIPTR io_error;

/* Open handles: an entry, a position, a mode. */
struct Handle {
    struct Entry *entry;
    size_t position;
    int writing;
    int used;
};
static struct Handle handles[4];

static int fail_open_from_lock;
static long fail_write_after = -1;
static int fail_close;
static const char *appear_during_write;
static size_t written_total;

static struct Entry *find(const char *name)
{
    int index;

    for (index = 0; index < ENTRIES; index++)
        if (directory[index].used && strcmp(directory[index].name, name) == 0)
            return &directory[index];
    return NULL;
}

static struct Entry *create(const char *name)
{
    int index;

    for (index = 0; index < ENTRIES; index++)
        if (!directory[index].used)
        {
            memset(&directory[index], 0, sizeof(directory[index]));
            directory[index].used = 1;
            strcpy(directory[index].name, name);
            return &directory[index];
        }
    abort();
}

static int entries(void)
{
    int index;
    int count = 0;

    for (index = 0; index < ENTRIES; index++)
        count += directory[index].used;
    return count;
}

SIPTR IoErr(void) { return io_error; }
SIPTR SetIoErr(SIPTR value) { SIPTR old = io_error; io_error = value; return old; }
void Forbid(void) {}
void Permit(void) {}
struct Task *FindTask(CONST_STRPTR name) { (void)name; return (struct Task *)0x1000; }
APTR AllocVec(IPTR size, ULONG requirements) { (void)requirements; return malloc(size); }
void FreeVec(APTR memory) { free(memory); }
BPTR CurrentDir(BPTR lock) { (void)lock; return BNULL; }

BPTR Lock(CONST_STRPTR name, LONG mode)
{
    struct Entry *entry = find((const char *)name);

    (void)mode;
    if (entry == NULL)
    {
        io_error = ERROR_OBJECT_NOT_FOUND;
        return BNULL;
    }
    return MKBADDR(entry);
}

LONG UnLock(BPTR lock) { (void)lock; return DOSTRUE; }
BPTR DupLock(BPTR lock) { return lock; }

static BPTR open_entry(struct Entry *entry, int writing)
{
    int index;

    for (index = 0; index < 4; index++)
        if (!handles[index].used)
        {
            handles[index].used = 1;
            handles[index].entry = entry;
            handles[index].position = 0;
            handles[index].writing = writing;
            return MKBADDR(&handles[index]);
        }
    abort();
}

BPTR OpenFromLock(BPTR lock)
{
    if (fail_open_from_lock)
    {
        io_error = ERROR_OBJECT_IN_USE;
        return BNULL;
    }
    return open_entry(BADDR(lock), 0);
}

BPTR Open(CONST_STRPTR name, LONG mode)
{
    struct Entry *entry = find((const char *)name);

    assert(mode == MODE_NEWFILE);
    /* MODE_NEWFILE truncates what exists: the danger the copy must avoid. */
    if (entry == NULL)
        entry = create((const char *)name);
    entry->length = 0;
    return open_entry(entry, 1);
}

LONG Close(BPTR file)
{
    struct Handle *handle = BADDR(file);

    handle->used = 0;
    if (handle->writing && fail_close)
    {
        io_error = ERROR_DISK_FULL;
        return DOSFALSE;
    }
    return DOSTRUE;
}

LONG Read(BPTR file, APTR buffer, LONG length)
{
    struct Handle *handle = BADDR(file);
    size_t left = handle->entry->length - handle->position;

    if ((size_t)length > left)
        length = (LONG)left;
    memcpy(buffer, handle->entry->bytes + handle->position, (size_t)length);
    handle->position += (size_t)length;
    return length;
}

LONG Write(BPTR file, CONST_APTR buffer, LONG length)
{
    struct Handle *handle = BADDR(file);

    if (appear_during_write != NULL && find(appear_during_write) == NULL)
    {
        struct Entry *other = create(appear_during_write);

        memcpy(other->bytes, "other task", 10);
        other->length = 10;
    }
    if (fail_write_after >= 0 && written_total >= (size_t)fail_write_after)
    {
        io_error = ERROR_DISK_FULL;
        return -1;
    }
    memcpy(handle->entry->bytes + handle->position, buffer, (size_t)length);
    handle->position += (size_t)length;
    handle->entry->length = handle->position;
    written_total += (size_t)length;
    return length;
}

LONG Rename(CONST_STRPTR from, CONST_STRPTR to)
{
    struct Entry *entry = find((const char *)from);

    if (find((const char *)to) != NULL)
    {
        io_error = ERROR_OBJECT_EXISTS;
        return DOSFALSE;
    }
    assert(entry != NULL);
    strcpy(entry->name, (const char *)to);
    return DOSTRUE;
}

LONG DeleteFile(CONST_STRPTR name)
{
    struct Entry *entry = find((const char *)name);

    if (entry == NULL)
    {
        io_error = ERROR_OBJECT_NOT_FOUND;
        return DOSFALSE;
    }
    entry->used = 0;
    return DOSTRUE;
}

int main(void)
{
    struct Entry *source = create("source");
    struct Entry *target;
    size_t index;

    /* Larger than one buffer, so the copy loops. */
    source->length = 150000;
    for (index = 0; index < source->length; index++)
        source->bytes[index] = (uint8_t)(index * 7);

    assert(afsplus_copy_file(MKBADDR(source), BNULL, (CONST_STRPTR)"copy")
        == 0);
    target = find("copy");
    assert(target != NULL && target->length == source->length);
    assert(memcmp(target->bytes, source->bytes, source->length) == 0);
    assert(entries() == 2 && io_error == 0);

    /* An existing target is refused and keeps its bytes. */
    target->length = 3;
    assert(afsplus_copy_file(MKBADDR(source), BNULL, (CONST_STRPTR)"copy")
        == ERROR_OBJECT_EXISTS);
    assert(find("copy")->length == 3 && entries() == 2);
    DeleteFile((CONST_STRPTR)"copy");

    /* A source that cannot be opened creates nothing at all. */
    fail_open_from_lock = 1;
    assert(afsplus_copy_file(MKBADDR(source), BNULL, (CONST_STRPTR)"copy")
        == ERROR_OBJECT_IN_USE);
    assert(entries() == 1);
    fail_open_from_lock = 0;

    /* A write that fails half-way, and a close that fails, leave nothing:
     * the next attempt is not refused over a leftover. */
    written_total = 0;
    fail_write_after = 65536;
    assert(afsplus_copy_file(MKBADDR(source), BNULL, (CONST_STRPTR)"copy")
        == ERROR_DISK_FULL);
    assert(entries() == 1);
    fail_write_after = -1;
    fail_close = 1;
    assert(afsplus_copy_file(MKBADDR(source), BNULL, (CONST_STRPTR)"copy")
        == ERROR_DISK_FULL);
    assert(entries() == 1);
    fail_close = 0;
    assert(afsplus_copy_file(MKBADDR(source), BNULL, (CONST_STRPTR)"copy")
        == 0);
    assert(entries() == 2);
    DeleteFile((CONST_STRPTR)"copy");

    /* Another task creates the target while the bytes are being copied: its
     * file is neither truncated nor replaced, and the copy fails. */
    appear_during_write = "copy";
    assert(afsplus_copy_file(MKBADDR(source), BNULL, (CONST_STRPTR)"copy")
        == ERROR_OBJECT_EXISTS);
    appear_during_write = NULL;
    target = find("copy");
    assert(target != NULL && target->length == 10
        && memcmp(target->bytes, "other task", 10) == 0);
    assert(entries() == 2 && io_error == 0);

    puts("afsplus copy stub: PASS");
    return 0;
}
