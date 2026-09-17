/* SPDX-License-Identifier: BSD-2-Clause */
/* Cross-read probe for one extent-map item.
 *   extent_probe <key-hex> <value-hex> reject
 *   extent_probe <key-hex> <value-hex> <logical> <physical> <count> <flags>
 * Exit 0: the C verdict equals the expectation; 1: it differs; 2: usage. */
#include "libafsplus_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int unhex(const char *text, uint8_t *out, size_t capacity, size_t *size)
{
    size_t length = strlen(text), i;
    if (length % 2u != 0u || length / 2u > capacity) return 0;
    for (i = 0u; i < length / 2u; ++i) {
        unsigned value;
        if (sscanf(text + 2u * i, "%2x", &value) != 1) return 0;
        out[i] = (uint8_t)value;
    }
    *size = length / 2u;
    return 1;
}

int main(int argc, char **argv)
{
    uint8_t key[16], value[64];
    size_t key_size, value_size;
    struct afspr_extent extent;
    int status;

    if (argc < 4 || !unhex(argv[1], key, sizeof(key), &key_size) ||
        !unhex(argv[2], value, sizeof(value), &value_size)) {
        return 2;
    }
    memset(&extent, 0xa5, sizeof(extent));
    status = afspr_decode_extent_item(key, key_size, value, value_size, &extent);
    if (strcmp(argv[3], "reject") == 0) {
        return status != AFSPR_OK && extent.flags == 0xa5a5a5a5u ? 0 : 1;
    }
    if (argc != 7) return 2;
    return status == AFSPR_OK &&
                   extent.logical_start == strtoull(argv[3], NULL, 0) &&
                   extent.physical_start == strtoull(argv[4], NULL, 0) &&
                   extent.block_count == strtoull(argv[5], NULL, 0) &&
                   extent.flags == strtoull(argv[6], NULL, 0) &&
                   extent.reserved32 == 0u
               ? 0
               : 1;
}
