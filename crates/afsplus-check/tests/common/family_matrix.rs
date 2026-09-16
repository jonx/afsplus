//! Reusable family-matrix driver. A family supplies fixture setup, the
//! operation and independent expected states; the driver applies one explicit
//! cache profile from formatting through every mount and runs modeled cuts,
//! write/flush faults, ambiguous publication, retained-snapshot, forced
//! eviction and resource-refusal variants. The contract is documented in
//! tiny_cache_matrix.md, section "Family matrix driver".

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use afsplus_block::{
    for_each_crash_state_with_budget, BlockDevice, BlockError, FaultBackend, FaultPlan,
    MemoryBackend, RecordedOp, RecordingBackend, TraceBackend, TraceEvent,
};
use afsplus_check::check_device;
use afsplus_core::mount::select_checkpoint;
use afsplus_core::verify::load_committed_state;
use afsplus_core::volume::{BatchOp, ObjectMetadata, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_options, mount_with_snapshot_limits, CoreError, MkfsOptions, MkfsParams, MountMode,
    MountOptions, NamePolicy, Volume,
};
use afsplus_format::ident::Identification;
use afsplus_format::reclaim::ReclaimCaps;
use afsplus_format::{Timespec, OBJECT_ROOT};

pub const BS: usize = 4096;

pub fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

/// Fixture variant a family builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Minimal committed fixture.
    Plain,
    /// Plain fixture plus a persistent snapshot taken before the operation.
    Retained,
    /// Fixture whose trees are pre-populated so bounded profiles must spill.
    Eviction,
    /// Fixture in which the operation must be refused for lack of resources.
    Refusal,
    /// Fixture with ordinary allocation exhausted in which the operation is
    /// admitted through the emergency metadata headroom.
    Exhausted,
}

/// Geometry of the formatted image; persistent snapshots are always enabled.
#[derive(Clone, Copy, Debug)]
pub struct Format {
    pub blocks: u64,
    pub region_size: u32,
    pub log_slots: u16,
    pub data_policy: bool,
    pub reclaim_caps: ReclaimCaps,
}

impl Format {
    pub fn new(blocks: u64, region_size: u32) -> Self {
        Self {
            blocks,
            region_size,
            log_slots: 0,
            data_policy: false,
            reclaim_caps: ReclaimCaps::default(),
        }
    }

    pub fn device(self) -> MemoryBackend {
        let mut device = MemoryBackend::new(BS, self.blocks);
        mkfs_with_options(
            &mut device,
            &MkfsParams {
                uuid: [0x5c; 16],
                label: "FamilyMatrix".into(),
                region_size: self.region_size,
                reclaim_caps: self.reclaim_caps,
                log_slots: self.log_slots,
                shared_extents: true,
                data_policy: self.data_policy,
                name_policy: NamePolicy::Sensitive,
                timestamp: ts(0),
            },
            MkfsOptions {
                persistent_snapshots: true,
            },
        )
        .unwrap();
        device
    }
}

/// Mounts read-write with the explicit profile and asserts it took effect.
pub fn open<D: BlockDevice>(device: D, pages: usize) -> Volume<D> {
    open_mode(device, pages, MountMode::ReadWrite)
}

/// Rejects checker errors and every warning except a stopped intent-log tail,
/// so retained-checkpoint findings fail.
pub fn assert_checker_clean<D: BlockDevice>(device: &mut D, context: &str) {
    let report = check_device(device);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
    assert!(
        report
            .warnings
            .iter()
            .all(|warning| warning.starts_with("intent log tail:")),
        "{context}: {:?}",
        report.warnings
    );
}

/// Name of the `index`th pre-populated entry, padded to `length` bytes.
pub fn padded_name(prefix: &str, index: usize, length: usize) -> String {
    let mut name = format!("{prefix}-{index:04}-");
    while name.len() < length {
        name.push('p');
    }
    name
}

/// Creates `count` empty root files with long names in batches of 64 and
/// returns the number of checkpoints published.
pub fn populate<D: BlockDevice>(
    volume: &mut Volume<D>,
    prefix: &str,
    count: usize,
    length: usize,
) -> u64 {
    let names: Vec<String> = (0..count)
        .map(|index| padded_name(prefix, index, length))
        .collect();
    let mut commits = 0;
    for chunk in names.chunks(64) {
        let operations: Vec<_> = chunk
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content: b"",
            })
            .collect();
        volume.run_batch(&operations, ts(800)).unwrap();
        commits += 1;
    }
    commits
}

/// One family of mutations qualified by the driver.
pub trait Family {
    type State;

    fn name(&self) -> &'static str;
    fn format(&self, variant: Variant) -> Format;
    /// Builds the committed fixture on a mounted volume.
    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> Self::State;
    /// A snapshot taken inside `setup`; otherwise the driver takes one after it.
    fn snapshot(&self, _state: &Self::State) -> Option<u64> {
        None
    }
    /// Objects the retained snapshot must preserve, with literal bytes.
    fn captured(&self, state: &Self::State) -> Vec<(u64, Vec<u8>)>;
    /// The operation; it must also succeed from every intermediate publication.
    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &Self::State,
    ) -> Result<(), CoreError>;
    /// Checkpoints the operation publishes from the committed fixture.
    fn publications(&self, _variant: Variant) -> u64 {
        1
    }
    /// Asserts the exact state after `delta` publications (0 is the fixture).
    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &Self::State,
        variant: Variant,
        delta: u64,
        context: &str,
    );
    /// Family-specific assertions on the successful recorded commit.
    fn after_success<D: BlockDevice>(
        &self,
        _volume: &mut Volume<D>,
        _state: &Self::State,
        _variant: Variant,
    ) {
    }
    /// Literal resident staged-node demand of the eviction operation's last
    /// commit, observed at the unlimited profile.
    fn eviction_demand(&self) -> u64 {
        panic!("{} declares no eviction variant", self.name())
    }
    fn is_refusal(&self, _error: &CoreError) -> bool {
        false
    }
    /// Whether a refused attempt may write provisional images to blocks that
    /// no selectable checkpoint reaches. Flushes stay forbidden.
    fn refusal_may_spill(&self) -> bool {
        false
    }
    /// Performs the specified corrective step and returns its checkpoints.
    fn relieve<D: BlockDevice>(&self, _volume: &mut Volume<D>, _state: &mut Self::State) -> u64 {
        panic!("{} declares no refusal retry", self.name())
    }
}

pub struct Captured {
    pub snapshot: u64,
    pub objects: Vec<(u64, ObjectMetadata, Vec<u8>)>,
}

pub fn verify_captured<D: BlockDevice>(
    volume: &mut Volume<D>,
    captured: &Option<Captured>,
    context: &str,
) {
    let Some(captured) = captured else {
        return;
    };
    let view = volume.snapshot_open(captured.snapshot).unwrap();
    for (id, metadata, bytes) in &captured.objects {
        assert_eq!(
            volume.snapshot_stat(&view, *id).unwrap(),
            Some(*metadata),
            "{context}: captured metadata of {id}"
        );
        let mut read = vec![0xa5; bytes.len() + 1];
        assert_eq!(
            volume
                .snapshot_read_file_at(&view, *id, 0, &mut read)
                .unwrap(),
            bytes.len(),
            "{context}: captured length of {id}"
        );
        assert_eq!(
            &read[..bytes.len()],
            &bytes[..],
            "{context}: captured bytes of {id}"
        );
        assert_eq!(read[bytes.len()], 0xa5, "{context}: captured EOF of {id}");
    }
}

pub struct Recording<S> {
    pub base: MemoryBackend,
    pub state: S,
    pub generation: u64,
    pub captured: Option<Captured>,
    pub log: Vec<RecordedOp>,
    pub spills: u64,
    pub peak: u64,
}

/// Builds and verifies the committed fixture under the profile.
pub fn prepare<F: Family>(
    family: &F,
    pages: usize,
    variant: Variant,
) -> (MemoryBackend, F::State, u64, Option<Captured>) {
    let context = format!("{} {variant:?} pages={pages} fixture", family.name());
    let mut volume = open(family.format(variant).device(), pages);
    let state = family.setup(&mut volume, variant);
    let captured = (variant == Variant::Retained).then(|| {
        let snapshot = family
            .snapshot(&state)
            .unwrap_or_else(|| volume.snapshot_create(ts(900)).unwrap());
        let view = volume.snapshot_open(snapshot).unwrap();
        let objects = family
            .captured(&state)
            .into_iter()
            .map(|(id, bytes)| {
                let metadata = volume
                    .snapshot_stat(&view, id)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{context}: object {id} is not captured"));
                (id, metadata, bytes)
            })
            .collect();
        Captured { snapshot, objects }
    });
    let generation = volume.generation();
    family.verify(&mut volume, &state, variant, 0, &context);
    verify_captured(&mut volume, &captured, &context);
    let mut base = volume.into_device();
    assert_checker_clean(&mut base, &context);
    (base, state, generation, captured)
}

fn longest_unflushed_tail(log: &[RecordedOp]) -> usize {
    let (mut longest, mut current) = (0, 0);
    for operation in log {
        match operation {
            RecordedOp::Write { .. } => {
                current += 1;
                longest = longest.max(current);
            }
            RecordedOp::Flush => current = 0,
        }
    }
    longest
}

/// Records the successful operation, checks spill evidence, and verifies the
/// published state live, after remount and through the checker.
pub fn record<F: Family>(family: &F, pages: usize, variant: Variant) -> Recording<F::State> {
    let (base, state, generation, captured) = prepare(family, pages, variant);
    let context = format!("{} {variant:?} pages={pages} success", family.name());
    let publications = family.publications(variant);
    let mut volume = open(RecordingBackend::new(base.clone()), pages);
    family
        .apply(&mut volume, &state)
        .unwrap_or_else(|error| panic!("{context}: {error}"));
    assert_eq!(volume.generation(), generation + publications, "{context}");
    let stats = volume.last_commit_stats().unwrap().tree_mutations;
    let (spills, peak) = (stats.staged_spill_writes, stats.max_resident_staged_nodes);
    family.after_success(&mut volume, &state, variant);
    family.verify(&mut volume, &state, variant, publications, &context);
    verify_captured(&mut volume, &captured, &context);
    let (after, log) = volume.into_device().into_parts();
    let writes = log
        .iter()
        .filter(|operation| matches!(operation, RecordedOp::Write { .. }))
        .count();
    eprintln!(
        "{context}: writes={writes} flushes={} longest_tail={} spills={spills} peak_staged={peak}",
        log.len() - writes,
        longest_unflushed_tail(&log)
    );
    assert!(
        peak <= pages as u64,
        "{context}: resident staged nodes {peak}"
    );
    if variant == Variant::Eviction {
        let demand = family.eviction_demand();
        if pages == usize::MAX {
            assert_eq!((spills, peak), (0, demand), "{context}: unlimited demand");
        } else if demand > pages as u64 {
            assert!(spills > 0, "{context}: demand {demand} must spill");
        } else {
            assert_eq!(spills, 0, "{context}: demand {demand} fits the profile");
        }
    }
    let mut remounted = open(after, pages);
    family.verify(&mut remounted, &state, variant, publications, &context);
    verify_captured(&mut remounted, &captured, &context);
    assert_checker_clean(remounted.device_mut(), &context);
    Recording {
        base,
        state,
        generation,
        captured,
        log,
        spills,
        peak,
    }
}

/// Every modeled cut within `budget` unflushed writes; the fixture and the
/// final publication must both occur, and intermediate publications retry.
pub fn cuts<F: Family>(
    family: &F,
    recording: &Recording<F::State>,
    pages: usize,
    variant: Variant,
    budget: usize,
) {
    let publications = family.publications(variant);
    let mut outcomes = vec![0u64; publications as usize + 1];
    for cut in 0..=recording.log.len() {
        for_each_crash_state_with_budget(&recording.base, &recording.log, cut, budget, |state| {
            let context = format!(
                "{} {variant:?} pages={pages}: {}",
                family.name(),
                state.description
            );
            let mut image = state.image;
            assert_checker_clean(&mut image, &context);
            let mut volume = open(image, pages);
            let generation = volume.generation();
            let delta = generation
                .checked_sub(recording.generation)
                .filter(|delta| *delta <= publications)
                .unwrap_or_else(|| panic!("{context}: disallowed generation {generation}"));
            family.verify(&mut volume, &recording.state, variant, delta, &context);
            verify_captured(&mut volume, &recording.captured, &context);
            if delta > 0 && delta < publications {
                family
                    .apply(&mut volume, &recording.state)
                    .unwrap_or_else(|error| panic!("{context}: retry {error}"));
                family.verify(
                    &mut volume,
                    &recording.state,
                    variant,
                    publications,
                    &context,
                );
            }
            outcomes[delta as usize] += 1;
        });
    }
    assert!(
        outcomes[0] > 0 && outcomes[publications as usize] > 0,
        "{} {variant:?} pages={pages}: both outcomes required {outcomes:?}",
        family.name()
    );
    eprintln!(
        "{} {variant:?} pages={pages}: cuts budget={budget} outcomes={outcomes:?} images={}",
        family.name(),
        outcomes.iter().sum::<u64>()
    );
}

/// Representative tear offsets of the power-cut model.
const TEAR_OFFSETS: [usize; 3] = [64, 2048, 4064];
/// Flush segments up to this many writes enumerate every full-write subset.
const EXHAUSTIVE_SEGMENT: usize = 12;

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Visits full-write subsets of one flush segment applied over `durable`:
/// all of them up to `EXHAUSTIVE_SEGMENT` writes, otherwise `sample` seeded
/// subsets. Returns (exhaustive, sampled) image counts.
fn segment_subsets(
    durable: &MemoryBackend,
    segment: &[(u64, &Vec<u8>)],
    sample: usize,
    rng: &mut u64,
    visit: &mut dyn FnMut(MemoryBackend, String),
) -> (u64, u64) {
    if segment.is_empty() {
        return (0, 0);
    }
    let exhaustive = segment.len() <= EXHAUSTIVE_SEGMENT;
    let masks: Vec<Vec<bool>> = if exhaustive {
        (0u64..1 << segment.len())
            .map(|mask| {
                (0..segment.len())
                    .map(|bit| mask & (1 << bit) != 0)
                    .collect()
            })
            .collect()
    } else {
        (0..sample)
            .map(|_| {
                (0..segment.len())
                    .map(|_| splitmix64(rng) & 1 == 1)
                    .collect()
            })
            .collect()
    };
    for (number, mask) in masks.iter().enumerate() {
        let mut image = durable.clone();
        for ((lba, data), keep) in segment.iter().zip(mask) {
            if *keep {
                image.apply_raw(*lba, data);
            }
        }
        let kind = if exhaustive {
            "subset"
        } else {
            "sampled subset"
        };
        visit(
            image,
            format!("{kind} {number} of a {}-write segment", segment.len()),
        );
    }
    let count = masks.len() as u64;
    if exhaustive {
        (count, 0)
    } else {
        (0, count)
    }
}

/// Sampled cut campaign for a transaction whose unflushed tail exceeds the
/// exhaustive budget. Images: every in-order write prefix of the log, each
/// write torn at the representative offsets after its in-order prefix, every
/// full-write subset of each flush segment of at most twelve writes, and
/// `sample` subsets drawn with a SplitMix64 generator seeded by `seed` for
/// each longer segment. Every image uses the `cuts` oracle, and the fixture
/// and final publication must both occur.
pub fn sampled_cuts<F: Family>(
    family: &F,
    recording: &Recording<F::State>,
    pages: usize,
    variant: Variant,
    sample: usize,
    seed: u64,
) {
    let publications = family.publications(variant);
    let mut outcomes = vec![0u64; publications as usize + 1];
    let mut check = |image: MemoryBackend, description: String| {
        let context = format!("{} {variant:?} pages={pages}: {description}", family.name());
        let mut image = image;
        assert_checker_clean(&mut image, &context);
        let mut volume = open(image, pages);
        let generation = volume.generation();
        let delta = generation
            .checked_sub(recording.generation)
            .filter(|delta| *delta <= publications)
            .unwrap_or_else(|| panic!("{context}: disallowed generation {generation}"));
        family.verify(&mut volume, &recording.state, variant, delta, &context);
        verify_captured(&mut volume, &recording.captured, &context);
        if delta > 0 && delta < publications {
            family
                .apply(&mut volume, &recording.state)
                .unwrap_or_else(|error| panic!("{context}: retry {error}"));
            family.verify(
                &mut volume,
                &recording.state,
                variant,
                publications,
                &context,
            );
        }
        outcomes[delta as usize] += 1;
    };
    let mut rng = seed;
    let (mut prefixes, mut tears, mut exhaustive, mut sampled) = (1u64, 0u64, 0u64, 0u64);
    let mut prefix = recording.base.clone();
    let mut durable = recording.base.clone();
    let mut segment: Vec<(u64, &Vec<u8>)> = Vec::new();
    check(prefix.clone(), "in-order prefix of 0 operations".into());
    for (index, operation) in recording.log.iter().enumerate() {
        match operation {
            RecordedOp::Write { lba, data } => {
                for tear in TEAR_OFFSETS.iter().filter(|tear| **tear < data.len()) {
                    let mut image = prefix.clone();
                    let mut torn = image.peek(*lba);
                    torn[..*tear].copy_from_slice(&data[..*tear]);
                    image.apply_raw(*lba, &torn);
                    check(
                        image,
                        format!("write {index} (lba {lba}) torn at byte {tear}"),
                    );
                    tears += 1;
                }
                prefix.apply_raw(*lba, data);
                segment.push((*lba, data));
                check(
                    prefix.clone(),
                    format!("in-order prefix through operation {index}"),
                );
                prefixes += 1;
            }
            RecordedOp::Flush => {
                let (full, drawn) =
                    segment_subsets(&durable, &segment, sample, &mut rng, &mut check);
                exhaustive += full;
                sampled += drawn;
                durable = prefix.clone();
                segment.clear();
            }
        }
    }
    let (full, drawn) = segment_subsets(&durable, &segment, sample, &mut rng, &mut check);
    exhaustive += full;
    sampled += drawn;
    assert!(
        outcomes[0] > 0 && outcomes[publications as usize] > 0,
        "{} {variant:?} pages={pages}: both outcomes required {outcomes:?}",
        family.name()
    );
    eprintln!(
        "{} {variant:?} pages={pages}: sampled cut campaign seed={seed:#x} sample={sample} prefixes={prefixes} tears={tears} exhaustive_subsets={exhaustive} sampled_subsets={sampled} outcomes={outcomes:?} images={}",
        family.name(),
        outcomes.iter().sum::<u64>()
    );
}

/// A before-write fault at every recorded write and a failure at every flush.
/// A failed checkpoint write or its publication barrier must poison mutation;
/// any other fault leaves the handle retryable. Remount must expose exactly
/// the publications that reached media before the fault.
pub fn faults<F: Family>(
    family: &F,
    recording: &Recording<F::State>,
    pages: usize,
    variant: Variant,
) {
    let slots = Identification::decode(&recording.base.peek(0))
        .unwrap()
        .checkpoint_slots;
    let publications = family.publications(variant);
    let mut plans = Vec::new();
    let (mut write_index, mut flush_index, mut published) = (0u64, 0u64, 0u64);
    let mut after_checkpoint_write = false;
    for operation in &recording.log {
        match operation {
            RecordedOp::Write { lba, .. } => {
                let checkpoint = slots.contains(lba);
                let plan = FaultPlan {
                    fail_write_index: Some(write_index),
                    ..Default::default()
                };
                plans.push((plan, published, checkpoint));
                published += u64::from(checkpoint);
                after_checkpoint_write = checkpoint;
                write_index += 1;
            }
            RecordedOp::Flush => {
                let plan = FaultPlan {
                    fail_flush_index: Some(flush_index),
                    ..Default::default()
                };
                plans.push((plan, published, after_checkpoint_write));
                after_checkpoint_write = false;
                flush_index += 1;
            }
        }
    }
    assert_eq!(
        published,
        publications,
        "{}: checkpoint writes",
        family.name()
    );
    let mut same_handle = 0u64;
    for (plan, published, uncertain) in &plans {
        let (plan, published, uncertain) = (*plan, *published, *uncertain);
        let context = format!("{} {variant:?} pages={pages} {plan:?}", family.name());
        let mut volume = open(FaultBackend::new(recording.base.clone(), plan), pages);
        assert!(
            family.apply(&mut volume, &recording.state).is_err(),
            "{context}: fault not reported"
        );
        assert!(volume.device_mut().tripped(), "{context}");
        if uncertain {
            assert!(
                matches!(
                    family.apply(&mut volume, &recording.state),
                    Err(CoreError::WindowPoisoned)
                ),
                "{context}: uncertain publication must poison"
            );
        } else {
            assert_eq!(
                volume.generation(),
                recording.generation + published,
                "{context}"
            );
            family.verify(&mut volume, &recording.state, variant, published, &context);
        }
        let mut recovered = open(volume.into_device().into_inner(), pages);
        assert_eq!(
            recovered.generation(),
            recording.generation + published,
            "{context}: reconciled generation"
        );
        family.verify(
            &mut recovered,
            &recording.state,
            variant,
            published,
            &context,
        );
        verify_captured(&mut recovered, &recording.captured, &context);
        assert_checker_clean(recovered.device_mut(), &context);
        if published < publications {
            family
                .apply(&mut recovered, &recording.state)
                .unwrap_or_else(|error| panic!("{context}: retry after remount {error}"));
        }
        family.verify(
            &mut recovered,
            &recording.state,
            variant,
            publications,
            &context,
        );
        let mut again = open(recovered.into_device(), pages);
        family.verify(
            &mut again,
            &recording.state,
            variant,
            publications,
            &context,
        );
        verify_captured(&mut again, &recording.captured, &context);
        assert_checker_clean(again.device_mut(), &context);

        if !uncertain {
            let mut retry = open(FaultBackend::new(recording.base.clone(), plan), pages);
            assert!(family.apply(&mut retry, &recording.state).is_err());
            family
                .apply(&mut retry, &recording.state)
                .unwrap_or_else(|error| panic!("{context}: same-handle retry {error}"));
            family.verify(
                &mut retry,
                &recording.state,
                variant,
                publications,
                &context,
            );
            let mut remounted = open(retry.into_device().into_inner(), pages);
            family.verify(
                &mut remounted,
                &recording.state,
                variant,
                publications,
                &context,
            );
            verify_captured(&mut remounted, &recording.captured, &context);
            assert_checker_clean(remounted.device_mut(), &context);
            same_handle += 1;
        }
    }
    eprintln!(
        "{} {variant:?} pages={pages}: faults={} writes={write_index} flushes={flush_index} same_handle_retries={same_handle}",
        family.name(),
        plans.len()
    );
}

/// Completed checkpoint write reported failed, or reads failing after it.
struct AmbiguousDevice {
    inner: MemoryBackend,
    checkpoints: [u64; 2],
    published: bool,
    fail_write: bool,
    tripped: bool,
    writes: u64,
    flushes: u64,
}

impl BlockDevice for AmbiguousDevice {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if self.published && !self.fail_write {
            self.tripped = true;
            return Err(BlockError::Injected("post-publication read"));
        }
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.writes += 1;
        self.inner.write_block(lba, data)?;
        if self.checkpoints.contains(&lba) && !self.published {
            self.published = true;
            if self.fail_write {
                self.tripped = true;
                return Err(BlockError::Injected("completed checkpoint write"));
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.flushes += 1;
        self.inner.flush()
    }
}

/// The first checkpoint write completes but is reported failed, or later
/// adoption reads fail. The same and an independent mutation must return
/// `WindowPoisoned` without I/O; remount exposes exactly one publication.
pub fn ambiguous<F: Family>(family: &F, pages: usize, variant: Variant) {
    let publications = family.publications(variant);
    for fail_write in [true, false] {
        let (base, state, generation, captured) = prepare(family, pages, variant);
        let context = format!(
            "{} {variant:?} pages={pages} completed_write={fail_write}",
            family.name()
        );
        let checkpoints = Identification::decode(&base.peek(0))
            .unwrap()
            .checkpoint_slots;
        let mut volume = open(
            AmbiguousDevice {
                inner: base,
                checkpoints,
                published: false,
                fail_write,
                tripped: false,
                writes: 0,
                flushes: 0,
            },
            pages,
        );
        assert!(
            family.apply(&mut volume, &state).is_err(),
            "{context}: ambiguous publication must be reported"
        );
        let device = volume.device_mut();
        assert!(device.published && device.tripped, "{context}");
        let io = (device.writes, device.flushes);
        assert!(
            matches!(
                family.apply(&mut volume, &state),
                Err(CoreError::WindowPoisoned)
            ),
            "{context}: same operation must refuse"
        );
        assert!(
            matches!(
                volume.create_file_in_root("poison-probe", b"never published", ts(990)),
                Err(CoreError::WindowPoisoned)
            ),
            "{context}: independent mutation must refuse"
        );
        let device = volume.device_mut();
        assert_eq!(
            (device.writes, device.flushes),
            io,
            "{context}: refusal I/O"
        );
        let mut recovered = open(volume.into_device().inner, pages);
        assert_eq!(recovered.generation(), generation + 1, "{context}");
        assert_eq!(recovered.lookup_root("poison-probe").unwrap(), None);
        family.verify(&mut recovered, &state, variant, 1, &context);
        verify_captured(&mut recovered, &captured, &context);
        assert_checker_clean(recovered.device_mut(), &context);
        if publications > 1 {
            family
                .apply(&mut recovered, &state)
                .unwrap_or_else(|error| panic!("{context}: retry {error}"));
        }
        let mut again = open(recovered.into_device(), pages);
        family.verify(&mut again, &state, variant, publications, &context);
        verify_captured(&mut again, &captured, &context);
        assert_checker_clean(again.device_mut(), &context);
    }
    eprintln!(
        "{} {variant:?} pages={pages}: ambiguous cases=2",
        family.name()
    );
}

/// Blocks reachable from the selected and older selectable checkpoints.
fn protected_blocks(base: &MemoryBackend) -> BTreeSet<u64> {
    let mut device = base.clone();
    let ident = Identification::decode(&device.peek(0)).unwrap();
    let selection = select_checkpoint(&mut device, &ident).unwrap();
    let mut protected: BTreeSet<u64> = [0, ident.checkpoint_slots[0], ident.checkpoint_slots[1]]
        .into_iter()
        .collect();
    for checkpoint in std::iter::once(selection.chosen).chain(selection.other) {
        let state = load_committed_state(&mut device, &ident, &checkpoint).unwrap();
        protected.extend(state.metadata_blocks);
        protected.extend(state.data_blocks);
    }
    protected
}

/// The refusal fixture must refuse with no flush and no write to a reachable
/// block (no write at all unless the family allows provisional spills),
/// preserve the exact fixture through remount, then succeed after the
/// family's specified corrective step.
pub fn refusal<F: Family>(family: &F, pages: usize) {
    let variant = Variant::Refusal;
    let (base, mut state, generation, _) = prepare(family, pages, variant);
    let context = format!("{} {variant:?} pages={pages}", family.name());
    let protected = protected_blocks(&base);
    let mut volume = open(TraceBackend::new(base), pages);
    volume.device_mut().reset();
    let error = family
        .apply(&mut volume, &state)
        .expect_err("resource refusal expected");
    assert!(family.is_refusal(&error), "{context}: unexpected {error}");
    let io = volume.device_mut().stats();
    let written: BTreeSet<u64> = volume
        .device_mut()
        .events()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Write { lba } => Some(*lba),
            _ => None,
        })
        .collect();
    assert_eq!(io.flushes, 0, "{context}: refusal flushed");
    if family.refusal_may_spill() {
        assert!(
            written.is_disjoint(&protected),
            "{context}: refusal wrote reachable blocks {:?}",
            written.intersection(&protected).collect::<Vec<_>>()
        );
    } else {
        assert_eq!(io.writes, 0, "{context}: refusal wrote");
    }
    assert_eq!(volume.generation(), generation, "{context}");
    family.verify(&mut volume, &state, variant, 0, &context);
    let mut recovered = open(volume.into_device().into_inner(), pages);
    assert_eq!(recovered.generation(), generation, "{context}");
    family.verify(&mut recovered, &state, variant, 0, &context);
    assert_checker_clean(recovered.device_mut(), &context);
    let relieved = family.relieve(&mut recovered, &mut state);
    let generation = generation + relieved;
    assert_eq!(
        recovered.generation(),
        generation,
        "{context}: corrective step"
    );
    family
        .apply(&mut recovered, &state)
        .unwrap_or_else(|error| panic!("{context}: retry {error}"));
    let publications = family.publications(variant);
    assert_eq!(recovered.generation(), generation + publications);
    let retry_spills = recovered
        .last_commit_stats()
        .unwrap()
        .tree_mutations
        .staged_spill_writes;
    family.verify(&mut recovered, &state, variant, publications, &context);
    let mut again = open(recovered.into_device(), pages);
    family.verify(&mut again, &state, variant, publications, &context);
    assert_checker_clean(again.device_mut(), &context);
    eprintln!(
        "{context}: refused writes={} unreachable_targets={} flushes=0 corrective_commits={relieved} retry_spills={retry_spills}",
        io.writes,
        written.len()
    );
}

/// Successful recording, modeled cuts and faults of the plain fixture.
pub fn plain<F: Family>(family: &F, pages: usize, budget: usize) {
    let recording = record(family, pages, Variant::Plain);
    cuts(family, &recording, pages, Variant::Plain, budget);
    faults(family, &recording, pages, Variant::Plain);
}

/// Every outcome of the retained-snapshot fixture: success, cuts, faults and
/// ambiguous publication, each preserving the captured objects exactly.
pub fn retained<F: Family>(family: &F, pages: usize, budget: usize) {
    let recording = record(family, pages, Variant::Retained);
    cuts(family, &recording, pages, Variant::Retained, budget);
    faults(family, &recording, pages, Variant::Retained);
    ambiguous(family, pages, Variant::Retained);
}

/// Forced eviction: spill evidence, faults at every write including spills,
/// and modeled cuts when the tail fits an explicit budget.
pub fn eviction<F: Family>(family: &F, pages: usize, budget: Option<usize>) {
    let recording = record(family, pages, Variant::Eviction);
    faults(family, &recording, pages, Variant::Eviction);
    if let Some(budget) = budget {
        cuts(family, &recording, pages, Variant::Eviction, budget);
    }
}

/// Forced eviction whose unflushed tail exceeds the exhaustive budget: spill
/// evidence, faults at every write and the seeded sampled cut campaign.
pub fn eviction_sampled<F: Family>(family: &F, pages: usize, sample: usize, seed: u64) {
    let recording = record(family, pages, Variant::Eviction);
    faults(family, &recording, pages, Variant::Eviction);
    sampled_cuts(family, &recording, pages, Variant::Eviction, sample, seed);
}

/// Generates one test per explicit profile inside a named module.
#[macro_export]
macro_rules! profile_tests {
    ($name:ident, |$pages:ident| $body:expr) => {
        mod $name {
            #[allow(unused_imports)]
            use super::*;

            fn run($pages: usize) {
                $body
            }

            #[test]
            fn two_pages() {
                run(2)
            }

            #[test]
            fn four_pages() {
                run(4)
            }

            #[test]
            fn eight_pages() {
                run(8)
            }

            #[test]
            fn unlimited() {
                run(usize::MAX)
            }
        }
    };
}

/// Mounts with an explicit mount mode and profile and asserts the profile.
pub fn open_mode<D: BlockDevice>(device: D, pages: usize, mode: MountMode) -> Volume<D> {
    try_open_mode(device, pages, mode).unwrap()
}

/// Mount attempt that reports its error, for recovery under injected faults.
pub fn try_open_mode<D: BlockDevice>(
    device: D,
    pages: usize,
    mode: MountMode,
) -> Result<Volume<D>, CoreError> {
    let volume = mount_with_snapshot_limits(
        device,
        MountOptions {
            mode,
            tree_cache_pages: NonZeroUsize::new(pages),
        },
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 128,
            reclaim_records: 8,
        },
    )?;
    assert_eq!(volume.tree_cache_pages(), pages);
    Ok(volume)
}

/// One family whose work is acknowledged by intent-log fsync groups and
/// published by the recovery transaction of the next mount. The oracle is the
/// acknowledged-prefix protocol: the groups whose records reached media
/// survive, later staged work may be absent, and nothing else is admissible.
pub trait ReplayFamily {
    type State;

    fn name(&self) -> &'static str;
    /// The image; `log_slots` must be nonzero for a family of this kind.
    fn format(&self, variant: Variant) -> Format;
    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> Self::State;
    /// A snapshot taken inside `setup`; otherwise the driver takes one after it.
    fn snapshot(&self, _state: &Self::State) -> Option<u64> {
        None
    }
    fn captured(&self, state: &Self::State) -> Vec<(u64, Vec<u8>)>;
    /// Number of fsync groups the family makes durable.
    fn groups(&self) -> usize;
    /// Stages group `index` and makes it durable; publishes no checkpoint.
    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &Self::State,
        index: usize,
    ) -> Result<(), CoreError>;
    /// Asserts the exact state once `acknowledged` groups are recovered.
    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &Self::State,
        variant: Variant,
        acknowledged: usize,
        context: &str,
    );
    /// Literal resident staged-node demand of the recovery commit, observed
    /// at the unlimited profile.
    fn eviction_demand(&self) -> u64 {
        panic!("{} declares no eviction variant", self.name())
    }
}

pub struct ReplayRecording<S> {
    /// Committed fixture before any group is logged.
    pub base: MemoryBackend,
    pub state: S,
    pub generation: u64,
    pub captured: Option<Captured>,
    /// Device operations of the logging phase.
    pub log: Vec<RecordedOp>,
    /// Image in which every group is acknowledged and none is published.
    pub logged: MemoryBackend,
    /// Device operations of the recovery transaction over `logged`.
    pub recovery: Vec<RecordedOp>,
    pub spills: u64,
    pub peak: u64,
}

fn replay_prepare<F: ReplayFamily>(
    family: &F,
    pages: usize,
    variant: Variant,
) -> (MemoryBackend, F::State, u64, Option<Captured>) {
    let context = format!("{} {variant:?} pages={pages} fixture", family.name());
    let format = family.format(variant);
    assert!(format.log_slots > 0, "{context}: replay needs log slots");
    let mut volume = open(format.device(), pages);
    let state = family.setup(&mut volume, variant);
    let captured = (variant == Variant::Retained).then(|| {
        let snapshot = family
            .snapshot(&state)
            .unwrap_or_else(|| volume.snapshot_create(ts(900)).unwrap());
        let view = volume.snapshot_open(snapshot).unwrap();
        let objects = family
            .captured(&state)
            .into_iter()
            .map(|(id, bytes)| {
                let metadata = volume
                    .snapshot_stat(&view, id)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{context}: object {id} is not captured"));
                (id, metadata, bytes)
            })
            .collect();
        Captured { snapshot, objects }
    });
    let generation = volume.generation();
    family.verify(&mut volume, &state, variant, 0, &context);
    verify_captured(&mut volume, &captured, &context);
    let mut base = volume.into_device();
    assert_checker_clean(&mut base, &context);
    (base, state, generation, captured)
}

/// Logs every group, checks that logging publishes no checkpoint, and records
/// the recovery transaction that publishes them all.
pub fn record_replay<F: ReplayFamily>(
    family: &F,
    pages: usize,
    variant: Variant,
) -> ReplayRecording<F::State> {
    let (base, state, generation, captured) = replay_prepare(family, pages, variant);
    let context = format!("{} {variant:?} pages={pages} logging", family.name());
    let groups = family.groups();
    let mut volume = open(RecordingBackend::new(base.clone()), pages);
    for index in 0..groups {
        family
            .log_group(&mut volume, &state, index)
            .unwrap_or_else(|error| panic!("{context}: group {index}: {error}"));
        assert_eq!(
            volume.window_unlogged_ops(),
            0,
            "{context}: group {index} is not durable"
        );
        assert_eq!(
            volume.generation(),
            generation,
            "{context}: logging published a checkpoint"
        );
    }
    let (logged, log) = volume.into_device().into_parts();
    let writes = log
        .iter()
        .filter(|operation| matches!(operation, RecordedOp::Write { .. }))
        .count();
    let raw = open_mode(logged.clone(), pages, MountMode::NoChanges);
    assert_eq!(raw.generation(), generation, "{context}: raw generation");
    assert_eq!(
        raw.pending_intent_records() as usize,
        groups,
        "{context}: acknowledged records"
    );
    drop(raw);
    let context = format!("{} {variant:?} pages={pages} recovery", family.name());
    let mut recovered = open_mode(
        RecordingBackend::new(logged.clone()),
        pages,
        MountMode::Recovery,
    );
    assert_eq!(
        recovered.generation(),
        generation + 1,
        "{context}: recovery publishes one checkpoint"
    );
    assert_eq!(recovered.pending_intent_records(), 0, "{context}");
    let stats = recovered.last_commit_stats().unwrap().tree_mutations;
    let (spills, peak) = (stats.staged_spill_writes, stats.max_resident_staged_nodes);
    family.verify(&mut recovered, &state, variant, groups, &context);
    verify_captured(&mut recovered, &captured, &context);
    let (after, recovery) = recovered.into_device().into_parts();
    let recovery_writes = recovery
        .iter()
        .filter(|operation| matches!(operation, RecordedOp::Write { .. }))
        .count();
    eprintln!(
        "{context}: groups={groups} log_writes={writes} log_flushes={} recovery_writes={recovery_writes} recovery_flushes={} recovery_tail={} spills={spills} peak_staged={peak}",
        log.len() - writes,
        recovery.len() - recovery_writes,
        longest_unflushed_tail(&recovery)
    );
    assert!(peak <= pages as u64, "{context}: staged nodes {peak}");
    if variant == Variant::Eviction {
        let demand = family.eviction_demand();
        if pages == usize::MAX {
            assert_eq!((spills, peak), (0, demand), "{context}: unlimited demand");
        } else if demand > pages as u64 {
            assert!(spills > 0, "{context}: demand {demand} must spill");
        } else {
            assert_eq!(spills, 0, "{context}: demand {demand} fits the profile");
        }
    }
    let mut remounted = open(after, pages);
    assert_eq!(remounted.generation(), generation + 1, "{context}: remount");
    family.verify(&mut remounted, &state, variant, groups, &context);
    verify_captured(&mut remounted, &captured, &context);
    assert_checker_clean(remounted.device_mut(), &context);
    ReplayRecording {
        base,
        state,
        generation,
        captured,
        log,
        logged,
        recovery,
        spills,
        peak,
    }
}

/// Recovers `image`, recovers the result again, and requires both mounts to
/// expose exactly `acknowledged` groups with a stable generation.
fn recover_twice<F: ReplayFamily>(
    family: &F,
    recording: &ReplayRecording<F::State>,
    pages: usize,
    variant: Variant,
    image: MemoryBackend,
    acknowledged: usize,
    context: &str,
) {
    let published = u64::from(acknowledged > 0);
    let mut recovered = open_mode(image, pages, MountMode::Recovery);
    assert_eq!(
        recovered.generation(),
        recording.generation + published,
        "{context}: recovered generation"
    );
    assert_eq!(recovered.pending_intent_records(), 0, "{context}: pending");
    family.verify(
        &mut recovered,
        &recording.state,
        variant,
        acknowledged,
        context,
    );
    verify_captured(&mut recovered, &recording.captured, context);
    assert_checker_clean(recovered.device_mut(), context);
    let mut again = open_mode(recovered.into_device(), pages, MountMode::Recovery);
    assert_eq!(
        again.generation(),
        recording.generation + published,
        "{context}: repeated recovery generation"
    );
    assert_eq!(again.pending_intent_records(), 0, "{context}: repeated");
    family.verify(&mut again, &recording.state, variant, acknowledged, context);
    verify_captured(&mut again, &recording.captured, context);
    assert_checker_clean(again.device_mut(), context);
}

/// Every modeled cut of the logging phase, including cuts inside an
/// intent-group publication. The acknowledged record count of the image names
/// the allowed state; recovery and a second recovery must reach exactly it.
pub fn replay_cuts<F: ReplayFamily>(
    family: &F,
    recording: &ReplayRecording<F::State>,
    pages: usize,
    variant: Variant,
    budget: usize,
) {
    let groups = family.groups();
    let mut outcomes = vec![0u64; groups + 1];
    for cut in 0..=recording.log.len() {
        for_each_crash_state_with_budget(&recording.base, &recording.log, cut, budget, |state| {
            let context = format!(
                "{} {variant:?} pages={pages} logging: {}",
                family.name(),
                state.description
            );
            let mut image = state.image;
            assert_checker_clean(&mut image, &context);
            let raw = open_mode(image, pages, MountMode::NoChanges);
            assert_eq!(
                raw.generation(),
                recording.generation,
                "{context}: logging published a checkpoint"
            );
            let acknowledged = raw.pending_intent_records() as usize;
            assert!(acknowledged <= groups, "{context}: {acknowledged} records");
            outcomes[acknowledged] += 1;
            recover_twice(
                family,
                recording,
                pages,
                variant,
                raw.into_device(),
                acknowledged,
                &context,
            );
        });
    }
    assert!(
        outcomes[0] > 0 && outcomes[groups] > 0,
        "{} {variant:?} pages={pages}: both outcomes required {outcomes:?}",
        family.name()
    );
    eprintln!(
        "{} {variant:?} pages={pages}: logging cuts budget={budget} acknowledged={outcomes:?} images={}",
        family.name(),
        outcomes.iter().sum::<u64>()
    );
}

/// Every modeled cut of the recovery transaction over the fully acknowledged
/// image. Each cut exposes the pre- or post-publication checkpoint, and a
/// recovery and a second recovery reach the complete acknowledged state.
pub fn recovery_cuts<F: ReplayFamily>(
    family: &F,
    recording: &ReplayRecording<F::State>,
    pages: usize,
    variant: Variant,
    budget: usize,
) {
    let groups = family.groups();
    let mut outcomes = [0u64; 2];
    for cut in 0..=recording.recovery.len() {
        for_each_crash_state_with_budget(
            &recording.logged,
            &recording.recovery,
            cut,
            budget,
            |state| {
                let context = format!(
                    "{} {variant:?} pages={pages} recovery: {}",
                    family.name(),
                    state.description
                );
                let mut image = state.image;
                assert_checker_clean(&mut image, &context);
                let raw = open_mode(image, pages, MountMode::NoChanges);
                let published = match raw.generation() {
                    generation if generation == recording.generation => false,
                    generation if generation == recording.generation + 1 => true,
                    generation => panic!("{context}: disallowed generation {generation}"),
                };
                assert_eq!(
                    raw.pending_intent_records() as usize,
                    if published { 0 } else { groups },
                    "{context}: pending records"
                );
                outcomes[usize::from(published)] += 1;
                recover_twice(
                    family,
                    recording,
                    pages,
                    variant,
                    raw.into_device(),
                    groups,
                    &context,
                );
            },
        );
    }
    assert!(
        outcomes[0] > 0 && outcomes[1] > 0,
        "{} {variant:?} pages={pages}: both outcomes required {outcomes:?}",
        family.name()
    );
    eprintln!(
        "{} {variant:?} pages={pages}: recovery cuts budget={budget} pre/post={outcomes:?} images={}",
        family.name(),
        outcomes.iter().sum::<u64>()
    );
}

/// Seeded cut campaign over the recovery transaction, for a replay whose
/// unflushed tail exceeds the exhaustive budget. Images: every in-order write
/// prefix, each write torn at the representative offsets after its in-order
/// prefix, and `sample` full-write subsets of each flush segment drawn with a
/// SplitMix64 generator seeded by `seed` (every subset when the segment has
/// fewer than `sample` of them). Each image recovers and recovers again to the
/// complete acknowledged state.
pub fn recovery_sampled_cuts<F: ReplayFamily>(
    family: &F,
    recording: &ReplayRecording<F::State>,
    pages: usize,
    variant: Variant,
    sample: usize,
    seed: u64,
) {
    let groups = family.groups();
    let mut outcomes = [0u64; 2];
    let mut check = |image: MemoryBackend, description: String| {
        let context = format!(
            "{} {variant:?} pages={pages} recovery: {description}",
            family.name()
        );
        let mut image = image;
        assert_checker_clean(&mut image, &context);
        let raw = open_mode(image, pages, MountMode::NoChanges);
        let published = match raw.generation() {
            generation if generation == recording.generation => false,
            generation if generation == recording.generation + 1 => true,
            generation => panic!("{context}: disallowed generation {generation}"),
        };
        assert_eq!(
            raw.pending_intent_records() as usize,
            if published { 0 } else { groups },
            "{context}: pending records"
        );
        outcomes[usize::from(published)] += 1;
        recover_twice(
            family,
            recording,
            pages,
            variant,
            raw.into_device(),
            groups,
            &context,
        );
    };
    let mut rng = seed;
    let (mut prefixes, mut tears, mut subsets) = (1u64, 0u64, 0u64);
    let mut prefix = recording.logged.clone();
    let mut durable = recording.logged.clone();
    let mut segment: Vec<(u64, &Vec<u8>)> = Vec::new();
    check(prefix.clone(), "in-order prefix of 0 operations".into());
    let draw = |durable: &MemoryBackend,
                    segment: &mut Vec<(u64, &Vec<u8>)>,
                    rng: &mut u64,
                    check: &mut dyn FnMut(MemoryBackend, String)|
     -> u64 {
        if segment.is_empty() {
            return 0;
        }
        let total = 1u128 << segment.len();
        let masks: Vec<Vec<bool>> = if total <= sample as u128 {
            (0u128..total)
                .map(|mask| (0..segment.len()).map(|bit| mask & (1 << bit) != 0).collect())
                .collect()
        } else {
            (0..sample)
                .map(|_| (0..segment.len()).map(|_| splitmix64(rng) & 1 == 1).collect())
                .collect()
        };
        for (number, mask) in masks.iter().enumerate() {
            let mut image = durable.clone();
            for ((lba, data), keep) in segment.iter().zip(mask) {
                if *keep {
                    image.apply_raw(*lba, data);
                }
            }
            check(
                image,
                format!("subset {number} of a {}-write segment", segment.len()),
            );
        }
        masks.len() as u64
    };
    for (index, operation) in recording.recovery.iter().enumerate() {
        match operation {
            RecordedOp::Write { lba, data } => {
                for tear in TEAR_OFFSETS.iter().filter(|tear| **tear < data.len()) {
                    let mut image = prefix.clone();
                    let mut torn = image.peek(*lba);
                    torn[..*tear].copy_from_slice(&data[..*tear]);
                    image.apply_raw(*lba, &torn);
                    check(
                        image,
                        format!("write {index} (lba {lba}) torn at byte {tear}"),
                    );
                    tears += 1;
                }
                prefix.apply_raw(*lba, data);
                segment.push((*lba, data));
                check(
                    prefix.clone(),
                    format!("in-order prefix through operation {index}"),
                );
                prefixes += 1;
            }
            RecordedOp::Flush => {
                subsets += draw(&durable, &mut segment, &mut rng, &mut check);
                durable = prefix.clone();
                segment.clear();
            }
        }
    }
    subsets += draw(&durable, &mut segment, &mut rng, &mut check);
    assert!(
        outcomes[0] > 0 && outcomes[1] > 0,
        "{} {variant:?} pages={pages}: both outcomes required {outcomes:?}",
        family.name()
    );
    eprintln!(
        "{} {variant:?} pages={pages}: sampled recovery campaign seed={seed:#x} sample={sample} prefixes={prefixes} tears={tears} subsets={subsets} pre/post={outcomes:?} images={}",
        family.name(),
        outcomes.iter().sum::<u64>()
    );
}

/// A before-write fault at every recovery write and a failure at every
/// recovery flush. The interrupted recovery must report the error; the media
/// then holds the writes that preceded the fault, and a later mount recovers
/// the complete acknowledged state and keeps it stable across a second
/// recovery.
pub fn replay_faults<F: ReplayFamily>(
    family: &F,
    recording: &ReplayRecording<F::State>,
    pages: usize,
    variant: Variant,
) {
    let groups = family.groups();
    let mut plans = Vec::new();
    let (mut writes, mut flushes) = (0u64, 0u64);
    let mut image = recording.logged.clone();
    for operation in &recording.recovery {
        match operation {
            RecordedOp::Write { lba, data } => {
                // The fault precedes its write, so the media holds exactly the
                // writes recorded before this one.
                plans.push((
                    FaultPlan {
                        fail_write_index: Some(writes),
                        ..Default::default()
                    },
                    image.clone(),
                ));
                image.apply_raw(*lba, data);
                writes += 1;
            }
            RecordedOp::Flush => {
                plans.push((
                    FaultPlan {
                        fail_flush_index: Some(flushes),
                        ..Default::default()
                    },
                    image.clone(),
                ));
                flushes += 1;
            }
        }
    }
    for (plan, interrupted) in &plans {
        let context = format!("{} {variant:?} pages={pages} {plan:?}", family.name());
        let attempt = try_open_mode(
            FaultBackend::new(recording.logged.clone(), *plan),
            pages,
            MountMode::Recovery,
        );
        assert!(
            attempt.is_err(),
            "{context}: interrupted recovery must report the fault"
        );
        let mut image = interrupted.clone();
        assert_checker_clean(&mut image, &context);
        let raw = open_mode(image, pages, MountMode::NoChanges);
        assert!(
            raw.generation() == recording.generation
                || raw.generation() == recording.generation + 1,
            "{context}: disallowed generation {}",
            raw.generation()
        );
        recover_twice(
            family,
            recording,
            pages,
            variant,
            raw.into_device(),
            groups,
            &context,
        );
    }
    eprintln!(
        "{} {variant:?} pages={pages}: recovery faults={} writes={writes} flushes={flushes}",
        family.name(),
        plans.len()
    );
}

/// Log durable groups, cut inside and after intent-group publication, recover
/// and recover again, and compare every image against the acknowledged prefix.
pub fn replay<F: ReplayFamily>(family: &F, pages: usize, variant: Variant, budget: usize) {
    let recording = record_replay(family, pages, variant);
    replay_cuts(family, &recording, pages, variant, budget);
    recovery_cuts(family, &recording, pages, variant, budget);
}

/// Replay whose recovery tail exceeds the exhaustive budget: logging cuts,
/// the seeded recovery campaign and the recovery fault matrix.
pub fn replay_sampled<F: ReplayFamily>(
    family: &F,
    pages: usize,
    variant: Variant,
    budget: usize,
    sample: usize,
    seed: u64,
) {
    let recording = record_replay(family, pages, variant);
    replay_cuts(family, &recording, pages, variant, budget);
    recovery_sampled_cuts(family, &recording, pages, variant, sample, seed);
    replay_faults(family, &recording, pages, variant);
}

/// Replay with the recovery fault matrix added.
pub fn replay_with_faults<F: ReplayFamily>(
    family: &F,
    pages: usize,
    variant: Variant,
    budget: usize,
) {
    let recording = record_replay(family, pages, variant);
    replay_cuts(family, &recording, pages, variant, budget);
    recovery_cuts(family, &recording, pages, variant, budget);
    replay_faults(family, &recording, pages, variant);
}
