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
#define AFSPR_BLOCK_TYPE_OBJECT UINT32_C(0x4f534641)
#define AFSPR_BLOCK_TYPE_TREE UINT32_C(0x54534641)
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
#define AFSPR_TREE_FIXED_PAYLOAD 32u
#define AFSPR_TREE_ITEM_FIXED 8u
#define AFSPR_TREE_MAX_LEVEL 15u
#define AFSPR_TREE_MAX_KEY 1020u
#define AFSPR_TREE_KIND_OBJECT_MAP 1u
#define AFSPR_TREE_KIND_DIRECTORY 2u
#define AFSPR_TREE_KIND_EXTENT_MAP 3u
#define AFSPR_OBJECT_PAYLOAD 96u
#define AFSPR_MAX_DIRECT_BLOCKS UINT64_C(4096)
#define AFSPR_EXTENT_VALUE_SIZE 24u
#define AFSPR_EXTENT_UNWRITTEN (UINT32_C(1) << 0)
#define AFSPR_EXTENT_SHARED (UINT32_C(1) << 1)
#define AFSPR_EXTENT_KNOWN_FLAGS (AFSPR_EXTENT_UNWRITTEN | AFSPR_EXTENT_SHARED)

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

struct afspr_tree_spec {
    uint8_t kind;
    uint64_t owner;
    uint64_t max_generation;
};

struct afspr_tree_node {
    const uint8_t *payload;
    size_t payload_len;
    uint8_t level;
    uint32_t count;
    uint64_t subtree_items;
    uint64_t leftmost_child;
    uint64_t leftmost_items;
};

struct afspr_tree_item {
    const uint8_t *key;
    size_t key_len;
    const uint8_t *value;
    size_t value_len;
};

struct afspr_extent {
    uint64_t logical_start;
    uint64_t physical_start;
    uint64_t block_count;
    uint32_t flags;
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

static void afspr_put_be64(uint8_t *p, uint64_t value)
{
    unsigned index;

    for (index = 0; index < 8u; ++index) {
        p[index] = (uint8_t)(value >> (56u - index * 8u));
    }
}

static uint64_t afspr_get_be64(const uint8_t *p)
{
    uint64_t value = 0u;
    unsigned index;

    for (index = 0; index < 8u; ++index) {
        value = (value << 8) | p[index];
    }
    return value;
}

static int afspr_bytes_compare(const uint8_t *left, size_t left_len,
                               const uint8_t *right, size_t right_len)
{
    size_t common = left_len < right_len ? left_len : right_len;
    int compared = memcmp(left, right, common);

    if (compared != 0) {
        return compared;
    }
    return left_len < right_len ? -1 : left_len > right_len ? 1 : 0;
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

static int afspr_tree_item_at(const struct afspr_tree_node *node,
                              uint32_t wanted,
                              struct afspr_tree_item *item)
{
    size_t offset = AFSPR_TREE_FIXED_PAYLOAD;
    uint32_t index;

    if (wanted >= node->count) {
        return AFSPR_ERR_CORRUPT;
    }
    for (index = 0; index <= wanted; ++index) {
        size_t key_len;
        size_t value_len;

        if (offset > node->payload_len ||
            node->payload_len - offset < AFSPR_TREE_ITEM_FIXED) {
            return AFSPR_ERR_CORRUPT;
        }
        key_len = afspr_get_le16(node->payload + offset);
        value_len = afspr_get_le16(node->payload + offset + 2u);
        offset += AFSPR_TREE_ITEM_FIXED;
        if (key_len > node->payload_len - offset ||
            value_len > node->payload_len - offset - key_len) {
            return AFSPR_ERR_CORRUPT;
        }
        if (index == wanted) {
            item->key = node->payload + offset;
            item->key_len = key_len;
            item->value = node->payload + offset + key_len;
            item->value_len = value_len;
            return AFSPR_OK;
        }
        offset += key_len + value_len;
    }
    return AFSPR_ERR_CORRUPT;
}

static int afspr_tree_child(const struct afspr_ident *ident,
                            const struct afspr_tree_item *item,
                            uint64_t *lba, uint64_t *subtree_items)
{
    if (item->value_len != 16u) {
        return AFSPR_ERR_CORRUPT;
    }
    *lba = afspr_get_le64(item->value);
    *subtree_items = afspr_get_le64(item->value + 8u);
    if (!afspr_is_allocatable(ident, *lba) || *subtree_items == 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

static int afspr_tree_item_shape(const struct afspr_tree_spec *spec,
                                 uint8_t level,
                                 const struct afspr_tree_item *item)
{
    if (level != 0u) {
        return item->value_len == 16u ? AFSPR_OK : AFSPR_ERR_CORRUPT;
    }
    if (spec->kind == AFSPR_TREE_KIND_OBJECT_MAP) {
        return item->key_len == 8u && item->value_len == 8u
                   ? AFSPR_OK
                   : AFSPR_ERR_CORRUPT;
    }
    if (spec->kind == AFSPR_TREE_KIND_EXTENT_MAP) {
        return item->key_len == 8u &&
                       item->value_len == AFSPR_EXTENT_VALUE_SIZE
                   ? AFSPR_OK
                   : AFSPR_ERR_CORRUPT;
    }
    if (spec->kind == AFSPR_TREE_KIND_DIRECTORY) {
        return item->value_len >= 16u ? AFSPR_OK : AFSPR_ERR_CORRUPT;
    }
    return AFSPR_ERR_UNSUPPORTED;
}

static int afspr_decode_tree_node(
    const uint8_t *block, size_t block_size, const struct afspr_ident *ident,
    const struct afspr_tree_spec *spec, int expected_level, int is_root,
    const uint8_t *lower, size_t lower_len, const uint8_t *upper,
    size_t upper_len, struct afspr_tree_node *node)
{
    struct afspr_header header;
    struct afspr_tree_item item;
    const uint8_t *previous_key = NULL;
    size_t previous_key_len = 0u;
    size_t offset = AFSPR_TREE_FIXED_PAYLOAD;
    uint64_t counted;
    uint32_t index;
    int status = afspr_verify_header(block, block_size,
                                     AFSPR_BLOCK_TYPE_TREE, &header);

    if (status != AFSPR_OK) {
        return status;
    }
    if (header.flags != 0u || header.owner != spec->owner ||
        header.generation == 0u || header.generation > spec->max_generation ||
        header.payload_len < AFSPR_TREE_FIXED_PAYLOAD) {
        return AFSPR_ERR_CORRUPT;
    }
    node->payload = block + AFSPR_HEADER_SIZE;
    node->payload_len = header.payload_len;
    if (node->payload[0] != spec->kind || node->payload[2] != 0u ||
        node->payload[3] != 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    node->level = node->payload[1];
    node->count = afspr_get_le32(node->payload + 4u);
    node->subtree_items = afspr_get_le64(node->payload + 8u);
    node->leftmost_child = afspr_get_le64(node->payload + 16u);
    node->leftmost_items = afspr_get_le64(node->payload + 24u);
    if (node->level > AFSPR_TREE_MAX_LEVEL ||
        (expected_level >= 0 && node->level != (uint8_t)expected_level) ||
        (!is_root && node->count == 0u) ||
        node->count >
            (node->payload_len - AFSPR_TREE_FIXED_PAYLOAD) /
                AFSPR_TREE_ITEM_FIXED) {
        return AFSPR_ERR_CORRUPT;
    }

    counted = node->level == 0u ? node->count : node->leftmost_items;
    if (node->level == 0u) {
        if (node->leftmost_child != 0u || node->leftmost_items != 0u) {
            return AFSPR_ERR_CORRUPT;
        }
    } else if (!afspr_is_allocatable(ident, node->leftmost_child) ||
               node->leftmost_items == 0u || node->count == 0u) {
        return AFSPR_ERR_CORRUPT;
    }

    for (index = 0; index < node->count; ++index) {
        uint64_t child_lba;
        uint64_t child_items;

        if (offset > node->payload_len ||
            node->payload_len - offset < AFSPR_TREE_ITEM_FIXED ||
            node->payload[offset + 4u] != 0u ||
            node->payload[offset + 5u] != 0u ||
            node->payload[offset + 6u] != 0u ||
            node->payload[offset + 7u] != 0u) {
            return AFSPR_ERR_CORRUPT;
        }
        item.key_len = afspr_get_le16(node->payload + offset);
        item.value_len = afspr_get_le16(node->payload + offset + 2u);
        offset += AFSPR_TREE_ITEM_FIXED;
        if (item.key_len == 0u || item.key_len > AFSPR_TREE_MAX_KEY ||
            item.value_len == 0u || item.key_len > node->payload_len - offset ||
            item.value_len > node->payload_len - offset - item.key_len) {
            return AFSPR_ERR_CORRUPT;
        }
        item.key = node->payload + offset;
        item.value = item.key + item.key_len;
        if ((previous_key != NULL &&
             afspr_bytes_compare(previous_key, previous_key_len, item.key,
                                 item.key_len) >= 0) ||
            afspr_tree_item_shape(spec, node->level, &item) != AFSPR_OK) {
            return AFSPR_ERR_CORRUPT;
        }
        if (node->level != 0u) {
            status = afspr_tree_child(ident, &item, &child_lba,
                                      &child_items);
            if (status != AFSPR_OK || UINT64_MAX - counted < child_items) {
                return AFSPR_ERR_CORRUPT;
            }
            counted += child_items;
        }
        previous_key = item.key;
        previous_key_len = item.key_len;
        offset += item.key_len + item.value_len;
    }
    if (offset != node->payload_len || counted != node->subtree_items) {
        return AFSPR_ERR_CORRUPT;
    }
    if (node->count != 0u) {
        struct afspr_tree_item first;
        struct afspr_tree_item last;

        if (afspr_tree_item_at(node, 0u, &first) != AFSPR_OK ||
            afspr_tree_item_at(node, node->count - 1u, &last) != AFSPR_OK) {
            return AFSPR_ERR_CORRUPT;
        }
        if (lower != NULL &&
            (afspr_bytes_compare(first.key, first.key_len, lower, lower_len) <
                 0 ||
             (node->level == 0u &&
              afspr_bytes_compare(first.key, first.key_len, lower,
                                  lower_len) != 0))) {
            return AFSPR_ERR_CORRUPT;
        }
        if (upper != NULL &&
            afspr_bytes_compare(last.key, last.key_len, upper, upper_len) >=
                0) {
            return AFSPR_ERR_CORRUPT;
        }
    }
    return AFSPR_OK;
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

static void afspr_ident_from_result(const struct afspr_probe_result *volume,
                                    struct afspr_ident *ident)
{
    memset(ident, 0, sizeof(*ident));
    ident->version = volume->identification_version;
    memcpy(ident->uuid, volume->uuid, sizeof(ident->uuid));
    ident->block_shift = volume->block_shift;
    ident->checksum_algorithm = volume->checksum_algorithm;
    ident->log_slots = volume->log_slots;
    ident->region_size = volume->region_size;
    ident->total_blocks = volume->total_blocks;
    ident->checkpoint_slots[0] = 1u;
    ident->checkpoint_slots[1] = 2u;
    ident->metadata_start = volume->metadata_start;
    ident->compat_features = volume->compat_features;
    ident->ro_compat_features = volume->ro_compat_features;
    ident->incompat_features = volume->incompat_features;
    ident->name_key_algorithm = volume->name_key_algorithm;
    memcpy(ident->unicode_version, volume->unicode_version,
           sizeof(ident->unicode_version));
}

static int afspr_prepare_operation(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, size_t minimum_scratch,
    struct afspr_ident *ident, struct afspr_diagnostic *diagnostic,
    size_t diagnostic_size)
{
    if (diagnostic != NULL && diagnostic_size < sizeof(*diagnostic)) {
        return AFSPR_ERR_ABI;
    }
    if (diagnostic != NULL) {
        afspr_diagnostic_init(diagnostic);
    }
    if (ops == NULL || scratch == NULL || volume == NULL || ident == NULL ||
        ops->read_blocks == NULL || scratch->buffer == NULL) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (ops->abi_version != AFSPR_ABI_VERSION ||
        ops->struct_size < sizeof(*ops) ||
        volume->abi_version != AFSPR_ABI_VERSION) {
        return afspr_report(diagnostic, AFSPR_ERR_ABI,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (ops->block_size != AFSP_DEFAULT_BLOCK_SIZE ||
        volume->block_shift != AFSP_DEFAULT_BLOCK_SHIFT) {
        return afspr_report(diagnostic, AFSPR_ERR_UNSUPPORTED,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (scratch->size < minimum_scratch) {
        return afspr_report(diagnostic, AFSPR_ERR_SCRATCH_TOO_SMALL,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    afspr_ident_from_result(volume, ident);
    if (volume->generation == 0u || volume->total_blocks == 0u ||
        volume->total_blocks > ops->block_count ||
        !afspr_geometry_valid(ident)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    return AFSPR_OK;
}

static int afspr_tree_lookup_fixed(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident, uint64_t root_lba,
    const struct afspr_tree_spec *spec, const uint8_t search[8], int floor,
    uint8_t found_key[8], uint8_t *found_value, size_t value_capacity,
    size_t *found_value_len, int *found, uint64_t *leaf_lba,
    struct afspr_diagnostic *diagnostic)
{
    uint64_t visited[AFSPR_TREE_MAX_LEVEL + 1u];
    uint8_t lower[8];
    uint8_t upper[8];
    size_t lower_len = 0u;
    size_t upper_len = 0u;
    uint64_t lba = root_lba;
    int expected_level = -1;
    unsigned depth;

    *found = 0;
    *found_value_len = 0u;
    if (!afspr_is_allocatable(ident, root_lba)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_TRAVERSAL,
                            AFSPR_NO_CHECKPOINT_SLOT, root_lba);
    }
    for (depth = 0; depth <= AFSPR_TREE_MAX_LEVEL; ++depth) {
        struct afspr_tree_node node;
        uint32_t index;
        uint32_t separator = 0u;
        int status;

        for (index = 0; index < depth; ++index) {
            if (visited[index] == lba) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_TRAVERSAL,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
        }
        visited[depth] = lba;
        status = afspr_read_one(ops, lba, (uint8_t *)scratch->buffer);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_TREE_READ,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        status = afspr_decode_tree_node(
            (const uint8_t *)scratch->buffer, ops->block_size, ident, spec,
            expected_level, depth == 0u,
            lower_len == 0u ? NULL : lower, lower_len,
            upper_len == 0u ? NULL : upper, upper_len, &node);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_TREE_DECODE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        if (node.level == 0u) {
            struct afspr_tree_item candidate;
            struct afspr_tree_item item;
            int have_candidate = 0;

            *leaf_lba = lba;

            for (index = 0; index < node.count; ++index) {
                int compared;

                status = afspr_tree_item_at(&node, index, &item);
                if (status != AFSPR_OK) {
                    return afspr_report(diagnostic, status,
                                        AFSPR_STAGE_TREE_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                compared = afspr_bytes_compare(item.key, item.key_len,
                                               search, 8u);
                if ((!floor && compared == 0) || (floor && compared <= 0)) {
                    candidate = item;
                    have_candidate = 1;
                    if (!floor || compared == 0) {
                        break;
                    }
                } else if (compared > 0) {
                    break;
                }
            }
            if (have_candidate) {
                if (candidate.key_len != 8u ||
                    candidate.value_len > value_capacity) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_TREE_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                memcpy(found_key, candidate.key, 8u);
                memcpy(found_value, candidate.value,
                       candidate.value_len);
                *found_value_len = candidate.value_len;
                *found = 1;
            }
            return AFSPR_OK;
        }

        for (index = 0; index < node.count; ++index) {
            struct afspr_tree_item item;

            status = afspr_tree_item_at(&node, index, &item);
            if (status != AFSPR_OK) {
                return afspr_report(diagnostic, status,
                                    AFSPR_STAGE_TREE_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (afspr_bytes_compare(item.key, item.key_len, search, 8u) <=
                0) {
                separator = index + 1u;
            } else {
                break;
            }
        }
        if (separator == 0u) {
            struct afspr_tree_item first;

            status = afspr_tree_item_at(&node, 0u, &first);
            if (status != AFSPR_OK || first.key_len != 8u) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            memcpy(upper, first.key, 8u);
            upper_len = 8u;
            lba = node.leftmost_child;
        } else {
            struct afspr_tree_item selected_item;
            uint64_t ignored_items;

            status = afspr_tree_item_at(&node, separator - 1u,
                                        &selected_item);
            if (status != AFSPR_OK || selected_item.key_len != 8u ||
                afspr_tree_child(ident, &selected_item, &lba,
                                 &ignored_items) != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            memcpy(lower, selected_item.key, 8u);
            lower_len = 8u;
            if (separator < node.count) {
                struct afspr_tree_item next;

                status = afspr_tree_item_at(&node, separator, &next);
                if (status != AFSPR_OK || next.key_len != 8u) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_TREE_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                memcpy(upper, next.key, 8u);
                upper_len = 8u;
            }
        }
        if (node.level == 0u || !afspr_is_allocatable(ident, lba)) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_TREE_TRAVERSAL,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        expected_level = (int)node.level - 1;
    }
    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_TREE_TRAVERSAL,
                        AFSPR_NO_CHECKPOINT_SLOT, lba);
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

uint64_t afspr_capabilities(void)
{
    return AFSPR_CAP_PROBE | AFSPR_CAP_OBJECT_LOOKUP |
           AFSPR_CAP_DIRECTORY_ORDINAL | AFSPR_CAP_FILE_READ;
}

static int afspr_decode_timespec(const uint8_t *encoded,
                                 struct afspr_timespec *timestamp)
{
    timestamp->seconds = (int64_t)afspr_get_le64(encoded);
    timestamp->nanoseconds = afspr_get_le32(encoded + 8u);
    timestamp->reserved = 0u;
    return timestamp->nanoseconds < UINT32_C(1000000000)
               ? AFSPR_OK
               : AFSPR_ERR_CORRUPT;
}

static int afspr_decode_object(const uint8_t *block, size_t block_size,
                               const struct afspr_ident *ident,
                               uint64_t max_generation,
                               uint64_t expected_object_id,
                               struct afspr_object *object)
{
    struct afspr_header header;
    const uint8_t *p;
    uint64_t expected_allocated;
    uint64_t data_end;
    uint64_t lba;
    int status = afspr_verify_header(block, block_size,
                                     AFSPR_BLOCK_TYPE_OBJECT, &header);

    if (status != AFSPR_OK) {
        return status;
    }
    if (header.flags != 0u || header.owner != expected_object_id ||
        header.generation == 0u || header.generation > max_generation ||
        header.payload_len < AFSPR_OBJECT_PAYLOAD) {
        return AFSPR_ERR_CORRUPT;
    }
    p = block + AFSPR_HEADER_SIZE;
    memset(object, 0, sizeof(*object));
    object->abi_version = AFSPR_ABI_VERSION;
    object->object_id = afspr_get_le64(p);
    object->type = p[8];
    object->flags = afspr_get_le16(p + 10u);
    object->link_count = afspr_get_le32(p + 12u);
    object->size_bytes = afspr_get_le64(p + 16u);
    object->allocated_bytes = afspr_get_le64(p + 24u);
    object->protection = afspr_get_le32(p + 68u);
    object->content_generation = afspr_get_le64(p + 72u);
    object->data_root = afspr_get_le64(p + 80u);
    object->data_blocks = afspr_get_le64(p + 88u);
    if (p[9] != 0u || object->object_id != expected_object_id ||
        object->link_count == 0u ||
        (object->flags & ~AFSPR_OBJECT_FLAG_EXTENT_TREE) != 0u ||
        afspr_decode_timespec(p + 32u, &object->created) != AFSPR_OK ||
        afspr_decode_timespec(p + 44u, &object->modified) != AFSPR_OK ||
        afspr_decode_timespec(p + 56u, &object->changed) != AFSPR_OK) {
        return AFSPR_ERR_CORRUPT;
    }
    if (object->type == AFSPR_OBJECT_DIRECTORY) {
        if (object->flags != 0u || object->size_bytes != 0u ||
            object->data_blocks != 0u ||
            !afspr_is_allocatable(ident, object->data_root)) {
            return AFSPR_ERR_CORRUPT;
        }
        return AFSPR_OK;
    }
    if (object->type != AFSPR_OBJECT_FILE) {
        return object->type == AFSPR_OBJECT_SYMLINK ||
                       object->type == AFSPR_OBJECT_INTERNAL
                   ? AFSPR_ERR_UNSUPPORTED
                   : AFSPR_ERR_CORRUPT;
    }
    if (object->data_blocks > UINT64_MAX / block_size) {
        return AFSPR_ERR_CORRUPT;
    }
    expected_allocated = object->data_blocks * block_size;
    if (object->allocated_bytes != expected_allocated) {
        return AFSPR_ERR_CORRUPT;
    }
    if ((object->flags & AFSPR_OBJECT_FLAG_EXTENT_TREE) != 0u) {
        return afspr_is_allocatable(ident, object->data_root)
                   ? AFSPR_OK
                   : AFSPR_ERR_CORRUPT;
    }
    if (object->data_blocks > AFSPR_MAX_DIRECT_BLOCKS) {
        return AFSPR_ERR_CORRUPT;
    }
    if (object->data_blocks == 0u) {
        return object->data_root == 0u && object->size_bytes == 0u
                   ? AFSPR_OK
                   : AFSPR_ERR_CORRUPT;
    }
    if (UINT64_MAX - object->data_root < object->data_blocks) {
        return AFSPR_ERR_CORRUPT;
    }
    data_end = object->data_root + object->data_blocks;
    if (object->size_bytes > expected_allocated ||
        object->size_bytes <= expected_allocated - block_size) {
        return AFSPR_ERR_CORRUPT;
    }
    for (lba = object->data_root; lba < data_end; ++lba) {
        if (!afspr_is_allocatable(ident, lba)) {
            return AFSPR_ERR_CORRUPT;
        }
    }
    return AFSPR_OK;
}

int afspr_lookup_object(const struct afspr_block_ops *ops,
                        const struct afspr_scratch *scratch,
                        const struct afspr_probe_result *volume,
                        uint64_t object_id, struct afspr_object *object,
                        size_t object_size,
                        struct afspr_diagnostic *diagnostic,
                        size_t diagnostic_size)
{
    struct afspr_ident ident;
    struct afspr_tree_spec spec;
    uint8_t key[8];
    uint8_t found_key[8];
    uint8_t value[8];
    size_t value_len;
    uint64_t object_lba;
    uint64_t leaf_lba;
    int found;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_MIN_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (object == NULL || object_size < sizeof(*object) || object_id == 0u) {
        return afspr_report(diagnostic,
                            object_size < sizeof(*object) ? AFSPR_ERR_ABI
                                                          : AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    spec.kind = AFSPR_TREE_KIND_OBJECT_MAP;
    spec.owner = 0u;
    spec.max_generation = volume->generation;
    afspr_put_be64(key, object_id);
    status = afspr_tree_lookup_fixed(
        ops, scratch, &ident, volume->object_map_block, &spec, key, 0,
        found_key, value, sizeof(value), &value_len, &found, &leaf_lba,
        diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    if (!found) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                            AFSPR_STAGE_TREE_TRAVERSAL,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            volume->object_map_block);
    }
    if (value_len != sizeof(value) || afspr_get_be64(found_key) != object_id) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, leaf_lba);
    }
    object_lba = afspr_get_le64(value);
    if (!afspr_is_allocatable(&ident, object_lba)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, object_lba);
    }
    status = afspr_read_one(ops, object_lba, (uint8_t *)scratch->buffer);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status, AFSPR_STAGE_OBJECT_READ,
                            AFSPR_NO_CHECKPOINT_SLOT, object_lba);
    }
    status = afspr_decode_object((const uint8_t *)scratch->buffer,
                                 ops->block_size, &ident,
                                 volume->generation, object_id, object);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status, AFSPR_STAGE_OBJECT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, object_lba);
    }
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        AFSPR_NO_CHECKPOINT_SLOT, object_lba);
}

static int afspr_ascii_key_matches(const struct afspr_ident *ident,
                                   const uint8_t *key, size_t key_len,
                                   const uint8_t *name, size_t name_len)
{
    size_t index;

    if (ident->name_key_algorithm == 0u) {
        return key_len == name_len && memcmp(key, name, name_len) == 0;
    }
    for (index = 0; index < name_len; ++index) {
        if (name[index] >= 0x80u) {
            return 1;
        }
    }
    if (key_len != name_len) {
        return 0;
    }
    for (index = 0; index < key_len; ++index) {
        if (key[index] >= 0x80u) {
            return 0;
        }
    }
    for (index = 0; index < name_len; ++index) {
        uint8_t expected = name[index];

        if (ident->name_key_algorithm == 2u && expected >= 'A' &&
            expected <= 'Z') {
            expected = (uint8_t)(expected + ('a' - 'A'));
        }
        if (key[index] != expected) {
            return 0;
        }
    }
    return 1;
}

static int afspr_decode_directory_entry(
    const struct afspr_ident *ident, const struct afspr_tree_item *item,
    uint64_t parent_id, void *name_buffer, size_t name_capacity,
    struct afspr_directory_entry *entry)
{
    size_t name_len;
    const uint8_t *name;

    if (item->value_len < 16u) {
        return AFSPR_ERR_CORRUPT;
    }
    name_len = afspr_get_le16(item->value);
    if (item->value_len != 16u + name_len || name_len == 0u ||
        name_len > 255u || item->value[3] != 0u || item->value[4] != 0u ||
        item->value[5] != 0u || item->value[6] != 0u ||
        item->value[7] != 0u ||
        (item->value[2] != AFSPR_OBJECT_FILE &&
         item->value[2] != AFSPR_OBJECT_DIRECTORY)) {
        return AFSPR_ERR_CORRUPT;
    }
    name = item->value + 16u;
    if (!afspr_valid_utf8(name, name_len) || memchr(name, 0, name_len) != NULL ||
        memchr(name, '/', name_len) != NULL ||
        afspr_get_le64(item->value + 8u) == 0u ||
        !afspr_ascii_key_matches(ident, item->key, item->key_len, name,
                                 name_len)) {
        return AFSPR_ERR_CORRUPT;
    }
    memset(entry, 0, sizeof(*entry));
    entry->abi_version = AFSPR_ABI_VERSION;
    entry->type_hint = item->value[2];
    entry->object_id = afspr_get_le64(item->value + 8u);
    entry->parent_id = parent_id;
    entry->name_len = name_len;
    if (name_capacity < name_len) {
        return AFSPR_ERR_BUFFER_TOO_SMALL;
    }
    if (name_buffer == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    memcpy(name_buffer, name, name_len);
    entry->name = (const uint8_t *)name_buffer;
    return AFSPR_OK;
}

int afspr_directory_entry_at(const struct afspr_block_ops *ops,
                             const struct afspr_scratch *scratch,
                             const struct afspr_probe_result *volume,
                             const struct afspr_object *directory,
                             uint64_t ordinal, void *name_buffer,
                             size_t name_capacity,
                             struct afspr_directory_entry *entry,
                             size_t entry_size, uint64_t *total_entries,
                             struct afspr_diagnostic *diagnostic,
                             size_t diagnostic_size)
{
    struct afspr_ident ident;
    struct afspr_tree_spec spec;
    uint64_t visited[AFSPR_TREE_MAX_LEVEL + 1u];
    uint8_t *block;
    uint8_t *lower;
    uint8_t *upper;
    size_t lower_len = 0u;
    size_t upper_len = 0u;
    uint64_t lba;
    uint64_t remaining = ordinal;
    int expected_level = -1;
    unsigned depth;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_TREE_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (directory == NULL || entry == NULL ||
        entry_size < sizeof(*entry)) {
        return afspr_report(diagnostic,
                            entry != NULL && entry_size < sizeof(*entry)
                                ? AFSPR_ERR_ABI
                                : AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (directory->abi_version != AFSPR_ABI_VERSION) {
        return afspr_report(diagnostic, AFSPR_ERR_ABI,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (directory->type != AFSPR_OBJECT_DIRECTORY) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_DIRECTORY,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (directory->flags != 0u || directory->size_bytes != 0u ||
        directory->data_blocks != 0u ||
        !afspr_is_allocatable(&ident, directory->data_root)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_TRAVERSAL,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            directory->data_root);
    }

    block = (uint8_t *)scratch->buffer;
    lower = block + AFSP_DEFAULT_BLOCK_SIZE;
    upper = lower + AFSPR_TREE_MAX_KEY;
    lba = directory->data_root;
    spec.kind = AFSPR_TREE_KIND_DIRECTORY;
    spec.owner = directory->object_id;
    spec.max_generation = volume->generation;

    for (depth = 0; depth <= AFSPR_TREE_MAX_LEVEL; ++depth) {
        struct afspr_tree_node node;
        uint32_t index;

        for (index = 0; index < depth; ++index) {
            if (visited[index] == lba) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_TRAVERSAL,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
        }
        visited[depth] = lba;
        status = afspr_read_one(ops, lba, block);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_TREE_READ,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        status = afspr_decode_tree_node(
            block, ops->block_size, &ident, &spec, expected_level,
            depth == 0u, lower_len == 0u ? NULL : lower, lower_len,
            upper_len == 0u ? NULL : upper, upper_len, &node);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_TREE_DECODE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        if (depth == 0u) {
            if (total_entries != NULL) {
                *total_entries = node.subtree_items;
            }
            if (ordinal >= node.subtree_items) {
                return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                                    AFSPR_STAGE_TREE_TRAVERSAL,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
        }
        if (node.level == 0u) {
            struct afspr_tree_item item;

            if (remaining >= node.count ||
                afspr_tree_item_at(&node, (uint32_t)remaining, &item) !=
                    AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_TRAVERSAL,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            status = afspr_decode_directory_entry(
                &ident, &item, directory->object_id, name_buffer,
                name_capacity, entry);
            return afspr_report(
                diagnostic, status,
                status == AFSPR_OK ? AFSPR_STAGE_COMPLETE
                                   : AFSPR_STAGE_DIRECTORY_DECODE,
                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }

        {
            uint32_t child_index = 0u;
            uint64_t child_lba = node.leftmost_child;
            uint64_t child_items = node.leftmost_items;
            int chosen = remaining < child_items;

            if (!chosen) {
                remaining -= child_items;
                for (index = 0; index < node.count; ++index) {
                    struct afspr_tree_item item;

                    if (afspr_tree_item_at(&node, index, &item) != AFSPR_OK ||
                        afspr_tree_child(&ident, &item, &child_lba,
                                         &child_items) != AFSPR_OK) {
                        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                            AFSPR_STAGE_TREE_DECODE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    if (remaining < child_items) {
                        child_index = index + 1u;
                        chosen = 1;
                        break;
                    }
                    remaining -= child_items;
                }
            }
            if (!chosen) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_TRAVERSAL,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (child_index == 0u) {
                struct afspr_tree_item first;

                if (afspr_tree_item_at(&node, 0u, &first) != AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_TREE_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                memcpy(upper, first.key, first.key_len);
                upper_len = first.key_len;
            } else {
                struct afspr_tree_item selected_item;

                if (afspr_tree_item_at(&node, child_index - 1u,
                                       &selected_item) != AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_TREE_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                memcpy(lower, selected_item.key, selected_item.key_len);
                lower_len = selected_item.key_len;
                if (child_index < node.count) {
                    struct afspr_tree_item next;

                    if (afspr_tree_item_at(&node, child_index, &next) !=
                        AFSPR_OK) {
                        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                            AFSPR_STAGE_TREE_DECODE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    memcpy(upper, next.key, next.key_len);
                    upper_len = next.key_len;
                }
            }
            lba = child_lba;
        }
        expected_level = (int)node.level - 1;
    }
    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_TREE_TRAVERSAL,
                        AFSPR_NO_CHECKPOINT_SLOT, lba);
}

static int afspr_decode_extent(const struct afspr_ident *ident,
                               const struct afspr_probe_result *volume,
                               const uint8_t key[8], const uint8_t value[24],
                               struct afspr_extent *extent)
{
    uint64_t logical_end;
    uint64_t physical_end;
    uint32_t first_region;
    uint32_t last_region;

    extent->logical_start = afspr_get_be64(key);
    extent->physical_start = afspr_get_le64(value);
    extent->block_count = afspr_get_le64(value + 8u);
    extent->flags = afspr_get_le32(value + 16u);
    if (value[20] != 0u || value[21] != 0u || value[22] != 0u ||
        value[23] != 0u || extent->block_count == 0u ||
        (extent->flags & ~AFSPR_EXTENT_KNOWN_FLAGS) != 0u ||
        UINT64_MAX - extent->logical_start < extent->block_count ||
        UINT64_MAX - extent->physical_start < extent->block_count) {
        return AFSPR_ERR_CORRUPT;
    }
    logical_end = extent->logical_start + extent->block_count;
    physical_end = extent->physical_start + extent->block_count;
    if (logical_end <= extent->logical_start ||
        physical_end > ident->total_blocks ||
        !afspr_is_allocatable(ident, extent->physical_start) ||
        !afspr_is_allocatable(ident, physical_end - 1u)) {
        return AFSPR_ERR_CORRUPT;
    }
    first_region =
        (uint32_t)(extent->physical_start / ident->region_size);
    last_region = (uint32_t)((physical_end - 1u) / ident->region_size);
    if (first_region != last_region ||
        ((extent->flags & AFSPR_EXTENT_SHARED) != 0u &&
         ((volume->ro_compat_features & AFSP_RO_COMPAT_SHARED_EXTENTS) == 0u ||
          volume->shared_extent_root_block == 0u))) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

static int afspr_extent_for_block(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident,
    const struct afspr_probe_result *volume, const struct afspr_object *file,
    uint64_t logical_block, struct afspr_extent *extent, int *mapped,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_tree_spec spec;
    uint8_t search[8];
    uint8_t found_key[8];
    uint8_t value[AFSPR_EXTENT_VALUE_SIZE];
    size_t value_len;
    uint64_t leaf_lba;
    int found;
    int status;

    spec.kind = AFSPR_TREE_KIND_EXTENT_MAP;
    spec.owner = file->object_id;
    spec.max_generation = volume->generation;
    afspr_put_be64(search, logical_block);
    status = afspr_tree_lookup_fixed(
        ops, scratch, ident, file->data_root, &spec, search, 1, found_key,
        value, sizeof(value), &value_len, &found, &leaf_lba, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    *mapped = 0;
    if (!found) {
        return AFSPR_OK;
    }
    if (value_len != AFSPR_EXTENT_VALUE_SIZE ||
        afspr_decode_extent(ident, volume, found_key, value, extent) !=
            AFSPR_OK) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_EXTENT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, leaf_lba);
    }
    if (logical_block - extent->logical_start < extent->block_count) {
        *mapped = 1;
    }
    return AFSPR_OK;
}

int afspr_read_file(const struct afspr_block_ops *ops,
                    const struct afspr_scratch *scratch,
                    const struct afspr_probe_result *volume,
                    const struct afspr_object *file, uint64_t offset,
                    void *destination, size_t destination_size,
                    size_t *bytes_read,
                    struct afspr_diagnostic *diagnostic,
                    size_t diagnostic_size)
{
    struct afspr_ident ident;
    uint64_t count;
    uint64_t end;
    uint64_t logical_block;
    uint64_t final_block;
    uint8_t *output = (uint8_t *)destination;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_MIN_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (file == NULL || bytes_read == NULL ||
        (destination == NULL && destination_size != 0u)) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    *bytes_read = 0u;
    if (file->abi_version != AFSPR_ABI_VERSION) {
        return afspr_report(diagnostic, AFSPR_ERR_ABI,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (file->type != AFSPR_OBJECT_FILE) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FILE,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if ((file->flags & ~AFSPR_OBJECT_FLAG_EXTENT_TREE) != 0u ||
        file->data_blocks > UINT64_MAX / ops->block_size ||
        file->allocated_bytes != file->data_blocks * ops->block_size) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if ((file->flags & AFSPR_OBJECT_FLAG_EXTENT_TREE) != 0u) {
        if (!afspr_is_allocatable(&ident, file->data_root)) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_ARGUMENTS,
                                AFSPR_NO_CHECKPOINT_SLOT, file->data_root);
        }
    } else if (file->data_blocks == 0u) {
        if (file->data_root != 0u || file->size_bytes != 0u) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_ARGUMENTS,
                                AFSPR_NO_CHECKPOINT_SLOT, file->data_root);
        }
    } else if (file->data_blocks > AFSPR_MAX_DIRECT_BLOCKS ||
               !afspr_is_allocatable(&ident, file->data_root) ||
               file->size_bytes > file->allocated_bytes ||
               file->size_bytes <= file->allocated_bytes - ops->block_size) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, file->data_root);
    }
    if (destination_size == 0u || offset >= file->size_bytes) {
        return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    count = file->size_bytes - offset;
    if (count > destination_size) {
        count = destination_size;
    }
    end = offset + count;
    logical_block = offset / ops->block_size;
    final_block = (end - 1u) / ops->block_size;

    for (; logical_block <= final_block; ++logical_block) {
        struct afspr_extent extent;
        uint64_t physical = 0u;
        uint64_t block_start = logical_block * ops->block_size;
        uint64_t copy_start = offset > block_start ? offset : block_start;
        uint64_t block_end = block_start + ops->block_size;
        uint64_t copy_end = end < block_end ? end : block_end;
        size_t source_offset = (size_t)(copy_start - block_start);
        size_t target_offset = (size_t)(copy_start - offset);
        size_t copy_size = (size_t)(copy_end - copy_start);
        int mapped = 0;

        if ((file->flags & AFSPR_OBJECT_FLAG_EXTENT_TREE) != 0u) {
            status = afspr_extent_for_block(
                ops, scratch, &ident, volume, file, logical_block, &extent,
                &mapped, diagnostic);
            if (status != AFSPR_OK) {
                return status;
            }
            if (mapped && (extent.flags & AFSPR_EXTENT_UNWRITTEN) == 0u) {
                physical = extent.physical_start + logical_block -
                           extent.logical_start;
            } else {
                mapped = 0;
            }
        } else if (logical_block < file->data_blocks) {
            if (UINT64_MAX - file->data_root < logical_block) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_DATA_READ,
                                    AFSPR_NO_CHECKPOINT_SLOT,
                                    file->data_root);
            }
            physical = file->data_root + logical_block;
            mapped = 1;
        }
        if (mapped) {
            if (!afspr_is_allocatable(&ident, physical)) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_DATA_READ,
                                    AFSPR_NO_CHECKPOINT_SLOT, physical);
            }
            status = afspr_read_one(ops, physical,
                                    (uint8_t *)scratch->buffer);
            if (status != AFSPR_OK) {
                return afspr_report(diagnostic, status,
                                    AFSPR_STAGE_DATA_READ,
                                    AFSPR_NO_CHECKPOINT_SLOT, physical);
            }
        } else {
            memset(scratch->buffer, 0, ops->block_size);
        }
        memmove(output + target_offset,
                (const uint8_t *)scratch->buffer + source_offset, copy_size);
    }
    *bytes_read = (size_t)count;
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
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
    case AFSPR_ERR_NOT_FOUND:
        return "not found";
    case AFSPR_ERR_NOT_FILE:
        return "object is not a file";
    case AFSPR_ERR_NOT_DIRECTORY:
        return "object is not a directory";
    case AFSPR_ERR_BUFFER_TOO_SMALL:
        return "output buffer too small";
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
    case AFSPR_STAGE_TREE_READ:
        return "tree read";
    case AFSPR_STAGE_TREE_DECODE:
        return "tree decode";
    case AFSPR_STAGE_TREE_TRAVERSAL:
        return "tree traversal";
    case AFSPR_STAGE_OBJECT_READ:
        return "object read";
    case AFSPR_STAGE_OBJECT_DECODE:
        return "object decode";
    case AFSPR_STAGE_DIRECTORY_DECODE:
        return "directory decode";
    case AFSPR_STAGE_EXTENT_DECODE:
        return "extent decode";
    case AFSPR_STAGE_DATA_READ:
        return "file data read";
    default:
        return "unknown probe stage";
    }
}
