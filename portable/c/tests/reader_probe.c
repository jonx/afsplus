/* SPDX-License-Identifier: BSD-2-Clause */

#include "libafsplus_reader.h"

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define TEST_BLOCK_SIZE 4096u
#define TEST_PREFIX_BLOCKS 3u
#define TEST_CHECKSUM_OFFSET 28u
#define TEST_IDENT_RO_COMPAT_OFFSET (32u + 145u)
#define TEST_CHECKPOINT_OBJECT_MAP_OFFSET (32u + 32u)
#define TEST_CHECKPOINT_SHARED_ROOT_OFFSET (32u + 88u)

struct file_device {
    FILE *file;
};

struct memory_device {
    uint8_t blocks[TEST_PREFIX_BLOCKS * TEST_BLOCK_SIZE];
};

static int file_read_blocks(void *ctx, uint64_t first_block, uint32_t count,
                            void *dst)
{
    struct file_device *device = (struct file_device *)ctx;
    uint64_t offset = first_block * TEST_BLOCK_SIZE;
    size_t bytes = TEST_BLOCK_SIZE;

    if (count != 1u ||
        first_block > (uint64_t)LONG_MAX / TEST_BLOCK_SIZE ||
        offset > (uint64_t)LONG_MAX ||
        fseek(device->file, (long)offset, SEEK_SET) != 0) {
        return -1;
    }
    return fread(dst, 1, bytes, device->file) == bytes ? 0 : -1;
}

static int memory_read_blocks(void *ctx, uint64_t first_block, uint32_t count,
                              void *dst)
{
    struct memory_device *device = (struct memory_device *)ctx;

    if (first_block >= TEST_PREFIX_BLOCKS ||
        count > TEST_PREFIX_BLOCKS - (uint32_t)first_block) {
        return -1;
    }
    memcpy(dst, device->blocks + (size_t)first_block * TEST_BLOCK_SIZE,
           (size_t)count * TEST_BLOCK_SIZE);
    return 0;
}

static void require(int condition, const char *message)
{
    if (!condition) {
        fprintf(stderr, "portable C reader test failed: %s\n", message);
        exit(1);
    }
}

static void put_le32(uint8_t *p, uint32_t value)
{
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
    p[2] = (uint8_t)(value >> 16);
    p[3] = (uint8_t)(value >> 24);
}

static void put_le64(uint8_t *p, uint64_t value)
{
    put_le32(p, (uint32_t)value);
    put_le32(p + 4, (uint32_t)(value >> 32));
}

static uint32_t crc32c(const uint8_t *data, size_t size)
{
    uint32_t crc = UINT32_MAX;
    size_t i;

    for (i = 0; i < size; ++i) {
        unsigned bit;

        crc ^= data[i];
        for (bit = 0; bit < 8u; ++bit) {
            uint32_t mask = (uint32_t)(0u - (crc & 1u));
            crc = (crc >> 1) ^ (UINT32_C(0x82f63b78) & mask);
        }
    }
    return ~crc;
}

static void reseal(uint8_t *block)
{
    put_le32(block + TEST_CHECKSUM_OFFSET, 0u);
    put_le32(block + TEST_CHECKSUM_OFFSET, crc32c(block, TEST_BLOCK_SIZE));
}

static int probe(const struct afspr_block_ops *ops, void *scratch,
                 size_t scratch_size, struct afspr_probe_result *result)
{
    struct afspr_scratch workspace;

    workspace.buffer = scratch;
    workspace.size = scratch_size;
    return afspr_probe(ops, &workspace, result, sizeof(*result));
}

static int probe_detailed(const struct afspr_block_ops *ops, void *scratch,
                          size_t scratch_size,
                          struct afspr_probe_result *result,
                          struct afspr_diagnostic *diagnostic)
{
    struct afspr_scratch workspace;

    workspace.buffer = scratch;
    workspace.size = scratch_size;
    return afspr_probe_detailed(ops, &workspace, result, sizeof(*result),
                                diagnostic, sizeof(*diagnostic));
}

int main(int argc, char **argv)
{
    struct file_device file_device;
    struct memory_device pristine;
    struct memory_device mutated;
    struct afspr_block_ops file_ops;
    struct afspr_block_ops memory_ops;
    struct afspr_probe_result first;
    struct afspr_probe_result fallback;
    struct afspr_diagnostic diagnostic;
    uint8_t scratch[TEST_BLOCK_SIZE];
    long image_size;
    uint64_t block_count;
    unsigned selected;
    unsigned other;
    int status;

    require(argc == 2, "usage: reader_probe <image>");
    file_device.file = fopen(argv[1], "rb");
    require(file_device.file != NULL, "cannot open Rust image");
    require(fseek(file_device.file, 0, SEEK_END) == 0,
            "cannot seek to image end");
    image_size = ftell(file_device.file);
    require(image_size > 0 && image_size % TEST_BLOCK_SIZE == 0,
            "image size is not block aligned");
    block_count = (uint64_t)image_size / TEST_BLOCK_SIZE;

    memset(&file_ops, 0, sizeof(file_ops));
    file_ops.abi_version = AFSPR_ABI_VERSION;
    file_ops.struct_size = (uint32_t)sizeof(file_ops);
    file_ops.ctx = &file_device;
    file_ops.read_blocks = file_read_blocks;
    file_ops.block_count = block_count;
    file_ops.block_size = TEST_BLOCK_SIZE;
    status = probe(&file_ops, scratch, sizeof(scratch), &first);
    require(status == AFSPR_OK, afspr_status_string(status));
    require(first.abi_version == AFSPR_ABI_VERSION,
            "result ABI version mismatch");
    require(first.identification_version == 3u,
            "Rust image did not use identification v3");
    require(strcmp(first.label, "PortableC") == 0,
            "volume label mismatch");
    require(first.total_blocks == block_count,
            "declared and container block counts differ");
    require(first.valid_checkpoint_mask == 3u,
            "Rust image must provide two retained checkpoints");
    require(first.generation >= 3u,
            "Rust image did not advance through multiple generations");

    require(fseek(file_device.file, 0, SEEK_SET) == 0,
            "cannot rewind Rust image");
    require(fread(pristine.blocks, 1, sizeof(pristine.blocks),
                  file_device.file) == sizeof(pristine.blocks),
            "cannot read boot and checkpoint blocks");
    require(fclose(file_device.file) == 0, "cannot close Rust image");

    memset(&memory_ops, 0, sizeof(memory_ops));
    memory_ops.abi_version = AFSPR_ABI_VERSION;
    memory_ops.struct_size = (uint32_t)sizeof(memory_ops);
    memory_ops.ctx = &mutated;
    memory_ops.read_blocks = memory_read_blocks;
    memory_ops.block_count = block_count;
    memory_ops.block_size = TEST_BLOCK_SIZE;
    selected = first.selected_checkpoint;
    other = selected ^ 1u;

    mutated = pristine;
    mutated.blocks[(selected + 1u) * TEST_BLOCK_SIZE + 64u] ^= 0x80u;
    status = probe_detailed(&memory_ops, scratch, sizeof(scratch), &fallback,
                            &diagnostic);
    require(status == AFSPR_OK, "torn newest checkpoint did not fall back");
    require(fallback.selected_checkpoint == other,
            "fallback selected the wrong checkpoint");
    require(fallback.generation < first.generation,
            "fallback generation is not older");
    require(fallback.valid_checkpoint_mask == (uint8_t)(1u << other),
            "corrupt checkpoint remained valid");
    require(diagnostic.stage == AFSPR_STAGE_COMPLETE &&
                diagnostic.checkpoint_status[selected] == AFSPR_ERR_CORRUPT &&
                diagnostic.checkpoint_status[other] == AFSPR_OK,
            "fallback diagnostics did not identify the corrupt slot");

    mutated = pristine;
    put_le64(mutated.blocks + (selected + 1u) * TEST_BLOCK_SIZE +
                 TEST_CHECKPOINT_OBJECT_MAP_OFFSET,
             0u);
    reseal(mutated.blocks + (selected + 1u) * TEST_BLOCK_SIZE);
    status = probe_detailed(&memory_ops, scratch, sizeof(scratch), &fallback,
                            &diagnostic);
    require(status == AFSPR_OK,
            "structurally invalid newest checkpoint did not fall back");
    require(fallback.selected_checkpoint == other &&
                diagnostic.checkpoint_status[selected] == AFSPR_ERR_CORRUPT,
            "valid-checksum structural corruption was not localized");

    mutated = pristine;
    put_le64(mutated.blocks + TEST_IDENT_RO_COMPAT_OFFSET, 0u);
    reseal(mutated.blocks);
    put_le64(mutated.blocks + (selected + 1u) * TEST_BLOCK_SIZE +
                 TEST_CHECKPOINT_SHARED_ROOT_OFFSET,
             first.object_map_block);
    reseal(mutated.blocks + (selected + 1u) * TEST_BLOCK_SIZE);
    status = probe_detailed(&memory_ops, scratch, sizeof(scratch), &fallback,
                            &diagnostic);
    require(status == AFSPR_ERR_CORRUPT,
            "selected checkpoint feature mismatch was hidden by fallback");
    require(diagnostic.stage == AFSPR_STAGE_CHECKPOINT_SELECTION &&
                diagnostic.checkpoint_slot == (int32_t)selected &&
                diagnostic.checkpoint_status[selected] == AFSPR_OK,
            "selected-state corruption diagnostic lost its slot or phase");

    mutated = pristine;
    mutated.blocks[(selected + 1u) * TEST_BLOCK_SIZE + 64u] ^= 0x80u;
    mutated.blocks[(other + 1u) * TEST_BLOCK_SIZE + 64u] ^= 0x40u;
    status = probe_detailed(&memory_ops, scratch, sizeof(scratch), &fallback,
                            &diagnostic);
    require(status == AFSPR_ERR_NO_CHECKPOINT,
            "two corrupt checkpoints were accepted");
    require(diagnostic.stage == AFSPR_STAGE_CHECKPOINT_SELECTION &&
                diagnostic.checkpoint_status[0] == AFSPR_ERR_CORRUPT &&
                diagnostic.checkpoint_status[1] == AFSPR_ERR_CORRUPT,
            "two-slot corruption diagnostics are incomplete");

    mutated = pristine;
    memcpy(mutated.blocks + (other + 1u) * TEST_BLOCK_SIZE,
           mutated.blocks + (selected + 1u) * TEST_BLOCK_SIZE,
           TEST_BLOCK_SIZE);
    status = probe_detailed(&memory_ops, scratch, sizeof(scratch), &fallback,
                            &diagnostic);
    require(status == AFSPR_ERR_AMBIGUOUS_CHECKPOINT,
            "same-generation checkpoints were not rejected");

    mutated = pristine;
    mutated.blocks[100] ^= 1u;
    status = probe_detailed(&memory_ops, scratch, sizeof(scratch), &fallback,
                            &diagnostic);
    require(status == AFSPR_ERR_CORRUPT,
            "corrupt identification block was accepted");
    require(diagnostic.stage == AFSPR_STAGE_IDENTIFICATION_DECODE &&
                diagnostic.block == 0u,
            "identification diagnostic points to the wrong stage or block");

    mutated = pristine;
    status = probe(&memory_ops, scratch, TEST_BLOCK_SIZE - 1u, &fallback);
    require(status == AFSPR_ERR_SCRATCH_TOO_SMALL,
            "short scratch buffer was accepted");

    ++memory_ops.abi_version;
    status = probe_detailed(&memory_ops, scratch, sizeof(scratch), &fallback,
                            &diagnostic);
    require(status == AFSPR_ERR_ABI &&
                diagnostic.stage == AFSPR_STAGE_ARGUMENTS,
            "ABI mismatch did not produce an argument diagnostic");
    --memory_ops.abi_version;

    status = afspr_probe_detailed(&memory_ops, NULL, &fallback,
                                  sizeof(fallback), &diagnostic,
                                  sizeof(diagnostic));
    require(status == AFSPR_ERR_INVALID_ARGUMENT &&
                diagnostic.stage == AFSPR_STAGE_ARGUMENTS,
            "null scratch did not produce an argument diagnostic");

    printf("portable-c-reader PASS generation=%llu slot=%u label=%s\n",
           (unsigned long long)first.generation,
           (unsigned)first.selected_checkpoint, first.label);
    return 0;
}
