/* SPDX-License-Identifier: BSD-2-Clause */

#include "libafsplus_reader.h"

#include <limits.h>
#include <stdio.h>
#include <string.h>

#define EXAMPLE_BLOCK_SIZE 4096u

struct file_reader {
    FILE *file;
};

static void print_diagnostic(const char *path, int status,
                             const struct afspr_diagnostic *diagnostic)
{
    fprintf(stderr, "%s: %s during %s (", path,
            afspr_status_string(status),
            afspr_probe_stage_string(diagnostic->stage));
    if (diagnostic->block == AFSPR_NO_BLOCK) {
        fputs("block=n/a", stderr);
    } else {
        fprintf(stderr, "block=%llu",
                (unsigned long long)diagnostic->block);
    }
    if (diagnostic->checkpoint_slot == AFSPR_NO_CHECKPOINT_SLOT) {
        fputs(" slot=n/a", stderr);
    } else {
        fprintf(stderr, " slot=%d", (int)diagnostic->checkpoint_slot);
    }
    fprintf(stderr, ", A=%s, B=%s)\n",
            afspr_status_string(diagnostic->checkpoint_status[0]),
            afspr_status_string(diagnostic->checkpoint_status[1]));
}

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
    struct afspr_object root;
    struct afspr_directory_entry first;
    struct afspr_intent_view intent;
    struct afspr_diagnostic diagnostic;
    uint8_t block[AFSPR_INTENT_SCRATCH_SIZE];
    uint8_t name[256];
    uint64_t total_entries;
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
        print_diagnostic(argv[1], status, &diagnostic);
        fclose(reader.file);
        return 1;
    }

    printf("label=%s generation=%llu checkpoint=%c blocks=%llu\n",
           result.label, (unsigned long long)result.generation,
           result.selected_checkpoint == 0u ? 'A' : 'B',
           (unsigned long long)result.total_blocks);

    status = afspr_scan_intent_log(&ops, &scratch, &result, &intent,
                                   sizeof(intent), &diagnostic,
                                   sizeof(diagnostic));
    if (status != AFSPR_OK) {
        print_diagnostic(argv[1], status, &diagnostic);
        fclose(reader.file);
        return 1;
    }
    printf("pending-intent-records=%u operations=%u tail=%s\n",
           intent.valid_records, intent.valid_operations,
           afspr_intent_tail_string(intent.tail_state));

    status = afspr_lookup_object(&ops, &scratch, &result, 1u, &root,
                                 sizeof(root), &diagnostic,
                                 sizeof(diagnostic));
    if (status != AFSPR_OK) {
        print_diagnostic(argv[1], status, &diagnostic);
        fclose(reader.file);
        return 1;
    }
    status = afspr_directory_entry_at(
        &ops, &scratch, &result, &root, 0u, name, sizeof(name), &first,
        sizeof(first), &total_entries, &diagnostic, sizeof(diagnostic));
    if (status == AFSPR_ERR_NOT_FOUND) {
        puts("root-entries=0");
    } else if (status != AFSPR_OK) {
        print_diagnostic(argv[1], status, &diagnostic);
        fclose(reader.file);
        return 1;
    } else {
        printf("root-entries=%llu first=%.*s object=%llu\n",
               (unsigned long long)total_entries, (int)first.name_len,
               (const char *)first.name,
               (unsigned long long)first.object_id);
    }
    fclose(reader.file);
    return 0;
}
