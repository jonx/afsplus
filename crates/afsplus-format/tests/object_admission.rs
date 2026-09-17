//! Exact object-record admission: header flags, payload length and the
//! unused tail are checked by every object reader, so no reader accepts
//! bytes that an ordinary rewrite would drop.
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::object::{ObjectRecord, ObjectType, SymlinkRecord};
use afsplus_format::{FormatError, Timespec, DEFAULT_BLOCK_SIZE};

const FIXED_PAYLOAD: usize = 96;

fn record(kind: ObjectType) -> ObjectRecord {
    ObjectRecord {
        object_id: 16,
        object_type: kind,
        flags: 0,
        link_count: 1,
        size_bytes: 0,
        allocated_bytes: 0,
        created: Timespec::default(),
        modified: Timespec::default(),
        changed: Timespec::default(),
        protection: 0x5a,
        content_generation: 1,
        data_root: if kind == ObjectType::Directory {
            123
        } else {
            0
        },
        data_blocks: 0,
    }
}

fn reseal(block: &mut [u8], flags: u16, payload_len: u32) {
    BlockHeader {
        block_type: block_type::OBJECT,
        flags,
        owner: 16,
        generation: 7,
        payload_len,
    }
    .seal(block);
    BlockHeader::verify(block, block_type::OBJECT).unwrap();
}

fn rejected_by_all_readers(block: &[u8], reason: &'static str) {
    for result in [
        ObjectRecord::decode(block).map(|_| ()),
        ObjectRecord::decode_with_generation(block).map(|_| ()),
        ObjectRecord::decode_metadata_with_generation(block).map(|_| ()),
    ] {
        assert_eq!(result, Err(FormatError::Invalid(reason)));
    }
}

#[test]
fn every_common_header_flag_is_rejected_after_crc_reseal() {
    for kind in [ObjectType::File, ObjectType::Directory] {
        let clean = record(kind).encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
        for bit in 0..16 {
            let mut block = clean.clone();
            reseal(&mut block, 1 << bit, FIXED_PAYLOAD as u32);
            rejected_by_all_readers(&block, "object header flags are nonzero");
        }
    }
}

#[test]
fn a_payload_longer_than_the_fixed_record_is_rejected() {
    for kind in [ObjectType::File, ObjectType::Directory] {
        let clean = record(kind).encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
        for extra in [
            1usize,
            8,
            16,
            DEFAULT_BLOCK_SIZE - HEADER_SIZE - FIXED_PAYLOAD,
        ] {
            // Zero extension bytes and nonzero ones are refused alike: the
            // length itself is the unnegotiated extension.
            for fill in [0u8, 0xa5] {
                let mut block = clean.clone();
                let end = HEADER_SIZE + FIXED_PAYLOAD + extra;
                block[HEADER_SIZE + FIXED_PAYLOAD..end].fill(fill);
                reseal(&mut block, 0, (FIXED_PAYLOAD + extra) as u32);
                rejected_by_all_readers(&block, "object payload length is not exact");
            }
        }
    }
}

#[test]
fn a_nonzero_unused_tail_is_rejected_at_every_position_class() {
    for kind in [ObjectType::File, ObjectType::Directory] {
        let clean = record(kind).encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
        for offset in [
            HEADER_SIZE + FIXED_PAYLOAD,
            HEADER_SIZE + FIXED_PAYLOAD + 1,
            DEFAULT_BLOCK_SIZE / 2,
            DEFAULT_BLOCK_SIZE - 1,
        ] {
            let mut block = clean.clone();
            block[offset] = 1;
            reseal(&mut block, 0, FIXED_PAYLOAD as u32);
            rejected_by_all_readers(&block, "object unused tail is nonzero");
        }
    }
}

#[test]
fn the_canonical_encoding_is_the_only_admitted_image_of_a_record() {
    // Negative control for the three tests above: the untouched encoding is
    // admitted by the same readers, decodes to the literal record and
    // re-encodes to the identical block, so a rewrite loses nothing.
    for kind in [ObjectType::File, ObjectType::Directory] {
        let expected = record(kind);
        let clean = expected.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
        assert_eq!(ObjectRecord::decode(&clean), Ok(expected));
        assert_eq!(
            ObjectRecord::decode_metadata_with_generation(&clean),
            Ok((expected, 7))
        );
        assert_eq!(expected.encode(DEFAULT_BLOCK_SIZE, 7).unwrap(), clean);
    }
}

#[test]
fn symlink_header_flags_are_rejected_by_the_metadata_reader_too() {
    let mut link = record(ObjectType::Symlink);
    link.size_bytes = 6;
    let clean = SymlinkRecord {
        record: link,
        target: "target",
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .unwrap();
    assert_eq!(
        ObjectRecord::decode_metadata_with_generation(&clean),
        Ok((link, 7))
    );
    let mut block = clean.clone();
    reseal(&mut block, 1, (FIXED_PAYLOAD + 6) as u32);
    assert_eq!(
        ObjectRecord::decode_metadata_with_generation(&block).map(|_| ()),
        Err(FormatError::Invalid("object header flags are nonzero"))
    );
    assert!(SymlinkRecord::decode(&block).is_err());
}
