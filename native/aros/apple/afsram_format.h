/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_AROS_APPLE_AFSRAM_FORMAT_H
#define AFSPLUS_AROS_APPLE_AFSRAM_FORMAT_H

#include <stdint.h>

#define AFSPLUS_AFSRAM_SECTOR_SIZE UINT32_C(512)
#define AFSPLUS_AFSRAM_ALIGNMENT UINT32_C(4096)
#define AFSPLUS_AFSRAM_HEADER_SIZE UINT32_C(4096)
#define AFSPLUS_AFSRAM_VERSION UINT32_C(2)
#define AFSPLUS_AFSRAM_FLAG_HANDLER UINT32_C(1)

struct AfsplusAfsRamPayload {
    uint64_t offset;
    uint64_t size;
    uint64_t handler_offset;
    uint64_t handler_size;
};

int afsplus_afsram_locate(
    const uint8_t *image,
    uint64_t image_size,
    struct AfsplusAfsRamPayload *payload);

#endif /* AFSPLUS_AROS_APPLE_AFSRAM_FORMAT_H */
