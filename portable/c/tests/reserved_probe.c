/* SPDX-License-Identifier: BSD-2-Clause */
/* Reads one image through the public entry point that reaches a given block
 * kind, so a Rust test can ask the portable reader the same question it asks
 * the core. Exit 0 means the reader accepted, 1 that it refused, 2 that the
 * probe could not run.
 *
 *   reserved_probe probe  <image>            identification and checkpoint
 *   reserved_probe lookup <image> <object>   object map tree nodes and record
 *   reserved_probe intent <image>            intent-log records
 *   reserved_probe alloc  <image>            region descriptors and bitmaps
 */
#include "libafsplus_reader.h"
#include "../reader_internal.h"
#include <stdio.h>
#include <stdlib.h>
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
    const char *kind;

    if (argc < 3) return 2;
    kind = argv[1];
    file = fopen(argv[2], "rb");
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
        return 1;
    }
    if (strcmp(kind, "probe") == 0) {
        (void)fclose(file);
        return 0;
    }
    if (strcmp(kind, "lookup") == 0 && argc == 4) {
        struct afspr_object object;
        int status = afspr_lookup_object(&ops, &space, &volume,
                                         (uint64_t)strtoull(argv[3], NULL, 0),
                                         &object, sizeof(object), NULL, 0u);
        (void)fclose(file);
        return status == AFSPR_OK ? 0 : 1;
    }
    if (strcmp(kind, "intent") == 0) {
        static struct afspr_intent_view view;
        int status = afspr_scan_intent_log(&ops, &space, &volume, &view,
                                           sizeof(view), NULL, 0u);
        (void)fclose(file);
        /* A record the reader refuses ends the scan before it: the view is
         * accepted and holds no operation. */
        return status == AFSPR_OK && view.valid_operations > 0u ? 0 : 1;
    }
    if (strcmp(kind, "alloc") == 0) {
        static struct afspr_intent_view view;
        uint64_t data_block = 0u;
        int status = afspr_scan_intent_log(&ops, &space, &volume, &view,
                                           sizeof(view), NULL, 0u);
        if (status == AFSPR_OK) {
            status = afspr_internal_find_log_data_block(
                &ops, &space, &volume, &view, 0u, &data_block, NULL, 0u);
        }
        (void)fclose(file);
        return status == AFSPR_OK ? 0 : 1;
    }
    (void)fclose(file);
    return 2;
}
