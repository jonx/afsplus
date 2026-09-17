//! Wire image of the attribute reference in the object record, of the
//! attribute set, and of the `"AFSA"` chain that holds it.
use afsplus_format::attrs::{
    decode_attribute_set, encode_attribute_set, validate_attribute_name, ATTRIBUTE_CHAIN,
    ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION,
};
use afsplus_format::chain::ChainSegment;
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::object::{
    AttributeRef, Comment, ObjectRecord, ObjectType, SecurityRef, SymlinkRecord,
    OBJECT_FLAG_ATTRIBUTES, OBJECT_FLAG_COMMENT, OBJECT_FLAG_SECURITY_REF,
};
use afsplus_format::security::SecuritySegment;
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
        protection: 0,
        content_generation: 1,
        data_root: 0,
        data_blocks: 0,
        security: None,
        attributes: None,
        comment: Comment::EMPTY,
    }
}

fn set() -> AttributeRef {
    AttributeRef {
        first_block: 0x0102_0304_0506_0708,
        total_len: 5000,
        segment_count: 2,
    }
}

fn security() -> SecurityRef {
    SecurityRef {
        first_block: 77,
        total_len: 10,
        segment_count: 1,
        flags: 0,
    }
}

fn reseal(block: &mut [u8], payload_len: u32) {
    BlockHeader {
        block_type: block_type::OBJECT,
        flags: 0,
        owner: 16,
        generation: 7,
        payload_len,
    }
    .seal(block);
}

#[test]
fn the_reference_sits_after_the_security_reference_and_before_the_comment() {
    let alone = file().with_attributes(Some(set()));
    assert_eq!(alone.flags, OBJECT_FLAG_ATTRIBUTES);
    assert_eq!(alone.fixed_payload_len(), 112);
    let block = alone.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(&block[HEADER_SIZE + 10..HEADER_SIZE + 12], &[0x10, 0x00]);
    assert_eq!(
        &block[HEADER_SIZE + 96..HEADER_SIZE + 112],
        &[8, 7, 6, 5, 4, 3, 2, 1, 0x88, 0x13, 0, 0, 2, 0, 0, 0]
    );
    assert_eq!(ObjectRecord::decode(&block).unwrap(), alone);

    let full = file()
        .with_security(Some(security()))
        .with_attributes(Some(set()))
        .with_comment(Comment::new("Dé").unwrap());
    assert_eq!(
        full.flags,
        OBJECT_FLAG_SECURITY_REF | OBJECT_FLAG_ATTRIBUTES | OBJECT_FLAG_COMMENT
    );
    assert_eq!(full.fixed_payload_len(), 96 + 16 + 16 + 1 + 3);
    let block = full.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(block[HEADER_SIZE + 96], 77);
    assert_eq!(block[HEADER_SIZE + 112], 8);
    assert_eq!(block[HEADER_SIZE + 128], 3);
    assert_eq!(
        &block[HEADER_SIZE + 129..HEADER_SIZE + 132],
        "Dé".as_bytes()
    );
    assert_eq!(ObjectRecord::decode(&block).unwrap(), full);

    // Removing the reference gives back the record without it, bit for bit.
    let without = full.with_attributes(None);
    assert_eq!(without.flags & OBJECT_FLAG_ATTRIBUTES, 0);
    assert_eq!(
        without.encode(DEFAULT_BLOCK_SIZE, 7).unwrap(),
        file()
            .with_security(Some(security()))
            .with_comment(Comment::new("Dé").unwrap())
            .encode(DEFAULT_BLOCK_SIZE, 7)
            .unwrap()
    );
}

#[test]
fn every_object_type_carries_the_reference() {
    let mut directory = file().with_attributes(Some(set()));
    directory.object_type = ObjectType::Directory;
    directory.data_root = 40;
    let block = directory.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(ObjectRecord::decode(&block).unwrap(), directory);

    let mut link = file().with_attributes(Some(set()));
    link.object_type = ObjectType::Symlink;
    link.size_bytes = 6;
    let symlink = SymlinkRecord {
        record: link,
        target: "target",
    };
    let block = symlink.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(&block[HEADER_SIZE + 112..HEADER_SIZE + 118], b"target");
    assert_eq!(SymlinkRecord::decode(&block).unwrap().0, symlink);
    // The reference takes 16 bytes of the longest target.
    let longest = "t".repeat(SymlinkRecord::maximum_target_bytes(DEFAULT_BLOCK_SIZE));
    let mut record = link;
    record.size_bytes = longest.len() as u64;
    assert!(SymlinkRecord {
        record,
        target: &longest
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .is_err());
}

#[test]
fn a_malformed_reference_is_refused_by_both_directions() {
    for bad in [
        AttributeRef {
            first_block: 0,
            ..set()
        },
        AttributeRef {
            total_len: 0,
            segment_count: 0,
            ..set()
        },
        AttributeRef {
            total_len: 65_537,
            segment_count: 17,
            ..set()
        },
        AttributeRef {
            segment_count: 3,
            ..set()
        },
    ] {
        assert_eq!(
            file()
                .with_attributes(Some(bad))
                .encode(DEFAULT_BLOCK_SIZE, 7),
            Err(FormatError::Invalid("invalid attribute reference")),
            "{bad:?}"
        );
        // The same fields written by hand are refused on the way in.
        let mut block = file()
            .with_attributes(Some(set()))
            .encode(DEFAULT_BLOCK_SIZE, 7)
            .unwrap();
        let at = HEADER_SIZE + 96;
        block[at..at + 8].copy_from_slice(&bad.first_block.to_le_bytes());
        block[at + 8..at + 12].copy_from_slice(&bad.total_len.to_le_bytes());
        block[at + 12..at + 14].copy_from_slice(&bad.segment_count.to_le_bytes());
        reseal(&mut block, 112);
        assert!(ObjectRecord::decode(&block).is_err(), "{bad:?}");
    }

    let good = file()
        .with_attributes(Some(set()))
        .encode(DEFAULT_BLOCK_SIZE, 7)
        .unwrap();

    let mut reserved = good.clone();
    reserved[HEADER_SIZE + 110] = 1;
    reseal(&mut reserved, 112);
    assert_eq!(
        ObjectRecord::decode(&reserved),
        Err(FormatError::Invalid(
            "attribute reference reserved field is nonzero"
        ))
    );

    // The flag without the field, and the field without the flag.
    let mut short = file().encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    short[HEADER_SIZE + 10] = 0x10;
    reseal(&mut short, 96);
    assert!(ObjectRecord::decode(&short).is_err());
    let mut unflagged = good.clone();
    unflagged[HEADER_SIZE + 10] = 0;
    reseal(&mut unflagged, 112);
    assert!(ObjectRecord::decode(&unflagged).is_err());

    let mut disagree = file().with_attributes(Some(set()));
    disagree.flags = 0;
    assert_eq!(
        disagree.encode(DEFAULT_BLOCK_SIZE, 7),
        Err(FormatError::Invalid(
            "attribute reference flag and field disagree"
        ))
    );

    // Bit 5 stays unassigned.
    let mut next_bit = file().encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    next_bit[HEADER_SIZE + 10] = 0x20;
    reseal(&mut next_bit, 96);
    assert!(ObjectRecord::decode(&next_bit).is_err());
}

#[test]
fn the_set_has_one_encoding() {
    let entries: [(&str, &[u8]); 3] = [
        ("aros.icon", b"\x01\x02"),
        ("user.empty", b""),
        ("user.note", b"hello"),
    ];
    let bytes = encode_attribute_set(&entries).unwrap();
    let mut expected = vec![3, 0, 0, 0];
    expected.extend_from_slice(&[9, 0, 2, 0]);
    expected.extend_from_slice(b"aros.icon\x01\x02");
    expected.extend_from_slice(&[10, 0, 0, 0]);
    expected.extend_from_slice(b"user.empty");
    expected.extend_from_slice(&[9, 0, 5, 0]);
    expected.extend_from_slice(b"user.notehello");
    assert_eq!(bytes, expected);
    assert_eq!(decode_attribute_set(&bytes).unwrap(), entries);

    // Every proper prefix and every extension is refused.
    for length in 0..bytes.len() {
        assert!(decode_attribute_set(&bytes[..length]).is_err(), "{length}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(
        decode_attribute_set(&longer),
        Err(FormatError::Invalid("attribute set length is not exact"))
    );
}

/// ADR-108: "One set has one encoding, so two implementations that hold the
/// same attributes write the same bytes." Encoding what a set decodes to must
/// give the bytes back, for every image the decoder accepts.
#[test]
fn every_accepted_set_is_the_only_encoding_of_its_attributes() {
    let value = vec![9u8; 300];
    let sets: [Vec<(&str, &[u8])>; 4] = [
        vec![("user.a", b"1")],
        vec![
            ("aros.icon", &value),
            ("user.empty", b""),
            ("user.note", b"hello"),
        ],
        // A name that is a proper prefix of the next, and the empty value.
        vec![("user.a", b""), ("user.ab", b"")],
        // Every namespace, in the order their bytes impose.
        vec![
            ("aros.x", b"1"),
            ("security.x", b"2"),
            ("system.x", b"3"),
            ("user.x", b"4"),
        ],
    ];
    for entries in &sets {
        let bytes = encode_attribute_set(entries).unwrap();
        let decoded = decode_attribute_set(&bytes).unwrap();
        assert_eq!(&decoded, entries);
        // The round trip the other way: encoding what the image decodes to
        // reproduces the image byte for byte.
        assert_eq!(encode_attribute_set(&decoded).unwrap(), bytes);
    }
}

#[test]
fn a_set_out_of_order_or_out_of_bounds_is_refused_by_both_directions() {
    let unsorted: [(&str, &[u8]); 2] = [("user.b", b"1"), ("user.a", b"2")];
    let duplicate: [(&str, &[u8]); 2] = [("user.a", b"1"), ("user.a", b"2")];
    for entries in [&unsorted[..], &duplicate[..], &[][..]] {
        assert!(encode_attribute_set(entries).is_err());
    }
    // Hand-written images of the same sets.
    let image = |entries: &[(&[u8], &[u8])], count: u16| {
        let mut out = count.to_le_bytes().to_vec();
        out.extend_from_slice(&[0, 0]);
        for (name, value) in entries {
            out.push(name.len() as u8);
            out.push(0);
            out.extend_from_slice(&(value.len() as u16).to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(value);
        }
        out
    };
    assert!(decode_attribute_set(&image(&[(b"user.a", b"1")], 1)).is_ok());
    let refused: [(Vec<u8>, &str); 9] = [
        (
            image(&[(b"user.b", b"1"), (b"user.a", b"2")], 2),
            "unsorted",
        ),
        (
            image(&[(b"user.a", b"1"), (b"user.a", b"2")], 2),
            "duplicate",
        ),
        (image(&[], 0), "empty"),
        (image(&[(b"user.a", b"1")], 2), "count above entries"),
        (
            image(&[(b"user.a", b"1"), (b"user.b", b"1")], 1),
            "count below",
        ),
        (image(&[(b"other.a", b"1")], 1), "unknown namespace"),
        (image(&[(b"user.", b"1")], 1), "namespace alone"),
        (image(&[(b"user.a\0", b"1")], 1), "NUL"),
        (image(&[(b"user.\xff", b"1")], 1), "not UTF-8"),
    ];
    for (bytes, what) in &refused {
        assert!(decode_attribute_set(bytes).is_err(), "{what}");
    }
    let mut reserved = image(&[(b"user.a", b"1")], 1);
    reserved[2] = 1;
    assert!(decode_attribute_set(&reserved).is_err());
    let mut entry_reserved = image(&[(b"user.a", b"1")], 1);
    entry_reserved[5] = 1;
    assert!(decode_attribute_set(&entry_reserved).is_err());

    // Two values of 65,535 bytes pass the entry bound and not the set bound.
    let big = vec![7u8; 65_535];
    assert!(
        encode_attribute_set(&[("user.a", &big[..32_000]), ("user.b", &big[..32_000])]).is_ok()
    );
    assert_eq!(
        encode_attribute_set(&[("user.a", &big), ("user.b", &big)]),
        Err(FormatError::Overflow("attribute set"))
    );
    assert!(decode_attribute_set(&vec![0u8; 65_537]).is_err());

    for name in ["user.x", "system.x", "security.x", "aros.x"] {
        validate_attribute_name(name).unwrap();
    }
    let long = format!("user.{}", "n".repeat(251));
    assert!(validate_attribute_name(&long[..255]).is_ok());
    assert!(validate_attribute_name(&long).is_err());
    for name in ["", "x", "User.x", "user", "trusted.x"] {
        assert!(validate_attribute_name(name).is_err(), "{name}");
    }
}

#[test]
fn the_set_chain_is_its_own_block_kind() {
    let bytes = encode_attribute_set(&[("user.note", b"hello")]).unwrap();
    let segment = ChainSegment {
        object_id: 16,
        format: ATTRIBUTE_SET_FORMAT,
        version: ATTRIBUTE_SET_VERSION,
        total_len: bytes.len() as u32,
        index: 0,
        count: 1,
        next: 0,
        bytes: &bytes,
    };
    let block = segment
        .encode(&ATTRIBUTE_CHAIN, DEFAULT_BLOCK_SIZE, 7)
        .unwrap();
    assert_eq!(&block[0..4], b"AFSA");
    assert_eq!(
        ChainSegment::decode(&ATTRIBUTE_CHAIN, &block).unwrap(),
        (segment, 7)
    );
    assert!(SecuritySegment::decode(&block).is_err());
}
