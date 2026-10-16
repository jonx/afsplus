/* Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * Licensed under the AROS Public License (APL) Version 1.1.
 */
#define main existing_packet_matrix
#include "packet_stub.c"
#undef main
#include "filesystem_paging_v2.h"

static void dispatch(struct AfsplusArosPacketContext *context,
    struct DosPacket *packet)
{
    assert(afsplus_aros_packet_process(context, packet) == 0);
}
static void close_handle(struct AfsplusArosPacketContext *context,
    struct FileHandle *handle)
{
    struct DosPacket packet;
    initialize_packet(&packet, ACTION_END);
    packet.dp_Arg1 = handle->fh_Arg1;
    dispatch(context, &packet);
    assert(packet.dp_Res1 == DOSTRUE && packet.dp_Res2 == 0);
}
static int32_t snapshot(struct AfsplusArosPacketContext *context,
    struct FileHandle *source, struct FileHandle *output, uint32_t *storage)
{
    struct DosPacket packet;
    struct FSV2_PagingRequest request = {0};
    request.struct_size = sizeof(request);
    request.version = FSV2_PAGING_VERSION;
    request.operation = FSV2_PAGING_SNAPSHOT_OPEN;
    request.flags = FSV2_SNAPSHOT_ALLOW_COPY;
    request.source = (uint64_t)source->fh_Arg1;
    request.destination = output;
    request.storage = 99;
    initialize_packet(&packet, ACTION_FILESYSTEM_PAGING_V2);
    packet.dp_Arg1 = (SIPTR)&request;
    dispatch(context, &packet);
    if (packet.dp_Res2 == 0) {
        assert(packet.dp_Res1 == DOSTRUE && request.size == file_size);
        assert(output->fh_Type == (void *)(uintptr_t)0x2000);
        *storage = request.storage;
    } else {
        assert(packet.dp_Res1 == DOSFALSE && request.storage == 99);
        assert(output->fh_Arg1 == 0);
    }
    return (int32_t)packet.dp_Res2;
}
int main(void)
{
    struct AfsplusArosPacketConfig config = {0};
    struct AfsplusArosPacketContext *context = NULL;
    struct FileHandle source = {0}, output = {0}, duplicate = {0};
    struct DosPacket packet;
    uint32_t storage = 0;
    uint8_t buffer[257];
    uint32_t baseline;
    config.abi_version = AFSPLUS_AROS_PACKET_ABI_VERSION;
    config.struct_size = sizeof(config);
    config.filesystem = STUB_FILESYSTEM;
    config.handler_port = (void *)(uintptr_t)0x2000;
    config.allocate = packet_allocate; config.free = packet_free;
    config.now = packet_now;
    assert(afsplus_aros_packet_create(&config, &context) == 0);
    initialize_packet(&packet, ACTION_FINDINPUT);
    packet.dp_Arg1 = (SIPTR)MKBADDR(&source);
    packet.dp_Arg3 = packet_bstr("source");
    dispatch(context, &packet);
    assert(packet.dp_Res1 == DOSTRUE);
    file_size = 65536 + 171;
    baseline = paging_live;
    /* Real clone dispatch: the FFI's actual CoW semantics are tested in Rust. */
    paging_clone_error = 0;
    assert(snapshot(context, &source, &output, &storage) == 0);
    assert(storage == FSV2_SNAPSHOT_REFLINK);
    close_handle(context, &output);
    assert(paging_live == baseline);
    /* Both refusal classes take the explicit immutable copy fallback. */
    for (unsigned profile = 0; profile != 2; profile++) {
        paging_clone_error = profile == 0 ? ERROR_ACTION_NOT_KNOWN
            : ERROR_DISK_WRITE_PROTECTED;
        paging_read_mode = 1;
        paging_source_byte = 0x35;
        memset(&output, 0, sizeof(output));
        assert(snapshot(context, &source, &output, &storage) == 0);
        assert(storage == FSV2_SNAPSHOT_RAM);
        paging_source_byte = 0x6a; /* mutate source after creation */
        initialize_packet(&packet, ACTION_READ);
        packet.dp_Arg1 = output.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)buffer; packet.dp_Arg3 = sizeof(buffer);
        dispatch(context, &packet);
        assert(packet.dp_Res1 == sizeof(buffer) && packet.dp_Res2 == 0);
        for (unsigned i = 0; i != sizeof(buffer); i++) assert(buffer[i] == 0x35);
        initialize_packet(&packet, ACTION_SEEK64);
        packet.dp_Arg1 = output.fh_Arg1;
        packet.dp_Arg2 = -3; packet.dp_Arg3 = OFFSET_END;
        dispatch(context, &packet);
        assert(packet.dp_Res1 == sizeof(buffer) && packet.dp_Res2 == 0);
        initialize_packet(&packet, ACTION_READ);
        packet.dp_Arg1 = output.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)buffer; packet.dp_Arg3 = sizeof(buffer);
        dispatch(context, &packet);
        assert(packet.dp_Res1 == 3 && buffer[0] == 0x35 && buffer[2] == 0x35);
        initialize_packet(&packet, ACTION_GET_FILE_POSITION64);
        packet.dp_Arg1 = output.fh_Arg1;
        dispatch(context, &packet);
        assert(packet.dp_Res1 == (SIPTR)file_size && packet.dp_Res2 == 0);
        initialize_packet(&packet, ACTION_WRITE);
        packet.dp_Arg1 = output.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)buffer; packet.dp_Arg3 = 1;
        dispatch(context, &packet);
        assert(packet.dp_Res1 == -1 && packet.dp_Res2 == ERROR_DISK_WRITE_PROTECTED);
        memset(&duplicate, 0, sizeof(duplicate));
        assert(snapshot(context, &output, &duplicate, &storage) == 0);
        close_handle(context, &output);
        initialize_packet(&packet, ACTION_READ);
        packet.dp_Arg1 = duplicate.fh_Arg1;
        packet.dp_Arg2 = (SIPTR)buffer; packet.dp_Arg3 = 1;
        dispatch(context, &packet);
        assert(packet.dp_Res1 == 1 && buffer[0] == 0x35);
        close_handle(context, &duplicate);
        assert(paging_live == baseline);
    }
    /* Fail each allocation: reserved cookie, snapshot, first/second chunks. */
    paging_clone_error = ERROR_ACTION_NOT_KNOWN;
    for (unsigned allocation = 1; allocation <= 4; allocation++) {
        memset(&output, 0, sizeof(output));
        paging_fail_at = paging_alloc_calls + allocation;
        assert(snapshot(context, &source, &output, &storage) == ERROR_NO_FREE_STORE);
        assert(paging_live == baseline);
        paging_fail_at = 0;
    }
    /* Real IO errors must not quietly fall back to a weaker source handle. */
    paging_clone_error = ERROR_SEEK_ERROR;
    memset(&output, 0, sizeof(output));
    assert(snapshot(context, &source, &output, &storage) == ERROR_SEEK_ERROR);
    assert(paging_live == baseline);
    close_handle(context, &source);
    assert(afsplus_aros_packet_destroy(context) == 0 && paging_live == 0);
    puts("mmap snapshot packet: clone / copy profiles / short read / immutable / shared backing / OOM PASS");
    return 0;
}
