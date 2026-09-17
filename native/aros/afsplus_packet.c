/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_packet.h"

#ifndef AFSPLUS_AROS_TRACE_STARTUP
#define AFSPLUS_AROS_TRACE_STARTUP 0
#endif

#if AFSPLUS_AROS_TRACE_STARTUP
extern void afsplus_aros_trace_stage(const char *stage);
#define AFSPLUS_PACKET_TRACE(stage) afsplus_aros_trace_stage(stage)
#else
#define AFSPLUS_PACKET_TRACE(stage) ((void)0)
#endif

#include <dos/dos.h>
#include <dos/dosextens.h>
#include <dos/dosasl.h>
#include <dos/exall.h>
#include <dos/record.h>
#include <aros/stdc/string.h>

#define AFSPLUS_NATIVE_LOCK_MAGIC UINT32_C(0x41464c4b)
#define AFSPLUS_NATIVE_FILE_MAGIC UINT32_C(0x41464648)
#define AFSPLUS_UNIX_TO_AMIGA_EPOCH INT64_C(252460800)
#define AFSPLUS_SECONDS_PER_DAY INT64_C(86400)
#define AFSPLUS_SECONDS_PER_MINUTE INT64_C(60)
#define AFSPLUS_TICKS_PER_SECOND UINT32_C(50)
/* fib_Comment holds a length byte and 79 characters; ACTION_SET_COMMENT
 * refuses more and every report is cut to it. */
#define AFSPLUS_DOS_COMMENT_MAX 79

/* One directory entry read from the filesystem that did not fit the caller's
 * ExAll buffer. It is returned first by the next ACTION_EXAMINE_ALL. */
struct AfsplusArosPendingEntry {
    struct AfsplusArosFileInfo info;
    uint8_t name[MAXFILENAMELENGTH];
    uint8_t comment[AFSPLUS_DOS_COMMENT_MAX];
    uint32_t comment_length;
};

struct AfsplusArosNativeLock {
    struct FileLock public_lock;
    uint32_t magic;
    struct AfsplusArosPacketContext *owner;
    uint64_t id;
    struct AfsplusArosNativeLock *next;
    /* Allocated by the first ACTION_EXAMINE_ALL on this lock. */
    struct AfsplusArosPendingEntry *exall;
    uint32_t exall_pending;
    /* eac_LastKey of the one ExAll sequence that owns the directory cursor,
     * zero when none does. */
    uint32_t exall_key;
};

struct AfsplusArosNativeFile {
    uint32_t magic;
    uint32_t writable;
    struct AfsplusArosPacketContext *owner;
    uint64_t id;
    struct AfsplusArosNativeFile *next;
};

struct AfsplusArosNativeNotify {
    struct NotifyRequest *request;
    uint64_t watch;
    struct AfsplusArosNativeNotify *next;
};

struct AfsplusArosPacketContext {
    struct AfsplusAros *filesystem;
    struct MsgPort *handler_port;
    BPTR volume_node;
    void *callback_context;
    AfsplusArosPacketAllocate allocate;
    AfsplusArosPacketFree free;
    AfsplusArosPacketNow now;
    struct AfsplusArosNativeLock *locks;
    struct AfsplusArosNativeFile *files;
    struct AfsplusArosNativeNotify *notifies;
    AfsplusArosPacketNotify notify;
    AfsplusArosPacketRelabel relabel;
    uint32_t exall_serial;
    uint32_t inhibited;
    uint32_t quit;
    uint64_t groups;
    uint32_t revision;
};

struct AfsplusPathOperation {
    const uint8_t *name;
    uint32_t length;
    uint32_t parent;
};

struct AfsplusResolvedParent {
    uint64_t id;
    uint32_t owned;
    const uint8_t *leaf;
    uint32_t leaf_length;
};

static int32_t packet_now(struct AfsplusArosPacketContext *context,
    int64_t *seconds, uint32_t *nanoseconds)
{
    int32_t error;

    if (context->now == NULL)
        return ERROR_ACTION_NOT_KNOWN;
    error = context->now(context->callback_context, seconds, nanoseconds);
    if (error == 0 && *nanoseconds >= UINT32_C(1000000000))
        return ERROR_BAD_NUMBER;
    return error;
}

static int32_t bstr_view(SIPTR raw, const uint8_t **bytes,
    uint32_t *length)
{
    size_t native_length;
    BSTR value;

    if (raw == 0 || bytes == NULL || length == NULL)
        return ERROR_INVALID_COMPONENT_NAME;
    value = (BSTR)raw;
#ifdef AROS_FAST_BSTR
    native_length = strlen((const char *)AROS_BSTR_ADDR(value));
#else
    native_length = (size_t)AROS_BSTR_strlen(value);
#endif
    if (native_length > UINT32_MAX)
        return ERROR_LINE_TOO_LONG;
    *bytes = (const uint8_t *)AROS_BSTR_ADDR(value);
    *length = (uint32_t)native_length;
    return 0;
}

static struct AfsplusArosNativeLock *find_lock(
    struct AfsplusArosPacketContext *context, BPTR raw)
{
    struct AfsplusArosNativeLock *lock;
    void *candidate;

    if (raw == BNULL)
        return NULL;
    candidate = BADDR(raw);
    for (lock = context->locks; lock != NULL; lock = lock->next)
        if ((void *)lock == candidate && lock->magic == AFSPLUS_NATIVE_LOCK_MAGIC
            && lock->owner == context)
            return lock;
    return NULL;
}

static struct AfsplusArosNativeFile *find_file(
    struct AfsplusArosPacketContext *context, BPTR raw)
{
    struct AfsplusArosNativeFile *file;
    void *candidate;

    if (raw == BNULL)
        return NULL;
    candidate = BADDR(raw);
    for (file = context->files; file != NULL; file = file->next)
        if ((void *)file == candidate && file->magic == AFSPLUS_NATIVE_FILE_MAGIC
            && file->owner == context)
            return file;
    return NULL;
}

static int32_t lock_id(struct AfsplusArosPacketContext *context, BPTR raw,
    uint64_t *id)
{
    struct AfsplusArosNativeLock *lock;

    if (raw == BNULL)
    {
        *id = 0;
        return 0;
    }
    lock = find_lock(context, raw);
    if (lock == NULL)
        return ERROR_INVALID_LOCK;
    *id = lock->id;
    return 0;
}

static uint32_t ffi_access(LONG access, int32_t *error)
{
    if (access == SHARED_LOCK)
        return AFSPLUS_AROS_LOCK_SHARED;
    if (access == EXCLUSIVE_LOCK)
        return AFSPLUS_AROS_LOCK_EXCLUSIVE;
    *error = ERROR_BAD_NUMBER;
    return AFSPLUS_AROS_LOCK_SHARED;
}

static uint32_t ffi_seek_mode(LONG mode, int32_t *error)
{
    if (mode == OFFSET_BEGINNING)
        return AFSPLUS_AROS_SEEK_BEGINNING;
    if (mode == OFFSET_CURRENT)
        return AFSPLUS_AROS_SEEK_CURRENT;
    if (mode == OFFSET_END)
        return AFSPLUS_AROS_SEEK_END;
    *error = ERROR_SEEK_ERROR;
    return AFSPLUS_AROS_SEEK_BEGINNING;
}

static struct AfsplusArosNativeLock *reserve_lock(
    struct AfsplusArosPacketContext *context, LONG access, int32_t *error)
{
    struct AfsplusArosNativeLock *lock;

    lock = context->allocate(context->callback_context, sizeof(*lock));
    if (lock == NULL)
    {
        *error = ERROR_NO_FREE_STORE;
        return NULL;
    }
    memset(lock, 0, sizeof(*lock));
    lock->public_lock.fl_Key = (IPTR)lock;
    lock->public_lock.fl_Access = access;
    lock->public_lock.fl_Task = context->handler_port;
    lock->public_lock.fl_Volume = context->volume_node;
    lock->magic = AFSPLUS_NATIVE_LOCK_MAGIC;
    lock->owner = context;
    return lock;
}

static void publish_lock(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeLock *lock, uint64_t id)
{
    lock->id = id;
    lock->next = context->locks;
    context->locks = lock;
}

static void discard_reserved_lock(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeLock *lock)
{
    lock->magic = 0;
    context->free(context->callback_context, lock, sizeof(*lock));
}

static struct AfsplusArosNativeLock *wrap_lock(
    struct AfsplusArosPacketContext *context, uint64_t id, LONG access,
    int32_t *error)
{
    struct AfsplusArosNativeLock *lock;

    if (id == 0)
    {
        *error = ERROR_INVALID_LOCK;
        return NULL;
    }
    lock = reserve_lock(context, access, error);
    if (lock != NULL)
        publish_lock(context, lock, id);
    return lock;
}

static struct AfsplusArosNativeFile *reserve_file(
    struct AfsplusArosPacketContext *context, uint32_t writable,
    int32_t *error)
{
    struct AfsplusArosNativeFile *file;

    file = context->allocate(context->callback_context, sizeof(*file));
    if (file == NULL)
    {
        *error = ERROR_NO_FREE_STORE;
        return NULL;
    }
    memset(file, 0, sizeof(*file));
    file->magic = AFSPLUS_NATIVE_FILE_MAGIC;
    file->writable = writable;
    file->owner = context;
    return file;
}

static void publish_file(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeFile *file, uint64_t id)
{
    file->id = id;
    file->next = context->files;
    context->files = file;
}

static void discard_reserved_file(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeFile *file)
{
    file->magic = 0;
    context->free(context->callback_context, file, sizeof(*file));
}

static void unlink_lock(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeLock *lock)
{
    struct AfsplusArosNativeLock **link;

    for (link = &context->locks; *link != NULL; link = &(*link)->next)
        if (*link == lock)
        {
            *link = lock->next;
            lock->magic = 0;
            if (lock->exall != NULL)
                context->free(context->callback_context, lock->exall,
                    sizeof(*lock->exall));
            context->free(context->callback_context, lock, sizeof(*lock));
            return;
        }
}

static void unlink_file(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeFile *file)
{
    struct AfsplusArosNativeFile **link;

    for (link = &context->files; *link != NULL; link = &(*link)->next)
        if (*link == file)
        {
            *link = file->next;
            file->magic = 0;
            context->free(context->callback_context, file, sizeof(*file));
            return;
        }
}

static void release_temporary_lock(struct AfsplusArosPacketContext *context,
    uint64_t id, uint32_t owned)
{
    if (owned && id != 0)
        (void)afsplus_aros_free_lock(context->filesystem, id);
}

static uint32_t path_start(const uint8_t *path, uint32_t length)
{
    uint32_t i;
    uint32_t start = 0;

    for (i = 0; i < length; i++)
        if (path[i] == ':')
            start = i + 1;
    return start;
}

static uint32_t next_path_operation(const uint8_t *path, uint32_t length,
    uint32_t *at, struct AfsplusPathOperation *operation)
{
    uint32_t start;

    if (*at >= length)
        return 0;
    start = *at;
    while (*at < length && path[*at] != '/')
        (*at)++;
    operation->name = path + start;
    operation->length = *at - start;
    operation->parent = operation->length == 0;
    if (*at < length)
        (*at)++;
    return 1;
}

static int32_t resolve_path_lock(struct AfsplusArosPacketContext *context,
    uint64_t base, const uint8_t *path, uint32_t length, uint32_t access,
    uint64_t *output)
{
    struct AfsplusPathOperation operation;
    struct AfsplusPathOperation ignored;
    uint32_t at = path_start(path, length);
    uint32_t owned = 0;
    uint64_t current = base;
    int32_t error;

    if (at != 0)
        current = 0;

    if (!next_path_operation(path, length, &at, &operation))
        return afsplus_aros_locate(context->filesystem, current, NULL, 0,
            access, output);

    do
    {
        uint32_t peek = at;
        uint32_t final = !next_path_operation(path, length, &peek, &ignored);
        uint32_t step_access = final ? access : AFSPLUS_AROS_LOCK_SHARED;
        uint64_t next = 0;

        if (operation.parent)
        {
            if (current != 0)
                error = afsplus_aros_parent_lock_with_access(
                    context->filesystem, current, step_access, &next);
            else
                error = 0;
            if (error == 0 && final && next == 0)
                error = afsplus_aros_locate(context->filesystem, 0, NULL, 0,
                    step_access, &next);
        }
        else
            error = afsplus_aros_locate(context->filesystem, current,
                operation.name, operation.length, step_access, &next);

        if (error != 0)
        {
            release_temporary_lock(context, current, owned);
            return error;
        }
        release_temporary_lock(context, current, owned);
        current = next;
        owned = current != 0;
    } while (next_path_operation(path, length, &at, &operation));

    *output = current;
    return 0;
}

static int32_t resolve_parent(struct AfsplusArosPacketContext *context,
    uint64_t base, const uint8_t *path, uint32_t length,
    struct AfsplusResolvedParent *result)
{
    uint32_t i;
    uint32_t colon = 0;
    uint32_t slash = UINT32_MAX;
    uint32_t leaf;
    uint32_t prefix_length;
    int32_t error;

    for (i = 0; i < length; i++)
    {
        if (path[i] == ':')
        {
            colon = i + 1;
            slash = UINT32_MAX;
        }
        else if (path[i] == '/' && i >= colon)
            slash = i;
    }
    leaf = slash != UINT32_MAX ? slash + 1 : colon;
    if (leaf >= length)
        return ERROR_INVALID_COMPONENT_NAME;

    result->leaf = path + leaf;
    result->leaf_length = length - leaf;
    result->id = base;
    result->owned = 0;

    if (slash == UINT32_MAX && colon == 0)
        return 0;
    /* Include the separator in the parent prefix. A trailing separator is
     * inert, while a second consecutive separator remains an empty component
     * and therefore performs the AmigaDOS parent operation. */
    prefix_length = slash != UINT32_MAX ? slash + 1 : colon;
    error = resolve_path_lock(context, base, path, prefix_length,
        AFSPLUS_AROS_LOCK_SHARED, &result->id);
    if (error == 0)
        result->owned = result->id != 0;
    return error;
}

static int32_t require_group(const struct AfsplusArosPacketContext *context,
    uint64_t group)
{
    return (context->groups & group) != 0 ? 0 : ERROR_ACTION_NOT_KNOWN;
}

static int32_t datestamp_to_unix(const struct DateStamp *date,
    int64_t *seconds, uint32_t *nanoseconds)
{
    if (date == NULL || date->ds_Days < 0 || date->ds_Minute < 0
        || date->ds_Minute >= 24 * 60 || date->ds_Tick < 0
        || date->ds_Tick >= 60 * (LONG)AFSPLUS_TICKS_PER_SECOND)
        return ERROR_BAD_NUMBER;
    *seconds = AFSPLUS_UNIX_TO_AMIGA_EPOCH
        + (int64_t)date->ds_Days * AFSPLUS_SECONDS_PER_DAY
        + (int64_t)date->ds_Minute * AFSPLUS_SECONDS_PER_MINUTE
        + date->ds_Tick / (LONG)AFSPLUS_TICKS_PER_SECOND;
    *nanoseconds = (uint32_t)(date->ds_Tick % (LONG)AFSPLUS_TICKS_PER_SECOND)
        * UINT32_C(20000000);
    return 0;
}

static uint32_t c_string_length(const uint8_t *text)
{
    uint32_t length = 0;

    while (text[length] != 0 && length < UINT32_MAX)
        length++;
    return length;
}

/* Resolves the object named by a DOS path to a base lock plus leaf. An empty
 * leaf ("", "VOL:" or "dir/") addresses the resolved lock's own object. */
static int32_t resolve_named_object(struct AfsplusArosPacketContext *context,
    uint64_t base, const uint8_t *path, uint32_t length,
    struct AfsplusResolvedParent *result)
{
    int32_t error = resolve_parent(context, base, path, length, result);

    if (error != ERROR_INVALID_COMPONENT_NAME)
        return error;
    result->leaf = path + length;
    result->leaf_length = 0;
    result->id = base;
    result->owned = 0;
    error = resolve_path_lock(context, base, path, length,
        AFSPLUS_AROS_LOCK_SHARED, &result->id);
    if (error == 0)
        result->owned = result->id != 0;
    return error;
}

/* ACTION_READ_LINK: finds the first soft link along path and writes the path
 * that dos.library retries with: the components before the link, the link
 * target, then the components after it. A target naming a volume replaces
 * the prefix. Returns the length, -2 when the buffer is too small, or -1 with
 * *error set. */
static SIPTR read_link_path(struct AfsplusArosPacketContext *context,
    uint64_t base, const uint8_t *path, uint32_t length, uint8_t *buffer,
    uint32_t capacity, int32_t *error)
{
    struct AfsplusPathOperation operation;
    uint32_t at = path_start(path, length);
    uint64_t current = at != 0 ? 0 : base;
    uint32_t owned = 0;

    *error = 0;
    for (;;)
    {
        uint32_t start = at;
        uint64_t next = 0;
        uint32_t required = 0;

        if (!next_path_operation(path, length, &at, &operation))
        {
            *error = ERROR_OBJECT_WRONG_TYPE;
            break;
        }
        if (operation.parent)
        {
            if (current != 0)
                *error = afsplus_aros_parent_lock_with_access(
                    context->filesystem, current, AFSPLUS_AROS_LOCK_SHARED,
                    &next);
        }
        else
        {
            *error = afsplus_aros_read_soft_link(context->filesystem,
                current, operation.name, operation.length, buffer, capacity,
                &required);
            if (*error == 0)
            {
                uint32_t rest = length - at;
                uint32_t prefix = start;
                uint32_t separator;
                uint32_t total;
                uint32_t i;

                release_temporary_lock(context, current, owned);
                if (required > capacity)
                    return -2;
                for (i = 0; i < required; i++)
                    if (buffer[i] == ':')
                        prefix = 0;
                separator = rest != 0 && required != 0
                    && buffer[required - 1] != '/'
                    && buffer[required - 1] != ':';
                if (required > UINT32_MAX - prefix
                    || rest > UINT32_MAX - prefix - required - separator - 1)
                    return -2;
                total = prefix + required + separator + rest;
                if (total + 1 > capacity)
                    return -2;
                memmove(buffer + prefix, buffer, required);
                memcpy(buffer, path, prefix);
                if (separator)
                    buffer[prefix + required] = '/';
                memcpy(buffer + prefix + required + separator, path + at,
                    rest);
                buffer[total] = 0;
                return (SIPTR)total;
            }
            if (*error != ERROR_OBJECT_WRONG_TYPE)
                break;
            *error = afsplus_aros_locate(context->filesystem, current,
                operation.name, operation.length, AFSPLUS_AROS_LOCK_SHARED,
                &next);
        }
        if (*error != 0)
            break;
        release_temporary_lock(context, current, owned);
        current = next;
        owned = current != 0;
    }
    release_temporary_lock(context, current, owned);
    return -1;
}

static void unix_to_datestamp(int64_t seconds, uint32_t nanoseconds,
    struct DateStamp *date)
{
    int64_t amiga;
    int64_t day_seconds;
    int64_t days;
    int64_t minutes;
    uint64_t ticks;

    if (seconds <= AFSPLUS_UNIX_TO_AMIGA_EPOCH)
        amiga = 0;
    else
        amiga = seconds - AFSPLUS_UNIX_TO_AMIGA_EPOCH;
    days = amiga / AFSPLUS_SECONDS_PER_DAY;
    day_seconds = amiga % AFSPLUS_SECONDS_PER_DAY;
    minutes = day_seconds / AFSPLUS_SECONDS_PER_MINUTE;
    ticks = (uint64_t)(day_seconds % AFSPLUS_SECONDS_PER_MINUTE)
        * AFSPLUS_TICKS_PER_SECOND;
    ticks += nanoseconds / UINT32_C(20000000);

    if (days > INT32_MAX)
        days = INT32_MAX;
    date->ds_Days = (LONG)days;
    date->ds_Minute = (LONG)minutes;
    date->ds_Tick = ticks > INT32_MAX ? INT32_MAX : (LONG)ticks;
}

static const size_t exall_fixed_size[] =
{
    0,
    offsetof(struct ExAllData, ed_Type),
    offsetof(struct ExAllData, ed_Size),
    offsetof(struct ExAllData, ed_Prot),
    offsetof(struct ExAllData, ed_Days),
    offsetof(struct ExAllData, ed_Comment),
    offsetof(struct ExAllData, ed_OwnerUID),
    sizeof(struct ExAllData)
};

/* Appends one entry at *cursor when it fits before end. Strings follow the
 * fixed part and the next entry starts pointer-aligned. */
static uint32_t exall_append(uint8_t **cursor, uint8_t *end, LONG type,
    const struct AfsplusArosPendingEntry *source, struct ExAllData **last)
{
    struct ExAllData *entry = (struct ExAllData *)*cursor;
    size_t name_length = source->info.name_length;
    size_t need = exall_fixed_size[type] + name_length + 1
        + (type >= ED_COMMENT ? (size_t)source->comment_length + 1 : 0);
    uint8_t *strings;

    /* The entry's own bytes decide whether it fits; the padding only places
     * the next entry and may run past a buffer this entry exactly fills. */
    if (need > (size_t)(end - *cursor))
        return 0;
    need = (need + sizeof(void *) - 1) & ~(sizeof(void *) - 1);
    if (need > (size_t)(end - *cursor))
        need = (size_t)(end - *cursor);
    strings = *cursor + exall_fixed_size[type];
    entry->ed_Next = NULL;
    entry->ed_Name = strings;
    memcpy(strings, source->name, name_length);
    strings[name_length] = 0;
    if (type >= ED_TYPE)
        entry->ed_Type = source->info.directory_entry_type;
    if (type >= ED_SIZE)
    {
        /* ed_Size is unsigned: a 32-bit record saturates at its maximum. */
        if (sizeof(entry->ed_Size) == sizeof(uint32_t)
            && source->info.size > (uint64_t)UINT32_MAX)
            entry->ed_Size = UINT32_MAX;
        else
            entry->ed_Size = source->info.size;
    }
    if (type >= ED_PROTECTION)
        entry->ed_Prot = source->info.protection;
    if (type >= ED_DATE)
    {
        struct DateStamp date;

        unix_to_datestamp(source->info.modified_seconds,
            source->info.modified_nanoseconds, &date);
        entry->ed_Days = (ULONG)date.ds_Days;
        entry->ed_Mins = (ULONG)date.ds_Minute;
        entry->ed_Ticks = (ULONG)date.ds_Tick;
    }
    if (type >= ED_COMMENT)
    {
        entry->ed_Comment = strings + name_length + 1;
        memcpy(entry->ed_Comment, source->comment, source->comment_length);
        entry->ed_Comment[source->comment_length] = 0;
    }
    if (type >= ED_OWNER)
    {
        entry->ed_OwnerUID = 0;
        entry->ed_OwnerGID = 0;
    }
    if (*last != NULL)
        (*last)->ed_Next = entry;
    *last = entry;
    *cursor += need;
    return 1;
}

static int32_t fill_fib_name(UBYTE *destination, size_t capacity,
    const uint8_t *name, uint32_t length)
{
    if ((size_t)length + 1 > capacity || length > UINT8_MAX)
        return ERROR_OBJECT_TOO_LARGE;
    destination[0] = (UBYTE)length;
    if (length != 0)
        memcpy(destination + 1, name, length);
    if ((size_t)length + 1 < capacity)
        destination[length + 1] = 0;
    return 0;
}

static int32_t fill_fib64(struct FileInfoBlock64 *fib,
    const struct AfsplusArosFileInfo *info, const uint8_t *name,
    const uint8_t *comment, uint32_t comment_length)
{
    int32_t error;

    memset(fib, 0, sizeof(*fib));
    error = fill_fib_name(fib->fib_FileName, sizeof(fib->fib_FileName),
        name, info->name_length);
    if (error == 0)
        error = fill_fib_name(fib->fib_Comment, sizeof(fib->fib_Comment),
            comment, comment_length);
    if (error != 0)
        return error;
    fib->fib_DiskKey = info->disk_key > (uint64_t)INTPTR_MAX
        ? INTPTR_MAX : (IPTR)info->disk_key;
    fib->fib_DirEntryType = (LONG)info->directory_entry_type;
    fib->fib_Protection = (LONG)info->protection;
    fib->fib_EntryType = (LONG)info->entry_type;
    fib->fib_Size = info->size;
    fib->fib_NumBlocks = info->blocks;
    unix_to_datestamp(info->modified_seconds, info->modified_nanoseconds,
        &fib->fib_Date);
    return 0;
}

#if !(__DOS64)
static int32_t fill_fib32(struct FileInfoBlock32 *fib,
    const struct AfsplusArosFileInfo *info, const uint8_t *name,
    const uint8_t *comment, uint32_t comment_length)
{
    int32_t error;

    memset(fib, 0, sizeof(*fib));
    error = fill_fib_name(fib->fib_FileName, sizeof(fib->fib_FileName),
        name, info->name_length);
    if (error == 0)
        error = fill_fib_name(fib->fib_Comment, sizeof(fib->fib_Comment),
            comment, comment_length);
    if (error != 0)
        return error;
    fib->fib_DiskKey = info->disk_key > (uint64_t)INTPTR_MAX
        ? INTPTR_MAX : (IPTR)info->disk_key;
    fib->fib_DirEntryType = (LONG)info->directory_entry_type;
    fib->fib_Protection = (LONG)info->protection;
    fib->fib_EntryType = (LONG)info->entry_type;
    fib->fib_Size = info->size > INT32_MAX ? INT32_MAX : (LONG)info->size;
    fib->fib_NumBlocks = info->blocks > INT32_MAX
        ? INT32_MAX : (LONG)info->blocks;
    unix_to_datestamp(info->modified_seconds, info->modified_nanoseconds,
        &fib->fib_Date);
    return 0;
}
#endif

static int32_t fill_packet_fib(LONG action, BPTR raw,
    const struct AfsplusArosFileInfo *info, const uint8_t *name,
    const uint8_t *comment, uint32_t comment_length)
{
    void *destination;

    if (raw == BNULL)
        return ERROR_INVALID_LOCK;
    destination = BADDR(raw);
    if (action == ACTION_EXAMINE_OBJECT64
        || action == ACTION_EXAMINE_NEXT64
        || action == ACTION_EXAMINE_FH64)
        return fill_fib64((struct FileInfoBlock64 *)destination, info, name,
            comment, comment_length);
#if (__DOS64)
    return fill_fib64((struct FileInfoBlock64 *)destination, info, name,
            comment, comment_length);
#else
    return fill_fib32((struct FileInfoBlock32 *)destination, info, name,
        comment, comment_length);
#endif
}

static void fill_info64(struct AfsplusArosPacketContext *context,
    struct InfoData64 *destination, const struct AfsplusArosDiskInfo *info)
{
    memset(destination, 0, sizeof(*destination));
    destination->id_DiskState = info->write_protected
        ? ID_WRITE_PROTECTED : ID_VALIDATED;
    destination->id_NumBlocks = info->total_blocks;
    destination->id_NumBlocksUsed = info->used_blocks;
    destination->id_BytesPerBlock = info->bytes_per_block > INT32_MAX
        ? INT32_MAX : (LONG)info->bytes_per_block;
    destination->id_DiskType = (LONG)info->disk_type;
    destination->id_VolumeNode = context->volume_node;
    destination->id_InUse = info->in_use ? DOSTRUE : DOSFALSE;
}

#if !(__DOS64)
static void fill_info32(struct AfsplusArosPacketContext *context,
    struct InfoData32 *destination, const struct AfsplusArosDiskInfo *info)
{
    memset(destination, 0, sizeof(*destination));
    destination->id_DiskState = info->write_protected
        ? ID_WRITE_PROTECTED : ID_VALIDATED;
    destination->id_NumBlocks = info->total_blocks > INT32_MAX
        ? INT32_MAX : (LONG)info->total_blocks;
    destination->id_NumBlocksUsed = info->used_blocks > INT32_MAX
        ? INT32_MAX : (LONG)info->used_blocks;
    destination->id_BytesPerBlock = info->bytes_per_block > INT32_MAX
        ? INT32_MAX : (LONG)info->bytes_per_block;
    destination->id_DiskType = (LONG)info->disk_type;
    destination->id_VolumeNode = context->volume_node;
    destination->id_InUse = info->in_use ? DOSTRUE : DOSFALSE;
}
#endif

static int32_t fill_packet_info(struct AfsplusArosPacketContext *context,
    LONG action, BPTR raw, const struct AfsplusArosDiskInfo *info)
{
    void *destination;

    if (raw == BNULL)
        return ERROR_INVALID_LOCK;
    destination = BADDR(raw);
    if (action == ACTION_INFO64)
        fill_info64(context, (struct InfoData64 *)destination, info);
#if (__DOS64)
    else
        fill_info64(context, (struct InfoData64 *)destination, info);
#else
    else
        fill_info32(context, (struct InfoData32 *)destination, info);
#endif
    return 0;
}

static int32_t add_offset(uint64_t base, int64_t offset, uint64_t *result)
{
    uint64_t magnitude;

    if (offset < 0)
    {
        magnitude = (uint64_t)(-(offset + 1)) + 1;
        if (magnitude > base)
            return ERROR_SEEK_ERROR;
        *result = base - magnitude;
    }
    else
    {
        magnitude = (uint64_t)offset;
        if (magnitude > UINT64_MAX - base)
            return ERROR_OBJECT_TOO_LARGE;
        *result = base + magnitude;
    }
    return 0;
}

static int32_t prospective_size(struct AfsplusArosPacketContext *context,
    uint64_t file, int64_t offset, uint32_t mode, uint64_t *size)
{
    uint64_t base;
    int32_t error;

    if (mode == AFSPLUS_AROS_SEEK_BEGINNING)
        base = 0;
    else if (mode == AFSPLUS_AROS_SEEK_CURRENT)
    {
        error = afsplus_aros_file_position(context->filesystem, file, &base);
        if (error != 0)
            return error;
    }
    else
    {
        error = afsplus_aros_file_size(context->filesystem, file, &base);
        if (error != 0)
            return error;
    }
    return add_offset(base, offset, size);
}

#if (__WORDSIZE != 64)
static uint32_t is_packet64_action(LONG action)
{
    return action == ACTION_CHANGE_FILE_POSITION64
        || action == ACTION_GET_FILE_POSITION64
        || action == ACTION_CHANGE_FILE_SIZE64
        || action == ACTION_GET_FILE_SIZE64;
}
#endif

static void store_packet_result(struct DosPacket *packet, SIPTR result,
    int64_t result64, int32_t error, uint32_t packet64)
{
#if (__WORDSIZE != 64)
    if (packet64)
    {
        struct DosPacket64 *wide = (struct DosPacket64 *)packet;
        wide->dp_Res1 = (QUAD)result64;
        wide->dp_Res2 = (ULONG)error;
        return;
    }
#else
    (void)result64;
    (void)packet64;
#endif
    packet->dp_Res1 = result;
    packet->dp_Res2 = error;
}

int32_t afsplus_aros_packet_create(
    const struct AfsplusArosPacketConfig *config,
    struct AfsplusArosPacketContext **output)
{
    struct AfsplusArosPacketContext *context;

    if (config == NULL || output == NULL)
        return ERROR_BAD_NUMBER;
    *output = NULL;
    if (config->abi_version != AFSPLUS_AROS_PACKET_ABI_VERSION
        || config->struct_size != sizeof(*config)
        || config->filesystem == NULL || config->allocate == NULL
        || config->free == NULL)
        return ERROR_BAD_NUMBER;
    context = config->allocate(config->callback_context, sizeof(*context));
    if (context == NULL)
        return ERROR_NO_FREE_STORE;
    memset(context, 0, sizeof(*context));
    context->filesystem = config->filesystem;
    context->handler_port = config->handler_port;
    context->volume_node = config->volume_node;
    context->callback_context = config->callback_context;
    context->allocate = config->allocate;
    context->free = config->free;
    context->now = config->now;
    context->notify = config->notify;
    context->relabel = config->relabel;
    {
        struct AfsplusArosInterface interface;

        memset(&interface, 0, sizeof(interface));
        interface.struct_size = sizeof(interface);
        if (afsplus_aros_interface(&interface) != 0
            || interface.abi_version != AFSPLUS_AROS_ABI_VERSION
            || (interface.groups & AFSPLUS_AROS_GROUP_BASE) == 0)
        {
            config->free(config->callback_context, context, sizeof(*context));
            return ERROR_BAD_NUMBER;
        }
        context->groups = interface.groups;
        context->revision = interface.interface_revision;
    }
    *output = context;
    return 0;
}

int32_t afsplus_aros_packet_destroy(
    struct AfsplusArosPacketContext *context)
{
    int32_t first_error = 0;

    if (context == NULL)
        return ERROR_BAD_NUMBER;
    while (context->files != NULL)
    {
        struct AfsplusArosNativeFile *file = context->files;
        int32_t error = 0;
        int32_t close_error;

        if (file->writable)
            error = afsplus_aros_fsync(context->filesystem, file->id);
        close_error = afsplus_aros_close(context->filesystem, file->id);
        if (error == 0)
            error = close_error;
        if (first_error == 0)
            first_error = error;
        unlink_file(context, file);
    }
    while (context->locks != NULL)
    {
        struct AfsplusArosNativeLock *lock = context->locks;
        int32_t error = afsplus_aros_free_lock(context->filesystem, lock->id);
        if (first_error == 0)
            first_error = error;
        unlink_lock(context, lock);
    }
    while (context->notifies != NULL)
    {
        struct AfsplusArosNativeNotify *node = context->notifies;

        context->notifies = node->next;
        /* EndNotify skips a request without a handler instead of sending a
         * packet to a port that is gone. */
        node->request->nr_Handler = NULL;
        (void)afsplus_aros_watch_remove(context->filesystem, node->watch);
        context->free(context->callback_context, node, sizeof(*node));
    }
    {
        AfsplusArosPacketFree free_callback = context->free;
        void *callback_context = context->callback_context;
        free_callback(callback_context, context, sizeof(*context));
    }
    return first_error;
}

uint32_t afsplus_aros_packet_notify_registered(
    const struct AfsplusArosPacketContext *context,
    const struct NotifyRequest *request)
{
    const struct AfsplusArosNativeNotify *node;

    if (context == NULL || request == NULL)
        return 0;
    for (node = context->notifies; node != NULL; node = node->next)
        if (node->request == request)
            return 1;
    return 0;
}

uint32_t afsplus_aros_packet_should_quit(
    const struct AfsplusArosPacketContext *context)
{
    return context != NULL ? context->quit : 0;
}

/* Turns the watches that fired during this packet into deliveries. The
 * filesystem coalesces per watch, so one packet yields at most one delivery
 * per request. */
static void deliver_notifications(struct AfsplusArosPacketContext *context)
{
    uint64_t fired[16];
    uint32_t count;
    uint32_t i;

    if (context->notify == NULL || context->notifies == NULL)
        return;
    do
    {
        count = 0;
        if (afsplus_aros_watch_drain(context->filesystem, fired,
                (uint32_t)(sizeof(fired) / sizeof(fired[0])), &count) != 0)
            return;
        for (i = 0; i < count; i++)
        {
            struct AfsplusArosNativeNotify *node;

            for (node = context->notifies; node != NULL; node = node->next)
                if (node->watch == fired[i])
                {
                    context->notify(context->callback_context,
                        node->request);
                    break;
                }
        }
    } while (count == sizeof(fired) / sizeof(fired[0]));
}

int32_t afsplus_aros_packet_process(
    struct AfsplusArosPacketContext *context, struct DosPacket *packet)
{
    SIPTR result = DOSFALSE;
    int64_t result64 = DOSFALSE;
    int32_t error = 0;
    uint32_t packet64 = 0;
    struct NotifyRequest *initial_notify = NULL;

    if (context == NULL || packet == NULL)
        return ERROR_BAD_NUMBER;
#if (__WORDSIZE != 64)
    if (is_packet64_action(packet->dp_Type))
    {
        struct DosPacket64 *wide = (struct DosPacket64 *)packet;
        packet64 = wide->dp_Res0 == DP64_INIT;
        if (!packet64)
        {
            store_packet_result(packet, DOSFALSE, DOSFALSE,
                ERROR_BAD_NUMBER, 0);
            return 0;
        }
    }
#endif

    switch (packet->dp_Type)
    {
    case ACTION_LOCATE_OBJECT:
    {
        const uint8_t *path;
        uint32_t path_length = 0;
        uint64_t base;
        uint64_t id = 0;
        LONG native_access = (LONG)packet->dp_Arg3;
        uint32_t access = ffi_access(native_access, &error);
        struct AfsplusArosNativeLock *lock = NULL;

        if (error == 0)
            error = lock_id(context, (BPTR)packet->dp_Arg1, &base);
        if (error == 0)
            error = bstr_view(packet->dp_Arg2, &path, &path_length);
        if (error == 0)
            error = resolve_path_lock(context, base, path, path_length,
                access, &id);
        if (error == 0)
            lock = wrap_lock(context, id, native_access, &error);
        if (error != 0 && id != 0 && lock == NULL)
            (void)afsplus_aros_free_lock(context->filesystem, id);
        if (error == 0)
            result = (SIPTR)MKBADDR(lock);
        break;
    }
    case ACTION_FREE_LOCK:
    {
        struct AfsplusArosNativeLock *lock = find_lock(context,
            (BPTR)packet->dp_Arg1);
        if (lock == NULL)
            error = ERROR_INVALID_LOCK;
        else
        {
            error = afsplus_aros_free_lock(context->filesystem, lock->id);
            if (error == 0)
            {
                unlink_lock(context, lock);
                result = DOSTRUE;
            }
        }
        break;
    }
    case ACTION_COPY_DIR:
    case ACTION_COPY_DIR_FH:
    {
        uint64_t id = 0;
        LONG native_access = SHARED_LOCK;
        struct AfsplusArosNativeLock *copy = NULL;

        if (packet->dp_Type == ACTION_COPY_DIR_FH)
        {
            struct AfsplusArosNativeFile *file = find_file(context,
                (BPTR)packet->dp_Arg1);
            if (file == NULL)
                error = ERROR_INVALID_LOCK;
            else
                error = afsplus_aros_lock_from_file(context->filesystem,
                    file->id, &id);
        }
        else if ((BPTR)packet->dp_Arg1 == BNULL)
            error = afsplus_aros_locate(context->filesystem, 0, NULL, 0,
                AFSPLUS_AROS_LOCK_SHARED, &id);
        else
        {
            struct AfsplusArosNativeLock *source = find_lock(context,
                (BPTR)packet->dp_Arg1);
            if (source == NULL)
                error = ERROR_INVALID_LOCK;
            else
            {
                native_access = source->public_lock.fl_Access;
                error = afsplus_aros_duplicate_lock(context->filesystem,
                    source->id, &id);
            }
        }
        if (error == 0)
            copy = wrap_lock(context, id, native_access, &error);
        if (error != 0 && id != 0 && copy == NULL)
            (void)afsplus_aros_free_lock(context->filesystem, id);
        if (error == 0)
            result = (SIPTR)MKBADDR(copy);
        break;
    }
    case ACTION_PARENT:
    case ACTION_PARENT_FH:
    {
        uint64_t id = 0;
        struct AfsplusArosNativeLock *parent = NULL;

        if (packet->dp_Type == ACTION_PARENT_FH)
        {
            struct AfsplusArosNativeFile *file = find_file(context,
                (BPTR)packet->dp_Arg1);
            if (file == NULL)
                error = ERROR_INVALID_LOCK;
            else
                error = afsplus_aros_parent_of_file(context->filesystem,
                    file->id, &id);
        }
        else
        {
            struct AfsplusArosNativeLock *lock = find_lock(context,
                (BPTR)packet->dp_Arg1);
            if (lock == NULL)
                error = ERROR_INVALID_LOCK;
            else
                error = afsplus_aros_parent_lock(context->filesystem,
                    lock->id, &id);
        }
        if (error == 0 && id != 0)
            parent = wrap_lock(context, id, SHARED_LOCK, &error);
        if (error != 0 && id != 0 && parent == NULL)
            (void)afsplus_aros_free_lock(context->filesystem, id);
        if (error == 0)
            result = parent != NULL ? (SIPTR)MKBADDR(parent) : 0;
        break;
    }
    case ACTION_SAME_LOCK:
    {
        uint64_t first;
        uint64_t second;
        uint32_t same = 0;

        error = lock_id(context, (BPTR)packet->dp_Arg1, &first);
        if (error == 0)
            error = lock_id(context, (BPTR)packet->dp_Arg2, &second);
        if (error == 0)
            error = afsplus_aros_same_lock(context->filesystem, first,
                second, &same);
        if (error == 0 && same)
            result = DOSTRUE;
        break;
    }
    case ACTION_FINDINPUT:
    case ACTION_FINDUPDATE:
    case ACTION_FINDOUTPUT:
    {
        AFSPLUS_PACKET_TRACE("find-output-enter");
        struct FileHandle *public_file = packet->dp_Arg1 != 0
            ? (struct FileHandle *)BADDR((BPTR)packet->dp_Arg1) : NULL;
        const uint8_t *path = NULL;
        uint32_t path_length = 0;
        uint64_t base;
        uint64_t id = 0;
        uint32_t mode;
        uint32_t writable;
        int64_t seconds = 0;
        uint32_t nanoseconds = 0;
        struct AfsplusResolvedParent parent;
        struct AfsplusArosNativeFile *file = NULL;
        uint32_t parent_ready = 0;

        if (public_file == NULL)
            error = ERROR_INVALID_LOCK;
        if (error == 0)
            error = lock_id(context, (BPTR)packet->dp_Arg2, &base);
        if (error == 0)
            error = bstr_view(packet->dp_Arg3, &path, &path_length);
        if (packet->dp_Type == ACTION_FINDINPUT)
        {
            mode = AFSPLUS_AROS_OPEN_OLD_FILE;
            writable = 0;
        }
        else if (packet->dp_Type == ACTION_FINDOUTPUT)
        {
            mode = AFSPLUS_AROS_OPEN_NEW_FILE;
            writable = 1;
        }
        else
        {
            mode = AFSPLUS_AROS_OPEN_READ_WRITE;
            writable = 1;
        }
        if (error == 0)
        {
            AFSPLUS_PACKET_TRACE("find-output-parent-before");
            error = resolve_parent(context, base, path, path_length, &parent);
            AFSPLUS_PACKET_TRACE("find-output-parent-after");
            parent_ready = error == 0;
        }
        if (error == 0 && writable)
        {
            AFSPLUS_PACKET_TRACE("find-output-now-before");
            error = packet_now(context, &seconds, &nanoseconds);
            AFSPLUS_PACKET_TRACE("find-output-now-after");
        }
        if (error == 0)
        {
            AFSPLUS_PACKET_TRACE("find-output-reserve-before");
            file = reserve_file(context, writable, &error);
            AFSPLUS_PACKET_TRACE("find-output-reserve-after");
        }
        if (error == 0)
        {
            AFSPLUS_PACKET_TRACE("find-output-rust-before");
            error = afsplus_aros_open(context->filesystem, parent.id,
                parent.leaf, parent.leaf_length, mode, seconds, nanoseconds,
                &id);
            AFSPLUS_PACKET_TRACE("find-output-rust-after");
        }
        if (error == 0)
        {
            AFSPLUS_PACKET_TRACE("find-output-publish");
            publish_file(context, file, id);
        }
        if (error != 0 && id != 0)
            (void)afsplus_aros_close(context->filesystem, id);
        if (error != 0 && file != NULL)
            discard_reserved_file(context, file);
        if (parent_ready)
            release_temporary_lock(context, parent.id, parent.owned);
        if (error == 0)
        {
            public_file->fh_Arg1 = (SIPTR)MKBADDR(file);
            public_file->fh_Port = DOSFALSE;
            result = DOSTRUE;
        }
        break;
    }
    case ACTION_READ:
    case ACTION_WRITE:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);
        uint32_t count = 0;
        uint64_t requested;

        if (file == NULL)
            error = ERROR_INVALID_LOCK;
        requested = packet->dp_Arg3 < 0 ? UINT64_MAX
            : (uint64_t)packet->dp_Arg3;
        if (error == 0 && requested > UINT32_MAX)
            error = ERROR_BAD_NUMBER;
        if (error == 0 && requested != 0 && packet->dp_Arg2 == 0)
            error = ERROR_BAD_NUMBER;
        if (error == 0 && packet->dp_Type == ACTION_READ)
            error = afsplus_aros_read(context->filesystem, file->id,
                (uint8_t *)packet->dp_Arg2, (uint32_t)requested, &count);
        else if (error == 0)
        {
            int64_t seconds;
            uint32_t nanoseconds;
            if (!file->writable)
                error = ERROR_DISK_WRITE_PROTECTED;
            if (error == 0)
                error = packet_now(context, &seconds, &nanoseconds);
            if (error == 0)
                error = afsplus_aros_write(context->filesystem, file->id,
                    (const uint8_t *)packet->dp_Arg2, (uint32_t)requested,
                    seconds, nanoseconds, &count);
        }
        result = error == 0 ? (SIPTR)count : (SIPTR)-1;
        break;
    }
    case ACTION_SEEK:
    case ACTION_SEEK64:
    case ACTION_CHANGE_FILE_POSITION64:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);
        int64_t offset;
        LONG native_mode;
        uint32_t mode;
        uint64_t old = 0;

        if (file == NULL)
            error = ERROR_INVALID_LOCK;
#if (__WORDSIZE != 64)
        if (error == 0 && packet->dp_Type == ACTION_SEEK64)
            error = ERROR_ACTION_NOT_KNOWN;
        if (packet64)
        {
            struct DosPacket64 *wide = (struct DosPacket64 *)packet;
            offset = (int64_t)wide->dp_Arg2;
            native_mode = (LONG)wide->dp_Arg3;
        }
        else
#endif
        {
            offset = (int64_t)packet->dp_Arg2;
            native_mode = (LONG)packet->dp_Arg3;
        }
        mode = ffi_seek_mode(native_mode, &error);
        if (error == 0 && packet->dp_Type != ACTION_CHANGE_FILE_POSITION64)
        {
            error = afsplus_aros_file_position(context->filesystem,
                file->id, &old);
            if (error == 0 && packet->dp_Type == ACTION_SEEK
                && old > INT32_MAX)
                error = ERROR_OBJECT_TOO_LARGE;
            if (error == 0 && packet->dp_Type == ACTION_SEEK64
                && old > INT64_MAX)
                error = ERROR_OBJECT_TOO_LARGE;
        }
        if (error == 0)
            error = afsplus_aros_seek(context->filesystem, file->id, offset,
                mode, &old);
        if (error == 0)
        {
            result64 = packet->dp_Type == ACTION_CHANGE_FILE_POSITION64
                ? DOSTRUE : (int64_t)old;
            result = (SIPTR)result64;
        }
        else
        {
            result64 = -1;
            result = -1;
        }
        break;
    }
    case ACTION_SET_FILE_SIZE:
    case ACTION_SET_FILE_SIZE64:
    case ACTION_CHANGE_FILE_SIZE64:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);
        int64_t offset;
        LONG native_mode;
        uint32_t mode;
        uint64_t size = 0;
        int64_t seconds;
        uint32_t nanoseconds;

        if (file == NULL)
            error = ERROR_INVALID_LOCK;
        else if (!file->writable)
            error = ERROR_DISK_WRITE_PROTECTED;
#if (__WORDSIZE != 64)
        if (error == 0 && packet->dp_Type == ACTION_SET_FILE_SIZE64)
            error = ERROR_ACTION_NOT_KNOWN;
        if (packet64)
        {
            struct DosPacket64 *wide = (struct DosPacket64 *)packet;
            offset = (int64_t)wide->dp_Arg2;
            native_mode = (LONG)wide->dp_Arg3;
        }
        else
#endif
        {
            offset = (int64_t)packet->dp_Arg2;
            native_mode = (LONG)packet->dp_Arg3;
        }
        mode = ffi_seek_mode(native_mode, &error);
        if (error == 0 && packet->dp_Type == ACTION_SET_FILE_SIZE)
        {
            error = prospective_size(context, file->id, offset, mode, &size);
            if (error == 0 && size > INT32_MAX)
                error = ERROR_OBJECT_TOO_LARGE;
        }
        else if (error == 0 && packet->dp_Type == ACTION_SET_FILE_SIZE64)
        {
            error = prospective_size(context, file->id, offset, mode, &size);
            if (error == 0 && size > INT64_MAX)
                error = ERROR_OBJECT_TOO_LARGE;
        }
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            error = afsplus_aros_set_file_size(context->filesystem, file->id,
                offset, mode, seconds, nanoseconds, &size);
        if (error == 0)
        {
            result64 = packet->dp_Type == ACTION_CHANGE_FILE_SIZE64
                ? DOSTRUE : (int64_t)size;
            result = (SIPTR)result64;
        }
        else
        {
            result64 = -1;
            result = -1;
        }
        break;
    }
    case ACTION_GET_FILE_POSITION64:
    case ACTION_GET_FILE_SIZE64:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);
        uint64_t value = 0;

        if (file == NULL)
            error = ERROR_INVALID_LOCK;
        else if (packet->dp_Type == ACTION_GET_FILE_POSITION64)
            error = afsplus_aros_file_position(context->filesystem,
                file->id, &value);
        else
            error = afsplus_aros_file_size(context->filesystem, file->id,
                &value);
        if (error == 0 && value > INT64_MAX)
            error = ERROR_OBJECT_TOO_LARGE;
        result64 = error == 0 ? (int64_t)value : -1;
        result = error == 0 ? (SIPTR)result64 : (SIPTR)-1;
        break;
    }
    case ACTION_END:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);
        int32_t close_error;

        if (file == NULL)
            error = ERROR_INVALID_LOCK;
        else
        {
            if (file->writable)
                error = afsplus_aros_fsync(context->filesystem, file->id);
            close_error = afsplus_aros_close(context->filesystem, file->id);
            if (error == 0)
                error = close_error;
            unlink_file(context, file);
            if (error == 0)
                result = DOSTRUE;
        }
        break;
    }
    case ACTION_CREATE_DIR:
    {
        const uint8_t *path;
        uint32_t path_length = 0;
        uint64_t base;
        uint64_t id = 0;
        int64_t seconds;
        uint32_t nanoseconds;
        struct AfsplusResolvedParent parent;
        struct AfsplusArosNativeLock *lock = NULL;
        uint32_t parent_ready = 0;

        error = lock_id(context, (BPTR)packet->dp_Arg1, &base);
        if (error == 0)
            error = bstr_view(packet->dp_Arg2, &path, &path_length);
        if (error == 0)
        {
            error = resolve_parent(context, base, path, path_length, &parent);
            parent_ready = error == 0;
        }
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            lock = reserve_lock(context, SHARED_LOCK, &error);
        if (error == 0)
            error = afsplus_aros_create_directory(context->filesystem,
                parent.id, parent.leaf, parent.leaf_length, seconds,
                nanoseconds, &id);
        if (error == 0)
            publish_lock(context, lock, id);
        if (error != 0 && id != 0)
            (void)afsplus_aros_free_lock(context->filesystem, id);
        if (error != 0 && lock != NULL)
            discard_reserved_lock(context, lock);
        if (parent_ready)
            release_temporary_lock(context, parent.id, parent.owned);
        if (error == 0)
            result = (SIPTR)MKBADDR(lock);
        break;
    }
    case ACTION_DELETE_OBJECT:
    {
        const uint8_t *path;
        uint32_t path_length = 0;
        uint64_t base;
        int64_t seconds;
        uint32_t nanoseconds;
        struct AfsplusResolvedParent parent;
        uint32_t parent_ready = 0;

        error = lock_id(context, (BPTR)packet->dp_Arg1, &base);
        if (error == 0)
            error = bstr_view(packet->dp_Arg2, &path, &path_length);
        if (error == 0)
        {
            error = resolve_parent(context, base, path, path_length, &parent);
            parent_ready = error == 0;
        }
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            error = afsplus_aros_delete_object(context->filesystem,
                parent.id, parent.leaf, parent.leaf_length, seconds,
                nanoseconds);
        if (parent_ready)
            release_temporary_lock(context, parent.id, parent.owned);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_RENAME_OBJECT:
    {
        const uint8_t *source_path;
        const uint8_t *target_path;
        uint32_t source_length;
        uint32_t target_length;
        uint64_t source_base;
        uint64_t target_base;
        int64_t seconds;
        uint32_t nanoseconds;
        struct AfsplusResolvedParent source;
        struct AfsplusResolvedParent target;
        uint32_t source_ready = 0;
        uint32_t target_ready = 0;

        error = lock_id(context, (BPTR)packet->dp_Arg1, &source_base);
        if (error == 0)
            error = lock_id(context, (BPTR)packet->dp_Arg3, &target_base);
        if (error == 0)
            error = bstr_view(packet->dp_Arg2, &source_path, &source_length);
        if (error == 0)
            error = bstr_view(packet->dp_Arg4, &target_path, &target_length);
        if (error == 0)
        {
            error = resolve_parent(context, source_base, source_path,
                source_length, &source);
            source_ready = error == 0;
        }
        if (error == 0)
        {
            error = resolve_parent(context, target_base, target_path,
                target_length, &target);
            target_ready = error == 0;
        }
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            error = afsplus_aros_rename(context->filesystem, source.id,
                source.leaf, source.leaf_length, target.id, target.leaf,
                target.leaf_length, seconds, nanoseconds);
        if (target_ready)
            release_temporary_lock(context, target.id, target.owned);
        if (source_ready)
            release_temporary_lock(context, source.id, source.owned);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_MAKE_LINK:
    {
        const uint8_t *path;
        uint32_t path_length = 0;
        uint64_t base;
        int64_t seconds;
        uint32_t nanoseconds;
        struct AfsplusResolvedParent parent;
        struct AfsplusArosNativeLock *source;
        uint32_t parent_ready = 0;

        uint32_t soft = (LONG)packet->dp_Arg4 == LINK_SOFT;

        error = lock_id(context, (BPTR)packet->dp_Arg1, &base);
        source = soft ? NULL : find_lock(context, (BPTR)packet->dp_Arg3);
        if (error == 0 && !soft && (LONG)packet->dp_Arg4 != LINK_HARD)
            error = ERROR_ACTION_NOT_KNOWN;
        if (error == 0 && !soft && source == NULL)
            error = ERROR_INVALID_LOCK;
        if (error == 0 && soft)
            error = require_group(context, AFSPLUS_AROS_GROUP_SOFT_LINKS);
        if (error == 0 && soft && packet->dp_Arg3 == 0)
            error = ERROR_REQUIRED_ARG_MISSING;
        if (error == 0)
            error = bstr_view(packet->dp_Arg2, &path, &path_length);
        if (error == 0)
        {
            error = resolve_parent(context, base, path, path_length, &parent);
            parent_ready = error == 0;
        }
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0 && soft)
        {
            const uint8_t *target = (const uint8_t *)packet->dp_Arg3;

            error = afsplus_aros_make_soft_link(context->filesystem,
                parent.id, parent.leaf, parent.leaf_length, target,
                c_string_length(target), seconds, nanoseconds);
        }
        else if (error == 0)
            error = afsplus_aros_make_hard_link(context->filesystem,
                parent.id, parent.leaf, parent.leaf_length, source->id,
                seconds, nanoseconds);
        if (parent_ready)
            release_temporary_lock(context, parent.id, parent.owned);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_SET_COMMENT:
    {
        const uint8_t *path;
        const uint8_t *comment;
        uint32_t path_length = 0;
        uint32_t comment_length = 0;
        uint64_t base;
        int64_t seconds;
        uint32_t nanoseconds;
        struct AfsplusResolvedParent object;
        uint32_t object_ready = 0;

        error = require_group(context, AFSPLUS_AROS_GROUP_DOS_COMMENT);
        if (error == 0)
            error = lock_id(context, (BPTR)packet->dp_Arg2, &base);
        if (error == 0)
            error = bstr_view(packet->dp_Arg3, &path, &path_length);
        if (error == 0)
            error = bstr_view(packet->dp_Arg4, &comment, &comment_length);
        if (error == 0 && comment_length > AFSPLUS_DOS_COMMENT_MAX)
            error = ERROR_COMMENT_TOO_BIG;
        if (error == 0)
        {
            error = resolve_named_object(context, base, path, path_length,
                &object);
            object_ready = error == 0;
        }
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            error = afsplus_aros_set_comment(context->filesystem, object.id,
                object.leaf, object.leaf_length, comment, comment_length,
                seconds, nanoseconds);
        if (object_ready)
            release_temporary_lock(context, object.id, object.owned);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_SET_PROTECT:
    case ACTION_SET_DATE:
    {
        const uint8_t *path;
        uint32_t path_length = 0;
        uint64_t base;
        int64_t seconds;
        uint32_t nanoseconds;
        int64_t modified_seconds = 0;
        uint32_t modified_nanoseconds = 0;
        struct AfsplusResolvedParent object;
        uint32_t object_ready = 0;

        error = require_group(context, AFSPLUS_AROS_GROUP_DOS_METADATA);
        if (error == 0)
            error = lock_id(context, (BPTR)packet->dp_Arg2, &base);
        if (error == 0)
            error = bstr_view(packet->dp_Arg3, &path, &path_length);
        if (error == 0 && packet->dp_Type == ACTION_SET_DATE)
            error = datestamp_to_unix(
                (const struct DateStamp *)packet->dp_Arg4,
                &modified_seconds, &modified_nanoseconds);
        if (error == 0)
        {
            error = resolve_named_object(context, base, path, path_length,
                &object);
            object_ready = error == 0;
        }
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0 && packet->dp_Type == ACTION_SET_PROTECT)
            error = afsplus_aros_set_protection(context->filesystem,
                object.id, object.leaf, object.leaf_length,
                (uint32_t)packet->dp_Arg4, seconds, nanoseconds);
        else if (error == 0)
            error = afsplus_aros_set_modified(context->filesystem,
                object.id, object.leaf, object.leaf_length, modified_seconds,
                modified_nanoseconds, seconds, nanoseconds);
        if (object_ready)
            release_temporary_lock(context, object.id, object.owned);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_READ_LINK:
    {
        const uint8_t *path = (const uint8_t *)packet->dp_Arg2;
        uint8_t *buffer = (uint8_t *)packet->dp_Arg3;
        uint64_t base;

        result = -1;
        error = require_group(context, AFSPLUS_AROS_GROUP_SOFT_LINKS);
        if (error == 0)
            error = lock_id(context, (BPTR)packet->dp_Arg1, &base);
        if (error == 0 && (path == NULL || buffer == NULL
            || packet->dp_Arg4 <= 0))
            error = ERROR_REQUIRED_ARG_MISSING;
        if (error == 0)
        {
            uint32_t capacity = packet->dp_Arg4 > (SIPTR)UINT32_MAX
                ? UINT32_MAX : (uint32_t)packet->dp_Arg4;

            result = read_link_path(context, base, path,
                c_string_length(path), buffer, capacity, &error);
            if (result == -2)
                error = ERROR_LINE_TOO_LONG;
        }
        break;
    }
    case ACTION_EXAMINE_OBJECT:
    case ACTION_EXAMINE_OBJECT64:
    case ACTION_EXAMINE_FH:
    case ACTION_EXAMINE_FH64:
    case ACTION_EXAMINE_NEXT:
    case ACTION_EXAMINE_NEXT64:
    {
        struct AfsplusArosFileInfo info;
        uint8_t name[MAXFILENAMELENGTH];
        uint8_t comment[AFSPLUS_DOS_COMMENT_MAX];
        uint32_t comment_length = 0;
        uint64_t temporary_root = 0;
        uint64_t id = 0;
        /* A library without the comment group reports no comment. */
        uint32_t comments = require_group(context,
            AFSPLUS_AROS_GROUP_DOS_COMMENT) == 0;

        if ((BPTR)packet->dp_Arg2 == BNULL)
            error = ERROR_INVALID_LOCK;
        if (packet->dp_Type == ACTION_EXAMINE_FH
            || packet->dp_Type == ACTION_EXAMINE_FH64)
        {
            struct AfsplusArosNativeFile *file = find_file(context,
                (BPTR)packet->dp_Arg1);
            if (error == 0 && file == NULL)
                error = ERROR_INVALID_LOCK;
            else if (error == 0)
                error = afsplus_aros_examine_file(context->filesystem,
                    file->id, &info, name, sizeof(name));
            if (error == 0 && comments)
                error = afsplus_aros_file_comment(context->filesystem,
                    file->id, comment, sizeof(comment), &comment_length);
        }
        else
        {
            struct AfsplusArosNativeLock *lock = find_lock(context,
                (BPTR)packet->dp_Arg1);
            if (error == 0 && (BPTR)packet->dp_Arg1 == BNULL
                && (packet->dp_Type == ACTION_EXAMINE_OBJECT
                    || packet->dp_Type == ACTION_EXAMINE_OBJECT64))
            {
                error = afsplus_aros_locate(context->filesystem, 0, NULL, 0,
                    AFSPLUS_AROS_LOCK_SHARED, &temporary_root);
                id = temporary_root;
            }
            else if (error == 0 && lock == NULL)
                error = ERROR_INVALID_LOCK;
            else if (error == 0)
                id = lock->id;
            if (error == 0 && lock != NULL
                && packet->dp_Type != ACTION_EXAMINE_FH
                && packet->dp_Type != ACTION_EXAMINE_FH64)
            {
                /* Examine and ExNext move the lock's only directory cursor,
                 * so an ExAll sequence on the same lock ends here and its
                 * continuation is refused instead of skipping entries. */
                lock->exall_pending = 0;
                lock->exall_key = 0;
            }

            if (error == 0 && (packet->dp_Type == ACTION_EXAMINE_OBJECT
                    || packet->dp_Type == ACTION_EXAMINE_OBJECT64))
                error = afsplus_aros_rewind_directory(context->filesystem,
                    id);
            if (error == 0 && (packet->dp_Type == ACTION_EXAMINE_NEXT
                    || packet->dp_Type == ACTION_EXAMINE_NEXT64))
            {
                error = afsplus_aros_examine_next(context->filesystem, id,
                    &info, name, sizeof(name));
                /* The entry is a child of the examined directory. */
                if (error == 0 && comments)
                    error = afsplus_aros_comment(context->filesystem, id,
                        name, info.name_length, comment, sizeof(comment),
                        &comment_length);
            }
            else if (error == 0)
            {
                error = afsplus_aros_examine_lock(context->filesystem, id,
                    &info, name, sizeof(name));
                if (error == 0 && comments)
                    error = afsplus_aros_comment(context->filesystem, id,
                        NULL, 0, comment, sizeof(comment), &comment_length);
            }
        }
        if (error == 0)
            error = fill_packet_fib(packet->dp_Type,
                (BPTR)packet->dp_Arg2, &info, name, comment,
                comment_length);
        if (temporary_root != 0)
            (void)afsplus_aros_free_lock(context->filesystem,
                temporary_root);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_EXAMINE_ALL:
    {
        struct AfsplusArosNativeLock *lock = find_lock(context,
            (BPTR)packet->dp_Arg1);
        uint8_t *cursor = (uint8_t *)packet->dp_Arg2;
        LONG type = (LONG)packet->dp_Arg4;
        struct ExAllControl *control = (struct ExAllControl *)packet->dp_Arg5;
        struct ExAllData *last = NULL;
        uint8_t *end;
        uint32_t finished = 0;

        /* Every return, errors included, reports its own entry count. */
        if (control != NULL)
            control->eac_Entries = 0;
        if (lock == NULL)
            error = ERROR_INVALID_LOCK;
        else if (cursor == NULL || control == NULL || packet->dp_Arg3 <= 0)
            error = ERROR_REQUIRED_ARG_MISSING;
        else if (type < ED_NAME || type > ED_OWNER)
            error = ERROR_BAD_NUMBER;
        /* Pattern and hook matching need dos.library, which this layer never
         * calls; dos.library then emulates ExAll through ExNext. */
        else if (control->eac_MatchString != NULL
            || control->eac_MatchFunc != NULL)
            error = ERROR_ACTION_NOT_KNOWN;
        if (error == 0 && lock->exall == NULL)
        {
            lock->exall = context->allocate(context->callback_context,
                sizeof(*lock->exall));
            if (lock->exall == NULL)
                error = ERROR_NO_FREE_STORE;
            lock->exall_pending = 0;
        }
        /* A lock has one directory cursor. A zero key starts a sequence,
         * which takes the cursor over; a continuation whose key no longer
         * owns the cursor is refused, never served from the wrong place. */
        if (error == 0 && control->eac_LastKey == 0)
        {
            lock->exall_pending = 0;
            error = afsplus_aros_rewind_directory(context->filesystem,
                lock->id);
            if (error == 0)
            {
                if (++context->exall_serial == 0)
                    context->exall_serial = 1;
                lock->exall_key = context->exall_serial;
                control->eac_LastKey = lock->exall_key;
            }
        }
        else if (error == 0 && (lock->exall_key == 0
            || control->eac_LastKey != (IPTR)lock->exall_key))
            error = ERROR_OBJECT_IN_USE;
        if (error != 0)
            break;

        end = cursor + (size_t)packet->dp_Arg3;
        for (;;)
        {
            if (!lock->exall_pending)
            {
                memset(lock->exall, 0, sizeof(*lock->exall));
                error = afsplus_aros_examine_next(context->filesystem,
                    lock->id, &lock->exall->info, lock->exall->name,
                    sizeof(lock->exall->name));
                if (error == ERROR_NO_MORE_ENTRIES)
                    finished = 1;
                if (error == 0 && type >= ED_COMMENT
                    && require_group(context,
                        AFSPLUS_AROS_GROUP_DOS_COMMENT) == 0)
                    error = afsplus_aros_comment(context->filesystem,
                        lock->id, lock->exall->name,
                        lock->exall->info.name_length, lock->exall->comment,
                        sizeof(lock->exall->comment),
                        &lock->exall->comment_length);
                if (error != 0)
                    break;
                lock->exall_pending = 1;
            }
            if (!exall_append(&cursor, end, type, lock->exall, &last))
            {
                /* Kept for the next call. A buffer too small for one entry
                 * is the caller's error. */
                if (control->eac_Entries == 0)
                    error = ERROR_BUFFER_OVERFLOW;
                break;
            }
            lock->exall_pending = 0;
            control->eac_Entries++;
        }
        if (finished)
        {
            error = ERROR_NO_MORE_ENTRIES;
            lock->exall_key = 0;
        }
        /* A read that fails after entries were packed must not discard
         * them: they are returned, and the next call meets the failure with
         * an empty buffer. */
        else if (error != 0 && error != ERROR_BUFFER_OVERFLOW
            && control->eac_Entries != 0)
            error = 0;
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_EXAMINE_ALL_END:
    {
        struct AfsplusArosNativeLock *lock = find_lock(context,
            (BPTR)packet->dp_Arg1);

        if (lock == NULL)
            error = ERROR_INVALID_LOCK;
        else
        {
            lock->exall_pending = 0;
            lock->exall_key = 0;
            error = afsplus_aros_rewind_directory(context->filesystem,
                lock->id);
        }
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_INFO:
    case ACTION_INFO64:
    case ACTION_DISK_INFO:
    {
        struct AfsplusArosDiskInfo info;
        BPTR destination = (BPTR)(packet->dp_Type == ACTION_DISK_INFO
            ? packet->dp_Arg1 : packet->dp_Arg2);

        error = afsplus_aros_disk_info(context->filesystem, &info);
        if (error == 0)
            error = fill_packet_info(context, packet->dp_Type, destination,
                &info);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_FLUSH:
        error = afsplus_aros_flush(context->filesystem);
        if (error == 0)
            result = DOSTRUE;
        break;
    case ACTION_INHIBIT:
        if (packet->dp_Arg1 == DOSTRUE)
        {
            if (context->locks != NULL || context->files != NULL)
                error = ERROR_OBJECT_IN_USE;
            else if (!context->inhibited)
            {
                error = afsplus_aros_flush(context->filesystem);
                if (error == 0)
                    context->inhibited = 1;
            }
        }
        else
            context->inhibited = 0;
        if (error == 0)
            result = DOSTRUE;
        break;
    case ACTION_CURRENT_VOLUME:
        result = (SIPTR)context->volume_node;
        break;
    case ACTION_DISK_TYPE:
    {
        struct AfsplusArosDiskInfo info;
        error = afsplus_aros_disk_info(context->filesystem, &info);
        if (error == 0)
            result = (SIPTR)info.disk_type;
        break;
    }
    case ACTION_IS_FILESYSTEM:
        result = DOSTRUE;
        break;
    case ACTION_DIE:
        /* A registered NotifyRequest points at the handler port through
         * nr_Handler; EndNotify sends ACTION_REMOVE_NOTIFY there. The port
         * must outlive every registration. */
        if (context->locks != NULL || context->files != NULL
            || context->notifies != NULL)
            error = ERROR_OBJECT_IN_USE;
        else
        {
            error = afsplus_aros_flush(context->filesystem);
            if (error == 0)
            {
                context->quit = 1;
                result = DOSTRUE;
            }
        }
        break;
#if (__WORDSIZE == 64)
    /* dos64.library sends these only where dp_Arg# is 64 bits wide; a 32-bit
     * build delegates LockRecord64 to the classic packets instead. */
    case ACTION_LOCK_RECORD64:
    case ACTION_FREE_RECORD64:
#endif
    case ACTION_LOCK_RECORD:
    case ACTION_FREE_RECORD:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);
        uint32_t wide = packet->dp_Type != ACTION_LOCK_RECORD
            && packet->dp_Type != ACTION_FREE_RECORD;
        uint32_t freeing = packet->dp_Type == ACTION_FREE_RECORD
            || packet->dp_Type == ACTION_FREE_RECORD64;
        /* Classic arguments are unsigned 32-bit values in a signed slot. */
        uint64_t offset = wide ? (uint64_t)packet->dp_Arg2
            : (uint64_t)(ULONG)packet->dp_Arg2;
        uint64_t length = wide ? (uint64_t)packet->dp_Arg3
            : (uint64_t)(ULONG)packet->dp_Arg3;
        LONG mode = (LONG)packet->dp_Arg4;

        error = require_group(context, AFSPLUS_AROS_GROUP_DOS_RECORDS);
        if (error == 0 && file == NULL)
            error = ERROR_INVALID_LOCK;
        if (error == 0 && freeing)
            error = afsplus_aros_free_record(context->filesystem, file->id,
                offset, length);
        else if (error == 0 && (mode < REC_EXCLUSIVE
            || mode > REC_SHARED_IMMED))
            error = ERROR_BAD_NUMBER;
        else if (error == 0)
        {
            error = afsplus_aros_lock_record(context->filesystem, file->id,
                offset, length,
                mode == REC_EXCLUSIVE || mode == REC_EXCLUSIVE_IMMED);
            /* This layer never waits. A waiting mode whose range is taken
             * reports its wait as expired; dp_Arg5 ticks are not honoured. */
            if (error == ERROR_LOCK_COLLISION
                && (mode == REC_EXCLUSIVE || mode == REC_SHARED))
                error = ERROR_LOCK_TIMEOUT;
        }
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_RENAME_DISK:
    {
        const uint8_t *name;
        uint32_t name_length = 0;
        int64_t seconds;
        uint32_t nanoseconds;

        if (context->relabel == NULL)
            error = ERROR_ACTION_NOT_KNOWN;
        if (error == 0)
            error = require_group(context, AFSPLUS_AROS_GROUP_VOLUME_LABEL);
        if (error == 0)
            error = bstr_view(packet->dp_Arg1, &name, &name_length);
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        /* Neither side changes alone: the handler first secures what it
         * needs to rename the DOS node, then the volume takes the label, then
         * the node follows or the preparation is undone. */
        if (error == 0)
            error = context->relabel(context->callback_context,
                AFSPLUS_AROS_RELABEL_PREPARE, name, name_length);
        if (error == 0)
        {
            error = afsplus_aros_set_volume_label(context->filesystem, name,
                name_length, seconds, nanoseconds);
            (void)context->relabel(context->callback_context,
                error == 0 ? AFSPLUS_AROS_RELABEL_COMMIT
                    : AFSPLUS_AROS_RELABEL_ABORT, name, name_length);
        }
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_WRITE_PROTECT:
        /* Added to the DOS_HANDLES group by interface revision 9. */
        error = require_group(context, AFSPLUS_AROS_GROUP_DOS_HANDLES);
        if (error == 0 && context->revision < 9)
            error = ERROR_ACTION_NOT_KNOWN;
        if (error == 0)
            error = afsplus_aros_set_write_protect(context->filesystem,
                packet->dp_Arg1 != DOSFALSE, (uint32_t)packet->dp_Arg2);
        if (error == 0)
            result = DOSTRUE;
        break;
    case ACTION_FH_FROM_LOCK:
    {
        struct FileHandle *public_file = packet->dp_Arg1 != 0
            ? (struct FileHandle *)BADDR((BPTR)packet->dp_Arg1) : NULL;
        struct AfsplusArosNativeLock *lock = find_lock(context,
            (BPTR)packet->dp_Arg2);
        struct AfsplusArosNativeFile *file = NULL;
        struct AfsplusArosDiskInfo disk;
        uint64_t id = 0;

        error = require_group(context, AFSPLUS_AROS_GROUP_DOS_HANDLES);
        if (error == 0 && (public_file == NULL || lock == NULL))
            error = ERROR_INVALID_LOCK;
        if (error == 0)
            error = afsplus_aros_disk_info(context->filesystem, &disk);
        if (error == 0)
            file = reserve_file(context, !disk.write_protected, &error);
        if (error == 0)
            error = afsplus_aros_open_from_lock(context->filesystem,
                lock->id, &id);
        if (error != 0)
        {
            /* The lock is untouched and stays the caller's. */
            if (file != NULL)
                discard_reserved_file(context, file);
            break;
        }
        /* The filesystem consumed the lock: only its wrapper is left. */
        unlink_lock(context, lock);
        publish_file(context, file, id);
        public_file->fh_Arg1 = (SIPTR)MKBADDR(file);
        public_file->fh_Port = DOSFALSE;
        result = DOSTRUE;
        break;
    }
    case ACTION_CHANGE_MODE:
    {
        uint32_t access = 0;

        error = require_group(context, AFSPLUS_AROS_GROUP_DOS_HANDLES);
        if (error == 0)
            switch ((LONG)packet->dp_Arg3)
            {
            case SHARED_LOCK:
            case MODE_OLDFILE:
            case MODE_READWRITE:
                access = AFSPLUS_AROS_LOCK_SHARED;
                break;
            case EXCLUSIVE_LOCK:
            case MODE_NEWFILE:
                access = AFSPLUS_AROS_LOCK_EXCLUSIVE;
                break;
            default:
                error = ERROR_BAD_NUMBER;
                break;
            }
        if (error == 0 && (LONG)packet->dp_Arg1 == CHANGE_LOCK)
        {
            struct AfsplusArosNativeLock *lock = find_lock(context,
                (BPTR)packet->dp_Arg2);

            if (lock == NULL)
                error = ERROR_INVALID_LOCK;
            else
                error = afsplus_aros_change_lock_mode(context->filesystem,
                    lock->id, access);
            if (error == 0)
                lock->public_lock.fl_Access
                    = access == AFSPLUS_AROS_LOCK_EXCLUSIVE
                        ? EXCLUSIVE_LOCK : SHARED_LOCK;
        }
        else if (error == 0 && (LONG)packet->dp_Arg1 == CHANGE_FH)
        {
            struct FileHandle *public_file = packet->dp_Arg2 != 0
                ? (struct FileHandle *)BADDR((BPTR)packet->dp_Arg2) : NULL;
            struct AfsplusArosNativeFile *file = public_file != NULL
                ? find_file(context, (BPTR)public_file->fh_Arg1) : NULL;

            if (file == NULL)
                error = ERROR_INVALID_LOCK;
            else
                error = afsplus_aros_change_file_mode(context->filesystem,
                    file->id, access);
        }
        else if (error == 0)
            error = ERROR_BAD_NUMBER;
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_ADD_NOTIFY:
    {
        struct NotifyRequest *request
            = (struct NotifyRequest *)packet->dp_Arg1;
        struct AfsplusArosNativeNotify *node = NULL;
        struct AfsplusResolvedParent object;
        uint32_t object_ready = 0;
        uint32_t length = 0;

        if (context->notify == NULL)
            error = ERROR_ACTION_NOT_KNOWN;
        if (error == 0)
            error = require_group(context, AFSPLUS_AROS_GROUP_NOTIFY);
        if (error == 0 && (request == NULL || request->nr_FullName == NULL))
            error = ERROR_REQUIRED_ARG_MISSING;
        if (error == 0)
        {
            struct AfsplusArosNativeNotify *known;

            for (known = context->notifies; known != NULL;
                known = known->next)
                if (known->request == request)
                {
                    error = ERROR_OBJECT_IN_USE;
                    break;
                }
        }
        if (error == 0)
        {
            node = context->allocate(context->callback_context,
                sizeof(*node));
            if (node == NULL)
                error = ERROR_NO_FREE_STORE;
        }
        if (error == 0)
        {
            length = c_string_length((const uint8_t *)request->nr_FullName);
            error = resolve_named_object(context, 0,
                (const uint8_t *)request->nr_FullName, length, &object);
            object_ready = error == 0;
        }
        if (error == 0)
            error = afsplus_aros_watch_add(context->filesystem, object.id,
                object.leaf, object.leaf_length, &node->watch);
        if (object_ready)
            release_temporary_lock(context, object.id, object.owned);
        if (error != 0)
        {
            if (node != NULL)
                context->free(context->callback_context, node,
                    sizeof(*node));
            break;
        }
        node->request = request;
        node->next = context->notifies;
        context->notifies = node;
        /* nr_MsgCount belongs to the requester and the delivery side. */
        request->nr_Handler = context->handler_port;
        result = DOSTRUE;
        if ((request->nr_Flags & NRF_NOTIFY_INITIAL) != 0)
        {
            uint64_t existing = 0;

            /* Initial notification only for an object that exists. */
            if (resolve_path_lock(context, 0,
                    (const uint8_t *)request->nr_FullName, length,
                    AFSPLUS_AROS_LOCK_SHARED, &existing) == 0)
            {
                release_temporary_lock(context, existing, existing != 0);
                initial_notify = request;
            }
        }
        break;
    }
    case ACTION_REMOVE_NOTIFY:
    {
        struct NotifyRequest *request
            = (struct NotifyRequest *)packet->dp_Arg1;
        struct AfsplusArosNativeNotify **link;

        error = ERROR_OBJECT_NOT_FOUND;
        for (link = &context->notifies; *link != NULL; link = &(*link)->next)
            if ((*link)->request == request)
            {
                struct AfsplusArosNativeNotify *node = *link;

                *link = node->next;
                error = afsplus_aros_watch_remove(context->filesystem,
                    node->watch);
                context->free(context->callback_context, node,
                    sizeof(*node));
                break;
            }
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    default:
        error = ERROR_ACTION_NOT_KNOWN;
        break;
    }

    store_packet_result(packet, result, result64, error, packet64);
    if (initial_notify != NULL)
        context->notify(context->callback_context, initial_notify);
    deliver_notifications(context);
    return 0;
}
