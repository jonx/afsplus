//! Orphan insertion and cleanup through the family-matrix driver; see
//! tiny_cache_matrix.md. Both operations publish two checkpoints: insertion
//! creates the preparatory orphan directory before moving the name, and
//! cleanup publishes the empty tail-trimmed file before removing the object.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{object_map, CoreError, Volume};
use afsplus_format::{OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

/// Seeded full-write subsets drawn per oversized flush segment.
const EVICTION_SAMPLE: usize = 256;

const POPULATION: usize = 400;
const NAME_LENGTH: usize = 240;
const PAYLOAD: &[u8] = b"orphan payload survives every cut";

fn orphan_format(variant: Variant) -> Format {
    let blocks = if variant == Variant::Eviction {
        4096
    } else {
        512
    };
    Format {
        log_slots: 8,
        ..Format::new(blocks, blocks as u32)
    }
}

fn populated(volume: &mut Volume<MemoryBackend>, variant: Variant) -> usize {
    if variant != Variant::Eviction {
        return 0;
    }
    matrix::populate(volume, "tree", POPULATION, NAME_LENGTH);
    POPULATION
}

/// Whether the selected checkpoint's object map holds internal object 2.
fn orphan_directory_present<D: BlockDevice>(volume: &mut Volume<D>) -> bool {
    let checkpoint = volume.checkpoint().clone();
    let geometry = volume.ident().geometry();
    object_map::lookup_lba(
        volume.device_mut(),
        &geometry,
        checkpoint.object_map_block,
        checkpoint.generation,
        OBJECT_ORPHAN_DIRECTORY,
    )
    .unwrap()
    .is_some()
}

struct Insertion;

struct InsertionState {
    object: u64,
    populated: usize,
    pressure: bool,
}

impl Family for Insertion {
    type State = InsertionState;

    fn name(&self) -> &'static str {
        "orphan insertion"
    }

    fn format(&self, variant: Variant) -> Format {
        orphan_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> InsertionState {
        let populated = populated(volume, variant);
        let object = volume.create_file_in_root("open", PAYLOAD, ts(2)).unwrap();
        let pressure = variant == Variant::Exhausted;
        if pressure {
            volume.set_reclaim_batch_blocks(1);
            let id = volume.create_file_in_root("pressure", b"", ts(3)).unwrap();
            // Reserve the largest range ordinary allocation admits.
            let mut reserve = volume.available_blocks();
            while volume
                .preallocate_file(id, 0, reserve * BS as u64, ts(4))
                .is_err()
            {
                reserve -= 1;
            }
            assert!(
                matches!(
                    volume.create_file_in_root("ordinary-probe", &[1; BS], ts(5)),
                    Err(CoreError::NoSpace)
                ),
                "ordinary allocation must be exhausted"
            );
        }
        InsertionState {
            object,
            populated,
            pressure,
        }
    }

    fn captured(&self, state: &InsertionState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.object, PAYLOAD.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &InsertionState,
    ) -> Result<(), CoreError> {
        let id = volume.orphan_file(OBJECT_ROOT, "open", ts(30))?;
        assert_eq!(id, state.object);
        Ok(())
    }

    fn publications(&self, _variant: Variant) -> u64 {
        2
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &InsertionState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let visible = delta < 2;
        let pressure = state.pressure;
        assert_eq!(
            volume.read_file(state.object).unwrap(),
            PAYLOAD,
            "{context}"
        );
        assert_eq!(
            volume.lookup_root("open").unwrap(),
            visible.then_some(state.object),
            "{context}: application name"
        );
        assert_eq!(
            orphan_directory_present(volume),
            delta >= 1,
            "{context}: preparatory orphan directory"
        );
        assert_eq!(
            volume.orphan_count().unwrap(),
            u64::from(!visible),
            "{context}"
        );
        assert_eq!(
            volume.orphan_object(state.object).unwrap(),
            !visible,
            "{context}"
        );
        assert_eq!(
            volume.lookup_root("pressure").unwrap().is_some(),
            pressure,
            "{context}: pressure file"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated + usize::from(visible) + usize::from(pressure),
            "{context}: root entries"
        );
    }

    /// Root-directory and orphan-directory paths stage at most three nodes
    /// with 400 long names.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

struct Cleanup;

struct CleanupState {
    object: u64,
    populated: usize,
}

fn fragmented_bytes() -> Vec<u8> {
    let mut bytes = vec![0; 3 * BS];
    bytes[..BS].fill(0x41);
    bytes[2 * BS..].fill(0x42);
    bytes
}

impl Family for Cleanup {
    type State = CleanupState;

    fn name(&self) -> &'static str {
        "orphan cleanup"
    }

    fn format(&self, variant: Variant) -> Format {
        orphan_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> CleanupState {
        let populated = populated(volume, variant);
        let object = volume.create_file_in_root("frag", b"", ts(2)).unwrap();
        volume.write_file_at(object, 0, &[0x41; BS], ts(3)).unwrap();
        volume
            .write_file_at(object, 2 * BS as u64, &[0x42; BS], ts(4))
            .unwrap();
        volume.orphan_file(OBJECT_ROOT, "frag", ts(5)).unwrap();
        CleanupState { object, populated }
    }

    fn captured(&self, state: &CleanupState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.object, fragmented_bytes())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CleanupState,
    ) -> Result<(), CoreError> {
        volume.set_orphan_cleanup_extent_budget(2);
        volume.cleanup_orphan(state.object, ts(50)).map(|_| ())
    }

    fn publications(&self, _variant: Variant) -> u64 {
        2
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CleanupState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(volume.lookup_root("frag").unwrap(), None, "{context}");
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated,
            "{context}"
        );
        assert!(orphan_directory_present(volume), "{context}");
        let pending = delta < 2;
        assert_eq!(
            volume.orphan_object(state.object).unwrap(),
            pending,
            "{context}"
        );
        assert_eq!(
            volume.orphan_count().unwrap(),
            u64::from(pending),
            "{context}"
        );
        let metadata = volume.visible_metadata(state.object).unwrap();
        match delta {
            0 => {
                assert_eq!(
                    volume.read_file(state.object).unwrap(),
                    fragmented_bytes(),
                    "{context}"
                );
                assert_eq!(
                    metadata.unwrap().allocated_bytes,
                    2 * BS as u64,
                    "{context}"
                );
            }
            1 => {
                assert!(
                    volume.read_file(state.object).unwrap().is_empty(),
                    "{context}: tail-trimmed file"
                );
                assert_eq!(metadata.unwrap().allocated_bytes, 0, "{context}");
            }
            _ => assert!(metadata.is_none(), "{context}: removed object"),
        }
    }

    /// Extent-map, object-map and orphan-directory paths stage at most three
    /// nodes with 400 long names.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

crate::profile_tests!(insertion_cuts_and_faults, |pages| matrix::plain(
    &Insertion, pages, 12
));
crate::profile_tests!(insertion_retained, |pages| matrix::retained(
    &Insertion, pages, 12
));
crate::profile_tests!(insertion_eviction, |pages| matrix::eviction_sampled(
    &Insertion,
    pages,
    EVICTION_SAMPLE,
    0x5eed_0003
));
crate::profile_tests!(insertion_ambiguous, |pages| matrix::ambiguous(
    &Insertion,
    pages,
    Variant::Plain
));
crate::profile_tests!(insertion_with_ordinary_allocation_exhausted, |pages| {
    let recording = matrix::record(&Insertion, pages, Variant::Exhausted);
    matrix::cuts(&Insertion, &recording, pages, Variant::Exhausted, 12);
    matrix::faults(&Insertion, &recording, pages, Variant::Exhausted);
});
crate::profile_tests!(cleanup_cuts_and_faults, |pages| matrix::plain(
    &Cleanup, pages, 12
));
crate::profile_tests!(cleanup_retained, |pages| matrix::retained(
    &Cleanup, pages, 12
));
crate::profile_tests!(cleanup_eviction, |pages| matrix::eviction(
    &Cleanup,
    pages,
    Some(10)
));
crate::profile_tests!(cleanup_ambiguous, |pages| matrix::ambiguous(
    &Cleanup,
    pages,
    Variant::Plain
));
