//! Reclaim steps and a spilled metadata batch through the family-matrix
//! driver; see tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::BatchOp;
use afsplus_core::{CoreError, Volume};
use afsplus_format::reclaim::ReclaimCaps;
use afsplus_format::OBJECT_ROOT;
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

/// Seeded full-write subsets drawn per oversized flush segment.
const EVICTION_SAMPLE: usize = 256;

fn accounting<D: BlockDevice>(volume: &Volume<D>) -> (u64, u64) {
    (volume.free_blocks(), volume.reclaim_pending_blocks())
}

struct ReclaimStep;

struct StepState {
    big: u64,
    snapshot: Option<u64>,
}

const SPANS: u64 = 8;

impl ReclaimStep {
    /// Free and pending blocks before and after the recorded step.
    fn expected(&self, variant: Variant) -> [(u64, u64); 2] {
        match variant {
            Variant::Plain => [(223, 7), (223, 14)],
            Variant::Retained => [(221, 9), (222, 8)],
            _ => [(969, 600), (1478, 172)],
        }
    }
}

impl Family for ReclaimStep {
    type State = StepState;

    fn name(&self) -> &'static str {
        "reclaim step"
    }

    fn format(&self, variant: Variant) -> Format {
        if variant == Variant::Eviction {
            // 1,024 regions span eight allocation-root leaves.
            Format::new(16 * 1024, 16)
        } else {
            Format {
                reclaim_caps: ReclaimCaps {
                    inline_entries: 4,
                    segment_refs: 3,
                    table_refs: 8,
                },
                ..Format::new(256, 256)
            }
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> StepState {
        volume.set_reclaim_batch_blocks(1);
        if variant == Variant::Eviction {
            // Preallocated spans cover every region; deleting them queues
            // pending blocks behind all allocation-root leaves.
            let span = volume.available_blocks() / (SPANS + 1);
            for index in 0..SPANS {
                let name = format!("span-{index}");
                let id = volume.create_file_in_root(&name, b"", ts(1)).unwrap();
                volume
                    .preallocate_file(id, 0, span * BS as u64, ts(2))
                    .unwrap();
            }
            for index in 0..SPANS {
                volume
                    .delete_file_in_root(&format!("span-{index}"), ts(3))
                    .unwrap();
            }
            return StepState {
                big: 0,
                snapshot: None,
            };
        }
        let big = volume
            .create_file_in_root("big", &[0xb7; 3 * BS], ts(1))
            .unwrap();
        let snapshot =
            (variant == Variant::Retained).then(|| volume.snapshot_create(ts(2)).unwrap());
        volume.delete_file_in_root("big", ts(3)).unwrap();
        StepState { big, snapshot }
    }

    fn snapshot(&self, state: &StepState) -> Option<u64> {
        state.snapshot
    }

    fn captured(&self, state: &StepState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.big, vec![0xb7; 3 * BS])]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &StepState,
    ) -> Result<(), CoreError> {
        let batch = if state.big == 0 { 1 << 16 } else { 2 };
        volume.set_reclaim_batch_blocks(batch);
        volume.reclaim_step(ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &StepState,
        variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert!(volume.list_root().unwrap().is_empty(), "{context}");
        assert_eq!(
            accounting(volume),
            self.expected(variant)[delta as usize],
            "{context}: free/pending"
        );
    }

    /// Promotion touches eight allocation-root leaves and their root.
    fn eviction_demand(&self) -> u64 {
        9
    }
}

/// One long-name create in a volume whose root directory, object map and
/// allocation root are pre-populated, recorded under a tail budget.
struct SpilledBatch;

struct BatchState {
    populated: usize,
}

const POPULATION: usize = 300;
const NAME_LENGTH: usize = 240;

impl Family for SpilledBatch {
    type State = BatchState;

    fn name(&self) -> &'static str {
        "spilled metadata batch"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(16 * 1024, 16)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> BatchState {
        volume.set_reclaim_batch_blocks(1);
        matrix::populate(volume, "tree", POPULATION, NAME_LENGTH);
        BatchState {
            populated: POPULATION,
        }
    }

    fn captured(&self, _state: &BatchState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &BatchState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(1);
        let name = matrix::padded_name("batch", 0, NAME_LENGTH);
        let operations = [BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: &name,
            content: b"",
        }];
        volume.run_batch(&operations, ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &BatchState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let name = matrix::padded_name("batch", 0, NAME_LENGTH);
        let id = volume.lookup_root(&name).unwrap();
        assert_eq!(id.is_some(), delta == 1, "{context}: batch entry");
        if let Some(id) = id {
            assert!(volume.read_file(id).unwrap().is_empty(), "{context}");
        }
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated + delta as usize,
            "{context}"
        );
    }

    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &BatchState,
        _variant: Variant,
    ) {
        let stats = volume.last_commit_stats().unwrap();
        assert_eq!(
            stats.allocation_tree_nodes_written, 2,
            "allocation-root nodes"
        );
        eprintln!(
            "spilled batch pages={} allocation_nodes={} spills={}",
            volume.tree_cache_pages(),
            stats.allocation_tree_nodes_written,
            stats.tree_mutations.staged_spill_writes
        );
    }

    /// Directory, object-map and allocation-root paths stage three nodes.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

crate::profile_tests!(reclaim_step_cuts_and_faults, |pages| matrix::plain(
    &ReclaimStep,
    pages,
    12
));
crate::profile_tests!(reclaim_step_retained, |pages| matrix::retained(
    &ReclaimStep,
    pages,
    12
));
crate::profile_tests!(reclaim_step_eviction, |pages| matrix::eviction_sampled(
    &ReclaimStep,
    pages,
    EVICTION_SAMPLE,
    0x5eed_0001
));
crate::profile_tests!(reclaim_step_ambiguous, |pages| matrix::ambiguous(
    &ReclaimStep,
    pages,
    Variant::Plain
));
crate::profile_tests!(spilled_batch_eviction, |pages| matrix::eviction_sampled(
    &SpilledBatch,
    pages,
    EVICTION_SAMPLE,
    0x5eed_0002
));
