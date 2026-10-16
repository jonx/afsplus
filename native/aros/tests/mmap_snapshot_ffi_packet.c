/* Copyright (C) 2026 John KNIPPER <code@jkn.me>
 * Licensed under the AROS Public License (APL) Version 1.1.
 */
#include "afsplus_packet.h"
#include "filesystem_paging_v2.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BLOCK_SIZE 4096U
#define BLOCKS 8192U
static uint32_t live;
static int32_t read_block(void *disk, uint64_t lba, uint8_t *data, uint32_t length)
{
    assert(lba < BLOCKS && length == BLOCK_SIZE);
    memcpy(data, (uint8_t *)disk + lba * BLOCK_SIZE, length);
    return 0;
}
static int32_t write_block(void *disk, uint64_t lba, const uint8_t *data,
    uint32_t length)
{
    assert(lba < BLOCKS && length == BLOCK_SIZE);
    memcpy((uint8_t *)disk + lba * BLOCK_SIZE, data, length);
    return 0;
}
static int32_t flush(void *disk) { (void)disk; return 0; }
static void *allocate(void *opaque, size_t size)
{
    void *allocation;
    (void)opaque;
    allocation = calloc(1, size);
    if (allocation != NULL) live++;
    return allocation;
}
static void release(void *opaque, void *allocation, size_t size)
{
    (void)opaque; (void)size;
    assert(live != 0); live--;
    free(allocation);
}
static int32_t now(void *opaque, int64_t *seconds, uint32_t *nanoseconds)
{
    (void)opaque; *seconds = 100; *nanoseconds = 0; return 0;
}
void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL); abort();
}
static void send(struct AfsplusArosPacketContext *context, struct DosPacket *packet)
{
    assert(afsplus_aros_packet_process(context, packet) == 0);
    if (packet->dp_Res2 != 0) printf("action %ld error %ld\n",
        (long)packet->dp_Type, (long)packet->dp_Res2);
    assert(packet->dp_Res2 == 0);
}
static void run_profile(uint32_t mount_mode)
{
    uint8_t uuid[16] = {0x55};
    uint8_t bytes[65536 + 171], output[sizeof(bytes)];
    struct AfsplusArosDevice device = {0};
    struct AfsplusArosMountConfig mount = {0};
    struct AfsplusArosPacketConfig config = {0};
    struct AfsplusAros *fs = NULL;
    struct AfsplusArosPacketContext *context = NULL;
    struct FileHandle source = {0}, snapshot = {0};
    struct DosPacket packet = {0};
    struct FSV2_PagingRequest request = {0};
    struct FileInfoBlock fib;
    uint64_t source_id = 0;
    uint32_t count = 0;
    device.abi_version = AFSPLUS_AROS_ABI_VERSION;
    device.struct_size = sizeof(device);
    device.context = calloc(BLOCKS, BLOCK_SIZE);
    assert(device.context != NULL);
    device.block_size = BLOCK_SIZE; device.total_blocks = BLOCKS;
    device.read_block = read_block; device.write_block = write_block;
    device.flush = flush;
    assert(afsplus_aros_format(&device, AFSPLUS_AROS_ENCODING_UTF8,
        (const uint8_t *)"Snapshot", 8, uuid, 1, 0) == 0);
    mount.abi_version = AFSPLUS_AROS_ABI_VERSION;
    mount.struct_size = sizeof(mount);
    mount.max_file_handles = 32; mount.max_locks = 32;
    mount.max_file_info_name_bytes = 107;
    assert(afsplus_aros_mount(&device, &mount, &fs) == 0);
    assert(afsplus_aros_open(fs, 0, (const uint8_t *)"source", 6,
        AFSPLUS_AROS_OPEN_NEW_FILE, 2, 0, &source_id) == 0);
    memset(bytes, 0x35, sizeof(bytes));
    assert(afsplus_aros_write_at(fs, source_id, 0, bytes, sizeof(bytes),
        3, 0, &count) == 0 && count == sizeof(bytes));
    assert(afsplus_aros_close(fs, source_id) == 0);
    assert(afsplus_aros_unmount(fs) == 0);
    mount.mount_mode = mount_mode;
    assert(afsplus_aros_mount(&device, &mount, &fs) == 0);
    config.abi_version = AFSPLUS_AROS_PACKET_ABI_VERSION;
    config.struct_size = sizeof(config); config.filesystem = fs;
    config.handler_port = (void *)(uintptr_t)0x2000;
    config.allocate = allocate; config.free = release; config.now = now;
    assert(afsplus_aros_packet_create(&config, &context) == 0);
    packet.dp_Type = ACTION_FINDINPUT;
    packet.dp_Arg1 = (SIPTR)MKBADDR(&source);
    packet.dp_Arg3 = (SIPTR)"source";
    send(context, &packet);
    request.struct_size = sizeof(request); request.version = FSV2_PAGING_VERSION;
    request.operation = FSV2_PAGING_SNAPSHOT_OPEN;
    request.flags = FSV2_SNAPSHOT_ALLOW_COPY;
    request.source = source.fh_Arg1; request.destination = &snapshot;
    memset(&packet, 0, sizeof(packet));
    packet.dp_Type = ACTION_FILESYSTEM_PAGING_V2; packet.dp_Arg1 = (SIPTR)&request;
    send(context, &packet);
    assert(request.size == sizeof(bytes));
    assert(request.storage == (mount_mode == AFSPLUS_AROS_MOUNT_READ_WRITE
        ? FSV2_SNAPSHOT_REFLINK : FSV2_SNAPSHOT_RAM));
    if (mount_mode == AFSPLUS_AROS_MOUNT_READ_WRITE) {
        /* A second real handle mutates and removes the source after clone. */
        assert(afsplus_aros_open(fs, 0, (const uint8_t *)"source", 6,
            AFSPLUS_AROS_OPEN_READ_WRITE, 101, 0, &source_id) == 0);
        memset(output, 0x6a, sizeof(output));
        assert(afsplus_aros_write_at(fs, source_id, 0, output, sizeof(output),
            102, 0, &count) == 0);
        assert(afsplus_aros_close(fs, source_id) == 0);
    }
    memset(&packet, 0, sizeof(packet)); packet.dp_Type = ACTION_END;
    packet.dp_Arg1 = source.fh_Arg1; send(context, &packet);
    if (mount_mode == AFSPLUS_AROS_MOUNT_READ_WRITE)
        assert(afsplus_aros_delete_object(fs, 0, (const uint8_t *)"source", 6,
            103, 0) == 0);
    memset(&packet, 0, sizeof(packet)); packet.dp_Type = ACTION_READ;
    packet.dp_Arg1 = snapshot.fh_Arg1; packet.dp_Arg2 = (SIPTR)output;
    packet.dp_Arg3 = sizeof(output); send(context, &packet);
    assert(packet.dp_Res1 == sizeof(output) && memcmp(output, bytes, sizeof(bytes)) == 0);
    memset(&packet, 0, sizeof(packet)); packet.dp_Type = ACTION_EXAMINE_FH;
    packet.dp_Arg1 = snapshot.fh_Arg1; packet.dp_Arg2 = (SIPTR)MKBADDR(&fib);
    send(context, &packet); assert(fib.fib_Size == sizeof(bytes));
    memset(&packet, 0, sizeof(packet)); packet.dp_Type = ACTION_END;
    packet.dp_Arg1 = snapshot.fh_Arg1; send(context, &packet);
    assert(afsplus_aros_packet_destroy(context) == 0 && live == 0);
    assert(afsplus_aros_unmount(fs) == 0);
    free(device.context);
}
int main(void)
{
    run_profile(AFSPLUS_AROS_MOUNT_READ_WRITE);
    run_profile(AFSPLUS_AROS_MOUNT_READ_ONLY);
    puts("real FFI + native snapshot packet: CoW mutation/unlink + read-only fallback + close PASS");
    return 0;
}
