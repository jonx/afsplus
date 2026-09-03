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
#define TEST_OBJECT_FLAGS_OFFSET (32u + 10u)

struct file_device {
    FILE *file;
    uint64_t trace[64];
    size_t trace_count;
    uint64_t corrupt_lba;
    int corrupt_mode;
    uint64_t fail_lba;
    uint64_t virtual_lba;
    const uint8_t *virtual_block;
};

struct memory_device {
    uint8_t blocks[TEST_PREFIX_BLOCKS * TEST_BLOCK_SIZE];
};

static void reseal(uint8_t *block);
static void put_le16(uint8_t *p, uint16_t value);
static void put_le64(uint8_t *p, uint64_t value);

static int file_read_blocks(void *ctx, uint64_t first_block, uint32_t count,
                            void *dst)
{
    struct file_device *device = (struct file_device *)ctx;
    uint64_t offset = first_block * TEST_BLOCK_SIZE;
    size_t bytes = TEST_BLOCK_SIZE;

    if (count != 1u || first_block == device->fail_lba) {
        return -1;
    }
    if (first_block == device->virtual_lba && device->virtual_block != NULL) {
        memcpy(dst, device->virtual_block, TEST_BLOCK_SIZE);
    } else if (
        first_block > (uint64_t)LONG_MAX / TEST_BLOCK_SIZE ||
        offset > (uint64_t)LONG_MAX ||
        fseek(device->file, (long)offset, SEEK_SET) != 0 ||
        fread(dst, 1, bytes, device->file) != bytes) {
        return -1;
    }
    if (device->trace_count < sizeof(device->trace) / sizeof(device->trace[0])) {
        device->trace[device->trace_count++] = first_block;
    }
    if (first_block == device->corrupt_lba) {
        if (device->corrupt_mode == 1) {
            ((uint8_t *)dst)[100] ^= 1u;
        } else if (device->corrupt_mode == 2) {
            ((uint8_t *)dst)[32] = 2u;
            reseal((uint8_t *)dst);
        } else if (device->corrupt_mode == 3) {
            ((uint8_t *)dst)[56] ^= 1u;
            reseal((uint8_t *)dst);
        } else if (device->corrupt_mode == 4) {
            put_le64((uint8_t *)dst + 16u, UINT64_MAX);
            reseal((uint8_t *)dst);
        } else if (device->corrupt_mode == 5) {
            ((uint8_t *)dst)[40] =
                (uint8_t)(((uint8_t *)dst)[40] + 1u);
            ((uint8_t *)dst)[56] =
                (uint8_t)(((uint8_t *)dst)[56] + 1u);
            reseal((uint8_t *)dst);
        } else if (device->corrupt_mode == 6) {
            put_le16((uint8_t *)dst + TEST_OBJECT_FLAGS_OFFSET,
                     AFSPR_OBJECT_FLAG_DATA_IN_PLACE);
            reseal((uint8_t *)dst);
        }
    }
    return 0;
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

static void put_le16(uint8_t *p, uint16_t value)
{
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
}

static void put_le64(uint8_t *p, uint64_t value)
{
    put_le32(p, (uint32_t)value);
    put_le32(p + 4, (uint32_t)(value >> 32));
}

static void put_be64(uint8_t *p, uint64_t value)
{
    unsigned index;

    for (index = 0; index < 8u; ++index) {
        p[index] = (uint8_t)(value >> (56u - index * 8u));
    }
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

static void make_extent_leaf(uint8_t *block, uint64_t owner,
                             uint64_t generation, uint64_t physical_start,
                             uint64_t block_count)
{
    uint8_t *payload = block + 32u;
    uint8_t *item = payload + 32u;

    memset(block, 0, TEST_BLOCK_SIZE);
    put_le32(block, UINT32_C(0x54534641));
    put_le16(block + 4u, 1u);
    put_le64(block + 8u, owner);
    put_le64(block + 16u, generation);
    put_le32(block + 24u, 72u);
    payload[0] = 3u;
    put_le32(payload + 4u, 1u);
    put_le64(payload + 8u, 1u);
    put_le16(item, 8u);
    put_le16(item + 2u, 24u);
    put_be64(item + 8u, 1u);
    put_le64(item + 16u, physical_start);
    put_le64(item + 24u, block_count);
    reseal(block);
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
    struct afspr_probe_result policy_volume;
    struct afspr_object root;
    struct afspr_object file;
    struct afspr_object tree_file;
    struct afspr_object policy_file;
    struct afspr_directory_entry entry;
    struct afspr_diagnostic diagnostic;
    uint8_t scratch[AFSPR_TREE_SCRATCH_SIZE];
    uint8_t name[256];
    uint8_t actual[777];
    uint8_t expected[777];
    uint8_t extent_leaf[TEST_BLOCK_SIZE];
    long image_size;
    uint64_t block_count;
    uint64_t total_entries = 0u;
    uint64_t file_id = 0u;
    uint64_t ordinal;
    uint64_t file_offset = 0u;
    uint64_t corrupt_tree_lba;
    uint64_t object_lba;
    uint64_t claimed_tree_lba;
    uint64_t claimed_child_lba;
    FILE *expected_file;
    size_t bytes_read;
    unsigned selected;
    unsigned other;
    int status;

    require(argc == 3, "usage: reader_probe <image> <expected-file>");
    memset(&file_device, 0, sizeof(file_device));
    file_device.corrupt_lba = UINT64_MAX;
    file_device.fail_lba = UINT64_MAX;
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

    require((afspr_capabilities() &
             (AFSPR_CAP_PROBE | AFSPR_CAP_OBJECT_LOOKUP |
              AFSPR_CAP_DIRECTORY_ORDINAL | AFSPR_CAP_FILE_READ |
              AFSPR_CAP_INTENT_LOG_SCAN | AFSPR_CAP_INTENT_FILE_READ |
              AFSPR_CAP_INTENT_NAMESPACE)) ==
                (AFSPR_CAP_PROBE | AFSPR_CAP_OBJECT_LOOKUP |
                 AFSPR_CAP_DIRECTORY_ORDINAL | AFSPR_CAP_FILE_READ |
                 AFSPR_CAP_INTENT_LOG_SCAN | AFSPR_CAP_INTENT_FILE_READ |
                 AFSPR_CAP_INTENT_NAMESPACE),
            "reader capability summary is incomplete");

    file_device.trace_count = 0u;
    status = afspr_lookup_object(&file_ops, &(struct afspr_scratch){
                                               scratch, sizeof(scratch)},
                                 &first, 1u, &root, sizeof(root), &diagnostic,
                                 sizeof(diagnostic));
    require(status == AFSPR_OK, afspr_status_string(status));
    require(root.type == AFSPR_OBJECT_DIRECTORY && root.object_id == 1u,
            "root object is not a directory");
    require(file_device.trace_count >= 3u,
            "object lookup did not cross an internal object-map node");
    claimed_tree_lba = file_device.trace[0];
    claimed_child_lba = file_device.trace[1];
    file_device.corrupt_lba = claimed_tree_lba;
    file_device.corrupt_mode = 5;
    status = afspr_lookup_object(
        &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)}, &first,
        1u, &root, sizeof(root), &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT &&
                diagnostic.stage == AFSPR_STAGE_TREE_DECODE &&
                diagnostic.block == claimed_child_lba,
            "parent/child subtree over-claim was not localized");
    file_device.corrupt_lba = UINT64_MAX;
    file_device.corrupt_mode = 0;
    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, 1u, &root, sizeof(root) - 1u,
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_ABI &&
                diagnostic.stage == AFSPR_STAGE_ARGUMENTS,
            "short object result did not fail at the ABI boundary");

    status = afspr_directory_entry_at(
        &file_ops, &(struct afspr_scratch){scratch, TEST_BLOCK_SIZE}, &first,
        &root, 0u, name, sizeof(name), &entry, sizeof(entry), NULL,
        &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_SCRATCH_TOO_SMALL &&
                diagnostic.stage == AFSPR_STAGE_ARGUMENTS,
            "directory traversal accepted one-block scratch");

    status = afspr_directory_entry_at(
        &file_ops,
        &(struct afspr_scratch){scratch, sizeof(scratch)}, &first, &root, 0u,
        NULL, 0u, &entry, sizeof(entry), &total_entries, &diagnostic,
        sizeof(diagnostic));
    require(status == AFSPR_ERR_BUFFER_TOO_SMALL && entry.name_len > 0u,
            "directory name sizing query did not report required bytes");
    require(total_entries >= 303u,
            "fixture did not build a multi-page root directory");

    for (ordinal = 0u; ordinal < total_entries; ++ordinal) {
        status = afspr_directory_entry_at(
            &file_ops,
            &(struct afspr_scratch){scratch, sizeof(scratch)}, &first, &root,
            ordinal, name, sizeof(name), &entry, sizeof(entry), NULL,
            &diagnostic, sizeof(diagnostic));
        require(status == AFSPR_OK, afspr_status_string(status));
        require(entry.parent_id == root.object_id && entry.name == name,
                "directory entry lost caller-owned name storage");
        if (entry.name_len == strlen("readme.md") &&
            memcmp(entry.name, "readme.md", entry.name_len) == 0) {
            file_id = entry.object_id;
        }
    }
    require(file_id != 0u, "reader did not enumerate readme.md");
    status = afspr_directory_entry_at(
        &file_ops,
        &(struct afspr_scratch){scratch, sizeof(scratch)}, &first, &root,
        total_entries, name, sizeof(name), &entry, sizeof(entry), NULL,
        &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_NOT_FOUND,
            "directory ordinal past the end was accepted");

    file_device.trace_count = 0u;
    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, file_id, &file, sizeof(file),
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_OK && file.type == AFSPR_OBJECT_FILE,
            "enumerated file object could not be decoded");
    require(file_device.trace_count >= 3u,
            "file lookup did not cross an internal object-map node");
    object_lba = file_device.trace[file_device.trace_count - 1u];
    corrupt_tree_lba = file_device.trace[file_device.trace_count - 2u];

    file_device.corrupt_lba = file_device.trace[0];
    file_device.corrupt_mode = 3;
    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, file_id, &file, sizeof(file),
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT &&
                diagnostic.stage == AFSPR_STAGE_TREE_DECODE &&
                diagnostic.block == file_device.corrupt_lba,
            "internal subtree-count corruption was not localized");

    file_device.corrupt_lba = corrupt_tree_lba;
    file_device.corrupt_mode = 1;
    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, file_id, &file, sizeof(file),
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT &&
                diagnostic.stage == AFSPR_STAGE_TREE_DECODE &&
                diagnostic.block == corrupt_tree_lba,
            "tree checksum corruption was not localized to its LBA");

    file_device.corrupt_mode = 2;
    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, file_id, &file, sizeof(file),
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT &&
                diagnostic.stage == AFSPR_STAGE_TREE_DECODE &&
                diagnostic.block == corrupt_tree_lba,
            "valid-checksum tree identity corruption was not localized");

    file_device.corrupt_lba = object_lba;
    file_device.corrupt_mode = 4;
    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, file_id, &file, sizeof(file),
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT &&
                diagnostic.stage == AFSPR_STAGE_OBJECT_DECODE &&
                diagnostic.block == object_lba,
            "future-generation object record was not rejected at its LBA");

    file_device.corrupt_mode = 6;
    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, file_id, &file, sizeof(file),
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT &&
                diagnostic.stage == AFSPR_STAGE_OBJECT_DECODE &&
                diagnostic.block == object_lba,
            "data-policy flag without its feature was accepted");
    file_device.corrupt_lba = UINT64_MAX;
    file_device.corrupt_mode = 0;

    status = afspr_lookup_object(&file_ops,
                                 &(struct afspr_scratch){scratch,
                                                         sizeof(scratch)},
                                 &first, file_id, &file, sizeof(file),
                                 &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_OK, "file lookup did not recover after injection");

    expected_file = fopen(argv[2], "rb");
    require(expected_file != NULL, "cannot open expected file content");
    for (;;) {
        size_t expected_count = fread(expected, 1, sizeof(expected),
                                      expected_file);

        if (expected_count == 0u) {
            require(feof(expected_file) != 0,
                    "failed while reading expected file content");
            break;
        }
        status = afspr_read_file(
            &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)},
            &first, &file, file_offset, actual, sizeof(actual), &bytes_read,
            &diagnostic, sizeof(diagnostic));
        require(status == AFSPR_OK && bytes_read == expected_count,
                "bounded file read returned the wrong byte count");
        require(memcmp(actual, expected, expected_count) == 0,
                "portable C file bytes differ from the Rust input");
        file_offset += expected_count;
    }
    require(fclose(expected_file) == 0, "cannot close expected file");
    require(file_offset == file.size_bytes,
            "decoded file size differs from source length");
    file_device.fail_lba = file.data_root;
    status = afspr_read_file(
        &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)}, &first,
        &file, 0u, actual, sizeof(actual), &bytes_read, &diagnostic,
        sizeof(diagnostic));
    require(status == AFSPR_ERR_IO &&
                diagnostic.stage == AFSPR_STAGE_DATA_READ &&
                diagnostic.block == file.data_root,
            "data I/O error did not identify its physical block");
    file_device.fail_lba = UINT64_MAX;
    status = afspr_read_file(
        &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)}, &first,
        &file, file.size_bytes, actual, sizeof(actual), &bytes_read,
        &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_OK && bytes_read == 0u,
            "file read did not report EOF cleanly");

    policy_file = file;
    policy_file.flags |= AFSPR_OBJECT_FLAG_DATA_IN_PLACE;
    status = afspr_read_file(
        &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)}, &first,
        &policy_file, policy_file.size_bytes, actual, sizeof(actual),
        &bytes_read, &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT,
            "caller-supplied data-policy flag bypassed feature congruence");
    policy_volume = first;
    policy_volume.compat_features |= AFSPR_COMPAT_DATA_POLICY;
    status = afspr_read_file(
        &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)},
        &policy_volume, &policy_file, policy_file.size_bytes, actual,
        sizeof(actual), &bytes_read, &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_OK && bytes_read == 0u,
            "data-policy flag was rejected when its feature was present");

    tree_file = file;
    tree_file.flags = AFSPR_OBJECT_FLAG_EXTENT_TREE;
    tree_file.data_root = block_count - 1u;
    tree_file.size_bytes += TEST_BLOCK_SIZE;
    make_extent_leaf(extent_leaf, tree_file.object_id, first.generation,
                     file.data_root, file.data_blocks);
    file_device.virtual_lba = tree_file.data_root;
    file_device.virtual_block = extent_leaf;
    file_offset = 0u;
    do {
        size_t request = (size_t)(TEST_BLOCK_SIZE - file_offset);

        if (request > sizeof(actual)) {
            request = sizeof(actual);
        }
        status = afspr_read_file(
            &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)},
            &first, &tree_file, file_offset, actual, request,
            &bytes_read, &diagnostic, sizeof(diagnostic));
        require(status == AFSPR_OK && bytes_read != 0u,
                "sparse extent-tree hole read failed");
        for (ordinal = 0u; ordinal < bytes_read; ++ordinal) {
            require(actual[ordinal] == 0u,
                    "missing extent did not read as zero");
        }
        file_offset += bytes_read;
    } while (file_offset < TEST_BLOCK_SIZE);
    require(file_offset == TEST_BLOCK_SIZE,
            "hole read crossed into the following extent");

    expected_file = fopen(argv[2], "rb");
    require(expected_file != NULL,
            "cannot reopen expected extent-tree content");
    for (;;) {
        size_t expected_count = fread(expected, 1, sizeof(expected),
                                      expected_file);

        if (expected_count == 0u) {
            require(feof(expected_file) != 0,
                    "failed while rereading expected file content");
            break;
        }
        status = afspr_read_file(
            &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)},
            &first, &tree_file, file_offset, actual, sizeof(actual),
            &bytes_read, &diagnostic, sizeof(diagnostic));
        require(status == AFSPR_OK && bytes_read == expected_count &&
                    memcmp(actual, expected, expected_count) == 0,
                "extent-tree bytes differ from their direct source extent");
        file_offset += bytes_read;
    }
    require(fclose(expected_file) == 0,
            "cannot close extent-tree expected file");
    require(file_offset == tree_file.size_bytes,
            "sparse extent-tree size accounting is inconsistent");

    put_le32(extent_leaf + 96u, UINT32_C(0x80000000));
    reseal(extent_leaf);
    status = afspr_read_file(
        &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)}, &first,
        &tree_file, TEST_BLOCK_SIZE, actual, sizeof(actual), &bytes_read,
        &diagnostic, sizeof(diagnostic));
    require(status == AFSPR_ERR_CORRUPT &&
                diagnostic.stage == AFSPR_STAGE_EXTENT_DECODE &&
                diagnostic.block == tree_file.data_root,
            "valid-checksum extent corruption was not localized");
    make_extent_leaf(extent_leaf, tree_file.object_id, first.generation,
                     file.data_root, file.data_blocks);
    file_device.virtual_lba = UINT64_MAX;
    file_device.virtual_block = NULL;

    status = afspr_read_file(
        &file_ops, &(struct afspr_scratch){scratch, sizeof(scratch)}, &first,
        &root, 0u, actual, sizeof(actual), &bytes_read, &diagnostic,
        sizeof(diagnostic));
    require(status == AFSPR_ERR_NOT_FILE,
            "directory was accepted by the file reader");

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
