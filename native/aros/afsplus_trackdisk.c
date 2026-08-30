/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsplus_trackdisk.h"

#include <aros/stdc/string.h>

static int32_t checked_multiply(uint64_t first, uint64_t second,
    uint64_t *result)
{
    if (first != 0 && second > UINT64_MAX / first)
        return ERROR_OBJECT_TOO_LARGE;
    *result = first * second;
    return 0;
}

int32_t afsplus_aros_trackdisk_geometry(uint64_t low_cylinder,
    uint64_t high_cylinder, uint64_t surfaces,
    uint64_t blocks_per_track, uint64_t size_block_longwords,
    uint64_t *partition_start_bytes, uint64_t *partition_length_bytes)
{
    uint64_t bytes_per_block;
    uint64_t blocks_per_cylinder;
    uint64_t start_blocks;
    uint64_t cylinder_count;
    uint64_t length_blocks;
    int32_t error;

    if (partition_start_bytes == NULL || partition_length_bytes == NULL
        || high_cylinder < low_cylinder || surfaces == 0
        || blocks_per_track == 0 || size_block_longwords == 0)
        return ERROR_BAD_NUMBER;
    error = checked_multiply(size_block_longwords, 4, &bytes_per_block);
    if (error == 0)
        error = checked_multiply(surfaces, blocks_per_track,
            &blocks_per_cylinder);
    if (error == 0)
        error = checked_multiply(low_cylinder, blocks_per_cylinder,
            &start_blocks);
    cylinder_count = high_cylinder - low_cylinder + 1;
    if (cylinder_count == 0)
        error = ERROR_OBJECT_TOO_LARGE;
    if (error == 0)
        error = checked_multiply(cylinder_count, blocks_per_cylinder,
            &length_blocks);
    if (error == 0)
        error = checked_multiply(start_blocks, bytes_per_block,
            partition_start_bytes);
    if (error == 0)
        error = checked_multiply(length_blocks, bytes_per_block,
            partition_length_bytes);
    return error;
}

static int32_t validate_access(const struct AfsplusArosTrackdisk *trackdisk,
    uint64_t lba, uint32_t length, uint64_t *offset)
{
    uint64_t relative;
    uint64_t ending;

    if (length != trackdisk->logical_block_size
        || lba >= trackdisk->total_blocks)
        return ERROR_BAD_NUMBER;
    if (checked_multiply(lba, trackdisk->logical_block_size, &relative) != 0
        || relative > UINT64_MAX - trackdisk->partition_start_bytes)
        return ERROR_OBJECT_TOO_LARGE;
    *offset = trackdisk->partition_start_bytes + relative;
    if (*offset > UINT64_MAX - length)
        return ERROR_OBJECT_TOO_LARGE;
    ending = *offset + length;
    if (ending > trackdisk->partition_start_bytes
            + trackdisk->partition_length_bytes
        || (!trackdisk->supports_64bit_offsets && ending > UINT64_C(0x100000000)))
        return ERROR_OBJECT_TOO_LARGE;
    return 0;
}

static int32_t read_block(void *context, uint64_t lba,
    uint8_t *destination, uint32_t length)
{
    struct AfsplusArosTrackdisk *trackdisk = context;
    uint64_t offset;
    int32_t error;

    if (trackdisk == NULL || (destination == NULL && length != 0))
        return ERROR_BAD_NUMBER;
    error = validate_access(trackdisk, lba, length, &offset);
    if (error != 0)
        return error;
    return trackdisk->transfer(trackdisk->context, trackdisk->read_command,
        offset, destination, length, 0);
}

static int32_t write_block(void *context, uint64_t lba,
    const uint8_t *source, uint32_t length)
{
    struct AfsplusArosTrackdisk *trackdisk = context;
    uint64_t offset;
    int32_t error;

    if (trackdisk == NULL || (source == NULL && length != 0))
        return ERROR_BAD_NUMBER;
    if (trackdisk->read_only)
        return ERROR_DISK_WRITE_PROTECTED;
    error = validate_access(trackdisk, lba, length, &offset);
    if (error != 0)
        return error;
    return trackdisk->transfer(trackdisk->context, trackdisk->write_command,
        offset, (void *)source, length, 1);
}

static int32_t flush_device(void *context)
{
    struct AfsplusArosTrackdisk *trackdisk = context;

    if (trackdisk == NULL || trackdisk->sync == NULL)
        return ERROR_BAD_NUMBER;
    if (trackdisk->read_only)
        return ERROR_DISK_WRITE_PROTECTED;
    return trackdisk->sync(trackdisk->context);
}

int32_t afsplus_aros_trackdisk_init(
    const struct AfsplusArosTrackdiskConfig *config,
    struct AfsplusArosTrackdisk *trackdisk,
    struct AfsplusArosDevice *device)
{
    uint64_t partition_end;

    if (config == NULL || trackdisk == NULL || device == NULL
        || config->abi_version != AFSPLUS_AROS_TRACKDISK_ABI_VERSION
        || config->struct_size != sizeof(*config) || config->transfer == NULL
        || config->partition_length_bytes == 0
        || config->device_block_size == 0
        || config->logical_block_size != AFSPLUS_AROS_ALPHA0_BLOCK_SIZE
        || config->logical_block_size % config->device_block_size != 0
        || config->partition_start_bytes % config->device_block_size != 0
        || config->partition_length_bytes % config->device_block_size != 0
        || config->partition_length_bytes % config->logical_block_size != 0
        || config->read_command == 0
        || (!config->read_only
            && (config->write_command == 0 || config->sync == NULL)))
        return ERROR_BAD_NUMBER;
    if (config->partition_start_bytes
        > UINT64_MAX - config->partition_length_bytes)
        return ERROR_OBJECT_TOO_LARGE;
    partition_end = config->partition_start_bytes
        + config->partition_length_bytes;
    if (!config->supports_64bit_offsets
        && partition_end > UINT64_C(0x100000000))
        return ERROR_OBJECT_TOO_LARGE;

    memset(trackdisk, 0, sizeof(*trackdisk));
    trackdisk->context = config->context;
    trackdisk->transfer = config->transfer;
    trackdisk->sync = config->sync;
    trackdisk->partition_start_bytes = config->partition_start_bytes;
    trackdisk->partition_length_bytes = config->partition_length_bytes;
    trackdisk->total_blocks = config->partition_length_bytes
        / config->logical_block_size;
    trackdisk->logical_block_size = config->logical_block_size;
    trackdisk->read_command = config->read_command;
    trackdisk->write_command = config->write_command;
    trackdisk->supports_64bit_offsets = !!config->supports_64bit_offsets;
    trackdisk->read_only = !!config->read_only;

    memset(device, 0, sizeof(*device));
    device->abi_version = AFSPLUS_AROS_ABI_VERSION;
    device->struct_size = sizeof(*device);
    device->context = trackdisk;
    device->block_size = trackdisk->logical_block_size;
    device->total_blocks = trackdisk->total_blocks;
    device->read_block = read_block;
    if (!trackdisk->read_only)
    {
        device->write_block = write_block;
        device->flush = flush_device;
    }
    return 0;
}
