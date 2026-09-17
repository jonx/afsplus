/* SPDX-License-Identifier: BSD-2-Clause */
/* Prints the label and the generation the portable reader reports for an
 * image: "<generation> <label>". Exit 1 when the image does not probe. */
#include "libafsplus_reader.h"
#include <stdio.h>
#include <string.h>

#define BLOCK 4096u

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

int main(int argc, char **argv)
{
    static uint8_t scratch[AFSPR_INTENT_SCRATCH_SIZE];
    struct afspr_block_ops ops;
    struct afspr_scratch space;
    struct afspr_probe_result volume;
    FILE *file;
    long end;

    if (argc != 2) return 2;
    file = fopen(argv[1], "rb");
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
        return 1;
    }
    printf("%llu %s", (unsigned long long)volume.generation, volume.label);
    return 0;
}
