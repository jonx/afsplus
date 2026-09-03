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
#define TRUNCATE_CURRENT_SIZE (BLOCK_SIZE + 123u)
#define TRUNCATE_GROW_SIZE (3u * BLOCK_SIZE + 7u)

struct file_device {
    FILE *file;
    const char *trace_label;
    uint64_t blocks;
    uint32_t reads;
    uint32_t writes;
    uint32_t flushes;
    uint32_t fail_read_number;
    uint32_t read_failures;
    int fail_write;
    int torn_write;
    int fail_flush;
    uint32_t fail_write_number;
    uint32_t torn_write_number;
    uint32_t fail_flush_number;
    uint64_t fail_read_block;
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
    if (device->writes == 0u && getenv("AFSPLUS_TRACE_WRITER_READS") != NULL) {
        fprintf(stderr,
                "writer-preflight-read mode=%s lba=%llu blocks=%u\n",
                device->trace_label, (unsigned long long)first,
                (unsigned int)count);
    }
    device->reads += count;
    if (count == 1u && first == device->fail_read_block) {
        device->fail_read_block = AFSPR_NO_BLOCK;
        ++device->read_failures;
        return -1;
    }
    if (device->fail_read_number != 0u &&
        device->reads == device->fail_read_number) {
        device->fail_read_number = 0u;
        ++device->read_failures;
        return -1;
    }
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
    bytes = device->torn_write != 0 ||
                    device->writes == device->torn_write_number
                ? 64u
                : BLOCK_SIZE;
    if (offset > (uint64_t)LONG_MAX ||
        fseek(device->file, (long)offset, SEEK_SET) != 0 ||
        fwrite(source, 1u, bytes, device->file) != bytes ||
        fflush(device->file) != 0) {
        return -1;
    }
    if (device->fail_write != 0 || device->torn_write != 0 ||
        device->writes == device->fail_write_number ||
        device->writes == device->torn_write_number) {
        return -1;
    }
    return 0;
}

static int flush_device(void *opaque)
{
    struct file_device *device = (struct file_device *)opaque;

    ++device->flushes;
    if (device->fail_flush != 0 ||
        device->flushes == device->fail_flush_number) {
        return -1;
    }
    return fflush(device->file) == 0 && fsync(fileno(device->file)) == 0
               ? 0
               : -1;
}

static int is_write_mode(const char *mode)
{
    return strcmp(mode, "write") == 0 ||
           strcmp(mode, "write-low-memory") == 0 ||
           strcmp(mode, "write-invalid-range") == 0 ||
           strcmp(mode, "write-nonzero-tail") == 0 ||
           strcmp(mode, "write-no-feature") == 0 ||
           strcmp(mode, "write-data-torn-retry") == 0 ||
           strcmp(mode, "write-data-flush-fail") == 0 ||
           strcmp(mode, "write-record-torn-retry") == 0 ||
           strcmp(mode, "write-record-flush-fail") == 0 ||
           strcmp(mode, "write-descriptor-read-fail") == 0 ||
           strcmp(mode, "write-bitmap-read-fail") == 0 ||
           strcmp(mode, "write-no-space") == 0;
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
    struct afspw_create_result create_result;
    struct afspw_truncate_result truncate_result;
    struct afspw_write_result write_result;
    struct afspw_diagnostic writer_diagnostic;
    struct afspr_timespec timestamp;
    uint8_t workspace[AFSPW_RECOMMENDED_SCRATCH_SIZE];
    uint8_t write_data[BLOCK_SIZE];
    long length;
    const char *mode;
    const char *target = "C-Written.BIN";
    size_t target_len = 13u;
    size_t index;
    uint32_t expected_writes;
    uint32_t mutation_reads;
    int status;

    require(argc == 2 || argc == 3,
            "usage: writer_probe IMAGE "
            "[success|low-memory|read-fail-retry|create|create-low-memory|"
            "create-exists|create-exhausted|create-bad-watermark|"
            "create-missing-parent|truncate-zero|truncate-grow|"
            "truncate-zero-low-memory|truncate-noop|"
            "truncate-tail-required|truncate-no-feature|"
            "truncate-missing|truncate-directory|"
            "truncate-torn-retry|truncate-flush-fail|delete|replace|"
            "torn-only|torn-retry|flush-fail|"
            "destination-exists|write|write-low-memory|"
            "write-invalid-range|write-nonzero-tail|write-no-feature|"
            "write-data-torn-retry|write-data-flush-fail|"
            "write-record-torn-retry|write-record-flush-fail|"
            "write-descriptor-read-fail|write-bitmap-read-fail|"
            "write-no-space]");
    mode = argc == 3 ? argv[2] : "success";
    if (strcmp(mode, "destination-exists") == 0) {
        target = "Replace.TXT";
        target_len = 11u;
    } else {
        require(strcmp(mode, "success") == 0 ||
                    strcmp(mode, "low-memory") == 0 ||
                    strcmp(mode, "read-fail-retry") == 0 ||
                    strcmp(mode, "create") == 0 ||
                    strcmp(mode, "create-low-memory") == 0 ||
                    strcmp(mode, "create-exists") == 0 ||
                    strcmp(mode, "create-exhausted") == 0 ||
                    strcmp(mode, "create-bad-watermark") == 0 ||
                    strcmp(mode, "create-missing-parent") == 0 ||
                    strcmp(mode, "truncate-zero") == 0 ||
                    strcmp(mode, "truncate-grow") == 0 ||
                    strcmp(mode, "truncate-zero-low-memory") == 0 ||
                    strcmp(mode, "truncate-noop") == 0 ||
                    strcmp(mode, "truncate-tail-required") == 0 ||
                    strcmp(mode, "truncate-no-feature") == 0 ||
                    strcmp(mode, "truncate-missing") == 0 ||
                    strcmp(mode, "truncate-directory") == 0 ||
                    strcmp(mode, "truncate-torn-retry") == 0 ||
                    strcmp(mode, "truncate-flush-fail") == 0 ||
                    strcmp(mode, "delete") == 0 ||
                    strcmp(mode, "replace") == 0 ||
                    strcmp(mode, "torn-only") == 0 ||
                    strcmp(mode, "torn-retry") == 0 ||
                    strcmp(mode, "flush-fail") == 0 ||
                    is_write_mode(mode),
                "unknown mode");
    }
    memset(&device, 0, sizeof(device));
    device.fail_read_block = AFSPR_NO_BLOCK;
    device.trace_label = mode;
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
    scratch.size = strcmp(mode, "low-memory") == 0 ||
                           strcmp(mode, "create-low-memory") == 0 ||
                           strcmp(mode, "truncate-zero-low-memory") == 0 ||
                           strcmp(mode, "write-low-memory") == 0
                       ? AFSPW_SCRATCH_SIZE
                       : sizeof(workspace);
    memset(&result, 0, sizeof(result));
    memset(&create_result, 0, sizeof(create_result));
    memset(&truncate_result, 0, sizeof(truncate_result));
    memset(&write_result, 0, sizeof(write_result));
    memset(write_data, 0, sizeof(write_data));
    memset(write_data, 0x3c, TRUNCATE_CURRENT_SIZE - BLOCK_SIZE);
    memset(&timestamp, 0, sizeof(timestamp));
    timestamp.seconds = 10;

    require((afspw_capabilities() & AFSPW_CAP_RENAME_FILE_NO_REPLACE) != 0u,
            "rename capability");
    require((afspw_capabilities() & AFSPW_CAP_DELETE_FILE) != 0u,
            "delete capability");
    require((afspw_capabilities() & AFSPW_CAP_RENAME_FILE_REPLACE) != 0u,
            "replace capability");
    require((afspw_capabilities() & AFSPW_CAP_CREATE_EMPTY_FILE) != 0u,
            "empty create capability");
    require((afspw_capabilities() &
             AFSPW_CAP_TRUNCATE_FILE_DATA_FREE) != 0u,
            "data-free truncate capability");
    require((afspw_capabilities() & AFSPW_CAP_WRITE_FILE_BLOCK_COW) != 0u,
            "one-block COW write capability");
    require(strcmp(afspw_status_string(AFSPW_ERR_OBJECT_ID_EXHAUSTED),
                   "object ID space exhausted") == 0 &&
                strcmp(afspw_stage_string(AFSPW_STAGE_CREATE_LOOKUP),
                       "create-lookup") == 0,
            "create status and stage strings");
    require(strcmp(
                afspw_status_string(AFSPW_ERR_TAIL_REWRITE_REQUIRED),
                "truncate tail rewrite required") == 0 &&
                strcmp(afspw_stage_string(AFSPW_STAGE_FILE_STATE),
                       "file-state") == 0,
            "truncate status and stage strings");
    require(strcmp(afspw_status_string(AFSPW_ERR_NO_SPACE),
                   "no space for ordinary growth") == 0 &&
                strcmp(afspw_status_string(AFSPW_ERR_NONZERO_TAIL),
                       "nonzero data beyond file end") == 0 &&
                strcmp(afspw_stage_string(AFSPW_STAGE_ALLOCATION),
                       "allocation") == 0 &&
                strcmp(afspw_stage_string(AFSPW_STAGE_DATA_FLUSH),
                       "data-flush") == 0,
            "COW write status and stage strings");
    device.torn_write = strcmp(mode, "torn-only") == 0 ||
                        strcmp(mode, "torn-retry") == 0 ||
                        strcmp(mode, "truncate-torn-retry") == 0;
    device.fail_flush = strcmp(mode, "flush-fail") == 0 ||
                        strcmp(mode, "truncate-flush-fail") == 0;
    device.fail_read_number =
        strcmp(mode, "read-fail-retry") == 0 ? 4u : 0u;
    if (strcmp(mode, "write-descriptor-read-fail") == 0) {
        device.fail_read_block = UINT64_C(4);
    } else if (strcmp(mode, "write-bitmap-read-fail") == 0) {
        device.fail_read_block = UINT64_C(7);
    }
    device.torn_write_number =
        strcmp(mode, "write-data-torn-retry") == 0
            ? 1u
            : (strcmp(mode, "write-record-torn-retry") == 0 ? 2u : 0u);
    device.fail_flush_number =
        strcmp(mode, "write-data-flush-fail") == 0
            ? 1u
            : (strcmp(mode, "write-record-flush-fail") == 0 ? 2u : 0u);
    if (strcmp(mode, "create") == 0 ||
        strcmp(mode, "create-low-memory") == 0 ||
        strcmp(mode, "create-exists") == 0 ||
        strcmp(mode, "create-exhausted") == 0 ||
        strcmp(mode, "create-bad-watermark") == 0 ||
        strcmp(mode, "create-missing-parent") == 0) {
        const char *create_name =
            (strcmp(mode, "create") == 0 ||
             strcmp(mode, "create-low-memory") == 0)
                ? "C-Empty.BIN"
                : (strcmp(mode, "create-exists") == 0 ? "fInAl.bIn"
                                                       : "New-Empty.BIN");
        uint64_t create_parent =
            strcmp(mode, "create-missing-parent") == 0 ? UINT64_C(999999)
                                                        : UINT64_C(1);

        status = afspw_create_empty_file(
            &writer_ops, &scratch, create_parent, create_name,
            strlen(create_name), &timestamp, &create_result,
            sizeof(create_result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (strcmp(mode, "truncate-zero") == 0 ||
               strcmp(mode, "truncate-grow") == 0 ||
               strcmp(mode, "truncate-zero-low-memory") == 0 ||
               strcmp(mode, "truncate-noop") == 0 ||
               strcmp(mode, "truncate-tail-required") == 0 ||
               strcmp(mode, "truncate-no-feature") == 0 ||
               strcmp(mode, "truncate-missing") == 0 ||
               strcmp(mode, "truncate-directory") == 0 ||
               strcmp(mode, "truncate-torn-retry") == 0 ||
               strcmp(mode, "truncate-flush-fail") == 0) {
        uint64_t truncate_size =
            strcmp(mode, "truncate-zero") == 0 ||
                    strcmp(mode, "truncate-zero-low-memory") == 0 ||
                    strcmp(mode, "truncate-torn-retry") == 0 ||
                    strcmp(mode, "truncate-flush-fail") == 0 ||
                    strcmp(mode, "truncate-no-feature") == 0
                ? UINT64_C(0)
                : (strcmp(mode, "truncate-grow") == 0
                       ? (uint64_t)TRUNCATE_GROW_SIZE
                       : (strcmp(mode, "truncate-noop") == 0
                              ? (uint64_t)TRUNCATE_CURRENT_SIZE
                              : (uint64_t)BLOCK_SIZE + UINT64_C(1)));
        uint64_t truncate_object =
            strcmp(mode, "truncate-missing") == 0
                ? UINT64_C(999999)
                : (strcmp(mode, "truncate-directory") == 0 ? UINT64_C(1)
                                                            : UINT64_C(16));

        status = afspw_truncate_file(
            &writer_ops, &scratch, truncate_object, truncate_size, &timestamp,
            &truncate_result, sizeof(truncate_result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (is_write_mode(mode)) {
        uint64_t requested_size =
            strcmp(mode, "write-invalid-range") == 0
                ? (uint64_t)BLOCK_SIZE
                : (uint64_t)TRUNCATE_CURRENT_SIZE;

        if (strcmp(mode, "write-nonzero-tail") == 0) {
            write_data[TRUNCATE_CURRENT_SIZE - BLOCK_SIZE] = 1u;
        }
        status = afspw_write_file_block_cow(
            &writer_ops, &scratch, UINT64_C(16), UINT64_C(1), write_data,
            sizeof(write_data), requested_size, &timestamp, &write_result,
            sizeof(write_result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (strcmp(mode, "delete") == 0) {
        status = afspw_delete_file(
            &writer_ops, &scratch, UINT64_C(1), "Final.BIN", 9u,
            &timestamp, &result, sizeof(result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (strcmp(mode, "replace") == 0) {
        status = afspw_rename_file_replace(
            &writer_ops, &scratch, UINT64_C(1), "Replace.TXT", 11u,
            UINT64_C(1), "Final.BIN", 9u, &timestamp, &result,
            sizeof(result), &writer_diagnostic, sizeof(writer_diagnostic));
    } else {
        status = afspw_rename_file_no_replace(
            &writer_ops, &scratch, UINT64_C(1), "Final.BIN", 9u,
            UINT64_C(1), target, target_len, &timestamp, &result,
            sizeof(result), &writer_diagnostic, sizeof(writer_diagnostic));
    }
    if (strcmp(mode, "destination-exists") == 0) {
        require(status == AFSPW_ERR_DESTINATION_EXISTS &&
                    writer_diagnostic.stage == AFSPW_STAGE_TARGET_LOOKUP &&
                    device.reads <= 21u && device.writes == 0u &&
                    device.flushes == 0u,
                "existing destination refused before I/O");
        require(fclose(device.file) == 0, "close destination image");
        printf("portable-c-writer destination-exists=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "create-exists") == 0) {
        require(status == AFSPW_ERR_DESTINATION_EXISTS &&
                    writer_diagnostic.stage == AFSPW_STAGE_CREATE_LOOKUP &&
                    device.reads <= 21u && device.writes == 0u &&
                    device.flushes == 0u,
                "existing create name refused before I/O");
        require(fclose(device.file) == 0, "close create conflict image");
        printf("portable-c-writer create-exists=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "create-exhausted") == 0) {
        require(status == AFSPW_ERR_OBJECT_ID_EXHAUSTED &&
                    writer_diagnostic.stage == AFSPW_STAGE_CREATE_LOOKUP &&
                    device.reads <= 9u && device.writes == 0u &&
                    device.flushes == 0u,
                "exhausted object IDs refused before I/O");
        require(fclose(device.file) == 0, "close exhausted image");
        printf("portable-c-writer create-exhausted=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "create-bad-watermark") == 0) {
        require(status == AFSPR_ERR_CORRUPT &&
                    writer_diagnostic.stage == AFSPW_STAGE_CREATE_LOOKUP &&
                    writer_diagnostic.reader_status == AFSPR_ERR_CORRUPT &&
                    writer_diagnostic.block != AFSPR_NO_BLOCK &&
                    device.writes == 0u && device.flushes == 0u,
                "regressed object watermark refused before I/O");
        require(fclose(device.file) == 0, "close bad-watermark image");
        printf("portable-c-writer create-bad-watermark=PASS reads=%u writes=0 flushes=0 block=%llu\n",
               device.reads,
               (unsigned long long)writer_diagnostic.block);
        return 0;
    }
    if (strcmp(mode, "create-missing-parent") == 0) {
        require(status == AFSPR_ERR_NOT_FOUND &&
                    writer_diagnostic.stage == AFSPW_STAGE_CREATE_LOOKUP &&
                    writer_diagnostic.reader_status == AFSPR_ERR_NOT_FOUND &&
                    device.writes == 0u && device.flushes == 0u,
                "missing create parent refused before I/O");
        require(fclose(device.file) == 0, "close missing-parent image");
        printf("portable-c-writer create-missing-parent=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "truncate-no-feature") == 0) {
        require(status == AFSPW_ERR_WRITE_FEATURE &&
                    writer_diagnostic.stage == AFSPW_STAGE_PROBE &&
                    writer_diagnostic.reader_status == AFSPR_ERR_UNSUPPORTED &&
                    device.writes == 0u && device.flushes == 0u,
                "data-update feature required before truncate I/O");
        require(fclose(device.file) == 0, "close no-feature image");
        printf("portable-c-writer truncate-no-feature=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "truncate-noop") == 0) {
        require(status == AFSPR_OK &&
                    truncate_result.abi_version == AFSPW_ABI_VERSION &&
                    truncate_result.prior_records == 7u &&
                    truncate_result.sequence == 0u &&
                    truncate_result.log_slot == AFSPR_NO_LOG_SLOT &&
                    truncate_result.base_generation != 0u &&
                    truncate_result.log_block == AFSPR_NO_BLOCK &&
                    truncate_result.previous_size ==
                        (uint64_t)TRUNCATE_CURRENT_SIZE &&
                    truncate_result.new_size ==
                        (uint64_t)TRUNCATE_CURRENT_SIZE &&
                    truncate_result.record_written == 0u &&
                    writer_diagnostic.stage == AFSPW_STAGE_COMPLETE &&
                    device.reads <= 21u && device.writes == 0u &&
                    device.flushes == 0u,
                "same-size truncate is a bounded no-op");
        require(fclose(device.file) == 0, "close truncate no-op image");
        printf("portable-c-writer truncate-noop=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "truncate-tail-required") == 0) {
        require(status == AFSPW_ERR_TAIL_REWRITE_REQUIRED &&
                    writer_diagnostic.stage == AFSPW_STAGE_FILE_STATE &&
                    writer_diagnostic.reader_status == AFSPR_OK &&
                    device.reads <= 21u && device.writes == 0u &&
                    device.flushes == 0u,
                "unaligned shrink requires safe tail rewrite");
        require(fclose(device.file) == 0,
                "close tail-rewrite-required image");
        printf("portable-c-writer truncate-tail-required=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "truncate-missing") == 0 ||
        strcmp(mode, "truncate-directory") == 0) {
        int expected_status = strcmp(mode, "truncate-missing") == 0
                                  ? AFSPR_ERR_NOT_FOUND
                                  : AFSPR_ERR_NOT_FILE;

        require(status == expected_status &&
                    writer_diagnostic.stage == AFSPW_STAGE_FILE_STATE &&
                    writer_diagnostic.reader_status == expected_status &&
                    device.reads <= 21u && device.writes == 0u &&
                    device.flushes == 0u,
                "truncate object type/existence diagnostics");
        require(fclose(device.file) == 0, "close invalid truncate image");
        printf("portable-c-writer mode=%s result=PASS reads=%u writes=0 flushes=0\n",
               mode, device.reads);
        return 0;
    }
    if (strcmp(mode, "write-invalid-range") == 0) {
        require(status == AFSPW_ERR_INVALID_WRITE_RANGE &&
                    writer_diagnostic.stage == AFSPW_STAGE_FILE_STATE &&
                    writer_diagnostic.reader_status == AFSPR_OK &&
                    device.writes == 0u && device.flushes == 0u,
                "shrinking complete-block write refused before I/O");
        require(fclose(device.file) == 0, "close invalid write image");
        printf("portable-c-writer write-invalid-range=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "write-nonzero-tail") == 0) {
        require(status == AFSPW_ERR_NONZERO_TAIL &&
                    writer_diagnostic.stage == AFSPW_STAGE_FILE_STATE &&
                    writer_diagnostic.reader_status == AFSPR_OK &&
                    device.writes == 0u && device.flushes == 0u,
                "nonzero bytes beyond EOF refused before I/O");
        require(fclose(device.file) == 0, "close nonzero-tail image");
        printf("portable-c-writer write-nonzero-tail=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "write-no-feature") == 0) {
        require(status == AFSPW_ERR_WRITE_FEATURE &&
                    writer_diagnostic.stage == AFSPW_STAGE_PROBE &&
                    writer_diagnostic.reader_status == AFSPR_ERR_UNSUPPORTED &&
                    device.writes == 0u && device.flushes == 0u,
                "data-update feature required before COW write I/O");
        require(fclose(device.file) == 0, "close no-feature write image");
        printf("portable-c-writer write-no-feature=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "write-no-space") == 0) {
        require(status == AFSPW_ERR_NO_SPACE &&
                    writer_diagnostic.stage == AFSPW_STAGE_ALLOCATION &&
                    writer_diagnostic.reader_status == AFSPR_OK &&
                    device.writes == 0u && device.flushes == 0u,
                "emergency floor refuses COW data before I/O");
        require(fclose(device.file) == 0, "close no-space write image");
        printf("portable-c-writer write-no-space=PASS reads=%u writes=0 flushes=0\n",
               device.reads);
        return 0;
    }
    if (strcmp(mode, "write-descriptor-read-fail") == 0 ||
        strcmp(mode, "write-bitmap-read-fail") == 0) {
        uint64_t failed_block =
            strcmp(mode, "write-descriptor-read-fail") == 0
                ? UINT64_C(4)
                : UINT64_C(7);

        require(status == AFSPR_ERR_IO &&
                    writer_diagnostic.stage == AFSPW_STAGE_ALLOCATION &&
                    writer_diagnostic.reader_status == AFSPR_ERR_IO &&
                    writer_diagnostic.block == failed_block &&
                    device.read_failures == 1u && device.writes == 0u &&
                    device.flushes == 0u,
                "allocation metadata read failure has exact coordinates");
        require(fclose(device.file) == 0,
                "close allocation read-failure image");
        printf("portable-c-writer mode=%s result=PASS reads=%u writes=0 flushes=0 block=%llu\n",
               mode, device.reads, (unsigned long long)failed_block);
        return 0;
    }
    if (strcmp(mode, "read-fail-retry") == 0) {
        require(status == AFSPR_ERR_IO &&
                    writer_diagnostic.stage == AFSPW_STAGE_INTENT_SCAN &&
                    writer_diagnostic.reader_status == AFSPR_ERR_IO &&
                    writer_diagnostic.block == UINT64_C(16) &&
                    device.read_failures == 1u && device.writes == 0u &&
                    device.flushes == 0u,
                "read failure reports exact stage and leaves media untouched");
        status = afspw_rename_file_no_replace(
            &writer_ops, &scratch, UINT64_C(1), "Final.BIN", 9u,
            UINT64_C(1), target, target_len, &timestamp, &result,
            sizeof(result), &writer_diagnostic, sizeof(writer_diagnostic));
    }
    if (strcmp(mode, "truncate-torn-retry") == 0) {
        require(status == AFSPW_ERR_WRITE_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_RECORD_WRITE &&
                    device.writes == 1u && device.flushes == 0u,
                "torn truncate record reported uncertain");
        device.torn_write = 0;
        clearerr(device.file);
        status = afspw_truncate_file(
            &writer_ops, &scratch, UINT64_C(16), UINT64_C(0), &timestamp,
            &truncate_result, sizeof(truncate_result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (strcmp(mode, "truncate-flush-fail") == 0) {
        require(status == AFSPW_ERR_DURABILITY_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_FLUSH &&
                    device.writes == 1u && device.flushes == 1u,
                "failed truncate flush reported uncertain");
        device.fail_flush = 0;
    }
    if (strcmp(mode, "write-data-torn-retry") == 0) {
        require(status == AFSPW_ERR_DATA_WRITE_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_DATA_WRITE &&
                    writer_diagnostic.block != AFSPR_NO_BLOCK &&
                    device.writes == 1u && device.flushes == 0u,
                "torn COW data write reported uncertain before record");
        device.torn_write_number = 0u;
        clearerr(device.file);
        status = afspw_write_file_block_cow(
            &writer_ops, &scratch, UINT64_C(16), UINT64_C(1), write_data,
            sizeof(write_data), (uint64_t)TRUNCATE_CURRENT_SIZE, &timestamp,
            &write_result, sizeof(write_result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (strcmp(mode, "write-data-flush-fail") == 0) {
        require(status == AFSPW_ERR_DATA_DURABILITY_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_DATA_FLUSH &&
                    writer_diagnostic.block != AFSPR_NO_BLOCK &&
                    device.writes == 1u && device.flushes == 1u,
                "failed COW data flush stops before record");
        device.fail_flush_number = 0u;
        require(fclose(device.file) == 0, "close data-flush image");
        printf("portable-c-writer write-data-flush-fail=PASS reads=%u writes=1 flushes=1\n",
               device.reads);
        return 0;
    } else if (strcmp(mode, "write-record-torn-retry") == 0) {
        require(status == AFSPW_ERR_WRITE_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_RECORD_WRITE &&
                    device.writes == 2u && device.flushes == 1u,
                "torn COW record reported after durable data");
        device.torn_write_number = 0u;
        clearerr(device.file);
        status = afspw_write_file_block_cow(
            &writer_ops, &scratch, UINT64_C(16), UINT64_C(1), write_data,
            sizeof(write_data), (uint64_t)TRUNCATE_CURRENT_SIZE, &timestamp,
            &write_result, sizeof(write_result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (strcmp(mode, "write-record-flush-fail") == 0) {
        require(status == AFSPW_ERR_DURABILITY_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_FLUSH &&
                    writer_diagnostic.block != AFSPR_NO_BLOCK &&
                    device.writes == 2u && device.flushes == 2u,
                "failed COW record flush reported uncertain");
        device.fail_flush_number = 0u;
    }
    if (strcmp(mode, "torn-only") == 0 ||
        strcmp(mode, "torn-retry") == 0) {
        require(status == AFSPW_ERR_WRITE_UNCERTAIN &&
                    writer_diagnostic.stage == AFSPW_STAGE_RECORD_WRITE &&
                    device.writes == 1u && device.flushes == 0u,
                "torn write reported uncertain");
        if (strcmp(mode, "torn-only") == 0) {
            require(device.reads <= 21u,
                    "cached torn preflight read ceiling");
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
    expected_writes =
        strcmp(mode, "write-record-torn-retry") == 0
            ? 4u
            : (strcmp(mode, "write-data-torn-retry") == 0
                   ? 3u
                   : (is_write_mode(mode) ? 2u
                                          : (strcmp(mode, "torn-retry") == 0 ||
                                                     strcmp(mode, "truncate-torn-retry") == 0
                                                 ? 2u
                                                 : 1u)));
    if (strcmp(mode, "flush-fail") != 0 &&
        strcmp(mode, "truncate-flush-fail") != 0 &&
        strcmp(mode, "write-record-flush-fail") != 0) {
        require(status == AFSPR_OK, afspw_status_string(status));
        if (strcmp(mode, "create") == 0 ||
            strcmp(mode, "create-low-memory") == 0) {
            require(create_result.abi_version == AFSPW_ABI_VERSION &&
                        create_result.prior_records == 7u &&
                        create_result.sequence == 8u &&
                        create_result.log_slot == 7u &&
                        create_result.base_generation != 0u &&
                        create_result.log_block != AFSPR_NO_BLOCK &&
                        create_result.object_id == UINT64_C(19),
                    "create result coordinates and monotone object ID");
            require(writer_diagnostic.stage == AFSPW_STAGE_COMPLETE &&
                        writer_diagnostic.block == create_result.log_block,
                    "successful create diagnostic");
        } else if (strcmp(mode, "truncate-zero") == 0 ||
                   strcmp(mode, "truncate-grow") == 0 ||
                   strcmp(mode, "truncate-zero-low-memory") == 0 ||
                   strcmp(mode, "truncate-torn-retry") == 0) {
            uint64_t expected_size = strcmp(mode, "truncate-grow") == 0
                                         ? (uint64_t)TRUNCATE_GROW_SIZE
                                         : UINT64_C(0);

            require(truncate_result.abi_version == AFSPW_ABI_VERSION &&
                        truncate_result.prior_records == 7u &&
                        truncate_result.sequence == 8u &&
                        truncate_result.log_slot == 7u &&
                        truncate_result.base_generation != 0u &&
                        truncate_result.log_block != AFSPR_NO_BLOCK &&
                        truncate_result.previous_size ==
                            (uint64_t)TRUNCATE_CURRENT_SIZE &&
                        truncate_result.new_size == expected_size &&
                        truncate_result.record_written == 1u &&
                        truncate_result.reserved == 0u,
                    "truncate result coordinates and sizes");
            require(writer_diagnostic.stage == AFSPW_STAGE_COMPLETE &&
                        writer_diagnostic.block == truncate_result.log_block,
                    "successful truncate diagnostic");
        } else if (is_write_mode(mode)) {
            require(write_result.abi_version == AFSPW_ABI_VERSION &&
                        write_result.prior_records == 7u &&
                        write_result.sequence == 8u &&
                        write_result.log_slot == 7u &&
                        write_result.base_generation != 0u &&
                        write_result.log_block != AFSPR_NO_BLOCK &&
                        write_result.data_block != AFSPR_NO_BLOCK &&
                        write_result.data_block != write_result.log_block &&
                        write_result.logical_block == 1u &&
                        write_result.previous_size ==
                            (uint64_t)TRUNCATE_CURRENT_SIZE &&
                        write_result.new_size ==
                            (uint64_t)TRUNCATE_CURRENT_SIZE &&
                        write_result.data_blocks == 1u &&
                        write_result.reserved == 0u,
                    "COW write result coordinates and sizes");
            require(writer_diagnostic.stage == AFSPW_STAGE_COMPLETE &&
                        writer_diagnostic.block == write_result.log_block,
                    "successful COW write diagnostic");
        } else {
            require(result.abi_version == AFSPW_ABI_VERSION &&
                        result.prior_records == 7u &&
                        result.sequence == 8u && result.log_slot == 7u &&
                        result.base_generation != 0u &&
                        result.log_block != AFSPR_NO_BLOCK,
                    "rename result coordinates");
            require(writer_diagnostic.stage == AFSPW_STAGE_COMPLETE &&
                        writer_diagnostic.block == result.log_block,
                    "successful completion diagnostic");
        }
    }
    require(device.writes == expected_writes &&
                device.flushes ==
                    (is_write_mode(mode)
                         ? (strcmp(mode, "write-record-torn-retry") == 0
                                ? 3u
                                : 2u)
                         : 1u),
            "bounded write and flush counts");
    mutation_reads = device.reads;
    if (strcmp(mode, "create-low-memory") == 0) {
        require(mutation_reads <= 144u,
                "8 KiB create preflight read ceiling");
    } else if (strcmp(mode, "truncate-zero-low-memory") == 0) {
        require(mutation_reads <= 178u,
                "8 KiB truncate preflight read ceiling");
    } else if (strcmp(mode, "low-memory") == 0) {
        require(mutation_reads <= 152u,
                "8 KiB fallback preflight read ceiling");
    } else if (strcmp(mode, "torn-retry") == 0 ||
               strcmp(mode, "truncate-torn-retry") == 0) {
        require(mutation_reads <= 42u,
                "two cached preflights read ceiling");
    } else if (strcmp(mode, "read-fail-retry") == 0) {
        require(mutation_reads <= 25u,
                "failed-read plus cached retry read ceiling");
    } else if (strcmp(mode, "write-low-memory") == 0) {
        require(mutation_reads <= 223u,
                "8 KiB COW write preflight read ceiling");
    } else if (strcmp(mode, "write-data-torn-retry") == 0 ||
               strcmp(mode, "write-record-torn-retry") == 0) {
        require(mutation_reads <= 50u,
                "two cached COW write preflights read ceiling");
    } else if (is_write_mode(mode)) {
        require(mutation_reads <= 25u,
                "cached COW write preflight read ceiling");
    } else {
        require(mutation_reads <= 21u,
                "cached preflight read ceiling");
    }

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
    if (strcmp(mode, "create") == 0 ||
        strcmp(mode, "create-low-memory") == 0) {
        uint64_t empty_size;

        require(lookup(&reader_ops, &scratch, &volume, &view,
                       "C-Empty.BIN", &entry) == AFSPR_OK &&
                    entry.type_hint == AFSPR_OBJECT_FILE &&
                    entry.object_id == create_result.object_id,
                "created empty file visible in durable view");
        require(afspr_intent_file_size(
                    &reader_ops, &scratch, &volume, &view, entry.object_id,
                    &empty_size, &reader_diagnostic,
                    sizeof(reader_diagnostic)) == AFSPR_OK &&
                    empty_size == 0u,
                "created file has zero durable length");
    } else if (strcmp(mode, "truncate-zero") == 0 ||
               strcmp(mode, "truncate-grow") == 0 ||
               strcmp(mode, "truncate-zero-low-memory") == 0 ||
               strcmp(mode, "truncate-torn-retry") == 0 ||
               strcmp(mode, "truncate-flush-fail") == 0) {
        uint64_t expected_size = strcmp(mode, "truncate-grow") == 0
                                     ? (uint64_t)TRUNCATE_GROW_SIZE
                                     : UINT64_C(0);
        uint64_t actual_size;

        require(afspr_intent_file_size(
                    &reader_ops, &scratch, &volume, &view, UINT64_C(16),
                    &actual_size, &reader_diagnostic,
                    sizeof(reader_diagnostic)) == AFSPR_OK &&
                    actual_size == expected_size,
                "truncated size visible in durable view");
    } else if (is_write_mode(mode)) {
        uint8_t readback[TRUNCATE_CURRENT_SIZE - BLOCK_SIZE];
        size_t bytes_read = 0u;

        require(afspr_read_intent_file(
                    &reader_ops, &scratch, &volume, &view, UINT64_C(16),
                    (uint64_t)BLOCK_SIZE, readback, sizeof(readback),
                    &bytes_read, &reader_diagnostic,
                    sizeof(reader_diagnostic)) == AFSPR_OK &&
                    bytes_read == sizeof(readback),
                "COW-written block readable in durable view");
        for (index = 0u; index < sizeof(readback); ++index) {
            require(readback[index] == 0x3cu,
                    "COW-written bytes match caller block");
        }
    } else if (strcmp(mode, "delete") == 0) {
        require(lookup(&reader_ops, &scratch, &volume, &view, "Final.BIN",
                       &entry) == AFSPR_ERR_NOT_FOUND,
                "deleted name absent in durable view");
        require(lookup(&reader_ops, &scratch, &volume, &view, "Replace.TXT",
                       &entry) == AFSPR_OK,
                "unrelated file remains after delete");
    } else if (strcmp(mode, "replace") == 0) {
        require(lookup(&reader_ops, &scratch, &volume, &view, "Replace.TXT",
                       &entry) == AFSPR_ERR_NOT_FOUND,
                "replacement source absent in durable view");
        require(lookup(&reader_ops, &scratch, &volume, &view, "Final.BIN",
                       &entry) == AFSPR_OK &&
                    entry.type_hint == AFSPR_OBJECT_FILE,
                "replacement target visible in durable view");
    } else {
        require(lookup(&reader_ops, &scratch, &volume, &view, "Final.BIN",
                       &entry) == AFSPR_ERR_NOT_FOUND,
                "old name absent in durable view");
        require(lookup(&reader_ops, &scratch, &volume, &view,
                       "c-written.bin", &entry) == AFSPR_OK &&
                    entry.type_hint == AFSPR_OBJECT_FILE,
                "new name visible in durable view");
    }

    if (is_write_mode(mode)) {
        status = afspw_write_file_block_cow(
            &writer_ops, &scratch, UINT64_C(16), UINT64_C(1), write_data,
            sizeof(write_data), (uint64_t)TRUNCATE_CURRENT_SIZE, &timestamp,
            &write_result, sizeof(write_result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    } else if (strcmp(mode, "truncate-zero") == 0 ||
        strcmp(mode, "truncate-grow") == 0 ||
        strcmp(mode, "truncate-zero-low-memory") == 0 ||
        strcmp(mode, "truncate-torn-retry") == 0 ||
        strcmp(mode, "truncate-flush-fail") == 0) {
        status = afspw_truncate_file(
            &writer_ops, &scratch, UINT64_C(16), (uint64_t)BLOCK_SIZE,
            &timestamp, &truncate_result, sizeof(truncate_result),
            &writer_diagnostic, sizeof(writer_diagnostic));
    } else {
        status = afspw_rename_file_no_replace(
            &writer_ops, &scratch, UINT64_C(1), "C-Written.BIN", 13u,
            UINT64_C(1), "No-Room.BIN", 11u, &timestamp, &result,
            sizeof(result), &writer_diagnostic,
            sizeof(writer_diagnostic));
    }
    require(status == AFSPW_ERR_LOG_FULL &&
                writer_diagnostic.stage == AFSPW_STAGE_INTENT_SCAN &&
                device.writes == expected_writes &&
                device.flushes == (is_write_mode(mode) ?
                                       (strcmp(mode, "write-record-torn-retry") == 0
                                            ? 3u
                                            : 2u)
                                                        : 1u),
            "full log refuses without I/O");

    require(fclose(device.file) == 0, "close image");
    printf("portable-c-writer mode=%s result=PASS prior=7 sequence=8 reads=%u writes=%u flushes=%u scratch=%zu\n",
           mode, mutation_reads, device.writes, device.flushes,
           scratch.size);
    return 0;
}
