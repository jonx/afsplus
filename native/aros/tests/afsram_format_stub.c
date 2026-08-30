/* SPDX-License-Identifier: BSD-2-Clause */

#include "afsram_format.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define FAT_BYTES (2880U * 512U)
#define HANDLER_OFFSET (FAT_BYTES + 4096U)
#define HANDLER_BYTES 1024U
#define PAYLOAD_OFFSET (FAT_BYTES + 8192U)
#define FIXTURE_BYTES (PAYLOAD_OFFSET + 8192U)

static void store_le16(uint8_t *bytes, uint16_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
}

static void store_le32(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
}

static void store_le64(uint8_t *bytes, uint64_t value)
{
    store_le32(bytes, (uint32_t)value);
    store_le32(bytes + 4, (uint32_t)(value >> 32));
}

static int expect_rejected(
    uint8_t *image, uint64_t image_size, const char *label)
{
    struct AfsplusAfsRamPayload payload;

    if (afsplus_afsram_locate(image, image_size, &payload)) {
        fprintf(stderr, "afsram format stub: accepted %s\n", label);
        return 0;
    }
    return 1;
}

int main(void)
{
    static uint8_t image[FIXTURE_BYTES];
    struct AfsplusAfsRamPayload payload;
    uint8_t saved;

    store_le16(image + 11, 512);
    store_le16(image + 19, 2880);
    image[510] = 0x55;
    image[511] = 0xaa;
    memcpy(image + FAT_BYTES, "AFSPRAM\0", 8);
    store_le32(image + FAT_BYTES + 8, AFSPLUS_AFSRAM_VERSION);
    store_le32(image + FAT_BYTES + 12, 4096);
    store_le64(image + FAT_BYTES + 16, 8192);
    store_le32(image + FAT_BYTES + 24, 512);
    store_le32(image + FAT_BYTES + 28, AFSPLUS_AFSRAM_FLAG_HANDLER);
    store_le64(image + FAT_BYTES + 32, HANDLER_OFFSET);
    store_le64(image + FAT_BYTES + 40, HANDLER_BYTES);
    image[HANDLER_OFFSET] = 0x7f;

    if (!afsplus_afsram_locate(image, sizeof(image), &payload) ||
        payload.offset != PAYLOAD_OFFSET || payload.size != 8192U ||
        payload.handler_offset != HANDLER_OFFSET ||
        payload.handler_size != HANDLER_BYTES) {
        fputs("afsram format stub: valid fixture rejected\n", stderr);
        return 1;
    }
    if (!expect_rejected(image, sizeof(image) - 1U, "truncated payload"))
        return 1;
    saved = image[FAT_BYTES];
    image[FAT_BYTES] ^= 1U;
    if (!expect_rejected(image, sizeof(image), "bad descriptor magic"))
        return 1;
    image[FAT_BYTES] = saved;
    image[FAT_BYTES + 48] = 1;
    if (!expect_rejected(image, sizeof(image), "nonzero reserved header"))
        return 1;
    image[FAT_BYTES + 48] = 0;
    store_le64(image + FAT_BYTES + 16, 4096);
    if (!expect_rejected(image, sizeof(image), "trailing payload bytes"))
        return 1;

    puts("afsram format stub: PASS");
    return 0;
}
