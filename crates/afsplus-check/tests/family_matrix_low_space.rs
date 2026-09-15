//! Near-full delete and ENOSPC during spilled metadata allocation through the
//! family-matrix driver; see tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::BatchOp;
use afsplus_core::{CoreError, Volume};
use afsplus_format::OBJECT_ROOT;
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

const POPULATION: usize = 300;
const NAME_LENGTH: usize = 240;
const BATCH: usize = 64;

fn accounting<D: BlockDevice>(volume: &Volume<D>) -> (u64, u64) {
    (volume.free_blocks(), volume.reclaim_pending_blocks())
}

/// Preallocates the largest range ordinary allocation admits and returns it.
fn reserve_available(volume: &mut Volume<MemoryBackend>, file: u64) -> u64 {
    let mut reserved = volume.available_blocks();
    while volume
        .preallocate_file(file, 0, reserved * BS as u64, ts(2))
        .is_err()
    {
        reserved -= 1;
    }
    reserved
}

struct NearFullDelete;

struct DeleteState {
    victim: u64,
    reserved: u64,
    populated: usize,
}

impl NearFullDelete {
    /// Free and pending blocks before and after the recorded delete.
    fn expected(&self, variant: Variant) -> [(u64, u64); 2] {
        match variant {
            Variant::Plain => [(16, 7), (12, 10)],
            Variant::Eviction => [(64, 36), (56, 45)],
            _ => unreachable!("near-full delete declares no {variant:?} fixture"),
        }
    }
}

impl Family for NearFullDelete {
    type State = DeleteState;

    fn name(&self) -> &'static str {
        "near-full delete"
    }

    fn format(&self, variant: Variant) -> Format {
        let blocks = if variant == Variant::Eviction {
            2048
        } else {
            512
        };
        Format::new(blocks, blocks as u32)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> DeleteState {
        volume.set_reclaim_batch_blocks(1);
        let mut populated = 0;
        if variant == Variant::Eviction {
            matrix::populate(volume, "tree", POPULATION, NAME_LENGTH);
            populated = POPULATION;
        }
        let victim = volume.create_file_in_root("victim", b"", ts(1)).unwrap();
        let reserved = reserve_available(volume, victim);
        DeleteState {
            victim,
            reserved,
            populated,
        }
    }

    fn captured(&self, _state: &DeleteState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &DeleteState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(1);
        volume.delete_file_in_root("victim", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DeleteState,
        variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let present = delta == 0;
        assert_eq!(
            volume.lookup_root("victim").unwrap(),
            present.then_some(state.victim),
            "{context}"
        );
        if present {
            let metadata = volume.visible_metadata(state.victim).unwrap().unwrap();
            assert_eq!(
                (metadata.size_bytes, metadata.allocated_bytes),
                (0, state.reserved * BS as u64),
                "{context}: reserved layout"
            );
        }
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated + usize::from(present),
            "{context}"
        );
        assert_eq!(
            accounting(volume),
            self.expected(variant)[delta as usize],
            "{context}: free/pending"
        );
    }

    /// Root-directory, object-map and reclaim paths stage three nodes with
    /// 300 long names.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

struct SpilledEnospc;

struct EnospcState {
    populated: usize,
    relieved: bool,
}

/// Ordinary allocation capacity restored before the batch retry.
const RETRY_AVAILABLE: u64 = 512;

fn batch_name(index: usize) -> String {
    matrix::padded_name("enospc", index, NAME_LENGTH)
}

impl Family for SpilledEnospc {
    type State = EnospcState;

    fn name(&self) -> &'static str {
        "ENOSPC during spilled metadata allocation"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(2048, 2048)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> EnospcState {
        volume.set_reclaim_batch_blocks(1);
        matrix::populate(volume, "tree", POPULATION, NAME_LENGTH);
        let pressure = volume.create_file_in_root("pressure", b"", ts(1)).unwrap();
        reserve_available(volume, pressure);
        EnospcState {
            populated: POPULATION,
            relieved: false,
        }
    }

    fn captured(&self, _state: &EnospcState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &EnospcState,
    ) -> Result<(), CoreError> {
        let names: Vec<String> = (0..BATCH).map(batch_name).collect();
        let operations: Vec<_> = names
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content: b"",
            })
            .collect();
        volume.run_batch(&operations, ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &EnospcState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        for index in 0..BATCH {
            let id = volume.lookup_root(&batch_name(index)).unwrap();
            assert_eq!(id.is_some(), delta == 1, "{context}: batch entry {index}");
        }
        let pressure = !state.relieved;
        assert_eq!(
            volume.lookup_root("pressure").unwrap().is_some(),
            pressure,
            "{context}"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated + usize::from(pressure) + BATCH * delta as usize,
            "{context}"
        );
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::NoSpace)
    }

    fn refusal_may_spill(&self) -> bool {
        true
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut EnospcState) -> u64 {
        state.relieved = true;
        let start = volume.generation();
        volume.delete_file_in_root("pressure", ts(40)).unwrap();
        volume.set_reclaim_batch_blocks(4096);
        // A zero-block step can advance the protected generation, so drain
        // until ordinary allocation has room for the batch.
        let mut steps = 0;
        while volume.available_blocks() < RETRY_AVAILABLE {
            volume.reclaim_step(ts(41 + steps)).unwrap();
            steps += 1;
            assert!(steps < 256, "reclaim did not restore capacity");
        }
        volume.generation() - start
    }
}

crate::profile_tests!(near_full_delete_cuts_and_faults, |pages| matrix::plain(
    &NearFullDelete,
    pages,
    12
));
crate::profile_tests!(near_full_delete_eviction, |pages| matrix::eviction(
    &NearFullDelete,
    pages,
    Some(12)
));
crate::profile_tests!(near_full_delete_ambiguous, |pages| matrix::ambiguous(
    &NearFullDelete,
    pages,
    Variant::Plain
));
crate::profile_tests!(spilled_enospc_refusal, |pages| matrix::refusal(
    &SpilledEnospc,
    pages
));
