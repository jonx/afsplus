/* SPDX-License-Identifier: BSD-2-Clause */

#include "harness.h"

#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define AFSPR_FUZZ_MAX_RECORDED_BLOCKS 128u

struct trace_device {
    FILE *file;
    uint64_t block_count;
    uint64_t lbas[AFSPR_FUZZ_MAX_RECORDED_BLOCKS];
    uint8_t *blocks;
    uint32_t used;
};

static void store_u32(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
}

static void store_u64(uint8_t *bytes, uint64_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
    bytes[4] = (uint8_t)(value >> 32);
    bytes[5] = (uint8_t)(value >> 40);
    bytes[6] = (uint8_t)(value >> 48);
    bytes[7] = (uint8_t)(value >> 56);
}

static int parse_u64(const char *text, uint64_t *value)
{
    char *end = NULL;
    unsigned long long parsed;

    errno = 0;
    parsed = strtoull(text, &end, 0);
    if (errno != 0 || end == text || *end != '\0') {
        return -1;
    }
    *value = (uint64_t)parsed;
    return 0;
}

static int trace_read(void *opaque, uint64_t first_block, uint32_t count,
                      void *destination)
{
    struct trace_device *device = (struct trace_device *)opaque;
    uint8_t *output = (uint8_t *)destination;
    uint32_t index;

    if (device == NULL || destination == NULL || count == 0u ||
        first_block > UINT64_MAX - ((uint64_t)count - 1u) ||
        first_block + count > device->block_count) {
        return -1;
    }
    for (index = 0u; index < count; ++index) {
        uint64_t lba = first_block + index;
        uint64_t byte_offset = lba * AFSPR_MIN_SCRATCH_SIZE;
        uint32_t known;

        if (byte_offset > (uint64_t)LONG_MAX ||
            fseek(device->file, (long)byte_offset, SEEK_SET) != 0 ||
            fread(output + ((size_t)index * AFSPR_MIN_SCRATCH_SIZE), 1,
                  AFSPR_MIN_SCRATCH_SIZE, device->file) !=
                AFSPR_MIN_SCRATCH_SIZE) {
            return -1;
        }
        for (known = 0u; known < device->used; ++known) {
            if (device->lbas[known] == lba) {
                break;
            }
        }
        if (known == device->used) {
            if (device->used == AFSPR_FUZZ_MAX_RECORDED_BLOCKS) {
                return -1;
            }
            device->lbas[device->used] = lba;
            memcpy(device->blocks +
                       ((size_t)device->used * AFSPR_MIN_SCRATCH_SIZE),
                   output + ((size_t)index * AFSPR_MIN_SCRATCH_SIZE),
                   AFSPR_MIN_SCRATCH_SIZE);
            ++device->used;
        }
    }
    return 0;
}

static int write_packet(const char *path, const struct trace_device *device,
                        const struct afspr_fuzz_request *request)
{
    uint8_t header[AFSPR_FUZZ_PACKET_HEADER_SIZE];
    uint8_t lba[8];
    FILE *output;
    uint32_t index;
    int failed = 0;

    memset(header, 0, sizeof(header));
    memcpy(header, "AFZF", 4);
    header[4] = AFSPR_FUZZ_PACKET_VERSION;
    header[5] = request->operation;
    store_u64(header + 8, request->block_count);
    store_u64(header + 16, request->argument);
    store_u64(header + 24, request->offset);
    store_u32(header + 32, request->output_size);
    store_u32(header + 36, device->used);

    output = fopen(path, "wb");
    if (output == NULL) {
        return -1;
    }
    if (fwrite(header, 1, sizeof(header), output) != sizeof(header)) {
        failed = 1;
    }
    for (index = 0u; index < device->used && !failed; ++index) {
        store_u64(lba, device->lbas[index]);
        if (fwrite(lba, 1, sizeof(lba), output) != sizeof(lba) ||
            fwrite(device->blocks +
                       ((size_t)index * AFSPR_MIN_SCRATCH_SIZE),
                   1, AFSPR_MIN_SCRATCH_SIZE, output) !=
                AFSPR_MIN_SCRATCH_SIZE) {
            failed = 1;
        }
    }
    if (fclose(output) != 0) {
        failed = 1;
    }
    return failed ? -1 : 0;
}

static int operation_succeeded(const struct afspr_fuzz_request *request,
                               const struct afspr_fuzz_outcome *outcome)
{
    if (outcome->probe_status != AFSPR_OK) {
        return 0;
    }
    if (request->operation == AFSPR_FUZZ_PROBE) {
        return 1;
    }
    if (request->operation == AFSPR_FUZZ_INTENT_SCAN) {
        return outcome->intent_status == AFSPR_OK;
    }
    if (outcome->object_status != AFSPR_OK) {
        return 0;
    }
    if (request->operation == AFSPR_FUZZ_LOOKUP_OBJECT) {
        return 1;
    }
    if (request->operation == AFSPR_FUZZ_DIRECTORY_ENTRY) {
        return outcome->directory_status == AFSPR_OK;
    }
    return outcome->read_status == AFSPR_OK;
}

int main(int argc, char **argv)
{
    struct trace_device device;
    struct afspr_block_ops ops;
    struct afspr_fuzz_request request;
    struct afspr_fuzz_outcome outcome;
    uint64_t operation;
    uint64_t output_size;
    long image_size;
    int result = EXIT_FAILURE;

    if (argc != 7) {
        fprintf(stderr,
                "usage: %s IMAGE OUTPUT OPERATION ARGUMENT OFFSET OUTPUT_SIZE\n",
                argv[0]);
        return EXIT_FAILURE;
    }
    memset(&device, 0, sizeof(device));
    memset(&request, 0, sizeof(request));
    if (parse_u64(argv[3], &operation) != 0 || operation > UINT8_MAX ||
        parse_u64(argv[4], &request.argument) != 0 ||
        parse_u64(argv[5], &request.offset) != 0 ||
        parse_u64(argv[6], &output_size) != 0 || output_size > UINT32_MAX) {
        fprintf(stderr, "invalid numeric argument\n");
        return EXIT_FAILURE;
    }
    request.operation = (uint8_t)operation;
    request.output_size = (uint32_t)output_size;
    device.file = fopen(argv[1], "rb");
    if (device.file == NULL || fseek(device.file, 0, SEEK_END) != 0) {
        fprintf(stderr, "cannot open image: %s\n", argv[1]);
        goto cleanup;
    }
    image_size = ftell(device.file);
    if (image_size <= 0 ||
        ((unsigned long)image_size % AFSPR_MIN_SCRATCH_SIZE) != 0u) {
        fprintf(stderr, "image is not a non-empty 4 KiB block device\n");
        goto cleanup;
    }
    device.block_count = (uint64_t)(unsigned long)image_size /
                         AFSPR_MIN_SCRATCH_SIZE;
    request.block_count = device.block_count;
    device.blocks = (uint8_t *)malloc((size_t)AFSPR_FUZZ_MAX_RECORDED_BLOCKS *
                                      AFSPR_MIN_SCRATCH_SIZE);
    if (device.blocks == NULL) {
        fprintf(stderr, "cannot allocate trace buffer\n");
        goto cleanup;
    }

    memset(&ops, 0, sizeof(ops));
    ops.abi_version = AFSPR_ABI_VERSION;
    ops.struct_size = (uint32_t)sizeof(ops);
    ops.ctx = &device;
    ops.read_blocks = trace_read;
    ops.block_count = device.block_count;
    ops.block_size = AFSPR_MIN_SCRATCH_SIZE;
    if (afspr_fuzz_exercise(&ops, &request, &outcome) != AFSPR_OK ||
        !operation_succeeded(&request, &outcome)) {
        fprintf(stderr,
                "trace did not complete: probe=%d object=%d directory=%d "
                "read=%d intent=%d stage=%s block=%llu\n",
                outcome.probe_status, outcome.object_status,
                outcome.directory_status, outcome.read_status,
                outcome.intent_status,
                afspr_probe_stage_string(outcome.diagnostic.stage),
                (unsigned long long)outcome.diagnostic.block);
        goto cleanup;
    }
    if (write_packet(argv[2], &device, &request) != 0) {
        fprintf(stderr, "cannot write packet: %s\n", argv[2]);
        goto cleanup;
    }
    printf("seed=%s operation=%u records=%u bytes=%lu\n", argv[2],
           (unsigned int)request.operation, (unsigned int)device.used,
           (unsigned long)(AFSPR_FUZZ_PACKET_HEADER_SIZE +
                           ((size_t)device.used *
                            AFSPR_FUZZ_PACKET_RECORD_SIZE)));
    result = EXIT_SUCCESS;

cleanup:
    free(device.blocks);
    if (device.file != NULL) {
        (void)fclose(device.file);
    }
    return result;
}
