//! Wire proof of the security preservation container: the object record's
//! security reference and the `"AFSX"` descriptor segment.
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::object::{
    ObjectRecord, ObjectType, SecurityRef, SymlinkRecord, OBJECT_FLAG_SECURITY_REF,
    SECURITY_REF_PROJECTION_DIVERGED,
};
use afsplus_format::security::{
    segment_capacity, segment_count, SecuritySegment, MAX_SECURITY_DESCRIPTOR_BYTES,
};
use afsplus_format::{FormatError, Timespec, DEFAULT_BLOCK_SIZE};

fn file() -> ObjectRecord {
    ObjectRecord {
        object_id: 16,
        object_type: ObjectType::File,
        flags: 0,
        link_count: 1,
        size_bytes: 0,
        allocated_bytes: 0,
        created: Timespec::default(),
        modified: Timespec::default(),
        changed: Timespec::default(),
        protection: 0x0f,
        content_generation: 1,
        data_root: 0,
        data_blocks: 0,
        security: None,
        attributes: None,
        comment: afsplus_format::object::Comment::EMPTY,
    }
}

fn reference() -> SecurityRef {
    SecurityRef {
        first_block: 0x0102_0304_0506_0708,
        total_len: 5000,
        segment_count: 2,
        flags: SECURITY_REF_PROJECTION_DIVERGED,
    }
}

fn reseal(block: &mut [u8], kind: u32, owner: u64, payload_len: u32) {
    BlockHeader {
        block_type: kind,
        flags: 0,
        owner,
        generation: 7,
        payload_len,
    }
    .seal(block);
}

#[test]
fn security_reference_has_the_documented_wire_bytes() {
    let record = file().with_security(Some(reference()));
    assert_eq!(record.flags, OBJECT_FLAG_SECURITY_REF);
    let block = record.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    // payload_len 112, flag bit 2, then the literal little-endian trailer.
    assert_eq!(&block[24..28], &112u32.to_le_bytes());
    assert_eq!(&block[HEADER_SIZE + 10..HEADER_SIZE + 12], &[0x04, 0x00]);
    assert_eq!(
        &block[HEADER_SIZE + 96..HEADER_SIZE + 112],
        &[
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // first block
            0x88, 0x13, 0x00, 0x00, // 5000 bytes
            0x02, 0x00, // two segments
            0x01, 0x00, // projection diverged
        ]
    );
    assert!(block[HEADER_SIZE + 112..].iter().all(|b| *b == 0));
    assert_eq!(ObjectRecord::decode(&block), Ok(record));
    assert_eq!(
        ObjectRecord::decode_metadata_with_generation(&block),
        Ok((record, 7))
    );
}

#[test]
fn a_record_without_a_reference_keeps_its_96_byte_image() {
    let block = file().encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(&block[24..28], &96u32.to_le_bytes());
    let attached = file().with_security(Some(reference()));
    assert_eq!(attached.with_security(None), file());
}

#[test]
fn flag_and_field_must_agree_in_both_directions() {
    let mut flag_only = file();
    flag_only.flags = OBJECT_FLAG_SECURITY_REF;
    let mut field_only = file();
    field_only.security = Some(reference());
    for record in [flag_only, field_only] {
        assert_eq!(
            record.encode(DEFAULT_BLOCK_SIZE, 7),
            Err(FormatError::Invalid(
                "security reference flag and field disagree"
            ))
        );
    }
    // On the wire: the flag with a 96-byte payload, and a 112-byte payload
    // without the flag, are both refused by the exact-length rule.
    let mut short = file().encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    short[HEADER_SIZE + 10] = 0x04;
    reseal(&mut short, block_type::OBJECT, 16, 96);
    let mut long = file().encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    reseal(&mut long, block_type::OBJECT, 16, 112);
    for block in [short, long] {
        assert_eq!(
            ObjectRecord::decode(&block),
            Err(FormatError::Invalid("object payload length is not exact"))
        );
    }
}

#[test]
fn malformed_references_are_refused() {
    let cases = [
        SecurityRef {
            first_block: 0,
            ..reference()
        },
        SecurityRef {
            total_len: 0,
            segment_count: 0,
            ..reference()
        },
        SecurityRef {
            total_len: MAX_SECURITY_DESCRIPTOR_BYTES + 1,
            segment_count: 17,
            ..reference()
        },
        SecurityRef {
            segment_count: 1,
            ..reference()
        },
        SecurityRef {
            flags: 2,
            ..reference()
        },
    ];
    for bad in cases {
        assert_eq!(
            file()
                .with_security(Some(bad))
                .encode(DEFAULT_BLOCK_SIZE, 7),
            Err(FormatError::Invalid("invalid security reference"))
        );
    }
    // The same fields arriving from the wire are refused by the decoder.
    let mut block = file()
        .with_security(Some(reference()))
        .encode(DEFAULT_BLOCK_SIZE, 7)
        .unwrap();
    block[HEADER_SIZE + 110] = 2;
    reseal(&mut block, block_type::OBJECT, 16, 112);
    assert_eq!(
        ObjectRecord::decode(&block),
        Err(FormatError::Invalid("invalid security reference"))
    );
}

#[test]
fn directories_and_symlinks_carry_a_reference_and_symlinks_keep_their_target() {
    let mut directory = file();
    directory.object_type = ObjectType::Directory;
    directory.data_root = 99;
    let directory = directory.with_security(Some(reference()));
    let block = directory.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(ObjectRecord::decode(&block), Ok(directory));

    let mut link = file();
    link.object_type = ObjectType::Symlink;
    link.size_bytes = 6;
    let link = link.with_security(Some(reference()));
    let block = SymlinkRecord {
        record: link,
        target: "target",
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .unwrap();
    assert_eq!(&block[24..28], &118u32.to_le_bytes());
    assert_eq!(&block[HEADER_SIZE + 112..HEADER_SIZE + 118], b"target");
    let (decoded, generation) = SymlinkRecord::decode(&block).unwrap();
    assert_eq!(
        (decoded.record, decoded.target, generation),
        (link, "target", 7)
    );

    // A reference takes 16 bytes from the longest admissible target.
    let longest = "x".repeat(DEFAULT_BLOCK_SIZE - HEADER_SIZE - 112);
    let mut full = link;
    full.size_bytes = longest.len() as u64;
    assert!(SymlinkRecord {
        record: full,
        target: &longest
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .is_ok());
    let over = "x".repeat(longest.len() + 1);
    full.size_bytes = over.len() as u64;
    assert!(SymlinkRecord {
        record: full,
        target: &over
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .is_err());
}

#[test]
fn segment_geometry_is_literal() {
    assert_eq!(segment_capacity(4096), 4040);
    assert_eq!(segment_count(1, 4096), Some(1));
    assert_eq!(segment_count(4040, 4096), Some(1));
    assert_eq!(segment_count(4041, 4096), Some(2));
    assert_eq!(segment_count(65_536, 4096), Some(17));
    assert_eq!(segment_count(0, 4096), None);
    assert_eq!(segment_count(65_537, 4096), None);
}

#[test]
fn descriptor_segment_roundtrips_with_the_documented_wire_bytes() {
    let bytes = [0xde, 0xad, 0xbe, 0xef, 0x00, 0xff];
    let segment = SecuritySegment {
        object_id: 16,
        format: 0x8000_0001,
        version: 3,
        total_len: 6,
        index: 0,
        count: 1,
        next: 0,
        bytes: &bytes,
    };
    let block = segment.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(&block[0..4], b"AFSX");
    assert_eq!(&block[8..16], &16u64.to_le_bytes());
    assert_eq!(&block[24..28], &30u32.to_le_bytes());
    assert_eq!(
        &block[HEADER_SIZE..HEADER_SIZE + 30],
        &[
            0x01, 0x00, 0x00, 0x80, // format
            0x03, 0x00, // version
            0x00, 0x00, // reserved
            0x06, 0x00, 0x00, 0x00, // total length
            0x00, 0x00, // index
            0x01, 0x00, // count
            0, 0, 0, 0, 0, 0, 0, 0, // next
            0xde, 0xad, 0xbe, 0xef, 0x00, 0xff,
        ]
    );
    assert_eq!(SecuritySegment::decode(&block), Ok((segment, 7)));
}

#[test]
fn descriptor_segment_admission_is_exact() {
    let bytes = vec![0x5a; 4040];
    let first = SecuritySegment {
        object_id: 16,
        format: 1,
        version: 0,
        total_len: 4041,
        index: 0,
        count: 2,
        next: 500,
        bytes: &bytes,
    };
    let clean = first.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(SecuritySegment::decode(&clean), Ok((first, 7)));

    let invalid = [
        SecuritySegment { format: 0, ..first },
        SecuritySegment {
            object_id: 0,
            ..first
        },
        SecuritySegment { next: 0, ..first },
        SecuritySegment { index: 2, ..first },
        SecuritySegment { count: 3, ..first },
        SecuritySegment {
            bytes: &bytes[..4039],
            ..first
        },
        SecuritySegment {
            index: 1,
            next: 0,
            ..first
        },
    ];
    for bad in invalid {
        assert!(bad.encode(DEFAULT_BLOCK_SIZE, 7).is_err(), "{bad:?}");
    }

    // Resealed corruptions of a valid short segment.
    let tail = [1u8, 2, 3];
    let last = SecuritySegment {
        object_id: 16,
        format: 1,
        version: 0,
        total_len: 3,
        index: 0,
        count: 1,
        next: 0,
        bytes: &tail,
    };
    let clean = last.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    let mut reserved = clean.clone();
    reserved[HEADER_SIZE + 6] = 1;
    reseal(&mut reserved, block_type::SECURITY_DESCRIPTOR, 16, 27);
    assert_eq!(
        SecuritySegment::decode(&reserved).map(|_| ()),
        Err(FormatError::Invalid(
            "security segment reserved field is nonzero"
        ))
    );
    let mut dirty_tail = clean.clone();
    dirty_tail[DEFAULT_BLOCK_SIZE - 1] = 1;
    reseal(&mut dirty_tail, block_type::SECURITY_DESCRIPTOR, 16, 27);
    assert_eq!(
        SecuritySegment::decode(&dirty_tail).map(|_| ()),
        Err(FormatError::Invalid("block unused tail is nonzero"))
    );
    let mut longer = clean.clone();
    reseal(&mut longer, block_type::SECURITY_DESCRIPTOR, 16, 28);
    assert_eq!(
        SecuritySegment::decode(&longer).map(|_| ()),
        Err(FormatError::Invalid("security segment length is not exact"))
    );
    let mut flagged = clean.clone();
    BlockHeader {
        block_type: block_type::SECURITY_DESCRIPTOR,
        flags: 1,
        owner: 16,
        generation: 7,
        payload_len: 27,
    }
    .seal(&mut flagged);
    assert_eq!(
        SecuritySegment::decode(&flagged).map(|_| ()),
        Err(FormatError::Invalid(
            "security segment header flags are nonzero"
        ))
    );
    // An object block is never admitted as a descriptor segment.
    let object = file().encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert!(matches!(
        SecuritySegment::decode(&object),
        Err(FormatError::WrongBlockType { .. })
    ));
}
