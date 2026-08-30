/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_trackdisk.h"

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint64_t last_offset;
static uint32_t last_command;
static uint32_t last_length;
static uint32_t last_writing;
static uint32_t sync_count;
static uint32_t transfer_count;
static int32_t transfer_result;
static struct afsp_io_activity_event activity_events[8];
static uint32_t activity_count;

void __assert(const char *expression, const char *file, unsigned int line)
{
    printf("assertion failed: %s (%s:%u)\n", expression, file, line);
    fflush(NULL);
    abort();
}

static int32_t transfer(void *context, uint32_t command,
    uint64_t byte_offset, void *buffer, uint32_t length, uint32_t writing)
{
    uint8_t *bytes = buffer;

    assert(context == (void *)(uintptr_t)UINT32_C(0x1234));
    assert(buffer != NULL);
    last_command = command;
    last_offset = byte_offset;
    last_length = length;
    last_writing = writing;
    transfer_count++;
    if (!writing)
        memset(bytes, 0x5a, length);
    return transfer_result;
}

static int32_t sync_device(void *context)
{
    assert(context == (void *)(uintptr_t)UINT32_C(0x1234));
    sync_count++;
    return 0;
}

static void activity(void *context,
    const struct afsp_io_activity_event *event)
{
    assert(context == (void *)(uintptr_t)UINT32_C(0x5678));
    assert(event != NULL && event->size == sizeof(*event));
    assert(event->version == AFSP_IO_ACTIVITY_EVENT_VERSION);
    assert(activity_count < 8);
    activity_events[activity_count++] = *event;
}

int main(void)
{
    struct AfsplusArosTrackdiskConfig config;
    struct AfsplusArosTrackdisk trackdisk;
    struct AfsplusArosDevice device;
    uint64_t start;
    uint64_t length;
    uint8_t block[4096];

    assert(afsplus_aros_trackdisk_geometry(2, 5, 4, 16, 128,
        &start, &length) == 0);
    assert(start == 65536);
    assert(length == 131072);
    assert(afsplus_aros_trackdisk_geometry(5, 2, 4, 16, 128,
        &start, &length) == ERROR_BAD_NUMBER);
    assert(afsplus_aros_trackdisk_geometry(0, UINT64_MAX, 2, 2, 128,
        &start, &length) == ERROR_OBJECT_TOO_LARGE);

    memset(&config, 0, sizeof(config));
    config.abi_version = AFSPLUS_AROS_TRACKDISK_ABI_VERSION;
    config.struct_size = sizeof(config);
    config.context = (void *)(uintptr_t)UINT32_C(0x1234);
    config.transfer = transfer;
    config.sync = sync_device;
    /* A legacy partition need only start on a physical sector, not on a
     * filesystem-logical-block boundary. */
    config.partition_start_bytes = 512;
    config.partition_length_bytes = 8192;
    config.device_block_size = 512;
    config.logical_block_size = sizeof(block);
    config.read_command = 2;
    config.write_command = 3;
    config.supports_64bit_offsets = 1;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device) == 0);
    assert(device.block_size == sizeof(block));
    assert(device.total_blocks == 2);
    assert(device.context == &trackdisk);
    assert(device.read_block != NULL && device.write_block != NULL
        && device.flush != NULL);

    memset(block, 0, sizeof(block));
    assert(device.read_block(device.context, 1, block, sizeof(block)) == 0);
    assert(last_command == 2 && last_offset == 4608
        && last_length == sizeof(block) && !last_writing);
    assert(block[0] == 0x5a && block[sizeof(block) - 1] == 0x5a);
    assert(device.write_block(device.context, 0, block, sizeof(block)) == 0);
    assert(last_command == 3 && last_offset == 512 && last_writing);
    assert(device.flush(device.context) == 0 && sync_count == 1);
    assert(device.read_block(device.context, 2, block, sizeof(block))
        == ERROR_BAD_NUMBER);
    assert(transfer_count == 2);
    assert(device.read_block(device.context, 0, block, sizeof(block) - 1)
        == ERROR_BAD_NUMBER);
    assert(transfer_count == 2);
    transfer_result = 17;
    assert(device.read_block(device.context, 0, block, sizeof(block)) == 17);
    assert(transfer_count == 3);
    transfer_result = 0;

    config.logical_block_size = 2048;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device)
        == ERROR_BAD_NUMBER);
    config.logical_block_size = sizeof(block);
    config.device_block_size = 8192;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device)
        == ERROR_BAD_NUMBER);
    config.device_block_size = 512;
    config.read_command = 0;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device)
        == ERROR_BAD_NUMBER);
    config.read_command = 2;

    config.partition_start_bytes = UINT64_C(0x100000000);
    config.supports_64bit_offsets = 0;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device)
        == ERROR_OBJECT_TOO_LARGE);
    config.supports_64bit_offsets = 1;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device) == 0);
    assert(device.read_block(device.context, 0, block, sizeof(block)) == 0);
    assert(last_offset == UINT64_C(0x100000000));

    config.partition_start_bytes = UINT64_C(0xfffff000);
    config.supports_64bit_offsets = 0;
    config.partition_length_bytes = 4096;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device) == 0);
    assert(device.read_block(device.context, 0, block, sizeof(block)) == 0);
    assert(last_offset == UINT64_C(0xfffff000));

    config.partition_start_bytes = 0;
    config.partition_length_bytes = 8192;
    config.read_only = 0;
    config.sync = sync_device;
    config.supports_64bit_offsets = 1;
    config.activity.emit = activity;
    config.activity.ctx = (void *)(uintptr_t)UINT32_C(0x5678);
    config.activity.operation_mask = AFSP_IO_ACTIVITY_MASK_WRITE
        | AFSP_IO_ACTIVITY_MASK_FLUSH;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device) == 0);
    assert(device.read_block(device.context, 0, block, sizeof(block)) == 0);
    assert(activity_count == 0);
    assert(device.write_block(device.context, 1, block, sizeof(block)) == 0);
    assert(device.flush(device.context) == 0);
    assert(activity_count == 4);
    assert(activity_events[0].operation == AFSP_IO_ACTIVITY_WRITE
        && activity_events[0].phase == AFSP_IO_ACTIVITY_BEGIN
        && activity_events[0].flags == AFSP_IO_ACTIVITY_LBA_VALID
        && activity_events[0].lba == 1 && activity_events[0].block_count == 1);
    assert(activity_events[1].phase == AFSP_IO_ACTIVITY_END
        && (activity_events[1].flags & AFSP_IO_ACTIVITY_SUCCESS) != 0);
    assert(activity_events[2].operation == AFSP_IO_ACTIVITY_FLUSH
        && activity_events[2].phase == AFSP_IO_ACTIVITY_BEGIN
        && activity_events[2].flags == 0
        && activity_events[2].block_count == 0);
    assert(activity_events[3].phase == AFSP_IO_ACTIVITY_END
        && activity_events[3].flags == AFSP_IO_ACTIVITY_SUCCESS);
    transfer_result = 17;
    assert(device.write_block(device.context, 0, block, sizeof(block)) == 17);
    assert(activity_count == 6);
    assert(activity_events[4].phase == AFSP_IO_ACTIVITY_BEGIN);
    assert(activity_events[5].phase == AFSP_IO_ACTIVITY_END
        && (activity_events[5].flags & AFSP_IO_ACTIVITY_SUCCESS) == 0);
    transfer_result = 0;

    config.activity.emit = NULL;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device)
        == ERROR_BAD_NUMBER);

    config.read_only = 1;
    config.sync = NULL;
    config.activity.emit = NULL;
    config.activity.ctx = NULL;
    config.activity.operation_mask = 0;
    assert(afsplus_aros_trackdisk_init(&config, &trackdisk, &device) == 0);
    assert(device.read_block != NULL);
    assert(device.write_block == NULL && device.flush == NULL);

    puts("afsplus trackdisk stub: PASS");
    return 0;
}
