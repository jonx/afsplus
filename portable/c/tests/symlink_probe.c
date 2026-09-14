/* SPDX-License-Identifier: BSD-2-Clause */
#include "libafsplus_reader.h"
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv)
{
    uint8_t block[4096], expected[4096];
    const uint8_t *target = NULL;
    size_t size = 123u, expected_size, i;
    uint64_t generation = 99u;
    struct afspr_object object;
    FILE *file;
    int status;
    if (argc != 3) return 2;
    file = fopen(argv[1], "rb");
    if (file == NULL) return 2;
    if (fread(block, 1u, sizeof(block), file) != sizeof(block)) return 2;
    if (fclose(file) != 0) return 2;
    memset(&object, 0, sizeof(object));
    object.object_id = 999u;
    status = afspr_decode_symlink_record(block, sizeof(block), &object,
                                         &target, &size, &generation);
    if (strcmp(argv[2], "reject") == 0) {
        return status != AFSPR_OK && target == NULL && size == 123u &&
               generation == 99u && object.object_id == 999u ? 0 : 1;
    }
    file = fopen(argv[2], "rb");
    if (file == NULL) return 2;
    expected_size = fread(expected, 1u, sizeof(expected), file);
    if (fclose(file) != 0) return 2;
    if (status != AFSPR_OK || target != block + 128u ||
        size != expected_size || memcmp(target, expected, size) != 0 ||
        generation != 7u || object.object_id != 16u ||
        object.size_bytes != expected_size || object.link_count != 1u ||
        object.type != AFSPR_OBJECT_SYMLINK || object.flags != 0u ||
        object.protection != 123u || object.content_generation != 5u ||
        object.created.seconds != -3 || object.created.nanoseconds != 999999999u ||
        object.modified.seconds != -3 || object.changed.seconds != -3 ||
        object.allocated_bytes != 0u || object.data_root != 0u || object.data_blocks != 0u)
        return 1;
    for (i = 0u; i < sizeof(block); ++i) {
        if (afspr_decode_symlink_record(block, i, &object, &target,
                                       &size, &generation) == AFSPR_OK) return 1;
    }
    return 0;
}
