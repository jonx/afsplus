/* SPDX-License-Identifier: BSD-2-Clause */

#include "libafsplus_reader.h"
#include "afsplus_format.h"
#include "reader_internal.h"

#include <limits.h>
#include <string.h>

#define AFSPR_HEADER_SIZE 32u
#define AFSPR_CHECKSUM_OFFSET 28u
#define AFSPR_HEADER_VERSION 1u
#define AFSPR_BLOCK_TYPE_IDENT UINT32_C(0x49534641)
#define AFSPR_BLOCK_TYPE_CHECKPOINT UINT32_C(0x43534641)
#define AFSPR_BLOCK_TYPE_OBJECT UINT32_C(0x4f534641)
#define AFSPR_BLOCK_TYPE_SECURITY UINT32_C(0x58534641)
#define AFSPR_BLOCK_TYPE_ATTRIBUTES UINT32_C(0x41534641)
#define AFSPR_BLOCK_TYPE_RECLAIM_ROOT UINT32_C(0x48534641)
#define AFSPR_BLOCK_TYPE_RECLAIM_SEGMENT UINT32_C(0x53534641)
#define AFSPR_BLOCK_TYPE_RECLAIM_TABLE UINT32_C(0x4c534641)
#define AFSPR_RECLAIM_ENTRY_SIZE 20u
#define AFSPR_RECLAIM_REF_SIZE 12u
#define AFSPR_RECLAIM_ROOT_FIXED 64u
#define AFSPR_RECLAIM_SEALED_FIXED 8u
#define AFSPR_RECLAIM_ROOT_VERSION 1u
#define AFSPR_RECLAIM_SEGMENT_ENTRY_CAP 202u
#define AFSPR_RECLAIM_TABLE_REF_CAP 338u
#define AFSPR_BLOCK_TYPE_TREE UINT32_C(0x54534641)
#define AFSPR_BLOCK_TYPE_INTENT UINT32_C(0x4a534641)
#define AFSPR_BLOCK_TYPE_BITMAP UINT32_C(0x42534641)
#define AFSPR_BLOCK_TYPE_REGION_DESCRIPTOR UINT32_C(0x47534641)
#define AFSPR_CHECKSUM_CRC32C 1u
#define AFSPR_IDENT_LEGACY_PAYLOAD 137u
#define AFSPR_IDENT_FEATURE_PAYLOAD 161u
#define AFSPR_IDENT_CURRENT_PAYLOAD 165u
/* 96 fixed bytes, then the label field: length (1), reserved (7), 64 bytes. */
#define AFSPR_CHECKPOINT_LABEL_OFFSET AFSP_CHECKPOINT_LABEL_OFFSET
#define AFSPR_CHECKPOINT_PAYLOAD AFSP_CHECKPOINT_PAYLOAD_BYTES
#define AFSPR_CHECKPOINT_SNAPSHOT_PAYLOAD AFSP_CHECKPOINT_SNAPSHOT_PAYLOAD_BYTES
#define AFSPR_OBJECT_FIRST_DYNAMIC UINT64_C(16)
#define AFSPR_MIN_REGION_BLOCKS UINT32_C(16)
#define AFSPR_MAX_REGION_BLOCKS UINT32_C(262144)
#define AFSPR_BITMAP_PAGE_BLOCKS UINT32_C(32384)
#define AFSPR_BOOTSTRAP_BLOCKS UINT64_C(3)
#define AFSPR_DESCRIPTOR_SLOTS UINT64_C(3)
#define AFSPR_BITMAP_SLOTS UINT64_C(3)
#define AFSPR_SUPPORTED_INCOMPAT                                             \
    (AFSP_INCOMPAT_INTENT_LOG | AFSP_INCOMPAT_INTENT_LOG_DATA_UPDATES |     \
     AFSP_INCOMPAT_SECURITY_DESCRIPTORS)
#define AFSPR_TREE_FIXED_PAYLOAD 32u
#define AFSPR_TREE_ITEM_FIXED 8u
#define AFSPR_TREE_MAX_LEVEL 15u
#define AFSPR_TREE_MAX_KEY 1020u
#define AFSPR_TREE_KIND_OBJECT_MAP 1u
#define AFSPR_TREE_KIND_DIRECTORY 2u
#define AFSPR_TREE_KIND_EXTENT_MAP 3u
#define AFSPR_TREE_KIND_ALLOCATION_ROOT 4u
#define AFSPR_TREE_KIND_SNAPSHOT_REGISTRY 6u
#define AFSPR_TREE_KIND_SNAPSHOT_LIFETIMES 7u
#define AFSPR_ALLOCATION_VALUE_SIZE 16u
#define AFSPR_REGION_DESCRIPTOR_FIXED 16u
#define AFSPR_BITMAP_FIXED 16u
#define AFSPR_MAX_BITMAP_PAGES                                           \
    ((AFSPR_MAX_REGION_BLOCKS + AFSPR_BITMAP_PAGE_BLOCKS - 1u) /          \
     AFSPR_BITMAP_PAGE_BLOCKS)
#define AFSPR_OBJECT_PAYLOAD 96u
#define AFSPR_SECURITY_REF_SIZE 16u
/* Flags the reader carries and never interprets. */
#define AFSPR_ATTRIBUTE_REF_SIZE 16u
#define AFSPR_OBJECT_PRESERVED_FLAGS                                        \
    (AFSPR_OBJECT_FLAG_SECURITY_REF | AFSPR_OBJECT_FLAG_COMMENT |           \
     AFSPR_OBJECT_FLAG_ATTRIBUTES)
#define AFSPR_SECURITY_SEGMENT_FIXED 24u
#define AFSPR_MAX_DIRECT_BLOCKS UINT64_C(4096)
/* The extent item layout and its flag bits are the spec header's. */
#define AFSPR_EXTENT_VALUE_SIZE sizeof(struct afsp_extent_value_wire)
#define AFSPR_EXTENT_UNWRITTEN ((uint32_t)AFSP_EXTENT_FLAG_UNWRITTEN)
#define AFSPR_EXTENT_SHARED ((uint32_t)AFSP_EXTENT_FLAG_SHARED)
#define AFSPR_EXTENT_KNOWN_FLAGS (AFSPR_EXTENT_UNWRITTEN | AFSPR_EXTENT_SHARED)
#define AFSPR_LOG_FIXED_PAYLOAD 32u
#define AFSPR_LOG_LEGACY_OP_FIXED 48u
#define AFSPR_LOG_OP_FIXED 64u
#define AFSPR_LOG_EXTENT_WIRE 12u
#define AFSPR_LOG_PREVIOUS_VERSION 2u
#define AFSPR_LOG_CURRENT_VERSION 3u
#define AFSPR_LOG_MAX_EXTENTS 16u
#define AFSPR_LOG_MAX_OPS 64u
#define AFSPR_DIRECTORY_VALUE_MAX (16u + AFSP_NAME_MAX_UTF8_BYTES)
#define AFSPR_NAMESPACE_LOWER_OFFSET AFSP_DEFAULT_BLOCK_SIZE
#define AFSPR_NAMESPACE_UPPER_OFFSET                                      \
    (AFSPR_NAMESPACE_LOWER_OFFSET + AFSPR_TREE_MAX_KEY)
#define AFSPR_NAMESPACE_KEY_OFFSET                                        \
    (AFSPR_NAMESPACE_UPPER_OFFSET + AFSPR_TREE_MAX_KEY)
#define AFSPR_NAMESPACE_VALUE_OFFSET                                      \
    (AFSPR_NAMESPACE_KEY_OFFSET + AFSPR_TREE_MAX_KEY)
#define AFSPR_NAMESPACE_NAME_OFFSET                                       \
    (AFSPR_NAMESPACE_VALUE_OFFSET + AFSPR_DIRECTORY_VALUE_MAX)

#if AFSPR_TREE_MAX_KEY != AFSPR_COMPARISON_KEY_CAPACITY
#error "public and internal comparison-key bounds differ"
#endif
#if AFSPR_NAMESPACE_NAME_OFFSET + AFSP_NAME_MAX_UTF8_BYTES > \
    AFSPR_INTENT_SCRATCH_SIZE
#error "intent scratch cannot hold the namespace workspace"
#endif

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
    uint8_t label_len;
    /* The slot is in the snapshot-bearing form. The reader implements no
     * snapshots; selection refuses such a slot instead of stepping past it. */
    uint8_t has_snapshot_roots;
    char label[AFSPR_LABEL_CAPACITY];
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

struct afspr_log_record {
    const uint8_t *payload;
    size_t payload_len;
    size_t operation_fixed;
    uint16_t operation_count;
    uint16_t version;
    uint32_t sequence;
    uint64_t base_generation;
};

struct afspr_log_operation {
    uint8_t type;
    uint8_t replace;
    uint16_t source_len;
    uint16_t target_len;
    uint16_t extent_count;
    uint64_t first;
    uint64_t second;
    uint64_t third;
    uint64_t fourth;
    uint32_t content_crc;
    struct afspr_timespec timestamp;
    const uint8_t *extents;
    const uint8_t *source_name;
    const uint8_t *target_name;
};

struct afspr_bitmap_binding_view {
    uint8_t slot;
    uint32_t free_blocks;
    uint64_t generation;
};

struct afspr_allocation_region_view {
    uint32_t valid_blocks;
    uint32_t free_blocks;
    uint32_t page_count;
    uint64_t descriptor_generation;
    struct afspr_bitmap_binding_view pages[AFSPR_MAX_BITMAP_PAGES];
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
    size_t tail;

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
    /* A block ends where its payload ends (ADR-112): every encoder seals a
     * zeroed block, and a byte past the payload belongs to no field. */
    for (tail = AFSPR_HEADER_SIZE + (size_t)header->payload_len;
         tail < block_size; ++tail) {
        if (block[tail] != 0u) return AFSPR_ERR_CORRUPT;
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
    if (spec->kind == AFSPR_TREE_KIND_ALLOCATION_ROOT) {
        return item->key_len == 4u &&
                       item->value_len == AFSPR_ALLOCATION_VALUE_SIZE
                   ? AFSPR_OK
                   : AFSPR_ERR_CORRUPT;
    }
    return AFSPR_ERR_UNSUPPORTED;
}

static int afspr_decode_tree_node(
    const uint8_t *block, size_t block_size, const struct afspr_ident *ident,
    const struct afspr_tree_spec *spec, int expected_level,
    uint64_t expected_subtree_items, int is_root, const uint8_t *lower,
    size_t lower_len, const uint8_t *upper, size_t upper_len,
    struct afspr_tree_node *node)
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
        (expected_subtree_items != 0u &&
         node->subtree_items != expected_subtree_items) ||
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
    /* The block belongs to the volume and has no flag namespace (ADR-114). */
    if (header.flags != 0u || header.owner != 0u) return AFSPR_ERR_CORRUPT;
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
    /* Exact for its version: a byte past the last field belongs to no field,
     * and a later layout arrives as a new version, not as a longer payload of
     * this one (ADR-114). */
    if ((size_t)header.payload_len != minimum_payload) {
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
        !afspr_valid_utf8(p + 73, ident->label_len) ||
        memchr(p + 73, 0, ident->label_len) != NULL) {
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

int afspr_decode_checkpoint_block(const void *input, size_t block_size,
                                  const uint8_t *volume_uuid,
                                  struct afspr_checkpoint_view *view_out)
{
    const uint8_t *block = (const uint8_t *)input;
    const uint8_t *p;
    const uint8_t *field;
    struct afspr_header header;
    struct afspr_checkpoint_view view;
    size_t i;
    int status;

    if (input == NULL || volume_uuid == NULL || view_out == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size,
                                 AFSPR_BLOCK_TYPE_CHECKPOINT, &header);
    if (status != AFSPR_OK) return status;
    if (header.flags != 0u || header.owner != 0u ||
        (header.payload_len != AFSPR_CHECKPOINT_PAYLOAD &&
         header.payload_len != AFSPR_CHECKPOINT_SNAPSHOT_PAYLOAD)) {
        return AFSPR_ERR_CORRUPT;
    }
    /* Nothing follows the payload: the header verification refused it
     * (ADR-112), so snapshot roots left behind a short payload length do not
     * read as a checkpoint without them (ADR-111). */
    p = block + AFSPR_HEADER_SIZE;
    if (memcmp(p, volume_uuid, 16u) != 0) return AFSPR_ERR_CORRUPT;
    memset(&view, 0, sizeof(view));
    view.generation = afspr_get_le64(p + 16);
    if (view.generation == 0u || view.generation != header.generation ||
        afspr_get_le64(p + 24) != AFSP_OBJECT_ROOT ||
        afspr_get_le64(p + 80) != 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    view.object_map_block = afspr_get_le64(p + 32);
    view.allocation_root_block = afspr_get_le64(p + 40);
    view.reclaim_root_block = afspr_get_le64(p + 48);
    view.next_object_id = afspr_get_le64(p + 56);
    view.committed_tx_id = afspr_get_le64(p + 64);
    view.free_blocks_total = afspr_get_le64(p + 72);
    view.shared_extent_root_block = afspr_get_le64(p + 88);
    /* The label field is canonical: length, zero reserved bytes, NUL-free
     * UTF-8 and zero padding. */
    field = p + AFSPR_CHECKPOINT_LABEL_OFFSET;
    view.label_len = field[0];
    if (view.label_len > 64u ||
        !afspr_valid_utf8(field + 8u, view.label_len) ||
        memchr(field + 8u, 0, view.label_len) != NULL) {
        return AFSPR_ERR_CORRUPT;
    }
    for (i = 1u; i < 8u; ++i) {
        if (field[i] != 0u) return AFSPR_ERR_CORRUPT;
    }
    for (i = view.label_len; i < 64u; ++i) {
        if (field[8u + i] != 0u) return AFSPR_ERR_CORRUPT;
    }
    memcpy(view.label, field + 8u, view.label_len);
    /* The snapshot roots of ADR-073 follow the label: both nonzero and
     * distinct. */
    if (header.payload_len == AFSPR_CHECKPOINT_SNAPSHOT_PAYLOAD) {
        view.has_snapshot_roots = 1u;
        view.snapshot_registry_block =
            afspr_get_le64(p + AFSPR_CHECKPOINT_PAYLOAD);
        view.snapshot_lifetimes_block =
            afspr_get_le64(p + AFSPR_CHECKPOINT_PAYLOAD + 8u);
        if (view.snapshot_registry_block == 0u ||
            view.snapshot_lifetimes_block == 0u ||
            view.snapshot_registry_block == view.snapshot_lifetimes_block) {
            return AFSPR_ERR_CORRUPT;
        }
    }
    *view_out = view;
    return AFSPR_OK;
}

/* The volume path: format-level decoding plus what the geometry decides. A
 * slot in the snapshot-bearing form stays a candidate, so that selection can
 * refuse it as the core does: a valid newest checkpoint whose form disagrees
 * with the volume's features is never stepped past in favour of an older
 * one. */
static int afspr_decode_checkpoint(const uint8_t *block, size_t block_size,
                                   const struct afspr_ident *ident,
                                   struct afspr_checkpoint *checkpoint)
{
    struct afspr_checkpoint_view view;
    int status = afspr_decode_checkpoint_block(block, block_size, ident->uuid,
                                               &view);

    if (status != AFSPR_OK) return status;
    checkpoint->has_snapshot_roots = (uint8_t)view.has_snapshot_roots;
    if (view.has_snapshot_roots != 0u &&
        (!afspr_is_allocatable(ident, view.snapshot_registry_block) ||
         !afspr_is_allocatable(ident, view.snapshot_lifetimes_block))) {
        return AFSPR_ERR_CORRUPT;
    }
    checkpoint->generation = view.generation;
    checkpoint->object_map_block = view.object_map_block;
    checkpoint->allocation_root_block = view.allocation_root_block;
    checkpoint->reclaim_root_block = view.reclaim_root_block;
    checkpoint->next_object_id = view.next_object_id;
    checkpoint->committed_tx_id = view.committed_tx_id;
    checkpoint->free_blocks_total = view.free_blocks_total;
    checkpoint->shared_extent_root_block = view.shared_extent_root_block;
    checkpoint->label_len = view.label_len;
    memset(checkpoint->label, 0, sizeof(checkpoint->label));
    memcpy(checkpoint->label, view.label, view.label_len);
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
    uint64_t expected_subtree_items = 0u;
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
            expected_level, expected_subtree_items, depth == 0u,
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
            expected_subtree_items = node.leftmost_items;
        } else {
            struct afspr_tree_item selected_item;

            status = afspr_tree_item_at(&node, separator - 1u,
                                        &selected_item);
            if (status != AFSPR_OK || selected_item.key_len != 8u ||
                afspr_tree_child(ident, &selected_item, &lba,
                                 &expected_subtree_items) != AFSPR_OK) {
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

/* Exact lookup for variable-length directory comparison keys. */
static int afspr_tree_lookup_variable(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident, uint64_t root_lba,
    const struct afspr_tree_spec *spec, const uint8_t *search,
    size_t search_len, uint8_t *found_value, size_t value_capacity,
    size_t *found_value_len, int *found, uint64_t *leaf_lba,
    struct afspr_diagnostic *diagnostic)
{
    uint64_t visited[AFSPR_TREE_MAX_LEVEL + 1u];
    uint8_t *block = (uint8_t *)scratch->buffer;
    uint8_t *lower = block + AFSPR_NAMESPACE_LOWER_OFFSET;
    uint8_t *upper = block + AFSPR_NAMESPACE_UPPER_OFFSET;
    size_t lower_len = 0u;
    size_t upper_len = 0u;
    uint64_t lba = root_lba;
    uint64_t expected_subtree_items = 0u;
    int expected_level = -1;
    unsigned depth;

    *found = 0;
    *found_value_len = 0u;
    if (search == NULL || search_len == 0u ||
        search_len > AFSPR_TREE_MAX_KEY ||
        !afspr_is_allocatable(ident, root_lba)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_TRAVERSAL,
                            AFSPR_NO_CHECKPOINT_SLOT, root_lba);
    }
    for (depth = 0u; depth <= AFSPR_TREE_MAX_LEVEL; ++depth) {
        struct afspr_tree_node node;
        uint32_t index;
        uint32_t separator = 0u;
        int status;

        for (index = 0u; index < depth; ++index) {
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
            block, ops->block_size, ident, spec, expected_level,
            expected_subtree_items, depth == 0u,
            lower_len == 0u ? NULL : lower, lower_len,
            upper_len == 0u ? NULL : upper, upper_len, &node);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_TREE_DECODE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        if (node.level == 0u) {
            struct afspr_tree_item item;

            *leaf_lba = lba;
            for (index = 0u; index < node.count; ++index) {
                int compared;

                status = afspr_tree_item_at(&node, index, &item);
                if (status != AFSPR_OK) {
                    return afspr_report(diagnostic, status,
                                        AFSPR_STAGE_TREE_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                compared = afspr_bytes_compare(item.key, item.key_len,
                                               search, search_len);
                if (compared == 0) {
                    if (item.value_len > value_capacity) {
                        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                            AFSPR_STAGE_TREE_DECODE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    memcpy(found_value, item.value, item.value_len);
                    *found_value_len = item.value_len;
                    *found = 1;
                    break;
                }
                if (compared > 0) {
                    break;
                }
            }
            return AFSPR_OK;
        }

        for (index = 0u; index < node.count; ++index) {
            struct afspr_tree_item item;

            status = afspr_tree_item_at(&node, index, &item);
            if (status != AFSPR_OK) {
                return afspr_report(diagnostic, status,
                                    AFSPR_STAGE_TREE_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (afspr_bytes_compare(item.key, item.key_len, search,
                                    search_len) <= 0) {
                separator = index + 1u;
            } else {
                break;
            }
        }
        if (separator == 0u) {
            struct afspr_tree_item first;

            status = afspr_tree_item_at(&node, 0u, &first);
            if (status != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            memcpy(upper, first.key, first.key_len);
            upper_len = first.key_len;
            lba = node.leftmost_child;
            expected_subtree_items = node.leftmost_items;
        } else {
            struct afspr_tree_item selected_item;

            status = afspr_tree_item_at(&node, separator - 1u,
                                        &selected_item);
            if (status != AFSPR_OK ||
                afspr_tree_child(ident, &selected_item, &lba,
                                 &expected_subtree_items) != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_TREE_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            memcpy(lower, selected_item.key, selected_item.key_len);
            lower_len = selected_item.key_len;
            if (separator < node.count) {
                struct afspr_tree_item next;

                status = afspr_tree_item_at(&node, separator, &next);
                if (status != AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_TREE_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                memcpy(upper, next.key, next.key_len);
                upper_len = next.key_len;
            }
        }
        if (!afspr_is_allocatable(ident, lba)) {
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
    struct afspr_ident ident = {0};
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
    /* The payload form belongs to the persistent-snapshots feature, which
     * this reader does not implement and its identification check refuses:
     * a selected checkpoint in that form disagrees with the volume. */
    if (candidates[selected].has_snapshot_roots != 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_CHECKPOINT_SELECTION,
                            (int32_t)selected,
                            ident.checkpoint_slots[selected]);
    }
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
    /* The current label is committed state; the identification block keeps
     * the one given at format time. */
    memcpy(result->label, candidates[selected].label, sizeof(result->label));
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
           AFSPR_CAP_DIRECTORY_ORDINAL | AFSPR_CAP_FILE_READ |
           AFSPR_CAP_INTENT_LOG_SCAN | AFSPR_CAP_INTENT_FILE_READ |
           AFSPR_CAP_INTENT_NAMESPACE;
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

/* Exact admission shared by every object decoder: the fixed payload length
 * the security-reference flag defines, a well-formed reference and a zero
 * tail. A rewrite re-encodes decoded fields into a zeroed block, so a byte
 * admitted without a field would be lost. */
static int afspr_object_shape_full(
    const uint8_t *block, size_t block_size, const struct afspr_header *header,
    size_t *fixed, struct afspr_security_reference *reference,
    struct afspr_attribute_reference *attributes, const uint8_t **comment,
    size_t *comment_size)
{
    const uint8_t *p = block + AFSPR_HEADER_SIZE;
    size_t payload = (size_t)header->payload_len;
    size_t security_end = AFSPR_OBJECT_PAYLOAD;
    size_t attributes_at;
    size_t capacity;
    uint16_t flags;

    memset(reference, 0, sizeof(*reference));
    memset(attributes, 0, sizeof(*attributes));
    *fixed = AFSPR_OBJECT_PAYLOAD;
    *comment = NULL;
    *comment_size = 0u;
    if (header->flags != 0u || payload < AFSPR_OBJECT_PAYLOAD) {
        return AFSPR_ERR_CORRUPT;
    }
    flags = afspr_get_le16(p + 10u);
    /* Object flags are a validated namespace: an unassigned bit refuses the
     * record in every decoder, the standalone ones included. */
    if ((flags & ~(AFSPR_OBJECT_FLAG_EXTENT_TREE |
                   AFSPR_OBJECT_FLAG_DATA_IN_PLACE |
                   AFSPR_OBJECT_PRESERVED_FLAGS)) != 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    if ((flags & AFSPR_OBJECT_FLAG_SECURITY_REF) != 0u) {
        security_end += AFSPR_SECURITY_REF_SIZE;
    }
    /* The attribute reference follows the security reference. From here
     * on security_end names the end of both. */
    attributes_at = security_end;
    if ((flags & AFSPR_OBJECT_FLAG_ATTRIBUTES) != 0u) {
        security_end += AFSPR_ATTRIBUTE_REF_SIZE;
    }
    if (payload < security_end) {
        return AFSPR_ERR_CORRUPT;
    }
    *fixed = security_end;
    /* The comment follows the reference: one length byte, 1 to 255, then
     * that many bytes of UTF-8 without NUL. */
    if ((flags & AFSPR_OBJECT_FLAG_COMMENT) != 0u) {
        size_t length;
        if (payload <= security_end) return AFSPR_ERR_CORRUPT;
        length = p[security_end];
        if (length == 0u || payload - security_end - 1u < length ||
            !afspr_valid_utf8(p + security_end + 1u, length) ||
            memchr(p + security_end + 1u, 0, length) != NULL) {
            return AFSPR_ERR_CORRUPT;
        }
        *comment = p + security_end + 1u;
        *comment_size = length;
        *fixed = security_end + 1u + length;
    }
    if (p[8] != AFSPR_OBJECT_SYMLINK && payload != *fixed) {
        return AFSPR_ERR_CORRUPT;
    }
    if (security_end == AFSPR_OBJECT_PAYLOAD) {
        return AFSPR_OK;
    }
    if (block_size <= AFSPR_HEADER_SIZE + AFSPR_SECURITY_SEGMENT_FIXED) {
        return AFSPR_ERR_CORRUPT;
    }
    capacity = block_size - AFSPR_HEADER_SIZE - AFSPR_SECURITY_SEGMENT_FIXED;
    if ((flags & AFSPR_OBJECT_FLAG_ATTRIBUTES) != 0u) {
        const uint8_t *a = p + attributes_at;
        attributes->present = 1u;
        attributes->first_block = afspr_get_le64(a);
        attributes->total_len = afspr_get_le32(a + 8u);
        attributes->segment_count = afspr_get_le16(a + 12u);
        if (attributes->first_block == 0u || attributes->total_len == 0u ||
            attributes->total_len > AFSPR_MAX_ATTRIBUTE_SET_BYTES ||
            ((size_t)attributes->total_len + capacity - 1u) / capacity !=
                (size_t)attributes->segment_count ||
            afspr_get_le16(a + 14u) != 0u) {
            return AFSPR_ERR_CORRUPT;
        }
    }
    if ((flags & AFSPR_OBJECT_FLAG_SECURITY_REF) == 0u) {
        return AFSPR_OK;
    }
    reference->present = 1u;
    reference->first_block = afspr_get_le64(p + 96u);
    reference->total_len = afspr_get_le32(p + 104u);
    reference->segment_count = afspr_get_le16(p + 108u);
    reference->flags = afspr_get_le16(p + 110u);
    if (reference->first_block == 0u || reference->total_len == 0u ||
        reference->total_len > AFSPR_MAX_SECURITY_DESCRIPTOR_BYTES ||
        ((size_t)reference->total_len + capacity - 1u) / capacity !=
            (size_t)reference->segment_count ||
        (reference->flags & ~AFSPR_SECURITY_REF_PROJECTION_DIVERGED) != 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

static int afspr_object_shape(const uint8_t *block, size_t block_size,
                              const struct afspr_header *header,
                              size_t *fixed,
                              struct afspr_security_reference *reference,
                              const uint8_t **comment, size_t *comment_size)
{
    struct afspr_attribute_reference attributes;
    return afspr_object_shape_full(block, block_size, header, fixed, reference,
                                   &attributes, comment, comment_size);
}

int afspr_decode_symlink_record(const void *input, size_t block_size,
                               struct afspr_object *object,
                               const uint8_t **target, size_t *target_size,
                               uint64_t *generation)
{
    const uint8_t *block = (const uint8_t *)input;
    const uint8_t *p;
    size_t length;
    size_t fixed;
    const uint8_t *comment;
    size_t comment_size;
    struct afspr_header header;
    struct afspr_object decoded;
    struct afspr_security_reference reference;
    int status;
    if (input == NULL || object == NULL || target == NULL ||
        target_size == NULL || generation == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size, AFSPR_BLOCK_TYPE_OBJECT, &header);
    if (status != AFSPR_OK) return status;
    if (afspr_object_shape(block, block_size, &header, &fixed, &reference,
                           &comment, &comment_size) !=
            AFSPR_OK ||
        (size_t)header.payload_len <= fixed)
        return AFSPR_ERR_CORRUPT;
    p = block + AFSPR_HEADER_SIZE;
    length = (size_t)header.payload_len - fixed;
    memset(&decoded, 0, sizeof(decoded));
    decoded.abi_version = AFSPR_ABI_VERSION;
    decoded.object_id = afspr_get_le64(p);
    decoded.type = p[8];
    decoded.flags = afspr_get_le16(p + 10u);
    decoded.link_count = afspr_get_le32(p + 12u);
    decoded.size_bytes = afspr_get_le64(p + 16u);
    decoded.allocated_bytes = afspr_get_le64(p + 24u);
    decoded.protection = afspr_get_le32(p + 68u);
    decoded.content_generation = afspr_get_le64(p + 72u);
    decoded.data_root = afspr_get_le64(p + 80u);
    decoded.data_blocks = afspr_get_le64(p + 88u);
    if (decoded.object_id == 0u || decoded.object_id != header.owner ||
        decoded.type != AFSPR_OBJECT_SYMLINK || p[9] != 0u ||
        (decoded.flags & ~AFSPR_OBJECT_PRESERVED_FLAGS) != 0u ||
        decoded.link_count == 0u ||
        decoded.size_bytes != length || decoded.allocated_bytes != 0u ||
        decoded.data_root != 0u || decoded.data_blocks != 0u ||
        afspr_decode_timespec(p + 32u, &decoded.created) != AFSPR_OK ||
        afspr_decode_timespec(p + 44u, &decoded.modified) != AFSPR_OK ||
        afspr_decode_timespec(p + 56u, &decoded.changed) != AFSPR_OK ||
        !afspr_valid_utf8(p + fixed, length) ||
        memchr(p + fixed, 0, length) != NULL)
        return AFSPR_ERR_CORRUPT;
    *object = decoded;
    *target = p + fixed;
    *target_size = length;
    *generation = header.generation;
    return AFSPR_OK;
}

int afspr_decode_security_reference(const void *input, size_t block_size,
                                    struct afspr_security_reference *reference)
{
    const uint8_t *block = (const uint8_t *)input;
    struct afspr_header header;
    struct afspr_security_reference decoded;
    size_t fixed;
    const uint8_t *comment;
    size_t comment_size;
    int status;

    if (input == NULL || reference == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size, AFSPR_BLOCK_TYPE_OBJECT,
                                 &header);
    if (status != AFSPR_OK) return status;
    if (afspr_object_shape(block, block_size, &header, &fixed, &decoded,
                           &comment, &comment_size) !=
            AFSPR_OK ||
        afspr_get_le64(block + AFSPR_HEADER_SIZE) != header.owner ||
        header.owner == 0u || block[AFSPR_HEADER_SIZE + 9u] != 0u ||
        block[AFSPR_HEADER_SIZE + 8u] < AFSPR_OBJECT_FILE ||
        block[AFSPR_HEADER_SIZE + 8u] > AFSPR_OBJECT_SYMLINK) {
        return AFSPR_ERR_CORRUPT;
    }
    *reference = decoded;
    return AFSPR_OK;
}

int afspr_decode_object_comment(const void *input, size_t block_size,
                                const uint8_t **comment_out,
                                size_t *comment_size_out)
{
    const uint8_t *block = (const uint8_t *)input;
    struct afspr_header header;
    struct afspr_security_reference reference;
    size_t fixed;
    const uint8_t *comment;
    size_t comment_size;
    int status;

    if (input == NULL || comment_out == NULL || comment_size_out == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size, AFSPR_BLOCK_TYPE_OBJECT,
                                 &header);
    if (status != AFSPR_OK) return status;
    if (afspr_object_shape(block, block_size, &header, &fixed, &reference,
                           &comment, &comment_size) != AFSPR_OK ||
        afspr_get_le64(block + AFSPR_HEADER_SIZE) != header.owner ||
        header.owner == 0u || block[AFSPR_HEADER_SIZE + 9u] != 0u ||
        block[AFSPR_HEADER_SIZE + 8u] < AFSPR_OBJECT_FILE ||
        block[AFSPR_HEADER_SIZE + 8u] > AFSPR_OBJECT_SYMLINK) {
        return AFSPR_ERR_CORRUPT;
    }
    *comment_out = comment;
    *comment_size_out = comment_size;
    return AFSPR_OK;
}

static int afspr_decode_chain_segment(const void *input, size_t block_size,
                                      uint32_t block_type, uint32_t max_bytes,
                                      struct afspr_security_segment *segment,
                                      const uint8_t **bytes,
                                      size_t *bytes_size, uint64_t *generation)
{
    const uint8_t *block = (const uint8_t *)input;
    const uint8_t *p;
    struct afspr_header header;
    struct afspr_security_segment decoded;
    size_t capacity, length, expected, count;
    int last;
    int status;

    if (input == NULL || segment == NULL || bytes == NULL ||
        bytes_size == NULL || generation == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size, block_type, &header);
    if (status != AFSPR_OK) return status;
    if (header.flags != 0u ||
        header.payload_len < AFSPR_SECURITY_SEGMENT_FIXED ||
        block_size <= AFSPR_HEADER_SIZE + AFSPR_SECURITY_SEGMENT_FIXED) {
        return AFSPR_ERR_CORRUPT;
    }
    p = block + AFSPR_HEADER_SIZE;
    capacity = block_size - AFSPR_HEADER_SIZE - AFSPR_SECURITY_SEGMENT_FIXED;
    length = (size_t)header.payload_len - AFSPR_SECURITY_SEGMENT_FIXED;
    memset(&decoded, 0, sizeof(decoded));
    decoded.object_id = header.owner;
    decoded.format = afspr_get_le32(p);
    decoded.version = afspr_get_le16(p + 4u);
    decoded.total_len = afspr_get_le32(p + 8u);
    decoded.index = afspr_get_le16(p + 12u);
    decoded.count = afspr_get_le16(p + 14u);
    decoded.next = afspr_get_le64(p + 16u);
    if (afspr_get_le16(p + 6u) != 0u || decoded.object_id == 0u ||
        decoded.format == 0u || decoded.total_len == 0u ||
        decoded.total_len > max_bytes) {
        return AFSPR_ERR_CORRUPT;
    }
    count = ((size_t)decoded.total_len + capacity - 1u) / capacity;
    if ((size_t)decoded.count != count || (size_t)decoded.index >= count) {
        return AFSPR_ERR_CORRUPT;
    }
    last = (size_t)decoded.index + 1u == count;
    expected = last ? (size_t)decoded.total_len - capacity * (count - 1u)
                    : capacity;
    if (last != (decoded.next == 0u) || length != expected) {
        return AFSPR_ERR_CORRUPT;
    }
    *segment = decoded;
    *bytes = p + AFSPR_SECURITY_SEGMENT_FIXED;
    *bytes_size = length;
    *generation = header.generation;
    return AFSPR_OK;
}

int afspr_decode_security_segment(const void *input, size_t block_size,
                                  struct afspr_security_segment *segment,
                                  const uint8_t **bytes, size_t *bytes_size,
                                  uint64_t *generation)
{
    return afspr_decode_chain_segment(input, block_size,
                                      AFSPR_BLOCK_TYPE_SECURITY,
                                      AFSPR_MAX_SECURITY_DESCRIPTOR_BYTES,
                                      segment, bytes, bytes_size, generation);
}

int afspr_decode_attribute_segment(const void *input, size_t block_size,
                                   struct afspr_security_segment *segment,
                                   const uint8_t **bytes, size_t *bytes_size,
                                   uint64_t *generation)
{
    return afspr_decode_chain_segment(input, block_size,
                                      AFSPR_BLOCK_TYPE_ATTRIBUTES,
                                      AFSPR_MAX_ATTRIBUTE_SET_BYTES, segment,
                                      bytes, bytes_size, generation);
}

int afspr_decode_attribute_reference(
    const void *input, size_t block_size,
    struct afspr_attribute_reference *reference)
{
    const uint8_t *block = (const uint8_t *)input;
    struct afspr_header header;
    struct afspr_security_reference security;
    struct afspr_attribute_reference decoded;
    size_t fixed;
    const uint8_t *comment;
    size_t comment_size;
    int status;

    if (input == NULL || reference == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size, AFSPR_BLOCK_TYPE_OBJECT,
                                 &header);
    if (status != AFSPR_OK) return status;
    if (afspr_object_shape_full(block, block_size, &header, &fixed, &security,
                                &decoded, &comment, &comment_size) !=
            AFSPR_OK ||
        afspr_get_le64(block + AFSPR_HEADER_SIZE) != header.owner ||
        header.owner == 0u || block[AFSPR_HEADER_SIZE + 9u] != 0u ||
        block[AFSPR_HEADER_SIZE + 8u] < AFSPR_OBJECT_FILE ||
        block[AFSPR_HEADER_SIZE + 8u] > AFSPR_OBJECT_SYMLINK) {
        return AFSPR_ERR_CORRUPT;
    }
    *reference = decoded;
    return AFSPR_OK;
}

/* Namespace prefixes of attribute names (ADR-108). */
static int afspr_attribute_name_ok(const uint8_t *name, size_t size)
{
    static const struct {
        const char *text;
        size_t length;
    } prefixes[] = {{"user.", 5u}, {"system.", 7u}, {"security.", 9u},
                    {"aros.", 5u}};
    size_t i;

    if (size == 0u || size > 255u || !afspr_valid_utf8(name, size) ||
        memchr(name, 0, size) != NULL) {
        return 0;
    }
    for (i = 0u; i < sizeof(prefixes) / sizeof(prefixes[0]); ++i) {
        if (size > prefixes[i].length &&
            memcmp(name, prefixes[i].text, prefixes[i].length) == 0) {
            return 1;
        }
    }
    return 0;
}

/* Entry at offset at, bounds-checked; next receives the following offset. */
static int afspr_attribute_entry(const uint8_t *set, size_t size, size_t at,
                                 struct afspr_attribute *entry, size_t *next)
{
    size_t name_size, value_size;

    if (at > size || size - at < 4u) return AFSPR_ERR_CORRUPT;
    name_size = set[at];
    value_size = afspr_get_le16(set + at + 2u);
    if (name_size == 0u || set[at + 1u] != 0u ||
        size - at - 4u < name_size + value_size) {
        return AFSPR_ERR_CORRUPT;
    }
    entry->name = set + at + 4u;
    entry->value = entry->name + name_size;
    entry->name_size = (uint16_t)name_size;
    entry->value_size = (uint16_t)value_size;
    *next = at + 4u + name_size + value_size;
    return AFSPR_OK;
}

int afspr_validate_attribute_set(const void *input, size_t set_size,
                                 uint32_t *count_out)
{
    const uint8_t *set = (const uint8_t *)input;
    struct afspr_attribute previous, entry;
    size_t at = 4u;
    uint32_t count, i;

    if (input == NULL || count_out == NULL) return AFSPR_ERR_INVALID_ARGUMENT;
    if (set_size < 4u || set_size > AFSPR_MAX_ATTRIBUTE_SET_BYTES) {
        return AFSPR_ERR_CORRUPT;
    }
    count = afspr_get_le16(set);
    if (count == 0u || afspr_get_le16(set + 2u) != 0u) return AFSPR_ERR_CORRUPT;
    memset(&previous, 0, sizeof(previous));
    for (i = 0u; i < count; ++i) {
        if (afspr_attribute_entry(set, set_size, at, &entry, &at) != AFSPR_OK ||
            !afspr_attribute_name_ok(entry.name, entry.name_size)) {
            return AFSPR_ERR_CORRUPT;
        }
        if (i != 0u) {
            /* Strictly ascending by bytes; a proper prefix sorts first. */
            size_t common = previous.name_size < entry.name_size
                                ? previous.name_size
                                : entry.name_size;
            int order = memcmp(previous.name, entry.name, common);
            if (order > 0 ||
                (order == 0 && previous.name_size >= entry.name_size)) {
                return AFSPR_ERR_CORRUPT;
            }
        }
        previous = entry;
    }
    if (at != set_size) return AFSPR_ERR_CORRUPT;
    *count_out = count;
    return AFSPR_OK;
}

int afspr_attribute_set_next(const void *input, size_t set_size,
                             size_t *cursor, struct afspr_attribute *attribute)
{
    const uint8_t *set = (const uint8_t *)input;
    struct afspr_attribute entry;
    size_t at, next;

    if (input == NULL || cursor == NULL || attribute == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    if (set_size < 4u) return AFSPR_ERR_CORRUPT;
    at = *cursor == 0u ? 4u : *cursor;
    if (at == set_size) return AFSPR_ERR_NOT_FOUND;
    if (afspr_attribute_entry(set, set_size, at, &entry, &next) != AFSPR_OK) {
        return AFSPR_ERR_CORRUPT;
    }
    *attribute = entry;
    *cursor = next;
    return AFSPR_OK;
}

static int afspr_decode_object(const uint8_t *block, size_t block_size,
                               const struct afspr_ident *ident,
                               uint64_t max_generation,
                               uint64_t expected_object_id,
                               struct afspr_object *object)
{
    struct afspr_header header;
    struct afspr_security_reference reference;
    const uint8_t *p;
    size_t fixed;
    const uint8_t *comment;
    size_t comment_size;
    uint64_t expected_allocated;
    uint64_t data_end;
    uint64_t lba;
    int status = afspr_verify_header(block, block_size,
                                     AFSPR_BLOCK_TYPE_OBJECT, &header);

    if (status != AFSPR_OK) {
        return status;
    }
    if (header.owner != expected_object_id ||
        header.generation == 0u || header.generation > max_generation ||
        afspr_object_shape(block, block_size, &header, &fixed, &reference,
                           &comment, &comment_size) !=
            AFSPR_OK ||
        /* Where a well-formed reference points is chain state, not record
         * admission: the object stays reachable and deletable. */
        (reference.present != 0u &&
         (ident->incompat_features & AFSP_INCOMPAT_SECURITY_DESCRIPTORS) ==
             0u)) {
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
        (object->flags & ~(AFSPR_OBJECT_FLAG_EXTENT_TREE |
                           AFSPR_OBJECT_FLAG_DATA_IN_PLACE |
                           AFSPR_OBJECT_PRESERVED_FLAGS)) != 0u ||
        ((object->flags & AFSPR_OBJECT_FLAG_DATA_IN_PLACE) != 0u &&
         object->type != AFSPR_OBJECT_FILE) ||
        ((object->flags & AFSPR_OBJECT_FLAG_DATA_IN_PLACE) != 0u &&
         (ident->compat_features & AFSP_COMPAT_DATA_POLICY) == 0u) ||
        afspr_decode_timespec(p + 32u, &object->created) != AFSPR_OK ||
        afspr_decode_timespec(p + 44u, &object->modified) != AFSPR_OK ||
        afspr_decode_timespec(p + 56u, &object->changed) != AFSPR_OK) {
        return AFSPR_ERR_CORRUPT;
    }
    if (object->type == AFSPR_OBJECT_DIRECTORY) {
        if ((object->flags & ~AFSPR_OBJECT_PRESERVED_FLAGS) != 0u ||
            object->size_bytes != 0u || object->data_blocks != 0u ||
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
    struct afspr_ident ident = {0};
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
    if (object_id == AFSPR_OBJECT_ORPHAN_DIRECTORY &&
        (ident.ro_compat_features & AFSPR_RO_COMPAT_ORPHAN_DIRECTORY) != 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                            AFSPR_STAGE_TREE_TRAVERSAL,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            volume->object_map_block);
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
    memmove(name_buffer, name, name_len);
    entry->name = (const uint8_t *)name_buffer;
    return AFSPR_OK;
}

static int afspr_directory_entry_at_internal(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_object *directory, uint64_t ordinal,
    void *name_buffer, size_t name_capacity,
    struct afspr_directory_entry *entry, size_t entry_size,
    uint64_t *total_entries, uint8_t *key_buffer, size_t key_capacity,
    size_t *key_len, struct afspr_diagnostic *diagnostic,
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
    uint64_t expected_subtree_items = 0u;
    uint64_t remaining = ordinal;
    int expected_level = -1;
    unsigned depth;
    int status = afspr_prepare_operation(
        ops, scratch, volume,
        key_buffer == NULL ? AFSPR_TREE_SCRATCH_SIZE
                           : AFSPR_INTENT_SCRATCH_SIZE,
        &ident, diagnostic, diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (directory == NULL || entry == NULL ||
        ((key_buffer == NULL) != (key_len == NULL)) ||
        (key_buffer != NULL && key_capacity < AFSPR_TREE_MAX_KEY) ||
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
    if (directory->object_id == AFSPR_OBJECT_ORPHAN_DIRECTORY &&
        (ident.ro_compat_features & AFSPR_RO_COMPAT_ORPHAN_DIRECTORY) != 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                            AFSPR_STAGE_TREE_TRAVERSAL,
                            AFSPR_NO_CHECKPOINT_SLOT, directory->data_root);
    }
    if (directory->type != AFSPR_OBJECT_DIRECTORY) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_DIRECTORY,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if ((directory->flags & ~AFSPR_OBJECT_PRESERVED_FLAGS) != 0u ||
        directory->size_bytes != 0u || directory->data_blocks != 0u ||
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
            expected_subtree_items, depth == 0u,
            lower_len == 0u ? NULL : lower, lower_len,
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
            if (status == AFSPR_OK && key_buffer != NULL) {
                memcpy(key_buffer, item.key, item.key_len);
                *key_len = item.key_len;
            }
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
            expected_subtree_items = child_items;
        }
        expected_level = (int)node.level - 1;
    }
    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_TREE_TRAVERSAL,
                        AFSPR_NO_CHECKPOINT_SLOT, lba);
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
    return afspr_directory_entry_at_internal(
        ops, scratch, volume, directory, ordinal, name_buffer,
        name_capacity, entry, entry_size, total_entries, NULL, 0u, NULL,
        diagnostic, diagnostic_size);
}

int afspr_decode_extent_item(const uint8_t *key, size_t key_size,
                             const uint8_t *value, size_t value_size,
                             struct afspr_extent *extent)
{
    struct afspr_extent decoded;

    if (key == NULL || value == NULL || extent == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    if (key_size != 8u || value_size != AFSPR_EXTENT_VALUE_SIZE) {
        return AFSPR_ERR_CORRUPT;
    }
    decoded.logical_start = afspr_get_be64(key);
    decoded.physical_start = afspr_get_le64(value);
    decoded.block_count = afspr_get_le64(value + 8u);
    decoded.flags = afspr_get_le32(value + 16u);
    decoded.reserved32 = 0u;
    if (value[20] != 0u || value[21] != 0u || value[22] != 0u ||
        value[23] != 0u || decoded.block_count == 0u ||
        (decoded.flags & ~AFSPR_EXTENT_KNOWN_FLAGS) != 0u ||
        UINT64_MAX - decoded.logical_start < decoded.block_count ||
        UINT64_MAX - decoded.physical_start < decoded.block_count) {
        return AFSPR_ERR_CORRUPT;
    }
    *extent = decoded;
    return AFSPR_OK;
}

static int afspr_decode_extent(const struct afspr_ident *ident,
                               const struct afspr_probe_result *volume,
                               const uint8_t key[8], const uint8_t value[24],
                               struct afspr_extent *extent)
{
    uint64_t physical_end;
    uint32_t first_region;
    uint32_t last_region;

    if (afspr_decode_extent_item(key, 8u, value, AFSPR_EXTENT_VALUE_SIZE,
                                 extent) != AFSPR_OK) {
        return AFSPR_ERR_CORRUPT;
    }
    physical_end = extent->physical_start + extent->block_count;
    if (physical_end > ident->total_blocks ||
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
    struct afspr_ident ident = {0};
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
    if ((file->flags & ~(AFSPR_OBJECT_FLAG_EXTENT_TREE |
                         AFSPR_OBJECT_FLAG_DATA_IN_PLACE |
                         AFSPR_OBJECT_PRESERVED_FLAGS)) != 0u ||
        ((file->flags & AFSPR_OBJECT_FLAG_DATA_IN_PLACE) != 0u &&
         (ident.compat_features & AFSP_COMPAT_DATA_POLICY) == 0u) ||
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

static uint64_t afspr_ceil_div_u64(uint64_t value, uint64_t divisor)
{
    return value / divisor + (value % divisor != 0u ? 1u : 0u);
}

static int afspr_log_slot_lba(const struct afspr_ident *ident,
                              uint32_t slot, uint64_t *lba)
{
    uint64_t regions = afspr_ceil_div_u64(ident->total_blocks,
                                          ident->region_size);
    uint64_t leaf_capacity;
    uint64_t fanout;
    uint64_t level_nodes;
    uint64_t logical_nodes;
    uint64_t ordinal;
    uint64_t region_zero_available;
    uint64_t full_reserved;
    uint64_t full_available;
    uint64_t full_after_zero;
    uint64_t full_span;
    uint64_t index;
    uint64_t region;
    uint32_t full_pages;
    int partial_last;

    if (slot >= ident->log_slots || regions == 0u) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    leaf_capacity = (AFSP_DEFAULT_BLOCK_SIZE - AFSPR_HEADER_SIZE -
                     AFSPR_TREE_FIXED_PAYLOAD) /
                    (AFSPR_TREE_ITEM_FIXED + 4u + 16u);
    fanout = leaf_capacity + 1u;
    if (leaf_capacity == 0u || fanout < 2u) {
        return AFSPR_ERR_CORRUPT;
    }
    level_nodes = afspr_ceil_div_u64(regions, leaf_capacity);
    logical_nodes = level_nodes;
    while (level_nodes > 1u) {
        level_nodes = afspr_ceil_div_u64(level_nodes, fanout);
        if (UINT64_MAX - logical_nodes < level_nodes) {
            return AFSPR_ERR_CORRUPT;
        }
        logical_nodes += level_nodes;
    }
    if (logical_nodes > (UINT64_MAX - 4u - slot) / 3u) {
        return AFSPR_ERR_CORRUPT;
    }
    ordinal = 4u + logical_nodes * 3u + slot;

    region_zero_available = afspr_region_valid_blocks(ident, 0u) -
                            afspr_region_reserved_blocks(ident, 0u);
    if (ordinal < region_zero_available) {
        *lba = afspr_region_reserved_blocks(ident, 0u) + ordinal;
        return AFSPR_OK;
    }
    index = ordinal - region_zero_available;
    if (regions == 1u) {
        return AFSPR_ERR_CORRUPT;
    }

    full_pages = ident->region_size / AFSPR_BITMAP_PAGE_BLOCKS;
    if (ident->region_size % AFSPR_BITMAP_PAGE_BLOCKS != 0u) {
        ++full_pages;
    }
    full_reserved = AFSPR_DESCRIPTOR_SLOTS +
                    (uint64_t)full_pages * AFSPR_BITMAP_SLOTS;
    full_available = ident->region_size - full_reserved;
    partial_last = ident->total_blocks % ident->region_size != 0u;
    full_after_zero = regions - 1u - (partial_last ? 1u : 0u);
    full_span = full_after_zero * full_available;
    if (index < full_span) {
        region = 1u + index / full_available;
        *lba = region * ident->region_size + full_reserved +
               index % full_available;
        return *lba < ident->total_blocks ? AFSPR_OK : AFSPR_ERR_CORRUPT;
    }
    index -= full_span;
    if (partial_last) {
        uint32_t last = (uint32_t)(regions - 1u);
        uint64_t reserved = afspr_region_reserved_blocks(ident, last);
        uint64_t available = afspr_region_valid_blocks(ident, last) - reserved;

        if (index < available) {
            *lba = (uint64_t)last * ident->region_size + reserved + index;
            return AFSPR_OK;
        }
    }
    return AFSPR_ERR_CORRUPT;
}

static int afspr_valid_name(const uint8_t *name, size_t size)
{
    size_t index;

    if (size == 0u || size > AFSP_NAME_MAX_UTF8_BYTES ||
        !afspr_valid_utf8(name, size)) {
        return 0;
    }
    for (index = 0u; index < size; ++index) {
        if (name[index] == 0u || name[index] == (uint8_t)'/') {
            return 0;
        }
    }
    return 1;
}

/*
 * The bootstrap reader deliberately carries no Unicode normalization tables.
 * Legacy keys are byte identity; the versioned Unicode profiles are exact for
 * ASCII and fail closed for every non-ASCII name until those tables land.
 */
static int afspr_name_key(const struct afspr_ident *ident,
                          const uint8_t *name, size_t name_len,
                          uint8_t *key, size_t *key_len)
{
    size_t index;

    if (!afspr_valid_name(name, name_len) || key == NULL || key_len == NULL) {
        return AFSPR_ERR_CORRUPT;
    }
    if (name_len > AFSPR_TREE_MAX_KEY) {
        return AFSPR_ERR_CORRUPT;
    }
    if (ident->name_key_algorithm != 0u) {
        for (index = 0u; index < name_len; ++index) {
            if (name[index] >= 0x80u) {
                return AFSPR_ERR_UNSUPPORTED;
            }
        }
    }
    for (index = 0u; index < name_len; ++index) {
        uint8_t byte = name[index];

        if (ident->name_key_algorithm == 2u && byte >= (uint8_t)'A' &&
            byte <= (uint8_t)'Z') {
            byte = (uint8_t)(byte + ((uint8_t)'a' - (uint8_t)'A'));
        }
        key[index] = byte;
    }
    *key_len = name_len;
    return AFSPR_OK;
}

static int afspr_log_extent_at(const struct afspr_log_operation *operation,
                               uint16_t index, uint64_t *start,
                               uint32_t *blocks)
{
    size_t offset;

    if (index >= operation->extent_count) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    offset = (size_t)index * AFSPR_LOG_EXTENT_WIRE;
    *start = afspr_get_le64(operation->extents + offset);
    *blocks = afspr_get_le32(operation->extents + offset + 8u);
    return AFSPR_OK;
}

static int afspr_validate_log_extents(
    const struct afspr_log_operation *operation, uint64_t *total)
{
    uint16_t index;

    *total = 0u;
    for (index = 0u; index < operation->extent_count; ++index) {
        uint64_t start;
        uint64_t end;
        uint32_t blocks;
        uint16_t previous;

        if (afspr_log_extent_at(operation, index, &start, &blocks) !=
                AFSPR_OK ||
            blocks == 0u || UINT64_MAX - start < blocks) {
            return AFSPR_ERR_CORRUPT;
        }
        end = start + blocks;
        for (previous = 0u; previous < index; ++previous) {
            uint64_t other_start;
            uint32_t other_blocks;
            uint64_t other_end;

            if (afspr_log_extent_at(operation, previous, &other_start,
                                    &other_blocks) != AFSPR_OK) {
                return AFSPR_ERR_CORRUPT;
            }
            other_end = other_start + other_blocks;
            if (other_start < end && start < other_end) {
                return AFSPR_ERR_CORRUPT;
            }
        }
        if (UINT64_MAX - *total < blocks) {
            return AFSPR_ERR_CORRUPT;
        }
        *total += blocks;
    }
    return AFSPR_OK;
}

static int afspr_decode_log_operation(
    const struct afspr_log_record *record, size_t *offset,
    struct afspr_log_operation *operation)
{
    const uint8_t *entry;
    size_t variable;
    size_t names_at;
    uint64_t extent_blocks;
    uint64_t logical_end;
    uint64_t file_blocks;

    if (*offset > record->payload_len ||
        record->operation_fixed > record->payload_len - *offset) {
        return AFSPR_ERR_CORRUPT;
    }
    entry = record->payload + *offset;
    memset(operation, 0, sizeof(*operation));
    operation->type = entry[0];
    operation->replace = entry[1];
    operation->source_len = afspr_get_le16(entry + 2u);
    operation->target_len = afspr_get_le16(entry + 4u);
    operation->extent_count = afspr_get_le16(entry + 6u);
    operation->first = afspr_get_le64(entry + 8u);
    operation->second = afspr_get_le64(entry + 16u);
    operation->third = afspr_get_le64(entry + 24u);
    operation->fourth = afspr_get_le64(entry + 32u);
    operation->content_crc = afspr_get_le32(entry + 40u);
    if (operation->extent_count > AFSPR_LOG_MAX_EXTENTS) {
        return AFSPR_ERR_CORRUPT;
    }
    if (record->version != 0u) {
        if (afspr_decode_timespec(entry + 48u, &operation->timestamp) !=
            AFSPR_OK) {
            return AFSPR_ERR_CORRUPT;
        }
    }

    if (operation->type == 1u && operation->target_len == 0u) {
        variable = (size_t)operation->extent_count *
                       AFSPR_LOG_EXTENT_WIRE +
                   operation->source_len;
    } else if (operation->type == 2u && operation->target_len == 0u &&
               operation->extent_count == 0u) {
        variable = operation->source_len;
    } else if (operation->type == 3u && operation->extent_count == 0u) {
        variable = (size_t)operation->source_len + operation->target_len;
    } else if ((operation->type == 4u || operation->type == 5u) &&
               record->version == AFSPR_LOG_CURRENT_VERSION &&
               operation->source_len == 0u &&
               operation->target_len == 0u) {
        variable = (size_t)operation->extent_count *
                   AFSPR_LOG_EXTENT_WIRE;
    } else {
        return AFSPR_ERR_CORRUPT;
    }
    if (variable > record->payload_len - *offset - record->operation_fixed) {
        return AFSPR_ERR_CORRUPT;
    }
    operation->extents = entry + record->operation_fixed;
    names_at = (size_t)operation->extent_count * AFSPR_LOG_EXTENT_WIRE;
    operation->source_name = operation->extents + names_at;
    operation->target_name = operation->source_name + operation->source_len;
    if (afspr_validate_log_extents(operation, &extent_blocks) != AFSPR_OK) {
        return AFSPR_ERR_CORRUPT;
    }

    switch (operation->type) {
    case 1u:
        if (!afspr_valid_name(operation->source_name,
                              operation->source_len) ||
            operation->third == 0u ||
            (operation->extent_count == 0u && operation->fourth != 0u) ||
            (operation->extent_count != 0u &&
             (operation->fourth == 0u ||
              afspr_ceil_div_u64(operation->fourth,
                                 AFSP_DEFAULT_BLOCK_SIZE) > extent_blocks))) {
            return AFSPR_ERR_CORRUPT;
        }
        break;
    case 2u:
        if (!afspr_valid_name(operation->source_name,
                              operation->source_len)) {
            return AFSPR_ERR_CORRUPT;
        }
        break;
    case 3u:
        if (!afspr_valid_name(operation->source_name,
                              operation->source_len) ||
            !afspr_valid_name(operation->target_name,
                              operation->target_len)) {
            return AFSPR_ERR_CORRUPT;
        }
        break;
    case 4u:
        if (operation->first == 0u || extent_blocks == 0u ||
            UINT64_MAX - operation->second < extent_blocks) {
            return AFSPR_ERR_CORRUPT;
        }
        logical_end = operation->second + extent_blocks;
        if (operation->fourth < operation->third ||
            operation->fourth == 0u) {
            return AFSPR_ERR_CORRUPT;
        }
        file_blocks = afspr_ceil_div_u64(operation->fourth,
                                         AFSP_DEFAULT_BLOCK_SIZE);
        if (logical_end > file_blocks ||
            (operation->fourth > operation->third &&
             logical_end != file_blocks)) {
            return AFSPR_ERR_CORRUPT;
        }
        break;
    case 5u:
        if (operation->first == 0u ||
            operation->third == operation->fourth || extent_blocks > 1u) {
            return AFSPR_ERR_CORRUPT;
        }
        if (operation->fourth == 0u && extent_blocks != 0u) {
            return AFSPR_ERR_CORRUPT;
        }
        if (extent_blocks == 0u) {
            if (operation->second != 0u || operation->content_crc != 0u) {
                return AFSPR_ERR_CORRUPT;
            }
        } else if (operation->fourth >= operation->third ||
                   operation->fourth % AFSP_DEFAULT_BLOCK_SIZE == 0u ||
                   operation->second !=
                       operation->fourth / AFSP_DEFAULT_BLOCK_SIZE) {
            return AFSPR_ERR_CORRUPT;
        }
        break;
    default:
        return AFSPR_ERR_CORRUPT;
    }

    *offset += record->operation_fixed + variable;
    return AFSPR_OK;
}

static int afspr_decode_log_record(
    const uint8_t *block, size_t block_size, struct afspr_log_record *record)
{
    struct afspr_header header;
    const uint8_t *payload;
    size_t offset = AFSPR_LOG_FIXED_PAYLOAD;
    uint16_t index;
    int status = afspr_verify_header(block, block_size,
                                     AFSPR_BLOCK_TYPE_INTENT, &header);

    /* The record belongs to the volume and has no flag namespace
     * (ADR-114). */
    if (status != AFSPR_OK || header.flags != 0u || header.owner != 0u ||
        header.payload_len < AFSPR_LOG_FIXED_PAYLOAD) {
        return status == AFSPR_OK ? AFSPR_ERR_CORRUPT : status;
    }
    payload = block + AFSPR_HEADER_SIZE;
    memset(record, 0, sizeof(*record));
    record->payload = payload;
    record->payload_len = header.payload_len;
    record->base_generation = afspr_get_le64(payload + 16u);
    record->sequence = afspr_get_le32(payload + 24u);
    record->operation_count = afspr_get_le16(payload + 28u);
    record->version = afspr_get_le16(payload + 30u);
    if (record->base_generation == 0u ||
        record->base_generation != header.generation ||
        record->sequence == 0u || record->operation_count == 0u ||
        record->operation_count > AFSPR_LOG_MAX_OPS) {
        return AFSPR_ERR_CORRUPT;
    }
    if (record->version == 0u) {
        record->operation_fixed = AFSPR_LOG_LEGACY_OP_FIXED;
    } else if (record->version == AFSPR_LOG_PREVIOUS_VERSION ||
               record->version == AFSPR_LOG_CURRENT_VERSION) {
        record->operation_fixed = AFSPR_LOG_OP_FIXED;
    } else {
        return AFSPR_ERR_UNSUPPORTED;
    }
    for (index = 0u; index < record->operation_count; ++index) {
        struct afspr_log_operation operation;

        status = afspr_decode_log_operation(record, &offset, &operation);
        if (status != AFSPR_OK) {
            return status;
        }
    }
    if (offset != record->payload_len) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

static int afspr_log_operation_at(const struct afspr_log_record *record,
                                  uint16_t wanted,
                                  struct afspr_log_operation *operation)
{
    size_t offset = AFSPR_LOG_FIXED_PAYLOAD;
    uint16_t index;

    if (wanted >= record->operation_count) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    for (index = 0u; index <= wanted; ++index) {
        int status = afspr_decode_log_operation(record, &offset, operation);

        if (status != AFSPR_OK) {
            return status;
        }
    }
    return AFSPR_OK;
}

static int afspr_log_extent_valid(const struct afspr_ident *ident,
                                  uint64_t start, uint32_t blocks)
{
    uint64_t end;

    if (blocks == 0u || UINT64_MAX - start < blocks) {
        return 0;
    }
    end = start + blocks;
    return end <= ident->total_blocks && afspr_is_allocatable(ident, start) &&
           afspr_is_allocatable(ident, end - 1u) &&
           start / ident->region_size == (end - 1u) / ident->region_size;
}

static int afspr_ranges_overlap(uint64_t start, uint32_t blocks,
                                uint64_t other_start,
                                uint32_t other_blocks)
{
    uint64_t end = start + blocks;
    uint64_t other_end = other_start + other_blocks;

    return other_start < end && start < other_end;
}

static int afspr_log_extent_used_in_record_before(
    const struct afspr_log_record *record, uint16_t before_operation,
    uint64_t start, uint32_t blocks, int *used)
{
    uint16_t operation_index;

    *used = 0;
    for (operation_index = 0u; operation_index < before_operation;
         ++operation_index) {
        struct afspr_log_operation operation;
        uint16_t extent_index;

        if (afspr_log_operation_at(record, operation_index, &operation) !=
            AFSPR_OK) {
            return AFSPR_ERR_CORRUPT;
        }
        for (extent_index = 0u; extent_index < operation.extent_count;
             ++extent_index) {
            uint64_t other_start;
            uint32_t other_blocks;

            if (afspr_log_extent_at(&operation, extent_index, &other_start,
                                    &other_blocks) != AFSPR_OK) {
                return AFSPR_ERR_CORRUPT;
            }
            if (afspr_ranges_overlap(start, blocks, other_start,
                                     other_blocks)) {
                *used = 1;
                return AFSPR_OK;
            }
        }
    }
    return AFSPR_OK;
}

static int afspr_log_extent_used_in_prior_slots(
    const struct afspr_block_ops *ops, const struct afspr_ident *ident,
    uint32_t before_slot, uint64_t start, uint32_t blocks, uint8_t *buffer, int *used,
    struct afspr_diagnostic *diagnostic)
{
    uint32_t slot;

    *used = 0;
    for (slot = 0u; slot < before_slot; ++slot) {
        struct afspr_log_record record;
        uint64_t lba;
        uint16_t operation_index;
        int status = afspr_log_slot_lba(ident, slot, &lba);

        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_INTENT_READ,
                                AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
        }
        status = afspr_read_one(ops, lba, buffer);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_INTENT_READ,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        if (afspr_decode_log_record(buffer, ops->block_size, &record) !=
            AFSPR_OK) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_INTENT_DECODE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        for (operation_index = 0u;
             operation_index < record.operation_count; ++operation_index) {
            struct afspr_log_operation operation;
            uint16_t extent_index;

            if (afspr_log_operation_at(&record, operation_index,
                                       &operation) != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            for (extent_index = 0u; extent_index < operation.extent_count;
                 ++extent_index) {
                uint64_t other_start;
                uint32_t other_blocks;

                if (afspr_log_extent_at(&operation, extent_index,
                                        &other_start,
                                        &other_blocks) != AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (afspr_ranges_overlap(start, blocks, other_start,
                                         other_blocks)) {
                    *used = 1;
                    return AFSPR_OK;
                }
            }
        }
    }
    return AFSPR_OK;
}

static int afspr_verify_log_operation_content(
    const struct afspr_block_ops *ops, const struct afspr_ident *ident,
    const struct afspr_log_operation *operation, uint8_t *buffer,
    uint64_t *failed_lba)
{
    uint64_t remaining;
    uint64_t extent_blocks;
    uint32_t crc = UINT32_MAX;
    uint16_t extent_index;

    *failed_lba = AFSPR_NO_BLOCK;
    if (operation->type == 2u || operation->type == 3u) {
        return AFSPR_OK;
    }
    if (operation->type == 1u) {
        remaining = operation->fourth;
    } else {
        if (afspr_validate_log_extents(operation, &extent_blocks) !=
                AFSPR_OK ||
            extent_blocks > UINT64_MAX / ops->block_size) {
            return AFSPR_ERR_CORRUPT;
        }
        remaining = extent_blocks * ops->block_size;
    }
    for (extent_index = 0u; extent_index < operation->extent_count;
         ++extent_index) {
        uint64_t start;
        uint32_t blocks;
        uint32_t block_index;

        if (afspr_log_extent_at(operation, extent_index, &start, &blocks) !=
                AFSPR_OK ||
            !afspr_log_extent_valid(ident, start, blocks)) {
            *failed_lba = start;
            return AFSPR_ERR_CORRUPT;
        }
        if (*failed_lba == AFSPR_NO_BLOCK) {
            *failed_lba = start;
        }
        for (block_index = 0u; block_index < blocks && remaining != 0u;
             ++block_index) {
            uint64_t lba = start + block_index;
            size_t take;

            if (afspr_read_one(ops, lba, buffer) != AFSPR_OK) {
                *failed_lba = lba;
                return AFSPR_ERR_IO;
            }
            take = remaining < ops->block_size ? (size_t)remaining
                                                : ops->block_size;
            crc = afspr_crc32c_update(crc, buffer, take);
            remaining -= take;
        }
    }
    if (remaining != 0u || ~crc != operation->content_crc) {
        return AFSPR_ERR_CORRUPT;
    }
    return AFSPR_OK;
}

int afspr_scan_intent_log(const struct afspr_block_ops *ops,
                          const struct afspr_scratch *scratch,
                          const struct afspr_probe_result *volume,
                          struct afspr_intent_view *view, size_t view_size,
                          struct afspr_diagnostic *diagnostic,
                          size_t diagnostic_size)
{
    struct afspr_ident ident;
    uint8_t *record_buffer;
    uint8_t *work_buffer;
    uint32_t slot;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_INTENT_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (view == NULL || view_size < sizeof(*view)) {
        return afspr_report(diagnostic,
                            view_size < sizeof(*view) ? AFSPR_ERR_ABI
                                                      : AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    memset(view, 0, sizeof(*view));
    view->abi_version = AFSPR_ABI_VERSION;
    view->tail_slot = AFSPR_NO_LOG_SLOT;
    view->tail_block = AFSPR_NO_BLOCK;
    view->base_generation = volume->generation;
    view->flags = AFSPR_INTENT_VIEW_FILE_DATA |
                  AFSPR_INTENT_VIEW_NAMESPACE;
    record_buffer = (uint8_t *)scratch->buffer;
    work_buffer = record_buffer + ops->block_size;
    if (ident.log_slots == 0u) {
        return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }

    for (slot = 0u; slot < ident.log_slots; ++slot) {
        struct afspr_log_record record;
        uint64_t lba;
        uint16_t operation_index;

        status = afspr_log_slot_lba(&ident, slot, &lba);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_INTENT_READ,
                                AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
        }
        status = afspr_read_one(ops, lba, record_buffer);
        if (status != AFSPR_OK) {
            return afspr_report(diagnostic, status, AFSPR_STAGE_INTENT_READ,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        status = afspr_decode_log_record(record_buffer, ops->block_size,
                                         &record);
        if (status != AFSPR_OK) {
            view->tail_state = AFSPR_INTENT_TAIL_INVALID;
            view->tail_slot = slot;
            view->tail_block = lba;
            return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                                AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
        }
        if (memcmp(record.payload, volume->uuid, sizeof(volume->uuid)) != 0 ||
            record.base_generation != volume->generation) {
            view->tail_state = AFSPR_INTENT_TAIL_STALE;
            view->tail_slot = slot;
            view->tail_block = lba;
            return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                                AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
        }
        if (record.sequence != slot + 1u) {
            view->tail_state = AFSPR_INTENT_TAIL_SEQUENCE;
            view->tail_slot = slot;
            view->tail_block = lba;
            return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                                AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
        }
        for (operation_index = 0u;
             operation_index < record.operation_count; ++operation_index) {
            struct afspr_log_operation operation;
            uint16_t extent_index;

            if (afspr_log_operation_at(&record, operation_index,
                                       &operation) != AFSPR_OK) {
                view->tail_state = AFSPR_INTENT_TAIL_INVALID;
                view->tail_slot = slot;
                view->tail_block = lba;
                return afspr_report(diagnostic, AFSPR_OK,
                                    AFSPR_STAGE_COMPLETE,
                                    AFSPR_NO_CHECKPOINT_SLOT,
                                    AFSPR_NO_BLOCK);
            }
            if ((operation.type == 4u || operation.type == 5u) &&
                (volume->incompat_features &
                 AFSP_INCOMPAT_INTENT_LOG_DATA_UPDATES) == 0u) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (operation.type >= 1u && operation.type <= 3u) {
                size_t ignored_key_len;
                int key_status = afspr_name_key(
                    &ident, operation.source_name, operation.source_len,
                    work_buffer, &ignored_key_len);

                if (key_status == AFSPR_ERR_UNSUPPORTED) {
                    view->flags &= ~(AFSPR_INTENT_VIEW_FILE_DATA |
                                     AFSPR_INTENT_VIEW_NAMESPACE);
                } else if (key_status != AFSPR_OK) {
                    view->tail_state = AFSPR_INTENT_TAIL_INVALID;
                    view->tail_slot = slot;
                    view->tail_block = lba;
                    return afspr_report(diagnostic, AFSPR_OK,
                                        AFSPR_STAGE_COMPLETE,
                                        AFSPR_NO_CHECKPOINT_SLOT,
                                        AFSPR_NO_BLOCK);
                }
                if (operation.type == 3u) {
                    key_status = afspr_name_key(
                        &ident, operation.target_name,
                        operation.target_len, work_buffer,
                        &ignored_key_len);
                    if (key_status == AFSPR_ERR_UNSUPPORTED) {
                        view->flags &= ~(AFSPR_INTENT_VIEW_FILE_DATA |
                                         AFSPR_INTENT_VIEW_NAMESPACE);
                    } else if (key_status != AFSPR_OK) {
                        view->tail_state = AFSPR_INTENT_TAIL_INVALID;
                        view->tail_slot = slot;
                        view->tail_block = lba;
                        return afspr_report(diagnostic, AFSPR_OK,
                                            AFSPR_STAGE_COMPLETE,
                                            AFSPR_NO_CHECKPOINT_SLOT,
                                            AFSPR_NO_BLOCK);
                    }
                }
            }
            for (extent_index = 0u; extent_index < operation.extent_count;
                 ++extent_index) {
                uint64_t start;
                uint32_t blocks;
                int used;

                if (afspr_log_extent_at(&operation, extent_index, &start,
                                        &blocks) != AFSPR_OK ||
                    !afspr_log_extent_valid(&ident, start, blocks)) {
                    view->tail_state = AFSPR_INTENT_TAIL_CONTENT;
                    view->tail_slot = slot;
                    view->tail_block = start;
                    return afspr_report(diagnostic, AFSPR_OK,
                                        AFSPR_STAGE_COMPLETE,
                                        AFSPR_NO_CHECKPOINT_SLOT,
                                        AFSPR_NO_BLOCK);
                }
                status = afspr_log_extent_used_in_record_before(
                    &record, operation_index, start, blocks, &used);
                if (status != AFSPR_OK) {
                    return afspr_report(diagnostic, status,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (!used) {
                    status = afspr_log_extent_used_in_prior_slots(
                        ops, &ident, slot, start, blocks, work_buffer, &used,
                        diagnostic);
                    if (status != AFSPR_OK) {
                        return status;
                    }
                }
                if (used) {
                    view->tail_state = AFSPR_INTENT_TAIL_CONTENT;
                    view->tail_slot = slot;
                    view->tail_block = start;
                    return afspr_report(diagnostic, AFSPR_OK,
                                        AFSPR_STAGE_COMPLETE,
                                        AFSPR_NO_CHECKPOINT_SLOT,
                                        AFSPR_NO_BLOCK);
                }
            }
            {
                uint64_t failed_lba;

                status = afspr_verify_log_operation_content(
                    ops, &ident, &operation, work_buffer, &failed_lba);
                if (status != AFSPR_OK) {
                    view->tail_state = AFSPR_INTENT_TAIL_CONTENT;
                    view->tail_slot = slot;
                    view->tail_block = failed_lba;
                    return afspr_report(diagnostic, AFSPR_OK,
                                        AFSPR_STAGE_COMPLETE,
                                        AFSPR_NO_CHECKPOINT_SLOT,
                                        AFSPR_NO_BLOCK);
                }
            }
        }
        ++view->valid_records;
        view->valid_operations += record.operation_count;
        view->last_sequence = record.sequence;
    }
    view->tail_state = AFSPR_INTENT_TAIL_FULL;
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
}

struct afspr_intent_file_state {
    int exists;
    int has_committed;
    uint32_t link_count;
    uint64_t size_bytes;
    struct afspr_object committed;
};

static int afspr_intent_views_equal(const struct afspr_intent_view *left,
                                    const struct afspr_intent_view *right)
{
    return left->valid_records == right->valid_records &&
           left->valid_operations == right->valid_operations &&
           left->tail_state == right->tail_state &&
           left->tail_slot == right->tail_slot &&
           left->flags == right->flags &&
           left->base_generation == right->base_generation &&
           left->last_sequence == right->last_sequence &&
           left->tail_block == right->tail_block;
}

static int afspr_validate_intent_view(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint32_t required_flags,
    struct afspr_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspr_intent_view current;
    int status;

    if (view == NULL) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (view->abi_version != AFSPR_ABI_VERSION ||
        (view->flags & ~(AFSPR_INTENT_VIEW_FILE_DATA |
                         AFSPR_INTENT_VIEW_NAMESPACE)) != 0u ||
        (required_flags & ~(AFSPR_INTENT_VIEW_FILE_DATA |
                            AFSPR_INTENT_VIEW_NAMESPACE)) != 0u ||
        view->base_generation != volume->generation ||
        view->valid_records > volume->log_slots) {
        return afspr_report(diagnostic, AFSPR_ERR_ABI,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_scan_intent_log(ops, scratch, volume, &current,
                                   sizeof(current), diagnostic,
                                   diagnostic_size);
    if (status != AFSPR_OK) {
        return status;
    }
    if (!afspr_intent_views_equal(view, &current)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            current.tail_block);
    }
    if ((view->flags & required_flags) != required_flags) {
        return afspr_report(diagnostic, AFSPR_ERR_UNSUPPORTED,
                            AFSPR_STAGE_INTENT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            view->tail_block);
    }
    return AFSPR_OK;
}

static int afspr_read_intent_record(
    const struct afspr_block_ops *ops, const struct afspr_ident *ident,
    const struct afspr_probe_result *volume, uint32_t slot, uint8_t *buffer,
    struct afspr_log_record *record, uint64_t *lba,
    struct afspr_diagnostic *diagnostic)
{
    int status = afspr_log_slot_lba(ident, slot, lba);

    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_READ,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_read_one(ops, *lba, buffer);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status, AFSPR_STAGE_INTENT_READ,
                            AFSPR_NO_CHECKPOINT_SLOT, *lba);
    }
    status = afspr_decode_log_record(buffer, ops->block_size, record);
    if (status != AFSPR_OK ||
        memcmp(record->payload, volume->uuid, sizeof(volume->uuid)) != 0 ||
        record->base_generation != volume->generation ||
        record->sequence != slot + 1u) {
        return afspr_report(diagnostic,
                            status == AFSPR_ERR_UNSUPPORTED
                                ? AFSPR_ERR_UNSUPPORTED
                                : AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, *lba);
    }
    return AFSPR_OK;
}

struct afspr_log_position {
    uint32_t slot;
    uint16_t operation;
};

struct afspr_namespace_operation {
    uint8_t type;
    uint8_t replace;
    uint16_t source_len;
    uint16_t target_len;
    uint64_t first;
    uint64_t second;
    uint64_t third;
    uint64_t fourth;
    uint8_t source_name[AFSP_NAME_MAX_UTF8_BYTES];
    uint8_t target_name[AFSP_NAME_MAX_UTF8_BYTES];
};

static void afspr_copy_namespace_operation(
    const struct afspr_log_operation *source,
    struct afspr_namespace_operation *destination)
{
    memset(destination, 0, sizeof(*destination));
    destination->type = source->type;
    destination->replace = source->replace;
    destination->source_len = source->source_len;
    destination->target_len = source->target_len;
    destination->first = source->first;
    destination->second = source->second;
    destination->third = source->third;
    destination->fourth = source->fourth;
    if (source->source_len != 0u) {
        memcpy(destination->source_name, source->source_name,
               source->source_len);
    }
    if (source->target_len != 0u) {
        memcpy(destination->target_name, source->target_name,
               source->target_len);
    }
}

static int afspr_namespace_result(
    uint64_t parent_id, uint64_t object_id, uint32_t type_hint,
    const uint8_t *name, size_t name_len, void *name_buffer,
    size_t name_capacity, struct afspr_directory_entry *entry)
{
    memset(entry, 0, sizeof(*entry));
    entry->abi_version = AFSPR_ABI_VERSION;
    entry->type_hint = type_hint;
    entry->object_id = object_id;
    entry->parent_id = parent_id;
    entry->name_len = name_len;
    if (name_capacity < name_len) {
        return AFSPR_ERR_BUFFER_TOO_SMALL;
    }
    if (name_buffer == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    memmove(name_buffer, name, name_len);
    entry->name = (const uint8_t *)name_buffer;
    return AFSPR_OK;
}

static int afspr_committed_directory_lookup_key(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, uint64_t directory_id,
    const uint8_t *key, size_t key_len, void *name_buffer,
    size_t name_capacity, struct afspr_directory_entry *entry,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_ident ident;
    struct afspr_object directory;
    struct afspr_tree_spec spec;
    struct afspr_tree_item item;
    uint8_t *workspace = (uint8_t *)scratch->buffer;
    uint8_t *stable_key = workspace + AFSPR_NAMESPACE_KEY_OFFSET;
    uint8_t *value = workspace + AFSPR_NAMESPACE_VALUE_OFFSET;
    size_t value_len;
    uint64_t leaf_lba;
    int found;
    int status;

    memmove(stable_key, key, key_len);
    afspr_ident_from_result(volume, &ident);
    status = afspr_lookup_object(
        ops, scratch, volume, directory_id, &directory, sizeof(directory),
        diagnostic, diagnostic == NULL ? 0u : sizeof(*diagnostic));
    if (status != AFSPR_OK) {
        return status;
    }
    if (directory.type != AFSPR_OBJECT_DIRECTORY) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_DIRECTORY,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if ((directory.flags & ~AFSPR_OBJECT_PRESERVED_FLAGS) != 0u ||
        directory.size_bytes != 0u ||
        directory.data_blocks != 0u ||
        !afspr_is_allocatable(&ident, directory.data_root)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, directory.data_root);
    }
    spec.kind = AFSPR_TREE_KIND_DIRECTORY;
    spec.owner = directory_id;
    spec.max_generation = volume->generation;
    status = afspr_tree_lookup_variable(
        ops, scratch, &ident, directory.data_root, &spec, stable_key,
        key_len, value, AFSPR_DIRECTORY_VALUE_MAX, &value_len, &found,
        &leaf_lba, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    if (!found) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, directory.data_root);
    }
    item.key = stable_key;
    item.key_len = key_len;
    item.value = value;
    item.value_len = value_len;
    status = afspr_decode_directory_entry(
        &ident, &item, directory_id, name_buffer, name_capacity, entry);
    return afspr_report(
        diagnostic, status,
        status == AFSPR_OK ? AFSPR_STAGE_COMPLETE
                           : AFSPR_STAGE_DIRECTORY_DECODE,
        AFSPR_NO_CHECKPOINT_SLOT, leaf_lba);
}

/*
 * Resolves one key against operations strictly before boundary. Rename
 * targets are chased backwards to their source key without recursion, so the
 * stack stays constant even for a log filled with rename chains.
 */
static int afspr_namespace_resolve_before(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, struct afspr_log_position boundary,
    uint64_t directory_id, const uint8_t *key, size_t key_len,
    void *name_buffer, size_t name_capacity,
    struct afspr_directory_entry *entry,
    int *created_in_log,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_ident ident;
    uint8_t *workspace = (uint8_t *)scratch->buffer;
    uint8_t *temporary_key = workspace + AFSPR_NAMESPACE_LOWER_OFFSET;
    uint8_t *current_key = workspace + AFSPR_NAMESPACE_KEY_OFFSET;
    uint8_t *base_name = workspace + AFSPR_NAMESPACE_NAME_OFFSET;
    uint8_t final_name[AFSP_NAME_MAX_UTF8_BYTES];
    size_t current_key_len = key_len;
    size_t final_name_len = 0u;
    uint64_t result_parent = directory_id;
    uint64_t chase;

    afspr_ident_from_result(volume, &ident);
    if (created_in_log != NULL) {
        *created_in_log = 0;
    }
    memmove(current_key, key, key_len);
    for (chase = 0u; chase <= view->valid_operations; ++chase) {
        uint32_t slot_cursor = boundary.slot;
        int followed = 0;

        if (boundary.slot < view->valid_records) {
            ++slot_cursor;
        }
        while (slot_cursor != 0u) {
            struct afspr_log_record record;
            uint64_t lba;
            uint16_t limit;
            int status;

            --slot_cursor;
            memset(&record, 0, sizeof(record));
            status = afspr_read_intent_record(
                ops, &ident, volume, slot_cursor, workspace, &record, &lba,
                diagnostic);
            if (status != AFSPR_OK) {
                return status;
            }
            limit = record.operation_count;
            if (boundary.slot < view->valid_records &&
                slot_cursor == boundary.slot) {
                limit = boundary.operation;
                if (limit > record.operation_count) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_NAMESPACE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
            }
            while (limit != 0u) {
                struct afspr_log_operation operation;
                size_t operation_key_len;
                int key_status;

                --limit;
                if (afspr_log_operation_at(&record, limit, &operation) !=
                    AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_NAMESPACE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (operation.type == 1u || operation.type == 2u) {
                    if (operation.first != directory_id) {
                        continue;
                    }
                    key_status = afspr_name_key(
                        &ident, operation.source_name, operation.source_len,
                        temporary_key, &operation_key_len);
                    if (key_status != AFSPR_OK) {
                        return afspr_report(diagnostic, key_status,
                                            AFSPR_STAGE_INTENT_NAMESPACE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    if (operation_key_len != current_key_len ||
                        memcmp(temporary_key, current_key,
                               current_key_len) != 0) {
                        continue;
                    }
                    if (operation.type == 2u) {
                        return afspr_report(diagnostic,
                                            AFSPR_ERR_NOT_FOUND,
                                            AFSPR_STAGE_INTENT_NAMESPACE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    if (final_name_len == 0u) {
                        memcpy(final_name, operation.source_name,
                               operation.source_len);
                        final_name_len = operation.source_len;
                    }
                    if (created_in_log != NULL) {
                        *created_in_log = 1;
                    }
                    return afspr_namespace_result(
                        result_parent, operation.third, AFSPR_OBJECT_FILE,
                        final_name, final_name_len, name_buffer,
                        name_capacity, entry);
                }
                if (operation.type == 3u) {
                    int target_match;
                    int source_match;

                    key_status = afspr_name_key(
                        &ident, operation.target_name,
                        operation.target_len, temporary_key,
                        &operation_key_len);
                    if (key_status != AFSPR_OK) {
                        return afspr_report(diagnostic, key_status,
                                            AFSPR_STAGE_INTENT_NAMESPACE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    target_match = operation.second == directory_id &&
                                   operation_key_len == current_key_len &&
                                   memcmp(temporary_key, current_key,
                                          current_key_len) == 0;
                    if (target_match) {
                        if (final_name_len == 0u) {
                            memcpy(final_name, operation.target_name,
                                   operation.target_len);
                            final_name_len = operation.target_len;
                        }
                        key_status = afspr_name_key(
                            &ident, operation.source_name,
                            operation.source_len, current_key,
                            &current_key_len);
                        if (key_status != AFSPR_OK) {
                            return afspr_report(
                                diagnostic, key_status,
                                AFSPR_STAGE_INTENT_NAMESPACE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
                        }
                        directory_id = operation.first;
                        boundary.slot = slot_cursor;
                        boundary.operation = limit;
                        followed = 1;
                        break;
                    }
                    key_status = afspr_name_key(
                        &ident, operation.source_name,
                        operation.source_len, temporary_key,
                        &operation_key_len);
                    if (key_status != AFSPR_OK) {
                        return afspr_report(diagnostic, key_status,
                                            AFSPR_STAGE_INTENT_NAMESPACE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    source_match = operation.first == directory_id &&
                                   operation_key_len == current_key_len &&
                                   memcmp(temporary_key, current_key,
                                          current_key_len) == 0;
                    if (source_match) {
                        return afspr_report(diagnostic,
                                            AFSPR_ERR_NOT_FOUND,
                                            AFSPR_STAGE_INTENT_NAMESPACE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                }
            }
            if (followed) {
                break;
            }
        }
        if (followed) {
            continue;
        }
        {
            struct afspr_directory_entry committed;
            memset(&committed, 0, sizeof(committed));
            int status = afspr_committed_directory_lookup_key(
                ops, scratch, volume, directory_id, current_key,
                current_key_len, base_name, AFSP_NAME_MAX_UTF8_BYTES,
                &committed, diagnostic);

            if (status != AFSPR_OK) {
                return status;
            }
            if (final_name_len == 0u) {
                memcpy(final_name, base_name, committed.name_len);
                final_name_len = committed.name_len;
            }
            return afspr_namespace_result(
                result_parent, committed.object_id, committed.type_hint,
                final_name, final_name_len, name_buffer, name_capacity,
                entry);
        }
    }
    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_INTENT_NAMESPACE,
                        AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
}

static int afspr_load_intent_file_state_before(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, struct afspr_log_position boundary,
    uint64_t object_id, struct afspr_intent_file_state *state,
    struct afspr_diagnostic *diagnostic);

static int afspr_namespace_parent_is_directory(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, uint64_t object_id,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_object object;
    memset(&object, 0, sizeof(object));
    int status = afspr_lookup_object(
        ops, scratch, volume, object_id, &object, sizeof(object), diagnostic,
        diagnostic == NULL ? 0u : sizeof(*diagnostic));

    if (status != AFSPR_OK) {
        return status;
    }
    return object.type == AFSPR_OBJECT_DIRECTORY ? AFSPR_OK
                                                 : AFSPR_ERR_NOT_DIRECTORY;
}

/* Validates the replay preconditions of every namespace operation. */
static int afspr_validate_namespace_prefix(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view,
    uint64_t *final_next_object_id,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_ident ident;
    uint8_t *workspace = (uint8_t *)scratch->buffer;
    uint8_t *key = workspace + AFSPR_NAMESPACE_KEY_OFFSET;
    uint8_t resolved_name[AFSP_NAME_MAX_UTF8_BYTES];
    uint64_t next_object_id = volume->next_object_id;
    uint32_t slot;

    afspr_ident_from_result(volume, &ident);
    for (slot = 0u; slot < view->valid_records; ++slot) {
        struct afspr_log_record first_record;
        uint64_t first_lba;
        uint16_t record_operation_count;
        uint16_t operation_count;
        uint16_t operation_index;
        int status;

        memset(&first_record, 0, sizeof(first_record));
        status = afspr_read_intent_record(
            ops, &ident, volume, slot, workspace, &first_record, &first_lba,
            diagnostic);

        if (status != AFSPR_OK) {
            return status;
        }
        record_operation_count = first_record.operation_count;
        operation_count = record_operation_count;
        for (operation_index = 0u; operation_index < operation_count;
             ++operation_index) {
            struct afspr_log_record record;
            struct afspr_log_operation decoded;
            struct afspr_namespace_operation operation;
            struct afspr_log_position before;
            struct afspr_directory_entry source;
            int source_created = 0;
            uint64_t lba;
            size_t key_len;

            status = afspr_read_intent_record(
                ops, &ident, volume, slot, workspace, &record, &lba,
                diagnostic);
            if (status != AFSPR_OK) {
                return status;
            }
            if (record.operation_count != record_operation_count ||
                afspr_log_operation_at(&record, operation_index, &decoded) !=
                    AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            afspr_copy_namespace_operation(&decoded, &operation);
            before.slot = slot;
            before.operation = operation_index;
            if (operation.type == 4u || operation.type == 5u) {
                struct afspr_intent_file_state file_state;

                status = afspr_load_intent_file_state_before(
                    ops, scratch, volume, view, before, operation.first,
                    &file_state, diagnostic);
                if (status != AFSPR_OK) {
                    if (status != AFSPR_ERR_NOT_FOUND &&
                        status != AFSPR_ERR_NOT_FILE) {
                        return status;
                    }
                    return afspr_report(
                        diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_INTENT_NAMESPACE,
                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (file_state.size_bytes != operation.third) {
                    return afspr_report(
                        diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_INTENT_NAMESPACE,
                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                continue;
            }
            if (operation.type < 1u || operation.type > 3u) {
                continue;
            }
            status = afspr_namespace_parent_is_directory(
                ops, scratch, volume, operation.first, diagnostic);
            if (status != AFSPR_OK) {
                if (status != AFSPR_ERR_NOT_FOUND &&
                    status != AFSPR_ERR_NOT_DIRECTORY) {
                    return status;
                }
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            status = afspr_name_key(
                &ident, operation.source_name, operation.source_len, key,
                &key_len);
            if (status != AFSPR_OK) {
                return afspr_report(diagnostic, status,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            status = afspr_namespace_resolve_before(
                ops, scratch, volume, view, before, operation.first, key,
                key_len, resolved_name, sizeof(resolved_name), &source,
                &source_created, diagnostic);
            if (operation.type == 1u) {
                if (status != AFSPR_ERR_NOT_FOUND ||
                    operation.third < next_object_id ||
                    operation.third == UINT64_MAX) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_NAMESPACE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                next_object_id = operation.third + 1u;
                continue;
            }
            if (status != AFSPR_OK) {
                if (status != AFSPR_ERR_NOT_FOUND) {
                    return status;
                }
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (source.type_hint != AFSPR_OBJECT_FILE) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (!source_created) {
                struct afspr_object source_object;

                status = afspr_lookup_object(
                    ops, scratch, volume, source.object_id, &source_object,
                    sizeof(source_object), diagnostic,
                    diagnostic == NULL ? 0u : sizeof(*diagnostic));
                if (status != AFSPR_OK) {
                    if (status != AFSPR_ERR_NOT_FOUND) {
                        return status;
                    }
                    return afspr_report(
                        diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_INTENT_NAMESPACE,
                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (source_object.type != AFSPR_OBJECT_FILE) {
                    return afspr_report(
                        diagnostic, AFSPR_ERR_CORRUPT,
                        AFSPR_STAGE_INTENT_NAMESPACE,
                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
            }
            if (operation.type == 2u) {
                continue;
            }
            status = afspr_namespace_parent_is_directory(
                ops, scratch, volume, operation.second, diagnostic);
            if (status != AFSPR_OK) {
                if (status != AFSPR_ERR_NOT_FOUND &&
                    status != AFSPR_ERR_NOT_DIRECTORY) {
                    return status;
                }
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            {
                uint8_t *source_key = workspace +
                                      AFSPR_NAMESPACE_LOWER_OFFSET;
                uint8_t *target_key = workspace +
                                      AFSPR_NAMESPACE_KEY_OFFSET;
                size_t source_key_len;
                size_t target_key_len;
                int same_key;

                status = afspr_name_key(
                    &ident, operation.source_name, operation.source_len,
                    source_key, &source_key_len);
                if (status == AFSPR_OK) {
                    status = afspr_name_key(
                        &ident, operation.target_name,
                        operation.target_len, target_key, &target_key_len);
                }
                if (status != AFSPR_OK) {
                    return afspr_report(diagnostic, status,
                                        AFSPR_STAGE_INTENT_NAMESPACE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                same_key = operation.first == operation.second &&
                           source_key_len == target_key_len &&
                           memcmp(source_key, target_key,
                                  source_key_len) == 0;
                if (!same_key) {
                    struct afspr_directory_entry target;
                    int target_created = 0;

                    status = afspr_namespace_resolve_before(
                        ops, scratch, volume, view, before,
                        operation.second, target_key, target_key_len,
                        resolved_name, sizeof(resolved_name), &target,
                        &target_created, diagnostic);
                    if (status == AFSPR_OK &&
                        (!operation.replace ||
                         target.type_hint != AFSPR_OBJECT_FILE)) {
                        return afspr_report(
                            diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    if (status == AFSPR_OK && !target_created) {
                        struct afspr_object target_object;
                        int object_status = afspr_lookup_object(
                            ops, scratch, volume, target.object_id,
                            &target_object, sizeof(target_object), diagnostic,
                            diagnostic == NULL ? 0u
                                               : sizeof(*diagnostic));

                        if (object_status != AFSPR_OK) {
                            if (object_status != AFSPR_ERR_NOT_FOUND) {
                                return object_status;
                            }
                            return afspr_report(
                                diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_INTENT_NAMESPACE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
                        }
                        if (target_object.type != AFSPR_OBJECT_FILE) {
                            return afspr_report(
                                diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_INTENT_NAMESPACE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
                        }
                    }
                    if (status != AFSPR_OK &&
                        status != AFSPR_ERR_NOT_FOUND) {
                        return status;
                    }
                }
            }
        }
    }
    if (final_next_object_id != NULL) {
        *final_next_object_id = next_object_id;
    }
    return AFSPR_OK;
}

int afspr_intent_directory_cursor_init(
    struct afspr_intent_directory_cursor *cursor, size_t cursor_size)
{
    if (cursor == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    if (cursor_size < sizeof(*cursor)) {
        return AFSPR_ERR_ABI;
    }
    memset(cursor, 0, sizeof(*cursor));
    cursor->abi_version = AFSPR_ABI_VERSION;
    return AFSPR_OK;
}

int afspr_internal_preflight_file_namespace(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, uint64_t source_parent_id,
    const void *source_name, size_t source_name_len,
    uint64_t target_parent_id, const void *target_name,
    size_t target_name_len, int source_must_exist, int inspect_target,
    struct afspr_intent_view *view, int *destination_conflict,
    int *destination_is_file, uint64_t *next_object_id, uint32_t *phase,
    struct afspr_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspr_ident ident;
    struct afspr_log_position end;
    struct afspr_directory_entry source;
    struct afspr_directory_entry target;
    uint8_t source_result_name[AFSP_NAME_MAX_UTF8_BYTES];
    uint8_t target_result_name[AFSP_NAME_MAX_UTF8_BYTES];
    uint8_t *key;
    size_t key_len;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_INTENT_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (phase != NULL) {
        *phase = AFSPR_INTERNAL_NAMESPACE_SCAN;
    }
    if (status != AFSPR_OK) {
        return status;
    }
    if (source_parent_id == 0u || source_name == NULL ||
        source_name_len == 0u ||
        source_name_len > AFSP_NAME_MAX_UTF8_BYTES ||
        (inspect_target != 0 &&
         (target_parent_id == 0u || target_name == NULL ||
          target_name_len == 0u ||
          target_name_len > AFSP_NAME_MAX_UTF8_BYTES)) ||
        view == NULL || destination_conflict == NULL ||
        destination_is_file == NULL || next_object_id == NULL ||
        phase == NULL) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    *destination_conflict = 0;
    *destination_is_file = 0;
    *next_object_id = 0u;
    status = afspr_scan_intent_log(ops, scratch, volume, view, sizeof(*view),
                                   diagnostic, diagnostic_size);
    if (status != AFSPR_OK || view->tail_state == AFSPR_INTENT_TAIL_FULL) {
        return status;
    }
    if ((view->flags & AFSPR_INTENT_VIEW_NAMESPACE) == 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_UNSUPPORTED,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, view->tail_block);
    }
    status = afspr_validate_namespace_prefix(
        ops, scratch, volume, view, next_object_id, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }

    *phase = AFSPR_INTERNAL_NAMESPACE_SOURCE;
    key = (uint8_t *)scratch->buffer + AFSPR_NAMESPACE_KEY_OFFSET;
    status = afspr_name_key(&ident, (const uint8_t *)source_name,
                            source_name_len, key, &key_len);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    end.slot = view->valid_records;
    end.operation = 0u;
    if (source_must_exist == 0) {
        status = afspr_namespace_parent_is_directory(
            ops, scratch, volume, source_parent_id, diagnostic);
        if (status != AFSPR_OK) {
            return status;
        }
    }
    status = afspr_namespace_resolve_before(
        ops, scratch, volume, view, end, source_parent_id, key, key_len,
        source_result_name, sizeof(source_result_name), &source, NULL,
        diagnostic);
    if (source_must_exist == 0) {
        if (status == AFSPR_ERR_NOT_FOUND) {
            return AFSPR_OK;
        }
        if (status == AFSPR_OK) {
            *destination_conflict = 1;
            return AFSPR_OK;
        }
        return status;
    }
    if (status != AFSPR_OK) {
        return status;
    }
    if (source.type_hint != AFSPR_OBJECT_FILE) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FILE,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }

    if (inspect_target == 0) {
        return AFSPR_OK;
    }

    *phase = AFSPR_INTERNAL_NAMESPACE_TARGET;
    status = afspr_name_key(&ident, (const uint8_t *)target_name,
                            target_name_len, key, &key_len);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_namespace_resolve_before(
        ops, scratch, volume, view, end, target_parent_id, key, key_len,
        target_result_name, sizeof(target_result_name), &target, NULL,
        diagnostic);
    if (status == AFSPR_ERR_NOT_FOUND) {
        return AFSPR_OK;
    }
    if (status != AFSPR_OK) {
        return status;
    }
    *destination_is_file = target.type_hint == AFSPR_OBJECT_FILE;
    *destination_conflict =
        source.object_id != target.object_id ||
        source.parent_id != target.parent_id ||
        source.name_len != target.name_len ||
        memcmp(source.name, target.name, source.name_len) != 0;
    return AFSPR_OK;
}

int afspr_internal_validate_object_watermark(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume, uint64_t next_object_id,
    struct afspr_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspr_ident ident;
    struct afspr_tree_spec spec;
    uint8_t search[8];
    uint8_t found_key[8];
    uint8_t value[8];
    size_t value_len;
    uint64_t leaf_lba;
    int found;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_MIN_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (next_object_id == 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    memset(search, UINT8_MAX, sizeof(search));
    spec.kind = AFSPR_TREE_KIND_OBJECT_MAP;
    spec.owner = 0u;
    spec.max_generation = volume->generation;
    status = afspr_tree_lookup_fixed(
        ops, scratch, &ident, volume->object_map_block, &spec, search, 1,
        found_key, value, sizeof(value), &value_len, &found, &leaf_lba,
        diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    if (!found || value_len != sizeof(value) ||
        afspr_get_be64(found_key) >= next_object_id) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, leaf_lba);
    }
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        AFSPR_NO_CHECKPOINT_SLOT, leaf_lba);
}

int afspr_lookup_intent_directory_entry(
    const struct afspr_block_ops *ops,
    const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t directory_id,
    const void *name, size_t name_len, void *name_buffer,
    size_t name_capacity, struct afspr_directory_entry *entry,
    size_t entry_size, struct afspr_diagnostic *diagnostic,
    size_t diagnostic_size)
{
    struct afspr_ident ident;
    struct afspr_log_position end;
    uint8_t stable_name[AFSP_NAME_MAX_UTF8_BYTES];
    uint8_t *key;
    size_t key_len;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_INTENT_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (directory_id == 0u || name == NULL ||
        name_len > sizeof(stable_name) || entry == NULL ||
        entry_size < sizeof(*entry)) {
        return afspr_report(
            diagnostic,
            entry != NULL && entry_size < sizeof(*entry) ? AFSPR_ERR_ABI
                                                          : AFSPR_ERR_INVALID_ARGUMENT,
            AFSPR_STAGE_ARGUMENTS, AFSPR_NO_CHECKPOINT_SLOT,
            AFSPR_NO_BLOCK);
    }
    memmove(stable_name, name, name_len);
    status = afspr_validate_intent_view(
        ops, scratch, volume, view, AFSPR_INTENT_VIEW_NAMESPACE,
        diagnostic, diagnostic_size);
    if (status != AFSPR_OK) {
        return status;
    }
    status = afspr_validate_namespace_prefix(ops, scratch, volume, view,
                                             NULL, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    key = (uint8_t *)scratch->buffer + AFSPR_NAMESPACE_KEY_OFFSET;
    status = afspr_name_key(&ident, stable_name, name_len, key,
                            &key_len);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    end.slot = view->valid_records;
    end.operation = 0u;
    status = afspr_namespace_resolve_before(
        ops, scratch, volume, view, end, directory_id, key, key_len,
        name_buffer, name_capacity, entry, NULL, diagnostic);
    if (status == AFSPR_ERR_BUFFER_TOO_SMALL ||
        status == AFSPR_ERR_INVALID_ARGUMENT) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (status != AFSPR_OK) {
        return status;
    }
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
}

static int afspr_find_log_namespace_candidate(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t directory_id,
    const uint8_t *after, size_t after_len, uint8_t *best,
    size_t *best_len, int *found, struct afspr_diagnostic *diagnostic)
{
    struct afspr_ident ident;
    uint8_t *record_buffer = (uint8_t *)scratch->buffer;
    uint8_t *candidate = record_buffer + AFSPR_NAMESPACE_UPPER_OFFSET;
    uint32_t slot;

    afspr_ident_from_result(volume, &ident);
    *found = 0;
    *best_len = 0u;
    for (slot = 0u; slot < view->valid_records; ++slot) {
        struct afspr_log_record record;
        uint64_t lba;
        uint16_t operation_index;
        int status = afspr_read_intent_record(
            ops, &ident, volume, slot, record_buffer, &record, &lba,
            diagnostic);

        if (status != AFSPR_OK) {
            return status;
        }
        for (operation_index = 0u;
             operation_index < record.operation_count; ++operation_index) {
            struct afspr_log_operation operation;
            const uint8_t *name;
            size_t name_len;
            size_t candidate_len;
            uint64_t parent_id;
            int compared;

            if (afspr_log_operation_at(&record, operation_index,
                                       &operation) != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (operation.type == 1u) {
                parent_id = operation.first;
                name = operation.source_name;
                name_len = operation.source_len;
            } else if (operation.type == 3u) {
                parent_id = operation.second;
                name = operation.target_name;
                name_len = operation.target_len;
            } else {
                continue;
            }
            if (parent_id != directory_id) {
                continue;
            }
            status = afspr_name_key(&ident, name, name_len, candidate,
                                    &candidate_len);
            if (status != AFSPR_OK) {
                return afspr_report(diagnostic, status,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            compared = after_len == 0u
                           ? 1
                           : afspr_bytes_compare(candidate, candidate_len,
                                                 after, after_len);
            if (compared <= 0 ||
                (*found && afspr_bytes_compare(candidate, candidate_len,
                                               best, *best_len) >= 0)) {
                continue;
            }
            memcpy(best, candidate, candidate_len);
            *best_len = candidate_len;
            *found = 1;
        }
    }
    return AFSPR_OK;
}

static int afspr_cursor_matches_view(
    const struct afspr_intent_directory_cursor *cursor,
    const struct afspr_intent_view *view, uint64_t directory_id)
{
    return cursor->directory_id == directory_id &&
           cursor->base_generation == view->base_generation &&
           cursor->last_sequence == view->last_sequence &&
           cursor->valid_records == view->valid_records &&
           cursor->valid_operations == view->valid_operations;
}

int afspr_intent_directory_next(
    const struct afspr_block_ops *ops,
    const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t directory_id,
    struct afspr_intent_directory_cursor *cursor, size_t cursor_size,
    void *name_buffer, size_t name_capacity,
    struct afspr_directory_entry *entry, size_t entry_size,
    struct afspr_diagnostic *diagnostic, size_t diagnostic_size)
{
    struct afspr_ident ident;
    struct afspr_object directory;
    uint8_t *workspace;
    uint8_t *base_key;
    uint8_t *log_key;
    uint8_t *temporary_name;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_INTENT_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (directory_id == 0u || cursor == NULL || entry == NULL ||
        cursor_size < sizeof(*cursor) || entry_size < sizeof(*entry)) {
        return afspr_report(
            diagnostic,
            (cursor != NULL && cursor_size < sizeof(*cursor)) ||
                    (entry != NULL && entry_size < sizeof(*entry))
                ? AFSPR_ERR_ABI
                : AFSPR_ERR_INVALID_ARGUMENT,
            AFSPR_STAGE_ARGUMENTS, AFSPR_NO_CHECKPOINT_SLOT,
            AFSPR_NO_BLOCK);
    }
    if (cursor->abi_version != AFSPR_ABI_VERSION ||
        (cursor->flags & ~(AFSPR_INTENT_CURSOR_STARTED |
                           AFSPR_INTENT_CURSOR_PENDING)) != 0u ||
        cursor->key_len > AFSPR_TREE_MAX_KEY ||
        ((cursor->flags & AFSPR_INTENT_CURSOR_PENDING) != 0u &&
         cursor->key_len == 0u)) {
        return afspr_report(diagnostic, AFSPR_ERR_ABI,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_validate_intent_view(
        ops, scratch, volume, view, AFSPR_INTENT_VIEW_NAMESPACE,
        diagnostic, diagnostic_size);
    if (status != AFSPR_OK) {
        return status;
    }
    status = afspr_validate_namespace_prefix(ops, scratch, volume, view,
                                             NULL, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    if ((cursor->flags & AFSPR_INTENT_CURSOR_STARTED) == 0u) {
        if (cursor->flags != 0u || cursor->directory_id != 0u ||
            cursor->base_generation != 0u || cursor->last_sequence != 0u ||
            cursor->valid_records != 0u ||
            cursor->valid_operations != 0u || cursor->base_ordinal != 0u ||
            cursor->key_len != 0u) {
            return afspr_report(diagnostic, AFSPR_ERR_ABI,
                                AFSPR_STAGE_ARGUMENTS,
                                AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
        }
        cursor->flags = AFSPR_INTENT_CURSOR_STARTED;
        cursor->directory_id = directory_id;
        cursor->base_generation = view->base_generation;
        cursor->last_sequence = view->last_sequence;
        cursor->valid_records = view->valid_records;
        cursor->valid_operations = view->valid_operations;
    } else if (!afspr_cursor_matches_view(cursor, view, directory_id)) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_lookup_object(
        ops, scratch, volume, directory_id, &directory, sizeof(directory),
        diagnostic, diagnostic == NULL ? 0u : sizeof(*diagnostic));
    if (status != AFSPR_OK) {
        return status;
    }
    if (directory.type != AFSPR_OBJECT_DIRECTORY) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_DIRECTORY,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }

    workspace = (uint8_t *)scratch->buffer;
    base_key = workspace + AFSPR_NAMESPACE_LOWER_OFFSET;
    log_key = workspace + AFSPR_NAMESPACE_KEY_OFFSET;
    temporary_name = workspace + AFSPR_NAMESPACE_NAME_OFFSET;
    for (;;) {
        if ((cursor->flags & AFSPR_INTENT_CURSOR_PENDING) != 0u) {
            struct afspr_log_position end;
            struct afspr_directory_entry resolved;

            end.slot = view->valid_records;
            end.operation = 0u;
            status = afspr_namespace_resolve_before(
                ops, scratch, volume, view, end, directory_id, cursor->key,
                cursor->key_len, temporary_name,
                AFSP_NAME_MAX_UTF8_BYTES, &resolved, NULL, diagnostic);
            if (status == AFSPR_ERR_NOT_FOUND) {
                cursor->flags &= ~AFSPR_INTENT_CURSOR_PENDING;
                continue;
            }
            if (status != AFSPR_OK) {
                return status;
            }
            status = afspr_namespace_result(
                directory_id, resolved.object_id, resolved.type_hint,
                temporary_name, resolved.name_len, name_buffer,
                name_capacity, entry);
            if (status == AFSPR_OK) {
                cursor->flags &= ~AFSPR_INTENT_CURSOR_PENDING;
            }
            return afspr_report(
                diagnostic, status,
                status == AFSPR_OK ? AFSPR_STAGE_COMPLETE
                                   : AFSPR_STAGE_INTENT_NAMESPACE,
                AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
        }
        {
            struct afspr_directory_entry base_entry;
            size_t base_key_len = 0u;
            size_t log_key_len = 0u;
            int have_base = 0;
            int have_log = 0;

            status = afspr_directory_entry_at_internal(
                ops, scratch, volume, &directory, cursor->base_ordinal,
                temporary_name, AFSP_NAME_MAX_UTF8_BYTES, &base_entry,
                sizeof(base_entry), NULL, base_key, AFSPR_TREE_MAX_KEY,
                &base_key_len, diagnostic, diagnostic_size);
            if (status == AFSPR_OK) {
                have_base = 1;
                if (cursor->key_len != 0u &&
                    afspr_bytes_compare(base_key, base_key_len, cursor->key,
                                        cursor->key_len) <= 0) {
                    ++cursor->base_ordinal;
                    continue;
                }
            } else if (status != AFSPR_ERR_NOT_FOUND) {
                return status;
            }
            status = afspr_find_log_namespace_candidate(
                ops, scratch, volume, view, directory_id, cursor->key,
                cursor->key_len, log_key, &log_key_len, &have_log,
                diagnostic);
            if (status != AFSPR_OK) {
                return status;
            }
            if (!have_base && !have_log) {
                return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                                    AFSPR_STAGE_COMPLETE,
                                    AFSPR_NO_CHECKPOINT_SLOT,
                                    AFSPR_NO_BLOCK);
            }
            if (have_base &&
                (!have_log ||
                 afspr_bytes_compare(base_key, base_key_len, log_key,
                                     log_key_len) <= 0)) {
                memcpy(cursor->key, base_key, base_key_len);
                cursor->key_len = (uint16_t)base_key_len;
                ++cursor->base_ordinal;
            } else {
                memcpy(cursor->key, log_key, log_key_len);
                cursor->key_len = (uint16_t)log_key_len;
            }
            cursor->flags |= AFSPR_INTENT_CURSOR_PENDING;
        }
    }
}

static int afspr_load_intent_file_state_before(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, struct afspr_log_position boundary,
    uint64_t object_id, struct afspr_intent_file_state *state,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_ident ident;
    uint8_t *workspace = (uint8_t *)scratch->buffer;
    uint8_t *key = workspace + AFSPR_NAMESPACE_KEY_OFFSET;
    uint8_t resolved_name[AFSP_NAME_MAX_UTF8_BYTES];
    uint32_t slot_count;
    uint32_t slot;
    int status;

    memset(state, 0, sizeof(*state));
    if (boundary.slot > view->valid_records ||
        (boundary.slot == view->valid_records && boundary.operation != 0u)) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_lookup_object(ops, scratch, volume, object_id,
                                 &state->committed,
                                 sizeof(state->committed), diagnostic,
                                 sizeof(*diagnostic));
    if (status == AFSPR_OK) {
        if (state->committed.type != AFSPR_OBJECT_FILE) {
            return afspr_report(diagnostic, AFSPR_ERR_NOT_FILE,
                                AFSPR_STAGE_OBJECT_DECODE,
                                AFSPR_NO_CHECKPOINT_SLOT,
                                AFSPR_NO_BLOCK);
        }
        state->exists = 1;
        state->has_committed = 1;
        state->link_count = state->committed.link_count;
        state->size_bytes = state->committed.size_bytes;
    } else if (status != AFSPR_ERR_NOT_FOUND) {
        return status;
    }
    afspr_ident_from_result(volume, &ident);
    slot_count = boundary.slot < view->valid_records
                     ? boundary.slot + 1u
                     : view->valid_records;

    for (slot = 0u; slot < slot_count; ++slot) {
        struct afspr_log_record first_record;
        uint64_t first_lba;
        uint16_t record_operation_count;
        uint16_t operation_count;
        uint16_t operation_index;

        memset(&first_record, 0, sizeof(first_record));
        status = afspr_read_intent_record(
            ops, &ident, volume, slot, workspace, &first_record,
            &first_lba, diagnostic);
        if (status != AFSPR_OK) {
            return status;
        }
        record_operation_count = first_record.operation_count;
        operation_count = record_operation_count;
        if (slot == boundary.slot) {
            if (boundary.operation > operation_count) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_NAMESPACE,
                                    AFSPR_NO_CHECKPOINT_SLOT, first_lba);
            }
            operation_count = boundary.operation;
        }
        for (operation_index = 0u; operation_index < operation_count;
             ++operation_index) {
            struct afspr_log_record record;
            struct afspr_log_operation decoded;
            struct afspr_namespace_operation operation;
            struct afspr_log_position before;
            uint64_t lba;

            status = afspr_read_intent_record(
                ops, &ident, volume, slot, workspace, &record, &lba,
                diagnostic);
            if (status != AFSPR_OK) {
                return status;
            }
            if (record.operation_count != record_operation_count ||
                afspr_log_operation_at(&record, operation_index, &decoded) !=
                    AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            afspr_copy_namespace_operation(&decoded, &operation);
            before.slot = slot;
            before.operation = operation_index;
            if (operation.type == 1u && operation.third == object_id) {
                if (state->exists) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                state->exists = 1;
                state->link_count = 1u;
                state->size_bytes = operation.fourth;
            } else if (operation.type == 2u || operation.type == 3u) {
                struct afspr_directory_entry removed;
                size_t key_len;
                int removes_link = operation.type == 2u;

                if (operation.type == 2u) {
                    status = afspr_name_key(
                        &ident, operation.source_name,
                        operation.source_len, key, &key_len);
                } else {
                    uint8_t *source_key = workspace +
                                          AFSPR_NAMESPACE_LOWER_OFFSET;
                    size_t source_key_len;

                    status = afspr_name_key(
                        &ident, operation.source_name,
                        operation.source_len, source_key,
                        &source_key_len);
                    if (status == AFSPR_OK) {
                        status = afspr_name_key(
                            &ident, operation.target_name,
                            operation.target_len, key, &key_len);
                    }
                    removes_link = status == AFSPR_OK &&
                                   !(operation.first == operation.second &&
                                     source_key_len == key_len &&
                                     memcmp(source_key, key,
                                            key_len) == 0);
                }
                if (status != AFSPR_OK) {
                    return afspr_report(diagnostic, status,
                                        AFSPR_STAGE_INTENT_NAMESPACE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (!removes_link) {
                    continue;
                }
                status = afspr_namespace_resolve_before(
                    ops, scratch, volume, view, before,
                    operation.type == 2u ? operation.first
                                         : operation.second,
                    key, key_len, resolved_name, sizeof(resolved_name),
                    &removed, NULL, diagnostic);
                if (operation.type == 3u &&
                    status == AFSPR_ERR_NOT_FOUND) {
                    continue;
                }
                if (status != AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_NAMESPACE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (removed.object_id == object_id) {
                    if (!state->exists || state->link_count == 0u) {
                        return afspr_report(
                            diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_NAMESPACE,
                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    --state->link_count;
                    state->exists = state->link_count != 0u;
                }
            } else if ((operation.type == 4u || operation.type == 5u) &&
                       operation.first == object_id) {
                if (!state->exists || operation.third != state->size_bytes) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                state->size_bytes = operation.fourth;
            }
        }
    }
    if (!state->exists) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                            AFSPR_STAGE_INTENT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT,
                            AFSPR_NO_BLOCK);
    }
    return AFSPR_OK;
}

static int afspr_load_intent_file_state(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t object_id,
    struct afspr_intent_file_state *state,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_log_position end;
    int status = afspr_validate_namespace_prefix(ops, scratch, volume, view,
                                                  NULL, diagnostic);

    if (status != AFSPR_OK) {
        return status;
    }
    end.slot = view->valid_records;
    end.operation = 0u;
    return afspr_load_intent_file_state_before(
        ops, scratch, volume, view, end, object_id, state, diagnostic);
}

int afspr_internal_intent_file_size(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t object_id,
    uint64_t *size_bytes, struct afspr_diagnostic *diagnostic,
    size_t diagnostic_size)
{
    struct afspr_intent_file_state state;
    int status;

    if (diagnostic != NULL && diagnostic_size < sizeof(*diagnostic)) {
        return AFSPR_ERR_ABI;
    }
    if (ops == NULL || scratch == NULL || volume == NULL || view == NULL ||
        size_bytes == NULL || object_id == 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_load_intent_file_state(ops, scratch, volume, view,
                                          object_id, &state, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    *size_bytes = state.size_bytes;
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
}

static uint32_t afspr_region_count(const struct afspr_ident *ident)
{
    uint64_t count = ident->total_blocks / ident->region_size;

    if (ident->total_blocks % ident->region_size != 0u) {
        ++count;
    }
    return (uint32_t)count;
}

static uint32_t afspr_bitmap_page_count(const struct afspr_ident *ident,
                                        uint32_t region)
{
    uint32_t valid = afspr_region_valid_blocks(ident, region);

    return (valid + AFSPR_BITMAP_PAGE_BLOCKS - 1u) /
           AFSPR_BITMAP_PAGE_BLOCKS;
}

static uint32_t afspr_bitmap_page_valid_blocks(
    const struct afspr_ident *ident, uint32_t region, uint32_t page)
{
    uint32_t valid = afspr_region_valid_blocks(ident, region);
    uint32_t first = page * AFSPR_BITMAP_PAGE_BLOCKS;
    uint32_t left = valid > first ? valid - first : 0u;

    return left < AFSPR_BITMAP_PAGE_BLOCKS ? left
                                           : AFSPR_BITMAP_PAGE_BLOCKS;
}

static int afspr_lookup_allocation_region(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident,
    const struct afspr_probe_result *volume, uint32_t region,
    uint8_t *descriptor_slot, uint32_t *free_blocks,
    uint64_t *descriptor_generation, struct afspr_diagnostic *diagnostic)
{
    struct afspr_tree_spec spec;
    uint8_t search[4];
    uint8_t value[AFSPR_ALLOCATION_VALUE_SIZE];
    size_t value_len;
    uint64_t leaf_lba = AFSPR_NO_BLOCK;
    int found;
    int status;

    search[0] = (uint8_t)(region >> 24);
    search[1] = (uint8_t)(region >> 16);
    search[2] = (uint8_t)(region >> 8);
    search[3] = (uint8_t)region;
    spec.kind = AFSPR_TREE_KIND_ALLOCATION_ROOT;
    spec.owner = 0u;
    spec.max_generation = volume->generation;
    status = afspr_tree_lookup_variable(
        ops, scratch, ident, volume->allocation_root_block, &spec, search,
        sizeof(search), value, sizeof(value), &value_len, &found, &leaf_lba,
        diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    if (!found || value_len != sizeof(value) || value[1] != 0u ||
        value[2] != 0u || value[3] != 0u ||
        value[0] >= AFSPR_DESCRIPTOR_SLOTS) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, leaf_lba);
    }
    *descriptor_slot = value[0];
    *free_blocks = afspr_get_le32(value + 4u);
    *descriptor_generation = afspr_get_le64(value + 8u);
    if (*free_blocks > afspr_region_valid_blocks(ident, region) ||
        *descriptor_generation == 0u ||
        *descriptor_generation > volume->generation) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_TREE_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, leaf_lba);
    }
    return AFSPR_OK;
}

static int afspr_load_allocation_region(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident,
    const struct afspr_probe_result *volume, uint32_t region,
    struct afspr_allocation_region_view *decoded,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_header header;
    uint8_t descriptor_slot;
    uint32_t recorded_free;
    uint64_t recorded_generation;
    uint64_t base = (uint64_t)region * ident->region_size;
    uint64_t head = region == 0u ? AFSPR_BOOTSTRAP_BLOCKS : 0u;
    uint64_t lba;
    const uint8_t *payload;
    uint64_t free_sum = 0u;
    uint32_t expected_pages = afspr_bitmap_page_count(ident, region);
    uint32_t index;
    int status = afspr_lookup_allocation_region(
        ops, scratch, ident, volume, region, &descriptor_slot,
        &recorded_free, &recorded_generation, diagnostic);

    if (status != AFSPR_OK) {
        return status;
    }
    if (expected_pages == 0u || expected_pages > AFSPR_MAX_BITMAP_PAGES) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ALLOCATION_DESCRIPTOR_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    lba = base + head + descriptor_slot;
    status = afspr_read_one(ops, lba, (uint8_t *)scratch->buffer);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_ALLOCATION_DESCRIPTOR_READ,
                            AFSPR_NO_CHECKPOINT_SLOT, lba);
    }
    status = afspr_verify_header((const uint8_t *)scratch->buffer,
                                 ops->block_size,
                                 AFSPR_BLOCK_TYPE_REGION_DESCRIPTOR,
                                 &header);
    if (status != AFSPR_OK || header.flags != 0u ||
        header.owner != region || header.generation != recorded_generation ||
        header.payload_len !=
            AFSPR_REGION_DESCRIPTOR_FIXED + expected_pages * 16u) {
        return afspr_report(
            diagnostic,
            status == AFSPR_ERR_UNSUPPORTED ? status : AFSPR_ERR_CORRUPT,
            AFSPR_STAGE_ALLOCATION_DESCRIPTOR_DECODE,
            AFSPR_NO_CHECKPOINT_SLOT, lba);
    }
    payload = (const uint8_t *)scratch->buffer + AFSPR_HEADER_SIZE;
    if (afspr_get_le32(payload) != region ||
        afspr_get_le32(payload + 4u) !=
            afspr_region_valid_blocks(ident, region) ||
        afspr_get_le32(payload + 8u) != recorded_free ||
        afspr_get_le32(payload + 12u) != expected_pages) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ALLOCATION_DESCRIPTOR_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, lba);
    }
    memset(decoded, 0, sizeof(*decoded));
    decoded->valid_blocks = afspr_get_le32(payload + 4u);
    decoded->free_blocks = recorded_free;
    decoded->page_count = expected_pages;
    decoded->descriptor_generation = recorded_generation;
    for (index = 0u; index < expected_pages; ++index) {
        const uint8_t *binding =
            payload + AFSPR_REGION_DESCRIPTOR_FIXED + index * 16u;
        uint32_t page_valid =
            afspr_bitmap_page_valid_blocks(ident, region, index);

        decoded->pages[index].slot = binding[0];
        decoded->pages[index].free_blocks = afspr_get_le32(binding + 4u);
        decoded->pages[index].generation = afspr_get_le64(binding + 8u);
        if (decoded->pages[index].slot >= AFSPR_BITMAP_SLOTS ||
            decoded->pages[index].free_blocks > page_valid ||
            decoded->pages[index].generation == 0u ||
            decoded->pages[index].generation > recorded_generation) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_ALLOCATION_DESCRIPTOR_DECODE,
                                AFSPR_NO_CHECKPOINT_SLOT, lba);
        }
        free_sum += decoded->pages[index].free_blocks;
    }
    if (free_sum != recorded_free) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ALLOCATION_DESCRIPTOR_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, lba);
    }
    return AFSPR_OK;
}

static int afspr_load_allocation_bitmap(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident, uint32_t region, uint32_t page,
    const struct afspr_bitmap_binding_view *binding, const uint8_t **bits,
    uint32_t *valid_blocks, uint64_t *bitmap_lba,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_header header;
    uint64_t base = (uint64_t)region * ident->region_size;
    uint64_t head = region == 0u ? AFSPR_BOOTSTRAP_BLOCKS : 0u;
    uint8_t *buffer =
        (uint8_t *)scratch->buffer + (size_t)ops->block_size;
    const uint8_t *payload;
    uint32_t expected_valid =
        afspr_bitmap_page_valid_blocks(ident, region, page);
    uint32_t expected_first = page * AFSPR_BITMAP_PAGE_BLOCKS;
    size_t byte_len = ((size_t)expected_valid + 7u) / 8u;
    uint32_t free_count = 0u;
    uint32_t index;
    int status;

    *bitmap_lba = base + head + AFSPR_DESCRIPTOR_SLOTS +
                  (uint64_t)page * AFSPR_BITMAP_SLOTS + binding->slot;
    status = afspr_read_one(ops, *bitmap_lba, buffer);
    if (status != AFSPR_OK) {
        return afspr_report(diagnostic, status,
                            AFSPR_STAGE_ALLOCATION_BITMAP_READ,
                            AFSPR_NO_CHECKPOINT_SLOT, *bitmap_lba);
    }
    status = afspr_verify_header(buffer, ops->block_size,
                                 AFSPR_BLOCK_TYPE_BITMAP, &header);
    if (status != AFSPR_OK || header.flags != 0u ||
        header.owner != region || header.generation != binding->generation ||
        header.payload_len != AFSPR_BITMAP_FIXED + byte_len) {
        return afspr_report(
            diagnostic,
            status == AFSPR_ERR_UNSUPPORTED ? status : AFSPR_ERR_CORRUPT,
            AFSPR_STAGE_ALLOCATION_BITMAP_DECODE,
            AFSPR_NO_CHECKPOINT_SLOT, *bitmap_lba);
    }
    payload = buffer + AFSPR_HEADER_SIZE;
    if (afspr_get_le32(payload) != region ||
        afspr_get_le32(payload + 4u) != page ||
        afspr_get_le32(payload + 8u) != expected_first ||
        afspr_get_le32(payload + 12u) != expected_valid ||
        expected_valid == 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ALLOCATION_BITMAP_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, *bitmap_lba);
    }
    *bits = payload + AFSPR_BITMAP_FIXED;
    *valid_blocks = expected_valid;
    for (index = 0u; index < expected_valid; ++index) {
        if (((*bits)[index / 8u] & (uint8_t)(1u << (index % 8u))) == 0u) {
            ++free_count;
        }
    }
    for (index = expected_valid; index < byte_len * 8u; ++index) {
        if (((*bits)[index / 8u] & (uint8_t)(1u << (index % 8u))) == 0u) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_ALLOCATION_BITMAP_DECODE,
                                AFSPR_NO_CHECKPOINT_SLOT, *bitmap_lba);
        }
    }
    if (free_count != binding->free_blocks) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_ALLOCATION_BITMAP_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, *bitmap_lba);
    }
    return AFSPR_OK;
}

static int afspr_validate_log_extent_is_free(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident,
    const struct afspr_probe_result *volume, uint64_t start, uint32_t blocks,
    struct afspr_diagnostic *diagnostic)
{
    struct afspr_allocation_region_view allocation;
    uint32_t region = (uint32_t)(start / ident->region_size);
    uint64_t base = (uint64_t)region * ident->region_size;
    uint32_t first = (uint32_t)(start - base);
    uint32_t final = first + blocks;
    uint32_t page = first / AFSPR_BITMAP_PAGE_BLOCKS;
    int status = afspr_load_allocation_region(
        ops, scratch, ident, volume, region, &allocation, diagnostic);

    if (status != AFSPR_OK) {
        return status;
    }
    while (first < final) {
        const uint8_t *bits;
        uint32_t valid;
        uint64_t bitmap_lba;
        uint32_t page_first = page * AFSPR_BITMAP_PAGE_BLOCKS;
        uint32_t local = first - page_first;
        uint32_t page_end;

        status = afspr_load_allocation_bitmap(
            ops, scratch, ident, region, page, &allocation.pages[page],
            &bits, &valid, &bitmap_lba, diagnostic);
        if (status != AFSPR_OK) {
            return status;
        }
        page_end = page_first + valid;
        while (first < final && first < page_end) {
            if ((bits[local / 8u] &
                 (uint8_t)(1u << (local % 8u))) != 0u) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, base + first);
            }
            ++first;
            ++local;
        }
        ++page;
    }
    return AFSPR_OK;
}

static int afspr_validate_log_reservations(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t *reserved_blocks,
    struct afspr_diagnostic *diagnostic)
{
    uint8_t *record_buffer = (uint8_t *)scratch->buffer;
    uint64_t total = 0u;
    uint32_t slot;

    for (slot = 0u; slot < view->valid_records; ++slot) {
        struct afspr_log_record first_record;
        uint64_t lba;
        uint16_t operation_index;
        int status = afspr_read_intent_record(
            ops, ident, volume, slot, record_buffer, &first_record, &lba,
            diagnostic);

        if (status != AFSPR_OK) {
            return status;
        }
        for (operation_index = 0u;
             operation_index < first_record.operation_count;
             ++operation_index) {
            struct afspr_log_record record;
            struct afspr_log_operation operation;
            uint16_t extent_count;
            uint16_t extent_index;

            status = afspr_read_intent_record(
                ops, ident, volume, slot, record_buffer, &record, &lba,
                diagnostic);
            if (status != AFSPR_OK) {
                return status;
            }
            if (afspr_log_operation_at(&record, operation_index,
                                       &operation) != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            extent_count = operation.extent_count;
            for (extent_index = 0u; extent_index < extent_count;
                 ++extent_index) {
                uint64_t start;
                uint32_t blocks;

                status = afspr_read_intent_record(
                    ops, ident, volume, slot, record_buffer, &record, &lba,
                    diagnostic);
                if (status != AFSPR_OK) {
                    return status;
                }
                if (afspr_log_operation_at(&record, operation_index,
                                           &operation) != AFSPR_OK ||
                    afspr_log_extent_at(&operation, extent_index, &start,
                                        &blocks) != AFSPR_OK ||
                    !afspr_log_extent_valid(ident, start, blocks)) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (UINT64_MAX - total < blocks) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                total += blocks;
                status = afspr_validate_log_extent_is_free(
                    ops, scratch, ident, volume, start, blocks, diagnostic);
                if (status != AFSPR_OK) {
                    return status;
                }
            }
        }
    }
    *reserved_blocks = total;
    return AFSPR_OK;
}

static int afspr_logged_extent_covering(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t candidate,
    uint64_t *covered_until, struct afspr_diagnostic *diagnostic)
{
    uint8_t *record_buffer = (uint8_t *)scratch->buffer;
    uint32_t slot;

    *covered_until = candidate;
    for (slot = 0u; slot < view->valid_records; ++slot) {
        struct afspr_log_record record;
        uint64_t lba;
        uint16_t operation_index;
        int status = afspr_read_intent_record(
            ops, ident, volume, slot, record_buffer, &record, &lba,
            diagnostic);

        if (status != AFSPR_OK) {
            return status;
        }
        for (operation_index = 0u;
             operation_index < record.operation_count; ++operation_index) {
            struct afspr_log_operation operation;
            uint16_t extent_index;

            if (afspr_log_operation_at(&record, operation_index,
                                       &operation) != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            for (extent_index = 0u; extent_index < operation.extent_count;
                 ++extent_index) {
                uint64_t start;
                uint32_t blocks;

                if (afspr_log_extent_at(&operation, extent_index, &start,
                                        &blocks) != AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (candidate >= start && candidate - start < blocks) {
                    *covered_until = start + blocks;
                    return AFSPR_OK;
                }
            }
        }
    }
    return AFSPR_OK;
}

int afspr_internal_find_log_data_block(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t free_block_floor,
    uint64_t *data_block, struct afspr_diagnostic *diagnostic,
    size_t diagnostic_size)
{
    struct afspr_ident ident;
    uint64_t reserved_blocks;
    uint32_t region;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_INTENT_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (view == NULL || data_block == NULL ||
        free_block_floor > volume->total_blocks) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    *data_block = AFSPR_NO_BLOCK;
    if (view->abi_version != AFSPR_ABI_VERSION ||
        view->base_generation != volume->generation ||
        view->valid_records > volume->log_slots ||
        (view->flags & AFSPR_INTENT_VIEW_FILE_DATA) == 0u ||
        view->last_sequence != view->valid_records) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, view->tail_block);
    }
    status = afspr_validate_log_reservations(
        ops, scratch, &ident, volume, view, &reserved_blocks, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    if (reserved_blocks > volume->free_blocks_total) {
        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                            AFSPR_STAGE_INTENT_DECODE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    if (volume->free_blocks_total - reserved_blocks <= free_block_floor) {
        return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                            AFSPR_STAGE_COMPLETE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }

    for (region = 0u; region < afspr_region_count(&ident); ++region) {
        struct afspr_allocation_region_view allocation;
        uint64_t base = (uint64_t)region * ident.region_size;
        uint32_t page;

        status = afspr_load_allocation_region(
            ops, scratch, &ident, volume, region, &allocation, diagnostic);
        if (status != AFSPR_OK) {
            return status;
        }
        if (allocation.free_blocks == 0u) {
            continue;
        }
        for (page = 0u; page < allocation.page_count; ++page) {
            const uint8_t *bits;
            uint32_t valid;
            uint64_t bitmap_lba;
            uint32_t page_first = page * AFSPR_BITMAP_PAGE_BLOCKS;
            uint32_t local = 0u;

            if (allocation.pages[page].free_blocks == 0u) {
                continue;
            }
            status = afspr_load_allocation_bitmap(
                ops, scratch, &ident, region, page,
                &allocation.pages[page], &bits, &valid, &bitmap_lba,
                diagnostic);
            if (status != AFSPR_OK) {
                return status;
            }
            while (local < valid) {
                uint32_t region_index = page_first + local;
                uint64_t candidate = base + region_index;

                if ((bits[local / 8u] &
                     (uint8_t)(1u << (local % 8u))) == 0u &&
                    afspr_is_allocatable(&ident, candidate)) {
                    uint64_t covered_until;

                    status = afspr_logged_extent_covering(
                        ops, scratch, &ident, volume, view, candidate,
                        &covered_until, diagnostic);
                    if (status != AFSPR_OK) {
                        return status;
                    }
                    if (covered_until == candidate) {
                        *data_block = candidate;
                        return afspr_report(
                            diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                            AFSPR_NO_CHECKPOINT_SLOT, candidate);
                    }
                    if (covered_until > base + page_first + valid) {
                        local = valid;
                    } else {
                        local = (uint32_t)(covered_until - base -
                                           page_first);
                    }
                    continue;
                }
                ++local;
            }
        }
    }
    return afspr_report(diagnostic, AFSPR_ERR_NOT_FOUND,
                        AFSPR_STAGE_COMPLETE, AFSPR_NO_CHECKPOINT_SLOT,
                        AFSPR_NO_BLOCK);
}

int afspr_intent_file_size(const struct afspr_block_ops *ops,
                           const struct afspr_scratch *scratch,
                           const struct afspr_probe_result *volume,
                           const struct afspr_intent_view *view,
                           uint64_t object_id, uint64_t *size_bytes,
                           struct afspr_diagnostic *diagnostic,
                           size_t diagnostic_size)
{
    struct afspr_intent_file_state state;
    struct afspr_ident ident;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_INTENT_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (size_bytes == NULL || object_id == 0u) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    status = afspr_validate_intent_view(
        ops, scratch, volume, view, AFSPR_INTENT_VIEW_FILE_DATA,
        diagnostic, diagnostic_size);
    if (status != AFSPR_OK) {
        return status;
    }
    status = afspr_load_intent_file_state(ops, scratch, volume, view,
                                          object_id, &state, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    *size_bytes = state.size_bytes;
    return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                        AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
}

static int afspr_log_operation_physical(
    const struct afspr_log_operation *operation, uint64_t relative,
    uint64_t *physical)
{
    uint16_t extent_index;

    for (extent_index = 0u; extent_index < operation->extent_count;
         ++extent_index) {
        uint64_t start;
        uint32_t blocks;

        if (afspr_log_extent_at(operation, extent_index, &start, &blocks) !=
            AFSPR_OK) {
            return AFSPR_ERR_CORRUPT;
        }
        if (relative < blocks) {
            if (UINT64_MAX - start < relative) {
                return AFSPR_ERR_CORRUPT;
            }
            *physical = start + relative;
            return AFSPR_OK;
        }
        relative -= blocks;
    }
    return AFSPR_ERR_NOT_FOUND;
}

static int afspr_committed_file_mapping(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_ident *ident,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_file_state *state, uint64_t logical_block,
    int *mapped, uint64_t *physical, struct afspr_diagnostic *diagnostic)
{
    *mapped = 0;
    *physical = 0u;
    if (!state->has_committed) {
        return AFSPR_OK;
    }
    if ((state->committed.flags & AFSPR_OBJECT_FLAG_EXTENT_TREE) != 0u) {
        struct afspr_extent extent;
        int status = afspr_extent_for_block(
            ops, scratch, ident, volume, &state->committed, logical_block,
            &extent, mapped, diagnostic);

        if (status != AFSPR_OK) {
            return status;
        }
        if (*mapped && (extent.flags & AFSPR_EXTENT_UNWRITTEN) == 0u) {
            *physical = extent.physical_start + logical_block -
                        extent.logical_start;
        } else {
            *mapped = 0;
        }
    } else if (logical_block < state->committed.data_blocks) {
        if (UINT64_MAX - state->committed.data_root < logical_block) {
            return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                AFSPR_STAGE_DATA_READ,
                                AFSPR_NO_CHECKPOINT_SLOT,
                                state->committed.data_root);
        }
        *physical = state->committed.data_root + logical_block;
        *mapped = 1;
    }
    return AFSPR_OK;
}

static int afspr_intent_file_mapping(
    const struct afspr_block_ops *ops, const struct afspr_scratch *scratch,
    const struct afspr_probe_result *volume,
    const struct afspr_intent_view *view, uint64_t object_id,
    const struct afspr_intent_file_state *state, uint64_t logical_block,
    int *mapped, uint64_t *physical, struct afspr_diagnostic *diagnostic)
{
    struct afspr_ident ident;
    uint64_t current_size = state->has_committed
                                ? state->committed.size_bytes
                                : 0u;
    int exists = state->has_committed;
    uint32_t slot;
    int status;

    afspr_ident_from_result(volume, &ident);
    status = afspr_committed_file_mapping(
        ops, scratch, &ident, volume, state, logical_block, mapped, physical,
        diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }

    for (slot = 0u; slot < view->valid_records; ++slot) {
        struct afspr_log_record record;
        uint64_t lba;
        uint16_t operation_index;

        status = afspr_read_intent_record(
            ops, &ident, volume, slot, (uint8_t *)scratch->buffer, &record,
            &lba, diagnostic);
        if (status != AFSPR_OK) {
            return status;
        }
        for (operation_index = 0u;
             operation_index < record.operation_count; ++operation_index) {
            struct afspr_log_operation operation;
            uint64_t old_blocks;
            uint64_t operation_blocks;

            if (afspr_log_operation_at(&record, operation_index,
                                       &operation) != AFSPR_OK) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            if (operation.type == 1u && operation.third == object_id) {
                if (exists) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                exists = 1;
                current_size = operation.fourth;
                *mapped = 0;
                if (afspr_log_operation_physical(
                        &operation, logical_block, physical) == AFSPR_OK) {
                    *mapped = 1;
                }
                continue;
            }
            if ((operation.type != 4u && operation.type != 5u) ||
                operation.first != object_id) {
                continue;
            }
            if (!exists || operation.third != current_size) {
                return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                    AFSPR_STAGE_INTENT_DECODE,
                                    AFSPR_NO_CHECKPOINT_SLOT, lba);
            }
            old_blocks = afspr_ceil_div_u64(current_size,
                                             ops->block_size);
            if (operation.fourth > current_size &&
                logical_block >= old_blocks) {
                *mapped = 0;
            }
            if (operation.type == 4u) {
                uint64_t relative;

                if (afspr_validate_log_extents(&operation,
                                               &operation_blocks) !=
                    AFSPR_OK) {
                    return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                        AFSPR_STAGE_INTENT_DECODE,
                                        AFSPR_NO_CHECKPOINT_SLOT, lba);
                }
                if (logical_block >= operation.second &&
                    logical_block - operation.second < operation_blocks) {
                    relative = logical_block - operation.second;
                    if (afspr_log_operation_physical(
                            &operation, relative, physical) != AFSPR_OK) {
                        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                            AFSPR_STAGE_INTENT_DECODE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    *mapped = 1;
                }
            } else {
                uint64_t new_blocks = afspr_ceil_div_u64(
                    operation.fourth, ops->block_size);

                if (logical_block >= new_blocks) {
                    *mapped = 0;
                }
                if (operation.extent_count == 1u &&
                    logical_block == operation.second) {
                    if (afspr_log_operation_physical(
                            &operation, 0u, physical) != AFSPR_OK) {
                        return afspr_report(diagnostic, AFSPR_ERR_CORRUPT,
                                            AFSPR_STAGE_INTENT_DECODE,
                                            AFSPR_NO_CHECKPOINT_SLOT, lba);
                    }
                    *mapped = 1;
                }
            }
            current_size = operation.fourth;
        }
    }
    return exists ? AFSPR_OK : AFSPR_ERR_NOT_FOUND;
}

int afspr_read_intent_file(const struct afspr_block_ops *ops,
                           const struct afspr_scratch *scratch,
                           const struct afspr_probe_result *volume,
                           const struct afspr_intent_view *view,
                           uint64_t object_id, uint64_t offset,
                           void *destination, size_t destination_size,
                           size_t *bytes_read,
                           struct afspr_diagnostic *diagnostic,
                           size_t diagnostic_size)
{
    struct afspr_intent_file_state state;
    struct afspr_ident ident;
    uint8_t *output = (uint8_t *)destination;
    uint64_t count;
    uint64_t end;
    uint64_t logical_block;
    uint64_t final_block;
    int status = afspr_prepare_operation(
        ops, scratch, volume, AFSPR_INTENT_SCRATCH_SIZE, &ident, diagnostic,
        diagnostic_size);

    if (status != AFSPR_OK) {
        return status;
    }
    if (bytes_read == NULL || object_id == 0u ||
        (destination == NULL && destination_size != 0u)) {
        return afspr_report(diagnostic, AFSPR_ERR_INVALID_ARGUMENT,
                            AFSPR_STAGE_ARGUMENTS,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    *bytes_read = 0u;
    status = afspr_validate_intent_view(
        ops, scratch, volume, view, AFSPR_INTENT_VIEW_FILE_DATA,
        diagnostic, diagnostic_size);
    if (status != AFSPR_OK) {
        return status;
    }
    status = afspr_load_intent_file_state(ops, scratch, volume, view,
                                          object_id, &state, diagnostic);
    if (status != AFSPR_OK) {
        return status;
    }
    if (destination_size == 0u || offset >= state.size_bytes) {
        return afspr_report(diagnostic, AFSPR_OK, AFSPR_STAGE_COMPLETE,
                            AFSPR_NO_CHECKPOINT_SLOT, AFSPR_NO_BLOCK);
    }
    count = state.size_bytes - offset;
    if (count > destination_size) {
        count = destination_size;
    }
    end = offset + count;
    logical_block = offset / ops->block_size;
    final_block = (end - 1u) / ops->block_size;

    for (; logical_block <= final_block; ++logical_block) {
        uint64_t physical;
        uint64_t block_start = logical_block * ops->block_size;
        uint64_t copy_start = offset > block_start ? offset : block_start;
        uint64_t block_end = block_start + ops->block_size;
        uint64_t copy_end = end < block_end ? end : block_end;
        size_t source_offset = (size_t)(copy_start - block_start);
        size_t target_offset = (size_t)(copy_start - offset);
        size_t copy_size = (size_t)(copy_end - copy_start);
        int mapped;

        status = afspr_intent_file_mapping(
            ops, scratch, volume, view, object_id, &state, logical_block,
            &mapped, &physical, diagnostic);
        if (status != AFSPR_OK) {
            return status;
        }
        if (mapped) {
            status = afspr_read_one(ops, physical,
                                    (uint8_t *)scratch->buffer);
            if (status != AFSPR_OK) {
                return afspr_report(diagnostic, status,
                                    AFSPR_STAGE_INTENT_DATA,
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
    case AFSPR_STAGE_INTENT_READ:
        return "intent-log read";
    case AFSPR_STAGE_INTENT_DECODE:
        return "intent-log decode";
    case AFSPR_STAGE_INTENT_DATA:
        return "intent-log data";
    case AFSPR_STAGE_INTENT_NAMESPACE:
        return "intent-log namespace";
    case AFSPR_STAGE_ALLOCATION_DESCRIPTOR_READ:
        return "allocation descriptor read";
    case AFSPR_STAGE_ALLOCATION_DESCRIPTOR_DECODE:
        return "allocation descriptor decode";
    case AFSPR_STAGE_ALLOCATION_BITMAP_READ:
        return "allocation bitmap read";
    case AFSPR_STAGE_ALLOCATION_BITMAP_DECODE:
        return "allocation bitmap decode";
    default:
        return "unknown probe stage";
    }
}

const char *afspr_intent_tail_string(uint32_t tail_state)
{
    switch (tail_state) {
    case AFSPR_INTENT_TAIL_NONE:
        return "none";
    case AFSPR_INTENT_TAIL_INVALID:
        return "invalid or empty record";
    case AFSPR_INTENT_TAIL_STALE:
        return "stale record";
    case AFSPR_INTENT_TAIL_SEQUENCE:
        return "non-contiguous sequence";
    case AFSPR_INTENT_TAIL_CONTENT:
        return "invalid or torn content";
    case AFSPR_INTENT_TAIL_FULL:
        return "all slots valid";
    default:
        return "unknown intent-log tail";
    }
}

/* Reclaim queue (ADR-036). */

void afspr_reclaim_entry_at(const uint8_t *area, uint32_t index,
                            struct afspr_reclaim_entry *entry)
{
    const uint8_t *p = area + (size_t)index * AFSPR_RECLAIM_ENTRY_SIZE;
    entry->start = afspr_get_le64(p);
    entry->blocks = afspr_get_le32(p + 8u);
    entry->retire_generation = afspr_get_le64(p + 12u);
    entry->reserved32 = 0u;
}

void afspr_reclaim_ref_at(const uint8_t *area, uint32_t index,
                          struct afspr_reclaim_ref *ref)
{
    const uint8_t *p = area + (size_t)index * AFSPR_RECLAIM_REF_SIZE;
    ref->lba = afspr_get_le64(p);
    ref->count = afspr_get_le32(p + 8u);
    ref->reserved32 = 0u;
}

/* A run has blocks, a retire generation, and an end that fits 64 bits. */
static int afspr_reclaim_entries_ok(const uint8_t *area, uint32_t count)
{
    uint32_t i;
    for (i = 0u; i < count; ++i) {
        struct afspr_reclaim_entry entry;
        afspr_reclaim_entry_at(area, i, &entry);
        if (entry.blocks == 0u || entry.retire_generation == 0u ||
            entry.start > UINT64_MAX - (uint64_t)entry.blocks) {
            return 0;
        }
    }
    return 1;
}

/* Exact admission around a payload (ADR-110): no header flags, no owner.
 * The zero tail is the header verification's (ADR-112). */
static int afspr_reclaim_envelope_ok(const struct afspr_header *header)
{
    return header->flags == 0u && header->owner == 0u;
}

static int afspr_all_zero(const uint8_t *bytes, size_t size)
{
    size_t i;
    for (i = 0u; i < size; ++i) {
        if (bytes[i] != 0u) return 0;
    }
    return 1;
}

static int afspr_reclaim_refs_ok(const uint8_t *area, uint32_t count,
                                 uint32_t cap)
{
    uint32_t i;
    for (i = 0u; i < count; ++i) {
        struct afspr_reclaim_ref ref;
        afspr_reclaim_ref_at(area, i, &ref);
        if (ref.count == 0u || ref.count > cap) return 0;
    }
    return 1;
}

int afspr_decode_reclaim_root(const void *input, size_t block_size,
                              struct afspr_reclaim_root *root_out,
                              uint64_t *generation)
{
    const uint8_t *block = (const uint8_t *)input;
    const uint8_t *p;
    struct afspr_header header;
    struct afspr_reclaim_root root;
    struct afspr_reclaim_ref first;
    size_t areas;
    int status;

    if (input == NULL || root_out == NULL || generation == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size,
                                 AFSPR_BLOCK_TYPE_RECLAIM_ROOT, &header);
    if (status != AFSPR_OK) return status;
    p = block + AFSPR_HEADER_SIZE;
    if (!afspr_reclaim_envelope_ok(&header) ||
        header.payload_len < AFSPR_RECLAIM_ROOT_FIXED ||
        afspr_get_le32(p) != AFSPR_RECLAIM_ROOT_VERSION ||
        afspr_get_le32(p + 4u) != 0u || afspr_get_le16(p + 50u) != 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    memset(&root, 0, sizeof(root));
    root.pending_blocks = afspr_get_le64(p + 8u);
    root.appended_blocks_total = afspr_get_le64(p + 16u);
    root.reclaimed_blocks_total = afspr_get_le64(p + 24u);
    root.head_segment_offset = afspr_get_le32(p + 32u);
    root.head_entry_offset = afspr_get_le32(p + 36u);
    root.head_block_offset = afspr_get_le32(p + 40u);
    root.inline_capacity = afspr_get_le16(p + 44u);
    root.segment_capacity = afspr_get_le16(p + 46u);
    root.table_capacity = afspr_get_le16(p + 48u);
    root.table_count = afspr_get_le32(p + 52u);
    root.segment_count = afspr_get_le32(p + 56u);
    root.inline_count = afspr_get_le32(p + 60u);
    /* Bounds first: the capacities place the three areas. */
    if (root.inline_capacity == 0u || root.segment_capacity == 0u ||
        root.table_capacity == 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    areas = (size_t)AFSPR_RECLAIM_ROOT_FIXED +
            ((size_t)root.table_capacity + (size_t)root.segment_capacity) *
                AFSPR_RECLAIM_REF_SIZE +
            (size_t)root.inline_capacity * AFSPR_RECLAIM_ENTRY_SIZE;
    if (areas > block_size - AFSPR_HEADER_SIZE ||
        areas != (size_t)header.payload_len ||
        root.table_count > root.table_capacity ||
        root.segment_count > root.segment_capacity ||
        root.inline_count > root.inline_capacity) {
        return AFSPR_ERR_CORRUPT;
    }
    root.tables = p + AFSPR_RECLAIM_ROOT_FIXED;
    root.segments =
        root.tables + (size_t)root.table_capacity * AFSPR_RECLAIM_REF_SIZE;
    root.inline_entries =
        root.segments + (size_t)root.segment_capacity * AFSPR_RECLAIM_REF_SIZE;
    if (!afspr_reclaim_refs_ok(root.tables, root.table_count,
                               AFSPR_RECLAIM_TABLE_REF_CAP) ||
        !afspr_reclaim_refs_ok(root.segments, root.segment_count,
                               AFSPR_RECLAIM_SEGMENT_ENTRY_CAP) ||
        !afspr_reclaim_entries_ok(root.inline_entries, root.inline_count)) {
        return AFSPR_ERR_CORRUPT;
    }
    /* Unused slots are zero: a rewrite would drop anything else. */
    if (!afspr_all_zero(root.tables + (size_t)root.table_count *
                                          AFSPR_RECLAIM_REF_SIZE,
                        (size_t)(root.table_capacity - root.table_count) *
                            AFSPR_RECLAIM_REF_SIZE) ||
        !afspr_all_zero(root.segments + (size_t)root.segment_count *
                                            AFSPR_RECLAIM_REF_SIZE,
                        (size_t)(root.segment_capacity - root.segment_count) *
                            AFSPR_RECLAIM_REF_SIZE) ||
        !afspr_all_zero(root.inline_entries + (size_t)root.inline_count *
                                                  AFSPR_RECLAIM_ENTRY_SIZE,
                        (size_t)(root.inline_capacity - root.inline_count) *
                            AFSPR_RECLAIM_ENTRY_SIZE)) {
        return AFSPR_ERR_CORRUPT;
    }
    /* The cursor names sealed blocks only, in FIFO order. */
    if (root.table_count == 0u) {
        if (root.head_segment_offset != 0u) return AFSPR_ERR_CORRUPT;
        if (root.segment_count == 0u) {
            if (root.head_entry_offset != 0u || root.head_block_offset != 0u) {
                return AFSPR_ERR_CORRUPT;
            }
        } else {
            afspr_reclaim_ref_at(root.segments, 0u, &first);
            if (root.head_entry_offset >= first.count) return AFSPR_ERR_CORRUPT;
        }
    } else {
        afspr_reclaim_ref_at(root.tables, 0u, &first);
        if (root.head_segment_offset >= first.count) return AFSPR_ERR_CORRUPT;
    }
    if (root.appended_blocks_total < root.reclaimed_blocks_total ||
        root.appended_blocks_total - root.reclaimed_blocks_total !=
            root.pending_blocks) {
        return AFSPR_ERR_CORRUPT;
    }
    *root_out = root;
    *generation = header.generation;
    return AFSPR_OK;
}

static int afspr_decode_reclaim_sealed(const void *input, size_t block_size,
                                       uint32_t block_type, size_t item_size,
                                       uint32_t cap, const uint8_t **area,
                                       uint32_t *count_out,
                                       uint64_t *generation)
{
    const uint8_t *block = (const uint8_t *)input;
    const uint8_t *p;
    struct afspr_header header;
    uint32_t count;
    int status;

    if (input == NULL || area == NULL || count_out == NULL ||
        generation == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    status = afspr_verify_header(block, block_size, block_type, &header);
    if (status != AFSPR_OK) return status;
    p = block + AFSPR_HEADER_SIZE;
    if (!afspr_reclaim_envelope_ok(&header) ||
        header.payload_len < AFSPR_RECLAIM_SEALED_FIXED ||
        afspr_get_le32(p + 4u) != 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    count = afspr_get_le32(p);
    if (count == 0u || count > cap ||
        (size_t)header.payload_len !=
            AFSPR_RECLAIM_SEALED_FIXED + (size_t)count * item_size) {
        return AFSPR_ERR_CORRUPT;
    }
    p += AFSPR_RECLAIM_SEALED_FIXED;
    if (item_size == AFSPR_RECLAIM_ENTRY_SIZE
            ? !afspr_reclaim_entries_ok(p, count)
            : !afspr_reclaim_refs_ok(p, count,
                                     AFSPR_RECLAIM_SEGMENT_ENTRY_CAP)) {
        return AFSPR_ERR_CORRUPT;
    }
    *area = p;
    *count_out = count;
    *generation = header.generation;
    return AFSPR_OK;
}

int afspr_decode_reclaim_segment(const void *block, size_t block_size,
                                 const uint8_t **entries, uint32_t *count,
                                 uint64_t *generation)
{
    return afspr_decode_reclaim_sealed(
        block, block_size, AFSPR_BLOCK_TYPE_RECLAIM_SEGMENT,
        AFSPR_RECLAIM_ENTRY_SIZE, AFSPR_RECLAIM_SEGMENT_ENTRY_CAP, entries,
        count, generation);
}

int afspr_decode_reclaim_table(const void *block, size_t block_size,
                               const uint8_t **refs, uint32_t *count,
                               uint64_t *generation)
{
    return afspr_decode_reclaim_sealed(
        block, block_size, AFSPR_BLOCK_TYPE_RECLAIM_TABLE,
        AFSPR_RECLAIM_REF_SIZE, AFSPR_RECLAIM_TABLE_REF_CAP, refs, count,
        generation);
}

/* Snapshot records (ADR-072). */

/* A value is 32 bytes whose unused part, from used on, is zero. */
static int afspr_snapshot_value_ok(const uint8_t *value, size_t value_size,
                                   size_t used)
{
    size_t i;
    if (value == NULL || value_size != AFSPR_SNAPSHOT_VALUE_SIZE) return 0;
    for (i = used; i < AFSPR_SNAPSHOT_VALUE_SIZE; ++i) {
        if (value[i] != 0u) return 0;
    }
    return 1;
}

int afspr_decode_snapshot_key(const uint8_t *key, size_t key_size,
                              uint64_t *id)
{
    uint64_t value = 0u;
    size_t i;
    if (key == NULL || id == NULL) return AFSPR_ERR_INVALID_ARGUMENT;
    if (key_size != AFSPR_SNAPSHOT_KEY_SIZE) return AFSPR_ERR_CORRUPT;
    for (i = 0u; i < AFSPR_SNAPSHOT_KEY_SIZE; ++i) {
        value = (value << 8) | key[i];
    }
    *id = value;
    return AFSPR_OK;
}

int afspr_decode_snapshot_registry_state(const uint8_t *value,
                                         size_t value_size,
                                         uint64_t *next_id)
{
    if (next_id == NULL) return AFSPR_ERR_INVALID_ARGUMENT;
    if (!afspr_snapshot_value_ok(value, value_size, 8u) ||
        afspr_get_le64(value) == 0u) {
        return AFSPR_ERR_CORRUPT;
    }
    *next_id = afspr_get_le64(value);
    return AFSPR_OK;
}

int afspr_decode_snapshot_record(const uint8_t *value, size_t value_size,
                                 uint64_t max_generation,
                                 uint64_t total_blocks,
                                 struct afspr_snapshot_record *record)
{
    struct afspr_snapshot_record decoded;
    if (record == NULL) return AFSPR_ERR_INVALID_ARGUMENT;
    if (!afspr_snapshot_value_ok(value, value_size, 24u)) {
        return AFSPR_ERR_CORRUPT;
    }
    decoded.generation = afspr_get_le64(value);
    decoded.committed_tx_id = afspr_get_le64(value + 8u);
    decoded.object_map_root = afspr_get_le64(value + 16u);
    if (decoded.generation == 0u || decoded.generation > max_generation ||
        decoded.committed_tx_id == 0u ||
        decoded.committed_tx_id > decoded.generation ||
        decoded.object_map_root == 0u ||
        decoded.object_map_root >= total_blocks) {
        return AFSPR_ERR_CORRUPT;
    }
    *record = decoded;
    return AFSPR_OK;
}

int afspr_decode_snapshot_lifetime(const uint8_t *value, size_t value_size,
                                   uint64_t start, uint64_t max_generation,
                                   uint64_t total_blocks,
                                   struct afspr_snapshot_lifetime *lifetime)
{
    struct afspr_snapshot_lifetime decoded;
    if (lifetime == NULL) return AFSPR_ERR_INVALID_ARGUMENT;
    if (!afspr_snapshot_value_ok(value, value_size, 24u)) {
        return AFSPR_ERR_CORRUPT;
    }
    decoded.blocks = afspr_get_le64(value);
    decoded.birth = afspr_get_le64(value + 8u);
    decoded.retirement = afspr_get_le64(value + 16u);
    if (start == 0u || decoded.blocks == 0u ||
        start > UINT64_MAX - decoded.blocks ||
        start + decoded.blocks > total_blocks || decoded.birth == 0u ||
        decoded.birth > max_generation ||
        (decoded.retirement != 0u &&
         (decoded.retirement <= decoded.birth ||
          decoded.retirement > max_generation))) {
        return AFSPR_ERR_CORRUPT;
    }
    *lifetime = decoded;
    return AFSPR_OK;
}

int afspr_decode_snapshot_ledger_state(const uint8_t *value,
                                       size_t value_size,
                                       uint64_t total_blocks,
                                       uint64_t *scan_position,
                                       uint64_t *retained_blocks)
{
    uint64_t position, retained;
    if (scan_position == NULL || retained_blocks == NULL) {
        return AFSPR_ERR_INVALID_ARGUMENT;
    }
    if (!afspr_snapshot_value_ok(value, value_size, 16u)) {
        return AFSPR_ERR_CORRUPT;
    }
    position = afspr_get_le64(value);
    retained = afspr_get_le64(value + 8u);
    if (total_blocks == 0u || position >= total_blocks ||
        retained > total_blocks) {
        return AFSPR_ERR_CORRUPT;
    }
    *scan_position = position;
    *retained_blocks = retained;
    return AFSPR_OK;
}
