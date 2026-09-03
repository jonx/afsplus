/* SPDX-License-Identifier: BSD-2-Clause */

#include "harness.h"

#include <string.h>

static const uint8_t afspr_fuzz_magic[4] = {'A', 'F', 'Z', 'F'};

struct afspr_packet_view {
    const uint8_t *records;
    size_t size;
    uint32_t record_count;
};

static uint32_t afspr_fuzz_load_u32(const uint8_t *bytes)
{
    return ((uint32_t)bytes[0]) | ((uint32_t)bytes[1] << 8) |
           ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}

static uint64_t afspr_fuzz_load_u64(const uint8_t *bytes)
{
    return ((uint64_t)bytes[0]) | ((uint64_t)bytes[1] << 8) |
           ((uint64_t)bytes[2] << 16) | ((uint64_t)bytes[3] << 24) |
           ((uint64_t)bytes[4] << 32) | ((uint64_t)bytes[5] << 40) |
           ((uint64_t)bytes[6] << 48) | ((uint64_t)bytes[7] << 56);
}

static void afspr_fuzz_init_outcome(struct afspr_fuzz_outcome *outcome)
{
    memset(outcome, 0, sizeof(*outcome));
    outcome->packet_status = AFSPR_NOT_CHECKED;
    outcome->probe_status = AFSPR_NOT_CHECKED;
    outcome->object_status = AFSPR_NOT_CHECKED;
    outcome->directory_status = AFSPR_NOT_CHECKED;
    outcome->read_status = AFSPR_NOT_CHECKED;
    outcome->diagnostic.abi_version = AFSPR_ABI_VERSION;
    outcome->diagnostic.status = AFSPR_NOT_CHECKED;
    outcome->diagnostic.stage = AFSPR_STAGE_NONE;
    outcome->diagnostic.checkpoint_slot = AFSPR_NO_CHECKPOINT_SLOT;
    outcome->diagnostic.block = AFSPR_NO_BLOCK;
    outcome->diagnostic.checkpoint_status[0] = AFSPR_NOT_CHECKED;
    outcome->diagnostic.checkpoint_status[1] = AFSPR_NOT_CHECKED;
}

int afspr_fuzz_decode_request(const uint8_t *data, size_t size,
                              struct afspr_fuzz_request *request,
                              uint32_t *record_count)
{
    uint32_t count;
    size_t available;

    if (data == NULL || request == NULL || record_count == NULL ||
        size < AFSPR_FUZZ_PACKET_HEADER_SIZE) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    if (memcmp(data, afspr_fuzz_magic, sizeof(afspr_fuzz_magic)) != 0 ||
        data[4] != AFSPR_FUZZ_PACKET_VERSION) {
        return AFSPR_ERR_UNSUPPORTED;
    }

    count = afspr_fuzz_load_u32(data + 36);
    available = (size - AFSPR_FUZZ_PACKET_HEADER_SIZE) /
                AFSPR_FUZZ_PACKET_RECORD_SIZE;
    if ((size_t)count > available) {
        return AFSPR_ERR_CORRUPT;
    }

    request->operation = data[5];
    request->block_count = afspr_fuzz_load_u64(data + 8);
    request->argument = afspr_fuzz_load_u64(data + 16);
    request->offset = afspr_fuzz_load_u64(data + 24);
    request->output_size = afspr_fuzz_load_u32(data + 32);
    *record_count = count;
    return AFSPR_OK;
}

static int afspr_packet_read(void *opaque, uint64_t first_block,
                             uint32_t count, void *destination)
{
    struct afspr_packet_view *view = (struct afspr_packet_view *)opaque;
    uint8_t *output = (uint8_t *)destination;
    uint32_t requested;

    if (view == NULL || destination == NULL || count == 0u || count > 16u) {
        return -1;
    }
    if (first_block > UINT64_MAX - ((uint64_t)count - 1u)) {
        return -1;
    }

    for (requested = 0u; requested < count; ++requested) {
        uint64_t wanted = first_block + requested;
        uint32_t record;
        int found = 0;

        for (record = 0u; record < view->record_count; ++record) {
            size_t position = (size_t)record * AFSPR_FUZZ_PACKET_RECORD_SIZE;
            const uint8_t *entry;

            if (position > view->size ||
                AFSPR_FUZZ_PACKET_RECORD_SIZE > view->size - position) {
                return -1;
            }
            entry = view->records + position;
            if (afspr_fuzz_load_u64(entry) == wanted) {
                memcpy(output + ((size_t)requested * AFSPR_MIN_SCRATCH_SIZE),
                       entry + 8, AFSPR_MIN_SCRATCH_SIZE);
                found = 1;
                break;
            }
        }
        if (!found) {
            return -1;
        }
    }
    return 0;
}

int afspr_fuzz_exercise(const struct afspr_block_ops *ops,
                        const struct afspr_fuzz_request *request,
                        struct afspr_fuzz_outcome *outcome)
{
    uint8_t scratch_bytes[AFSPR_TREE_SCRATCH_SIZE];
    uint8_t name[256];
    uint8_t output[AFSPR_FUZZ_MAX_OUTPUT];
    struct afspr_scratch scratch;
    struct afspr_probe_result volume;
    struct afspr_object object;
    struct afspr_directory_entry entry;
    uint64_t total_entries = 0u;
    size_t output_size;
    size_t bytes_read = 0u;

    if (ops == NULL || request == NULL || outcome == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    afspr_fuzz_init_outcome(outcome);
    outcome->packet_status = AFSPR_OK;
    scratch.buffer = scratch_bytes;
    scratch.size = sizeof(scratch_bytes);
    memset(&volume, 0, sizeof(volume));
    memset(&object, 0, sizeof(object));
    memset(&entry, 0, sizeof(entry));

    outcome->probe_status = afspr_probe_detailed(
        ops, &scratch, &volume, sizeof(volume), &outcome->diagnostic,
        sizeof(outcome->diagnostic));
    if (outcome->probe_status != AFSPR_OK ||
        request->operation == AFSPR_FUZZ_PROBE) {
        return AFSPR_OK;
    }

    if (request->operation == AFSPR_FUZZ_LOOKUP_OBJECT ||
        request->operation == AFSPR_FUZZ_READ_OBJECT) {
        outcome->selected_object_id = request->argument;
        outcome->object_status = afspr_lookup_object(
            ops, &scratch, &volume, request->argument, &object,
            sizeof(object), &outcome->diagnostic, sizeof(outcome->diagnostic));
        if (outcome->object_status != AFSPR_OK ||
            request->operation == AFSPR_FUZZ_LOOKUP_OBJECT) {
            return AFSPR_OK;
        }
    } else if (request->operation == AFSPR_FUZZ_DIRECTORY_ENTRY ||
               request->operation == AFSPR_FUZZ_DIRECTORY_FILE) {
        outcome->object_status = afspr_lookup_object(
            ops, &scratch, &volume, 1u, &object, sizeof(object),
            &outcome->diagnostic, sizeof(outcome->diagnostic));
        if (outcome->object_status != AFSPR_OK) {
            return AFSPR_OK;
        }
        outcome->directory_status = afspr_directory_entry_at(
            ops, &scratch, &volume, &object, request->argument, name,
            sizeof(name), &entry, sizeof(entry), &total_entries,
            &outcome->diagnostic, sizeof(outcome->diagnostic));
        if (outcome->directory_status != AFSPR_OK ||
            request->operation == AFSPR_FUZZ_DIRECTORY_ENTRY) {
            return AFSPR_OK;
        }
        outcome->selected_object_id = entry.object_id;
        outcome->object_status = afspr_lookup_object(
            ops, &scratch, &volume, entry.object_id, &object, sizeof(object),
            &outcome->diagnostic, sizeof(outcome->diagnostic));
        if (outcome->object_status != AFSPR_OK) {
            return AFSPR_OK;
        }
    } else {
        return AFSPR_OK;
    }

    output_size = request->output_size;
    if (output_size > sizeof(output)) {
        output_size = sizeof(output);
    }
    outcome->bytes_requested = output_size;
    outcome->read_status = afspr_read_file(
        ops, &scratch, &volume, &object, request->offset, output, output_size,
        &bytes_read, &outcome->diagnostic, sizeof(outcome->diagnostic));
    return AFSPR_OK;
}

int afspr_fuzz_run_input(const uint8_t *data, size_t size,
                         struct afspr_fuzz_outcome *outcome)
{
    struct afspr_fuzz_request request;
    struct afspr_packet_view view;
    struct afspr_block_ops ops;
    uint32_t record_count = 0u;
    int status;

    if (outcome == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    afspr_fuzz_init_outcome(outcome);
    status = afspr_fuzz_decode_request(data, size, &request, &record_count);
    outcome->packet_status = status;
    if (status != AFSPR_OK) {
        return status;
    }

    view.records = data + AFSPR_FUZZ_PACKET_HEADER_SIZE;
    view.size = size - AFSPR_FUZZ_PACKET_HEADER_SIZE;
    view.record_count = record_count;
    memset(&ops, 0, sizeof(ops));
    ops.abi_version = AFSPR_ABI_VERSION;
    ops.struct_size = (uint32_t)sizeof(ops);
    ops.ctx = &view;
    ops.read_blocks = afspr_packet_read;
    ops.block_count = request.block_count;
    ops.block_size = AFSPR_MIN_SCRATCH_SIZE;
    return afspr_fuzz_exercise(&ops, &request, outcome);
}
