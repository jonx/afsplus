//! Snapshot-bearing checkpoint wire and fixed-geometry structural admission.
use super::{checkpoint_seed, UUID};
use afsplus_format::checkpoint::{Checkpoint, SnapshotRoots};
use afsplus_format::geometry::Geometry;
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::DEFAULT_BLOCK_SIZE;

fn geometry() -> Geometry {
    Geometry {
        block_size: DEFAULT_BLOCK_SIZE,
        total_blocks: 8192,
        region_size: 4096,
    }
}
pub(super) fn seed() -> Result<Vec<u8>, String> {
    let mut value = Checkpoint::decode(&checkpoint_seed()?, &UUID).map_err(|e| e.to_string())?;
    value.snapshot_roots = Some(SnapshotRoots {
        registry: 45,
        lifetimes: 46,
    });
    value.encode(DEFAULT_BLOCK_SIZE).map_err(|e| e.to_string())
}
fn word(p: &[u8], n: usize) -> u64 {
    u64::from_le_bytes(p[n..n + 8].try_into().unwrap())
}
// Two 4096-block regions, one bitmap page per region, 3 descriptor +3 bitmap
// slots, plus3 bootstrap blocks in region0. Literal bounds are an independent
// oracle for this fixture, not a call back into Geometry::is_allocatable.
fn allocatable(lba: u64) -> bool {
    (9..4096).contains(&lba) || (4102..8192).contains(&lba)
}
fn expected(input: &[u8]) -> Option<Checkpoint> {
    let h = BlockHeader::verify(input, block_type::CHECKPOINT).ok()?;
    if h.flags != 0 || h.owner != 0 || !matches!(h.payload_len, 168 | 184) {
        return None;
    }
    // Nothing follows the payload (ADR-111).
    if input[HEADER_SIZE + h.payload_len as usize..]
        .iter()
        .any(|b| *b != 0)
    {
        return None;
    }
    let p = &input[HEADER_SIZE..HEADER_SIZE + h.payload_len as usize];
    if p[..16] != UUID
        || word(p, 16) == 0
        || word(p, 16) != word(input, 16)
        || word(p, 24) != 1
        || word(p, 80) != 0
    {
        return None;
    }
    // Label field (ADR-104): length, seven zero bytes, NUL-free UTF-8, zero
    // padding to 64 bytes.
    let length = p[96] as usize;
    if length > 64
        || p[97..104].iter().any(|b| *b != 0)
        || p[104 + length..168].iter().any(|b| *b != 0)
        || p[104..104 + length].contains(&0)
    {
        return None;
    }
    let label = std::str::from_utf8(&p[104..104 + length]).ok()?.to_owned();
    let roots = if p.len() == 184 {
        let (registry, lifetimes) = (word(p, 168), word(p, 176));
        if registry == lifetimes || !allocatable(registry) || !allocatable(lifetimes) {
            return None;
        }
        Some(SnapshotRoots {
            registry,
            lifetimes,
        })
    } else {
        None
    };
    if [32, 40, 48]
        .iter()
        .any(|offset| !allocatable(word(p, *offset)))
        || (word(p, 88) != 0 && !allocatable(word(p, 88)))
        || word(p, 56) < 16
        || word(p, 72) > 8192
    {
        return None;
    }
    Some(Checkpoint {
        uuid: UUID,
        generation: word(p, 16),
        root_object_id: word(p, 24),
        object_map_block: word(p, 32),
        allocation_root_block: word(p, 40),
        reclaim_root_block: word(p, 48),
        next_object_id: word(p, 56),
        committed_tx_id: word(p, 64),
        free_blocks_total: word(p, 72),
        flags: word(p, 80),
        shared_extent_root_block: word(p, 88),
        label,
        snapshot_roots: roots,
    })
}
fn decoded(input: &[u8]) -> Option<Checkpoint> {
    Checkpoint::decode(input, &UUID)
        .ok()
        .filter(|v| v.validate_structural(&geometry()).is_ok())
}
pub(super) fn accepts(input: &[u8]) -> bool {
    decoded(input).is_some()
}
pub(super) fn exercise(input: &[u8]) -> Result<(), String> {
    let actual = decoded(input);
    if actual != expected(input) {
        return Err("snapshot checkpoint independent fields/admission mismatch".into());
    }
    if let Some(value) = actual {
        let bytes = value.encode(input.len()).map_err(|e| e.to_string())?;
        if decoded(&bytes) != Some(value.clone()) || expected(&bytes) != Some(value) {
            return Err("snapshot checkpoint canonical mismatch".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn resealed(offset: usize, value: u64) -> Vec<u8> {
        let mut bytes = seed().unwrap();
        let h = BlockHeader::verify(&bytes, block_type::CHECKPOINT).unwrap();
        bytes[HEADER_SIZE + offset..HEADER_SIZE + offset + 8].copy_from_slice(&value.to_le_bytes());
        h.seal(&mut bytes);
        bytes
    }
    #[test]
    fn snapshot_checkpoint_roots_and_lengths_are_explicit() {
        let bytes = seed().unwrap();
        let header = BlockHeader::verify(&bytes, block_type::CHECKPOINT).unwrap();
        assert_eq!(header.payload_len, 184);
        assert!(decoded(&bytes).unwrap().snapshot_roots.is_some());
        for n in 0..=DEFAULT_BLOCK_SIZE {
            exercise(&bytes[..n]).unwrap();
        }
        for len in 0..=192 {
            let mut changed = bytes.clone();
            BlockHeader {
                payload_len: len,
                ..header
            }
            .seal(&mut changed);
            // The seed carries its roots: under the short length they are a
            // nonzero tail, so only its own length is admitted.
            assert_eq!(accepts(&changed), len == 184);
            exercise(&changed).unwrap();
        }
        for offset in [168, 176] {
            for lba in [0, 1, 8, 9, 4095, 4096, 4101, 4102, 8191, 8192, u64::MAX] {
                let changed = resealed(offset, lba);
                assert_eq!(accepts(&changed), allocatable(lba));
                exercise(&changed).unwrap();
            }
        }
        for (offset, value) in [
            (168, 46),
            (176, 45),
            // Label field: length above the bound, a reserved byte, padding.
            (96, 65),
            (96, 0x0100),
            (160, 1),
            (16, 0),
            (16, 8),
            (24, 0),
            (80, 1),
            (72, 8193),
            (56, 15),
        ] {
            let changed = resealed(offset, value);
            assert!(!accepts(&changed));
            exercise(&changed).unwrap();
        }
        for (flags, owner, generation) in [(1, 0, 7), (0, 1, 7), (0, 0, 0), (0, 0, 8)] {
            let mut changed = bytes.clone();
            BlockHeader {
                flags,
                owner,
                generation,
                ..header
            }
            .seal(&mut changed);
            assert!(!accepts(&changed));
            exercise(&changed).unwrap();
        }
        let mut changed = bytes.clone();
        changed[HEADER_SIZE] ^= 1;
        header.seal(&mut changed);
        assert!(!accepts(&changed));
        exercise(&changed).unwrap();
    }
    #[test]
    fn legacy_and_snapshot_shapes_do_not_infer_the_volume_feature_bit() {
        // Feature negotiation lives in mount::select_checkpoint, outside this
        // format-only target. Decode exposes shape; it cannot see Identification.
        let legacy = checkpoint_seed().unwrap();
        assert!(Checkpoint::decode(&legacy, &UUID)
            .unwrap()
            .snapshot_roots
            .is_none());
        assert!(Checkpoint::decode(&seed().unwrap(), &UUID)
            .unwrap()
            .snapshot_roots
            .is_some());
        exercise(&legacy).unwrap();
    }
}
