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
#include <dos/exall.h>
#include <dos/notify.h>
#include <dos/record.h>
#include <aros/stdc/string.h>

#define AFSPLUS_NATIVE_LOCK_MAGIC UINT32_C(0x41464c4b)
#define AFSPLUS_NATIVE_FILE_MAGIC UINT32_C(0x41464648)
#define AFSPLUS_UNIX_TO_AMIGA_EPOCH INT64_C(252460800)
#define AFSPLUS_SECONDS_PER_DAY INT64_C(86400)
#define AFSPLUS_SECONDS_PER_MINUTE INT64_C(60)
#define AFSPLUS_TICKS_PER_SECOND UINT32_C(50)

struct AfsplusArosNativeLock {
    struct FileLock public_lock;
    uint32_t magic;
    struct AfsplusArosPacketContext *owner;
    uint64_t id;
    struct AfsplusArosNativeLock *next;
    /* One directory entry fetched by ACTION_EXAMINE_ALL that did not fit
     * its buffer; the next enumeration step consumes it first. */
    uint32_t pending;
    struct AfsplusArosFileInfo pending_info;
    uint8_t pending_name[MAXFILENAMELENGTH];
};

/* One StartNotify() request. The parent directory lock and, once the
 * object exists, the object lock stay held so that changes compare by
 * identity instead of by path text. */
struct AfsplusNotifyNode {
    struct NotifyRequest *request;
    uint64_t parent_lock;
    uint32_t parent_owned;
    uint64_t object_lock;
    uint32_t leaf_length;
    uint8_t leaf[MAXFILENAMELENGTH];
    struct AfsplusNotifyNode *next;
};

/* One LockRecord() range, keyed by object so that handles on the same file
 * see each other. */
struct AfsplusRecordNode {
    struct AfsplusArosNativeFile *file;
    uint64_t object_id;
    uint64_t offset;
    uint64_t length;
    uint32_t exclusive;
    struct AfsplusRecordNode *next;
};

struct AfsplusArosNativeFile {
    uint32_t magic;
    uint32_t writable;
    struct AfsplusArosPacketContext *owner;
    uint64_t id;
    struct AfsplusArosNativeFile *next;
};

struct AfsplusArosPacketContext {
    struct AfsplusAros *filesystem;
    struct MsgPort *handler_port;
    BPTR volume_node;
    void *callback_context;
    AfsplusArosPacketAllocate allocate;
    AfsplusArosPacketFree free;
    AfsplusArosPacketNow now;
    AfsplusArosPacketNotify notify;
    struct AfsplusArosNativeLock *locks;
    struct AfsplusArosNativeFile *files;
    struct AfsplusNotifyNode *notifies;
    struct AfsplusRecordNode *records;
    uint32_t inhibited;
    uint32_t quit;
    uint32_t write_protected;
    IPTR write_protect_key;
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

/* Inverse of unix_to_datestamp. A DateStamp cannot express a time before
 * the Amiga epoch, so the result is never earlier than it. */
static void datestamp_to_unix(const struct DateStamp *date, int64_t *seconds,
    uint32_t *nanoseconds)
{
    int64_t days = date->ds_Days < 0 ? 0 : date->ds_Days;
    int64_t minutes = date->ds_Minute < 0 ? 0 : date->ds_Minute;
    int64_t ticks = date->ds_Tick < 0 ? 0 : date->ds_Tick;

    *seconds = AFSPLUS_UNIX_TO_AMIGA_EPOCH + days * AFSPLUS_SECONDS_PER_DAY
        + minutes * AFSPLUS_SECONDS_PER_MINUTE
        + ticks / AFSPLUS_TICKS_PER_SECOND;
    *nanoseconds = (uint32_t)(ticks % AFSPLUS_TICKS_PER_SECOND)
        * UINT32_C(20000000);
}

/* Resolves the (lock, name) pair of ACTION_SET_PROTECT and ACTION_SET_DATE.
 * An empty name designates the lock's own object and resolves no parent. */
static int32_t resolve_attribute_target(
    struct AfsplusArosPacketContext *context, BPTR lock, SIPTR name,
    struct AfsplusResolvedParent *target, uint32_t *ready)
{
    const uint8_t *path;
    uint32_t path_length = 0;
    uint64_t base;
    int32_t error;

    *ready = 0;
    error = lock_id(context, lock, &base);
    if (error == 0)
        error = bstr_view(name, &path, &path_length);
    if (error != 0)
        return error;
    if (path_length == 0)
    {
        target->id = base;
        target->owned = 0;
        target->leaf = path;
        target->leaf_length = 0;
        return 0;
    }
    error = resolve_parent(context, base, path, path_length, target);
    *ready = error == 0;
    return error;
}

/* ---- write protection ------------------------------------------------ */

static uint32_t action_mutates(LONG action)
{
    switch (action)
    {
    case ACTION_FINDOUTPUT:
    case ACTION_FINDUPDATE:
    case ACTION_WRITE:
    case ACTION_SET_FILE_SIZE:
    case ACTION_SET_FILE_SIZE64:
    case ACTION_CHANGE_FILE_SIZE64:
    case ACTION_DELETE_OBJECT:
    case ACTION_RENAME_OBJECT:
    case ACTION_CREATE_DIR:
    case ACTION_MAKE_LINK:
    case ACTION_SET_PROTECT:
    case ACTION_SET_DATE:
    case ACTION_SET_COMMENT:
    case ACTION_FORMAT:
    case ACTION_RENAME_DISK:
        return 1;
    default:
        return 0;
    }
}

/* ---- notification ---------------------------------------------------- */

static void drop_notify(struct AfsplusArosPacketContext *context,
    struct AfsplusNotifyNode *node)
{
    struct AfsplusNotifyNode **link;

    for (link = &context->notifies; *link != NULL; link = &(*link)->next)
        if (*link == node)
        {
            *link = node->next;
            break;
        }
    if (node->object_lock != 0)
        (void)afsplus_aros_free_lock(context->filesystem, node->object_lock);
    release_temporary_lock(context, node->parent_lock, node->parent_owned);
    context->free(context->callback_context, node, sizeof(*node));
}

static uint32_t leaf_equal_nocase(const uint8_t *a, uint32_t a_length,
    const uint8_t *b, uint32_t b_length)
{
    uint32_t i;

    if (a_length != b_length)
        return 0;
    for (i = 0; i < a_length; i++)
    {
        uint8_t x = a[i];
        uint8_t y = b[i];
        if (x >= 'A' && x <= 'Z')
            x = (uint8_t)(x - 'A' + 'a');
        if (y >= 'A' && y <= 'Z')
            y = (uint8_t)(y - 'A' + 'a');
        if (x != y)
            return 0;
    }
    return 1;
}

static uint32_t locks_same(struct AfsplusArosPacketContext *context,
    uint64_t first, uint64_t second)
{
    uint32_t same = 0;

    if (afsplus_aros_same_lock(context->filesystem, first, second, &same) != 0)
        return 0;
    return same;
}

static void fire_notify(struct AfsplusArosPacketContext *context,
    struct AfsplusNotifyNode *node)
{
    if (context->notify != NULL)
        (void)context->notify(context->callback_context, node->request);
}

/* A watched object may not exist when the request is posted; the object
 * lock is taken as soon as a change under its parent names it. */
static void refresh_notify_object(struct AfsplusArosPacketContext *context,
    struct AfsplusNotifyNode *node)
{
    uint64_t lock = 0;

    if (node->object_lock != 0)
        return;
    if (afsplus_aros_locate(context->filesystem, node->parent_lock,
            node->leaf, node->leaf_length, AFSPLUS_AROS_LOCK_SHARED,
            &lock) == 0)
        node->object_lock = lock;
}

/* The entry `leaf` under the directory `parent` was created, removed or
 * changed: notify watchers of that entry and watchers of the directory. */
static void notify_entry_changed(struct AfsplusArosPacketContext *context,
    uint64_t parent, const uint8_t *leaf, uint32_t leaf_length)
{
    struct AfsplusNotifyNode *node;

    for (node = context->notifies; node != NULL; node = node->next)
    {
        if (locks_same(context, parent, node->parent_lock)
            && leaf_equal_nocase(leaf, leaf_length, node->leaf,
                node->leaf_length))
        {
            refresh_notify_object(context, node);
            fire_notify(context, node);
        }
        else if (node->object_lock != 0
            && locks_same(context, parent, node->object_lock))
            fire_notify(context, node);
    }
}

/* The object behind `lock` changed in place (data or attributes): notify
 * by (parent directory, name), which also covers a watcher posted before
 * the object existed and watchers of the directory itself. */
static void notify_object_changed(struct AfsplusArosPacketContext *context,
    uint64_t lock)
{
    struct AfsplusArosFileInfo info;
    uint8_t name[MAXFILENAMELENGTH];
    uint64_t parent = 0;

    if (context->notifies == NULL || lock == 0)
        return;
    if (afsplus_aros_examine_lock(context->filesystem, lock, &info, name,
            sizeof(name)) != 0)
        return;
    if (afsplus_aros_parent_lock(context->filesystem, lock, &parent) != 0)
        return;
    notify_entry_changed(context, parent, name, info.name_length);
    if (parent != 0)
        (void)afsplus_aros_free_lock(context->filesystem, parent);
}

static void notify_file_changed(struct AfsplusArosPacketContext *context,
    uint64_t file)
{
    uint64_t lock = 0;

    if (context->notifies == NULL)
        return;
    if (afsplus_aros_lock_from_file(context->filesystem, file, &lock) != 0)
        return;
    notify_object_changed(context, lock);
    (void)afsplus_aros_free_lock(context->filesystem, lock);
}

static int32_t add_notify(struct AfsplusArosPacketContext *context,
    struct NotifyRequest *request)
{
    struct AfsplusNotifyNode *node;
    struct AfsplusResolvedParent parent;
    const uint8_t *name;
    size_t name_length;
    int32_t error;

    if (context->notify == NULL)
        return ERROR_ACTION_NOT_KNOWN;
    if (request == NULL || request->nr_FullName == NULL)
        return ERROR_BAD_NUMBER;
    if ((request->nr_Flags & (NRF_SEND_MESSAGE | NRF_SEND_SIGNAL)) == 0)
        return ERROR_BAD_NUMBER;
    name = (const uint8_t *)request->nr_FullName;
    name_length = strlen((const char *)request->nr_FullName);
    if (name_length > UINT32_MAX)
        return ERROR_LINE_TOO_LONG;
    error = resolve_parent(context, 0, name, (uint32_t)name_length, &parent);
    if (error != 0)
        return error;
    if (parent.leaf_length >= sizeof(node->leaf))
    {
        release_temporary_lock(context, parent.id, parent.owned);
        return ERROR_OBJECT_TOO_LARGE;
    }
    node = context->allocate(context->callback_context, sizeof(*node));
    if (node == NULL)
    {
        release_temporary_lock(context, parent.id, parent.owned);
        return ERROR_NO_FREE_STORE;
    }
    memset(node, 0, sizeof(*node));
    node->request = request;
    node->parent_lock = parent.id;
    node->parent_owned = parent.owned;
    node->leaf_length = parent.leaf_length;
    memcpy(node->leaf, parent.leaf, parent.leaf_length);
    node->next = context->notifies;
    context->notifies = node;
    request->nr_MsgCount = 0;
    request->nr_Handler = context->handler_port;
    refresh_notify_object(context, node);
    if ((request->nr_Flags & NRF_NOTIFY_INITIAL) && node->object_lock != 0)
        fire_notify(context, node);
    return 0;
}

static int32_t remove_notify(struct AfsplusArosPacketContext *context,
    struct NotifyRequest *request)
{
    struct AfsplusNotifyNode *node;

    for (node = context->notifies; node != NULL; node = node->next)
        if (node->request == request)
        {
            drop_notify(context, node);
            return 0;
        }
    return ERROR_OBJECT_NOT_FOUND;
}

/* ---- record locks ---------------------------------------------------- */

static void drop_record(struct AfsplusArosPacketContext *context,
    struct AfsplusRecordNode *node)
{
    struct AfsplusRecordNode **link;

    for (link = &context->records; *link != NULL; link = &(*link)->next)
        if (*link == node)
        {
            *link = node->next;
            break;
        }
    context->free(context->callback_context, node, sizeof(*node));
}

static void drop_file_records(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeFile *file)
{
    struct AfsplusRecordNode *node = context->records;

    while (node != NULL)
    {
        struct AfsplusRecordNode *next = node->next;
        if (node->file == file)
            drop_record(context, node);
        node = next;
    }
}

static uint32_t ranges_overlap(uint64_t a_offset, uint64_t a_length,
    uint64_t b_offset, uint64_t b_length)
{
    if (a_length == 0 || b_length == 0)
        return 0;
    if (a_offset >= b_offset)
        return a_offset - b_offset < b_length;
    return b_offset - a_offset < a_length;
}

static int32_t lock_record(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeFile *file, uint64_t offset, uint64_t length,
    uint32_t mode, uint64_t timeout)
{
    struct AfsplusArosFileInfo info;
    uint8_t name[MAXFILENAMELENGTH];
    struct AfsplusRecordNode *node;
    uint32_t exclusive = mode == REC_EXCLUSIVE || mode == REC_EXCLUSIVE_IMMED;
    uint32_t immediate = mode == REC_EXCLUSIVE_IMMED
        || mode == REC_SHARED_IMMED;
    int32_t error;

    if (mode > REC_SHARED_IMMED)
        return ERROR_BAD_NUMBER;
    error = afsplus_aros_examine_file(context->filesystem, file->id, &info,
        name, sizeof(name));
    if (error != 0)
        return error;
    for (node = context->records; node != NULL; node = node->next)
    {
        if (node->object_id != info.object_id)
            continue;
        if (!(exclusive || node->exclusive))
            continue;
        if (!ranges_overlap(offset, length, node->offset, node->length))
            continue;
        /* The translator cannot wait on the packet loop: a timed request
         * that collides fails as if the timeout had expired. */
        return immediate || timeout == 0 ? ERROR_LOCK_COLLISION
            : ERROR_LOCK_TIMEOUT;
    }
    node = context->allocate(context->callback_context, sizeof(*node));
    if (node == NULL)
        return ERROR_NO_FREE_STORE;
    memset(node, 0, sizeof(*node));
    node->file = file;
    node->object_id = info.object_id;
    node->offset = offset;
    node->length = length;
    node->exclusive = exclusive;
    node->next = context->records;
    context->records = node;
    return 0;
}

static int32_t free_record(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeFile *file, uint64_t offset, uint64_t length)
{
    struct AfsplusRecordNode *node;

    for (node = context->records; node != NULL; node = node->next)
        if (node->file == file && node->offset == offset
            && node->length == length)
        {
            drop_record(context, node);
            return 0;
        }
    return ERROR_RECORD_NOT_LOCKED;
}

/* ---- ExAll ----------------------------------------------------------- */

/* Byte size of one ExAllData record at a given ED_ level. */
static size_t exall_record_size(LONG level)
{
    switch (level)
    {
    case ED_NAME:
        return offsetof(struct ExAllData, ed_Type);
    case ED_TYPE:
        return offsetof(struct ExAllData, ed_Size);
    case ED_SIZE:
        return offsetof(struct ExAllData, ed_Prot);
    case ED_PROTECTION:
        return offsetof(struct ExAllData, ed_Days);
    case ED_DATE:
        return offsetof(struct ExAllData, ed_Comment);
    case ED_COMMENT:
        return offsetof(struct ExAllData, ed_OwnerUID);
    default:
        return sizeof(struct ExAllData);
    }
}

/* Fetches the next directory entry of `lock`, honouring a pushed-back one. */
static int32_t next_directory_entry(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeLock *lock, struct AfsplusArosFileInfo *info,
    uint8_t *name, size_t capacity)
{
    if (lock->pending)
    {
        *info = lock->pending_info;
        memcpy(name, lock->pending_name, sizeof(lock->pending_name) < capacity
            ? sizeof(lock->pending_name) : capacity);
        lock->pending = 0;
        return 0;
    }
    return afsplus_aros_examine_next(context->filesystem, lock->id, info,
        name, (uint32_t)capacity);
}

static int32_t examine_all(struct AfsplusArosPacketContext *context,
    struct AfsplusArosNativeLock *lock, uint8_t *buffer, size_t size,
    LONG level, struct ExAllControl *control, SIPTR *result)
{
    const size_t align = sizeof(IPTR);
    size_t record_size = exall_record_size(level);
    size_t used = 0;
    struct ExAllData *previous = NULL;
    struct AfsplusArosFileInfo info;
    uint8_t name[MAXFILENAMELENGTH];
    int32_t error;

    if (level < ED_NAME || level > ED_OWNER)
        return ERROR_BAD_NUMBER;
    if (control == NULL || buffer == NULL)
        return ERROR_BAD_NUMBER;
    /* dos.library emulates pattern and hook matching on top of ExNext
     * when the handler declines; it does so once and remembers. */
    if (control->eac_MatchString != NULL || control->eac_MatchFunc != NULL)
        return ERROR_ACTION_NOT_KNOWN;
    control->eac_Entries = 0;
    if (control->eac_LastKey == 0)
    {
        lock->pending = 0;
        error = afsplus_aros_rewind_directory(context->filesystem, lock->id);
        if (error != 0)
            return error;
        control->eac_LastKey = 1;
    }
    *result = DOSFALSE;
    for (;;)
    {
        size_t name_length;
        size_t entry_bytes;
        struct ExAllData *entry;
        uint8_t *text;

        error = next_directory_entry(context, lock, &info, name, sizeof(name));
        if (error == ERROR_NO_MORE_ENTRIES)
        {
            /* Exhausted: DOSFALSE with ERROR_NO_MORE_ENTRIES, entries kept. */
            return control->eac_Entries != 0 ? 0 : ERROR_NO_MORE_ENTRIES;
        }
        if (error != 0)
            return error;
        name_length = info.name_length;
        /* record, name + NUL, empty comment + NUL, rounded to pointer size */
        entry_bytes = record_size + name_length + 1 + 1;
        entry_bytes = (entry_bytes + align - 1) & ~(align - 1);
        if (used + entry_bytes > size)
        {
            /* Does not fit: push it back for the next call. */
            lock->pending = 1;
            lock->pending_info = info;
            memcpy(lock->pending_name, name, sizeof(lock->pending_name));
            if (control->eac_Entries == 0)
                return ERROR_NO_FREE_STORE;
            *result = DOSTRUE;
            return 0;
        }
        entry = (struct ExAllData *)(buffer + used);
        memset(entry, 0, record_size);
        text = buffer + used + record_size;
        memcpy(text, name, name_length);
        text[name_length] = 0;
        text[name_length + 1] = 0;
        entry->ed_Name = text;
        if (level >= ED_TYPE)
            entry->ed_Type = (LONG)info.directory_entry_type;
        if (level >= ED_SIZE)
        {
#if (__DOS64)
            entry->ed_Size = info.size;
#else
            entry->ed_Size = info.size > UINT32_MAX ? UINT32_MAX
                : (ULONG)info.size;
#endif
        }
        if (level >= ED_PROTECTION)
            entry->ed_Prot = info.protection;
        if (level >= ED_DATE)
        {
            struct DateStamp stamp;
            unix_to_datestamp(info.modified_seconds,
                info.modified_nanoseconds, &stamp);
            entry->ed_Days = (ULONG)stamp.ds_Days;
            entry->ed_Mins = (ULONG)stamp.ds_Minute;
            entry->ed_Ticks = (ULONG)stamp.ds_Tick;
        }
        if (level >= ED_COMMENT)
            entry->ed_Comment = text + name_length + 1;
        if (previous != NULL)
            previous->ed_Next = entry;
        previous = entry;
        used += entry_bytes;
        control->eac_Entries++;
    }
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
    const struct AfsplusArosFileInfo *info, const uint8_t *name)
{
    int32_t error;

    memset(fib, 0, sizeof(*fib));
    error = fill_fib_name(fib->fib_FileName, sizeof(fib->fib_FileName),
        name, info->name_length);
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
    const struct AfsplusArosFileInfo *info, const uint8_t *name)
{
    int32_t error;

    memset(fib, 0, sizeof(*fib));
    error = fill_fib_name(fib->fib_FileName, sizeof(fib->fib_FileName),
        name, info->name_length);
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
    const struct AfsplusArosFileInfo *info, const uint8_t *name)
{
    void *destination;

    if (raw == BNULL)
        return ERROR_INVALID_LOCK;
    destination = BADDR(raw);
    if (action == ACTION_EXAMINE_OBJECT64
        || action == ACTION_EXAMINE_NEXT64
        || action == ACTION_EXAMINE_FH64)
        return fill_fib64((struct FileInfoBlock64 *)destination, info, name);
#if (__DOS64)
    return fill_fib64((struct FileInfoBlock64 *)destination, info, name);
#else
    return fill_fib32((struct FileInfoBlock32 *)destination, info, name);
#endif
}

static void fill_info64(struct AfsplusArosPacketContext *context,
    struct InfoData64 *destination, const struct AfsplusArosDiskInfo *info)
{
    memset(destination, 0, sizeof(*destination));
    destination->id_DiskState = info->write_protected
            || context->write_protected
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
            || context->write_protected
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
    *output = context;
    return 0;
}

void afsplus_aros_packet_set_volume_node(
    struct AfsplusArosPacketContext *context, BPTR volume_node)
{
    if (context != NULL)
        context->volume_node = volume_node;
}

int32_t afsplus_aros_packet_destroy(
    struct AfsplusArosPacketContext *context)
{
    int32_t first_error = 0;

    if (context == NULL)
        return ERROR_BAD_NUMBER;
    while (context->notifies != NULL)
        drop_notify(context, context->notifies);
    while (context->records != NULL)
        drop_record(context, context->records);
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
    {
        AfsplusArosPacketFree free_callback = context->free;
        void *callback_context = context->callback_context;
        free_callback(callback_context, context, sizeof(*context));
    }
    return first_error;
}

uint32_t afsplus_aros_packet_should_quit(
    const struct AfsplusArosPacketContext *context)
{
    return context != NULL ? context->quit : 0;
}

int32_t afsplus_aros_packet_process(
    struct AfsplusArosPacketContext *context, struct DosPacket *packet)
{
    SIPTR result = DOSFALSE;
    int64_t result64 = DOSFALSE;
    int32_t error = 0;
    uint32_t packet64 = 0;

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

    if (context->write_protected && action_mutates(packet->dp_Type))
    {
        store_packet_result(packet, DOSFALSE, DOSFALSE,
            ERROR_DISK_WRITE_PROTECTED, packet64);
        return 0;
    }

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
            drop_file_records(context, file);
            if (file->writable)
            {
                error = afsplus_aros_fsync(context->filesystem, file->id);
                notify_file_changed(context, file->id);
            }
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
        if (error == 0)
            notify_entry_changed(context, parent.id, parent.leaf,
                parent.leaf_length);
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
        if (error == 0)
            notify_entry_changed(context, parent.id, parent.leaf,
                parent.leaf_length);
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
        if (error == 0)
        {
            notify_entry_changed(context, source.id, source.leaf,
                source.leaf_length);
            notify_entry_changed(context, target.id, target.leaf,
                target.leaf_length);
        }
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

        error = lock_id(context, (BPTR)packet->dp_Arg1, &base);
        source = find_lock(context, (BPTR)packet->dp_Arg3);
        if (error == 0 && source == NULL)
            error = ERROR_INVALID_LOCK;
        if (error == 0 && (LONG)packet->dp_Arg4 != LINK_HARD)
            error = ERROR_ACTION_NOT_KNOWN;
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
            error = afsplus_aros_make_hard_link(context->filesystem,
                parent.id, parent.leaf, parent.leaf_length, source->id,
                seconds, nanoseconds);
        if (error == 0)
            notify_entry_changed(context, parent.id, parent.leaf,
                parent.leaf_length);
        if (parent_ready)
            release_temporary_lock(context, parent.id, parent.owned);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_SET_PROTECT:
    {
        struct AfsplusResolvedParent target;
        uint32_t target_ready;
        int64_t seconds;
        uint32_t nanoseconds;

        error = resolve_attribute_target(context, (BPTR)packet->dp_Arg2,
            packet->dp_Arg3, &target, &target_ready);
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            error = afsplus_aros_set_protection(context->filesystem,
                target.id, target.leaf, target.leaf_length,
                (uint32_t)packet->dp_Arg4, seconds, nanoseconds);
        if (error == 0)
        {
            if (target.leaf_length != 0)
                notify_entry_changed(context, target.id, target.leaf,
                    target.leaf_length);
            else
                notify_object_changed(context, target.id);
        }
        if (target_ready)
            release_temporary_lock(context, target.id, target.owned);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_SET_DATE:
    {
        struct AfsplusResolvedParent target;
        uint32_t target_ready;
        const struct DateStamp *date = (const struct DateStamp *)packet->dp_Arg4;
        int64_t modified_seconds;
        uint32_t modified_nanoseconds;
        int64_t seconds;
        uint32_t nanoseconds;

        if (date == NULL)
            error = ERROR_BAD_NUMBER;
        else
            error = resolve_attribute_target(context, (BPTR)packet->dp_Arg2,
                packet->dp_Arg3, &target, &target_ready);
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
        {
            datestamp_to_unix(date, &modified_seconds, &modified_nanoseconds);
            error = afsplus_aros_set_date(context->filesystem, target.id,
                target.leaf, target.leaf_length, modified_seconds,
                modified_nanoseconds, seconds, nanoseconds);
        }
        if (error == 0)
        {
            if (target.leaf_length != 0)
                notify_entry_changed(context, target.id, target.leaf,
                    target.leaf_length);
            else
                notify_object_changed(context, target.id);
        }
        if (date != NULL && target_ready)
            release_temporary_lock(context, target.id, target.owned);
        if (error == 0)
            result = DOSTRUE;
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
        uint64_t temporary_root = 0;
        uint64_t id = 0;

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

            if (error == 0 && (packet->dp_Type == ACTION_EXAMINE_OBJECT
                    || packet->dp_Type == ACTION_EXAMINE_OBJECT64))
            {
                if (lock != NULL)
                    lock->pending = 0;
                error = afsplus_aros_rewind_directory(context->filesystem,
                    id);
            }
            if (error == 0 && (packet->dp_Type == ACTION_EXAMINE_NEXT
                    || packet->dp_Type == ACTION_EXAMINE_NEXT64))
                error = next_directory_entry(context, lock, &info, name,
                    sizeof(name));
            else if (error == 0)
                error = afsplus_aros_examine_lock(context->filesystem, id,
                    &info, name, sizeof(name));
        }
        if (error == 0)
            error = fill_packet_fib(packet->dp_Type,
                (BPTR)packet->dp_Arg2, &info, name);
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

        if (lock == NULL)
            error = ERROR_INVALID_LOCK;
        else
            error = examine_all(context, lock, (uint8_t *)packet->dp_Arg2,
                (size_t)packet->dp_Arg3, (LONG)packet->dp_Arg4,
                (struct ExAllControl *)packet->dp_Arg5, &result);
        break;
    }
    case ACTION_EXAMINE_ALL_END:
    {
        struct AfsplusArosNativeLock *lock = find_lock(context,
            (BPTR)packet->dp_Arg1);
        struct ExAllControl *control = (struct ExAllControl *)packet->dp_Arg5;

        if (lock == NULL)
            error = ERROR_INVALID_LOCK;
        else
        {
            lock->pending = 0;
            if (control != NULL)
                control->eac_LastKey = 0;
            error = afsplus_aros_rewind_directory(context->filesystem,
                lock->id);
        }
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_ADD_NOTIFY:
        error = add_notify(context, (struct NotifyRequest *)packet->dp_Arg1);
        if (error == 0)
            result = DOSTRUE;
        break;
    case ACTION_REMOVE_NOTIFY:
        error = remove_notify(context,
            (struct NotifyRequest *)packet->dp_Arg1);
        if (error == 0)
            result = DOSTRUE;
        break;
    case ACTION_LOCK_RECORD:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);

        if (file == NULL)
            error = ERROR_INVALID_LOCK;
        else
            error = lock_record(context, file, (uint64_t)packet->dp_Arg2,
                (uint64_t)packet->dp_Arg3, (uint32_t)packet->dp_Arg4,
                (uint64_t)packet->dp_Arg5);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_FREE_RECORD:
    {
        struct AfsplusArosNativeFile *file = find_file(context,
            (BPTR)packet->dp_Arg1);

        if (file == NULL)
            error = ERROR_INVALID_LOCK;
        else
            error = free_record(context, file, (uint64_t)packet->dp_Arg2,
                (uint64_t)packet->dp_Arg3);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_FH_FROM_LOCK:
    {
        struct FileHandle *public_file = packet->dp_Arg1 != 0
            ? (struct FileHandle *)BADDR((BPTR)packet->dp_Arg1) : NULL;
        struct AfsplusArosNativeLock *lock = find_lock(context,
            (BPTR)packet->dp_Arg2);
        struct AfsplusArosNativeFile *file = NULL;
        uint32_t writable;
        uint64_t id = 0;
        int64_t seconds = 0;
        uint32_t nanoseconds = 0;

        if (public_file == NULL || lock == NULL)
            error = ERROR_INVALID_LOCK;
        writable = lock != NULL && lock->public_lock.fl_Access == EXCLUSIVE_LOCK;
        if (error == 0 && writable && context->write_protected)
            error = ERROR_DISK_WRITE_PROTECTED;
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            file = reserve_file(context, writable, &error);
        if (error == 0)
            error = afsplus_aros_open_from_lock(context->filesystem,
                lock->id, seconds, nanoseconds, &id);
        if (error == 0)
        {
            /* The Rust lock is consumed; only the wrapper remains. */
            unlink_lock(context, lock);
            publish_file(context, file, id);
            public_file->fh_Arg1 = (SIPTR)MKBADDR(file);
            public_file->fh_Port = DOSFALSE;
            result = DOSTRUE;
        }
        else if (file != NULL)
            discard_reserved_file(context, file);
        break;
    }
    case ACTION_CHANGE_MODE:
    {
        if ((LONG)packet->dp_Arg1 == CHANGE_LOCK)
        {
            struct AfsplusArosNativeLock *lock = find_lock(context,
                (BPTR)packet->dp_Arg2);
            LONG native_access = (LONG)packet->dp_Arg3;
            uint32_t access = ffi_access(native_access, &error);

            if (error == 0 && lock == NULL)
                error = ERROR_INVALID_LOCK;
            if (error == 0)
                error = afsplus_aros_change_lock_access(context->filesystem,
                    lock->id, access);
            if (error == 0)
                lock->public_lock.fl_Access = native_access;
        }
        else if ((LONG)packet->dp_Arg1 == CHANGE_FH)
        {
            struct AfsplusArosNativeFile *file = find_file(context,
                (BPTR)packet->dp_Arg2);
            uint32_t writable = (LONG)packet->dp_Arg3 != MODE_OLDFILE;

            if (file == NULL)
                error = ERROR_INVALID_LOCK;
            else if (writable && context->write_protected)
                error = ERROR_DISK_WRITE_PROTECTED;
            else
                error = afsplus_aros_change_file_access(context->filesystem,
                    file->id, writable);
            if (error == 0)
                file->writable = writable;
        }
        else
            error = ERROR_BAD_NUMBER;
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_WRITE_PROTECT:
        if (packet->dp_Arg1 == DOSTRUE)
        {
            if (context->write_protected)
                error = ERROR_DISK_WRITE_PROTECTED;
            else
            {
                context->write_protected = 1;
                context->write_protect_key = (IPTR)packet->dp_Arg2;
            }
        }
        else if (context->write_protected
            && context->write_protect_key != (IPTR)packet->dp_Arg2)
            error = ERROR_INVALID_LOCK;
        else
            context->write_protected = 0;
        if (error == 0)
            result = DOSTRUE;
        break;
    case ACTION_FORMAT:
    {
        const uint8_t *label;
        uint32_t label_length = 0;
        int64_t seconds;
        uint32_t nanoseconds;

        if (!context->inhibited || context->locks != NULL
            || context->files != NULL)
            error = ERROR_OBJECT_IN_USE;
        if (error == 0)
            error = bstr_view(packet->dp_Arg1, &label, &label_length);
        if (error == 0)
            error = packet_now(context, &seconds, &nanoseconds);
        if (error == 0)
            error = afsplus_aros_format(context->filesystem, label,
                label_length, seconds, nanoseconds);
        if (error == 0)
            result = DOSTRUE;
        break;
    }
    case ACTION_RENAME_DISK:
    {
        const uint8_t *label;
        uint32_t label_length = 0;

        error = bstr_view(packet->dp_Arg1, &label, &label_length);
        if (error == 0)
            error = afsplus_aros_set_label(context->filesystem, label,
                label_length);
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
        if (context->locks != NULL || context->files != NULL)
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
    default:
        error = ERROR_ACTION_NOT_KNOWN;
        break;
    }

    store_packet_result(packet, result, result64, error, packet64);
    return 0;
}
