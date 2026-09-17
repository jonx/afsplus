/* SPDX-License-Identifier: BSD-2-Clause */
/* Cross-read probe for exact object admission and the security preservation
 * container. Exit 0 means the C verdict equals the expectation on the command
 * line; 1 means it differs; 2 means the probe itself could not run.
 *
 *   security_probe ref <block> reject
 *   security_probe ref <block> none
 *   security_probe ref <block> <first> <len> <count> <flags>
 *   security_probe comment <block> reject
 *   security_probe comment <block> none
 *   security_probe comment <block> <bytes-file>
 *   security_probe seg <block> reject
 *   security_probe seg <block> <owner> <format> <version> <total> <index>
 *                      <count> <next> <generation> <bytes-file>
 *   security_probe lookup <image> <object-id> corrupt
 *   security_probe lookup <image> <object-id> <type> <flags> <protection>
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

static int load(const char *path, uint8_t *block)
{
    FILE *file = fopen(path, "rb");
    size_t got;
    if (file == NULL) return 0;
    got = fread(block, 1u, BLOCK, file);
    return fclose(file) == 0 && got == BLOCK;
}

static int read_blocks(void *ctx, uint64_t first, uint32_t count, void *dst)
{
    FILE *file = (FILE *)ctx;
    if (count != 1u || first > (uint64_t)(0x7fffffffL / (long)BLOCK) ||
        fseek(file, (long)(first * BLOCK), SEEK_SET) != 0 ||
        fread(dst, 1u, BLOCK, file) != BLOCK) {
        return -1;
    }
    return 0;
}

static int probe_reference(int argc, char **argv)
{
    uint8_t block[BLOCK];
    struct afspr_security_reference reference;
    size_t size;
    int status;

    if (!load(argv[2], block)) return 2;
    memset(&reference, 0xa5, sizeof(reference));
    status = afspr_decode_security_reference(block, BLOCK, &reference);
    for (size = 0u; size < BLOCK; ++size) {
        struct afspr_security_reference shorter;
        if (afspr_decode_security_reference(block, size, &shorter) ==
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
                       reference.segment_count == 0u && reference.flags == 0u
                   ? 0
                   : 1;
    }
    if (argc != 7) return 2;
    return reference.present == 1u &&
                   reference.first_block == number(argv[3]) &&
                   reference.total_len == number(argv[4]) &&
                   reference.segment_count == number(argv[5]) &&
                   reference.flags == number(argv[6])
               ? 0
               : 1;
}

static int probe_comment(char **argv)
{
    static uint8_t block[BLOCK], expected[BLOCK];
    const uint8_t *comment = block; /* poisoned: must stay on error */
    size_t size = 77u, expected_size, shorter;
    FILE *file;
    int status;

    if (!load(argv[2], block)) return 2;
    status = afspr_decode_object_comment(block, BLOCK, &comment, &size);
    for (shorter = 0u; shorter < BLOCK; ++shorter) {
        const uint8_t *other;
        size_t other_size;
        if (afspr_decode_object_comment(block, shorter, &other, &other_size) ==
            AFSPR_OK) {
            return 1;
        }
    }
    if (strcmp(argv[3], "reject") == 0) {
        return status != AFSPR_OK && comment == block && size == 77u ? 0 : 1;
    }
    if (status != AFSPR_OK) return 1;
    if (strcmp(argv[3], "none") == 0) {
        return comment == NULL && size == 0u ? 0 : 1;
    }
    file = fopen(argv[3], "rb");
    if (file == NULL) return 2;
    expected_size = fread(expected, 1u, sizeof(expected), file);
    if (fclose(file) != 0) return 2;
    return comment != NULL && size == expected_size &&
                   memcmp(comment, expected, size) == 0
               ? 0
               : 1;
}

static int probe_segment(int argc, char **argv)
{
    uint8_t block[BLOCK], expected[BLOCK];
    struct afspr_security_segment segment;
    const uint8_t *bytes = NULL;
    size_t size = 77u, expected_size, shorter;
    uint64_t generation = 99u;
    FILE *file;
    int status;

    if (!load(argv[2], block)) return 2;
    memset(&segment, 0, sizeof(segment));
    segment.object_id = 999u;
    status = afspr_decode_security_segment(block, BLOCK, &segment, &bytes,
                                           &size, &generation);
    for (shorter = 0u; shorter < BLOCK; ++shorter) {
        struct afspr_security_segment other;
        const uint8_t *other_bytes;
        size_t other_size;
        uint64_t other_generation;
        if (afspr_decode_security_segment(block, shorter, &other,
                                          &other_bytes, &other_size,
                                          &other_generation) == AFSPR_OK) {
            return 1;
        }
    }
    if (strcmp(argv[3], "reject") == 0) {
        return status != AFSPR_OK && bytes == NULL && size == 77u &&
                       generation == 99u && segment.object_id == 999u
                   ? 0
                   : 1;
    }
    if (argc != 12 || status != AFSPR_OK) return argc != 12 ? 2 : 1;
    file = fopen(argv[11], "rb");
    if (file == NULL) return 2;
    expected_size = fread(expected, 1u, sizeof(expected), file);
    if (fclose(file) != 0) return 2;
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

static int probe_lookup(int argc, char **argv)
{
    static uint8_t scratch[AFSPR_INTENT_SCRATCH_SIZE];
    struct afspr_block_ops ops;
    struct afspr_scratch space;
    struct afspr_probe_result volume;
    struct afspr_object object;
    FILE *file = fopen(argv[2], "rb");
    long end;
    int status;

    if (file == NULL || fseek(file, 0, SEEK_END) != 0) return 2;
    end = ftell(file);
    if (end <= 0 || (unsigned long)end % BLOCK != 0u) return 2;
    memset(&ops, 0, sizeof(ops));
    ops.abi_version = AFSPR_ABI_VERSION;
    ops.struct_size = (uint32_t)sizeof(ops);
    ops.ctx = file;
    ops.read_blocks = read_blocks;
    ops.block_count = (uint64_t)end / BLOCK;
    ops.block_size = BLOCK;
    space.buffer = scratch;
    space.size = sizeof(scratch);
    if (afspr_probe(&ops, &space, &volume, sizeof(volume)) != AFSPR_OK) {
        (void)fclose(file);
        return strcmp(argv[4], "unmountable") == 0 ? 0 : 2;
    }
    status = afspr_lookup_object(&ops, &space, &volume, number(argv[3]),
                                 &object, sizeof(object), NULL, 0u);
    (void)fclose(file);
    if (strcmp(argv[4], "corrupt") == 0) {
        return status == AFSPR_ERR_CORRUPT ? 0 : 1;
    }
    if (argc != 7) return 2;
    /* A symlink is structurally admitted and reported as unsupported by the
     * volume lookup, exactly as before the container. */
    if (number(argv[4]) == AFSPR_OBJECT_SYMLINK) {
        return status == AFSPR_ERR_UNSUPPORTED ? 0 : 1;
    }
    return status == AFSPR_OK && object.type == number(argv[4]) &&
                   object.flags == number(argv[5]) &&
                   object.protection == number(argv[6])
               ? 0
               : 1;
}

int main(int argc, char **argv)
{
    if (argc < 4) return 2;
    if (strcmp(argv[1], "ref") == 0) return probe_reference(argc, argv);
    if (strcmp(argv[1], "comment") == 0) return probe_comment(argv);
    if (strcmp(argv[1], "seg") == 0) return probe_segment(argc, argv);
    if (strcmp(argv[1], "lookup") == 0 && argc >= 5) {
        return probe_lookup(argc, argv);
    }
    return 2;
}
