//! Encode/decode round-trips and corruption rejection for every prototype
//! block type. Every decoder must reject a flipped byte via its CRC and must
//! never panic on arbitrary input (`docs/21-security-and-corruption.md`).

use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::crc32c::CHECKSUM_CRC32C;
use afsplus_format::dir::{comparison_key, DirBlock, DirEntry};
use afsplus_format::geometry::Geometry;
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::ident::{
    FeatureFlags, Identification, NameKeyAlgorithm, IDENT_VERSION_FEATURES, IDENT_VERSION_LEGACY,
    INCOMPAT_INTENT_LOG, INCOMPAT_INTENT_LOG_DATA_UPDATES, UNICODE_VERSION_16_0_0,
};
use afsplus_format::intent_log::{LogOp, LogRecord};
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::omap::ObjectMap;
use afsplus_format::reclaim::{
    ReclaimCaps, ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable, SegmentRef, TableRef,
};
use afsplus_format::region::{BitmapBinding, RegionDescriptor};
use afsplus_format::retired::RetiredList;
use afsplus_format::tree::{
    child_value, key_u64, ChildRef, TreeItem, TreeKind, TreeNode, MAX_TREE_LEVEL,
};
use afsplus_format::{
    le, FormatError, Timespec, DEFAULT_BLOCK_SHIFT, DEFAULT_BLOCK_SIZE, OBJECT_ROOT,
};

const BS: usize = DEFAULT_BLOCK_SIZE;

fn ts() -> Timespec {
    Timespec {
        seconds: 1_780_000_000,
        nanoseconds: 123_456_789,
    }
}

fn sample_ident() -> Identification {
    Identification {
        uuid: [7u8; 16],
        block_shift: DEFAULT_BLOCK_SHIFT,
        checksum_algorithm: CHECKSUM_CRC32C,
        region_size: 256,
        log_slots: 8,
        features: FeatureFlags {
            incompat: INCOMPAT_INTENT_LOG,
            ..FeatureFlags::default()
        },
        name_key_algorithm: NameKeyAlgorithm::UnicodeNfc,
        unicode_version: UNICODE_VERSION_16_0_0,
        total_blocks: 1024,
        checkpoint_slots: [1, 2],
        metadata_start: 9,
        label: "Test Volume".into(),
    }
}

fn sample_checkpoint() -> Checkpoint {
    Checkpoint {
        uuid: [7u8; 16],
        generation: 5,
        root_object_id: OBJECT_ROOT,
        object_map_block: 10,
        allocation_root_block: 12,
        reclaim_root_block: 11,
        next_object_id: 20,
        committed_tx_id: 5,
        free_blocks_total: 800,
        flags: 0,
        shared_extent_root_block: 0,
    }
}

fn sample_record() -> ObjectRecord {
    ObjectRecord {
        object_id: 17,
        object_type: ObjectType::File,
        flags: 0,
        link_count: 1,
        size_bytes: 5000,
        allocated_bytes: 8192,
        created: ts(),
        modified: ts(),
        changed: ts(),
        protection: 0,
        content_generation: 5,
        data_root: 100,
        data_blocks: 2,
    }
}

fn sample_dir() -> DirBlock {
    let mut dir = DirBlock::new(OBJECT_ROOT);
    for name in ["beta.txt", "alpha.txt", "Émoji-☂.rs"] {
        dir.insert(DirEntry {
            key: comparison_key(name.as_bytes()),
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: 17,
        })
        .unwrap();
    }
    dir
}

fn sample_bitmap() -> BitmapPage {
    let mut page = BitmapPage::all_free(3, 0, 0, 100);
    for index in [0, 1, 2, 50, 99] {
        page.set_allocated(index, true);
    }
    page
}

fn sample_region_descriptor() -> RegionDescriptor {
    RegionDescriptor {
        region: 3,
        valid_blocks: 100,
        free_blocks: 95,
        pages: vec![BitmapBinding {
            slot: 1,
            free_blocks: 95,
            generation: 5,
        }],
    }
}

fn sample_tree_leaf() -> TreeNode {
    TreeNode {
        kind: TreeKind::ObjectMap,
        owner: 0,
        level: 0,
        subtree_items: 3,
        leftmost_child: 0,
        leftmost_items: 0,
        items: [1u64, 17, u64::MAX]
            .into_iter()
            .enumerate()
            .map(|(index, id)| TreeItem {
                key: key_u64(id).to_vec(),
                value: (100 + index as u64).to_le_bytes().to_vec(),
            })
            .collect(),
    }
}

fn sample_retired() -> RetiredList {
    let mut list = RetiredList::default();
    list.insert(42, 4).unwrap();
    list.insert(7, 5).unwrap();
    list.insert(100, 5).unwrap();
    list
}

#[test]
fn identification_roundtrip() {
    let ident = sample_ident();
    let block = ident.encode(BS).unwrap();
    assert_eq!(Identification::decode(&block).unwrap(), ident);
}

#[test]
fn intent_log_data_update_feature_requires_the_base_log() {
    let mut ident = sample_ident();
    ident.features.incompat = INCOMPAT_INTENT_LOG_DATA_UPDATES;
    assert!(ident.encode(BS).is_err());
    ident.features.incompat = INCOMPAT_INTENT_LOG | INCOMPAT_INTENT_LOG_DATA_UPDATES;
    assert_eq!(
        Identification::decode(&ident.encode(BS).unwrap()).unwrap(),
        ident
    );
}

#[test]
fn legacy_identification_derives_the_intent_log_feature() {
    const LEGACY_PAYLOAD_LEN: usize = 137;
    let ident = sample_ident();
    let mut block = ident.encode(BS).unwrap();
    le::put_u32(
        &mut block[HEADER_SIZE + 12..HEADER_SIZE + 16],
        IDENT_VERSION_LEGACY,
    );
    block[HEADER_SIZE + LEGACY_PAYLOAD_LEN..].fill(0);
    BlockHeader {
        block_type: block_type::IDENTIFICATION,
        flags: 0,
        owner: 0,
        generation: 0,
        payload_len: LEGACY_PAYLOAD_LEN as u32,
    }
    .seal(&mut block);

    let decoded = Identification::decode(&block).unwrap();
    assert_eq!(decoded.log_slots, ident.log_slots);
    assert_eq!(decoded.features.incompat, INCOMPAT_INTENT_LOG);
    assert_eq!(decoded.name_key_algorithm, NameKeyAlgorithm::LegacyIdentity);
    assert_eq!(decoded.unicode_version, [0, 0, 0]);
}

#[test]
fn version_two_identification_preserves_features_and_uses_identity_keys() {
    const FEATURE_PAYLOAD_LEN: usize = 161;
    let ident = sample_ident();
    let mut block = ident.encode(BS).unwrap();
    le::put_u32(
        &mut block[HEADER_SIZE + 12..HEADER_SIZE + 16],
        IDENT_VERSION_FEATURES,
    );
    block[HEADER_SIZE + FEATURE_PAYLOAD_LEN..].fill(0);
    BlockHeader {
        block_type: block_type::IDENTIFICATION,
        flags: 0,
        owner: 0,
        generation: 0,
        payload_len: FEATURE_PAYLOAD_LEN as u32,
    }
    .seal(&mut block);

    let decoded = Identification::decode(&block).unwrap();
    assert_eq!(decoded.features, ident.features);
    assert_eq!(decoded.name_key_algorithm, NameKeyAlgorithm::LegacyIdentity);
    assert_eq!(decoded.unicode_version, [0, 0, 0]);
}

#[test]
fn identification_rejects_unknown_name_key_algorithm_and_unicode_version() {
    let ident = sample_ident();
    let mut unknown_algorithm = ident.encode(BS).unwrap();
    unknown_algorithm[HEADER_SIZE + 161] = 0xff;
    BlockHeader {
        block_type: block_type::IDENTIFICATION,
        flags: 0,
        owner: 0,
        generation: 0,
        payload_len: 165,
    }
    .seal(&mut unknown_algorithm);
    assert!(matches!(
        Identification::decode(&unknown_algorithm),
        Err(FormatError::Invalid(
            "unsupported directory comparison-key algorithm"
        ))
    ));

    let mut unknown_unicode = ident.encode(BS).unwrap();
    unknown_unicode[HEADER_SIZE + 162..HEADER_SIZE + 165].copy_from_slice(&[15, 1, 0]);
    BlockHeader {
        block_type: block_type::IDENTIFICATION,
        flags: 0,
        owner: 0,
        generation: 0,
        payload_len: 165,
    }
    .seal(&mut unknown_unicode);
    assert!(matches!(
        Identification::decode(&unknown_unicode),
        Err(FormatError::Invalid(
            "unsupported directory Unicode table version"
        ))
    ));
}

#[test]
fn checkpoint_roundtrip_and_uuid_binding() {
    let checkpoint = sample_checkpoint();
    let block = checkpoint.encode(BS).unwrap();
    assert_eq!(Checkpoint::decode(&block, &[7u8; 16]).unwrap(), checkpoint);
    // A checkpoint from another volume must never be accepted.
    assert!(Checkpoint::decode(&block, &[8u8; 16]).is_err());
}

#[test]
fn checkpoint_rejects_every_reserved_field() {
    let checkpoint = sample_checkpoint();

    for (flags, owner) in [(1, 0), (0, 1)] {
        let mut block = checkpoint.encode(BS).unwrap();
        BlockHeader {
            block_type: block_type::CHECKPOINT,
            flags,
            owner,
            generation: checkpoint.generation,
            payload_len: 96,
        }
        .seal(&mut block);
        assert!(matches!(
            Checkpoint::decode(&block, &checkpoint.uuid),
            Err(FormatError::Invalid(
                "checkpoint header reserved fields are nonzero"
            ))
        ));
    }

    let mut payload_flags = checkpoint.clone();
    payload_flags.flags = 1;
    assert!(matches!(
        Checkpoint::decode(&payload_flags.encode(BS).unwrap(), &checkpoint.uuid),
        Err(FormatError::Invalid(
            "checkpoint payload reserved flags are nonzero"
        ))
    ));
}

#[test]
fn checkpoint_structural_validation() {
    let geo = Geometry {
        block_size: BS,
        total_blocks: 1024,
        region_size: 256,
    };
    sample_checkpoint().validate_structural(&geo).unwrap();

    // The allocation root is mandatory (ADR-061); zero is unencodable and
    // structurally invalid (LBA 0 is the identification block).
    let mut ckpt = sample_checkpoint();
    ckpt.allocation_root_block = 0;
    assert!(ckpt.encode(BS).is_err());
    assert!(ckpt.validate_structural(&geo).is_err());

    // Object map inside the reserved area.
    let mut ckpt = sample_checkpoint();
    ckpt.object_map_block = 4;
    assert!(ckpt.validate_structural(&geo).is_err());

    // Allocation root out of bounds.
    let mut ckpt = sample_checkpoint();
    ckpt.allocation_root_block = geo.total_blocks;
    assert!(ckpt.validate_structural(&geo).is_err());

    // Free total exceeding the volume.
    let mut ckpt = sample_checkpoint();
    ckpt.free_blocks_total = geo.total_blocks + 1;
    assert!(ckpt.validate_structural(&geo).is_err());
}

#[test]
fn checkpoint_shared_extent_root_roundtrip() {
    let geo = Geometry {
        block_size: BS,
        total_blocks: 1024,
        region_size: 256,
    };
    // Zero root: no shared-extent tree on this volume (ADR-061).
    let checkpoint = sample_checkpoint();
    let decoded = Checkpoint::decode(&checkpoint.encode(BS).unwrap(), &checkpoint.uuid).unwrap();
    assert_eq!(decoded, checkpoint);
    assert_eq!(decoded.shared_extent_root_block, 0);

    // Nonzero root survives the roundtrip and validates when allocatable.
    let mut shared = sample_checkpoint();
    shared.shared_extent_root_block = 13;
    shared.validate_structural(&geo).unwrap();
    let decoded = Checkpoint::decode(&shared.encode(BS).unwrap(), &shared.uuid).unwrap();
    assert_eq!(decoded, shared);

    // A root outside allocatable bounds is structurally invalid.
    let mut out_of_bounds = sample_checkpoint();
    out_of_bounds.shared_extent_root_block = geo.total_blocks;
    assert!(out_of_bounds.validate_structural(&geo).is_err());
    let mut reserved = sample_checkpoint();
    reserved.shared_extent_root_block = 1;
    assert!(reserved.validate_structural(&geo).is_err());
}

#[test]
fn object_record_roundtrip() {
    let record = sample_record();
    let block = record.encode(BS, 5).unwrap();
    assert_eq!(ObjectRecord::decode(&block).unwrap(), record);
}

#[test]
fn object_record_rejects_zero_link_count_and_bad_extents() {
    let mut record = sample_record();
    record.link_count = 0;
    assert!(matches!(record.encode(BS, 5), Err(FormatError::Invalid(_))));

    // Size larger than the extent capacity.
    let mut record = sample_record();
    record.size_bytes = 3 * BS as u64;
    assert!(record.encode(BS, 5).is_err());

    // Empty file with a dangling data pointer.
    let mut record = sample_record();
    record.data_blocks = 0;
    record.size_bytes = 0;
    assert!(record.encode(BS, 5).is_err());

    // Absurd extent length is capped.
    let mut record = sample_record();
    record.data_blocks = u64::MAX;
    assert!(record.encode(BS, 5).is_err());

    // A short extent whose exclusive end wraps the block address space must
    // also be rejected before any caller can construct an overflowing range.
    let mut record = sample_record();
    record.data_root = u64::MAX;
    record.data_blocks = 1;
    record.size_bytes = BS as u64;
    record.allocated_bytes = BS as u64;
    assert!(matches!(
        record.encode(BS, 5),
        Err(FormatError::Overflow(_))
    ));
}

#[test]
fn bitmap_roundtrip_and_free_count() {
    let page = sample_bitmap();
    assert_eq!(page.free_blocks(), 95);
    let block = page.encode(BS, 9).unwrap();
    let (decoded, generation) = BitmapPage::decode(&block).unwrap();
    assert_eq!(generation, 9);
    assert_eq!(decoded.free_blocks(), 95);
    assert!(decoded.is_allocated(50));
    assert!(!decoded.is_allocated(51));
    // Trailing bits beyond valid range must be sealed as allocated.
    let mut torn = block.clone();
    // Bit 101 lives in payload byte 16 + 12; clearing it must be detected.
    let byte_index = 32 + 16 + (101 / 8);
    torn[byte_index] &= !(1 << (101 % 8));
    assert!(BitmapPage::decode(&torn).is_err());
}

#[test]
fn retired_list_roundtrip_and_double_retire() {
    let list = sample_retired();
    let block = list.encode(BS, 5).unwrap();
    assert_eq!(RetiredList::decode(&block).unwrap(), list);
    assert!(list.contains(42));
    assert!(!list.contains(43));
    let mut list = sample_retired();
    assert!(
        list.insert(42, 5).is_err(),
        "double retire must be rejected"
    );
}

fn sample_reclaim_root() -> ReclaimRoot {
    let mut root = ReclaimRoot::empty(ReclaimCaps {
        inline_entries: 8,
        segment_refs: 4,
        table_refs: 4,
    });
    root.table_refs.push(TableRef {
        lba: 40,
        ref_count: 2,
    });
    root.segment_refs.push(SegmentRef {
        lba: 41,
        entry_count: 3,
    });
    root.inline_entries.push(ReclaimEntry {
        start: 100,
        blocks: 5,
        retire_generation: 7,
    });
    root.head_segment_offset = 1;
    root.head_entry_offset = 2;
    root.head_block_offset = 1;
    root.appended_blocks_total = 60;
    root.reclaimed_blocks_total = 20;
    root.pending_blocks = 40;
    root
}

fn sample_reclaim_segment() -> ReclaimSegment {
    ReclaimSegment {
        entries: vec![
            ReclaimEntry {
                start: 100,
                blocks: 5,
                retire_generation: 3,
            },
            ReclaimEntry {
                start: 200,
                blocks: 1,
                retire_generation: 4,
            },
        ],
    }
}

fn sample_reclaim_table() -> ReclaimTable {
    ReclaimTable {
        refs: vec![
            SegmentRef {
                lba: 300,
                entry_count: 10,
            },
            SegmentRef {
                lba: 301,
                entry_count: 202,
            },
        ],
    }
}

#[test]
fn reclaim_structures_roundtrip() {
    let root = sample_reclaim_root();
    let block = root.encode(BS, 9).unwrap();
    let (decoded, generation) = ReclaimRoot::decode(&block).unwrap();
    assert_eq!(decoded, root);
    assert_eq!(generation, 9);

    let segment = sample_reclaim_segment();
    let block = segment.encode(BS, 4).unwrap();
    assert_eq!(ReclaimSegment::decode(&block).unwrap(), (segment, 4));

    let table = sample_reclaim_table();
    let block = table.encode(BS, 5).unwrap();
    assert_eq!(ReclaimTable::decode(&block).unwrap(), (table, 5));
}

#[test]
fn reclaim_structures_reject_inconsistencies() {
    // Cursor pointing past the first table's refs.
    let mut root = sample_reclaim_root();
    root.head_segment_offset = 2;
    assert!(root.encode(BS, 9).is_err());
    // Totals that do not reconcile with pending.
    let mut root = sample_reclaim_root();
    root.pending_blocks = 39;
    assert!(root.encode(BS, 9).is_err());
    // Areas exceeding their recorded capacities.
    let mut root = sample_reclaim_root();
    for i in 0..9 {
        root.inline_entries.push(ReclaimEntry {
            start: 500 + i,
            blocks: 1,
            retire_generation: 1,
        });
    }
    assert!(root.encode(BS, 9).is_err());
    // Zero-length runs and zero generations.
    assert!(ReclaimSegment {
        entries: vec![ReclaimEntry {
            start: 1,
            blocks: 0,
            retire_generation: 1
        }],
    }
    .encode(BS, 1)
    .is_err());
    assert!(ReclaimSegment {
        entries: vec![ReclaimEntry {
            start: 1,
            blocks: 1,
            retire_generation: 0
        }],
    }
    .encode(BS, 1)
    .is_err());
    // Empty sealed blocks are invalid by construction.
    assert!(ReclaimSegment::default().encode(BS, 1).is_err());
    assert!(ReclaimTable::default().encode(BS, 1).is_err());
}

fn sample_log_record() -> LogRecord {
    LogRecord {
        uuid: [7u8; 16],
        base_generation: 9,
        sequence: 2,
        ops: vec![
            LogOp::Create {
                parent_id: 1,
                name: b"HEAD.lock".to_vec(),
                expected_object_id: 21,
                size_bytes: 5000,
                content_crc: 0xDEAD_BEEF,
                extents: vec![(300, 2)],
                timestamp: ts(),
            },
            LogOp::Delete {
                parent_id: 1,
                name: b"old".to_vec(),
                timestamp: ts(),
            },
            LogOp::Rename {
                source_parent_id: 1,
                source_name: b"HEAD.lock".to_vec(),
                target_parent_id: 1,
                target_name: b"HEAD".to_vec(),
                replace: true,
                timestamp: ts(),
            },
            LogOp::Write {
                object_id: 7,
                logical_start: 3,
                expected_size_bytes: 16_384,
                new_size_bytes: 20_000,
                content_crc: 0x1234_5678,
                extents: vec![(500, 1), (700, 1)],
                timestamp: ts(),
            },
            LogOp::Truncate {
                object_id: 7,
                logical_start: 1,
                expected_size_bytes: 20_000,
                new_size_bytes: 5000,
                content_crc: 0x8765_4321,
                extents: vec![(900, 1)],
                timestamp: ts(),
            },
        ],
    }
}

#[test]
fn intent_log_record_roundtrip_and_rejections() {
    let record = sample_log_record();
    let block = record.encode(BS).unwrap();
    assert_eq!(le::get_u16(&block[HEADER_SIZE + 30..HEADER_SIZE + 32]), 3);
    assert_eq!(LogRecord::decode(&block).unwrap(), record);

    let mut namespace_only = record.clone();
    namespace_only.ops.truncate(3);
    let namespace_block = namespace_only.encode(BS).unwrap();
    assert_eq!(
        le::get_u16(&namespace_block[HEADER_SIZE + 30..HEADER_SIZE + 32]),
        2
    );

    // Zero-length runs, missing IDs, and oversized groups are rejected.
    let mut bad = sample_log_record();
    bad.ops[0] = LogOp::Create {
        parent_id: 1,
        name: b"x".to_vec(),
        expected_object_id: 0,
        size_bytes: 0,
        content_crc: 0,
        extents: Vec::new(),
        timestamp: ts(),
    };
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    if let LogOp::Create { extents, .. } = &mut bad.ops[0] {
        extents[0].1 = 0;
    }
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    bad.ops[3] = LogOp::Write {
        object_id: 0,
        logical_start: 0,
        expected_size_bytes: 1,
        new_size_bytes: 1,
        content_crc: 0,
        extents: vec![(1, 1)],
        timestamp: ts(),
    };
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    bad.ops[4] = LogOp::Truncate {
        object_id: 7,
        logical_start: 1,
        expected_size_bytes: 5000,
        new_size_bytes: 5000,
        content_crc: 0,
        extents: Vec::new(),
        timestamp: ts(),
    };
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    if let LogOp::Write { extents, .. } = &mut bad.ops[3] {
        *extents = vec![(1000, 2), (1001, 1)];
    }
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    if let LogOp::Write { extents, .. } = &mut bad.ops[3] {
        *extents = vec![(1000, 3)];
    }
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    if let LogOp::Truncate {
        logical_start,
        extents,
        ..
    } = &mut bad.ops[4]
    {
        *logical_start = 0;
        extents.clear();
    }
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    bad.sequence = 0;
    assert!(bad.encode(BS).is_err());
    let mut bad = sample_log_record();
    bad.ops.clear();
    assert!(bad.encode(BS).is_err());
}

#[test]
fn legacy_intent_record_decodes_with_its_historical_zero_timestamp() {
    const FIXED_PAYLOAD: usize = 32;
    const LEGACY_OP_FIXED: usize = 48;
    const CURRENT_OP_FIXED: usize = 64;
    let name = b"old";
    let record = LogRecord {
        uuid: [7u8; 16],
        base_generation: 9,
        sequence: 1,
        ops: vec![LogOp::Delete {
            parent_id: 1,
            name: name.to_vec(),
            timestamp: Timespec::default(),
        }],
    };
    let current = record.encode(BS).unwrap();
    let mut legacy = vec![0u8; BS];
    let payload = HEADER_SIZE;
    legacy[payload..payload + FIXED_PAYLOAD]
        .copy_from_slice(&current[payload..payload + FIXED_PAYLOAD]);
    le::put_u16(&mut legacy[payload + 30..payload + 32], 0);
    legacy[payload + FIXED_PAYLOAD..payload + FIXED_PAYLOAD + LEGACY_OP_FIXED].copy_from_slice(
        &current[payload + FIXED_PAYLOAD..payload + FIXED_PAYLOAD + LEGACY_OP_FIXED],
    );
    legacy[payload + FIXED_PAYLOAD + LEGACY_OP_FIXED
        ..payload + FIXED_PAYLOAD + LEGACY_OP_FIXED + name.len()]
        .copy_from_slice(
            &current[payload + FIXED_PAYLOAD + CURRENT_OP_FIXED
                ..payload + FIXED_PAYLOAD + CURRENT_OP_FIXED + name.len()],
        );
    BlockHeader {
        block_type: block_type::INTENT_LOG,
        flags: 0,
        owner: 0,
        generation: record.base_generation,
        payload_len: (FIXED_PAYLOAD + LEGACY_OP_FIXED + name.len()) as u32,
    }
    .seal(&mut legacy);

    assert_eq!(LogRecord::decode(&legacy).unwrap(), record);
}

#[test]
fn version_two_intent_record_remains_readable() {
    let mut record = sample_log_record();
    record.ops.truncate(3);
    let mut block = record.encode(BS).unwrap();
    le::put_u16(&mut block[HEADER_SIZE + 30..HEADER_SIZE + 32], 2);
    BlockHeader {
        block_type: block_type::INTENT_LOG,
        flags: 0,
        owner: 0,
        generation: record.base_generation,
        payload_len: le::get_u32(&block[24..28]),
    }
    .seal(&mut block);
    assert_eq!(LogRecord::decode(&block).unwrap(), record);
}

#[test]
fn timespec_short_buffer_is_an_error_not_a_panic() {
    assert!(Timespec::read(&[]).is_err());
    assert!(Timespec::read(&[0u8; 11]).is_err());
    let mut buf = [0u8; 12];
    ts().write(&mut buf);
    assert_eq!(Timespec::read(&buf).unwrap(), ts());
}

#[test]
fn geometry_reserved_blocks() {
    let geo = Geometry {
        block_size: BS,
        total_blocks: 300,
        region_size: 128,
    };
    geo.validate().unwrap();
    assert_eq!(geo.region_count(), 3);
    assert_eq!(geo.region_valid_blocks(2), 44);
    // Region 0: ident, checkpoints, descriptor slots, bitmap slots.
    for lba in 0..9 {
        assert!(geo.is_reserved(lba), "lba {lba}");
    }
    assert!(!geo.is_reserved(9));
    // Region 1: three descriptor plus three bitmap slots at its base.
    for lba in 128..134 {
        assert!(geo.is_reserved(lba), "lba {lba}");
    }
    assert!(!geo.is_reserved(134));
    assert_eq!(geo.descriptor_slot_lba(0, 0), 3);
    assert_eq!(geo.descriptor_slot_lba(0, 2), 5);
    assert_eq!(geo.bitmap_slot_lba(0, 0, 0), 6);
    assert_eq!(geo.bitmap_slot_lba(0, 0, 2), 8);
    assert_eq!(geo.descriptor_slot_lba(1, 1), 129);
    assert_eq!(geo.bitmap_slot_lba(1, 0, 1), 132);
    assert!(!geo.is_allocatable(299 + 1));
}

#[test]
fn single_region_geometry_uses_region_relative_bounds() {
    let geo = Geometry {
        block_size: BS,
        total_blocks: 16_384,
        region_size: 16_384,
    };
    geo.validate().unwrap();
    assert_eq!(geo.region_count(), 1);
    assert_eq!(geo.region_valid_blocks(0), 16_384);
    assert_eq!(geo.region0_reserved_blocks(), 9);
    assert!(geo.is_reserved(8));
    assert!(!geo.is_allocatable(8));
    assert!(!geo.is_reserved(9));
    assert!(geo.is_allocatable(9));
    assert!(!geo.is_reserved(16_384));
    assert!(!geo.is_allocatable(16_384));
}

#[test]
fn multi_page_region_descriptor_roundtrip() {
    let geo = Geometry {
        block_size: BS,
        total_blocks: 262_144,
        region_size: 262_144,
    };
    geo.validate().unwrap();
    assert_eq!(geo.bitmap_page_count(0), 9);
    // 30 allocation-metadata blocks plus ident and two checkpoints.
    assert_eq!(geo.region0_reserved_blocks(), 33);

    let pages: Vec<_> = (0..geo.bitmap_page_count(0))
        .map(|page_index| BitmapBinding {
            slot: (page_index % 3) as u8,
            free_blocks: geo.bitmap_page_valid_blocks(0, page_index),
            generation: 4,
        })
        .collect();
    let descriptor = RegionDescriptor {
        region: 0,
        valid_blocks: geo.region_valid_blocks(0),
        free_blocks: pages.iter().map(|binding| binding.free_blocks).sum(),
        pages,
    };
    let encoded = descriptor.encode(BS, 4).unwrap();
    let (decoded, generation) = RegionDescriptor::decode(&encoded).unwrap();
    assert_eq!(generation, 4);
    decoded.validate(&geo, 0, generation).unwrap();
    assert_eq!(decoded, descriptor);
}

#[test]
fn shared_tree_leaf_and_internal_nodes_roundtrip() {
    assert_eq!(TreeNode::fixed_item_capacity(BS, 4, 16).unwrap(), 144);
    let leaf = sample_tree_leaf();
    let encoded = leaf.encode(BS, 7).unwrap();
    let (decoded, generation) = TreeNode::decode(&encoded).unwrap();
    assert_eq!(decoded, leaf);
    assert_eq!(generation, 7);
    assert!(leaf.fits(BS));

    let internal = TreeNode {
        kind: TreeKind::ObjectMap,
        owner: 0,
        level: 1,
        subtree_items: 300,
        leftmost_child: 40,
        leftmost_items: 100,
        items: vec![
            TreeItem {
                key: key_u64(100).to_vec(),
                value: child_value(ChildRef {
                    lba: 41,
                    subtree_items: 100,
                })
                .unwrap(),
            },
            TreeItem {
                key: key_u64(200).to_vec(),
                value: child_value(ChildRef {
                    lba: 42,
                    subtree_items: 100,
                })
                .unwrap(),
            },
        ],
    };
    let encoded = internal.encode(BS, 8).unwrap();
    assert_eq!(TreeNode::decode(&encoded).unwrap(), (internal, 8));
}

#[test]
fn shared_tree_rejects_bad_order_depth_children_and_hostile_counts() {
    let mut node = sample_tree_leaf();
    node.items.swap(0, 1);
    assert!(node.encode(BS, 1).is_err());

    let mut node = sample_tree_leaf();
    node.level = MAX_TREE_LEVEL + 1;
    assert!(node.encode(BS, 1).is_err());

    let bad_internal = TreeNode {
        kind: TreeKind::Directory,
        owner: 1,
        level: 1,
        subtree_items: 2,
        leftmost_child: 10,
        leftmost_items: 1,
        items: vec![TreeItem {
            key: b"x".to_vec(),
            value: vec![1, 2, 3],
        }],
    };
    assert!(bad_internal.encode(BS, 1).is_err());

    // A checksummed but hostile count must fail bounds-first, without trying
    // to reserve attacker-controlled memory.
    let mut encoded = sample_tree_leaf().encode(BS, 1).unwrap();
    let header =
        BlockHeader::verify(&encoded, afsplus_format::header::block_type::TREE_NODE).unwrap();
    afsplus_format::le::put_u32(&mut encoded[32 + 4..32 + 8], u32::MAX);
    header.seal(&mut encoded);
    assert!(TreeNode::decode(&encoded).is_err());

    let numeric = [0u64, 1, 255, 256, u64::MAX]
        .map(key_u64)
        .map(|key| key.to_vec());
    assert!(numeric.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn dir_block_roundtrip_preserves_original_names_and_key_order() {
    let dir = sample_dir();
    let block = dir.encode(BS, 5).unwrap();
    let decoded = DirBlock::decode(&block).unwrap();
    assert_eq!(decoded, dir);
    let keys: Vec<_> = decoded.entries.iter().map(|e| e.key.clone()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert!(decoded
        .entries
        .iter()
        .any(|e| e.name == "Émoji-☂.rs".as_bytes()));
}

#[test]
fn dir_rejects_duplicate_and_invalid_names() {
    let mut dir = sample_dir();
    let dup = DirEntry {
        key: comparison_key(b"alpha.txt"),
        name: b"alpha.txt".to_vec(),
        child_type_hint: 1,
        child_id: 18,
    };
    assert!(matches!(dir.insert(dup), Err(FormatError::Invalid(_))));
    assert!(afsplus_format::validate_name(b"").is_err());
    assert!(afsplus_format::validate_name(b"a/b").is_err());
    assert!(afsplus_format::validate_name(&[0xFF, 0xFE]).is_err());
    assert!(afsplus_format::validate_name(&[b'x'; 256]).is_err());
    assert!(afsplus_format::validate_name("naïve-☂.txt".as_bytes()).is_ok());
}

#[test]
fn omap_roundtrip_ordering_and_removal() {
    let mut omap = ObjectMap::default();
    omap.upsert(17, 100).unwrap();
    omap.upsert(1, 50).unwrap();
    omap.upsert(17, 101).unwrap(); // update in place
    let block = omap.encode(BS, 5).unwrap();
    let decoded = ObjectMap::decode(&block).unwrap();
    assert_eq!(decoded, omap);
    assert_eq!(decoded.lookup(17), Some(101));
    assert_eq!(omap.remove(17), Some(101));
    assert_eq!(omap.lookup(17), None);
}

#[test]
fn every_flipped_byte_is_detected() {
    // CRC32C must catch any single-byte corruption in any block type.
    let blocks: Vec<Vec<u8>> = vec![
        sample_ident().encode(BS).unwrap(),
        sample_checkpoint().encode(BS).unwrap(),
        sample_record().encode(BS, 5).unwrap(),
        sample_dir().encode(BS, 5).unwrap(),
        sample_bitmap().encode(BS, 5).unwrap(),
        sample_region_descriptor().encode(BS, 5).unwrap(),
        sample_tree_leaf().encode(BS, 5).unwrap(),
        sample_retired().encode(BS, 5).unwrap(),
    ];
    for block in blocks {
        for offset in (0..BS).step_by(97) {
            let mut corrupt = block.clone();
            corrupt[offset] ^= 0x40;
            assert!(
                Identification::decode(&corrupt).is_err()
                    && Checkpoint::decode(&corrupt, &[7u8; 16]).is_err()
                    && ObjectRecord::decode(&corrupt).is_err()
                    && DirBlock::decode(&corrupt).is_err()
                    && ObjectMap::decode(&corrupt).is_err()
                    && BitmapPage::decode(&corrupt).is_err()
                    && RegionDescriptor::decode(&corrupt).is_err()
                    && TreeNode::decode(&corrupt).is_err()
                    && RetiredList::decode(&corrupt).is_err(),
                "corruption at offset {offset} was not detected"
            );
        }
    }
}

#[test]
fn decoders_reject_garbage_without_panicking() {
    let mut garbage = vec![0u8; BS];
    for (i, byte) in garbage.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(31).wrapping_add(7);
    }
    assert!(Identification::decode(&garbage).is_err());
    assert!(Checkpoint::decode(&garbage, &[0u8; 16]).is_err());
    assert!(ObjectRecord::decode(&garbage).is_err());
    assert!(DirBlock::decode(&garbage).is_err());
    assert!(ObjectMap::decode(&garbage).is_err());
    assert!(BitmapPage::decode(&garbage).is_err());
    assert!(RegionDescriptor::decode(&garbage).is_err());
    assert!(TreeNode::decode(&garbage).is_err());
    assert!(RetiredList::decode(&garbage).is_err());
    // Truncated buffers.
    assert!(Identification::decode(&garbage[..16]).is_err());
    assert!(DirBlock::decode(&[]).is_err());
}

#[test]
fn dir_overflow_is_reported_not_truncated() {
    let mut dir = DirBlock::new(OBJECT_ROOT);
    for i in 0..=100u64 {
        let name = format!("file-with-a-rather-long-name-{i:060}");
        dir.insert(DirEntry {
            key: comparison_key(name.as_bytes()),
            name: name.into_bytes(),
            child_type_hint: 1,
            child_id: 16 + i,
        })
        .unwrap();
    }
    assert!(matches!(dir.encode(BS, 5), Err(FormatError::Overflow(_))));
}
