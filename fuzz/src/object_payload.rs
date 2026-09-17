//! Payload-aware object readers and borrowed inline symlinks.
use super::{object_seed, CodecTarget};
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::object::{ObjectRecord, ObjectType, SymlinkRecord};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE};

pub(super) fn handles(t: CodecTarget) -> bool {
    matches!(t, CodecTarget::InlineSymlink | CodecTarget::ObjectMetadata)
}
pub(super) fn seed(t: CodecTarget) -> Result<Vec<u8>, String> {
    let mut record = ObjectRecord::decode(&object_seed()?).map_err(|e| e.to_string())?;
    match t {
        CodecTarget::InlineSymlink => {
            let target = "../café/文件";
            record.object_type = ObjectType::Symlink;
            record.flags = 0;
            record.size_bytes = target.len() as u64;
            record.allocated_bytes = 0;
            record.data_root = 0;
            record.data_blocks = 0;
            SymlinkRecord { record, target }
                .encode(DEFAULT_BLOCK_SIZE, 7)
                .map_err(|e| e.to_string())
        }
        CodecTarget::ObjectMetadata => {
            record.object_type = ObjectType::Directory;
            record.size_bytes = 0;
            record.data_blocks = 0;
            record.data_root = 123;
            record
                .encode(DEFAULT_BLOCK_SIZE, 7)
                .map_err(|e| e.to_string())
        }
        _ => unreachable!(),
    }
}
fn u16_at(p: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(p[at..at + 2].try_into().unwrap())
}
fn u32_at(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(p[at..at + 4].try_into().unwrap())
}
fn u64_at(p: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(p[at..at + 8].try_into().unwrap())
}
fn timestamp(p: &[u8], at: usize) -> Option<Timespec> {
    let nanos = u32_at(p, at + 8);
    if nanos >= 1_000_000_000 {
        return None;
    }
    Some(Timespec {
        seconds: i64::from_le_bytes(p[at..at + 8].try_into().unwrap()),
        nanoseconds: nanos,
    })
}
// Header verification is shared; payload admission and extraction are independent.
fn expected(input: &[u8]) -> Option<(ObjectRecord, u64, Option<&str>)> {
    let h = BlockHeader::verify(input, block_type::OBJECT).ok()?;
    let p = h.payload(input);
    if p.len() < 96 {
        return None;
    }
    if p[9] != 0 {
        return None;
    }
    // Exact admission: zero header flags, zero tail, and no payload beyond
    // the fixed record unless the type defines one.
    if h.flags != 0 || input[HEADER_SIZE + p.len()..].iter().any(|b| *b != 0) {
        return None;
    }
    if p[8] != 3 && p.len() != 96 {
        return None;
    }
    let kind = match p[8] {
        1 => ObjectType::File,
        2 => ObjectType::Directory,
        3 => ObjectType::Symlink,
        _ => return None,
    };
    let record = ObjectRecord {
        object_id: u64_at(p, 0),
        object_type: kind,
        flags: u16_at(p, 10),
        link_count: u32_at(p, 12),
        size_bytes: u64_at(p, 16),
        allocated_bytes: u64_at(p, 24),
        created: timestamp(p, 32)?,
        modified: timestamp(p, 44)?,
        changed: timestamp(p, 56)?,
        protection: u32_at(p, 68),
        content_generation: u64_at(p, 72),
        data_root: u64_at(p, 80),
        data_blocks: u64_at(p, 88),
    };
    let r = &record;
    if r.object_id == 0 || r.object_id != h.owner || r.link_count == 0 || r.flags & !3 != 0 {
        return None;
    }
    let target = match kind {
        ObjectType::Symlink => {
            let target = std::str::from_utf8(&p[96..]).ok()?;
            if h.flags != 0
                || p[9] != 0
                || r.flags != 0
                || r.data_root != 0
                || r.data_blocks != 0
                || r.allocated_bytes != 0
                || r.size_bytes != target.len() as u64
                || target.is_empty()
                || target.as_bytes().contains(&0)
                || input[HEADER_SIZE + p.len()..].iter().any(|b| *b != 0)
            {
                return None;
            }
            Some(target)
        }
        ObjectType::Directory => {
            if r.flags != 0 || r.data_root == 0 || r.size_bytes != 0 || r.data_blocks != 0 {
                return None;
            }
            None
        }
        ObjectType::File => {
            let allocated = u128::from(r.data_blocks) * input.len() as u128;
            if allocated != u128::from(r.allocated_bytes) {
                return None;
            }
            if r.flags & 1 != 0 {
                if r.data_root == 0 {
                    return None;
                }
            } else if r.data_blocks > 4096 {
                return None;
            } else if r.data_blocks == 0 {
                if r.data_root != 0 || r.size_bytes != 0 {
                    return None;
                }
            } else if r.data_root == 0
                || u128::from(r.data_root) + u128::from(r.data_blocks) > u128::from(u64::MAX)
                || u128::from(r.size_bytes) > allocated
                || u128::from(r.size_bytes) <= (u128::from(r.data_blocks) - 1) * input.len() as u128
            {
                return None;
            }
            None
        }
        ObjectType::Internal => unreachable!(),
    };
    Some((record, h.generation, target))
}
pub(super) fn accepts(t: CodecTarget, input: &[u8]) -> bool {
    if t == CodecTarget::InlineSymlink {
        SymlinkRecord::decode(input).is_ok()
    } else {
        ObjectRecord::decode_metadata_with_generation(input).is_ok()
    }
}
pub(super) fn exercise(t: CodecTarget, input: &[u8]) -> Result<(), String> {
    let wanted = expected(input);
    let metadata = ObjectRecord::decode_metadata_with_generation(input).ok();
    if metadata != wanted.as_ref().map(|(r, g, _)| (*r, *g)) {
        return Err("object metadata independent admission/fields mismatch".into());
    }
    let borrowed = SymlinkRecord::decode(input).ok();
    let expected_link = wanted.and_then(|(r, g, target)| {
        target.map(|s| {
            (
                SymlinkRecord {
                    record: r,
                    target: s,
                },
                g,
            )
        })
    });
    if borrowed != expected_link {
        return Err("symlink independent admission/fields mismatch".into());
    }
    let generic = ObjectRecord::decode_with_generation(input).ok();
    if generic != metadata.filter(|(r, _)| r.object_type != ObjectType::Symlink) {
        return Err("generic/metadata object boundary mismatch".into());
    }
    if let Some((record, generation, target)) = wanted {
        let bytes = if let Some(target) = target {
            let (link, _) = borrowed.unwrap();
            if link.target.as_ptr() != input[HEADER_SIZE + 96..].as_ptr() {
                return Err("symlink target is not borrowed from payload".into());
            }
            SymlinkRecord { record, target }.encode(input.len(), generation)
        } else {
            record.encode(input.len(), generation)
        }
        .map_err(|e| e.to_string())?;
        if expected(&bytes) != wanted {
            return Err("object canonical field mismatch".into());
        }
        if t == CodecTarget::InlineSymlink && target.is_none() && accepts(t, input) {
            return Err("non-symlink accepted as symlink".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reseal(bytes: &mut [u8]) {
        let mut clean = bytes.to_vec();
        clean[28..32].fill(0);
        let crc = afsplus_format::crc32c::crc32c(&clean);
        bytes[28..32].copy_from_slice(&crc.to_le_bytes());
    }
    #[test]
    fn symlink_utf8_lengths_flags_tail_and_short_buffers_are_checked() {
        let bytes = seed(CodecTarget::InlineSymlink).unwrap();
        for len in 0..=bytes.len() {
            exercise(CodecTarget::InlineSymlink, &bytes[..len]).unwrap();
        }
        for at in [
            HEADER_SIZE + 9,
            HEADER_SIZE + 10,
            HEADER_SIZE + 12,
            HEADER_SIZE + 16,
            HEADER_SIZE + 24,
            HEADER_SIZE + 80,
            HEADER_SIZE + 88,
            HEADER_SIZE + 96,
            DEFAULT_BLOCK_SIZE - 1,
        ] {
            let mut bad = bytes.clone();
            bad[at] ^= 0xff;
            reseal(&mut bad);
            exercise(CodecTarget::InlineSymlink, &bad).unwrap();
        }
        for target in [
            vec![],
            vec![0],
            vec![0xff],
            vec![0xc0, 0xaf],
            vec![0xed, 0xa0, 0x80],
            vec![0xf4, 0x90, 0x80, 0x80],
            vec![0xc3],
            b"valid".to_vec(),
            "é".as_bytes().to_vec(),
        ] {
            let mut value = bytes.clone();
            let old = BlockHeader::verify(&value, block_type::OBJECT).unwrap();
            value[HEADER_SIZE + 96..].fill(0);
            value[HEADER_SIZE + 96..HEADER_SIZE + 96 + target.len()].copy_from_slice(&target);
            value[HEADER_SIZE + 16..HEADER_SIZE + 24]
                .copy_from_slice(&(target.len() as u64).to_le_bytes());
            BlockHeader {
                payload_len: 96 + target.len() as u32,
                ..old
            }
            .seal(&mut value);
            let valid =
                !target.is_empty() && !target.contains(&0) && std::str::from_utf8(&target).is_ok();
            assert_eq!(accepts(CodecTarget::InlineSymlink, &value), valid);
            exercise(CodecTarget::InlineSymlink, &value).unwrap();
        }
        let mut bad_header = bytes.clone();
        let header = BlockHeader::verify(&bad_header, block_type::OBJECT).unwrap();
        BlockHeader { flags: 1, ..header }.seal(&mut bad_header);
        assert!(!accepts(CodecTarget::InlineSymlink, &bad_header));
        exercise(CodecTarget::InlineSymlink, &bad_header).unwrap();
        let (link, generation) = SymlinkRecord::decode(&bytes).unwrap();
        for size in 0..HEADER_SIZE + 96 + link.target.len() {
            assert!(link.encode(size, generation).is_err());
        }
        let boundary = link
            .encode(HEADER_SIZE + 96 + link.target.len(), generation)
            .unwrap();
        exercise(CodecTarget::InlineSymlink, &boundary).unwrap();
        let maximum = "a".repeat(DEFAULT_BLOCK_SIZE - HEADER_SIZE - 96);
        let mut record = link.record;
        record.size_bytes = maximum.len() as u64;
        let boundary = SymlinkRecord {
            record,
            target: &maximum,
        }
        .encode(DEFAULT_BLOCK_SIZE, 7)
        .unwrap();
        exercise(CodecTarget::InlineSymlink, &boundary).unwrap();
    }
    #[test]
    fn explicit_object_types_and_layouts_use_metadata_reader() {
        let file = ObjectRecord::decode(&object_seed().unwrap()).unwrap();
        let directory = ObjectRecord::decode(&seed(CodecTarget::ObjectMetadata).unwrap()).unwrap();
        let mut empty = file;
        empty.size_bytes = 0;
        empty.data_blocks = 0;
        empty.data_root = 0;
        empty.allocated_bytes = 0;
        let mut tree = empty;
        tree.flags = 1;
        tree.data_root = 123;
        tree.size_bytes = 8192;
        let mut policy = file;
        policy.flags = 2;
        for record in [file, directory, empty, tree, policy] {
            let bytes = record.encode(DEFAULT_BLOCK_SIZE, 7).unwrap();
            exercise(CodecTarget::ObjectMetadata, &bytes).unwrap();
            for size in 0..128 {
                assert!(record.encode(size, 7).is_err());
            }
            for offset in [8, 9, 10, 12, 40, 52, 64, 80, 88] {
                let mut changed = bytes.clone();
                changed[HEADER_SIZE + offset] ^= 0xff;
                reseal(&mut changed);
                exercise(CodecTarget::ObjectMetadata, &changed).unwrap();
            }
        }
        let mut internal = seed(CodecTarget::ObjectMetadata).unwrap();
        internal[HEADER_SIZE + 8] = 4;
        reseal(&mut internal);
        assert!(!accepts(CodecTarget::ObjectMetadata, &internal));
        exercise(CodecTarget::ObjectMetadata, &internal).unwrap();
    }
}
