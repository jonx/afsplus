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
#define AFSPW_LOG_DATA_RECORD_VERSION 3u
#define AFSPW_OP_CREATE 1u
#define AFSPW_OP_DELETE 2u
#define AFSPW_OP_RENAME 3u
#define AFSPW_OP_WRITE 4u
#define AFSPW_OP_TRUNCATE 5u
#define AFSPW_LOG_EXTENT_WIRE 12u
#define AFSPW_MIN_HEADROOM_VOLUME_BLOCKS UINT64_C(64)
#define AFSPW_HEADROOM_SCALE_BLOCKS UINT64_C(32)
#define AFSPW_MIN_HEADROOM_BLOCKS UINT64_C(8)
#define AFSPW_MAX_HEADROOM_BLOCKS UINT64_C(64)

enum afspw_namespace_kind {
    AFSPW_NAMESPACE_CREATE = 1,
    AFSPW_NAMESPACE_DELETE = 2,
    AFSPW_NAMESPACE_RENAME_NO_REPLACE = 3,
    AFSPW_NAMESPACE_RENAME_REPLACE = 4
};

struct afspw_reader_context {
    const struct afspw_block_ops *ops;
    uint8_t *cache;
    uint64_t cache_lbas[AFSPW_MAX_CACHED_BLOCKS];
    uint64_t cache_ages[AFSPW_MAX_CACHED_BLOCKS];
    uint64_t cache_clock;
    uint32_t cache_blocks;
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
    struct afspw_reader_context *reader =
        (struct afspw_reader_context *)context;
    uint32_t entry;

    if (count == 1u) {
        for (entry = 0u; entry < reader->cache_blocks; ++entry) {
            if (reader->cache_lbas[entry] == first_block) {
                memcpy(destination,
                       reader->cache +
                           (size_t)entry * reader->ops->block_size,
                       reader->ops->block_size);
                reader->cache_ages[entry] = ++reader->cache_clock;
                return 0;
            }
        }
    }
    if (reader->ops->read_blocks(reader->ops->ctx, first_block, count,
                                 destination) != 0) {
        return -1;
    }
    if (count == 1u && reader->cache_blocks != 0u) {
        uint32_t victim = 0u;

        for (entry = 0u; entry < reader->cache_blocks; ++entry) {
            if (reader->cache_lbas[entry] == UINT64_MAX) {
                victim = entry;
                break;
            }
            if (reader->cache_ages[entry] < reader->cache_ages[victim]) {
                victim = entry;
            }
        }
        memcpy(reader->cache +
                   (size_t)victim * reader->ops->block_size,
               destination, reader->ops->block_size);
        reader->cache_lbas[victim] = first_block;
        reader->cache_ages[victim] = ++reader->cache_clock;
    }
    return 0;
}

static void afspw_init_reader(const struct afspw_block_ops *ops,
                              const struct afspr_scratch *scratch,
                              struct afspr_block_ops *reader_ops,
                              struct afspw_reader_context *reader_context)
{
    size_t available_blocks =
        (scratch->size - AFSPW_SCRATCH_SIZE) / ops->block_size;
    uint32_t entry;

    memset(reader_ops, 0, sizeof(*reader_ops));
    memset(reader_context, 0, sizeof(*reader_context));
    reader_context->ops = ops;
    reader_context->cache =
        (uint8_t *)scratch->buffer + AFSPW_SCRATCH_SIZE;
    if (available_blocks > AFSPW_MAX_CACHED_BLOCKS) {
        available_blocks = AFSPW_MAX_CACHED_BLOCKS;
    }
    reader_context->cache_blocks = (uint32_t)available_blocks;
    for (entry = 0u; entry < reader_context->cache_blocks; ++entry) {
        reader_context->cache_lbas[entry] = UINT64_MAX;
    }
    reader_ops->abi_version = AFSPR_ABI_VERSION;
    reader_ops->struct_size = (uint32_t)sizeof(*reader_ops);
    reader_ops->ctx = reader_context;
    reader_ops->read_blocks = afspw_read_adapter;
    reader_ops->block_count = ops->block_count;
    reader_ops->block_size = ops->block_size;
}

static int afspw_prepare_volume(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    int needs_orphan, int needs_data_updates,
    struct afspr_block_ops *reader_ops,
    struct afspw_reader_context *reader_context,
    struct afspr_probe_result *volume,
    struct afspw_diagnostic *diagnostic)
{
    struct afspr_diagnostic reader_diagnostic;
    int status;

    afspw_init_reader(ops, scratch, reader_ops, reader_context);
    memset(&reader_diagnostic, 0, sizeof(reader_diagnostic));
    status = afspr_probe_detailed(reader_ops, scratch, volume,
                                  sizeof(*volume), &reader_diagnostic,
                                  sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        return afspw_reader_failure(diagnostic, status, AFSPW_STAGE_PROBE,
                                    &reader_diagnostic);
    }
    if ((volume->incompat_features & AFSP_INCOMPAT_INTENT_LOG) == 0u ||
        volume->log_slots == 0u ||
        (volume->ro_compat_features &
         ~(AFSP_RO_COMPAT_SHARED_EXTENTS |
           AFSP_RO_COMPAT_ORPHAN_DIRECTORY)) != 0u ||
        (needs_orphan != 0 &&
         (volume->ro_compat_features &
          AFSP_RO_COMPAT_ORPHAN_DIRECTORY) == 0u) ||
        (needs_data_updates != 0 &&
         (volume->incompat_features &
          AFSP_INCOMPAT_INTENT_LOG_DATA_UPDATES) == 0u)) {
        return afspw_report(diagnostic, AFSPW_ERR_WRITE_FEATURE,
                            AFSPW_STAGE_PROBE, AFSPR_ERR_UNSUPPORTED,
                            AFSPR_NO_BLOCK, 0u);
    }
    return AFSPR_OK;
}

static int afspw_encode_namespace(
    uint8_t *block, size_t block_size,
    const struct afspr_probe_result *volume, uint32_t sequence,
    enum afspw_namespace_kind kind, uint64_t source_parent_id,
    const uint8_t *source_name, size_t source_name_len,
    uint64_t target_parent_id, const uint8_t *target_name,
    size_t target_name_len, uint64_t created_object_id,
    const struct afspr_timespec *timestamp)
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
    operation[0] = kind == AFSPW_NAMESPACE_CREATE
                       ? AFSPW_OP_CREATE
                       : (kind == AFSPW_NAMESPACE_DELETE ? AFSPW_OP_DELETE
                                                         : AFSPW_OP_RENAME);
    operation[1] = kind == AFSPW_NAMESPACE_RENAME_REPLACE ? 1u : 0u;
    afspw_put_le16(operation + 2u, (uint16_t)source_name_len);
    afspw_put_le16(operation + 4u, (uint16_t)target_name_len);
    afspw_put_le64(operation + 8u, source_parent_id);
    afspw_put_le64(operation + 16u, target_parent_id);
    if (kind == AFSPW_NAMESPACE_CREATE) {
        afspw_put_le64(operation + 24u, created_object_id);
    }
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

static int afspw_encode_truncate(
    uint8_t *block, size_t block_size,
    const struct afspr_probe_result *volume, uint32_t sequence,
    uint64_t object_id, uint64_t expected_size, uint64_t new_size,
    const struct afspr_timespec *timestamp)
{
    const size_t payload_len = AFSPW_LOG_FIXED_PAYLOAD + AFSPW_LOG_OP_FIXED;
    uint8_t *payload;
    uint8_t *operation;

    if (payload_len > block_size - AFSPW_HEADER_SIZE) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    memset(block, 0, block_size);
    payload = block + AFSPW_HEADER_SIZE;
    memcpy(payload, volume->uuid, sizeof(volume->uuid));
    afspw_put_le64(payload + 16u, volume->generation);
    afspw_put_le32(payload + 24u, sequence);
    afspw_put_le16(payload + 28u, 1u);
    afspw_put_le16(payload + 30u, AFSPW_LOG_DATA_RECORD_VERSION);

    operation = payload + AFSPW_LOG_FIXED_PAYLOAD;
    operation[0] = AFSPW_OP_TRUNCATE;
    afspw_put_le64(operation + 8u, object_id);
    afspw_put_le64(operation + 24u, expected_size);
    afspw_put_le64(operation + 32u, new_size);
    afspw_put_le64(operation + 48u, (uint64_t)timestamp->seconds);
    afspw_put_le32(operation + 56u, timestamp->nanoseconds);

    afspw_put_le32(block, AFSPW_BLOCK_TYPE_INTENT);
    afspw_put_le16(block + 4u, AFSPW_HEADER_VERSION);
    afspw_put_le64(block + 16u, volume->generation);
    afspw_put_le32(block + 24u, (uint32_t)payload_len);
    afspw_put_le32(block + AFSPW_CHECKSUM_OFFSET,
                   afspw_block_crc32c(block, block_size));
    return AFSPR_OK;
}

static int afspw_encode_write(
    uint8_t *block, size_t block_size,
    const struct afspr_probe_result *volume, uint32_t sequence,
    uint64_t object_id, uint64_t logical_block, uint64_t expected_size,
    uint64_t new_size, uint32_t content_crc, uint64_t data_block,
    const struct afspr_timespec *timestamp)
{
    const size_t payload_len = AFSPW_LOG_FIXED_PAYLOAD +
                               AFSPW_LOG_OP_FIXED +
                               AFSPW_LOG_EXTENT_WIRE;
    uint8_t *payload;
    uint8_t *operation;
    uint8_t *extent;

    if (payload_len > block_size - AFSPW_HEADER_SIZE) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    memset(block, 0, block_size);
    payload = block + AFSPW_HEADER_SIZE;
    memcpy(payload, volume->uuid, sizeof(volume->uuid));
    afspw_put_le64(payload + 16u, volume->generation);
    afspw_put_le32(payload + 24u, sequence);
    afspw_put_le16(payload + 28u, 1u);
    afspw_put_le16(payload + 30u, AFSPW_LOG_DATA_RECORD_VERSION);

    operation = payload + AFSPW_LOG_FIXED_PAYLOAD;
    operation[0] = AFSPW_OP_WRITE;
    afspw_put_le16(operation + 6u, 1u);
    afspw_put_le64(operation + 8u, object_id);
    afspw_put_le64(operation + 16u, logical_block);
    afspw_put_le64(operation + 24u, expected_size);
    afspw_put_le64(operation + 32u, new_size);
    afspw_put_le32(operation + 40u, content_crc);
    afspw_put_le64(operation + 48u, (uint64_t)timestamp->seconds);
    afspw_put_le32(operation + 56u, timestamp->nanoseconds);
    extent = operation + AFSPW_LOG_OP_FIXED;
    afspw_put_le64(extent, data_block);
    afspw_put_le32(extent + 8u, 1u);

    afspw_put_le32(block, AFSPW_BLOCK_TYPE_INTENT);
    afspw_put_le16(block + 4u, AFSPW_HEADER_VERSION);
    afspw_put_le64(block + 16u, volume->generation);
    afspw_put_le32(block + 24u, (uint32_t)payload_len);
    afspw_put_le32(block + AFSPW_CHECKSUM_OFFSET,
                   afspw_block_crc32c(block, block_size));
    return AFSPR_OK;
}

static uint64_t afspw_emergency_headroom(uint64_t total_blocks)
{
    uint64_t headroom;

    if (total_blocks < AFSPW_MIN_HEADROOM_VOLUME_BLOCKS) {
        return 0u;
    }
    headroom = total_blocks / AFSPW_HEADROOM_SCALE_BLOCKS;
    if (total_blocks % AFSPW_HEADROOM_SCALE_BLOCKS != 0u) {
        ++headroom;
    }
    if (headroom < AFSPW_MIN_HEADROOM_BLOCKS) {
        return AFSPW_MIN_HEADROOM_BLOCKS;
    }
    if (headroom > AFSPW_MAX_HEADROOM_BLOCKS) {
        return AFSPW_MAX_HEADROOM_BLOCKS;
    }
    return headroom;
}

static int afspw_buffers_overlap(const void *left, size_t left_size,
                                 const void *right, size_t right_size)
{
    uintptr_t left_start = (uintptr_t)left;
    uintptr_t right_start = (uintptr_t)right;
    uintptr_t left_end = left_start + left_size;
    uintptr_t right_end = right_start + right_size;

    if (left_end < left_start || right_end < right_start) {
        return 1;
    }
    return left_start < right_end && right_start < left_end;
}

uint64_t afspw_capabilities(void)
{
    return AFSPW_CAP_RENAME_FILE_NO_REPLACE | AFSPW_CAP_DELETE_FILE |
           AFSPW_CAP_RENAME_FILE_REPLACE | AFSPW_CAP_CREATE_EMPTY_FILE |
           AFSPW_CAP_TRUNCATE_FILE_DATA_FREE |
           AFSPW_CAP_WRITE_FILE_BLOCK_COW;
}

static int afspw_append_namespace(
    enum afspw_namespace_kind kind,
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t source_parent_id, const void *source_name,
    size_t source_name_len, uint64_t target_parent_id,
    const void *target_name, size_t target_name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_rename_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size,
    uint64_t *created_object_id)
{
    struct afspr_block_ops reader_ops;
    struct afspw_reader_context reader_context;
    struct afspr_probe_result volume;
    struct afspr_diagnostic reader_diagnostic;
    struct afspr_intent_view view;
    uint32_t preflight_phase;
    uint32_t sequence;
    uint64_t next_object_id;
    int destination_conflict;
    int destination_is_file;
    int source_must_exist = kind != AFSPW_NAMESPACE_CREATE;
    int inspect_target = kind == AFSPW_NAMESPACE_RENAME_NO_REPLACE ||
                         kind == AFSPW_NAMESPACE_RENAME_REPLACE;
    int needs_orphan = kind == AFSPW_NAMESPACE_DELETE ||
                       kind == AFSPW_NAMESPACE_RENAME_REPLACE;
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

    memset(&reader_diagnostic, 0, sizeof(reader_diagnostic));
    status = afspw_prepare_volume(ops, scratch, needs_orphan, 0,
                                  &reader_ops, &reader_context, &volume,
                                  diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }

    status = afspr_internal_preflight_file_namespace(
        &reader_ops, scratch, &volume, source_parent_id, source_name,
        source_name_len, target_parent_id, target_name, target_name_len,
        source_must_exist, inspect_target, &view, &destination_conflict,
        &destination_is_file, &next_object_id, &preflight_phase,
        &reader_diagnostic, sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        uint32_t stage = AFSPW_STAGE_INTENT_SCAN;

        if (preflight_phase == AFSPR_INTERNAL_NAMESPACE_SOURCE) {
            stage = kind == AFSPW_NAMESPACE_CREATE
                        ? AFSPW_STAGE_CREATE_LOOKUP
                        : AFSPW_STAGE_SOURCE_LOOKUP;
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

    if (kind == AFSPW_NAMESPACE_CREATE && destination_conflict != 0) {
        return afspw_report(diagnostic, AFSPW_ERR_DESTINATION_EXISTS,
                            AFSPW_STAGE_CREATE_LOOKUP, AFSPR_OK,
                            AFSPR_NO_BLOCK, sequence);
    }
    if (kind == AFSPW_NAMESPACE_CREATE && next_object_id == UINT64_MAX) {
        return afspw_report(diagnostic, AFSPW_ERR_OBJECT_ID_EXHAUSTED,
                            AFSPW_STAGE_CREATE_LOOKUP, AFSPR_OK,
                            AFSPR_NO_BLOCK, sequence);
    }
    if (kind == AFSPW_NAMESPACE_CREATE) {
        status = afspr_internal_validate_object_watermark(
            &reader_ops, scratch, &volume, next_object_id,
            &reader_diagnostic, sizeof(reader_diagnostic));
        if (status != AFSPR_OK) {
            return afspw_reader_failure(diagnostic, status,
                                        AFSPW_STAGE_CREATE_LOOKUP,
                                        &reader_diagnostic);
        }
    }
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
        target_name_len, next_object_id, timestamp);
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
    if (created_object_id != NULL) {
        *created_object_id = next_object_id;
    }
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
        diagnostic_size, NULL);
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
        diagnostic_size, NULL);
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
        diagnostic_size, NULL);
}

int afspw_create_empty_file(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t parent_id, const void *name, size_t name_len,
    const struct afspr_timespec *timestamp,
    struct afspw_create_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspw_rename_result common;
    uint64_t object_id = 0u;
    int status;

    if (diagnostic != NULL && diagnostic_size < sizeof(*diagnostic)) {
        return AFSPR_ERR_ABI;
    }
    if (result == NULL || result_size < sizeof(*result)) {
        return afspw_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPW_STAGE_ARGUMENTS, AFSPR_NOT_CHECKED,
                            AFSPR_NO_BLOCK, 0u);
    }
    memset(result, 0, sizeof(*result));
    result->abi_version = AFSPW_ABI_VERSION;
    status = afspw_append_namespace(
        AFSPW_NAMESPACE_CREATE, ops, scratch, parent_id, name, name_len, 0u,
        NULL, 0u, timestamp, &common, sizeof(common), diagnostic,
        diagnostic_size, &object_id);
    if (status != AFSPR_OK) {
        return status;
    }
    result->prior_records = common.prior_records;
    result->sequence = common.sequence;
    result->log_slot = common.log_slot;
    result->base_generation = common.base_generation;
    result->log_block = common.log_block;
    result->object_id = object_id;
    return AFSPR_OK;
}

int afspw_truncate_file(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t object_id, uint64_t new_size,
    const struct afspr_timespec *timestamp,
    struct afspw_truncate_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspr_block_ops reader_ops;
    struct afspw_reader_context reader_context;
    struct afspr_probe_result volume;
    struct afspr_diagnostic reader_diagnostic;
    struct afspr_intent_view view;
    uint64_t current_size;
    uint32_t sequence;
    int status;

    if (diagnostic != NULL && diagnostic_size < sizeof(*diagnostic)) {
        return AFSPR_ERR_ABI;
    }
    if (diagnostic != NULL) {
        memset(diagnostic, 0, sizeof(*diagnostic));
        diagnostic->block = AFSPR_NO_BLOCK;
    }
    if (ops == NULL || scratch == NULL || timestamp == NULL ||
        result == NULL || ops->abi_version != AFSPW_ABI_VERSION ||
        ops->struct_size < sizeof(*ops) || ops->read_blocks == NULL ||
        ops->write_blocks == NULL || ops->flush == NULL ||
        ops->block_size != AFSP_DEFAULT_BLOCK_SIZE ||
        ops->block_count == 0u || scratch->buffer == NULL ||
        scratch->size < AFSPW_SCRATCH_SIZE ||
        result_size < sizeof(*result) || object_id == 0u ||
        timestamp->nanoseconds >= UINT32_C(1000000000) ||
        timestamp->reserved != 0u) {
        return afspw_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPW_STAGE_ARGUMENTS, AFSPR_NOT_CHECKED,
                            AFSPR_NO_BLOCK, 0u);
    }
    memset(result, 0, sizeof(*result));
    result->abi_version = AFSPW_ABI_VERSION;
    result->log_slot = AFSPR_NO_LOG_SLOT;
    result->log_block = AFSPR_NO_BLOCK;
    result->new_size = new_size;

    status = afspw_prepare_volume(ops, scratch, 0, 1, &reader_ops,
                                  &reader_context, &volume, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    memset(&reader_diagnostic, 0, sizeof(reader_diagnostic));
    status = afspr_scan_intent_log(&reader_ops, scratch, &volume, &view,
                                   sizeof(view), &reader_diagnostic,
                                   sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        return afspw_reader_failure(diagnostic, status,
                                    AFSPW_STAGE_INTENT_SCAN,
                                    &reader_diagnostic);
    }
    status = afspr_internal_intent_file_size(
        &reader_ops, scratch, &volume, &view, object_id, &current_size,
        &reader_diagnostic, sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        return afspw_reader_failure(diagnostic, status,
                                    AFSPW_STAGE_FILE_STATE,
                                    &reader_diagnostic);
    }
    result->prior_records = view.valid_records;
    result->base_generation = volume.generation;
    result->previous_size = current_size;
    if (new_size == current_size) {
        return afspw_report(diagnostic, AFSPR_OK, AFSPW_STAGE_COMPLETE,
                            AFSPR_OK, AFSPR_NO_BLOCK, 0u);
    }
    sequence = view.valid_records + 1u;
    if (new_size < current_size &&
        new_size % (uint64_t)ops->block_size != 0u) {
        return afspw_report(diagnostic, AFSPW_ERR_TAIL_REWRITE_REQUIRED,
                            AFSPW_STAGE_FILE_STATE, AFSPR_OK,
                            AFSPR_NO_BLOCK, sequence);
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
    status = afspw_encode_truncate(
        (uint8_t *)scratch->buffer, ops->block_size, &volume, sequence,
        object_id, current_size, new_size, timestamp);
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
    result->sequence = sequence;
    result->log_slot = view.tail_slot;
    result->log_block = view.tail_block;
    result->record_written = 1u;
    return afspw_report(diagnostic, AFSPR_OK, AFSPW_STAGE_COMPLETE,
                        AFSPR_OK, view.tail_block, sequence);
}

int afspw_write_file_block_cow(
    const struct afspw_block_ops *ops, const struct afspr_scratch *scratch,
    uint64_t object_id, uint64_t logical_block, const void *block_data,
    size_t block_data_size, uint64_t new_size,
    const struct afspr_timespec *timestamp,
    struct afspw_write_result *result, size_t result_size,
    struct afspw_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspr_block_ops reader_ops;
    struct afspw_reader_context reader_context;
    struct afspr_probe_result volume;
    struct afspr_diagnostic reader_diagnostic;
    struct afspr_intent_view view;
    const uint8_t *data = (const uint8_t *)block_data;
    uint64_t current_size;
    uint64_t file_blocks;
    uint64_t data_block;
    uint32_t content_crc;
    uint32_t sequence;
    size_t tail_offset;
    size_t index;
    int status;

    if (diagnostic != NULL && diagnostic_size < sizeof(*diagnostic)) {
        return AFSPR_ERR_ABI;
    }
    if (diagnostic != NULL) {
        memset(diagnostic, 0, sizeof(*diagnostic));
        diagnostic->block = AFSPR_NO_BLOCK;
    }
    if (ops == NULL || scratch == NULL || block_data == NULL ||
        timestamp == NULL || result == NULL ||
        ops->abi_version != AFSPW_ABI_VERSION ||
        ops->struct_size < sizeof(*ops) || ops->read_blocks == NULL ||
        ops->write_blocks == NULL || ops->flush == NULL ||
        ops->block_size != AFSP_DEFAULT_BLOCK_SIZE ||
        ops->block_count == 0u || scratch->buffer == NULL ||
        scratch->size < AFSPW_SCRATCH_SIZE ||
        result_size < sizeof(*result) || object_id == 0u ||
        block_data_size != ops->block_size || new_size == 0u ||
        logical_block == UINT64_MAX ||
        logical_block > UINT64_MAX / ops->block_size ||
        timestamp->nanoseconds >= UINT32_C(1000000000) ||
        timestamp->reserved != 0u ||
        afspw_buffers_overlap(block_data, block_data_size, scratch->buffer,
                              scratch->size)) {
        return afspw_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPW_STAGE_ARGUMENTS, AFSPR_NOT_CHECKED,
                            AFSPR_NO_BLOCK, 0u);
    }
    memset(result, 0, sizeof(*result));
    result->abi_version = AFSPW_ABI_VERSION;
    result->log_slot = AFSPR_NO_LOG_SLOT;
    result->log_block = AFSPR_NO_BLOCK;
    result->data_block = AFSPR_NO_BLOCK;
    result->logical_block = logical_block;
    result->new_size = new_size;

    status = afspw_prepare_volume(ops, scratch, 0, 1, &reader_ops,
                                  &reader_context, &volume, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    memset(&reader_diagnostic, 0, sizeof(reader_diagnostic));
    status = afspr_scan_intent_log(&reader_ops, scratch, &volume, &view,
                                   sizeof(view), &reader_diagnostic,
                                   sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        return afspw_reader_failure(diagnostic, status,
                                    AFSPW_STAGE_INTENT_SCAN,
                                    &reader_diagnostic);
    }
    status = afspr_internal_intent_file_size(
        &reader_ops, scratch, &volume, &view, object_id, &current_size,
        &reader_diagnostic, sizeof(reader_diagnostic));
    if (status != AFSPR_OK) {
        return afspw_reader_failure(diagnostic, status,
                                    AFSPW_STAGE_FILE_STATE,
                                    &reader_diagnostic);
    }
    result->prior_records = view.valid_records;
    result->base_generation = volume.generation;
    result->previous_size = current_size;
    sequence = view.valid_records + 1u;
    if (new_size < current_size) {
        return afspw_report(diagnostic, AFSPW_ERR_INVALID_WRITE_RANGE,
                            AFSPW_STAGE_FILE_STATE, AFSPR_OK,
                            AFSPR_NO_BLOCK, sequence);
    }
    file_blocks = new_size / ops->block_size;
    if (new_size % ops->block_size != 0u) {
        ++file_blocks;
    }
    if (logical_block >= file_blocks ||
        (new_size > current_size && logical_block + 1u != file_blocks)) {
        return afspw_report(diagnostic, AFSPW_ERR_INVALID_WRITE_RANGE,
                            AFSPW_STAGE_FILE_STATE, AFSPR_OK,
                            AFSPR_NO_BLOCK, sequence);
    }
    tail_offset = (size_t)(new_size % ops->block_size);
    if (tail_offset != 0u && logical_block + 1u == file_blocks) {
        for (index = tail_offset; index < block_data_size; ++index) {
            if (data[index] != 0u) {
                return afspw_report(diagnostic, AFSPW_ERR_NONZERO_TAIL,
                                    AFSPW_STAGE_FILE_STATE, AFSPR_OK,
                                    AFSPR_NO_BLOCK, sequence);
            }
        }
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
    status = afspr_internal_find_log_data_block(
        &reader_ops, scratch, &volume, &view,
        afspw_emergency_headroom(volume.total_blocks), &data_block,
        &reader_diagnostic, sizeof(reader_diagnostic));
    if (status == AFSPR_ERR_NOT_FOUND) {
        return afspw_report(diagnostic, AFSPW_ERR_NO_SPACE,
                            AFSPW_STAGE_ALLOCATION, AFSPR_OK,
                            AFSPR_NO_BLOCK, sequence);
    }
    if (status != AFSPR_OK) {
        return afspw_reader_failure(diagnostic, status,
                                    AFSPW_STAGE_ALLOCATION,
                                    &reader_diagnostic);
    }

    content_crc = ~afspw_crc32c_update(UINT32_MAX, data, block_data_size);
    status = afspw_encode_write(
        (uint8_t *)scratch->buffer, ops->block_size, &volume, sequence,
        object_id, logical_block, current_size, new_size, content_crc,
        data_block, timestamp);
    if (status != AFSPR_OK) {
        return afspw_report(diagnostic, status, AFSPW_STAGE_ENCODE,
                            AFSPR_NOT_CHECKED, view.tail_block, sequence);
    }
    if (ops->write_blocks(ops->ctx, data_block, 1u, block_data) != 0) {
        return afspw_report(diagnostic, AFSPW_ERR_DATA_WRITE_UNCERTAIN,
                            AFSPW_STAGE_DATA_WRITE, AFSPR_NOT_CHECKED,
                            data_block, sequence);
    }
    if (ops->flush(ops->ctx) != 0) {
        return afspw_report(
            diagnostic, AFSPW_ERR_DATA_DURABILITY_UNCERTAIN,
            AFSPW_STAGE_DATA_FLUSH, AFSPR_NOT_CHECKED, data_block,
            sequence);
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
    result->sequence = sequence;
    result->log_slot = view.tail_slot;
    result->log_block = view.tail_block;
    result->data_block = data_block;
    result->data_blocks = 1u;
    return afspw_report(diagnostic, AFSPR_OK, AFSPW_STAGE_COMPLETE,
                        AFSPR_OK, view.tail_block, sequence);
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
    case AFSPW_ERR_OBJECT_ID_EXHAUSTED:
        return "object ID space exhausted";
    case AFSPW_ERR_TAIL_REWRITE_REQUIRED:
        return "truncate tail rewrite required";
    case AFSPW_ERR_NO_SPACE:
        return "no space for ordinary growth";
    case AFSPW_ERR_INVALID_WRITE_RANGE:
        return "invalid complete-block write range";
    case AFSPW_ERR_NONZERO_TAIL:
        return "nonzero data beyond file end";
    case AFSPW_ERR_DATA_WRITE_UNCERTAIN:
        return "data write uncertain";
    case AFSPW_ERR_DATA_DURABILITY_UNCERTAIN:
        return "data durability uncertain";
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
    case AFSPW_STAGE_CREATE_LOOKUP:
        return "create-lookup";
    case AFSPW_STAGE_FILE_STATE:
        return "file-state";
    case AFSPW_STAGE_ALLOCATION:
        return "allocation";
    case AFSPW_STAGE_DATA_WRITE:
        return "data-write";
    case AFSPW_STAGE_DATA_FLUSH:
        return "data-flush";
    default:
        return "unknown";
    }
}
