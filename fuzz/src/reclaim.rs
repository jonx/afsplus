//! Reclaim wire admission, separate from the caller's queue and volume checks.
use super::CodecTarget;
use afsplus_format::header::{BlockHeader, HEADER_SIZE};
use afsplus_format::reclaim::{
    ReclaimCaps, ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable, SegmentRef, TableRef,
};
use afsplus_format::DEFAULT_BLOCK_SIZE;

pub(super) fn handles(t: CodecTarget) -> bool {
    matches!(
        t,
        CodecTarget::ReclaimRoot | CodecTarget::ReclaimSegment | CodecTarget::ReclaimTable
    )
}

#[derive(Debug, PartialEq)]
enum Value {
    Root(ReclaimRoot),
    Segment(ReclaimSegment),
    Table(ReclaimTable),
}
impl Value {
    fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, String> {
        match self {
            Self::Root(v) => v.encode(block_size, generation),
            Self::Segment(v) => v.encode(block_size, generation),
            Self::Table(v) => v.encode(block_size, generation),
        }
        .map_err(|e| e.to_string())
    }
}

pub(super) fn seed(t: CodecTarget) -> Result<Vec<u8>, String> {
    let entries = vec![
        ReclaimEntry {
            start: 123,
            blocks: 3,
            retire_generation: 5,
        },
        ReclaimEntry {
            start: 900,
            blocks: 7,
            retire_generation: 11,
        },
    ];
    let refs = vec![
        SegmentRef {
            lba: 333,
            entry_count: 2,
        },
        SegmentRef {
            lba: 444,
            entry_count: 202,
        },
    ];
    let value = match t {
        CodecTarget::ReclaimRoot => Value::Root(ReclaimRoot {
            pending_blocks: 17,
            appended_blocks_total: 23,
            reclaimed_blocks_total: 6,
            head_segment_offset: 1,
            head_entry_offset: 1,
            head_block_offset: 2,
            caps: ReclaimCaps {
                inline_entries: 3,
                segment_refs: 3,
                table_refs: 2,
            },
            table_refs: vec![TableRef {
                lba: 222,
                ref_count: 3,
            }],
            segment_refs: refs,
            inline_entries: entries,
        }),
        CodecTarget::ReclaimSegment => Value::Segment(ReclaimSegment { entries }),
        CodecTarget::ReclaimTable => Value::Table(ReclaimTable { refs }),
        _ => unreachable!(),
    };
    value.encode(DEFAULT_BLOCK_SIZE, 17)
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
fn entry(p: &[u8], at: usize) -> Option<ReclaimEntry> {
    let (start, blocks, retire_generation) = (u64_at(p, at), u32_at(p, at + 8), u64_at(p, at + 12));
    if blocks == 0
        || retire_generation == 0
        || u128::from(start) + u128::from(blocks) > u128::from(u64::MAX)
    {
        return None;
    }
    Some(ReclaimEntry {
        start,
        blocks,
        retire_generation,
    })
}
fn segment_ref(p: &[u8], at: usize) -> Option<SegmentRef> {
    let entry_count = u32_at(p, at + 8);
    (1..=202).contains(&entry_count).then_some(SegmentRef {
        lba: u64_at(p, at),
        entry_count,
    })
}

// The common header is separately qualified. All payload predicates and field
// extraction below are independent of reclaim decode/validate/encode methods.
fn expected(t: CodecTarget, input: &[u8]) -> Option<(Value, u64)> {
    let header = BlockHeader::verify(input, t.block_type()).ok()?;
    let p = &input[HEADER_SIZE..HEADER_SIZE + header.payload_len as usize];
    // Exact admission (ADR-110): no header flags, no owner, a zero tail.
    if header.flags != 0
        || header.owner != 0
        || input[HEADER_SIZE + p.len()..].iter().any(|&b| b != 0)
    {
        return None;
    }
    let value = match t {
        CodecTarget::ReclaimRoot => {
            if p.len() < 64 || u32_at(p, 0) != 1 {
                return None;
            }
            if p[4..8].iter().chain(&p[50..52]).any(|&b| b != 0) {
                return None;
            }
            let (ic, sc, tc) = (u16_at(p, 44), u16_at(p, 46), u16_at(p, 48));
            let size = 64 + usize::from(ic) * 20 + (usize::from(sc) + usize::from(tc)) * 12;
            if ic == 0 || sc == 0 || tc == 0 || size > input.len() - HEADER_SIZE || size != p.len()
            {
                return None;
            }
            let (tn, sn, en) = (u32_at(p, 52), u32_at(p, 56), u32_at(p, 60));
            if tn > u32::from(tc) || sn > u32::from(sc) || en > u32::from(ic) {
                return None;
            }
            let mut tables = Vec::new();
            for i in 0..tn as usize {
                let at = 64 + i * 12;
                let count = u32_at(p, at + 8);
                if !(1..=338).contains(&count) {
                    return None;
                }
                tables.push(TableRef {
                    lba: u64_at(p, at),
                    ref_count: count,
                });
            }
            let mut segments = Vec::new();
            for i in 0..sn as usize {
                segments.push(segment_ref(p, 64 + usize::from(tc) * 12 + i * 12)?);
            }
            let mut entries = Vec::new();
            for i in 0..en as usize {
                entries.push(entry(
                    p,
                    64 + (usize::from(tc) + usize::from(sc)) * 12 + i * 20,
                )?);
            }
            // Unused slots of the three areas are zero.
            let (t0, s0) = (
                64 + usize::from(tc) * 12,
                64 + (usize::from(tc) + usize::from(sc)) * 12,
            );
            if p[64 + tn as usize * 12..t0].iter().any(|&b| b != 0)
                || p[t0 + sn as usize * 12..s0].iter().any(|&b| b != 0)
                || p[s0 + en as usize * 20..].iter().any(|&b| b != 0)
            {
                return None;
            }
            let (hs, he, hb) = (u32_at(p, 32), u32_at(p, 36), u32_at(p, 40));
            if (tn == 0 && hs != 0)
                || (tn != 0 && hs >= tables[0].ref_count)
                || (tn == 0 && sn == 0 && (he != 0 || hb != 0))
                || (tn == 0 && sn != 0 && he >= segments[0].entry_count)
            {
                return None;
            }
            let (pending, appended, reclaimed) = (u64_at(p, 8), u64_at(p, 16), u64_at(p, 24));
            if u128::from(reclaimed) + u128::from(pending) != u128::from(appended) {
                return None;
            }
            Value::Root(ReclaimRoot {
                pending_blocks: pending,
                appended_blocks_total: appended,
                reclaimed_blocks_total: reclaimed,
                head_segment_offset: hs,
                head_entry_offset: he,
                head_block_offset: hb,
                caps: ReclaimCaps {
                    inline_entries: ic,
                    segment_refs: sc,
                    table_refs: tc,
                },
                table_refs: tables,
                segment_refs: segments,
                inline_entries: entries,
            })
        }
        CodecTarget::ReclaimSegment => {
            if p.len() < 8 {
                return None;
            }
            if p[4..8].iter().any(|&b| b != 0) {
                return None;
            }
            let n = u32_at(p, 0) as usize;
            if !(1..=202).contains(&n) || p.len() != 8 + n * 20 {
                return None;
            }
            let mut entries = Vec::new();
            for i in 0..n {
                entries.push(entry(p, 8 + i * 20)?);
            }
            Value::Segment(ReclaimSegment { entries })
        }
        CodecTarget::ReclaimTable => {
            if p.len() < 8 {
                return None;
            }
            if p[4..8].iter().any(|&b| b != 0) {
                return None;
            }
            let n = u32_at(p, 0) as usize;
            if !(1..=338).contains(&n) || p.len() != 8 + n * 12 {
                return None;
            }
            let mut refs = Vec::new();
            for i in 0..n {
                refs.push(segment_ref(p, 8 + i * 12)?);
            }
            Value::Table(ReclaimTable { refs })
        }
        _ => unreachable!(),
    };
    Some((value, u64_at(input, 16)))
}
fn decoded(t: CodecTarget, input: &[u8]) -> Option<(Value, u64)> {
    match t {
        CodecTarget::ReclaimRoot => ReclaimRoot::decode(input)
            .ok()
            .map(|(v, g)| (Value::Root(v), g)),
        CodecTarget::ReclaimSegment => ReclaimSegment::decode(input)
            .ok()
            .map(|(v, g)| (Value::Segment(v), g)),
        CodecTarget::ReclaimTable => ReclaimTable::decode(input)
            .ok()
            .map(|(v, g)| (Value::Table(v), g)),
        _ => unreachable!(),
    }
}
pub(super) fn accepts(t: CodecTarget, input: &[u8]) -> bool {
    decoded(t, input).is_some()
}
pub(super) fn exercise(t: CodecTarget, input: &[u8]) -> Result<(), String> {
    let actual = decoded(t, input);
    let wanted = expected(t, input);
    if actual != wanted {
        return Err(format!("{t}: independent admission or field mismatch"));
    }
    if let Some((value, generation)) = actual {
        let canonical = value.encode(input.len(), generation)?;
        if decoded(t, &canonical) != Some((value, generation))
            || expected(t, &canonical) != decoded(t, &canonical)
        {
            return Err(format!("{t}: canonical roundtrip mismatch"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit(t: CodecTarget, offset: usize, value: u64, width: usize) -> Vec<u8> {
        let mut bytes = seed(t).unwrap();
        let header = BlockHeader::verify(&bytes, t.block_type()).unwrap();
        bytes[HEADER_SIZE + offset..HEADER_SIZE + offset + width]
            .copy_from_slice(&value.to_le_bytes()[..width]);
        header.seal(&mut bytes);
        bytes
    }
    #[test]
    fn resealed_lengths_counts_totals_and_cursors_obey_independent_oracles() {
        for t in [
            CodecTarget::ReclaimRoot,
            CodecTarget::ReclaimSegment,
            CodecTarget::ReclaimTable,
        ] {
            let bytes = seed(t).unwrap();
            for n in 0..=bytes.len() {
                exercise(t, &bytes[..n]).unwrap();
            }
            let header = BlockHeader::verify(&bytes, t.block_type()).unwrap();
            for offset in 0..header.payload_len as usize {
                let mut changed = bytes.clone();
                changed[HEADER_SIZE + offset] ^= 0xff;
                header.seal(&mut changed);
                exercise(t, &changed).unwrap();
            }
            for n in [
                0,
                1,
                7,
                8,
                31,
                32,
                63,
                64,
                header.payload_len as usize - 1,
                header.payload_len as usize + 1,
            ] {
                let mut changed = bytes.clone();
                BlockHeader {
                    payload_len: n as u32,
                    ..header
                }
                .seal(&mut changed);
                exercise(t, &changed).unwrap();
            }
        }
        for (offset, width, values) in [
            (0, 4, vec![0, 2]),
            (8, 8, vec![0, u64::MAX]),
            (16, 8, vec![0, 5, u64::MAX]),
            (24, 8, vec![24, u64::MAX]),
            (32, 4, vec![3, u32::MAX as u64]),
            (44, 2, vec![0, u16::MAX as u64]),
            (46, 2, vec![0, u16::MAX as u64]),
            (48, 2, vec![0, u16::MAX as u64]),
            (52, 4, vec![3, u32::MAX as u64]),
            (56, 4, vec![4, u32::MAX as u64]),
            (60, 4, vec![4, u32::MAX as u64]),
            (72, 4, vec![0, 339]),
            (96, 4, vec![0, 203]),
            (132, 4, vec![0]),
            (124, 8, vec![u64::MAX]),
            (136, 8, vec![0]),
        ] {
            for v in values {
                let bytes = edit(CodecTarget::ReclaimRoot, offset, v, width);
                assert!(
                    !accepts(CodecTarget::ReclaimRoot, &bytes),
                    "offset{offset} value{v}"
                );
                exercise(CodecTarget::ReclaimRoot, &bytes).unwrap();
            }
        }
        for (t, offset, values) in [
            (
                CodecTarget::ReclaimSegment,
                0,
                vec![0, 203, u32::MAX as u64],
            ),
            (CodecTarget::ReclaimTable, 0, vec![0, 339, u32::MAX as u64]),
            (CodecTarget::ReclaimSegment, 16, vec![0]),
            (CodecTarget::ReclaimTable, 16, vec![0, 203]),
        ] {
            for value in values {
                let bytes = edit(t, offset, value, 4);
                assert!(!accepts(t, &bytes));
                exercise(t, &bytes).unwrap();
            }
        }
    }
    #[test]
    fn sealed_capacity_boundaries_and_reserved_rejection_are_explicit() {
        let entry = ReclaimEntry {
            start: 123,
            blocks: 7,
            retire_generation: 11,
        };
        let reference = SegmentRef {
            lba: 333,
            entry_count: 202,
        };
        for (t, value) in [
            (
                CodecTarget::ReclaimSegment,
                Value::Segment(ReclaimSegment {
                    entries: vec![entry; 202],
                }),
            ),
            (
                CodecTarget::ReclaimTable,
                Value::Table(ReclaimTable {
                    refs: vec![reference; 338],
                }),
            ),
        ] {
            let bytes = value.encode(DEFAULT_BLOCK_SIZE, 17).unwrap();
            assert!(accepts(t, &bytes));
            exercise(t, &bytes).unwrap();
        }
        // Reserved bytes are zero in the documented wire representation.
        for t in [
            CodecTarget::ReclaimRoot,
            CodecTarget::ReclaimSegment,
            CodecTarget::ReclaimTable,
        ] {
            let bytes = edit(t, 4, 1, 4);
            assert!(!accepts(t, &bytes));
            exercise(t, &bytes).unwrap();
        }
        let bytes = edit(CodecTarget::ReclaimRoot, 50, 1, 2);
        assert!(!accepts(CodecTarget::ReclaimRoot, &bytes));
        exercise(CodecTarget::ReclaimRoot, &bytes).unwrap();
    }

    #[test]
    fn root_cursor_relations_cover_inline_segment_and_table_heads() {
        let bytes = seed(CodecTarget::ReclaimRoot).unwrap();
        let (Value::Root(mut root), generation) =
            decoded(CodecTarget::ReclaimRoot, &bytes).unwrap()
        else {
            panic!()
        };
        root.table_refs.clear();
        root.head_segment_offset = 0;
        let segment_head = root.encode(DEFAULT_BLOCK_SIZE, generation).unwrap();
        exercise(CodecTarget::ReclaimRoot, &segment_head).unwrap();
        for (offset, value) in [(32, 1u32), (36, 2)] {
            let mut bad = segment_head.clone();
            let h = BlockHeader::verify(&bad, CodecTarget::ReclaimRoot.block_type()).unwrap();
            bad[HEADER_SIZE + offset..HEADER_SIZE + offset + 4]
                .copy_from_slice(&value.to_le_bytes());
            h.seal(&mut bad);
            assert!(!accepts(CodecTarget::ReclaimRoot, &bad));
            exercise(CodecTarget::ReclaimRoot, &bad).unwrap();
        }
        root.segment_refs.clear();
        root.head_entry_offset = 0;
        root.head_block_offset = 0;
        let inline_head = root.encode(DEFAULT_BLOCK_SIZE, generation).unwrap();
        exercise(CodecTarget::ReclaimRoot, &inline_head).unwrap();
        for offset in [32, 36, 40] {
            let mut bad = inline_head.clone();
            let h = BlockHeader::verify(&bad, CodecTarget::ReclaimRoot.block_type()).unwrap();
            bad[HEADER_SIZE + offset] = 1;
            h.seal(&mut bad);
            assert!(!accepts(CodecTarget::ReclaimRoot, &bad));
            exercise(CodecTarget::ReclaimRoot, &bad).unwrap();
        }
    }
}
