/* SPDX-License-Identifier: BSD-2-Clause */
/* Cross-read probe for one checkpoint block, with and without the snapshot
 * roots of ADR-073. Exit 0 means the C verdict equals the expectation on the
 * command line; 1 means it differs; 2 means the probe could not run.
 *
 *   checkpoint_probe <block> <uuid-hex> reject
 *   checkpoint_probe <block> <uuid-hex> <generation> <object-map>
 *                    <allocation-root> <reclaim-root> <next-object>
 *                    <transaction> <free-blocks> <shared-root>
 *                    <has-snapshot-roots> <registry> <lifetimes> <label-file>
 */
#include "libafsplus_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BLOCK 4096u

static uint64_t number(const char *text)
{
    return (uint64_t)strtoull(text, NULL, 0);
}

static int slurp(const char *path, uint8_t *buffer, size_t capacity,
                 size_t *size)
{
    FILE *file = fopen(path, "rb");
    if (file == NULL) return 0;
    *size = fread(buffer, 1u, capacity, file);
    return fclose(file) == 0;
}

static int nibble(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
}

int main(int argc, char **argv)
{
    static uint8_t block[BLOCK];
    uint8_t uuid[16], label[BLOCK];
    struct afspr_checkpoint_view view;
    size_t size, label_size, shorter, i;
    int status;

    if (argc < 4 || strlen(argv[2]) != 32u) return 2;
    for (i = 0u; i < 16u; ++i) {
        int high = nibble(argv[2][2u * i]), low = nibble(argv[2][2u * i + 1u]);
        if (high < 0 || low < 0) return 2;
        uuid[i] = (uint8_t)(high * 16 + low);
    }
    if (!slurp(argv[1], block, sizeof(block), &size) || size != BLOCK) return 2;
    memset(&view, 0xa5, sizeof(view));
    status = afspr_decode_checkpoint_block(block, BLOCK, uuid, &view);
    for (shorter = 0u; shorter < BLOCK; ++shorter) {
        struct afspr_checkpoint_view other;
        if (afspr_decode_checkpoint_block(block, shorter, uuid, &other) ==
            AFSPR_OK) {
            return 1;
        }
    }
    if (strcmp(argv[3], "reject") == 0) {
        /* The view stays untouched on error. */
        return status != AFSPR_OK && view.has_snapshot_roots == 0xa5a5a5a5u
                   ? 0
                   : 1;
    }
    if (argc != 15 || status != AFSPR_OK) return argc != 15 ? 2 : 1;
    if (!slurp(argv[14], label, sizeof(label), &label_size)) return 2;
    return view.generation == number(argv[3]) &&
                   view.object_map_block == number(argv[4]) &&
                   view.allocation_root_block == number(argv[5]) &&
                   view.reclaim_root_block == number(argv[6]) &&
                   view.next_object_id == number(argv[7]) &&
                   view.committed_tx_id == number(argv[8]) &&
                   view.free_blocks_total == number(argv[9]) &&
                   view.shared_extent_root_block == number(argv[10]) &&
                   view.has_snapshot_roots == number(argv[11]) &&
                   view.snapshot_registry_block == number(argv[12]) &&
                   view.snapshot_lifetimes_block == number(argv[13]) &&
                   view.label_len == label_size &&
                   memcmp(view.label, label, label_size) == 0
               ? 0
               : 1;
}
