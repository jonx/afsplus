//! Residual shared-ownership families through the family-matrix driver; see
//! tiny_cache_matrix.md. These families close a replace rename whose target is
//! shared, under a retained snapshot and under forced eviction, and the
//! bounded-budget refusal of a write that splits a shared run.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::shared_extents::{self, SharedRun};
use afsplus_core::volume::FileEditLimits;
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
/// Blocks of the shared run.
const RUN_BLOCKS: usize = 2;
/// Subsets drawn per flush segment longer than the exhaustive budget.
const SAMPLE: usize = 32;

fn shared_format(variant: Variant) -> Format {
    if variant == Variant::Eviction {
        Format::new(WIDE, WIDE as u32)
    } else {
        Format::new(SMALL, SMALL as u32)
    }
}

/// Block counts and reference counts of every record of the selected
/// checkpoint, in canonical order.
fn run_shape<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<(u64, u32)> {
    let root = volume.checkpoint().shared_extent_root_block;
    if root == 0 {
        return Vec::new();
    }
    let geometry = volume.ident().geometry();
    let generation = volume.generation();
    let records: Vec<SharedRun> =
        shared_extents::load_all(volume.device_mut(), &geometry, root, generation)
            .unwrap()
            .records;
    records
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

fn shared_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(RUN_BLOCKS * BS);
    for block in 0..RUN_BLOCKS {
        bytes.extend(std::iter::repeat_n(0x21 + block as u8, BS));
    }
    bytes
}

fn population(volume: &mut Volume<MemoryBackend>, variant: Variant) -> usize {
    if variant == Variant::Eviction {
        matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
        POPULATION
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Replace rename whose target is shared
// ---------------------------------------------------------------------------

const INCOMING: &[u8] = b"incoming bytes that replace a shared victim";

struct ReplaceSharedState {
    /// The replaced object, which shares its run with the peer.
    victim: u64,
    peer: u64,
    incoming: u64,
    population: usize,
}

struct ReplaceShared;

impl Family for ReplaceShared {
    type State = ReplaceSharedState;

    fn name(&self) -> &'static str {
        "replace rename of a shared target"
    }

    fn format(&self, variant: Variant) -> Format {
        shared_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> ReplaceSharedState {
        let population = population(volume, variant);
        let victim = volume
            .create_file_in_root("victim", &shared_bytes(), ts(2))
            .unwrap();
        let peer = volume
            .clone_file(victim, OBJECT_ROOT, "peer", ts(3))
            .unwrap();
        let incoming = volume
            .create_file_in_root("incoming", INCOMING, ts(4))
            .unwrap();
        ReplaceSharedState {
            victim,
            peer,
            incoming,
            population,
        }
    }

    fn captured(&self, state: &ReplaceSharedState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.victim, shared_bytes()),
            (state.peer, shared_bytes()),
            (state.incoming, INCOMING.to_vec()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ReplaceSharedState,
    ) -> Result<(), CoreError> {
        volume.rename_replace(OBJECT_ROOT, "incoming", OBJECT_ROOT, "victim", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ReplaceSharedState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 2 + usize::from(!published),
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("incoming").unwrap(),
            (!published).then_some(state.incoming),
            "{context}: source entry"
        );
        // The target name always resolves: to the shared victim before the
        // publication and to the incoming object after it.
        let at_victim = volume
            .lookup_root("victim")
            .unwrap()
            .unwrap_or_else(|| panic!("{context}: the target name must always resolve"));
        if published {
            assert_eq!(at_victim, state.incoming, "{context}: replacement identity");
            file_bytes(volume, state.incoming, INCOMING, context);
            assert!(
                volume.stat(state.victim).unwrap().is_none(),
                "{context}: the replaced object survived"
            );
        } else {
            assert_eq!(at_victim, state.victim, "{context}: victim identity");
            file_bytes(volume, state.victim, &shared_bytes(), context);
            file_bytes(volume, state.incoming, INCOMING, context);
        }
        assert_eq!(
            volume.lookup_root("peer").unwrap(),
            Some(state.peer),
            "{context}: peer entry"
        );
        file_bytes(volume, state.peer, &shared_bytes(), context);
        // Retiring the shared victim leaves the peer as the only owner, so the
        // reference record disappears with the publication.
        let expected = if published {
            Vec::new()
        } else {
            vec![(RUN_BLOCKS as u64, 2)]
        };
        assert_eq!(run_shape(volume), expected, "{context}: shared runs");
    }

    /// Measured demand of the replacement over a wide root directory.
    fn eviction_demand(&self) -> u64 {
        REPLACE_SHARED_DEMAND
    }
}

/// Measured resident staged-node demand of the shared-target replacement.
const REPLACE_SHARED_DEMAND: u64 = 3;

// ---------------------------------------------------------------------------
// Bounded-budget refusal of a write that splits a shared run
// ---------------------------------------------------------------------------

const SHARED_WRITE_OFFSET: u64 = 40;
const SHARED_WRITE_LENGTH: usize = 200;

fn origin_after() -> Vec<u8> {
    let mut bytes = shared_bytes();
    let start = SHARED_WRITE_OFFSET as usize;
    bytes[start..start + SHARED_WRITE_LENGTH].fill(0x9d);
    bytes
}

/// The budget the refusal fixture starts with: one mapping record, which the
/// split of a shared run cannot fit.
const REFUSED_LIMITS: FileEditLimits = FileEditLimits {
    max_blocks: 1,
    max_records: 1,
};
/// The budget the corrective step admits.
const ADMITTED_LIMITS: FileEditLimits = FileEditLimits {
    max_blocks: 1,
    max_records: 8,
};

struct BudgetState {
    origin: u64,
    peer: u64,
    limits: FileEditLimits,
}

struct SharedWriteBudget;

impl Family for SharedWriteBudget {
    type State = BudgetState;

    fn name(&self) -> &'static str {
        "bounded write into a shared run"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(SMALL, SMALL as u32)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> BudgetState {
        let origin = volume
            .create_file_in_root("origin", &shared_bytes(), ts(2))
            .unwrap();
        let peer = volume
            .clone_file(origin, OBJECT_ROOT, "peer", ts(3))
            .unwrap();
        let limits = if variant == Variant::Refusal {
            REFUSED_LIMITS
        } else {
            ADMITTED_LIMITS
        };
        BudgetState {
            origin,
            peer,
            limits,
        }
    }

    fn captured(&self, state: &BudgetState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.origin, shared_bytes()), (state.peer, shared_bytes())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &BudgetState,
    ) -> Result<(), CoreError> {
        volume.write_file_at_bounded(
            state.origin,
            SHARED_WRITE_OFFSET,
            &[0x9d; SHARED_WRITE_LENGTH],
            ts(30),
            state.limits,
        )
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &BudgetState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            2,
            "{context}: root entries"
        );
        let expected = if published {
            origin_after()
        } else {
            shared_bytes()
        };
        file_bytes(volume, state.origin, &expected, context);
        file_bytes(volume, state.peer, &shared_bytes(), context);
        let expected = if published {
            vec![((RUN_BLOCKS - 1) as u64, 2)]
        } else {
            vec![(RUN_BLOCKS as u64, 2)]
        };
        assert_eq!(run_shape(volume), expected, "{context}: shared runs");
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::PrototypeLimit(_))
    }

    /// The specified retry of a bounded refusal is the same edit with a budget
    /// that admits it; no checkpoint is published in between.
    fn relieve<D: BlockDevice>(&self, _volume: &mut Volume<D>, state: &mut BudgetState) -> u64 {
        state.limits = ADMITTED_LIMITS;
        0
    }
}

// ---------------------------------------------------------------------------
// Generated tests
// ---------------------------------------------------------------------------

const REPLACE_SHARED_SEED: u64 = 0x5eed_5a7e_2001;

crate::profile_tests!(replace_shared, |pages| matrix::plain(
    &ReplaceShared,
    pages,
    12
));
crate::profile_tests!(replace_shared_retained, |pages| matrix::retained(
    &ReplaceShared,
    pages,
    12
));
crate::profile_tests!(replace_shared_eviction, |pages| matrix::eviction_sampled(
    &ReplaceShared,
    pages,
    SAMPLE,
    REPLACE_SHARED_SEED
));
crate::profile_tests!(shared_write_budget_refusal, |pages| matrix::refusal(
    &SharedWriteBudget,
    pages
));
