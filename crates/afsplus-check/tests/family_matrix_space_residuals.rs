//! Residual low-space and in-place policy combinations of the tiny-cache
//! matrix: a resource refusal of the in-place write, a retained snapshot
//! across a near-full delete, and a data-block ENOSPC inside a spilled batch.
//! See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::{BatchOp, DataUpdatePolicy};
use afsplus_core::{CoreError, Volume};
use afsplus_format::OBJECT_ROOT;
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

const B: u64 = BS as u64;
const POPULATION: usize = 300;
const NAME_LENGTH: usize = 240;
const BATCH: usize = 64;
/// Blocks of content each entry of the data-bearing batch carries.
const BATCH_BLOCKS: usize = 2;

fn accounting<D: BlockDevice>(volume: &Volume<D>) -> (u64, u64) {
    (volume.free_blocks(), volume.reclaim_pending_blocks())
}

/// Preallocates the largest range ordinary allocation admits, leaving `leave`
/// blocks of capacity behind, and returns the reserved block count.
fn reserve_leaving(volume: &mut Volume<MemoryBackend>, file: u64, leave: u64) -> u64 {
    let mut reserved = volume.available_blocks().saturating_sub(leave);
    while volume
        .preallocate_file(file, 0, reserved * B, ts(2))
        .is_err()
    {
        reserved -= 1;
    }
    reserved
}

/// Preallocates the largest range ordinary allocation admits.
fn reserve_available(volume: &mut Volume<MemoryBackend>, file: u64) -> u64 {
    reserve_leaving(volume, file, 0)
}

/// Blocks of ordinary capacity the spilled batch fixture leaves behind: more
/// than the measured metadata demand of a batch of this shape and fewer than
/// its data blocks.
const ENOSPC_HEADROOM: u64 = 120;
/// Measured metadata demand of sixty-four spilled long-name entries.
const ENOSPC_METADATA: u64 = 89;

// ---------------------------------------------------------------------------
// Resource refusal of the in-place write.
// ---------------------------------------------------------------------------

/// Bytes of the flagged file before the operation.
fn in_place_before() -> Vec<u8> {
    vec![0x11; 2 * BS]
}

/// The extending write falls back to full COW, so it needs this many fresh
/// blocks; the near-full fixture has none of them.
const EXTENSION_BLOCKS: usize = 128;
const EXTENSION: usize = EXTENSION_BLOCKS * BS;

fn in_place_after() -> Vec<u8> {
    let mut after = in_place_before();
    after.resize(2 * BS + EXTENSION, 0);
    after[2 * BS..].fill(0x7e);
    after
}

struct InPlaceRefusal;

struct InPlaceState {
    file: u64,
    reserved: u64,
    relieved: bool,
}

impl Family for InPlaceRefusal {
    type State = InPlaceState;

    fn name(&self) -> &'static str {
        "in-place write resource refusal"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format {
            data_policy: true,
            ..Format::new(512, 512)
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> InPlaceState {
        volume.set_reclaim_batch_blocks(1);
        let file = volume
            .create_file_in_root("db", &in_place_before(), ts(2))
            .unwrap();
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, ts(3))
            .unwrap();
        let pressure = volume.create_file_in_root("pressure", b"", ts(4)).unwrap();
        let reserved = reserve_available(volume, pressure);
        InPlaceState {
            file,
            reserved,
            relieved: false,
        }
    }

    fn captured(&self, _state: &InPlaceState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &InPlaceState,
    ) -> Result<(), CoreError> {
        volume.write_file_at(state.file, 2 * B, &[0x7e; EXTENSION], ts(20))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &InPlaceState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.lookup_root("db").unwrap(),
            Some(state.file),
            "{context}"
        );
        assert_eq!(
            volume.file_data_policy(state.file).unwrap(),
            DataUpdatePolicy::InPlacePrivate,
            "{context}: policy flag"
        );
        let expected = if delta == 1 {
            in_place_after()
        } else {
            in_place_before()
        };
        assert_eq!(
            volume.read_file(state.file).unwrap(),
            expected,
            "{context}: bytes"
        );
        let pressure = !state.relieved;
        assert_eq!(
            volume.lookup_root("pressure").unwrap().is_some(),
            pressure,
            "{context}: pressure file"
        );
        if pressure {
            let id = volume.lookup_root("pressure").unwrap().unwrap();
            let metadata = volume.visible_metadata(id).unwrap().unwrap();
            assert_eq!(
                (metadata.size_bytes, metadata.allocated_bytes),
                (0, state.reserved * B),
                "{context}: reserved layout"
            );
        }
        assert_eq!(
            volume.list_root().unwrap().len(),
            1 + usize::from(pressure),
            "{context}: root count"
        );
    }

    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &InPlaceState,
        _variant: Variant,
    ) {
        assert_eq!(
            volume
                .last_commit_stats()
                .unwrap()
                .data_blocks_overwritten_in_place,
            0,
            "the extending write falls back to full COW"
        );
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::NoSpace)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut InPlaceState) -> u64 {
        state.relieved = true;
        let start = volume.generation();
        volume.delete_file_in_root("pressure", ts(40)).unwrap();
        volume.set_reclaim_batch_blocks(4096);
        let mut steps = 0;
        while volume.available_blocks() < 2 * EXTENSION_BLOCKS as u64 {
            volume.reclaim_step(ts(41 + steps)).unwrap();
            steps += 1;
            assert!(steps < 256, "reclaim did not restore capacity");
        }
        volume.generation() - start
    }
}

// ---------------------------------------------------------------------------
// Retained snapshot across a near-full delete.
// ---------------------------------------------------------------------------

/// Literal bytes of the two files the snapshot captures.
fn keep_bytes() -> Vec<u8> {
    vec![0x4b; 2 * BS]
}
fn victim_bytes() -> Vec<u8> {
    vec![0x76; BS]
}

struct RetainedNearFull;

struct RetainedState {
    keep: u64,
    victim: u64,
    reserved: u64,
    snapshot: u64,
}

impl RetainedNearFull {
    /// Free and pending blocks before and after the recorded delete.
    const EXPECTED: [(u64, u64); 2] = [(16, 14), (12, 17)];
}

impl Family for RetainedNearFull {
    type State = RetainedState;

    fn name(&self) -> &'static str {
        "near-full delete under a retained snapshot"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(512, 512)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> RetainedState {
        volume.set_reclaim_batch_blocks(1);
        let keep = volume
            .create_file_in_root("keep", &keep_bytes(), ts(1))
            .unwrap();
        let victim = volume
            .create_file_in_root("victim", &victim_bytes(), ts(2))
            .unwrap();
        let snapshot = volume.snapshot_create(ts(3)).unwrap();
        let pressure = volume.create_file_in_root("pressure", b"", ts(4)).unwrap();
        let reserved = reserve_available(volume, pressure);
        RetainedState {
            keep,
            victim,
            reserved,
            snapshot,
        }
    }

    fn snapshot(&self, state: &RetainedState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &RetainedState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.keep, keep_bytes()), (state.victim, victim_bytes())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &RetainedState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(1);
        volume.delete_file_in_root("victim", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RetainedState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let present = delta == 0;
        assert_eq!(
            volume.lookup_root("victim").unwrap(),
            present.then_some(state.victim),
            "{context}: victim name"
        );
        assert_eq!(
            volume.lookup_root("keep").unwrap(),
            Some(state.keep),
            "{context}: survivor name"
        );
        assert_eq!(
            volume.read_file(state.keep).unwrap(),
            keep_bytes(),
            "{context}: survivor bytes"
        );
        if present {
            assert_eq!(
                volume.read_file(state.victim).unwrap(),
                victim_bytes(),
                "{context}: victim bytes"
            );
        }
        let pressure = volume.lookup_root("pressure").unwrap().unwrap();
        let metadata = volume.visible_metadata(pressure).unwrap().unwrap();
        assert_eq!(
            (metadata.size_bytes, metadata.allocated_bytes),
            (0, state.reserved * B),
            "{context}: reserved layout"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            2 + usize::from(present),
            "{context}: root count"
        );
        assert_eq!(
            accounting(volume),
            Self::EXPECTED[delta as usize],
            "{context}: free/pending"
        );
    }
}

// ---------------------------------------------------------------------------
// Data-block ENOSPC inside a spilled batch.
// ---------------------------------------------------------------------------

struct SpilledDataEnospc;

struct DataEnospcState {
    populated: usize,
    probed: usize,
    relieved: bool,
}

/// Ordinary allocation capacity restored before the batch retry.
const RETRY_AVAILABLE: u64 = 512;

fn batch_name(index: usize) -> String {
    matrix::padded_name("payload", index, NAME_LENGTH)
}
fn probe_name(index: usize) -> String {
    matrix::padded_name("probe", index, NAME_LENGTH)
}
fn batch_content(index: usize) -> Vec<u8> {
    vec![(index % 251 + 2) as u8; BATCH_BLOCKS * BS]
}

impl Family for SpilledDataEnospc {
    type State = DataEnospcState;

    fn name(&self) -> &'static str {
        "data-block ENOSPC inside a spilled batch"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(2048, 2048)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> DataEnospcState {
        volume.set_reclaim_batch_blocks(1);
        matrix::populate(volume, "tree", POPULATION, NAME_LENGTH);
        // A batch of the same shape without content measures the metadata
        // demand of sixty-four spilled entries.
        let before = volume.available_blocks();
        let names: Vec<String> = (0..BATCH).map(probe_name).collect();
        let operations: Vec<_> = names
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content: b"",
            })
            .collect();
        volume.run_batch(&operations, ts(10)).unwrap();
        let metadata_demand = before - volume.available_blocks();
        let pressure = volume.create_file_in_root("pressure", b"", ts(11)).unwrap();
        reserve_leaving(volume, pressure, ENOSPC_HEADROOM);
        // The remaining capacity admits another metadata-only batch of this
        // shape and falls short of the batch's data blocks.
        assert_eq!(
            metadata_demand, ENOSPC_METADATA,
            "measured metadata demand of the batch shape"
        );
        let available = volume.available_blocks();
        assert!(
            available >= metadata_demand,
            "metadata demand {metadata_demand} exceeds the remaining {available} blocks"
        );
        assert!(
            available < (BATCH * BATCH_BLOCKS) as u64,
            "remaining {available} blocks cover the batch's data demand"
        );
        eprintln!(
            "{}: metadata demand {metadata_demand} blocks, remaining {available} blocks, data demand {} blocks",
            self.name(),
            BATCH * BATCH_BLOCKS
        );
        DataEnospcState {
            populated: POPULATION,
            probed: BATCH,
            relieved: false,
        }
    }

    fn captured(&self, _state: &DataEnospcState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &DataEnospcState,
    ) -> Result<(), CoreError> {
        let names: Vec<String> = (0..BATCH).map(batch_name).collect();
        let contents: Vec<Vec<u8>> = (0..BATCH).map(batch_content).collect();
        let operations: Vec<_> = names
            .iter()
            .zip(&contents)
            .map(|(name, content)| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content,
            })
            .collect();
        volume.run_batch(&operations, ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DataEnospcState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        for index in 0..BATCH {
            let id = volume.lookup_root(&batch_name(index)).unwrap();
            assert_eq!(id.is_some(), delta == 1, "{context}: batch entry {index}");
            if let Some(id) = id {
                assert_eq!(
                    volume.read_file(id).unwrap(),
                    batch_content(index),
                    "{context}: payload {index}"
                );
            }
            assert!(
                volume.lookup_root(&probe_name(index)).unwrap().is_some(),
                "{context}: probe entry {index}"
            );
        }
        let pressure = !state.relieved;
        assert_eq!(
            volume.lookup_root("pressure").unwrap().is_some(),
            pressure,
            "{context}: pressure file"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated + state.probed + usize::from(pressure) + BATCH * delta as usize,
            "{context}: root count"
        );
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::NoSpace)
    }

    fn refusal_may_spill(&self) -> bool {
        true
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut DataEnospcState) -> u64 {
        state.relieved = true;
        let start = volume.generation();
        volume.delete_file_in_root("pressure", ts(40)).unwrap();
        volume.set_reclaim_batch_blocks(4096);
        let mut steps = 0;
        while volume.available_blocks() < RETRY_AVAILABLE {
            volume.reclaim_step(ts(41 + steps)).unwrap();
            steps += 1;
            assert!(steps < 256, "reclaim did not restore capacity");
        }
        volume.generation() - start
    }
}

crate::profile_tests!(in_place_refusal, |pages| matrix::refusal(
    &InPlaceRefusal,
    pages
));
crate::profile_tests!(retained_near_full_delete, |pages| matrix::retained(
    &RetainedNearFull,
    pages,
    12
));
crate::profile_tests!(spilled_data_enospc_refusal, |pages| matrix::refusal(
    &SpilledDataEnospc,
    pages
));
