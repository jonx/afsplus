use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::reclaim::{
    ReclaimCaps, ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable, SegmentRef,
};
use afsplus_format::{FormatError, DEFAULT_BLOCK_SIZE};

#[test]
fn reclaim_reserved_bytes_are_rejected_after_checksum_verification() {
    let root = ReclaimRoot::empty(ReclaimCaps::default())
        .encode(DEFAULT_BLOCK_SIZE, 7)
        .unwrap();
    let segment = ReclaimSegment {
        entries: vec![ReclaimEntry {
            start: 123,
            blocks: 4,
            retire_generation: 6,
        }],
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .unwrap();
    let table = ReclaimTable {
        refs: vec![SegmentRef {
            lba: 234,
            entry_count: 1,
        }],
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .unwrap();
    for (kind, bytes, offsets) in [
        (block_type::RECLAIM_ROOT, root, vec![4, 5, 6, 7, 50, 51]),
        (block_type::RECLAIM_SEGMENT, segment, vec![4, 5, 6, 7]),
        (block_type::RECLAIM_TABLE, table, vec![4, 5, 6, 7]),
    ] {
        let decode = |input: &[u8]| match kind {
            block_type::RECLAIM_ROOT => ReclaimRoot::decode(input).map(|_| ()),
            block_type::RECLAIM_SEGMENT => ReclaimSegment::decode(input).map(|_| ()),
            _ => ReclaimTable::decode(input).map(|_| ()),
        };
        assert!(decode(&bytes).is_ok());
        let header = BlockHeader::verify(&bytes, kind).unwrap();
        for offset in offsets {
            for value in [1, 0x80, 0xff] {
                let mut malformed = bytes.clone();
                malformed[HEADER_SIZE + offset] = value;
                header.seal(&mut malformed);
                BlockHeader::verify(&malformed, kind).unwrap();
                assert!(
                    matches!(decode(&malformed),Err(FormatError::Invalid(reason)) if reason.contains("reserved bytes")),
                    "kind{kind} offset{offset} value{value}"
                );
                malformed[HEADER_SIZE + offset] = 0;
                header.seal(&mut malformed);
                assert_eq!(malformed, bytes);
                assert!(decode(&malformed).is_ok());
            }
        }
    }
}
