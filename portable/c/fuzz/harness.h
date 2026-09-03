/* SPDX-License-Identifier: BSD-2-Clause */
#ifndef AFSPLUS_PORTABLE_C_FUZZ_HARNESS_H
#define AFSPLUS_PORTABLE_C_FUZZ_HARNESS_H

#include "libafsplus_reader.h"

#include <stddef.h>
#include <stdint.h>

#define AFSPR_FUZZ_PACKET_HEADER_SIZE 40u
#define AFSPR_FUZZ_PACKET_RECORD_SIZE (8u + AFSPR_MIN_SCRATCH_SIZE)
#define AFSPR_FUZZ_PACKET_VERSION 1u
#define AFSPR_FUZZ_MAX_OUTPUT 8192u

enum afspr_fuzz_operation {
    AFSPR_FUZZ_PROBE = 0,
    AFSPR_FUZZ_LOOKUP_OBJECT = 1,
    AFSPR_FUZZ_DIRECTORY_ENTRY = 2,
    AFSPR_FUZZ_READ_OBJECT = 3,
    AFSPR_FUZZ_DIRECTORY_FILE = 4,
    AFSPR_FUZZ_INTENT_SCAN = 5,
    AFSPR_FUZZ_INTENT_NAMESPACE = 6
};

struct afspr_fuzz_request {
    uint8_t operation;
    uint64_t block_count;
    uint64_t argument;
    uint64_t offset;
    uint32_t output_size;
};

struct afspr_fuzz_outcome {
    int packet_status;
    int probe_status;
    int object_status;
    int directory_status;
    int read_status;
    int intent_status;
    uint64_t selected_object_id;
    size_t bytes_requested;
    struct afspr_diagnostic diagnostic;
};

int afspr_fuzz_decode_request(const uint8_t *data, size_t size,
                              struct afspr_fuzz_request *request,
                              uint32_t *record_count);

int afspr_fuzz_exercise(const struct afspr_block_ops *ops,
                        const struct afspr_fuzz_request *request,
                        struct afspr_fuzz_outcome *outcome);

int afspr_fuzz_run_input(const uint8_t *data, size_t size,
                         struct afspr_fuzz_outcome *outcome);

/* Standard entry point recognized by libFuzzer and compatible engines. */
int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size);

#endif
