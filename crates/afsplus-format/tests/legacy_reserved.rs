use afsplus_format::dir::{DirBlock, DirEntry};
use afsplus_format::header::HEADER_SIZE;
use afsplus_format::omap::ObjectMap;
use afsplus_format::retired::RetiredList;

#[test]
fn legacy_readers_reject_resealed_reserved_bytes() {
    let mut dir = DirBlock::new(1);
    dir.insert(DirEntry {
        key: b"a".to_vec(),
        name: b"a".to_vec(),
        child_type_hint: 1,
        child_id: 16,
    })
    .unwrap();
    let mut map = ObjectMap::default();
    map.upsert(16, 32).unwrap();
    let mut retired = RetiredList::default();
    retired.insert(33, 7).unwrap();
    for (kind, bytes, offsets) in [
        (
            0,
            dir.encode(4096, 7).unwrap(),
            vec![4, 5, 6, 7, 13, 14, 15],
        ),
        (1, map.encode(4096, 7).unwrap(), vec![4, 5, 6, 7]),
        (2, retired.encode(4096, 7).unwrap(), vec![4, 5, 6, 7]),
    ] {
        let accepts = |bytes: &[u8]| match kind {
            0 => DirBlock::decode(bytes).is_ok(),
            1 => ObjectMap::decode(bytes).is_ok(),
            _ => RetiredList::decode(bytes).is_ok(),
        };
        assert!(accepts(&bytes));
        for offset in offsets {
            for value in [1, 0x80, 0xff] {
                let mut changed = bytes.clone();
                changed[HEADER_SIZE + offset] = value;
                changed[28..32].fill(0);
                let crc = afsplus_format::crc32c::crc32c(&changed);
                changed[28..32].copy_from_slice(&crc.to_le_bytes());
                assert!(
                    !accepts(&changed),
                    "kind={kind} offset={offset} value={value}"
                );
            }
        }
    }
}
