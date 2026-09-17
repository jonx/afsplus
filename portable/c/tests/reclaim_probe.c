/* SPDX-License-Identifier: BSD-2-Clause */
/* Cross-read probe for the reclaim queue blocks (ADR-036). Exit 0 means the
 * C verdict equals the expectation on the command line; 1 means it differs;
 * 2 means the probe itself could not run.
 *
 *   reclaim_probe root <block> reject
 *   reclaim_probe root <block> <generation> <pending> <appended> <reclaimed>
 *                      <head-segment> <head-entry> <head-block>
 *                      <inline-cap> <segment-cap> <table-cap>
 *                      <tables> <segments> <entries> <listing-file>
 *   reclaim_probe segment <block> reject
 *   reclaim_probe segment <block> <generation> <count> <listing-file>
 *   reclaim_probe table <block> reject
 *   reclaim_probe table <block> <generation> <count> <listing-file>
 *
 * A listing is every item in order, little-endian: a ref as block (64 bits)
 * and count (32); an entry as start (64), blocks (32), retire generation
 * (64). A root lists its tables, then its segments, then its entries.
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

static void put(uint8_t *out, size_t *used, uint64_t value, unsigned bytes)
{
    unsigned i;
    for (i = 0u; i < bytes; ++i) {
        out[(*used)++] = (uint8_t)(value >> (8u * i));
    }
}

static void list_refs(const uint8_t *area, uint32_t count, uint8_t *out,
                      size_t *used)
{
    uint32_t i;
    for (i = 0u; i < count; ++i) {
        struct afspr_reclaim_ref ref;
        afspr_reclaim_ref_at(area, i, &ref);
        put(out, used, ref.lba, 8u);
        put(out, used, ref.count, 4u);
    }
}

static void list_entries(const uint8_t *area, uint32_t count, uint8_t *out,
                         size_t *used)
{
    uint32_t i;
    for (i = 0u; i < count; ++i) {
        struct afspr_reclaim_entry entry;
        afspr_reclaim_entry_at(area, i, &entry);
        put(out, used, entry.start, 8u);
        put(out, used, entry.blocks, 4u);
        put(out, used, entry.retire_generation, 8u);
    }
}

static int same(const char *path, const uint8_t *listing, size_t used)
{
    static uint8_t expected[2u * BLOCK];
    size_t size;
    if (!slurp(path, expected, sizeof(expected), &size)) return 2;
    return size == used && memcmp(expected, listing, used) == 0 ? 0 : 1;
}

static int probe_root(int argc, char **argv, const uint8_t *block)
{
    static uint8_t listing[2u * BLOCK];
    struct afspr_reclaim_root root;
    uint64_t generation = 99u;
    size_t used = 0u, shorter;
    int status;

    memset(&root, 0xa5, sizeof(root));
    status = afspr_decode_reclaim_root(block, BLOCK, &root, &generation);
    for (shorter = 0u; shorter < BLOCK; ++shorter) {
        struct afspr_reclaim_root other;
        uint64_t other_generation;
        if (afspr_decode_reclaim_root(block, shorter, &other,
                                      &other_generation) == AFSPR_OK) {
            return 1;
        }
    }
    if (strcmp(argv[3], "reject") == 0) {
        return status != AFSPR_OK && generation == 99u &&
                       root.table_count == 0xa5a5a5a5u
                   ? 0
                   : 1;
    }
    if (argc != 17 || status != AFSPR_OK) return argc != 17 ? 2 : 1;
    if (generation != number(argv[3]) ||
        root.pending_blocks != number(argv[4]) ||
        root.appended_blocks_total != number(argv[5]) ||
        root.reclaimed_blocks_total != number(argv[6]) ||
        root.head_segment_offset != number(argv[7]) ||
        root.head_entry_offset != number(argv[8]) ||
        root.head_block_offset != number(argv[9]) ||
        root.inline_capacity != number(argv[10]) ||
        root.segment_capacity != number(argv[11]) ||
        root.table_capacity != number(argv[12]) ||
        root.table_count != number(argv[13]) ||
        root.segment_count != number(argv[14]) ||
        root.inline_count != number(argv[15])) {
        return 1;
    }
    list_refs(root.tables, root.table_count, listing, &used);
    list_refs(root.segments, root.segment_count, listing, &used);
    list_entries(root.inline_entries, root.inline_count, listing, &used);
    return same(argv[16], listing, used);
}

static int probe_sealed(int argc, char **argv, const uint8_t *block,
                        int is_segment)
{
    static uint8_t listing[2u * BLOCK];
    const uint8_t *area = block;
    uint32_t count = 77u;
    uint64_t generation = 99u;
    size_t used = 0u, shorter;
    int status;

    status = is_segment ? afspr_decode_reclaim_segment(block, BLOCK, &area,
                                                       &count, &generation)
                        : afspr_decode_reclaim_table(block, BLOCK, &area,
                                                     &count, &generation);
    for (shorter = 0u; shorter < BLOCK; ++shorter) {
        const uint8_t *other_area;
        uint32_t other_count;
        uint64_t other_generation;
        if ((is_segment
                 ? afspr_decode_reclaim_segment(block, shorter, &other_area,
                                                &other_count,
                                                &other_generation)
                 : afspr_decode_reclaim_table(block, shorter, &other_area,
                                              &other_count,
                                              &other_generation)) ==
            AFSPR_OK) {
            return 1;
        }
    }
    /* One kind never reads as the other. */
    if (status == AFSPR_OK) {
        const uint8_t *other_area;
        uint32_t other_count;
        uint64_t other_generation;
        if ((is_segment
                 ? afspr_decode_reclaim_table(block, BLOCK, &other_area,
                                              &other_count, &other_generation)
                 : afspr_decode_reclaim_segment(block, BLOCK, &other_area,
                                                &other_count,
                                                &other_generation)) ==
            AFSPR_OK) {
            return 1;
        }
    }
    if (strcmp(argv[3], "reject") == 0) {
        return status != AFSPR_OK && area == block && count == 77u &&
                       generation == 99u
                   ? 0
                   : 1;
    }
    if (argc != 6 || status != AFSPR_OK) return argc != 6 ? 2 : 1;
    if (generation != number(argv[3]) || count != number(argv[4])) return 1;
    if (is_segment) {
        list_entries(area, count, listing, &used);
    } else {
        list_refs(area, count, listing, &used);
    }
    return same(argv[5], listing, used);
}

int main(int argc, char **argv)
{
    static uint8_t block[BLOCK];
    size_t size;

    if (argc < 4) return 2;
    if (!slurp(argv[2], block, sizeof(block), &size) || size != BLOCK) return 2;
    if (strcmp(argv[1], "root") == 0) return probe_root(argc, argv, block);
    if (strcmp(argv[1], "segment") == 0) {
        return probe_sealed(argc, argv, block, 1);
    }
    if (strcmp(argv[1], "table") == 0) {
        return probe_sealed(argc, argv, block, 0);
    }
    return 2;
}
