//! Independent payload admission for executable legacy one-block readers.
use super::CodecTarget;
use afsplus_format::dir::{DirBlock, DirEntry};
use afsplus_format::header::{BlockHeader, HEADER_SIZE};
use afsplus_format::omap::{ObjectMap, OmapEntry};
use afsplus_format::retired::{RetiredEntry, RetiredList};

#[derive(Debug, PartialEq)]
enum Value {
    Directory(DirBlock),
    Map(ObjectMap),
    Retired(RetiredList),
}
impl Value {
    fn encode(&self, size: usize, generation: u64) -> Result<Vec<u8>, String> {
        match self {
            Self::Directory(v) => v.encode(size, generation),
            Self::Map(v) => v.encode(size, generation),
            Self::Retired(v) => v.encode(size, generation),
        }
        .map_err(|e| e.to_string())
    }
}
pub(super) fn handles(t: CodecTarget) -> bool {
    matches!(
        t,
        CodecTarget::LegacyDirectory | CodecTarget::LegacyObjectMap | CodecTarget::LegacyRetired
    )
}
pub(super) fn seed(t: CodecTarget) -> Result<Vec<u8>, String> {
    let value = match t {
        CodecTarget::LegacyDirectory => Value::Directory(DirBlock {
            owner: 1,
            entries: vec![
                DirEntry {
                    key: b"a".to_vec(),
                    name: b"a".to_vec(),
                    child_type_hint: 1,
                    child_id: 16,
                },
                DirEntry {
                    key: b"z".to_vec(),
                    name: b"z".to_vec(),
                    child_type_hint: 2,
                    child_id: 17,
                },
            ],
        }),
        CodecTarget::LegacyObjectMap => Value::Map(ObjectMap {
            entries: vec![
                OmapEntry {
                    object_id: 16,
                    block: 32,
                },
                OmapEntry {
                    object_id: 17,
                    block: 40,
                },
            ],
        }),
        CodecTarget::LegacyRetired => Value::Retired(RetiredList {
            entries: vec![
                RetiredEntry {
                    lba: 32,
                    retire_generation: 3,
                },
                RetiredEntry {
                    lba: 40,
                    retire_generation: 5,
                },
            ],
        }),
        _ => unreachable!(),
    };
    value.encode(4096, 7)
}
fn u64_at(p: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(p[at..at + 8].try_into().unwrap())
}
fn expected(t: CodecTarget, bytes: &[u8]) -> Option<Value> {
    let h = BlockHeader::verify(bytes, t.block_type()).ok()?;
    let p = &bytes[HEADER_SIZE..HEADER_SIZE + h.payload_len as usize];
    if p.len() < 8 || p[4..8].iter().any(|b| *b != 0) {
        return None;
    }
    let count = u32::from_le_bytes(p[..4].try_into().unwrap()) as usize;
    if count > (p.len() - 8) / 16 {
        return None;
    }
    if t == CodecTarget::LegacyDirectory {
        let mut entries: Vec<DirEntry> = Vec::new();
        let mut at = 8;
        for _ in 0..count {
            if p.len() - at < 16 {
                return None;
            }
            let kl = u16::from_le_bytes(p[at..at + 2].try_into().unwrap()) as usize;
            let nl = u16::from_le_bytes(p[at + 2..at + 4].try_into().unwrap()) as usize;
            let hint = p[at + 4];
            let id = u64_at(p, at + 8);
            if p[at + 5..at + 8].iter().any(|b| *b != 0) || id == 0 {
                return None;
            }
            at += 16;
            if kl == 0
                || kl > 1020
                || nl == 0
                || nl > 255
                || kl > p.len() - at
                || nl > p.len() - at - kl
            {
                return None;
            }
            let key = p[at..at + kl].to_vec();
            at += kl;
            let name = p[at..at + nl].to_vec();
            at += nl;
            if std::str::from_utf8(&name).is_err()
                || name.contains(&0)
                || name.contains(&b'/')
                || entries.last().is_some_and(|e| e.key >= key)
            {
                return None;
            }
            entries.push(DirEntry {
                key,
                name,
                child_type_hint: hint,
                child_id: id,
            });
        }
        if at != p.len() {
            return None;
        }
        return Some(Value::Directory(DirBlock {
            owner: h.owner,
            entries,
        }));
    }
    if p.len() != 8 + count * 16 {
        return None;
    }
    let pairs: Vec<_> = (0..count)
        .map(|i| (u64_at(p, 8 + i * 16), u64_at(p, 16 + i * 16)))
        .collect();
    if pairs.windows(2).any(|v| v[0].0 >= v[1].0) {
        return None;
    }
    match t {
        CodecTarget::LegacyObjectMap if pairs.iter().all(|v| v.0 != 0) => {
            Some(Value::Map(ObjectMap {
                entries: pairs
                    .into_iter()
                    .map(|(object_id, block)| OmapEntry { object_id, block })
                    .collect(),
            }))
        }
        CodecTarget::LegacyRetired if pairs.iter().all(|v| v.1 != 0) => {
            Some(Value::Retired(RetiredList {
                entries: pairs
                    .into_iter()
                    .map(|(lba, retire_generation)| RetiredEntry {
                        lba,
                        retire_generation,
                    })
                    .collect(),
            }))
        }
        _ => None,
    }
}
fn decoded(t: CodecTarget, bytes: &[u8]) -> Option<Value> {
    match t {
        CodecTarget::LegacyDirectory => DirBlock::decode(bytes).ok().map(Value::Directory),
        CodecTarget::LegacyObjectMap => ObjectMap::decode(bytes).ok().map(Value::Map),
        CodecTarget::LegacyRetired => RetiredList::decode(bytes).ok().map(Value::Retired),
        _ => unreachable!(),
    }
}
pub(super) fn accepts(t: CodecTarget, bytes: &[u8]) -> bool {
    decoded(t, bytes).is_some()
}
pub(super) fn exercise(t: CodecTarget, bytes: &[u8]) -> Result<(), String> {
    let actual = decoded(t, bytes);
    if actual != expected(t, bytes) {
        return Err("legacy independent admission/fields mismatch".into());
    }
    if let Some(value) = actual {
        let generation = u64_at(bytes, 16);
        let canonical = value.encode(bytes.len(), generation)?;
        if decoded(t, &canonical) != Some(value) {
            return Err("legacy canonical mismatch".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_lengths_reserved_fields_and_order_have_independent_oracles() {
        for t in [
            CodecTarget::LegacyDirectory,
            CodecTarget::LegacyObjectMap,
            CodecTarget::LegacyRetired,
        ] {
            let bytes = seed(t).unwrap();
            for end in 0..=bytes.len() {
                exercise(t, &bytes[..end]).unwrap();
            }
            let header = BlockHeader::verify(&bytes, t.block_type()).unwrap();
            for offset in [0, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 23, 24, 31] {
                let mut changed = bytes.clone();
                changed[HEADER_SIZE + offset] ^= 0xff;
                header.seal(&mut changed);
                exercise(t, &changed).unwrap();
                if (4..8).contains(&offset) {
                    assert!(!accepts(t, &changed));
                }
            }
            for len in 0..=80 {
                let mut changed = bytes.clone();
                BlockHeader {
                    payload_len: len,
                    ..header
                }
                .seal(&mut changed);
                exercise(t, &changed).unwrap();
            }
            let value = decoded(t, &bytes).unwrap();
            let minimum = HEADER_SIZE + header.payload_len as usize;
            for size in 0..minimum {
                assert!(value.encode(size, 7).is_err());
            }
            let exact = value.encode(minimum, 7).unwrap();
            exercise(t, &exact).unwrap();
            assert!(accepts(t, &exact));
        }
    }
}
