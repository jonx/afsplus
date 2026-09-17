/* SPDX-License-Identifier: BSD-2-Clause */
/* Cross-read probe for extended attributes (ADR-108). Exit 0 means the C
 * verdict equals the expectation on the command line; 1 means it differs; 2
 * means the probe itself could not run.
 *
 *   attribute_probe ref <block> reject
 *   attribute_probe ref <block> none
 *   attribute_probe ref <block> <first> <len> <count>
 *   attribute_probe fields <block> <security-first> <attribute-first>
 *                          <comment-size>
 *   attribute_probe seg <block> reject
 *   attribute_probe seg <block> <owner> <format> <version> <total> <index>
 *                       <count> <next> <generation> <bytes-file>
 *   attribute_probe set <set-file> reject
 *   attribute_probe set <set-file> <count> <listing-file>
 *
 * A listing is, per attribute: name size and value size as 16-bit
 * little-endian, then the name, then the value.
 */
#include "libafsplus_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BLOCK 4096u
#define SET_MAX 70000u

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

static int load(const char *path, uint8_t *block)
{
    size_t got;
    return slurp(path, block, BLOCK, &got) && got == BLOCK;
}

static int probe_reference(int argc, char **argv)
{
    uint8_t block[BLOCK];
    struct afspr_attribute_reference reference;
    size_t size;
    int status;

    if (!load(argv[2], block)) return 2;
    memset(&reference, 0xa5, sizeof(reference));
    status = afspr_decode_attribute_reference(block, BLOCK, &reference);
    for (size = 0u; size < BLOCK; ++size) {
        struct afspr_attribute_reference shorter;
        if (afspr_decode_attribute_reference(block, size, &shorter) ==
            AFSPR_OK) {
            return 1;
        }
    }
    if (strcmp(argv[3], "reject") == 0) {
        /* Outputs stay untouched on error. */
        return status != AFSPR_OK && reference.present == 0xa5a5a5a5u ? 0 : 1;
    }
    if (status != AFSPR_OK) return 1;
    if (strcmp(argv[3], "none") == 0) {
        return reference.present == 0u && reference.first_block == 0u &&
                       reference.total_len == 0u &&
                       reference.segment_count == 0u
                   ? 0
                   : 1;
    }
    if (argc != 6) return 2;
    return reference.present == 1u &&
                   reference.first_block == number(argv[3]) &&
                   reference.total_len == number(argv[4]) &&
                   reference.segment_count == number(argv[5])
               ? 0
               : 1;
}

/* The three optional fields of one record, through their own decoders. */
static int probe_fields(int argc, char **argv)
{
    uint8_t block[BLOCK];
    struct afspr_security_reference security;
    struct afspr_attribute_reference attributes;
    const uint8_t *comment;
    size_t comment_size;

    if (argc != 6 || !load(argv[2], block)) return 2;
    if (afspr_decode_security_reference(block, BLOCK, &security) != AFSPR_OK ||
        afspr_decode_attribute_reference(block, BLOCK, &attributes) !=
            AFSPR_OK ||
        afspr_decode_object_comment(block, BLOCK, &comment, &comment_size) !=
            AFSPR_OK) {
        return 1;
    }
    return security.first_block == number(argv[3]) &&
                   attributes.first_block == number(argv[4]) &&
                   comment_size == number(argv[5])
               ? 0
               : 1;
}

static int probe_segment(int argc, char **argv)
{
    uint8_t block[BLOCK], expected[BLOCK];
    struct afspr_security_segment segment, other;
    const uint8_t *bytes = NULL, *other_bytes;
    size_t size = 77u, expected_size, shorter, other_size;
    uint64_t generation = 99u, other_generation;
    int status;

    if (!load(argv[2], block)) return 2;
    memset(&segment, 0, sizeof(segment));
    segment.object_id = 999u;
    status = afspr_decode_attribute_segment(block, BLOCK, &segment, &bytes,
                                            &size, &generation);
    for (shorter = 0u; shorter < BLOCK; ++shorter) {
        if (afspr_decode_attribute_segment(block, shorter, &other,
                                           &other_bytes, &other_size,
                                           &other_generation) == AFSPR_OK) {
            return 1;
        }
    }
    /* One kind never reads as the other. */
    if (status == AFSPR_OK &&
        afspr_decode_security_segment(block, BLOCK, &other, &other_bytes,
                                      &other_size,
                                      &other_generation) == AFSPR_OK) {
        return 1;
    }
    if (strcmp(argv[3], "reject") == 0) {
        return status != AFSPR_OK && bytes == NULL && size == 77u &&
                       generation == 99u && segment.object_id == 999u
                   ? 0
                   : 1;
    }
    if (argc != 12 || status != AFSPR_OK) return argc != 12 ? 2 : 1;
    if (!slurp(argv[11], expected, sizeof(expected), &expected_size)) return 2;
    return segment.object_id == number(argv[3]) &&
                   segment.format == number(argv[4]) &&
                   segment.version == number(argv[5]) &&
                   segment.total_len == number(argv[6]) &&
                   segment.index == number(argv[7]) &&
                   segment.count == number(argv[8]) &&
                   segment.next == number(argv[9]) &&
                   generation == number(argv[10]) && bytes == block + 56u &&
                   size == expected_size &&
                   memcmp(bytes, expected, size) == 0
               ? 0
               : 1;
}

static int probe_set(int argc, char **argv)
{
    static uint8_t set[SET_MAX], expected[SET_MAX], listing[SET_MAX];
    struct afspr_attribute attribute;
    size_t size, expected_size, cursor = 0u, used = 0u, shorter;
    uint32_t count = 0xa5a5a5a5u, seen = 0u;
    int status, step;

    if (!slurp(argv[2], set, sizeof(set), &size)) return 2;
    status = afspr_validate_attribute_set(set, size, &count);
    if (strcmp(argv[3], "reject") == 0) {
        return status != AFSPR_OK && count == 0xa5a5a5a5u ? 0 : 1;
    }
    if (argc != 5 || status != AFSPR_OK) return argc != 5 ? 2 : 1;
    /* Admission is exact: no proper prefix of a set is a set. */
    for (shorter = 0u; shorter < size; ++shorter) {
        uint32_t other;
        if (afspr_validate_attribute_set(set, shorter, &other) == AFSPR_OK) {
            return 1;
        }
    }
    while ((step = afspr_attribute_set_next(set, size, &cursor, &attribute)) ==
           AFSPR_OK) {
        if (used + 4u + attribute.name_size + attribute.value_size >
            sizeof(listing)) {
            return 2;
        }
        listing[used++] = (uint8_t)(attribute.name_size & 0xffu);
        listing[used++] = (uint8_t)(attribute.name_size >> 8);
        listing[used++] = (uint8_t)(attribute.value_size & 0xffu);
        listing[used++] = (uint8_t)(attribute.value_size >> 8);
        memcpy(listing + used, attribute.name, attribute.name_size);
        used += attribute.name_size;
        memcpy(listing + used, attribute.value, attribute.value_size);
        used += attribute.value_size;
        ++seen;
    }
    if (step != AFSPR_ERR_NOT_FOUND || cursor != size) return 1;
    if (!slurp(argv[4], expected, sizeof(expected), &expected_size)) return 2;
    return count == number(argv[3]) && seen == count &&
                   used == expected_size &&
                   memcmp(listing, expected, used) == 0
               ? 0
               : 1;
}

int main(int argc, char **argv)
{
    if (argc < 4) return 2;
    if (strcmp(argv[1], "ref") == 0) return probe_reference(argc, argv);
    if (strcmp(argv[1], "fields") == 0) return probe_fields(argc, argv);
    if (strcmp(argv[1], "seg") == 0) return probe_segment(argc, argv);
    if (strcmp(argv[1], "set") == 0) return probe_set(argc, argv);
    return 2;
}
