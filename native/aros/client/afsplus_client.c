/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_client.h"

#include <dos/dosextens.h>
#include <proto/dos.h>
#include <proto/exec.h>

#include <string.h>

struct MsgPort *afsplus_client_file_port(BPTR file)
{
    struct FileHandle *handle = BADDR(file);

    return handle != NULL ? handle->fh_Type : NULL;
}

struct MsgPort *afsplus_client_lock_port(BPTR lock)
{
    struct FileLock *native = BADDR(lock);

    return native != NULL ? native->fl_Task : NULL;
}

/* Ports whose handler answered that it does not know the packet. A fallback
 * then costs no failed request each time. The memory is only a hint: a port
 * address reused by a handler that has the transport keeps falling back,
 * which is correct and merely slower, until another port displaces it. */
#define ABSENT_PORTS 4
static struct MsgPort *absent_ports[ABSENT_PORTS];
static uint32_t absent_next;

static uint32_t known_absent(const struct MsgPort *port)
{
    uint32_t index;
    uint32_t found = 0;

    Forbid();
    for (index = 0; index < ABSENT_PORTS; index++)
        if (absent_ports[index] == port)
            found = 1;
    Permit();
    return found;
}

static void remember_absent(struct MsgPort *port)
{
    Forbid();
    absent_ports[absent_next] = port;
    absent_next = (absent_next + 1) % ABSENT_PORTS;
    Permit();
}

void afsplus_client_forget_ports(void)
{
    uint32_t index;

    Forbid();
    for (index = 0; index < ABSENT_PORTS; index++)
        absent_ports[index] = NULL;
    Permit();
}

static void begin(struct AfsplusExtRequest *request, uint32_t operation)
{
    memset(request, 0, sizeof(*request));
    request->operation = operation;
}

LONG afsplus_client_send(struct MsgPort *port,
    struct AfsplusExtRequest *request)
{
    LONG error;

    if (port == NULL || request == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    request->magic = AFSPLUS_EXT_MAGIC;
    request->version = AFSPLUS_EXT_VERSION;
    request->header_size = sizeof(*request);
    if (DoPkt(port, ACTION_AFSPLUS_EXT, (SIPTR)request, 0, 0, 0, 0))
        error = 0;
    else
    {
        error = (LONG)IoErr();
        /* A failure without a reason is still a failure. */
        if (error == 0)
            error = ERROR_UNKNOWN;
    }
    SetIoErr(0);
    return error;
}

LONG afsplus_client_interface(struct MsgPort *port, uint32_t *revision,
    uint32_t *packet_abi, uint64_t *groups)
{
    struct AfsplusExtRequest request;
    LONG error;

    begin(&request, AFSPLUS_EXT_INTERFACE);
    error = afsplus_client_send(port, &request);
    if (error != 0)
        return error;
    if (revision != NULL)
        *revision = request.output_count;
    if (packet_abi != NULL)
        *packet_abi = request.output_flags;
    if (groups != NULL)
        *groups = request.output_value;
    return 0;
}

static uint64_t file_object(BPTR file)
{
    struct FileHandle *handle = BADDR(file);

    return handle != NULL ? (uint64_t)(uintptr_t)handle->fh_Arg1 : 0;
}

/* Seek there, transfer, seek back. A position that could not be restored is
 * the error the caller must hear whatever the transfer did, because the
 * promise of these calls is then broken: ERROR_SEEK_ERROR. */
static LONG classic_at(BPTR file, uint64_t offset, void *buffer,
    uint32_t length, uint32_t writing, uint32_t *count)
{
    LONG previous;
    LONG moved;
    LONG error = 0;

    if (offset > (uint64_t)INT32_MAX || length > (uint32_t)INT32_MAX
        || offset + length > (uint64_t)INT32_MAX)
        return ERROR_OBJECT_TOO_LARGE;
    previous = Seek(file, (LONG)offset, OFFSET_BEGINNING);
    if (previous < 0)
        return (LONG)IoErr() != 0 ? (LONG)IoErr() : ERROR_SEEK_ERROR;
    moved = writing ? Write(file, buffer, (LONG)length)
        : Read(file, buffer, (LONG)length);
    if (moved < 0)
        error = (LONG)IoErr() != 0 ? (LONG)IoErr() : ERROR_UNKNOWN;
    else
        *count = (uint32_t)moved;
    if (Seek(file, previous, OFFSET_BEGINNING) < 0)
        error = ERROR_SEEK_ERROR;
    SetIoErr(0);
    return error;
}

static LONG transfer_at(BPTR file, uint64_t offset, void *buffer,
    uint32_t length, uint32_t writing, uint32_t *count)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port;
    LONG error;

    if (count == NULL || (length != 0 && buffer == NULL))
        return ERROR_REQUIRED_ARG_MISSING;
    *count = 0;
    begin(&request, writing ? AFSPLUS_EXT_WRITE_AT : AFSPLUS_EXT_READ_AT);
    request.object[0] = file_object(file);
    request.offset[0] = offset;
    request.buffer = buffer;
    request.buffer_size = length;
    port = afsplus_client_file_port(file);
    if (port != NULL && known_absent(port))
        return classic_at(file, offset, buffer, length, writing, count);
    error = afsplus_client_send(port, &request);
    /* Only "the packet is unknown" is a reason to fall back. A handler that
     * knows the packet and cannot serve it answers ERROR_NOT_IMPLEMENTED,
     * and a classic transfer would fail the same way after moving the
     * position. */
    if (error == ERROR_ACTION_NOT_KNOWN)
    {
        remember_absent(port);
        return classic_at(file, offset, buffer, length, writing, count);
    }
    if (error == 0)
        *count = request.output_count;
    return error;
}

LONG afsplus_client_read_at(BPTR file, uint64_t offset, void *buffer,
    uint32_t length, uint32_t *count)
{
    return transfer_at(file, offset, buffer, length, 0, count);
}

LONG afsplus_client_write_at(BPTR file, uint64_t offset, const void *buffer,
    uint32_t length, uint32_t *count)
{
    /* The request block carries one untyped buffer for both directions. */
    return transfer_at(file, offset, (void *)(uintptr_t)buffer, length, 1,
        count);
}

LONG afsplus_client_clone_file(BPTR source, BPTR target_directory,
    CONST_STRPTR name)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port = afsplus_client_lock_port(source);

    if (name == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    if (port == NULL)
        return ERROR_INVALID_LOCK;
    /* A zero target lock is the root of the handler's own volume only by
     * that handler's reading; between two handlers it names nothing. */
    if (target_directory != BNULL
        && afsplus_client_lock_port(target_directory) != port)
        return ERROR_RENAME_ACROSS_DEVICES;
    begin(&request, AFSPLUS_EXT_CLONE_FILE);
    request.object[0] = (uint64_t)(uintptr_t)source;
    request.object[1] = (uint64_t)(uintptr_t)target_directory;
    request.name1 = (const uint8_t *)name;
    request.name_length[1] = (uint32_t)strlen((const char *)name);
    return afsplus_client_send(port, &request);
}

LONG afsplus_client_clone_range(BPTR source_file, uint64_t source_offset,
    BPTR target_file, uint64_t target_offset, uint64_t length)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port = afsplus_client_file_port(source_file);

    if (port == NULL)
        return ERROR_INVALID_LOCK;
    if (afsplus_client_file_port(target_file) != port)
        return ERROR_RENAME_ACROSS_DEVICES;
    begin(&request, AFSPLUS_EXT_CLONE_RANGE);
    request.object[0] = file_object(source_file);
    request.object[1] = file_object(target_file);
    request.offset[0] = source_offset;
    request.offset[1] = target_offset;
    request.length = length;
    return afsplus_client_send(port, &request);
}

LONG afsplus_client_preallocate(BPTR file, uint64_t offset, uint64_t length)
{
    struct AfsplusExtRequest request;

    begin(&request, AFSPLUS_EXT_PREALLOCATE);
    request.object[0] = file_object(file);
    request.offset[0] = offset;
    request.length = length;
    return afsplus_client_send(afsplus_client_file_port(file), &request);
}

static LONG report(struct MsgPort *port, uint32_t operation, void *output,
    uint32_t size)
{
    struct AfsplusExtRequest request;

    if (output == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    /* struct_size is the first field of every report struct. */
    memset(output, 0, size);
    memcpy(output, &size, sizeof(size));
    begin(&request, operation);
    request.buffer = output;
    request.buffer_size = size;
    return afsplus_client_send(port, &request);
}

LONG afsplus_client_capabilities(struct MsgPort *port,
    struct AfsplusArosCapabilities *output)
{
    return report(port, AFSPLUS_EXT_CAPABILITIES, output, sizeof(*output));
}

LONG afsplus_client_counters(struct MsgPort *port,
    struct AfsplusArosCounters *output)
{
    return report(port, AFSPLUS_EXT_COUNTERS, output, sizeof(*output));
}

LONG afsplus_client_health(struct MsgPort *port,
    struct AfsplusArosHealth *output)
{
    return report(port, AFSPLUS_EXT_HEALTH, output, sizeof(*output));
}

LONG afsplus_client_info_json(struct MsgPort *port, char *buffer,
    uint32_t capacity, uint32_t *required)
{
    struct AfsplusExtRequest request;
    LONG error;

    if (required == NULL || (capacity != 0 && buffer == NULL))
        return ERROR_REQUIRED_ARG_MISSING;
    *required = 0;
    begin(&request, AFSPLUS_EXT_INFO_JSON);
    request.buffer = buffer;
    request.buffer_size = capacity;
    error = afsplus_client_send(port, &request);
    if (error == 0)
        *required = (uint32_t)request.output_value;
    return error;
}

LONG afsplus_client_packet_counts(struct MsgPort *port, uint32_t which,
    struct AfsplusExtPacketCount *records, uint32_t capacity,
    uint32_t *stored, uint32_t *total)
{
    struct AfsplusExtRequest request;
    LONG error;

    if (stored == NULL || total == NULL || (capacity != 0 && records == NULL)
        || capacity > UINT32_MAX / sizeof(*records))
        return ERROR_REQUIRED_ARG_MISSING;
    *stored = 0;
    *total = 0;
    begin(&request, AFSPLUS_EXT_PACKET_COUNTS);
    request.flags = which;
    request.buffer = records;
    request.buffer_size = capacity * (uint32_t)sizeof(*records);
    error = afsplus_client_send(port, &request);
    if (error == 0)
    {
        *stored = request.output_count;
        *total = (uint32_t)request.output_value;
    }
    return error;
}

static LONG attribute_request(uint32_t operation, BPTR lock,
    CONST_STRPTR attribute, void *buffer, uint32_t size, uint32_t mode,
    uint32_t *required)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port = afsplus_client_lock_port(lock);
    LONG error;

    if (port == NULL)
        return ERROR_INVALID_LOCK;
    if (size != 0 && buffer == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    begin(&request, operation);
    /* The lock's own object: an empty object name. */
    request.object[0] = (uint64_t)(uintptr_t)lock;
    if (attribute != NULL)
    {
        request.name1 = (const uint8_t *)attribute;
        request.name_length[1] = (uint32_t)strlen((const char *)attribute);
    }
    request.buffer = buffer;
    request.buffer_size = size;
    request.flags = mode;
    error = afsplus_client_send(port, &request);
    if (error == 0 && required != NULL)
        *required = (uint32_t)request.output_value;
    return error;
}

LONG afsplus_client_get_attribute(BPTR lock, CONST_STRPTR attribute,
    void *value, uint32_t capacity, uint32_t *required)
{
    if (attribute == NULL || required == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *required = 0;
    return attribute_request(AFSPLUS_EXT_GET_ATTRIBUTE, lock, attribute,
        value, capacity, 0, required);
}

LONG afsplus_client_list_attributes(BPTR lock, char *names,
    uint32_t capacity, uint32_t *required)
{
    if (required == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *required = 0;
    return attribute_request(AFSPLUS_EXT_LIST_ATTRIBUTES, lock, NULL, names,
        capacity, 0, required);
}

LONG afsplus_client_set_attribute(BPTR lock, CONST_STRPTR attribute,
    const void *value, uint32_t length, uint32_t mode)
{
    if (attribute == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    return attribute_request(AFSPLUS_EXT_SET_ATTRIBUTE, lock, attribute,
        (void *)(uintptr_t)value, length, mode, NULL);
}

LONG afsplus_client_dir_open(BPTR lock, uint64_t *walk)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port = afsplus_client_lock_port(lock);
    LONG error;

    if (walk == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *walk = 0;
    if (port == NULL)
        return ERROR_INVALID_LOCK;
    begin(&request, AFSPLUS_EXT_DIR_OPEN);
    request.object[0] = (uint64_t)(uintptr_t)lock;
    error = afsplus_client_send(port, &request);
    if (error == 0)
        *walk = request.output_value;
    return error;
}

LONG afsplus_client_dir_read(BPTR lock, uint64_t walk, void *records,
    uint32_t capacity, uint32_t limit, uint32_t *count, uint32_t *eof)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port = afsplus_client_lock_port(lock);
    LONG error;

    if (count == NULL || eof == NULL || records == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *count = 0;
    *eof = 0;
    if (port == NULL)
        return ERROR_INVALID_LOCK;
    begin(&request, AFSPLUS_EXT_DIR_READ);
    request.offset[0] = walk;
    request.length = limit;
    request.buffer = records;
    request.buffer_size = capacity;
    error = afsplus_client_send(port, &request);
    if (error == 0)
    {
        *count = request.output_count;
        *eof = request.output_flags;
    }
    return error;
}

LONG afsplus_client_dir_close(BPTR lock, uint64_t walk)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port = afsplus_client_lock_port(lock);

    if (port == NULL)
        return ERROR_INVALID_LOCK;
    begin(&request, AFSPLUS_EXT_DIR_CLOSE);
    request.offset[0] = walk;
    return afsplus_client_send(port, &request);
}

LONG afsplus_client_watch_add(BPTR lock, CONST_STRPTR name,
    uint64_t *watch)
{
    struct AfsplusExtRequest request;
    struct MsgPort *port = afsplus_client_lock_port(lock);
    size_t length = name != NULL ? strlen((const char *)name) : 0;
    LONG error;

    if (watch == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *watch = 0;
    if (port == NULL)
        return ERROR_INVALID_LOCK;
    if (length > AFSPLUS_EXT_NAME_MAX)
        return ERROR_INVALID_COMPONENT_NAME;
    begin(&request, AFSPLUS_EXT_WATCH_ADD);
    request.object[0] = (uint64_t)(uintptr_t)lock;
    request.name0 = (const uint8_t *)name;
    request.name_length[0] = (uint32_t)length;
    error = afsplus_client_send(port, &request);
    if (error == 0)
        *watch = request.output_value;
    return error;
}

LONG afsplus_client_watch_take(struct MsgPort *port, uint64_t watch,
    uint32_t *changed)
{
    struct AfsplusExtRequest request;
    LONG error;

    if (changed == NULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *changed = 0;
    begin(&request, AFSPLUS_EXT_WATCH_TAKE);
    request.offset[0] = watch;
    error = afsplus_client_send(port, &request);
    if (error == 0)
        *changed = request.output_flags;
    return error;
}

LONG afsplus_client_watch_remove(struct MsgPort *port, uint64_t watch)
{
    struct AfsplusExtRequest request;

    begin(&request, AFSPLUS_EXT_WATCH_REMOVE);
    request.offset[0] = watch;
    return afsplus_client_send(port, &request);
}

LONG afsplus_client_health_events(struct MsgPort *port,
    struct AfsplusArosHealthEvent *events, uint32_t capacity,
    uint32_t *count, uint64_t *lost)
{
    struct AfsplusExtRequest request;
    LONG error;

    if (count == NULL || lost == NULL || (capacity != 0 && events == NULL)
        || capacity > UINT32_MAX / sizeof(*events))
        return ERROR_REQUIRED_ARG_MISSING;
    *count = 0;
    *lost = 0;
    begin(&request, AFSPLUS_EXT_HEALTH_EVENTS);
    request.buffer = events;
    request.buffer_size = capacity * (uint32_t)sizeof(*events);
    error = afsplus_client_send(port, &request);
    if (error == 0)
    {
        *count = request.output_count;
        *lost = request.output_value;
    }
    return error;
}

LONG afsplus_client_trace_events(struct MsgPort *port,
    struct afsp_trace_event *events, uint32_t capacity, uint32_t *count,
    uint64_t *lost)
{
    struct AfsplusExtRequest request;
    LONG error;

    if (count == NULL || lost == NULL || (capacity != 0 && events == NULL)
        || capacity > UINT32_MAX / sizeof(*events))
        return ERROR_REQUIRED_ARG_MISSING;
    *count = 0;
    *lost = 0;
    begin(&request, AFSPLUS_EXT_TRACE_EVENTS);
    request.buffer = events;
    request.buffer_size = capacity * (uint32_t)sizeof(*events);
    error = afsplus_client_send(port, &request);
    if (error == 0)
    {
        *count = request.output_count;
        *lost = request.output_value;
    }
    return error;
}

LONG afsplus_client_trace_counters(struct MsgPort *port,
    struct AfsplusArosTraceCounters *output)
{
    return report(port, AFSPLUS_EXT_TRACE_COUNTERS, output, sizeof(*output));
}

LONG afsplus_client_commit_policy(struct MsgPort *port,
    struct AfsplusExtCommitPolicy *output)
{
    return report(port, AFSPLUS_EXT_COMMIT_POLICY, output, sizeof(*output));
}

LONG afsplus_client_snapshot_create(struct MsgPort *port, uint64_t *id)
{
    struct AfsplusExtRequest r; LONG e;
    if (id == NULL) return ERROR_REQUIRED_ARG_MISSING;
    *id = 0; begin(&r, AFSPLUS_EXT_SNAPSHOT_CREATE);
    e = afsplus_client_send(port, &r); if (!e) *id = r.output_value; return e;
}
LONG afsplus_client_snapshot_list(struct MsgPort *port, uint64_t low,
    struct AfsplusArosSnapshotInfo *entries, uint32_t capacity, uint32_t *count, uint64_t *next)
{
    struct AfsplusExtRequest r; LONG e;
    if (entries == NULL || count == NULL || next == NULL || capacity == 0 || capacity > 64)
        return ERROR_BAD_NUMBER;
    *count = 0; *next = 0; begin(&r, AFSPLUS_EXT_SNAPSHOT_LIST);
    r.offset[0] = low; r.length = capacity; r.buffer = entries;
    r.buffer_size = capacity * sizeof(*entries);
    e = afsplus_client_send(port, &r); if (!e) { *count = r.output_count; *next = r.output_value; } return e;
}
LONG afsplus_client_snapshot_delete(struct MsgPort *port, uint64_t id)
{
    struct AfsplusExtRequest r; begin(&r, AFSPLUS_EXT_SNAPSHOT_DELETE);
    r.object[0] = id; return afsplus_client_send(port, &r);
}
LONG afsplus_client_snapshot_open(struct MsgPort *port, uint64_t id, uint64_t *reader)
{
    struct AfsplusExtRequest r; LONG e;
    if (reader == NULL) return ERROR_REQUIRED_ARG_MISSING;
    *reader = 0; begin(&r, AFSPLUS_EXT_SNAPSHOT_OPEN); r.object[0] = id;
    e = afsplus_client_send(port, &r); if (!e) *reader = r.output_value; return e;
}
LONG afsplus_client_snapshot_close(struct MsgPort *port, uint64_t reader)
{
    struct AfsplusExtRequest r; begin(&r, AFSPLUS_EXT_SNAPSHOT_CLOSE);
    r.offset[0] = reader; return afsplus_client_send(port, &r);
}
LONG afsplus_client_snapshot_lookup(struct MsgPort *port, uint64_t reader,
    uint64_t directory, CONST_STRPTR name, uint64_t *object)
{
    struct AfsplusExtRequest r; LONG e;
    if (name == NULL || object == NULL) return ERROR_REQUIRED_ARG_MISSING;
    *object = 0; begin(&r, AFSPLUS_EXT_SNAPSHOT_LOOKUP); r.offset[0] = reader;
    r.object[0] = directory; r.name0 = (const uint8_t *)name;
    r.name_length[0] = (uint32_t)strlen((const char *)name);
    e = afsplus_client_send(port, &r); if (!e) *object = r.output_value; return e;
}
LONG afsplus_client_snapshot_read(struct MsgPort *port, uint64_t reader,
    uint64_t object, uint64_t offset, void *buffer, uint32_t capacity, uint32_t *count)
{
    struct AfsplusExtRequest r; LONG e;
    if (count == NULL || (buffer == NULL && capacity != 0)) return ERROR_REQUIRED_ARG_MISSING;
    *count = 0; begin(&r, AFSPLUS_EXT_SNAPSHOT_READ); r.offset[0] = reader;
    r.object[0] = object; r.offset[1] = offset; r.buffer = buffer; r.buffer_size = capacity;
    e = afsplus_client_send(port, &r); if (!e) *count = r.output_count; return e;
}
LONG afsplus_client_snapshot_stat(struct MsgPort *port, uint64_t reader,
    uint64_t object, struct AfsplusArosStat *output)
{
    struct AfsplusExtRequest r;
    if (output == NULL) return ERROR_REQUIRED_ARG_MISSING;
    memset(output, 0, sizeof(*output)); output->struct_size = sizeof(*output);
    begin(&r, AFSPLUS_EXT_SNAPSHOT_STAT); r.offset[0] = reader; r.object[0] = object;
    r.buffer = output; r.buffer_size = sizeof(*output); return afsplus_client_send(port, &r);
}
LONG afsplus_client_snapshot_dir_open(struct MsgPort *port, uint64_t reader,
    uint64_t object, uint64_t *directory)
{
    struct AfsplusExtRequest r; LONG e;
    if (directory == NULL) return ERROR_REQUIRED_ARG_MISSING;
    *directory = 0; begin(&r, AFSPLUS_EXT_SNAPSHOT_DIR_OPEN); r.offset[0] = reader; r.object[0] = object;
    e = afsplus_client_send(port, &r); if (!e) *directory = r.output_value; return e;
}
LONG afsplus_client_snapshot_dir_read(struct MsgPort *port, uint64_t directory,
    void *buffer, uint32_t capacity, uint32_t limit, uint32_t *count, uint32_t *eof)
{
    struct AfsplusExtRequest r; LONG e;
    if (count == NULL || eof == NULL || buffer == NULL) return ERROR_REQUIRED_ARG_MISSING;
    *count = 0; *eof = 0; begin(&r, AFSPLUS_EXT_SNAPSHOT_DIR_READ); r.offset[0] = directory;
    r.buffer = buffer; r.buffer_size = capacity; r.length = limit;
    e = afsplus_client_send(port, &r); if (!e) { *count = r.output_count; *eof = r.output_flags; } return e;
}
LONG afsplus_client_snapshot_dir_close(struct MsgPort *port, uint64_t directory)
{
    struct AfsplusExtRequest r; begin(&r, AFSPLUS_EXT_SNAPSHOT_DIR_CLOSE);
    r.offset[0] = directory; return afsplus_client_send(port, &r);
}

LONG afsplus_client_snapshot_restore_file(struct MsgPort *port, uint64_t reader,
    uint64_t source, BPTR destination, CONST_STRPTR name, uint64_t *object)
{
    struct AfsplusExtRequest request;
    LONG error;
    if (object == NULL || name == NULL || destination == BNULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *object = 0;
    if (afsplus_client_lock_port(destination) != port)
        return ERROR_RENAME_ACROSS_DEVICES;
    begin(&request, AFSPLUS_EXT_SNAPSHOT_RESTORE_FILE);
    request.offset[0] = reader;
    request.object[0] = source;
    request.object[1] = (uint64_t)(uintptr_t)destination;
    request.name0 = (const uint8_t *)name;
    request.name_length[0] = (uint32_t)strlen((const char *)name);
    error = afsplus_client_send(port, &request);
    if (error == 0) *object = request.output_value;
    return error;
}

LONG afsplus_client_snapshot_restore_tree(struct MsgPort *port, uint64_t reader,
    uint64_t source, BPTR destination, CONST_STRPTR name, uint32_t max_nodes,
    uint32_t max_depth, uint32_t max_metadata_bytes, uint64_t *object)
{
    struct AfsplusExtRequest request;
    LONG error;
    if (object == NULL || name == NULL || destination == BNULL)
        return ERROR_REQUIRED_ARG_MISSING;
    *object = 0;
    if (max_nodes == 0 || max_depth == 0 || max_metadata_bytes == 0)
        return ERROR_BAD_NUMBER;
    if (afsplus_client_lock_port(destination) != port)
        return ERROR_RENAME_ACROSS_DEVICES;
    begin(&request, AFSPLUS_EXT_SNAPSHOT_RESTORE_TREE);
    request.offset[0] = reader;
    request.offset[1] = max_depth;
    request.object[0] = source;
    request.object[1] = (uint64_t)(uintptr_t)destination;
    request.name0 = (const uint8_t *)name;
    request.name_length[0] = (uint32_t)strlen((const char *)name);
    request.length = max_nodes;
    request.buffer_size = max_metadata_bytes;
    error = afsplus_client_send(port, &request);
    if (error == 0) *object = request.output_value;
    return error;
}
