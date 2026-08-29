//! Encode/decode round-trips and corruption rejection for every prototype
//! block type. Every decoder must reject a flipped byte via its CRC and must
//! never panic on arbitrary input (`docs/21-security-and-corruption.md`).

use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::crc32c::CHECKSUM_CRC32C;
use afsplus_format::dir::{comparison_key, DirBlock, DirEntry};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::omap::ObjectMap;
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
        total_blocks: 1024,
        checkpoint_slots: [1, 2],
        metadata_start: 3,
        label: "Test Volume".into(),
    }
}

fn sample_checkpoint() -> Checkpoint {
    Checkpoint {
        uuid: [7u8; 16],
        generation: 5,
        root_object_id: OBJECT_ROOT,
        object_map_block: 10,
        next_free_block: 42,
        next_object_id: 20,
        committed_tx_id: 5,
        flags: 0,
    }
}

fn sample_record() -> ObjectRecord {
    ObjectRecord {
        object_id: 17,
        object_type: ObjectType::File,
        flags: 0,
        link_count: 1,
        size_bytes: 0,
        allocated_bytes: 0,
        created: ts(),
        modified: ts(),
        changed: ts(),
        protection: 0,
        content_generation: 5,
        data_root: 0,
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
fn object_record_roundtrip() {
    let record = sample_record();
    let block = record.encode(BS, 5).unwrap();
    assert_eq!(ObjectRecord::decode(&block).unwrap(), record);
}

#[test]
fn dir_block_roundtrip_preserves_original_names_and_key_order() {
    let dir = sample_dir();
    let block = dir.encode(BS, 5).unwrap();
    let decoded = DirBlock::decode(&block).unwrap();
    assert_eq!(decoded, dir);
    // Strict byte-wise key ordering, and original spelling preserved.
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
fn omap_roundtrip_and_ordering() {
    let mut omap = ObjectMap::default();
    omap.upsert(17, 100).unwrap();
    omap.upsert(1, 50).unwrap();
    omap.upsert(17, 101).unwrap(); // update in place
    let block = omap.encode(BS, 5).unwrap();
    let decoded = ObjectMap::decode(&block).unwrap();
    assert_eq!(decoded, omap);
    assert_eq!(decoded.lookup(17), Some(101));
    assert_eq!(decoded.lookup(1), Some(50));
    assert_eq!(decoded.lookup(2), None);
}

#[test]
fn every_flipped_byte_is_detected() {
    // CRC32C must catch any single-byte corruption in any block type.
    let blocks: Vec<Vec<u8>> = vec![
        sample_ident().encode(BS).unwrap(),
        sample_checkpoint().encode(BS).unwrap(),
        sample_record().encode(BS, 5).unwrap(),
        sample_dir().encode(BS, 5).unwrap(),
    ];
    for block in blocks {
        // Sample offsets across the whole block, including header and slack.
        for offset in (0..BS).step_by(97) {
            let mut corrupt = block.clone();
            corrupt[offset] ^= 0x40;
            assert!(
                Identification::decode(&corrupt).is_err()
                    && Checkpoint::decode(&corrupt, &[7u8; 16]).is_err()
                    && ObjectRecord::decode(&corrupt).is_err()
                    && DirBlock::decode(&corrupt).is_err()
                    && ObjectMap::decode(&corrupt).is_err(),
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
    // Truncated buffers.
    assert!(Identification::decode(&garbage[..16]).is_err());
    assert!(DirBlock::decode(&[]).is_err());
}

#[test]
fn dir_overflow_is_reported_not_truncated() {
    let mut dir = DirBlock::new(OBJECT_ROOT);
    let mut i = 0u64;
    loop {
        let name = format!("file-with-a-rather-long-name-{i:060}");
        dir.insert(DirEntry {
            key: comparison_key(name.as_bytes()),
            name: name.into_bytes(),
            child_type_hint: 1,
            child_id: 16 + i,
        })
        .unwrap();
        i += 1;
        if i > 100 {
            break;
        }
    }
    assert!(matches!(dir.encode(BS, 5), Err(FormatError::Overflow(_))));
}
