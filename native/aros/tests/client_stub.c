/* SPDX-License-Identifier: BSD-2-Clause */

/* Host matrix of the extension-packet client: what it sends, and what it
 * does when the handler does not know the packet. dos.library is faked. */

#include "../client/afsplus_client.h"

#include <dos/dosextens.h>
#include <proto/dos.h>

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* AROS <assert.h> calls this; the host libc has no such symbol. */
void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL);
    abort();
}

static struct MsgPort handler_port;
static struct MsgPort other_port;
static SIPTR io_error;
static uint32_t transport_present = 1;
static int32_t handler_error;
static uint32_t packets_sent;
static struct AfsplusExtRequest last_request;
static struct MsgPort *last_port;

/* A fake file of 64 bytes, byte i holding i, with a classic position. */
static uint8_t file_bytes[64];
static LONG file_position;
static uint32_t seeks;
static LONG fail_transfer;
/* Fails the n-th Seek from now (1: the next one); 0: none. */
static uint32_t fail_seek_in;

static int forbid_depth;

void Forbid(void)
{
    forbid_depth++;
}

void Permit(void)
{
    assert(forbid_depth > 0);
    forbid_depth--;
}

SIPTR IoErr(void)
{
    return io_error;
}

SIPTR SetIoErr(SIPTR result)
{
    SIPTR previous = io_error;

    io_error = result;
    return previous;
}

SIPTR DoPkt(struct MsgPort *port, LONG action, SIPTR arg1, SIPTR arg2,
    SIPTR arg3, SIPTR arg4, SIPTR arg5)
{
    struct AfsplusExtRequest *request = (struct AfsplusExtRequest *)arg1;

    assert(action == ACTION_AFSPLUS_EXT);
    assert(arg2 == 0 && arg3 == 0 && arg4 == 0 && arg5 == 0);
    packets_sent++;
    last_port = port;
    if (!transport_present)
    {
        io_error = ERROR_ACTION_NOT_KNOWN;
        return DOSFALSE;
    }
    assert(request->magic == AFSPLUS_EXT_MAGIC);
    assert(request->version == AFSPLUS_EXT_VERSION);
    assert(request->header_size == sizeof(*request));
    last_request = *request;
    if (handler_error != 0)
    {
        io_error = handler_error;
        return DOSFALSE;
    }
    switch (request->operation)
    {
    case AFSPLUS_EXT_INTERFACE:
        request->output_count = 14;
        request->output_flags = 4;
        request->output_value = UINT64_C(0x7FFF);
        break;
    case AFSPLUS_EXT_READ_AT:
        memcpy(request->buffer, file_bytes + request->offset[0],
            request->buffer_size);
        request->output_count = request->buffer_size;
        break;
    case AFSPLUS_EXT_WRITE_AT:
        request->output_count = request->buffer_size;
        break;
    case AFSPLUS_EXT_INFO_JSON:
        request->output_value = 300;
        break;
    case AFSPLUS_EXT_GET_ATTRIBUTE:
        request->output_value = 9;
        if (request->buffer_size >= 9)
            memcpy(request->buffer, "DONOTWAIT", 9);
        break;
    case AFSPLUS_EXT_PACKET_COUNTS:
        request->output_count = request->buffer_size / 24;
        request->output_value = 9;
        break;
    case AFSPLUS_EXT_COUNTERS:
    {
        struct AfsplusArosCounters *counters = request->buffer;

        assert(counters->struct_size == sizeof(*counters));
        assert(request->buffer_size == sizeof(*counters));
        break;
    }
    default:
        break;
    }
    /* A real DoPkt leaves the packet's dp_Res2 in IoErr on success too. */
    io_error = 0x5151;
    return DOSTRUE;
}

LONG Seek(BPTR file, LONG position, LONG mode)
{
    LONG previous = file_position;

    (void)file;
    assert(mode == OFFSET_BEGINNING);
    seeks++;
    if (fail_seek_in != 0 && --fail_seek_in == 0)
    {
        io_error = ERROR_DISK_NOT_VALIDATED;
        return -1;
    }
    if (position < 0 || position > (LONG)sizeof(file_bytes))
    {
        io_error = ERROR_SEEK_ERROR;
        return -1;
    }
    file_position = position;
    return previous;
}

static LONG transfer(uint8_t *buffer, const uint8_t *source, LONG length)
{
    LONG available = (LONG)sizeof(file_bytes) - file_position;

    if (fail_transfer != 0)
    {
        io_error = fail_transfer;
        return -1;
    }
    if (length > available)
        length = available;
    if (buffer != NULL)
        memcpy(buffer, file_bytes + file_position, (size_t)length);
    else
        memcpy(file_bytes + file_position, source, (size_t)length);
    file_position += length;
    return length;
}

LONG Read(BPTR file, APTR buffer, LONG length)
{
    (void)file;
    return transfer(buffer, NULL, length);
}

LONG Write(BPTR file, CONST_APTR buffer, LONG length)
{
    (void)file;
    return transfer(NULL, buffer, length);
}

int main(void)
{
    struct FileHandle handle;
    struct FileHandle foreign_handle;
    struct FileLock lock;
    struct FileLock foreign_lock;
    struct AfsplusArosCounters counters;
    BPTR file;
    uint8_t data[8];
    uint32_t count = 99;
    uint32_t revision = 0;
    uint32_t abi = 0;
    uint32_t required = 0;
    uint64_t groups = 0;
    size_t index;

    for (index = 0; index < sizeof(file_bytes); index++)
        file_bytes[index] = (uint8_t)index;
    memset(&handle, 0, sizeof(handle));
    handle.fh_Type = &handler_port;
    handle.fh_Arg1 = (SIPTR)0x1234560;
    foreign_handle = handle;
    foreign_handle.fh_Type = &other_port;
    memset(&lock, 0, sizeof(lock));
    lock.fl_Task = &handler_port;
    foreign_lock = lock;
    foreign_lock.fl_Task = &other_port;
    file = MKBADDR(&handle);

    assert(afsplus_client_file_port(file) == &handler_port);
    assert(afsplus_client_file_port(BNULL) == NULL);
    assert(afsplus_client_lock_port(MKBADDR(&lock)) == &handler_port);

    /* No secondary result survives a call, success or failure. */
    assert(afsplus_client_interface(&handler_port, &revision, &abi, &groups)
        == 0);
    assert(revision == 14 && abi == 4 && groups == UINT64_C(0x7FFF));
    assert(io_error == 0 && last_port == &handler_port);
    assert(afsplus_client_interface(NULL, NULL, NULL, NULL)
        == ERROR_REQUIRED_ARG_MISSING);

    /* With the transport the classic position is never touched. */
    file_position = 7;
    assert(afsplus_client_read_at(file, 16, data, sizeof(data), &count) == 0);
    assert(count == 8 && data[0] == 16 && data[7] == 23);
    assert(last_request.object[0] == UINT64_C(0x1234560));
    assert(last_request.offset[0] == 16 && seeks == 0 && file_position == 7);
    assert(afsplus_client_write_at(file, UINT64_C(0x900000000), data, 8,
        &count) == 0);
    assert(last_request.operation == AFSPLUS_EXT_WRITE_AT);
    assert(last_request.offset[0] == UINT64_C(0x900000000) && seeks == 0);

    /* A handler error other than "unknown packet" is the answer: no
     * fallback hides it. */
    handler_error = ERROR_DISK_FULL;
    count = 99;
    assert(afsplus_client_write_at(file, 0, data, 8, &count)
        == ERROR_DISK_FULL);
    assert(count == 0 && seeks == 0 && io_error == 0);
    handler_error = 0;

    /* A handler that knows the packet and cannot serve it is not a missing
     * transport: no fallback, and the position stays where it was. */
    handler_error = ERROR_NOT_IMPLEMENTED;
    assert(afsplus_client_read_at(file, 0, data, 8, &count)
        == ERROR_NOT_IMPLEMENTED);
    assert(seeks == 0 && file_position == 7);
    handler_error = 0;

    /* Without the transport: seek, transfer, seek back. */
    transport_present = 0;
    file_position = 7;
    memset(data, 0, sizeof(data));
    packets_sent = 0;
    assert(afsplus_client_read_at(file, 32, data, sizeof(data), &count) == 0);
    assert(count == 8 && data[0] == 32 && data[7] == 39);
    assert(seeks == 2 && file_position == 7 && io_error == 0);
    /* The port is remembered: the next fallback asks nothing first. */
    assert(packets_sent == 1);
    assert(afsplus_client_read_at(file, 32, data, sizeof(data), &count) == 0);
    assert(packets_sent == 1 && seeks == 4);
    seeks = 2;
    /* A short read near the end is a count, not an error. */
    assert(afsplus_client_read_at(file, 60, data, sizeof(data), &count) == 0);
    assert(count == 4 && file_position == 7);
    data[0] = 0xEE;
    assert(afsplus_client_write_at(file, 3, data, 1, &count) == 0);
    assert(count == 1 && file_bytes[3] == 0xEE && file_position == 7);
    /* A failed transfer still restores the position and keeps its error. */
    fail_transfer = ERROR_DISK_FULL;
    assert(afsplus_client_write_at(file, 3, data, 1, &count)
        == ERROR_DISK_FULL);
    assert(count == 0 && file_position == 7);
    /* A position that cannot be restored is reported as such, also when the
     * transfer failed too: that error must not hide the lost position. */
    fail_seek_in = 2;
    assert(afsplus_client_write_at(file, 3, data, 1, &count)
        == ERROR_SEEK_ERROR);
    assert(count == 0 && file_position == 3);
    fail_transfer = 0;
    fail_seek_in = 2;
    assert(afsplus_client_read_at(file, 40, data, 4, &count)
        == ERROR_SEEK_ERROR);
    assert(count == 4 && data[0] == 40 && io_error == 0);
    file_position = 7;
    /* Beyond what a classic Seek expresses nothing is attempted. */
    seeks = 0;
    assert(afsplus_client_read_at(file, UINT64_C(0x80000000), data, 1,
        &count) == ERROR_OBJECT_TOO_LARGE);
    assert(afsplus_client_read_at(file, INT32_MAX, data, 1, &count)
        == ERROR_OBJECT_TOO_LARGE);
    assert(seeks == 0);

    /* Everything else has no classic equivalent and says so. */
    assert(afsplus_client_clone_file(MKBADDR(&lock), BNULL,
        (CONST_STRPTR)"copy") == ERROR_ACTION_NOT_KNOWN);
    assert(afsplus_client_preallocate(file, 0, 4096)
        == ERROR_ACTION_NOT_KNOWN);
    assert(afsplus_client_info_json(&handler_port, NULL, 0, &required)
        == ERROR_ACTION_NOT_KNOWN);
    transport_present = 1;
    /* Still remembered as absent until the program says otherwise. */
    seeks = 0;
    assert(afsplus_client_read_at(file, 16, data, 8, &count) == 0);
    assert(seeks == 2);
    afsplus_client_forget_ports();
    seeks = 0;
    assert(afsplus_client_read_at(file, 16, data, 8, &count) == 0);
    assert(seeks == 0 && forbid_depth == 0);

    /* Objects of two handlers are never sent to one of them. */
    packets_sent = 0;
    assert(afsplus_client_clone_file(MKBADDR(&lock), MKBADDR(&foreign_lock),
        (CONST_STRPTR)"copy") == ERROR_RENAME_ACROSS_DEVICES);
    assert(afsplus_client_clone_range(file, 0, MKBADDR(&foreign_handle), 0,
        16) == ERROR_RENAME_ACROSS_DEVICES);
    assert(afsplus_client_clone_file(BNULL, BNULL, (CONST_STRPTR)"copy")
        == ERROR_INVALID_LOCK);
    assert(packets_sent == 0);
    assert(afsplus_client_clone_file(MKBADDR(&lock), MKBADDR(&lock),
        (CONST_STRPTR)"copy") == 0);
    assert(last_request.object[0] == (uint64_t)(uintptr_t)MKBADDR(&lock));
    assert(last_request.name_length[1] == 4
        && memcmp(last_request.name1, "copy", 4) == 0);
    assert(afsplus_client_clone_range(file, 4, file, 8, 16) == 0);
    assert(last_request.offset[0] == 4 && last_request.offset[1] == 8
        && last_request.length == 16);

    /* Reports declare the size this client knows. */
    memset(&counters, 0xFF, sizeof(counters));
    assert(afsplus_client_counters(&handler_port, &counters) == 0);
    assert(counters.struct_size == sizeof(counters));
    assert(afsplus_client_info_json(&handler_port, (char *)data,
        sizeof(data), &required) == 0);
    assert(required == 300);

    /* Attributes address the lock's own object and carry the mode in flags;
     * a lock of no handler sends nothing. */
    {
        uint8_t value[16];
        uint32_t needed = 0;

        memset(value, 0, sizeof(value));
        assert(afsplus_client_get_attribute(MKBADDR(&lock),
            (CONST_STRPTR)"aros.tooltype", value, sizeof(value), &needed)
            == 0);
        assert(needed == 9 && memcmp(value, "DONOTWAIT", 9) == 0);
        assert(last_request.object[0] == (uint64_t)(uintptr_t)MKBADDR(&lock));
        assert(last_request.name_length[0] == 0);
        assert(last_request.name_length[1] == 13 && last_request.flags == 0);
        assert(afsplus_client_set_attribute(MKBADDR(&lock),
            (CONST_STRPTR)"user.kind", "text", 4,
            AFSPLUS_AROS_ATTRIBUTE_REPLACE) == 0);
        assert(last_request.operation == AFSPLUS_EXT_SET_ATTRIBUTE);
        assert(last_request.flags == AFSPLUS_AROS_ATTRIBUTE_REPLACE);
        assert(last_request.buffer_size == 4);
        packets_sent = 0;
        assert(afsplus_client_list_attributes(BNULL, NULL, 0, &needed)
            == ERROR_INVALID_LOCK);
        assert(afsplus_client_set_attribute(MKBADDR(&lock), NULL, "x", 1, 0)
            == ERROR_REQUIRED_ARG_MISSING);
        assert(packets_sent == 0);
    }

    /* The record array is sized in bytes on the wire. */
    {
        struct AfsplusExtPacketCount records[3];
        uint32_t stored = 0;
        uint32_t total = 0;

        assert(afsplus_client_packet_counts(&handler_port,
            AFSPLUS_EXT_COUNT_BY_ERROR, records, 3, &stored, &total) == 0);
        assert(last_request.flags == AFSPLUS_EXT_COUNT_BY_ERROR);
        assert(last_request.buffer_size == 72 && stored == 3 && total == 9);
        assert(afsplus_client_packet_counts(&handler_port, 0, NULL, 3,
            &stored, &total) == ERROR_REQUIRED_ARG_MISSING);
    }

    puts("afsplus client stub: PASS");
    return 0;
}
