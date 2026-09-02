/* SPDX-License-Identifier: BSD-2-Clause */

#include "libafsplus_reader.h"
#include "afsplus_format.h"

#include <limits.h>
#include <string.h>

#define AFSPR_HEADER_SIZE 32u
#define AFSPR_CHECKSUM_OFFSET 28u
#define AFSPR_HEADER_VERSION 1u
#define AFSPR_BLOCK_TYPE_IDENT UINT32_C(0x49534641)
#define AFSPR_BLOCK_TYPE_CHECKPOINT UINT32_C(0x43534641)
#define AFSPR_CHECKSUM_CRC32C 1u
#define AFSPR_IDENT_LEGACY_PAYLOAD 137u
#define AFSPR_IDENT_FEATURE_PAYLOAD 161u
#define AFSPR_IDENT_CURRENT_PAYLOAD 165u
#define AFSPR_CHECKPOINT_PAYLOAD 96u
#define AFSPR_OBJECT_FIRST_DYNAMIC UINT64_C(16)
#define AFSPR_MIN_REGION_BLOCKS UINT32_C(16)
#define AFSPR_MAX_REGION_BLOCKS UINT32_C(262144)
#define AFSPR_BITMAP_PAGE_BLOCKS UINT32_C(32384)
#define AFSPR_BOOTSTRAP_BLOCKS UINT64_C(3)
#define AFSPR_DESCRIPTOR_SLOTS UINT64_C(3)
#define AFSPR_BITMAP_SLOTS UINT64_C(3)
#define AFSPR_SUPPORTED_INCOMPAT                                             \
    (AFSP_INCOMPAT_INTENT_LOG | AFSP_INCOMPAT_INTENT_LOG_DATA_UPDATES)

struct afspr_header {
    uint16_t flags;
    uint64_t owner;
    uint64_t generation;
    uint32_t payload_len;
};

struct afspr_ident {
    uint32_t version;
    uint8_t uuid[16];
    uint8_t block_shift;
    uint8_t checksum_algorithm;
    uint16_t log_slots;
    uint32_t region_size;
    uint64_t total_blocks;
    uint64_t checkpoint_slots[2];
    uint64_t metadata_start;
    uint64_t compat_features;
    uint64_t ro_compat_features;
    uint64_t incompat_features;
    uint8_t name_key_algorithm;
    uint8_t unicode_version[3];
    uint8_t label_len;
    char label[AFSPR_LABEL_CAPACITY];
};

struct afspr_checkpoint {
    uint64_t generation;
    uint64_t object_map_block;
    uint64_t allocation_root_block;
    uint64_t reclaim_root_block;
    uint64_t next_object_id;
    uint64_t committed_tx_id;
    uint64_t free_blocks_total;
    uint64_t shared_extent_root_block;
};

static uint16_t afspr_get_le16(const uint8_t *p)
{
    return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}

static uint32_t afspr_get_le32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static uint64_t afspr_get_le64(const uint8_t *p)
{
    return (uint64_t)afspr_get_le32(p) |
           ((uint64_t)afspr_get_le32(p + 4) << 32);
}

static uint32_t afspr_crc32c_update(uint32_t crc, const uint8_t *data,
                                    size_t size)
{
    size_t i;

    for (i = 0; i < size; ++i) {
        unsigned bit;

        crc ^= data[i];
        for (bit = 0; bit < 8u; ++bit) {
            uint32_t mask = (uint32_t)(0u - (crc & 1u));
            crc = (crc >> 1) ^ (UINT32_C(0x82f63b78) & mask);
        }
    }
    return crc;
}

static uint32_t afspr_block_crc32c(const uint8_t *block, size_t block_size)
{
    static const uint8_t zero[4] = {0, 0, 0, 0};
    uint32_t crc = UINT32_MAX;

    crc = afspr_crc32c_update(crc, block, AFSPR_CHECKSUM_OFFSET);
    crc = afspr_crc32c_update(crc, zero, sizeof(zero));
    crc = afspr_crc32c_update(
        crc, block + AFSPR_CHECKSUM_OFFSET + sizeof(zero),
        block_size - AFSPR_CHECKSUM_OFFSET - sizeof(zero));
    return ~crc;
}

static int afspr_verify_header(const uint8_t *block, size_t block_size,
                               uint32_t expected_type,
                               struct afspr_header *header)
{
    uint32_t stored;

    if (block_size < AFSPR_HEADER_SIZE) {
        return AFSPR_ERR_CORRUPT;
    }
    stored = afspr_get_le32(block + AFSPR_CHECKSUM_OFFSET);
    if (stored != afspr_block_crc32c(block, block_size)) {
        return AFSPR_ERR_CORRUPT;
    }
    if (afspr_get_le32(block) != expected_type) {
        return AFSPR_ERR_CORRUPT;
    }
    if (afspr_get_le16(block + 4) != AFSPR_HEADER_VERSION) {
        return AFSPR_ERR_UNSUPPORTED;
    }
    header->flags = afspr_get_le16(block + 6);
    header->owner = afspr_get_le64(block + 8);
    header->generation = afspr_get_le64(block + 16);
    header->payload_len = afspr_get_le32(block + 24);
    if ((size_t)header->payload_len > block_size - AFSPR_HEADER_SIZE) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

static int afspr_valid_utf8(const uint8_t *text, size_t size)
{
    size_t i = 0;

    while (i < size) {
        uint8_t first = text[i++];
        size_t continuation;
        uint8_t lower = 0x80u;
        uint8_t upper = 0xbfu;

        if (first < 0x80u) {
            continue;
        }
        if (first >= 0xc2u && first <= 0xdfu) {
            continuation = 1;
        } else if (first >= 0xe0u && first <= 0xefu) {
            continuation = 2;
            if (first == 0xe0u) {
                lower = 0xa0u;
            } else if (first == 0xedu) {
                upper = 0x9fu;
            }
        } else if (first >= 0xf0u && first <= 0xf4u) {
            continuation = 3;
            if (first == 0xf0u) {
                lower = 0x90u;
            } else if (first == 0xf4u) {
                upper = 0x8fu;
            }
        } else {
            return 0;
        }
        if (i + continuation > size || text[i] < lower || text[i] > upper) {
            return 0;
        }
        ++i;
        while (--continuation != 0u) {
            if (text[i] < 0x80u || text[i] > 0xbfu) {
                return 0;
            }
            ++i;
        }
    }
    return 1;
}

static int afspr_power_of_two_u32(uint32_t value)
{
    return value != 0u && (value & (value - 1u)) == 0u;
}

static uint32_t afspr_region_valid_blocks(const struct afspr_ident *ident,
                                          uint32_t region)
{
    uint64_t base = (uint64_t)region * ident->region_size;
    uint64_t left = ident->total_blocks - base;

    return left < ident->region_size ? (uint32_t)left : ident->region_size;
}

static uint64_t afspr_region_reserved_blocks(const struct afspr_ident *ident,
                                             uint32_t region)
{
    uint32_t valid = afspr_region_valid_blocks(ident, region);
    uint32_t pages = valid / AFSPR_BITMAP_PAGE_BLOCKS;

    if (valid % AFSPR_BITMAP_PAGE_BLOCKS != 0u) {
        ++pages;
    }
    return AFSPR_DESCRIPTOR_SLOTS + (uint64_t)pages * AFSPR_BITMAP_SLOTS +
           (region == 0u ? AFSPR_BOOTSTRAP_BLOCKS : UINT64_C(0));
}

static int afspr_geometry_valid(const struct afspr_ident *ident)
{
    uint64_t regions;
    uint32_t last;

    if (ident->block_shift != AFSP_DEFAULT_BLOCK_SHIFT ||
        !afspr_power_of_two_u32(ident->region_size) ||
        ident->region_size < AFSPR_MIN_REGION_BLOCKS ||
        ident->region_size > AFSPR_MAX_REGION_BLOCKS ||
        ident->total_blocks == 0u) {
        return 0;
    }
    regions = ident->total_blocks / ident->region_size;
    if (ident->total_blocks % ident->region_size != 0u) {
        ++regions;
    }
    if (regions == 0u || regions > UINT32_MAX) {
        return 0;
    }
    last = (uint32_t)(regions - 1u);
    if ((uint64_t)afspr_region_valid_blocks(ident, 0) <=
            afspr_region_reserved_blocks(ident, 0) ||
        (last != 0u &&
         (uint64_t)afspr_region_valid_blocks(ident, last) <=
             afspr_region_reserved_blocks(ident, last))) {
        return 0;
    }
    return ident->metadata_start == afspr_region_reserved_blocks(ident, 0);
}

static int afspr_is_allocatable(const struct afspr_ident *ident, uint64_t lba)
{
    uint32_t region;
    uint64_t base;

    if (lba >= ident->total_blocks) {
        return 0;
    }
    region = (uint32_t)(lba / ident->region_size);
    base = (uint64_t)region * ident->region_size;
    return lba - base >= afspr_region_reserved_blocks(ident, region);
}

static int afspr_decode_ident(const uint8_t *block, size_t block_size,
                              struct afspr_ident *ident)
{
    struct afspr_header header;
    const uint8_t *p;
    size_t minimum_payload;
    int status = afspr_verify_header(block, block_size,
                                     AFSPR_BLOCK_TYPE_IDENT, &header);

    if (status != AFSPR_OK) {
        return status;
    }
    p = block + AFSPR_HEADER_SIZE;
    if (header.payload_len < AFSPR_IDENT_LEGACY_PAYLOAD ||
        afspr_get_le64(p) != AFSP_MAGIC_U64 ||
        afspr_get_le32(p + 8) != AFSP_FORMAT_EPOCH) {
        return AFSPR_ERR_CORRUPT;
    }
    ident->version = afspr_get_le32(p + 12);
    if (ident->version == 1u) {
        minimum_payload = AFSPR_IDENT_LEGACY_PAYLOAD;
    } else if (ident->version == 2u) {
        minimum_payload = AFSPR_IDENT_FEATURE_PAYLOAD;
    } else if (ident->version == 3u) {
        minimum_payload = AFSPR_IDENT_CURRENT_PAYLOAD;
    } else {
        return AFSPR_ERR_UNSUPPORTED;
    }
    if ((size_t)header.payload_len < minimum_payload) {
        return AFSPR_ERR_CORRUPT;
    }

    memcpy(ident->uuid, p + 16, sizeof(ident->uuid));
    ident->block_shift = p[32];
    ident->checksum_algorithm = p[33];
    ident->log_slots = afspr_get_le16(p + 34);
    ident->region_size = afspr_get_le32(p + 36);
    ident->total_blocks = afspr_get_le64(p + 40);
    ident->checkpoint_slots[0] = afspr_get_le64(p + 48);
    ident->checkpoint_slots[1] = afspr_get_le64(p + 56);
    ident->metadata_start = afspr_get_le64(p + 64);
    ident->label_len = p[72];
    if (ident->label_len > 64u ||
        !afspr_valid_utf8(p + 73, ident->label_len)) {
        return AFSPR_ERR_CORRUPT;
    }
    memcpy(ident->label, p + 73, ident->label_len);
    ident->label[ident->label_len] = '\0';

    if (ident->version >= 2u) {
        ident->compat_features = afspr_get_le64(p + 137);
        ident->ro_compat_features = afspr_get_le64(p + 145);
        ident->incompat_features = afspr_get_le64(p + 153);
    } else {
        ident->compat_features = 0u;
        ident->ro_compat_features = 0u;
        ident->incompat_features = ident->log_slots != 0u
                                         ? AFSP_INCOMPAT_INTENT_LOG
                                         : UINT64_C(0);
    }
    if (ident->version == 3u) {
        ident->name_key_algorithm = p[161];
        memcpy(ident->unicode_version, p + 162,
               sizeof(ident->unicode_version));
    } else {
        ident->name_key_algorithm = 0u;
        memset(ident->unicode_version, 0, sizeof(ident->unicode_version));
    }

    if (ident->block_shift != AFSP_DEFAULT_BLOCK_SHIFT ||
        ident->checksum_algorithm != AFSPR_CHECKSUM_CRC32C) {
        return AFSPR_ERR_UNSUPPORTED;
    }
    if ((ident->incompat_features & ~AFSPR_SUPPORTED_INCOMPAT) != 0u ||
        ident->name_key_algorithm > 2u) {
        return AFSPR_ERR_UNSUPPORTED;
    }
    if ((ident->log_slots != 0u) !=
        ((ident->incompat_features & AFSP_INCOMPAT_INTENT_LOG) != 0u)) {
        return AFSPR_ERR_CORRUPT;
    }
    if ((ident->incompat_features &
         AFSP_INCOMPAT_INTENT_LOG_DATA_UPDATES) != 0u &&
        (ident->incompat_features & AFSP_INCOMPAT_INTENT_LOG) == 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    if ((ident->name_key_algorithm == 0u &&
         (ident->unicode_version[0] != 0u ||
          ident->unicode_version[1] != 0u ||
          ident->unicode_version[2] != 0u)) ||
        (ident->name_key_algorithm != 0u &&
         (ident->unicode_version[0] != 16u ||
          ident->unicode_version[1] != 0u ||
          ident->unicode_version[2] != 0u))) {
        return AFSPR_ERR_UNSUPPORTED;
    }
    if (ident->checkpoint_slots[0] != 1u ||
        ident->checkpoint_slots[1] != 2u || !afspr_geometry_valid(ident)) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

static int afspr_decode_checkpoint(const uint8_t *block, size_t block_size,
                                   const struct afspr_ident *ident,
                                   struct afspr_checkpoint *checkpoint)
{
    struct afspr_header header;
    const uint8_t *p;
    uint64_t root_object;
    int status = afspr_verify_header(block, block_size,
                                     AFSPR_BLOCK_TYPE_CHECKPOINT, &header);

    if (status != AFSPR_OK) {
        return status;
    }
    if (header.payload_len != AFSPR_CHECKPOINT_PAYLOAD) {
        return AFSPR_ERR_CORRUPT;
    }
    p = block + AFSPR_HEADER_SIZE;
    if (memcmp(p, ident->uuid, sizeof(ident->uuid)) != 0) {
        return AFSPR_ERR_CORRUPT;
    }
    checkpoint->generation = afspr_get_le64(p + 16);
    root_object = afspr_get_le64(p + 24);
    if (checkpoint->generation == 0u ||
        checkpoint->generation != header.generation ||
        root_object != AFSP_OBJECT_ROOT) {
        return AFSPR_ERR_CORRUPT;
    }
    checkpoint->object_map_block = afspr_get_le64(p + 32);
    checkpoint->allocation_root_block = afspr_get_le64(p + 40);
    checkpoint->reclaim_root_block = afspr_get_le64(p + 48);
    checkpoint->next_object_id = afspr_get_le64(p + 56);
    checkpoint->committed_tx_id = afspr_get_le64(p + 64);
    checkpoint->free_blocks_total = afspr_get_le64(p + 72);
    checkpoint->shared_extent_root_block = afspr_get_le64(p + 88);
    if (!afspr_is_allocatable(ident, checkpoint->object_map_block) ||
        !afspr_is_allocatable(ident, checkpoint->allocation_root_block) ||
        !afspr_is_allocatable(ident, checkpoint->reclaim_root_block) ||
        (checkpoint->shared_extent_root_block != 0u &&
         !afspr_is_allocatable(ident,
                               checkpoint->shared_extent_root_block)) ||
        checkpoint->next_object_id < AFSPR_OBJECT_FIRST_DYNAMIC ||
        checkpoint->free_blocks_total > ident->total_blocks) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

static int afspr_read_one(const struct afspr_block_ops *ops, uint64_t block,
                          uint8_t *buffer)
{
    if (block >= ops->block_count ||
        ops->read_blocks(ops->ctx, block, 1u, buffer) != 0) {
        return AFSPR_ERR_IO;
    }
    return AFSPR_OK;
}

static void afspr_diagnostic_init(struct afspr_diagnostic *diagnostic)
{
    memset(diagnostic, 0, sizeof(*diagnostic));
    diagnostic->abi_version = AFSPR_ABI_VERSION;
    diagnostic->status = AFSPR_NOT_CHECKED;
    diagnostic->checkpoint_slot = AFSPR_NO_CHECKPOINT_SLOT;
    diagnostic->block = AFSPR_NO_BLOCK;
    diagnostic->checkpoint_status[0] = AFSPR_NOT_CHECKED;
    diagnostic->checkpoint_status[1] = AFSPR_NOT_CHECKED;
}

static int afspr_report(struct afspr_diagnostic *diagnostic, int status,
                        uint32_t stage, int32_t checkpoint_slot,
                        uint64_t block)
{
    if (diagnostic != NULL) {
        diagnostic->status = status;
        diagnostic->stage = stage;
        diagnostic->checkpoint_slot = checkpoint_slot;
        diagnostic->block = block;
    }
    return status;
}

int afspr_probe_detailed(const struct afspr_block_ops *ops,
                         const struct afspr_scratch *scratch,
                         struct afspr_probe_result *result, size_t result_size,
                         struct afspr_diagnostic *diagnostic,
                         size_t diagnostic_size)
{
    struct afspr_ident ident;
    struct afspr_checkpoint candidates[2];
    uint8_t valid_mask = 0u;
    uint8_t selected;
    uint8_t *buffer;
    unsigned slot;
    int status;

    if (diagnostic != NULL && diagnostic_size < sizeof(*diagnostic)) {
        return AFSPR_ERR_ABI;
    }
    if (diagnostic != NULL) {
        afspr_diagnostic_init(diagnostic);
    }
    if (ops == NULL || scratch == NULL || result == NULL ||
        ops->read_blocks == NULL || scratch->buffer == NULL) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (ops->abi_version != AFSPR_ABI_VERSION ||
        ops->struct_size < sizeof(*ops) || result_size < sizeof(*result)) {
        return afspr_report(diagnostic, AFSPR_ERR_ABI,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (ops->block_size != AFSP_DEFAULT_BLOCK_SIZE) {
        return afspr_report(diagnostic, AFSPR_ERR_UNSUPPORTED,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (scratch->size < ops->block_size) {
        return afspr_report(diagnostic, AFSPR_ERR_SCRATCH_TOO_SMALL,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    buffer = (uint8_t *)scratch->buffer;
    memset(&ident, 0, sizeof(ident));
    status = afspr_read_one(ops, 0u, buffer);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_IDENTIFICATION_READ,
                            AFSPR_NO_CHECKPOINT_SLOT, 0u);
    }
    status = afspr_decode_ident(buffer, ops->block_size, &ident);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_IDENTIFICATION_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, 0u);
    }
    if (ident.total_blocks > ops->block_count) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_IDENTIFICATION_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, 0u);
    }

    memset(candidates, 0, sizeof(candidates));
    for (slot = 0; slot < 2u; ++slot) {
        status = afspr_read_one(ops, ident.checkpoint_slots[slot], buffer);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status,
                                AFSPR_STAGE_CHECKPOINT_READ, (int32_t)slot,
                                ident.checkpoint_slots[slot]);
        }
        status = afspr_decode_checkpoint(buffer, ops->block_size, &ident,
                                         &candidates[slot]);
        if (status == AFSPR_OK) {
            valid_mask = (uint8_t)(valid_mask | (uint8_t)(1u << slot));
        }
        if (diagnostic != NULL) {
            diagnostic->checkpoint_status[slot] = status;
        }
    }
    if (valid_mask == 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_NO_CHECKPOINT,
                            AFSPR_STAGE_CHECKPOINT_SELECTION,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            AFSPR_NO_BLOCK);
    }
    if (valid_mask == 3u &&
        candidates[0].generation == candidates[1].generation) {
        return afspr_report(diagnostic, AFSPR_ERR_AMBIGUOUS_CHECKPOINT,
                            AFSPR_STAGE_CHECKPOINT_SELECTION,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            AFSPR_NO_BLOCK);
    }
    selected = (valid_mask == 2u ||
                (valid_mask == 3u && candidates[1].generation >
                                        candidates[0].generation))
                   ? 1u
                   : 0u;
    if (candidates[selected].shared_extent_root_block != 0u &&
        (ident.ro_compat_features & AFSP_RO_COMPAT_SHARED_EXTENTS) == 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_CHECKPOINT_SELECTION,
                            (int32_t)selected,
                            ident.checkpoint_slots[selected]);
    }

    memset(result, 0, sizeof(*result));
    result->abi_version = AFSPR_ABI_VERSION;
    result->identification_version = ident.version;
    memcpy(result->uuid, ident.uuid, sizeof(result->uuid));
    result->block_shift = ident.block_shift;
    result->checksum_algorithm = ident.checksum_algorithm;
    result->log_slots = ident.log_slots;
    result->region_size = ident.region_size;
    result->total_blocks = ident.total_blocks;
    result->metadata_start = ident.metadata_start;
    result->compat_features = ident.compat_features;
    result->ro_compat_features = ident.ro_compat_features;
    result->incompat_features = ident.incompat_features;
    result->name_key_algorithm = ident.name_key_algorithm;
    memcpy(result->unicode_version, ident.unicode_version,
           sizeof(result->unicode_version));
    memcpy(result->label, ident.label, sizeof(result->label));
    result->selected_checkpoint = selected;
    result->valid_checkpoint_mask = valid_mask;
    result->generation = candidates[selected].generation;
    result->object_map_block = candidates[selected].object_map_block;
    result->allocation_root_block = candidates[selected].allocation_root_block;
    result->reclaim_root_block = candidates[selected].reclaim_root_block;
    result->next_object_id = candidates[selected].next_object_id;
    result->committed_tx_id = candidates[selected].committed_tx_id;
    result->free_blocks_total = candidates[selected].free_blocks_total;
    result->shared_extent_root_block =
        candidates[selected].shared_extent_root_block;
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        (int32_t)selected,
                        ident.checkpoint_slots[selected]);
}

int afspr_probe(const struct afspr_block_ops *ops,
                const struct afspr_scratch *scratch,
                struct afspr_probe_result *result, size_t result_size)
{
    return afspr_probe_detailed(ops, scratch, result, result_size, NULL, 0u);
}

const char *afspr_status_string(int status)
{
    switch (status) {
    case AFSPR_NOT_CHECKED:
        return "not checked";
    case AFSPR_OK:
        return "success";
    case AFSPR_ERR_INVALID_ARGUMENT:
        return "invalid argument";
    case AFSPR_ERR_ABI:
        return "ABI mismatch";
    case AFSPR_ERR_IO:
        return "block I/O failure";
    case AFSPR_ERR_SCRATCH_TOO_SMALL:
        return "scratch buffer too small";
    case AFSPR_ERR_UNSUPPORTED:
        return "unsupported format";
    case AFSPR_ERR_CORRUPT:
        return "corrupt format";
    case AFSPR_ERR_NO_CHECKPOINT:
        return "no valid checkpoint";
    case AFSPR_ERR_AMBIGUOUS_CHECKPOINT:
        return "ambiguous checkpoints";
    default:
        return "unknown reader status";
    }
}

const char *afspr_probe_stage_string(uint32_t stage)
{
    switch (stage) {
    case AFSPR_STAGE_NONE:
        return "none";
    case AFSPR_STAGE_ARGUMENTS:
        return "arguments";
    case AFSPR_STAGE_IDENTIFICATION_READ:
        return "identification read";
    case AFSPR_STAGE_IDENTIFICATION_DECODE:
        return "identification decode";
    case AFSPR_STAGE_CHECKPOINT_READ:
        return "checkpoint read";
    case AFSPR_STAGE_CHECKPOINT_DECODE:
        return "checkpoint decode";
    case AFSPR_STAGE_CHECKPOINT_SELECTION:
        return "checkpoint selection";
    case AFSPR_STAGE_COMPLETE:
        return "complete";
    default:
        return "unknown probe stage";
    }
}
