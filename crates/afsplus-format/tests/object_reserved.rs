use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE};
#[test]
fn generic_and_metadata_readers_reject_reserved_object_byte() {
    for kind in [ObjectType::File, ObjectType::Directory] {
        let record = ObjectRecord {
            object_id: 16,
            object_type: kind,
            flags: 0,
            link_count: 1,
            size_bytes: 0,
            allocated_bytes: 0,
            created: Timespec::default(),
            modified: Timespec::default(),
            changed: Timespec::default(),
            protection: 0,
            content_generation: 1,
            data_root: if kind == ObjectType::Directory {
                123
            } else {
                0
            },
            data_blocks: 0,
        };
        let bytes = record.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
        let header = BlockHeader::verify(&bytes, block_type::OBJECT).unwrap();
        for value in [1, 0x80, 0xff] {
            let mut bad = bytes.clone();
            bad[HEADER_SIZE + 9] = value;
            header.seal(&mut bad);
            BlockHeader::verify(&bad, block_type::OBJECT).unwrap();
            assert!(ObjectRecord::decode(&bad).is_err());
            assert!(ObjectRecord::decode_with_generation(&bad).is_err());
            assert!(ObjectRecord::decode_metadata_with_generation(&bad).is_err());
            bad[HEADER_SIZE + 9] = 0;
            header.seal(&mut bad);
            assert_eq!(bad, bytes);
            assert_eq!(
                ObjectRecord::decode_metadata_with_generation(&bad).unwrap(),
                (record, 7)
            );
        }
    }
}
