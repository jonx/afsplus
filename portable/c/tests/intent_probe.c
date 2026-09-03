/* SPDX-License-Identifier: BSD-2-Clause */

#include "libafsplus_reader.h"

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BLOCK_SIZE 4096u
#define CHECKSUM_OFFSET 28u
#define LOG_FIRST_EXTENT_OFFSET 128u
#define LOG_SEQUENCE_OFFSET 56u

struct file_device {
    FILE *file;
    uint64_t fail_lba;
};

static uint32_t load_u32(const uint8_t *bytes)
{
    return ((uint32_t)bytes[0]) | ((uint32_t)bytes[1] << 8) |
           ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}

static uint64_t load_u64(const uint8_t *bytes)
{
    return ((uint64_t)load_u32(bytes)) |
           ((uint64_t)load_u32(bytes + 4u) << 32);
}

static void store_u32(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
}

static void store_u64(uint8_t *bytes, uint64_t value)
{
    store_u32(bytes, (uint32_t)value);
    store_u32(bytes + 4u, (uint32_t)(value >> 32));
}

static uint32_t crc32c_update(uint32_t crc, const uint8_t *bytes, size_t size)
{
    size_t index;

    for (index = 0u; index < size; ++index) {
        unsigned int bit;

        crc ^= bytes[index];
        for (bit = 0u; bit < 8u; ++bit) {
            uint32_t mask = (uint32_t)(0u - (crc & 1u));
            crc = (crc >> 1) ^ (UINT32_C(0x82f63b78) & mask);
        }
    }
    return crc;
}

static void reseal(uint8_t block[BLOCK_SIZE])
{
    static const uint8_t zero[4] = {0u, 0u, 0u, 0u};
    uint32_t crc = UINT32_MAX;

    crc = crc32c_update(crc, block, CHECKSUM_OFFSET);
    crc = crc32c_update(crc, zero, sizeof(zero));
    crc = crc32c_update(crc, block + CHECKSUM_OFFSET + sizeof(zero),
                        BLOCK_SIZE - CHECKSUM_OFFSET - sizeof(zero));
    store_u32(block + CHECKSUM_OFFSET, ~crc);
}

static int read_blocks(void *opaque, uint64_t first, uint32_t count,
                       void *destination)
{
    struct file_device *device = (struct file_device *)opaque;
    uint8_t *output = (uint8_t *)destination;
    uint32_t index;

    for (index = 0u; index < count; ++index) {
        uint64_t lba = first + index;
        uint64_t offset = lba * BLOCK_SIZE;
        size_t got;

        if (lba == device->fail_lba || offset > (uint64_t)LONG_MAX ||
            fseek(device->file, (long)offset, SEEK_SET) != 0) {
            return -1;
        }
        got = fread(output + (size_t)index * BLOCK_SIZE, 1, BLOCK_SIZE,
                    device->file);
        if (got < BLOCK_SIZE) {
            if (ferror(device->file)) {
                return -1;
            }
            memset(output + (size_t)index * BLOCK_SIZE + got, 0,
                   BLOCK_SIZE - got);
            clearerr(device->file);
        }
    }
    return 0;
}

static int read_block(struct file_device *device, uint64_t lba,
                      uint8_t block[BLOCK_SIZE])
{
    return read_blocks(device, lba, 1u, block);
}

static int write_block(struct file_device *device, uint64_t lba,
                       const uint8_t block[BLOCK_SIZE])
{
    uint64_t offset = lba * BLOCK_SIZE;

    if (offset > (uint64_t)LONG_MAX ||
        fseek(device->file, (long)offset, SEEK_SET) != 0 ||
        fwrite(block, 1, BLOCK_SIZE, device->file) != BLOCK_SIZE ||
        fflush(device->file) != 0) {
        return -1;
    }
    return 0;
}

static void require_status(int actual, int expected, const char *label,
                           const struct afspr_diagnostic *diagnostic)
{
    if (actual != expected) {
        fprintf(stderr,
                "%s: got %s, expected %s (stage=%s block=%llu)\n", label,
                afspr_status_string(actual), afspr_status_string(expected),
                afspr_probe_stage_string(diagnostic->stage),
                (unsigned long long)diagnostic->block);
        exit(EXIT_FAILURE);
    }
}

static uint8_t *load_file(const char *path, size_t *size)
{
    FILE *file = fopen(path, "rb");
    long length;
    uint8_t *bytes;

    if (file == NULL) {
        fprintf(stderr, "cannot open expected file %s\n", path);
        exit(EXIT_FAILURE);
    }
    if (fseek(file, 0, SEEK_END) != 0 || (length = ftell(file)) < 0 ||
        fseek(file, 0, SEEK_SET) != 0) {
        fprintf(stderr, "cannot size expected file %s\n", path);
        (void)fclose(file);
        exit(EXIT_FAILURE);
    }
    *size = (size_t)(unsigned long)length;
    bytes = (uint8_t *)malloc(*size == 0u ? 1u : *size);
    if (bytes == NULL) {
        fprintf(stderr, "cannot allocate expected file %s\n", path);
        (void)fclose(file);
        exit(EXIT_FAILURE);
    }
    if (fread(bytes, 1, *size, file) != *size) {
        fprintf(stderr, "cannot read expected file %s\n", path);
        free(bytes);
        (void)fclose(file);
        exit(EXIT_FAILURE);
    }
    if (fclose(file) != 0) {
        fprintf(stderr, "cannot close expected file %s\n", path);
        free(bytes);
        exit(EXIT_FAILURE);
    }
    return bytes;
}

static void compare_intent_file(const struct afspr_block_ops *ops,
                                const struct afspr_scratch *scratch,
                                const struct afspr_probe_result *volume,
                                const struct afspr_intent_view *view,
                                uint64_t object_id, const char *expected_path)
{
    struct afspr_diagnostic diagnostic;
    uint8_t output[777];
    size_t expected_size;
    uint8_t *expected = load_file(expected_path, &expected_size);
    uint64_t reported_size = 0u;
    size_t offset = 0u;
    int status = afspr_intent_file_size(
        ops, scratch, volume, view, object_id, &reported_size, &diagnostic,
        sizeof(diagnostic));

    require_status(status, AFSPR_OK, "intent file size", &diagnostic);
    if (reported_size != expected_size) {
        fprintf(stderr, "object %llu size %llu, expected %lu\n",
                (unsigned long long)object_id,
                (unsigned long long)reported_size,
                (unsigned long)expected_size);
        exit(EXIT_FAILURE);
    }
    while (offset < expected_size) {
        size_t got = 0u;
        size_t wanted = expected_size - offset;

        if (wanted > sizeof(output)) {
            wanted = sizeof(output);
        }
        status = afspr_read_intent_file(
            ops, scratch, volume, view, object_id, offset, output, wanted,
            &got, &diagnostic, sizeof(diagnostic));
        require_status(status, AFSPR_OK, "intent file read", &diagnostic);
        if (got != wanted || memcmp(output, expected + offset, wanted) != 0) {
            fprintf(stderr, "object %llu differs at offset %lu\n",
                    (unsigned long long)object_id, (unsigned long)offset);
            exit(EXIT_FAILURE);
        }
        offset += got;
    }
    {
        size_t got = 1u;

        status = afspr_read_intent_file(
            ops, scratch, volume, view, object_id, reported_size, output,
            sizeof(output), &got, &diagnostic, sizeof(diagnostic));
        require_status(status, AFSPR_OK, "intent file EOF", &diagnostic);
        if (got != 0u) {
            fprintf(stderr, "object %llu EOF returned data\n",
                    (unsigned long long)object_id);
            exit(EXIT_FAILURE);
        }
    }
    free(expected);
}

static void require_entry(const struct afspr_directory_entry *entry,
                          uint64_t object_id, const char *name,
                          const char *label)
{
    size_t name_len = strlen(name);

    if (entry->object_id != object_id ||
        entry->type_hint != AFSPR_OBJECT_FILE ||
        entry->parent_id != 1u || entry->name_len != name_len ||
        memcmp(entry->name, name, name_len) != 0) {
        fprintf(stderr, "%s: unexpected directory entry\n", label);
        exit(EXIT_FAILURE);
    }
}

static void check_namespace(const struct afspr_block_ops *ops,
                            const struct afspr_scratch *scratch,
                            const struct afspr_probe_result *volume,
                            const struct afspr_intent_view *view)
{
    struct afspr_intent_directory_cursor cursor;
    struct afspr_directory_entry entry;
    struct afspr_diagnostic diagnostic;
    uint8_t name[255];
    int status;

    status = afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, 1u, "replace.txt", 11u, name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_OK, "rename replacement lookup",
                   &diagnostic);
    require_entry(&entry, 18u, "Replace.TXT",
                  "rename replacement lookup");
    status = afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, 1u, "final.bin", 9u, name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_OK, "hard-link survivor lookup",
                   &diagnostic);
    require_entry(&entry, 16u, "Final.BIN", "hard-link survivor lookup");
    status = afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, 1u, "pending.txt", 11u, name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_ERR_NOT_FOUND, "renamed source lookup",
                   &diagnostic);
    status = afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, 1u, "alias.bin", 9u, name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_ERR_NOT_FOUND, "deleted alias lookup",
                   &diagnostic);
    status = afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, 1u, "missing", 7u, name, sizeof(name),
        &entry, sizeof(entry), &diagnostic, sizeof(diagnostic));
    require_status(status, AFSPR_ERR_NOT_FOUND, "missing intent lookup",
                   &diagnostic);
    status = afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, 1u, "\xc3\xa9", 2u, name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_ERR_UNSUPPORTED,
                   "unsupported Unicode-profile lookup", &diagnostic);
    if (diagnostic.stage != AFSPR_STAGE_INTENT_NAMESPACE) {
        fprintf(stderr, "Unicode lookup lost its namespace diagnostic\n");
        exit(EXIT_FAILURE);
    }

    status = afspr_intent_directory_cursor_init(&cursor, sizeof(cursor));
    require_status(status, AFSPR_OK, "cursor init", &diagnostic);
    status = afspr_intent_directory_next(
        ops, scratch, volume, view, 1u, &cursor, sizeof(cursor), name, 1u,
        &entry, sizeof(entry), &diagnostic, sizeof(diagnostic));
    require_status(status, AFSPR_ERR_BUFFER_TOO_SMALL,
                   "cursor sizing query", &diagnostic);
    if (entry.name_len != strlen("Final.BIN")) {
        fprintf(stderr, "cursor sizing query lost required name length\n");
        exit(EXIT_FAILURE);
    }
    status = afspr_intent_directory_next(
        ops, scratch, volume, view, 1u, &cursor, sizeof(cursor), name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_OK, "cursor retry", &diagnostic);
    require_entry(&entry, 16u, "Final.BIN", "cursor retry");
    status = afspr_intent_directory_next(
        ops, scratch, volume, view, 1u, &cursor, sizeof(cursor), name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_OK, "cursor logged entry", &diagnostic);
    require_entry(&entry, 18u, "Replace.TXT", "cursor logged entry");
    status = afspr_intent_directory_next(
        ops, scratch, volume, view, 1u, &cursor, sizeof(cursor), name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));
    require_status(status, AFSPR_ERR_NOT_FOUND, "cursor EOF", &diagnostic);
}

static void require_namespace_corrupt(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t expected_lba,
    const char *label)
{
    struct afspr_directory_entry entry;
    struct afspr_diagnostic diagnostic;
    uint8_t name[255];
    int status = afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, 1u, "replace.txt", 11u, name,
        sizeof(name), &entry, sizeof(entry), &diagnostic,
        sizeof(diagnostic));

    require_status(status, AFSPR_ERR_CORRUPT, label, &diagnostic);
    if (diagnostic.stage != AFSPR_STAGE_INTENT_NAMESPACE ||
        diagnostic.block != expected_lba) {
        fprintf(stderr,
                "%s lost record coordinate: stage=%u block=%llu "
                "expected=%llu\n",
                label, diagnostic.stage,
                (unsigned long long)diagnostic.block,
                (unsigned long long)expected_lba);
        exit(EXIT_FAILURE);
    }
}

int main(int argc, char **argv)
{
    struct file_device device;
    struct afspr_block_ops ops;
    struct afspr_scratch scratch;
    struct afspr_probe_result volume;
    struct afspr_intent_view view;
    struct afspr_object policy_object;
    struct afspr_diagnostic diagnostic;
    uint8_t scratch_bytes[AFSPR_INTENT_SCRATCH_SIZE];
    uint8_t log_original[BLOCK_SIZE];
    uint8_t log_mutated[BLOCK_SIZE];
    uint8_t data_original[BLOCK_SIZE];
    uint8_t data_mutated[BLOCK_SIZE];
    uint64_t first_log;
    uint64_t data_lba;
    uint64_t saved_features;
    int status;

    if (argc != 4) {
        fprintf(stderr, "usage: %s IMAGE EXPECTED CREATED_EXPECTED\n",
                argv[0]);
        return EXIT_FAILURE;
    }
    device.file = fopen(argv[1], "r+b");
    if (device.file == NULL) {
        fprintf(stderr, "cannot open %s\n", argv[1]);
        return EXIT_FAILURE;
    }
    device.fail_lba = UINT64_MAX;
    memset(&ops, 0, sizeof(ops));
    ops.abi_version = AFSPR_ABI_VERSION;
    ops.struct_size = (uint32_t)sizeof(ops);
    ops.ctx = &device;
    ops.read_blocks = read_blocks;
    ops.block_count = 4096u;
    ops.block_size = BLOCK_SIZE;
    scratch.buffer = scratch_bytes;
    scratch.size = sizeof(scratch_bytes);

    status = afspr_probe_detailed(&ops, &scratch, &volume, sizeof(volume),
                                  &diagnostic, sizeof(diagnostic));
    require_status(status, AFSPR_OK, "probe", &diagnostic);
    status = afspr_scan_intent_log(&ops, &scratch, &volume, &view,
                                   sizeof(view), &diagnostic,
                                   sizeof(diagnostic));
    require_status(status, AFSPR_OK, "valid scan", &diagnostic);
    if (view.valid_records != 7u || view.valid_operations != 7u ||
        view.last_sequence != 7u ||
        view.tail_state != AFSPR_INTENT_TAIL_INVALID ||
        view.tail_slot != 7u || view.tail_block == AFSPR_NO_BLOCK) {
        fprintf(stderr,
                "unexpected view records=%u operations=%u sequence=%llu "
                "tail=%s slot=%u block=%llu\n",
                view.valid_records, view.valid_operations,
                (unsigned long long)view.last_sequence,
                afspr_intent_tail_string(view.tail_state), view.tail_slot,
                (unsigned long long)view.tail_block);
        return EXIT_FAILURE;
    }
    if ((view.flags & AFSPR_INTENT_VIEW_FILE_DATA) == 0u) {
        fprintf(stderr, "create/write/truncate prefix lacks file-data view\n");
        return EXIT_FAILURE;
    }
    if ((view.flags & AFSPR_INTENT_VIEW_NAMESPACE) == 0u ||
        (afspr_capabilities() & AFSPR_CAP_INTENT_NAMESPACE) == 0u) {
        fprintf(stderr, "intent namespace capability is unavailable\n");
        return EXIT_FAILURE;
    }
    if (strcmp(afspr_probe_stage_string(AFSPR_STAGE_INTENT_NAMESPACE),
               "intent-log namespace") != 0) {
        fprintf(stderr, "intent namespace stage is not printable\n");
        return EXIT_FAILURE;
    }
    status = afspr_lookup_object(
        &ops, &scratch, &volume, 17u, &policy_object,
        sizeof(policy_object), &diagnostic, sizeof(diagnostic));
    require_status(status, AFSPR_OK, "data-policy object", &diagnostic);
    if ((volume.compat_features & AFSPR_COMPAT_DATA_POLICY) == 0u ||
        (policy_object.flags & AFSPR_OBJECT_FLAG_DATA_IN_PLACE) == 0u) {
        fprintf(stderr, "Rust fixture lacks the persistent data-policy pair\n");
        return EXIT_FAILURE;
    }
    check_namespace(&ops, &scratch, &volume, &view);
    device.fail_lba = volume.object_map_block;
    {
        struct afspr_directory_entry ignored_entry;
        uint8_t ignored_name[255];

        status = afspr_lookup_intent_directory_entry(
            &ops, &scratch, &volume, &view, 1u, "final.bin", 9u,
            ignored_name, sizeof(ignored_name), &ignored_entry,
            sizeof(ignored_entry), &diagnostic, sizeof(diagnostic));
        require_status(status, AFSPR_ERR_IO, "namespace metadata I/O",
                       &diagnostic);
        if (diagnostic.stage != AFSPR_STAGE_TREE_READ ||
            diagnostic.block != volume.object_map_block) {
            fprintf(stderr, "namespace I/O lost its tree coordinate\n");
            return EXIT_FAILURE;
        }
    }
    device.fail_lba = UINT64_MAX;
    compare_intent_file(&ops, &scratch, &volume, &view, 16u, argv[2]);
    compare_intent_file(&ops, &scratch, &volume, &view, 18u, argv[3]);
    {
        uint64_t ignored_size;

        status = afspr_intent_file_size(
            &ops, &scratch, &volume, &view, 17u, &ignored_size,
            &diagnostic, sizeof(diagnostic));
        require_status(status, AFSPR_ERR_NOT_FOUND,
                       "replaced object identity", &diagnostic);
    }
    {
        struct afspr_intent_view forged = view;
        uint64_t ignored_size;

        ++forged.last_sequence;
        status = afspr_intent_file_size(
            &ops, &scratch, &volume, &forged, 16u, &ignored_size,
            &diagnostic, sizeof(diagnostic));
        require_status(status, AFSPR_ERR_CORRUPT, "forged view",
                       &diagnostic);
    }
    first_log = view.tail_block - 7u;
    if (read_block(&device, first_log + 4u, log_original) != 0) {
        return EXIT_FAILURE;
    }
    memcpy(log_mutated, log_original, sizeof(log_mutated));
    log_mutated[128] = (uint8_t)'q';
    reseal(log_mutated);
    if (write_block(&device, first_log + 4u, log_mutated) != 0) {
        return EXIT_FAILURE;
    }
    require_namespace_corrupt(&ops, &scratch, &volume, &view,
                              first_log + 4u,
                              "missing rename source");
    if (write_block(&device, first_log + 4u, log_original) != 0) {
        return EXIT_FAILURE;
    }

    if (read_block(&device, first_log + 6u, log_original) != 0) {
        return EXIT_FAILURE;
    }
    memcpy(log_mutated, log_original, sizeof(log_mutated));
    log_mutated[65] = 0u;
    reseal(log_mutated);
    if (write_block(&device, first_log + 6u, log_mutated) != 0) {
        return EXIT_FAILURE;
    }
    require_namespace_corrupt(&ops, &scratch, &volume, &view,
                              first_log + 6u,
                              "replace flag mismatch");
    if (write_block(&device, first_log + 6u, log_original) != 0) {
        return EXIT_FAILURE;
    }

    if (read_block(&device, first_log + 3u, log_original) != 0) {
        return EXIT_FAILURE;
    }
    memcpy(log_mutated, log_original, sizeof(log_mutated));
    store_u64(log_mutated + 88u, 17u);
    reseal(log_mutated);
    if (write_block(&device, first_log + 3u, log_mutated) != 0) {
        return EXIT_FAILURE;
    }
    require_namespace_corrupt(&ops, &scratch, &volume, &view,
                              first_log + 3u,
                              "create object watermark");
    if (write_block(&device, first_log + 3u, log_original) != 0) {
        return EXIT_FAILURE;
    }

    if (read_block(&device, first_log + 1u, log_original) != 0) {
        return EXIT_FAILURE;
    }
    memcpy(log_mutated, log_original, sizeof(log_mutated));
    store_u64(log_mutated + 88u, load_u64(log_original + 88u) + 1u);
    store_u64(log_mutated + 96u, load_u64(log_original + 96u) + 1u);
    reseal(log_mutated);
    if (write_block(&device, first_log + 1u, log_mutated) != 0) {
        return EXIT_FAILURE;
    }
    require_namespace_corrupt(&ops, &scratch, &volume, &view,
                              first_log + 1u,
                              "write expected-size mismatch");
    if (write_block(&device, first_log + 1u, log_original) != 0) {
        return EXIT_FAILURE;
    }
    data_lba = load_u64(log_original + LOG_FIRST_EXTENT_OFFSET);
    if (read_block(&device, data_lba, data_original) != 0) {
        return EXIT_FAILURE;
    }

    memcpy(data_mutated, data_original, sizeof(data_mutated));
    data_mutated[0] ^= UINT8_C(0x80);
    if (write_block(&device, data_lba, data_mutated) != 0) {
        return EXIT_FAILURE;
    }
    status = afspr_scan_intent_log(&ops, &scratch, &volume, &view,
                                   sizeof(view), &diagnostic,
                                   sizeof(diagnostic));
    require_status(status, AFSPR_OK, "torn data scan", &diagnostic);
    if (view.valid_records != 1u ||
        view.tail_state != AFSPR_INTENT_TAIL_CONTENT ||
        view.tail_slot != 1u || view.tail_block != data_lba) {
        fprintf(stderr, "torn data did not terminate the prefix exactly\n");
        return EXIT_FAILURE;
    }
    if (write_block(&device, data_lba, data_original) != 0) {
        return EXIT_FAILURE;
    }

    if (read_block(&device, first_log, log_original) != 0) {
        return EXIT_FAILURE;
    }
    memcpy(log_mutated, log_original, sizeof(log_mutated));
    log_mutated[32] ^= UINT8_C(0x01);
    reseal(log_mutated);
    if (write_block(&device, first_log, log_mutated) != 0) {
        return EXIT_FAILURE;
    }
    status = afspr_scan_intent_log(&ops, &scratch, &volume, &view,
                                   sizeof(view), &diagnostic,
                                   sizeof(diagnostic));
    require_status(status, AFSPR_OK, "stale binding scan", &diagnostic);
    if (view.valid_records != 0u ||
        view.tail_state != AFSPR_INTENT_TAIL_STALE || view.tail_slot != 0u ||
        view.tail_block != first_log) {
        fprintf(stderr, "stale UUID did not terminate the prefix exactly\n");
        return EXIT_FAILURE;
    }
    if (write_block(&device, first_log, log_original) != 0) {
        return EXIT_FAILURE;
    }

    if (read_block(&device, first_log + 1u, log_original) != 0) {
        return EXIT_FAILURE;
    }
    memcpy(log_mutated, log_original, sizeof(log_mutated));
    store_u32(log_mutated + LOG_SEQUENCE_OFFSET, 99u);
    reseal(log_mutated);
    if (write_block(&device, first_log + 1u, log_mutated) != 0) {
        return EXIT_FAILURE;
    }
    status = afspr_scan_intent_log(&ops, &scratch, &volume, &view,
                                   sizeof(view), &diagnostic,
                                   sizeof(diagnostic));
    require_status(status, AFSPR_OK, "sequence scan", &diagnostic);
    if (view.valid_records != 1u ||
        view.tail_state != AFSPR_INTENT_TAIL_SEQUENCE ||
        view.tail_slot != 1u || view.tail_block != first_log + 1u) {
        fprintf(stderr, "sequence gap did not terminate the prefix exactly\n");
        return EXIT_FAILURE;
    }
    if (write_block(&device, first_log + 1u, log_original) != 0) {
        return EXIT_FAILURE;
    }

    saved_features = volume.incompat_features;
    volume.incompat_features &= ~UINT64_C(2);
    status = afspr_scan_intent_log(&ops, &scratch, &volume, &view,
                                   sizeof(view), &diagnostic,
                                   sizeof(diagnostic));
    require_status(status, AFSPR_ERR_CORRUPT, "missing data feature",
                   &diagnostic);
    if (diagnostic.stage != AFSPR_STAGE_INTENT_DECODE ||
        diagnostic.block != first_log + 1u) {
        fprintf(stderr, "feature error lost its intent-log LBA\n");
        return EXIT_FAILURE;
    }
    volume.incompat_features = saved_features;

    scratch.size = AFSPR_INTENT_SCRATCH_SIZE - 1u;
    status = afspr_scan_intent_log(&ops, &scratch, &volume, &view,
                                   sizeof(view), &diagnostic,
                                   sizeof(diagnostic));
    require_status(status, AFSPR_ERR_SCRATCH_TOO_SMALL, "short scratch",
                   &diagnostic);
    scratch.size = sizeof(scratch_bytes);
    status = afspr_scan_intent_log(&ops, &scratch, &volume, &view,
                                   sizeof(view) - 1u, &diagnostic,
                                   sizeof(diagnostic));
    require_status(status, AFSPR_ERR_ABI, "short view", &diagnostic);

    if (fclose(device.file) != 0) {
        return EXIT_FAILURE;
    }
    printf("portable-c-intent PASS records=7 operations=7 first-log=%llu\n",
           (unsigned long long)first_log);
    return EXIT_SUCCESS;
}
