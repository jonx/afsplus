/* SPDX-License-Identifier: BSD-2-Clause */

#include "libafsplus_reader.h"

#include <limits.h>
#include <stdio.h>
#include <string.h>

#define EXAMPLE_BLOCK_SIZE 4096u

struct file_reader {
    FILE *file;
};

static int read_blocks(void *ctx, uint64_t first_block, uint32_t count,
                       void *dst)
{
    struct file_reader *reader = (struct file_reader *)ctx;
    uint64_t offset = first_block * EXAMPLE_BLOCK_SIZE;
    size_t bytes = (size_t)count * EXAMPLE_BLOCK_SIZE;

    if (count == 0u || bytes / EXAMPLE_BLOCK_SIZE != (size_t)count ||
        first_block > (uint64_t)LONG_MAX / EXAMPLE_BLOCK_SIZE ||
        offset > (uint64_t)LONG_MAX) {
        return -1;
    }
    if (fseek(reader->file, (long)offset, SEEK_SET) != 0) {
        return -1;
    }
    return fread(dst, 1, bytes, reader->file) == bytes ? 0 : -1;
}

int main(int argc, char **argv)
{
    struct file_reader reader;
    struct afspr_block_ops ops;
    struct afspr_scratch scratch;
    struct afspr_probe_result result;
    struct afspr_diagnostic diagnostic;
    uint8_t block[EXAMPLE_BLOCK_SIZE];
    long bytes;
    int status;

    if (argc != 2) {
        fprintf(stderr, "usage: %s <afsplus-image>\n", argv[0]);
        return 2;
    }
    reader.file = fopen(argv[1], "rb");
    if (reader.file == NULL || fseek(reader.file, 0, SEEK_END) != 0 ||
        (bytes = ftell(reader.file)) <= 0 ||
        bytes % EXAMPLE_BLOCK_SIZE != 0) {
        fprintf(stderr, "%s: cannot determine an aligned image size\n",
                argv[1]);
        if (reader.file != NULL) {
            fclose(reader.file);
        }
        return 1;
    }

    memset(&ops, 0, sizeof(ops));
    ops.abi_version = AFSPR_ABI_VERSION;
    ops.struct_size = (uint32_t)sizeof(ops);
    ops.ctx = &reader;
    ops.read_blocks = read_blocks;
    ops.block_count = (uint64_t)bytes / EXAMPLE_BLOCK_SIZE;
    ops.block_size = EXAMPLE_BLOCK_SIZE;
    scratch.buffer = block;
    scratch.size = sizeof(block);

    status = afspr_probe_detailed(&ops, &scratch, &result, sizeof(result),
                                  &diagnostic, sizeof(diagnostic));
    if (status != AFSPR_OK) {
        fprintf(stderr, "%s: %s during %s (", argv[1],
                afspr_status_string(status),
                afspr_probe_stage_string(diagnostic.stage));
        if (diagnostic.block == AFSPR_NO_BLOCK) {
            fputs("block=n/a", stderr);
        } else {
            fprintf(stderr, "block=%llu",
                    (unsigned long long)diagnostic.block);
        }
        if (diagnostic.checkpoint_slot == AFSPR_NO_CHECKPOINT_SLOT) {
            fputs(" slot=n/a", stderr);
        } else {
            fprintf(stderr, " slot=%d", (int)diagnostic.checkpoint_slot);
        }
        fprintf(stderr, ", A=%s, B=%s)\n",
                afspr_status_string(diagnostic.checkpoint_status[0]),
                afspr_status_string(diagnostic.checkpoint_status[1]));
        fclose(reader.file);
        return 1;
    }

    printf("label=%s generation=%llu checkpoint=%c blocks=%llu\n",
           result.label, (unsigned long long)result.generation,
           result.selected_checkpoint == 0u ? 'A' : 'B',
           (unsigned long long)result.total_blocks);
    fclose(reader.file);
    return 0;
}
