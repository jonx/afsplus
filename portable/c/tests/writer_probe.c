/* SPDX-License-Identifier: BSD-2-Clause */

#define _POSIX_C_SOURCE 200809L

#include "libafsplus_reader.h"
#include "libafsplus_writer.h"

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define BLOCK_SIZE 4096u

struct file_device {
    FILE *file;
    uint64_t blocks;
    uint32_t reads;
    uint32_t writes;
    uint32_t flushes;
    int fail_write;
    int torn_write;
    int fail_flush;
};

static void require(int condition, const char *message)
{
    if (!condition) {
        fprintf(stderr, "portable C writer test failed: %s\n", message);
        exit(1);
    }
}

static int read_blocks(void *opaque, uint64_t first, uint32_t count,
                       void *destination)
{
    struct file_device *device = (struct file_device *)opaque;
    uint64_t offset;
    size_t bytes;
    size_t got;

    if (count == 0u || first >= device->blocks ||
        count > device->blocks - first ||
        first > UINT64_MAX / BLOCK_SIZE) {
        return -1;
    }
    device->reads += count;
    offset = first * BLOCK_SIZE;
    bytes = (size_t)count * BLOCK_SIZE;
    if (offset > (uint64_t)LONG_MAX ||
        fseek(device->file, (long)offset, SEEK_SET) != 0) {
        return -1;
    }
    got = fread(destination, 1u, bytes, device->file);
    if (got < bytes) {
        if (ferror(device->file)) {
            return -1;
        }
        memset((uint8_t *)destination + got, 0, bytes - got);
        clearerr(device->file);
    }
    return 0;
}

static int write_blocks(void *opaque, uint64_t first, uint32_t count,
                        const void *source)
{
    struct file_device *device = (struct file_device *)opaque;
    uint64_t offset;
    size_t bytes;

    ++device->writes;
    if (count != 1u || first >= device->blocks ||
        first > UINT64_MAX / BLOCK_SIZE) {
        return -1;
    }
    offset = first * BLOCK_SIZE;
    bytes = device->torn_write != 0 ? 64u : BLOCK_SIZE;
    if (offset > (uint64_t)LONG_MAX ||
        fseek(device->file, (long)offset, SEEK_SET) != 0 ||
        fwrite(source, 1u, bytes, device->file) != bytes ||
        fflush(device->file) != 0) {
        return -1;
    }
    if (device->fail_write != 0 || device->torn_write != 0) {
        return -1;
    }
    return 0;
}

static int flush_device(void *opaque)
{
    struct file_device *device = (struct file_device *)opaque;

    ++device->flushes;
    if (device->fail_flush != 0) {
        return -1;
    }
    return fflush(device->file) == 0 && fsync(fileno(device->file)) == 0
               ? 0
               : -1;
}

static int lookup(const struct afspr_block_ops *ops,
                  const struct afspr_scratch *scratch,
                  const struct afspr_probe_result *volume,
                  const struct afspr_intent_view *view, const char *name,
                  struct afspr_directory_entry *entry)
{
    uint8_t name_buffer[256];
    struct afspr_diagnostic diagnostic;

    return afspr_lookup_intent_directory_entry(
        ops, scratch, volume, view, UINT64_C(1), name, strlen(name),
        name_buffer, sizeof(name_buffer), entry, sizeof(*entry), &diagnostic,
        sizeof(diagnostic));
}

int main(int argc, char **argv)
{
    struct file_device device;
    struct afspw_block_ops writer_ops;
    struct afspr_block_ops reader_ops;
    struct afspr_scratch scratch;
    struct afspr_probe_result volume;
    struct afspr_intent_view view;
    struct afspr_directory_entry entry;
    struct afspr_diagnostic reader_diagnostic;
    struct afspw_rename_result result;
    struct afspw_diagnostic writer_diagnostic;
    struct afspr_timespec timestamp;
    uint8_t workspace[AFSPW_SCRATCH_SIZE];
    long length;
    const char *mode;
    const char *target = "C-Written.BIN";
    size_t target_len = 13u;
    uint32_t expected_writes;
    uint32_t mutation_reads;
    int status;

    require(argc == 2 || argc == 3,
            "usage: writer_probe IMAGE [success|torn-only|torn-retry|flush-fail|destination-exists]");
    mode = argc == 3 ? argv[2] : "success";
    if (strcmp(mode, "destination-exists") == 0) {
        target = "Replace.TXT";
        target_len = 11u;
    } else {
        require(strcmp(mode, "success") == 0 ||
                    strcmp(mode, "torn-only") == 0 ||
                    strcmp(mode, "torn-retry") == 0 ||
                    strcmp(mode, "flush-fail") == 0,
                "unknown mode");
    }
    memset(&device, 0, sizeof(device));
    device.file = fopen(argv[1], "r+b");
    require(device.file != NULL, "open image");
    require(fseek(device.file, 0L, SEEK_END) == 0, "seek image end");
    length = ftell(device.file);
    require(length > 0 && (unsigned long)length % BLOCK_SIZE == 0u,
            "image geometry");
    device.blocks = (uint64_t)(unsigned long)length / BLOCK_SIZE;

    memset(&writer_ops, 0, sizeof(writer_ops));
    writer_ops.abi_version = AFSPW_ABI_VERSION;
    writer_ops.struct_size = (uint32_t)sizeof(writer_ops);
    writer_ops.ctx = &device;
    writer_ops.read_blocks = read_blocks;
    writer_ops.write_blocks = write_blocks;
    writer_ops.flush = flush_device;
    writer_ops.block_count = device.blocks;
    writer_ops.block_size = BLOCK_SIZE;
    scratch.buffer = workspace;
    scratch.size = sizeof(workspace);
    memset(&timestamp, 0, sizeof(timestamp));
    timestamp.seconds = 10;

    require((afspw_capabilities() & AFSPW_CAP_RENAME_FILE_NO_REPLACE) != 0u,
            "rename capability");
    device.torn_write = strcmp(mode, "torn-only") == 0 ||
                        strcmp(mode, "torn-retry") == 0;
    device.fail_flush = strcmp(mode, "flush-fail") == 0;
    status = afspw_rename_file_no_replace(
        &writer_ops, &scratch, UINT64_C(1), "Final.BIN", 9u, UINT64_C(1),
        target, target_len, &timestamp, &result, sizeof(result),
        &writer_diagnostic, sizeof(writer_diagnostic));
    if (strcmp(mode, "destination-exists") == 0) {
        require(status == AFSPW_ERR_DESTINATION_EXISTS &&
                    writer_diagnostic.stage == AFSPW_STAGE_TARGET_LOOKUP &&
                    device.writes == 0u && device.flushes == 0u,
                "existing destination refused before I/O");
        require(fclose(device.file) == 0, "close destination image");
        printf("portable-c-writer destination-exists=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "torn-only") == 0 ||
        strcmp(mode, "torn-retry") == 0) {
        require(status == AFSPW_ERR_WRITE_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_RECORD_WRITE &&
                    device.writes == 1u && device.flushes == 0u,
                "torn write reported uncertain");
        if (strcmp(mode, "torn-only") == 0) {
            require(fclose(device.file) == 0, "close torn image");
            printf("portable-c-writer torn-only=PASS reads=%u writes=1 flushes=0\n",
                   device.reads);
            return 0;
        }
        device.torn_write = 0;
        clearerr(device.file);
        status = afspw_rename_file_no_replace(
            &writer_ops, &scratch, UINT64_C(1), "Final.BIN", 9u,
            UINT64_C(1), "C-Written.BIN", 13u, &timestamp, &result,
            sizeof(result), &writer_diagnostic, sizeof(writer_diagnostic));
    } else if (strcmp(mode, "flush-fail") == 0) {
        require(status == AFSPW_ERR_DURABILITY_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_FLUSH &&
                    device.writes == 1u && device.flushes == 1u,
                "failed flush reported uncertain");
        device.fail_flush = 0;
    }
    expected_writes = strcmp(mode, "torn-retry") == 0 ? 2u : 1u;
    if (strcmp(mode, "flush-fail") != 0) {
        require(status == AFSPR_OK, afspw_status_string(status));
        require(result.abi_version == AFSPW_ABI_VERSION &&
                    result.prior_records == 7u && result.sequence == 8u &&
                    result.log_slot == 7u && result.base_generation != 0u &&
                    result.log_block != AFSPR_NO_BLOCK,
                "rename result coordinates");
        require(writer_diagnostic.stage == AFSPW_STAGE_COMPLETE &&
                    writer_diagnostic.block == result.log_block,
                "successful completion diagnostic");
    }
    require(device.writes == expected_writes && device.flushes == 1u,
            "bounded write and flush counts");
    mutation_reads = device.reads;

    memset(&reader_ops, 0, sizeof(reader_ops));
    reader_ops.abi_version = AFSPR_ABI_VERSION;
    reader_ops.struct_size = (uint32_t)sizeof(reader_ops);
    reader_ops.ctx = &device;
    reader_ops.read_blocks = read_blocks;
    reader_ops.block_count = device.blocks;
    reader_ops.block_size = BLOCK_SIZE;
    status = afspr_probe_detailed(&reader_ops, &scratch, &volume,
                                  sizeof(volume), &reader_diagnostic,
                                  sizeof(reader_diagnostic));
    require(status == AFSPR_OK, "probe after C write");
    status = afspr_scan_intent_log(&reader_ops, &scratch, &volume, &view,
                                   sizeof(view), &reader_diagnostic,
                                   sizeof(reader_diagnostic));
    require(status == AFSPR_OK && view.valid_records == 8u &&
                view.last_sequence == 8u &&
                view.tail_state == AFSPR_INTENT_TAIL_FULL,
            "C record is the complete durable tail");
    require(lookup(&reader_ops, &scratch, &volume, &view, "Final.BIN",
                   &entry) == AFSPR_ERR_NOT_FOUND,
            "old name absent in durable view");
    require(lookup(&reader_ops, &scratch, &volume, &view, "c-written.bin",
                   &entry) == AFSPR_OK &&
                entry.type_hint == AFSPR_OBJECT_FILE,
            "new name visible in durable view");

    status = afspw_rename_file_no_replace(
        &writer_ops, &scratch, UINT64_C(1), "C-Written.BIN", 13u,
        UINT64_C(1), "No-Room.BIN", 11u, &timestamp, &result,
        sizeof(result), &writer_diagnostic, sizeof(writer_diagnostic));
    require(status == AFSPW_ERR_LOG_FULL &&
                device.writes == expected_writes &&
                device.flushes == 1u,
            "full log refuses without I/O");

    require(fclose(device.file) == 0, "close image");
    printf("portable-c-writer mode=%s result=PASS prior=7 sequence=8 reads=%u writes=%u flushes=%u scratch=8192\n",
           mode, mutation_reads, device.writes, device.flushes);
    return 0;
}
