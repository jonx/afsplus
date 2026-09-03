/* SPDX-License-Identifier: BSD-2-Clause */

#include "libafsplus_writer.h"
#include "afsplus_format.h"
#include "reader_internal.h"

#include <limits.h>
#include <string.h>

#define AFSPW_HEADER_SIZE 32u
#define AFSPW_CHECKSUM_OFFSET 28u
#define AFSPW_HEADER_VERSION 1u
#define AFSPW_BLOCK_TYPE_INTENT UINT32_C(0x4a534641)
#define AFSPW_LOG_FIXED_PAYLOAD 32u
#define AFSPW_LOG_OP_FIXED 64u
#define AFSPW_LOG_RECORD_VERSION 2u
#define AFSPW_OP_DELETE 2u
#define AFSPW_OP_RENAME 3u

enum afspw_namespace_kind {
    AFSPW_NAMESPACE_DELETE = 1,
    AFSPW_NAMESPACE_RENAME_NO_REPLACE = 2,
    AFSPW_NAMESPACE_RENAME_REPLACE = 3
};

static void afspw_put_le16(uint8_t *p, uint16_t value)
{
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
}

static void afspw_put_le32(uint8_t *p, uint32_t value)
{
    unsigned index;

    for (index = 0u; index < 4u; ++index) {
        p[index] = (uint8_t)(value >> (index * 8u));
    }
}

static void afspw_put_le64(uint8_t *p, uint64_t value)
{
    unsigned index;

    for (index = 0u; index < 8u; ++index) {
        p[index] = (uint8_t)(value >> (index * 8u));
    }
}

static uint32_t afspw_crc32c_update(uint32_t crc, const uint8_t *data,
                                    size_t size)
{
    size_t index;

    for (index = 0u; index < size; ++index) {
        unsigned bit;

        crc ^= data[index];
        for (bit = 0u; bit < 8u; ++bit) {
            uint32_t mask = (uint32_t)(0u - (crc & 1u));
            crc = (crc >> 1) ^ (UINT32_C(0x82f63b78) & mask);
        }
    }
    return crc;
}

static uint32_t afspw_block_crc32c(const uint8_t *block, size_t block_size)
{
    static const uint8_t zero[4] = {0u, 0u, 0u, 0u};
    uint32_t crc = UINT32_MAX;

    crc = afspw_crc32c_update(crc, block, AFSPW_CHECKSUM_OFFSET);
    crc = afspw_crc32c_update(crc, zero, sizeof(zero));
    crc = afspw_crc32c_update(
        crc, block + AFSPW_CHECKSUM_OFFSET + sizeof(zero),
        block_size - AFSPW_CHECKSUM_OFFSET - sizeof(zero));
    return ~crc;
}

static int afspw_report(struct afspw_diagnostic *diagnostic, int status,
                        uint32_t stage, int reader_status, uint64_t block,
                        uint32_t sequence)
{
    if (diagnostic != NULL) {
        diagnostic->abi_version = AFSPW_ABI_VERSION;
        diagnostic->status = status;
        diagnostic->stage = stage;
        diagnostic->reader_status = reader_status;
        diagnostic->block = block;
        diagnostic->sequence = sequence;
        diagnostic->reserved = 0u;
    }
    return status;
}

static int afspw_reader_failure(struct afspw_diagnostic *diagnostic,
                                int status, uint32_t stage,
                                const struct afspr_diagnostic *reader)
{
    uint64_t block = reader != NULL ? reader->block : AFSPR_NO_BLOCK;

    return afspw_report(diagnostic, status, stage, status, block, 0u);
}

static int afspw_read_adapter(void *context, uint64_t first_block,
                              uint32_t count, void *destination)
{
    const struct afspw_block_ops *ops =
        (const struct afspw_block_ops *)context;

    return ops->read_blocks(ops->ctx, first_block, count, destination);
}

static int afspw_encode_namespace(
    uint8_t *block, size_t block_size,
    const struct afspr_probe_result *volume, uint32_t sequence,
    enum afspw_namespace_kind kind, uint64_t source_parent_id,
    const uint8_t *source_name, size_t source_name_len,
    uint64_t target_parent_id, const uint8_t *target_name,
    size_t target_name_len, const struct afspr_timespec *timestamp)
{
    size_t payload_len = AFSPW_LOG_FIXED_PAYLOAD + AFSPW_LOG_OP_FIXED +
                         source_name_len + target_name_len;
    uint8_t *payload;
    uint8_t *operation;

    if (source_name_len > UINT16_MAX || target_name_len > UINT16_MAX ||
        payload_len > block_size - AFSPW_HEADER_SIZE) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    memset(block, 0, block_size);
    payload = block + AFSPW_HEADER_SIZE;
    memcpy(payload, volume->uuid, sizeof(volume->uuid));
    afspw_put_le64(payload + 16u, volume->generation);
    afspw_put_le32(payload + 24u, sequence);
    afspw_put_le16(payload + 28u, 1u);
    afspw_put_le16(payload + 30u, AFSPW_LOG_RECORD_VERSION);

    operation = payload + AFSPW_LOG_FIXED_PAYLOAD;
    operation[0] = kind == AFSPW_NAMESPACE_DELETE ? AFSPW_OP_DELETE
                                                   : AFSPW_OP_RENAME;
    operation[1] = kind == AFSPW_NAMESPACE_RENAME_REPLACE ? 1u : 0u;
    afspw_put_le16(operation + 2u, (uint16_t)source_name_len);
    afspw_put_le16(operation + 4u, (uint16_t)target_name_len);
    afspw_put_le64(operation + 8u, source_parent_id);
    afspw_put_le64(operation + 16u, target_parent_id);
    afspw_put_le64(operation + 48u, (uint64_t)timestamp->seconds);
    afspw_put_le32(operation + 56u, timestamp->nanoseconds);
    memcpy(operation + AFSPW_LOG_OP_FIXED, source_name, source_name_len);
    if (target_name_len != 0u) {
        memcpy(operation + AFSPW_LOG_OP_FIXED + source_name_len, target_name,
               target_name_len);
    }

    afspw_put_le32(block, AFSPW_BLOCK_TYPE_INTENT);
    afspw_put_le16(block + 4u, AFSPW_HEADER_VERSION);
    afspw_put_le64(block + 16u, volume->generation);
    afspw_put_le32(block + 24u, (uint32_t)payload_len);
    afspw_put_le32(block + AFSPW_CHECKSUM_OFFSET,
                   afspw_block_crc32c(block, block_size));
    return AFSPR_OK;
}

uint64_t afspw_capabilities(void)
{
    return AFSPW_CAP_RENAME_FILE_NO_REPLACE | AFSPW_CAP_DELETE_FILE |
           AFSPW_CAP_RENAME_FILE_REPLACE;
}

static int afspw_append_namespace(
    enum afspw_namespace_kind kind,
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t source_parent_id, const void *source_name,
    size_t source_name_len, uint64_t target_parent_id,
    const void *target_name, size_t target_name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspr_block_ops reader_ops;
    struct afspr_probe_result volume;
    struct afspr_diagnostic reader_diagnostic;
    struct afspr_intent_view view;
    uint32_t preflight_phase;
    uint32_t sequence;
    int destination_conflict;
    int destination_is_file;
    int inspect_target = kind != AFSPW_NAMESPACE_DELETE;
    int needs_orphan = kind != AFSPW_NAMESPACE_RENAME_NO_REPLACE;
    int status;

    if (diagnostic != NULL && diagnostic_size < sizeof(*diagnostic)) {
        return AFSPR_ERR_ABI;
    }
    if (diagnostic != NULL) {
        memset(diagnostic, 0, sizeof(*diagnostic));
        diagnostic->block = AFSPR_NO_BLOCK;
    }
    if (ops == NULL || scratch == NULL || source_name == NULL ||
        timestamp == NULL || result == NULL ||
        (inspect_target != 0 && target_name == NULL) ||
        ops->abi_version != AFSPW_ABI_VERSION ||
        ops->struct_size < sizeof(*ops) || ops->read_blocks == NULL ||
        ops->write_blocks == NULL || ops->flush == NULL ||
        ops->block_size != AFSP_DEFAULT_BLOCK_SIZE ||
        ops->block_count == 0u || scratch->buffer == NULL ||
        scratch->size < AFSPW_SCRATCH_SIZE ||
        result_size < sizeof(*result) || source_parent_id == 0u ||
        (inspect_target != 0 && target_parent_id == 0u) ||
        source_name_len == 0u ||
        source_name_len > AFSP_NAME_MAX_UTF8_BYTES ||
        (inspect_target != 0 &&
         (target_name_len == 0u ||
          target_name_len > AFSP_NAME_MAX_UTF8_BYTES)) ||
        (inspect_target == 0 && target_name_len != 0u) ||
        timestamp->nanoseconds >= UINT32_C(1000000000) ||
        timestamp->reserved != 0u) {
        return afspw_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPW_STAGE_ARGUMENTS, AFSPR_NOT_CHECKED,
                            AFSPR_NO_BLOCK, 0u);
    }
    if (inspect_target != 0 && source_parent_id == target_parent_id &&
        source_name_len == target_name_len &&
        memcmp(source_name, target_name, source_name_len) == 0) {
        return afspw_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPW_STAGE_ARGUMENTS, AFSPR_NOT_CHECKED,
                            AFSPR_NO_BLOCK, 0u);
    }
    memset(result, 0, sizeof(*result));
    result->abi_version = AFSPW_ABI_VERSION;

    memset(&reader_ops, 0, sizeof(reader_ops));
    reader_ops.abi_version = AFSPR_ABI_VERSION;
    reader_ops.struct_size = (uint32_t)sizeof(reader_ops);
    reader_ops.ctx = (void *)ops;
    reader_ops.read_blocks = afspw_read_adapter;
    reader_ops.block_count = ops->block_count;
    reader_ops.block_size = ops->block_size;

    memset(&reader_diagnostic, 0, sizeof(reader_diagnostic));
    status = afspr_probe_detailed(&reader_ops, scratch, &volume,
                                  sizeof(volume), &reader_diagnostic,
                                  sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        return afspw_reader_failure(diagnostic, status, AFSPW_STAGE_PROBE,
                                    &reader_diagnostic);
    }
    if ((volume.incompat_features & AFSP_INCOMPAT_INTENT_LOG) == 0u ||
        volume.log_slots == 0u ||
        (volume.ro_compat_features &
         ~(AFSP_RO_COMPAT_SHARED_EXTENTS |
           AFSP_RO_COMPAT_ORPHAN_DIRECTORY)) != 0u) {
        return afspw_report(diagnostic, AFSPW_ERR_WRITE_FEATURE,
                            AFSPW_STAGE_PROBE, AFSPR_ERR_UNSUPPORTED,
                            AFSPR_NO_BLOCK, 0u);
    }
    if (needs_orphan != 0 &&
        (volume.ro_compat_features & AFSP_RO_COMPAT_ORPHAN_DIRECTORY) == 0u) {
        return afspw_report(diagnostic, AFSPW_ERR_WRITE_FEATURE,
                            AFSPW_STAGE_PROBE, AFSPR_ERR_UNSUPPORTED,
                            AFSPR_NO_BLOCK, 0u);
    }

    status = afspr_internal_preflight_file_namespace(
        &reader_ops, scratch, &volume, source_parent_id, source_name,
        source_name_len, target_parent_id, target_name, target_name_len,
        inspect_target, &view, &destination_conflict, &destination_is_file,
        &preflight_phase, &reader_diagnostic, sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        uint32_t stage = AFSPW_STAGE_INTENT_SCAN;

        if (preflight_phase == AFSPR_INTERNAL_NAMESPACE_SOURCE) {
            stage = AFSPW_STAGE_SOURCE_LOOKUP;
        } else if (preflight_phase == AFSPR_INTERNAL_NAMESPACE_TARGET) {
            stage = AFSPW_STAGE_TARGET_LOOKUP;
        }
        return afspw_reader_failure(diagnostic, status, stage,
                                    &reader_diagnostic);
    }
    if ((view.flags & AFSPR_INTENT_VIEW_NAMESPACE) == 0u) {
        return afspw_report(diagnostic, AFSPW_ERR_WRITE_FEATURE,
                            AFSPW_STAGE_INTENT_SCAN, AFSPR_ERR_UNSUPPORTED,
                            view.tail_block, 0u);
    }
    if (view.tail_state == AFSPR_INTENT_TAIL_FULL ||
        view.tail_slot == AFSPR_NO_LOG_SLOT ||
        view.tail_block == AFSPR_NO_BLOCK) {
        return afspw_report(diagnostic, AFSPW_ERR_LOG_FULL,
                            AFSPW_STAGE_INTENT_SCAN, AFSPR_OK,
                            AFSPR_NO_BLOCK, 0u);
    }
    if (view.tail_slot != view.valid_records ||
        view.last_sequence != view.valid_records) {
        return afspw_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPW_STAGE_INTENT_SCAN, AFSPR_ERR_CORRUPT,
                            view.tail_block, 0u);
    }
    sequence = view.valid_records + 1u;

    if (kind == AFSPW_NAMESPACE_RENAME_NO_REPLACE &&
        destination_conflict != 0) {
        return afspw_report(diagnostic, AFSPW_ERR_DESTINATION_EXISTS,
                            AFSPW_STAGE_TARGET_LOOKUP, AFSPR_OK,
                            AFSPR_NO_BLOCK, sequence);
    }
    if (kind == AFSPW_NAMESPACE_RENAME_REPLACE &&
        destination_conflict != 0 && destination_is_file == 0) {
        return afspw_report(diagnostic, AFSPR_ERR_NOT_FILE,
                            AFSPW_STAGE_TARGET_LOOKUP, AFSPR_ERR_NOT_FILE,
                            AFSPR_NO_BLOCK, sequence);
    }

    status = afspw_encode_namespace(
        (uint8_t *)scratch->buffer, ops->block_size, &volume, sequence,
        kind, source_parent_id, (const uint8_t *)source_name,
        source_name_len, target_parent_id, (const uint8_t *)target_name,
        target_name_len, timestamp);
    if (status != AFSPR_OK) {
        return afspw_report(diagnostic, status, AFSPW_STAGE_ENCODE,
                            AFSPR_NOT_CHECKED, view.tail_block, sequence);
    }
    if (ops->write_blocks(ops->ctx, view.tail_block, 1u,
                          scratch->buffer) != 0) {
        return afspw_report(diagnostic, AFSPW_ERR_WRITE_UNCERTAIN,
                            AFSPW_STAGE_RECORD_WRITE, AFSPR_NOT_CHECKED,
                            view.tail_block, sequence);
    }
    if (ops->flush(ops->ctx) != 0) {
        return afspw_report(diagnostic, AFSPW_ERR_DURABILITY_UNCERTAIN,
                            AFSPW_STAGE_FLUSH, AFSPR_NOT_CHECKED,
                            view.tail_block, sequence);
    }

    result->prior_records = view.valid_records;
    result->sequence = sequence;
    result->log_slot = view.tail_slot;
    result->base_generation = volume.generation;
    result->log_block = view.tail_block;
    return afspw_report(diagnostic, AFSPR_OK, AFSPW_STAGE_COMPLETE,
                        AFSPR_OK, view.tail_block, sequence);
}

int afspw_rename_file_no_replace(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t source_parent_id, const void *source_name,
    size_t source_name_len, uint64_t target_parent_id,
    const void *target_name, size_t target_name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size)
{
    return afspw_append_namespace(
        AFSPW_NAMESPACE_RENAME_NO_REPLACE, ops, scratch, source_parent_id,
        source_name, source_name_len, target_parent_id, target_name,
        target_name_len, timestamp, result, result_size, diagnostic,
        diagnostic_size);
}

int afspw_delete_file(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t parent_id, const void *name, size_t name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size)
{
    return afspw_append_namespace(
        AFSPW_NAMESPACE_DELETE, ops, scratch, parent_id, name, name_len, 0u,
        NULL, 0u, timestamp, result, result_size, diagnostic,
        diagnostic_size);
}

int afspw_rename_file_replace(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t source_parent_id, const void *source_name,
    size_t source_name_len, uint64_t target_parent_id,
    const void *target_name, size_t target_name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size)
{
    return afspw_append_namespace(
        AFSPW_NAMESPACE_RENAME_REPLACE, ops, scratch, source_parent_id,
        source_name, source_name_len, target_parent_id, target_name,
        target_name_len, timestamp, result, result_size, diagnostic,
        diagnostic_size);
}

const char *afspw_status_string(int status)
{
    switch (status) {
    case AFSPW_ERR_LOG_FULL:
        return "intent log full";
    case AFSPW_ERR_DESTINATION_EXISTS:
        return "destination exists";
    case AFSPW_ERR_WRITE_UNCERTAIN:
        return "record write uncertain";
    case AFSPW_ERR_DURABILITY_UNCERTAIN:
        return "durability uncertain";
    case AFSPW_ERR_WRITE_FEATURE:
        return "volume feature unsupported for writing";
    default:
        return afspr_status_string(status);
    }
}

const char *afspw_stage_string(uint32_t stage)
{
    switch (stage) {
    case AFSPW_STAGE_NONE:
        return "none";
    case AFSPW_STAGE_ARGUMENTS:
        return "arguments";
    case AFSPW_STAGE_PROBE:
        return "probe";
    case AFSPW_STAGE_INTENT_SCAN:
        return "intent-scan";
    case AFSPW_STAGE_SOURCE_LOOKUP:
        return "source-lookup";
    case AFSPW_STAGE_TARGET_LOOKUP:
        return "target-lookup";
    case AFSPW_STAGE_ENCODE:
        return "encode";
    case AFSPW_STAGE_RECORD_WRITE:
        return "record-write";
    case AFSPW_STAGE_FLUSH:
        return "flush";
    case AFSPW_STAGE_COMPLETE:
        return "complete";
    default:
        return "unknown";
    }
}
