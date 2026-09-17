//! Wire image of the object record's comment field (ADR-106).
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::object::{
    Comment, ObjectRecord, ObjectType, SecurityRef, SymlinkRecord, OBJECT_FLAG_COMMENT,
    OBJECT_FLAG_SECURITY_REF,
};
use afsplus_format::{FormatError, Timespec, DEFAULT_BLOCK_SIZE};

fn file() -> ObjectRecord {
    ObjectRecord {
        object_id: 16,
        object_type: ObjectType::File,
        flags: 0,
        link_count: 1,
        owner_uid: 0,
        owner_gid: 0,
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

fn reference() -> SecurityRef {
    SecurityRef {
        first_block: 77,
        total_len: 10,
        segment_count: 1,
        flags: 0,
    }
}

/// A corruption of an encoded block; returns the payload length to reseal with.
type Damage = Box<dyn Fn(&mut Vec<u8>) -> u32>;

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
fn the_comment_follows_the_fixed_payload_and_the_security_reference() {
    let record = file().with_comment(Comment::new("Dé").unwrap());
    assert_eq!(record.flags, OBJECT_FLAG_COMMENT);
    let block = record.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(&block[24..28], &108u32.to_le_bytes());
    assert_eq!(&block[HEADER_SIZE + 10..HEADER_SIZE + 12], &[0x08, 0x00]);
    assert_eq!(
        &block[HEADER_SIZE + 104..HEADER_SIZE + 108],
        &[3, b'D', 0xc3, 0xa9]
    );
    assert!(block[HEADER_SIZE + 108..].iter().all(|b| *b == 0));
    assert_eq!(ObjectRecord::decode(&block), Ok(record));
    assert_eq!(ObjectRecord::decode(&block).unwrap().comment.as_str(), "Dé");

    // With a security reference the comment starts at 120.
    let both = record.with_security(Some(reference()));
    assert_eq!(both.flags, OBJECT_FLAG_COMMENT | OBJECT_FLAG_SECURITY_REF);
    let block = both.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(&block[24..28], &124u32.to_le_bytes());
    assert_eq!(
        &block[HEADER_SIZE + 120..HEADER_SIZE + 124],
        &[3, b'D', 0xc3, 0xa9]
    );
    assert_eq!(ObjectRecord::decode(&block), Ok(both));

    // A record without a comment keeps its image, and removal restores it.
    assert_eq!(record.with_comment(Comment::EMPTY), file());
    assert_eq!(
        &file().encode(DEFAULT_BLOCK_SIZE, 7).unwrap()[24..28],
        &104u32.to_le_bytes()
    );
}

#[test]
fn a_symlink_keeps_its_target_after_the_comment() {
    let mut link = file();
    link.object_type = ObjectType::Symlink;
    link.size_bytes = 6;
    let link = link.with_comment(Comment::new("note").unwrap());
    let block = SymlinkRecord {
        record: link,
        target: "target",
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .unwrap();
    assert_eq!(&block[24..28], &(104u32 + 5 + 6).to_le_bytes());
    assert_eq!(
        &block[HEADER_SIZE + 104..HEADER_SIZE + 109],
        &[4, b'n', b'o', b't', b'e']
    );
    assert_eq!(&block[HEADER_SIZE + 109..HEADER_SIZE + 115], b"target");
    let (decoded, _) = SymlinkRecord::decode(&block).unwrap();
    assert_eq!((decoded.record, decoded.target), (link, "target"));
    assert_eq!(
        ObjectRecord::decode_metadata_with_generation(&block),
        Ok((link, 7))
    );

    // ADR-106: "A symlink's longest target shrinks by the comment's wire
    // length." Five bytes of comment cost five bytes of target, exactly.
    let room = SymlinkRecord::maximum_target_bytes(DEFAULT_BLOCK_SIZE);
    for (comment, wire) in [("", 0usize), ("note", 5), (&"x".repeat(255), 256)] {
        let mut record = file();
        record.object_type = ObjectType::Symlink;
        let record = record.with_comment(Comment::new(comment).unwrap());
        let longest = "t".repeat(room - wire);
        let mut fits = record;
        fits.size_bytes = longest.len() as u64;
        assert!(
            SymlinkRecord {
                record: fits,
                target: &longest,
            }
            .encode(DEFAULT_BLOCK_SIZE, 7)
            .is_ok(),
            "a target of {} bytes with a {wire}-byte comment field",
            longest.len()
        );
        let one_more = "t".repeat(room - wire + 1);
        let mut over = record;
        over.size_bytes = one_more.len() as u64;
        assert!(
            SymlinkRecord {
                record: over,
                target: &one_more,
            }
            .encode(DEFAULT_BLOCK_SIZE, 7)
            .is_err(),
            "one byte past the room a {wire}-byte comment field leaves"
        );
    }
}

#[test]
fn bounds_and_refusals() {
    let longest = "é".repeat(127) + "x";
    assert_eq!(longest.len(), 255);
    let record = file().with_comment(Comment::new(&longest).unwrap());
    let block = record.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
    assert_eq!(
        ObjectRecord::decode(&block).unwrap().comment.as_str(),
        longest
    );
    assert_eq!(
        Comment::new(&"x".repeat(256)),
        Err(FormatError::Overflow("object comment"))
    );
    assert_eq!(
        Comment::new("a\0b"),
        Err(FormatError::Invalid("object comment contains NUL"))
    );
    assert!(Comment::new("").unwrap().is_empty());

    // Flag and field must agree in both directions.
    let mut flag_only = file();
    flag_only.flags = OBJECT_FLAG_COMMENT;
    let mut field_only = file();
    field_only.comment = Comment::new("x").unwrap();
    for record in [flag_only, field_only] {
        assert_eq!(
            record.encode(DEFAULT_BLOCK_SIZE, 7),
            Err(FormatError::Invalid("comment flag and field disagree"))
        );
    }

    // Resealed wire corruptions of a record with the comment "note".
    let clean = file()
        .with_comment(Comment::new("note").unwrap())
        .encode(DEFAULT_BLOCK_SIZE, 7)
        .unwrap();
    let cases: [(&str, Damage); 6] = [
        (
            "zero length under the flag",
            Box::new(|b| {
                b[HEADER_SIZE + 104] = 0;
                b[HEADER_SIZE + 97..HEADER_SIZE + 101].fill(0);
                97
            }),
        ),
        (
            "length past the payload",
            Box::new(|b| {
                b[HEADER_SIZE + 104] = 5;
                101
            }),
        ),
        ("payload longer than the comment", Box::new(|_| 102)),
        (
            "NUL inside",
            Box::new(|b| {
                b[HEADER_SIZE + 98] = 0;
                101
            }),
        ),
        (
            "invalid UTF-8",
            Box::new(|b| {
                b[HEADER_SIZE + 98] = 0xff;
                101
            }),
        ),
        (
            "flag cleared with the bytes left",
            Box::new(|b| {
                b[HEADER_SIZE + 10] = 0;
                101
            }),
        ),
    ];
    for (what, damage) in cases {
        let mut block = clean.clone();
        let payload = damage(&mut block);
        reseal(&mut block, payload);
        assert!(ObjectRecord::decode(&block).is_err(), "{what}");
        assert!(
            ObjectRecord::decode_metadata_with_generation(&block).is_err(),
            "{what}"
        );
    }
}
