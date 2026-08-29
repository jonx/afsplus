//! Encode/decode round-trips and corruption rejection for every prototype
//! block type. Every decoder must reject a flipped byte via its CRC and must
//! never panic on arbitrary input (`docs/21-security-and-corruption.md`).

use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::{Checkpoint, RegionRecord};
use afsplus_format::crc32c::CHECKSUM_CRC32C;
use afsplus_format::dir::{comparison_key, DirBlock, DirEntry};
use afsplus_format::geometry::Geometry;
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::omap::ObjectMap;
use afsplus_format::retired::RetiredList;
use afsplus_format::{FormatError, Timespec, DEFAULT_BLOCK_SHIFT, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

const BS: usize = DEFAULT_BLOCK_SIZE;

fn ts() -> Timespec {
    Timespec { seconds: 1_780_000_000, nanoseconds: 123_456_789 }
}

fn sample_ident() -> Identification {
    Identification {
        uuid: [7u8; 16],
        block_shift: DEFAULT_BLOCK_SHIFT,
        checksum_algorithm: CHECKSUM_CRC32C,
        region_size: 256,
        total_blocks: 1024,
        checkpoint_slots: [1, 2],
        metadata_start: 6,
        label: "Test Volume".into(),
    }
}

fn sample_checkpoint() -> Checkpoint {
    Checkpoint {
        uuid: [7u8; 16],
        generation: 5,
        root_object_id: OBJECT_ROOT,
        object_map_block: 10,
        retired_list_block: 11,
        next_object_id: 20,
        committed_tx_id: 5,
        flags: 0,
        regions: vec![
            RegionRecord { slot: 0, free_blocks: 100, bitmap_generation: 5 },
            RegionRecord { slot: 2, free_blocks: 200, bitmap_generation: 3 },
            RegionRecord { slot: 1, free_blocks: 250, bitmap_generation: 1 },
            RegionRecord { slot: 0, free_blocks: 250, bitmap_generation: 1 },
        ],
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
    let mut page = BitmapPage::all_free(3, 100);
    for index in [0, 1, 2, 50, 99] {
        page.set_allocated(index, true);
    }
    page
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
fn checkpoint_roundtrip_and_uuid_binding() {
    let checkpoint = sample_checkpoint();
    let block = checkpoint.encode(BS).unwrap();
    assert_eq!(Checkpoint::decode(&block, &[7u8; 16]).unwrap(), checkpoint);
    // A checkpoint from another volume must never be accepted.
    assert!(Checkpoint::decode(&block, &[8u8; 16]).is_err());
}

#[test]
fn checkpoint_structural_validation() {
    let geo = Geometry { block_size: BS, total_blocks: 1024, region_size: 256 };
    sample_checkpoint().validate_structural(&geo).unwrap();

    // Wrong region count.
    let mut ckpt = sample_checkpoint();
    ckpt.regions.pop();
    assert!(ckpt.validate_structural(&geo).is_err());

    // Slot out of range.
    let mut ckpt = sample_checkpoint();
    ckpt.regions[0].slot = 3;
    assert!(ckpt.validate_structural(&geo).is_err());

    // Bitmap generation from the future.
    let mut ckpt = sample_checkpoint();
    ckpt.regions[1].bitmap_generation = 6;
    assert!(ckpt.validate_structural(&geo).is_err());

    // Object map inside the reserved area.
    let mut ckpt = sample_checkpoint();
    ckpt.object_map_block = 4;
    assert!(ckpt.validate_structural(&geo).is_err());

    // Free count exceeding the region.
    let mut ckpt = sample_checkpoint();
    ckpt.regions[0].free_blocks = 257;
    assert!(ckpt.validate_structural(&geo).is_err());
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
    // Bit 101 lives in payload byte 8 + 12; clearing it must be detected.
    let byte_index = 32 + 8 + (101 / 8);
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
    assert!(list.insert(42, 5).is_err(), "double retire must be rejected");
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
    let geo = Geometry { block_size: BS, total_blocks: 300, region_size: 128 };
    geo.validate().unwrap();
    assert_eq!(geo.region_count(), 3);
    assert_eq!(geo.region_valid_blocks(2), 44);
    // Region 0: ident, checkpoints, bitmap slots.
    for lba in 0..6 {
        assert!(geo.is_reserved(lba), "lba {lba}");
    }
    assert!(!geo.is_reserved(6));
    // Region 1: three bitmap slots at its base.
    for lba in 128..131 {
        assert!(geo.is_reserved(lba), "lba {lba}");
    }
    assert!(!geo.is_reserved(131));
    assert_eq!(geo.bitmap_slot_lba(0, 0), 3);
    assert_eq!(geo.bitmap_slot_lba(0, 2), 5);
    assert_eq!(geo.bitmap_slot_lba(1, 1), 129);
    assert!(!geo.is_allocatable(299 + 1));
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
    assert!(decoded.entries.iter().any(|e| e.name == "Émoji-☂.rs".as_bytes()));
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
