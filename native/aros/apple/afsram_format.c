/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsram_format.h"

#include <stddef.h>

static const uint8_t afsram_magic[8] = {
    'A', 'F', 'S', 'P', 'R', 'A', 'M', 0
};

static uint16_t load_le16(const uint8_t *bytes)
{
    return (uint16_t)bytes[0] | (uint16_t)((uint16_t)bytes[1] << 8);
}

static uint32_t load_le32(const uint8_t *bytes)
{
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) |
        ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}

static uint64_t load_le64(const uint8_t *bytes)
{
    return (uint64_t)load_le32(bytes) |
        ((uint64_t)load_le32(bytes + 4) << 32);
}

static int bytes_equal(const uint8_t *left, const uint8_t *right, size_t size)
{
    size_t index;

    for (index = 0; index < size; ++index) {
        if (left[index] != right[index])
            return 0;
    }
    return 1;
}

static int bytes_zero(const uint8_t *bytes, uint64_t size)
{
    uint64_t index;

    for (index = 0; index < size; ++index) {
        if (bytes[index] != 0)
            return 0;
    }
    return 1;
}

static int align_up(uint64_t value, uint64_t alignment, uint64_t *result)
{
    uint64_t mask = alignment - 1U;

    if (value > UINT64_MAX - mask)
        return 0;
    *result = (value + mask) & ~mask;
    return 1;
}

int afsplus_afsram_locate(
    const uint8_t *image,
    uint64_t image_size,
    struct AfsplusAfsRamPayload *payload)
{
    const uint8_t *header;
    uint64_t fat_bytes;
    uint64_t header_offset;
    uint64_t payload_offset;
    uint64_t payload_size;
    uint64_t handler_offset;
    uint64_t handler_size;
    uint64_t handler_end;
    uint32_t total_sectors;
    uint16_t total_sectors16;

    if (image == NULL || payload == NULL || image_size < 512U ||
        load_le16(image + 11) != AFSPLUS_AFSRAM_SECTOR_SIZE ||
        image[510] != 0x55 || image[511] != 0xaa)
        return 0;

    total_sectors16 = load_le16(image + 19);
    total_sectors = total_sectors16 != 0 ? total_sectors16 :
        load_le32(image + 32);
    if (total_sectors == 0)
        return 0;
    fat_bytes = (uint64_t)total_sectors * AFSPLUS_AFSRAM_SECTOR_SIZE;
    if (!align_up(fat_bytes, AFSPLUS_AFSRAM_ALIGNMENT, &header_offset) ||
        header_offset > image_size ||
        AFSPLUS_AFSRAM_HEADER_SIZE > image_size - header_offset)
        return 0;

    header = image + header_offset;
    if (!bytes_equal(header, afsram_magic, sizeof(afsram_magic)) ||
        load_le32(header + 8) != AFSPLUS_AFSRAM_VERSION ||
        load_le32(header + 12) != AFSPLUS_AFSRAM_HEADER_SIZE ||
        load_le32(header + 24) != AFSPLUS_AFSRAM_SECTOR_SIZE ||
        load_le32(header + 28) != AFSPLUS_AFSRAM_FLAG_HANDLER ||
        !bytes_zero(header + 48, AFSPLUS_AFSRAM_HEADER_SIZE - 48U))
        return 0;

    payload_size = load_le64(header + 16);
    handler_offset = load_le64(header + 32);
    handler_size = load_le64(header + 40);
    if (handler_offset != header_offset + AFSPLUS_AFSRAM_HEADER_SIZE ||
        handler_size == 0 || handler_size > UINT32_MAX ||
        handler_offset > UINT64_MAX - handler_size)
        return 0;
    handler_end = handler_offset + handler_size;
    if (!align_up(handler_end, AFSPLUS_AFSRAM_ALIGNMENT, &payload_offset) ||
        handler_end > image_size || payload_offset > image_size ||
        !bytes_zero(image + handler_end, payload_offset - handler_end))
        return 0;
    if (payload_size == 0 ||
        (payload_size & (AFSPLUS_AFSRAM_ALIGNMENT - 1U)) != 0 ||
        payload_size > UINT32_MAX || payload_offset > image_size ||
        payload_size != image_size - payload_offset)
        return 0;

    payload->offset = payload_offset;
    payload->size = payload_size;
    payload->handler_offset = handler_offset;
    payload->handler_size = handler_size;
    return 1;
}
