//! CloneFile and CloneRange through the family-matrix driver; see
//! tiny_cache_matrix.md. The families cover first sharing, an unaligned
//! CloneRange whose boundaries are copied and whose interior is shared, and a
//! write into a clone that a snapshot retains after its publication.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::shared_extents::{self, SharedRun};
use afsplus_core::{CoreError, Volume};
use afsplus_format::OBJECT_ROOT;
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

/// Volume geometry of the bounded fixtures.
const SMALL: u64 = 512;
/// Volume geometry of the pre-populated eviction fixtures.
const WIDE: u64 = 1024;
/// Entries the eviction fixtures hold in the root directory.
const POPULATION: usize = 150;
/// Long root names used to force staged-node eviction.
const NAME_LENGTH: usize = 240;

fn clone_format(variant: Variant) -> Format {
    if variant == Variant::Eviction {
        Format::new(WIDE, WIDE as u32)
    } else {
        Format::new(SMALL, SMALL as u32)
    }
}

/// Canonical reference records of the selected checkpoint.
fn shared_runs<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<SharedRun> {
    let root = volume.checkpoint().shared_extent_root_block;
    if root == 0 {
        return Vec::new();
    }
    let geometry = volume.ident().geometry();
    let generation = volume.generation();
    shared_extents::load_all(volume.device_mut(), &geometry, root, generation)
        .unwrap()
        .records
}

/// Block counts and reference counts of every record, in canonical order.
fn run_shape<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<(u64, u32)> {
    shared_runs(volume)
        .into_iter()
        .map(|run| (run.block_count, run.reference_count))
        .collect()
}

/// Reads the whole file with a sentinel past the end, so a longer file fails.
fn file_bytes<D: BlockDevice>(volume: &mut Volume<D>, id: u64, expected: &[u8], context: &str) {
    let mut read = vec![0xa5; expected.len() + 1];
    assert_eq!(
        volume.read_file_at(id, 0, &mut read).unwrap(),
        expected.len(),
        "{context}: length of {id}"
    );
    assert_eq!(
        &read[..expected.len()],
        expected,
        "{context}: bytes of {id}"
    );
    assert_eq!(read[expected.len()], 0xa5, "{context}: EOF of {id}");
}

/// Populates the root of an eviction fixture and reports the entry count.
fn population(volume: &mut Volume<MemoryBackend>, variant: Variant) -> usize {
    if variant == Variant::Eviction {
        matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
        POPULATION
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// CloneFile
// ---------------------------------------------------------------------------

/// Three blocks with a distinct byte each, so a partial share is visible.
const SOURCE_BLOCKS: usize = 3;

fn source_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(SOURCE_BLOCKS * BS);
    for block in 0..SOURCE_BLOCKS {
        bytes.extend(std::iter::repeat_n(0x11 + block as u8, BS));
    }
    bytes
}

/// Bytes of the file the refusal fixture keeps at the clone's name.
const OCCUPANT: &[u8] = b"occupant bytes the refused clone must not overwrite";

struct CloneState {
    source: u64,
    population: usize,
    /// Whether the corrective step of the refusal fixture has run.
    relieved: bool,
}

struct CloneFile;

impl Family for CloneFile {
    type State = CloneState;

    fn name(&self) -> &'static str {
        "clone file"
    }

    fn format(&self, variant: Variant) -> Format {
        clone_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> CloneState {
        let population = population(volume, variant);
        let source = volume
            .create_file_in_root("origin", &source_bytes(), ts(2))
            .unwrap();
        if variant == Variant::Refusal {
            volume.create_file_in_root("copy", OCCUPANT, ts(3)).unwrap();
        }
        CloneState {
            source,
            population,
            relieved: false,
        }
    }

    fn captured(&self, state: &CloneState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.source, source_bytes())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CloneState,
    ) -> Result<(), CoreError> {
        volume
            .clone_file(state.source, OBJECT_ROOT, "copy", ts(30))
            .map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CloneState,
        variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        let blocked = variant == Variant::Refusal && !state.relieved;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 1 + usize::from(published || blocked),
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("origin").unwrap(),
            Some(state.source),
            "{context}: source entry"
        );
        file_bytes(volume, state.source, &source_bytes(), context);
        let copy = volume.lookup_root("copy").unwrap();
        if published {
            let copy = copy.unwrap_or_else(|| panic!("{context}: clone is absent"));
            assert_ne!(copy, state.source, "{context}: clone identity");
            file_bytes(volume, copy, &source_bytes(), context);
            assert_eq!(
                run_shape(volume),
                vec![(SOURCE_BLOCKS as u64, 2)],
                "{context}: shared runs"
            );
        } else {
            if blocked {
                file_bytes(volume, copy.unwrap(), OCCUPANT, context);
            } else {
                assert_eq!(copy, None, "{context}: clone is present");
            }
            assert!(run_shape(volume).is_empty(), "{context}: shared runs");
        }
    }

    /// The clone writes one object record and one root entry above the
    /// populated root leaf, and creates the reference tree.
    fn eviction_demand(&self) -> u64 {
        CLONE_FILE_DEMAND
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::AlreadyExists)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut CloneState) -> u64 {
        volume.delete_file_in_root("copy", ts(40)).unwrap();
        state.relieved = true;
        1
    }
}

// ---------------------------------------------------------------------------
// Unaligned CloneRange
// ---------------------------------------------------------------------------

/// Destination file length in blocks.
const TARGET_BLOCKS: usize = 4;
/// Byte offset of the cloned range in both files: an unaligned start whose
/// leading partial block must be copied.
const RANGE_OFFSET: u64 = 100;
/// Range length, chosen so the end is unaligned too and exactly one interior
/// block is shared.
const RANGE_LENGTH: u64 = 2 * BS as u64 + 50;

fn target_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(TARGET_BLOCKS * BS);
    for block in 0..TARGET_BLOCKS {
        bytes.extend(std::iter::repeat_n(0x81 + block as u8, BS));
    }
    bytes
}

/// The destination after the range clone: the bytes outside the range keep
/// their old values, the bytes inside take the source's.
fn target_after() -> Vec<u8> {
    let mut bytes = target_bytes();
    let source = source_bytes();
    let start = RANGE_OFFSET as usize;
    let end = start + RANGE_LENGTH as usize;
    bytes[start..end].copy_from_slice(&source[start..end]);
    bytes
}

struct RangeState {
    source: u64,
    target: u64,
    population: usize,
    /// Filler the refusal fixture holds, released by the corrective step.
    ballast: Option<u64>,
}

struct CloneRange;

impl Family for CloneRange {
    type State = RangeState;

    fn name(&self) -> &'static str {
        "unaligned clone range"
    }

    fn format(&self, variant: Variant) -> Format {
        if variant == Variant::Refusal {
            // Small enough that one filler file exhausts the free space.
            Format::new(REFUSAL_BLOCKS, REFUSAL_BLOCKS as u32)
        } else {
            clone_format(variant)
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> RangeState {
        let population = population(volume, variant);
        let source = volume
            .create_file_in_root("origin", &source_bytes(), ts(2))
            .unwrap();
        let target = volume
            .create_file_in_root("target", &target_bytes(), ts(3))
            .unwrap();
        let ballast = (variant == Variant::Refusal).then(|| ballast(volume));
        RangeState {
            source,
            target,
            population,
            ballast,
        }
    }

    fn captured(&self, state: &RangeState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.source, source_bytes()),
            (state.target, target_bytes()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RangeState,
    ) -> Result<(), CoreError> {
        volume.clone_range(
            state.source,
            RANGE_OFFSET,
            state.target,
            RANGE_OFFSET,
            RANGE_LENGTH,
            ts(30),
        )
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RangeState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 2 + usize::from(state.ballast.is_some()),
            "{context}: root entries"
        );
        file_bytes(volume, state.source, &source_bytes(), context);
        let expected = if published {
            target_after()
        } else {
            target_bytes()
        };
        file_bytes(volume, state.target, &expected, context);
        // One interior block of the destination range is aligned in both
        // files, so exactly that block is shared; the two partial boundaries
        // are copied into private blocks.
        let runs = run_shape(volume);
        if published {
            assert_eq!(runs, vec![(1, 2)], "{context}: shared runs");
        } else {
            assert!(runs.is_empty(), "{context}: shared runs");
        }
    }

    fn eviction_demand(&self) -> u64 {
        CLONE_RANGE_DEMAND
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::NoSpace)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut RangeState) -> u64 {
        state.ballast = None;
        release_ballast(volume)
    }
}

/// Deletes the filler and drains the reclaim queue, so the retry has space.
fn release_ballast<D: BlockDevice>(volume: &mut Volume<D>) -> u64 {
    let before = volume.generation();
    volume.delete_file_in_root("ballast", ts(40)).unwrap();
    volume.set_reclaim_batch_blocks(1024);
    while volume.reclaim_pending_blocks() > 0 && volume.reclaim_step(ts(41)).unwrap() > 0 {}
    volume.generation() - before
}

/// Volume size of the low-space refusal fixture.
const REFUSAL_BLOCKS: u64 = 256;

/// Fills the volume until one more block cannot be allocated, so the boundary
/// copies of the range clone are refused. The corrective step deletes it.
fn ballast(volume: &mut Volume<MemoryBackend>) -> u64 {
    let filler = volume.create_file_in_root("ballast", b"", ts(9)).unwrap();
    let mut offset = 0u64;
    loop {
        match volume.write_file_at(filler, offset, &[0x5b; BS], ts(9)) {
            Ok(()) => offset += BS as u64,
            Err(CoreError::NoSpace) => break,
            Err(error) => panic!("filling the refusal fixture: {error}"),
        }
    }
    filler
}

// ---------------------------------------------------------------------------
// Writing into a retained clone after its publication
// ---------------------------------------------------------------------------

const CLONE_WRITE_OFFSET: u64 = 100;
const CLONE_WRITE_LENGTH: usize = 300;

fn clone_after() -> Vec<u8> {
    let mut bytes = source_bytes();
    let start = CLONE_WRITE_OFFSET as usize;
    bytes[start..start + CLONE_WRITE_LENGTH].fill(0xc7);
    bytes
}

struct MutationState {
    source: u64,
    clone: u64,
    population: usize,
}

struct CloneMutation;

impl Family for CloneMutation {
    type State = MutationState;

    fn name(&self) -> &'static str {
        "write into a published clone"
    }

    fn format(&self, variant: Variant) -> Format {
        clone_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> MutationState {
        let population = population(volume, variant);
        let source = volume
            .create_file_in_root("origin", &source_bytes(), ts(2))
            .unwrap();
        let clone = volume
            .clone_file(source, OBJECT_ROOT, "copy", ts(3))
            .unwrap();
        MutationState {
            source,
            clone,
            population,
        }
    }

    /// The snapshot the driver takes after setup captures the published clone,
    /// so the write must copy the shared block rather than change it.
    fn captured(&self, state: &MutationState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.source, source_bytes()),
            (state.clone, source_bytes()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MutationState,
    ) -> Result<(), CoreError> {
        volume.write_file_at(
            state.clone,
            CLONE_WRITE_OFFSET,
            &[0xc7; CLONE_WRITE_LENGTH],
            ts(30),
        )
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MutationState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 2,
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("copy").unwrap(),
            Some(state.clone),
            "{context}: clone entry"
        );
        file_bytes(volume, state.source, &source_bytes(), context);
        let expected = if published {
            clone_after()
        } else {
            source_bytes()
        };
        file_bytes(volume, state.clone, &expected, context);
        // The write moves the clone off the first shared block, leaving the
        // remaining two blocks shared twice.
        let expected = if published {
            vec![((SOURCE_BLOCKS - 1) as u64, 2)]
        } else {
            vec![(SOURCE_BLOCKS as u64, 2)]
        };
        assert_eq!(run_shape(volume), expected, "{context}: shared runs");
    }

    fn eviction_demand(&self) -> u64 {
        CLONE_MUTATION_DEMAND
    }
}

/// A plain fixture whose unflushed tail exceeds the exhaustive budget: the
/// recording, the seeded sampled cut campaign and the full fault matrix.
fn plain_sampled<F: Family>(family: &F, pages: usize, sample: usize, seed: u64) {
    let recording = matrix::record(family, pages, Variant::Plain);
    matrix::sampled_cuts(family, &recording, pages, Variant::Plain, sample, seed);
    matrix::faults(family, &recording, pages, Variant::Plain);
}

/// The same for a retained-snapshot fixture, with ambiguous publication.
fn retained_sampled<F: Family>(family: &F, pages: usize, sample: usize, seed: u64) {
    let recording = matrix::record(family, pages, Variant::Retained);
    matrix::sampled_cuts(family, &recording, pages, Variant::Retained, sample, seed);
    matrix::faults(family, &recording, pages, Variant::Retained);
    matrix::ambiguous(family, pages, Variant::Retained);
}

// ---------------------------------------------------------------------------
// Generated tests
// ---------------------------------------------------------------------------

/// Measured resident staged-node demands at the unlimited profile. The clone
/// transactions descend one object-map path over the populated root, so they
/// evict at two pages and the measured demand is the limit of these fixtures.
const CLONE_FILE_DEMAND: u64 = 3;
const CLONE_RANGE_DEMAND: u64 = 1;
const CLONE_MUTATION_DEMAND: u64 = 1;
/// Seed and sample size of the sampled cut campaign of the spilled clone.
const CLONE_FILE_SEED: u64 = 0x5eed_c10e_0001;
const SAMPLE: usize = 32;

crate::profile_tests!(clone_file, |pages| plain_sampled(
    &CloneFile,
    pages,
    SAMPLE,
    CLONE_FILE_SEED
));
crate::profile_tests!(clone_file_retained, |pages| retained_sampled(
    &CloneFile,
    pages,
    SAMPLE,
    CLONE_FILE_SEED
));
crate::profile_tests!(clone_file_refusal, |pages| matrix::refusal(
    &CloneFile, pages
));
crate::profile_tests!(clone_file_eviction, |pages| matrix::eviction_sampled(
    &CloneFile,
    pages,
    SAMPLE,
    CLONE_FILE_SEED
));
crate::profile_tests!(clone_range, |pages| matrix::plain(&CloneRange, pages, 12));
crate::profile_tests!(clone_range_retained, |pages| matrix::retained(
    &CloneRange,
    pages,
    12
));
crate::profile_tests!(clone_range_refusal, |pages| matrix::refusal(
    &CloneRange,
    pages
));
crate::profile_tests!(clone_range_eviction, |pages| matrix::eviction(
    &CloneRange,
    pages,
    None
));
crate::profile_tests!(clone_mutation_retained, |pages| matrix::retained(
    &CloneMutation,
    pages,
    12
));
crate::profile_tests!(clone_mutation_eviction, |pages| matrix::eviction(
    &CloneMutation,
    pages,
    None
));
