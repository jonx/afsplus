//! Residual batch and directory-structure families through the family-matrix
//! driver; see tiny_cache_matrix.md. These families close the combinations the
//! baseline structure matrix left open: a batch mixing creates and deletes with
//! payload tails longer than two blocks, a deferred-window batch under a
//! bounded cache, a leaf split inside a directory holding more than one
//! interior level, the single create, mkdir, rmdir and hard-link paths, a
//! cross-directory rename between two such deep directories, and symlink
//! targets at the maximum representable length.

mod common;

use afsplus_block::{
    BlockDevice, FaultBackend, FaultPlan, MemoryBackend, RecordedOp, TraceBackend,
};
use afsplus_core::volume::BatchOp;
use afsplus_core::{CoreError, Volume};
use afsplus_format::ident::Identification;
use afsplus_format::object::SymlinkRecord;
use afsplus_format::OBJECT_ROOT;
use common::family_matrix::{self as matrix, padded_name, ts, Family, Format, Variant, BS};

/// Volume geometry of the bounded fixtures.
const SMALL: u64 = 512;
/// Volume geometry of the pre-populated eviction fixtures.
const WIDE: u64 = 1024;
/// Volume geometry of the deep-directory fixtures.
const DEEP: u64 = 8192;
/// Entries the eviction fixtures hold in the root directory.
const POPULATION: usize = 150;
/// Long root names used to force staged-node eviction.
const NAME_LENGTH: usize = 240;
/// Subsets drawn per flush segment longer than the exhaustive budget.
const SAMPLE: usize = 32;

fn small() -> Format {
    Format::new(SMALL, SMALL as u32)
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

/// Reads the opaque symlink target with a sentinel past the end.
fn link_bytes<D: BlockDevice>(volume: &mut Volume<D>, id: u64, expected: &str, context: &str) {
    let mut read = vec![0xa5; expected.len() + 1];
    assert_eq!(
        volume.read_link(id, &mut read).unwrap(),
        expected.len(),
        "{context}: link length"
    );
    assert_eq!(
        &read[..expected.len()],
        expected.as_bytes(),
        "{context}: link target"
    );
    assert_eq!(read[expected.len()], 0xa5, "{context}: link EOF");
}

// ---------------------------------------------------------------------------
// One batch that both creates and deletes, with payload tails above two blocks
// ---------------------------------------------------------------------------

/// Payload lengths of the created entries. Every one spans more than two
/// blocks, so the borrowed payload path zero-pads a tail beyond the two-block
/// lengths of the baseline batch fixture.
const LONG_TAILS: [usize; 3] = [2 * BS + 1, 3 * BS, 4 * BS - 7];
/// Payload length of each entry the same batch removes.
const REMOVED_LENGTH: usize = BS + 5;

fn created_payload(slot: usize) -> Vec<u8> {
    vec![0x60 + slot as u8; LONG_TAILS[slot]]
}

fn removed_payload(slot: usize) -> Vec<u8> {
    vec![0x30 + slot as u8; REMOVED_LENGTH]
}

struct MixedBatchState {
    /// Names the batch creates, in batch order.
    created: Vec<String>,
    /// Names the batch removes, with their object identifiers.
    removed: Vec<(String, u64)>,
}

struct MixedBatch;

impl Family for MixedBatch {
    type State = MixedBatchState;

    fn name(&self) -> &'static str {
        "batch mixing creates and deletes"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> MixedBatchState {
        let created: Vec<String> = (0..LONG_TAILS.len())
            .map(|slot| format!("made-{slot:02}"))
            .collect();
        let removed = (0..LONG_TAILS.len())
            .map(|slot| {
                let name = format!("gone-{slot:02}");
                let id = volume
                    .create_file_in_root(&name, &removed_payload(slot), ts(10 + slot as i64))
                    .unwrap();
                (name, id)
            })
            .collect();
        MixedBatchState { created, removed }
    }

    fn captured(&self, state: &MixedBatchState) -> Vec<(u64, Vec<u8>)> {
        state
            .removed
            .iter()
            .enumerate()
            .map(|(slot, (_, id))| (*id, removed_payload(slot)))
            .collect()
    }

    /// The batch interleaves creates and deletes so one transaction stages
    /// both directions in the same directory leaf.
    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MixedBatchState,
    ) -> Result<(), CoreError> {
        let payloads: Vec<Vec<u8>> = (0..state.created.len()).map(created_payload).collect();
        let mut operations: Vec<BatchOp<'_>> = Vec::new();
        for slot in 0..state.created.len() {
            operations.push(BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: &state.created[slot],
                content: &payloads[slot],
            });
            operations.push(BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: &state.removed[slot].0,
            });
        }
        volume.run_batch(&operations, ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MixedBatchState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.created.len(),
            "{context}: root entries"
        );
        for (slot, name) in state.created.iter().enumerate() {
            let found = volume.lookup_root(name).unwrap();
            if published {
                let id = found.unwrap_or_else(|| panic!("{context}: {name} is absent"));
                file_bytes(volume, id, &created_payload(slot), context);
            } else {
                assert_eq!(found, None, "{context}: {name} is present");
            }
        }
        for (slot, (name, id)) in state.removed.iter().enumerate() {
            let found = volume.lookup_root(name).unwrap();
            if published {
                assert_eq!(found, None, "{context}: {name} survived");
                assert!(
                    volume.stat(*id).unwrap().is_none(),
                    "{context}: object {id} survived"
                );
            } else {
                assert_eq!(found, Some(*id), "{context}: {name}");
                file_bytes(volume, *id, &removed_payload(slot), context);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Deferred-window batch
// ---------------------------------------------------------------------------

/// Payload of each entry the deferred window creates.
const WINDOW_PAYLOAD: &[u8] = b"deferred window entry payload";
/// Payload of the entry the same window removes.
const WINDOW_VICTIM: &[u8] = b"deferred window victim payload";

struct WindowState {
    created: Vec<String>,
    victim: u64,
    population: usize,
}

struct WindowBatch;

impl Family for WindowBatch {
    type State = WindowState;

    fn name(&self) -> &'static str {
        "deferred-window batch"
    }

    fn format(&self, variant: Variant) -> Format {
        let blocks = if variant == Variant::Eviction {
            WIDE
        } else {
            SMALL
        };
        Format {
            log_slots: 8,
            ..Format::new(blocks, blocks as u32)
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> WindowState {
        let mut population = 0;
        let created: Vec<String> = if variant == Variant::Eviction {
            matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
            population = POPULATION;
            (0..3)
                .map(|slot| {
                    let index = POPULATION * slot / 3;
                    format!("{}~win", padded_name("wide", index, NAME_LENGTH))
                })
                .collect()
        } else {
            (0..3).map(|slot| format!("win-{slot:02}")).collect()
        };
        let victim = volume
            .create_file_in_root("window-victim", WINDOW_VICTIM, ts(5))
            .unwrap();
        WindowState {
            created,
            victim,
            population,
        }
    }

    fn captured(&self, state: &WindowState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.victim, WINDOW_VICTIM.to_vec())]
    }

    /// The window stages three creates and one delete and publishes them in a
    /// single deferred commit.
    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &WindowState,
    ) -> Result<(), CoreError> {
        for name in &state.created {
            volume.window_op(
                &BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name,
                    content: WINDOW_PAYLOAD,
                },
                ts(30),
            )?;
        }
        volume.window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "window-victim",
            },
            ts(30),
        )?;
        volume.window_commit(ts(31))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &WindowState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + if published { state.created.len() } else { 1 },
            "{context}: root entries"
        );
        for name in &state.created {
            let found = volume.lookup_root(name).unwrap();
            if published {
                let id = found.unwrap_or_else(|| panic!("{context}: {name} is absent"));
                file_bytes(volume, id, WINDOW_PAYLOAD, context);
            } else {
                assert_eq!(found, None, "{context}: {name} is present");
            }
        }
        // A deferred-window delete of a final visible link moves the object
        // into the reserved orphan directory (ADR-066), so its bytes survive
        // the publication and the application name disappears.
        let found = volume.lookup_root("window-victim").unwrap();
        assert_eq!(
            found,
            (!published).then_some(state.victim),
            "{context}: victim entry"
        );
        assert_eq!(
            volume.orphan_object(state.victim).unwrap(),
            published,
            "{context}: orphan entry"
        );
        assert_eq!(
            volume.orphan_count().unwrap(),
            u64::from(published),
            "{context}: orphan count"
        );
        file_bytes(volume, state.victim, WINDOW_VICTIM, context);
    }

    /// Measured demand of the deferred commit over a wide root directory.
    fn eviction_demand(&self) -> u64 {
        WINDOW_DEMAND
    }
}

/// Measured resident staged-node demand of the deferred-window commit over a
/// root directory holding 150 long names.
const WINDOW_DEMAND: u64 = 8;

/// A before-write fault at every recorded write and a failure at every flush
/// of a deferred-window commit. Any publication failure inside a window is
/// uncertain by contract, so the handle must refuse every further deferred or
/// immediate mutation and only a remount reconciles the volume. Remount
/// exposes exactly the publications that reached media, passes the checker and
/// admits the retry.
fn window_faults<F: Family>(
    family: &F,
    recording: &matrix::Recording<F::State>,
    pages: usize,
    variant: Variant,
) {
    let slots = Identification::decode(&recording.base.peek(0))
        .unwrap()
        .checkpoint_slots;
    let publications = family.publications(variant);
    let mut plans = Vec::new();
    let (mut write_index, mut flush_index, mut published) = (0u64, 0u64, 0u64);
    for operation in &recording.log {
        match operation {
            RecordedOp::Write { lba, .. } => {
                plans.push((
                    FaultPlan {
                        fail_write_index: Some(write_index),
                        ..Default::default()
                    },
                    published,
                ));
                published += u64::from(slots.contains(lba));
                write_index += 1;
            }
            RecordedOp::Flush => {
                plans.push((
                    FaultPlan {
                        fail_flush_index: Some(flush_index),
                        ..Default::default()
                    },
                    published,
                ));
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
    for (plan, published) in &plans {
        let (plan, published) = (*plan, *published);
        let context = format!("{} {variant:?} pages={pages} {plan:?}", family.name());
        let mut volume = matrix::open(FaultBackend::new(recording.base.clone(), plan), pages);
        assert!(
            family.apply(&mut volume, &recording.state).is_err(),
            "{context}: fault not reported"
        );
        assert!(volume.device_mut().tripped(), "{context}");
        assert!(
            matches!(
                family.apply(&mut volume, &recording.state),
                Err(CoreError::WindowPoisoned)
            ),
            "{context}: the window must refuse further deferred mutation"
        );
        assert!(
            matches!(
                volume.create_file_in_root("poison-probe", b"never", ts(990)),
                Err(CoreError::WindowPoisoned)
            ),
            "{context}: the window must refuse an immediate mutation"
        );
        let mut recovered = matrix::open(volume.into_device().into_inner(), pages);
        assert_eq!(
            recovered.generation(),
            recording.generation + published,
            "{context}: reconciled generation"
        );
        assert_eq!(
            recovered.lookup_root("poison-probe").unwrap(),
            None,
            "{context}: the refused probe must publish nothing"
        );
        family.verify(
            &mut recovered,
            &recording.state,
            variant,
            published,
            &context,
        );
        matrix::verify_captured(&mut recovered, &recording.captured, &context);
        matrix::assert_checker_clean(recovered.device_mut(), &context);
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
        let mut again = matrix::open(recovered.into_device(), pages);
        family.verify(
            &mut again,
            &recording.state,
            variant,
            publications,
            &context,
        );
        matrix::verify_captured(&mut again, &recording.captured, &context);
        matrix::assert_checker_clean(again.device_mut(), &context);
    }
    eprintln!(
        "{} {variant:?} pages={pages}: faults={} writes={write_index} flushes={flush_index} remount_required={}",
        family.name(),
        plans.len(),
        plans.len()
    );
}

/// Recording, sampled cuts and the remount-required fault matrix of one
/// deferred-window variant.
fn window_matrix(pages: usize, variant: Variant, seed: u64) {
    let recording = matrix::record(&WindowBatch, pages, variant);
    matrix::sampled_cuts(&WindowBatch, &recording, pages, variant, SAMPLE, seed);
    window_faults(&WindowBatch, &recording, pages, variant);
}

// ---------------------------------------------------------------------------
// Leaf split inside a directory with more than one interior level
// ---------------------------------------------------------------------------

/// Entries of the deep directory. With 250-byte names a leaf holds seven
/// entries, so this population raises two interior levels above the leaves.
const DEEP_ENTRIES: usize = 600;
/// Length of every name in the deep fixtures.
const DEEP_NAME_LENGTH: usize = 250;
/// Subjects the split batch inserts. They share one adjacent key range, so
/// they all land in the same full leaf and split it.
const DEEP_SUBJECTS: usize = 8;
/// Entry the subjects sort immediately after.
const DEEP_ANCHOR_INDEX: usize = 300;
/// Measured descent depth of the deep directory: a root, two interior levels
/// and the leaves.
const DEEP_DEPTH: u8 = 4;
/// Measured leaf splits of the deep split transaction.
const DEEP_SPLITS: u64 = 2;

const DEEP_BYTES: &[u8] = b"deep split subject";

fn deep_name(index: usize) -> String {
    padded_name("entry", index, DEEP_NAME_LENGTH)
}

/// Subject names sorting between entry `DEEP_ANCHOR_INDEX` and its successor.
fn deep_subjects() -> Vec<String> {
    (0..DEEP_SUBJECTS)
        .map(|slot| {
            let mut name = format!("entry-{DEEP_ANCHOR_INDEX:04}-s{slot:02}-");
            while name.len() < DEEP_NAME_LENGTH {
                name.push('s');
            }
            name
        })
        .collect()
}

fn fill_deep(volume: &mut Volume<MemoryBackend>, directory: u64) -> Vec<String> {
    let names: Vec<String> = (0..DEEP_ENTRIES).map(deep_name).collect();
    for chunk in names.chunks(64) {
        let operations: Vec<BatchOp<'_>> = chunk
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: directory,
                name,
                content: b"",
            })
            .collect();
        volume.run_batch(&operations, ts(2)).unwrap();
    }
    names
}

struct DeepState {
    directory: u64,
    present: Vec<String>,
    subjects: Vec<String>,
}

fn verify_deep<D: BlockDevice>(
    volume: &mut Volume<D>,
    state: &DeepState,
    holds_subjects: bool,
    context: &str,
) {
    let mut listed: Vec<String> = volume
        .list_directory(state.directory)
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    listed.sort();
    let mut expected = state.present.clone();
    if holds_subjects {
        expected.extend(state.subjects.iter().cloned());
    }
    expected.sort();
    assert_eq!(listed, expected, "{context}: directory entries");
    for subject in &state.subjects {
        let found = volume
            .lookup_in_directory(state.directory, subject)
            .unwrap();
        if holds_subjects {
            file_bytes(
                volume,
                found.unwrap_or_else(|| panic!("{context}: {subject} is absent")),
                DEEP_BYTES,
                context,
            );
        } else {
            assert_eq!(found, None, "{context}: {subject} is present");
        }
    }
    assert_eq!(
        volume.list_root().unwrap(),
        vec![("deep".to_string(), state.directory)],
        "{context}: root"
    );
}

struct DeepSplit;

impl Family for DeepSplit {
    type State = DeepState;

    fn name(&self) -> &'static str {
        "split of a deep directory"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(DEEP, DEEP as u32)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> DeepState {
        let directory = volume.create_directory_in_root("deep", ts(1)).unwrap();
        let present = fill_deep(volume, directory);
        DeepState {
            directory,
            present,
            subjects: deep_subjects(),
        }
    }

    fn captured(&self, _state: &DeepState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DeepState,
    ) -> Result<(), CoreError> {
        let operations: Vec<BatchOp<'_>> = state
            .subjects
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: state.directory,
                name,
                content: DEEP_BYTES,
            })
            .collect();
        volume.run_batch(&operations, ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DeepState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        verify_deep(volume, state, delta == 1, context);
    }

    /// The subjects fill one leaf of a directory whose descent passes two
    /// interior levels, so the transaction splits below an interior node and
    /// raises no new directory root.
    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &DeepState,
        _variant: Variant,
    ) {
        let stats = volume.last_commit_stats().unwrap().tree_mutations;
        assert_eq!(stats.max_depth, DEEP_DEPTH, "deep split: descent depth");
        assert_eq!(stats.splits, DEEP_SPLITS, "deep split: splits");
        assert_eq!(stats.root_splits, 0, "deep split: root splits");
    }
}

// ---------------------------------------------------------------------------
// Cross-directory rename between two deep directories
// ---------------------------------------------------------------------------

const DEEP_MOVED_BYTES: &[u8] = b"moved between two deep directories";

struct DeepRenameState {
    source: u64,
    target: u64,
    source_present: Vec<String>,
    target_present: Vec<String>,
    moved: String,
    file: u64,
}

struct DeepCrossRename;

impl DeepCrossRename {
    /// The moved name sorts into an adjacent key range of both directories.
    fn moved_name() -> String {
        let mut name = format!("entry-{DEEP_ANCHOR_INDEX:04}-m-");
        while name.len() < DEEP_NAME_LENGTH {
            name.push('m');
        }
        name
    }
}

impl Family for DeepCrossRename {
    type State = DeepRenameState;

    fn name(&self) -> &'static str {
        "rename between two deep directories"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(DEEP, DEEP as u32)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> DeepRenameState {
        let source = volume.create_directory_in_root("source", ts(1)).unwrap();
        let target = volume.create_directory_in_root("target", ts(2)).unwrap();
        let mut source_present = fill_deep(volume, source);
        let target_present = fill_deep(volume, target);
        let moved = Self::moved_name();
        let file = volume
            .create_file_in_directory(source, &moved, DEEP_MOVED_BYTES, ts(4))
            .unwrap();
        source_present.push(moved.clone());
        DeepRenameState {
            source,
            target,
            source_present,
            target_present,
            moved,
            file,
        }
    }

    fn captured(&self, state: &DeepRenameState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, DEEP_MOVED_BYTES.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DeepRenameState,
    ) -> Result<(), CoreError> {
        volume.rename(
            state.source,
            &state.moved,
            state.target,
            &state.moved,
            ts(30),
        )
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DeepRenameState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let moved = delta == 1;
        for (directory, present, holds) in [
            (state.source, &state.source_present, !moved),
            (state.target, &state.target_present, moved),
        ] {
            let mut listed: Vec<String> = volume
                .list_directory(directory)
                .unwrap()
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            listed.sort();
            let mut expected: Vec<String> = present
                .iter()
                .filter(|name| *name != &state.moved)
                .cloned()
                .collect();
            if holds {
                expected.push(state.moved.clone());
            }
            expected.sort();
            assert_eq!(listed, expected, "{context}: entries of {directory}");
        }
        let (holder, empty) = if moved {
            (state.target, state.source)
        } else {
            (state.source, state.target)
        };
        assert_eq!(
            volume.lookup_in_directory(holder, &state.moved).unwrap(),
            Some(state.file),
            "{context}: renamed entry"
        );
        assert_eq!(
            volume.lookup_in_directory(empty, &state.moved).unwrap(),
            None,
            "{context}: stale entry"
        );
        file_bytes(volume, state.file, DEEP_MOVED_BYTES, context);
        assert_eq!(
            volume.stat(state.file).unwrap().unwrap().link_count,
            1,
            "{context}: link count"
        );
    }

    /// Both directories carry two interior levels above their leaves.
    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &DeepRenameState,
        _variant: Variant,
    ) {
        let stats = volume.last_commit_stats().unwrap().tree_mutations;
        assert_eq!(stats.max_depth, DEEP_DEPTH, "deep rename: descent depth");
    }
}

// ---------------------------------------------------------------------------
// Single create, mkdir, rmdir and hard link
// ---------------------------------------------------------------------------

const SINGLE_BYTES: &[u8] = b"single create payload";
const OCCUPANT: &[u8] = b"occupant bytes the refused path must keep";

struct SingleState {
    /// Object occupying the operation's name in the refusal fixture.
    occupant: Option<u64>,
    /// Subject the operation acts on, when the fixture creates one.
    subject: u64,
    relieved: bool,
}

/// Creates the blocking occupant of the refusal fixture.
fn occupy(volume: &mut Volume<MemoryBackend>, variant: Variant, name: &str) -> Option<u64> {
    (variant == Variant::Refusal)
        .then(|| volume.create_file_in_root(name, OCCUPANT, ts(4)).unwrap())
}

/// Removes the occupant so the refused operation can be retried.
fn free_name<D: BlockDevice>(volume: &mut Volume<D>, state: &mut SingleState, name: &str) -> u64 {
    volume.delete_file_in_root(name, ts(40)).unwrap();
    state.occupant = None;
    state.relieved = true;
    1
}

struct SingleCreate;

impl Family for SingleCreate {
    type State = SingleState;

    fn name(&self) -> &'static str {
        "single file create"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
        SingleState {
            occupant: occupy(volume, variant, "subject"),
            subject: 0,
            relieved: false,
        }
    }

    fn captured(&self, _state: &SingleState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &SingleState,
    ) -> Result<(), CoreError> {
        volume
            .create_file_in_root("subject", SINGLE_BYTES, ts(30))
            .map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SingleState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            usize::from(published || state.occupant.is_some()),
            "{context}: root entries"
        );
        match volume.lookup_root("subject").unwrap() {
            None => assert!(
                !published && state.occupant.is_none(),
                "{context}: subject is absent"
            ),
            Some(id) if published => file_bytes(volume, id, SINGLE_BYTES, context),
            Some(id) => {
                assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                file_bytes(volume, id, OCCUPANT, context);
            }
        }
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::AlreadyExists)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
        free_name(volume, state, "subject")
    }
}

struct Mkdir;

impl Family for Mkdir {
    type State = SingleState;

    fn name(&self) -> &'static str {
        "mkdir"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
        SingleState {
            occupant: occupy(volume, variant, "made"),
            subject: 0,
            relieved: false,
        }
    }

    fn captured(&self, _state: &SingleState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &SingleState,
    ) -> Result<(), CoreError> {
        volume.create_directory_in_root("made", ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SingleState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            usize::from(published || state.occupant.is_some()),
            "{context}: root entries"
        );
        match volume.lookup_root("made").unwrap() {
            None => assert!(
                !published && state.occupant.is_none(),
                "{context}: directory is absent"
            ),
            Some(id) if published => {
                assert!(
                    volume.list_directory(id).unwrap().is_empty(),
                    "{context}: the new directory must be empty"
                );
            }
            Some(id) => {
                assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                file_bytes(volume, id, OCCUPANT, context);
            }
        }
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::AlreadyExists)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
        free_name(volume, state, "made")
    }
}

struct Rmdir;

impl Family for Rmdir {
    type State = SingleState;

    fn name(&self) -> &'static str {
        "rmdir"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    /// The refusal fixture leaves one entry inside the directory, so the
    /// removal is refused until the corrective step deletes it.
    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
        let subject = volume.create_directory_in_root("doomed", ts(2)).unwrap();
        if variant == Variant::Refusal {
            volume
                .create_file_in_directory(subject, "resident", OCCUPANT, ts(3))
                .unwrap();
        }
        SingleState {
            occupant: None,
            subject,
            relieved: variant != Variant::Refusal,
        }
    }

    fn captured(&self, _state: &SingleState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &SingleState,
    ) -> Result<(), CoreError> {
        volume.remove_directory(OBJECT_ROOT, "doomed", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SingleState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            usize::from(!published),
            "{context}: root entries"
        );
        if published {
            assert_eq!(volume.lookup_root("doomed").unwrap(), None, "{context}");
            assert!(
                volume.stat(state.subject).unwrap().is_none(),
                "{context}: the directory object survived"
            );
        } else {
            assert_eq!(
                volume.lookup_root("doomed").unwrap(),
                Some(state.subject),
                "{context}"
            );
            let listed = volume.list_directory(state.subject).unwrap();
            let expected = if state.relieved {
                Vec::new()
            } else {
                vec!["resident".to_string()]
            };
            assert_eq!(
                listed.into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
                expected,
                "{context}: directory entries"
            );
        }
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::DirectoryNotEmpty)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
        volume
            .delete_file(state.subject, "resident", ts(40))
            .unwrap();
        state.relieved = true;
        1
    }
}

struct HardLink;

impl Family for HardLink {
    type State = SingleState;

    fn name(&self) -> &'static str {
        "hard link"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
        let subject = volume
            .create_file_in_root("original", SINGLE_BYTES, ts(2))
            .unwrap();
        SingleState {
            occupant: occupy(volume, variant, "alias"),
            subject,
            relieved: false,
        }
    }

    fn captured(&self, state: &SingleState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.subject, SINGLE_BYTES.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SingleState,
    ) -> Result<(), CoreError> {
        volume.link_file(state.subject, OBJECT_ROOT, "alias", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SingleState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            1 + usize::from(published || state.occupant.is_some()),
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("original").unwrap(),
            Some(state.subject),
            "{context}: original entry"
        );
        file_bytes(volume, state.subject, SINGLE_BYTES, context);
        let alias = volume.lookup_root("alias").unwrap();
        let expected_links = if published { 2 } else { 1 };
        assert_eq!(
            volume.stat(state.subject).unwrap().unwrap().link_count,
            expected_links,
            "{context}: link count"
        );
        match alias {
            None => assert!(
                !published && state.occupant.is_none(),
                "{context}: alias is absent"
            ),
            Some(id) if published => {
                assert_eq!(id, state.subject, "{context}: alias identity");
                file_bytes(volume, id, SINGLE_BYTES, context);
            }
            Some(id) => {
                assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                file_bytes(volume, id, OCCUPANT, context);
            }
        }
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::AlreadyExists)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
        free_name(volume, state, "alias")
    }
}

// ---------------------------------------------------------------------------
// Symlink targets at the maximum representable length
// ---------------------------------------------------------------------------

/// Longest target an inline symlink record can represent at this block size.
fn max_target() -> String {
    let limit = SymlinkRecord::maximum_target_bytes(BS);
    let mut target = "WORK:afsplus/maximum/".to_string();
    while target.len() < limit {
        target.push('t');
    }
    assert_eq!(target.len(), limit);
    target
}

/// The second target, also at the maximum length and distinct from the first.
fn other_max_target() -> String {
    let mut target = max_target();
    target.replace_range(0..1, "S");
    target
}

struct MaxLinkState {
    directory: u64,
    link: u64,
    occupant: Option<u64>,
}

fn max_link_fixture(
    volume: &mut Volume<MemoryBackend>,
    variant: Variant,
    create_link: bool,
) -> MaxLinkState {
    let directory = volume.create_directory_in_root("links", ts(1)).unwrap();
    let link = if create_link {
        volume
            .create_symlink(OBJECT_ROOT, "link", &max_target(), ts(2))
            .unwrap()
    } else {
        0
    };
    let occupant = (variant == Variant::Refusal)
        .then(|| volume.create_file_in_root("link", OCCUPANT, ts(3)).unwrap());
    MaxLinkState {
        directory,
        link,
        occupant,
    }
}

struct MaxSymlinkCreate;

impl Family for MaxSymlinkCreate {
    type State = MaxLinkState;

    fn name(&self) -> &'static str {
        "symlink create at the maximum target length"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> MaxLinkState {
        max_link_fixture(volume, variant, false)
    }

    fn captured(&self, _state: &MaxLinkState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &MaxLinkState,
    ) -> Result<(), CoreError> {
        volume
            .create_symlink(OBJECT_ROOT, "link", &other_max_target(), ts(30))
            .map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MaxLinkState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            1 + usize::from(published || state.occupant.is_some()),
            "{context}: root entries"
        );
        match volume.lookup_root("link").unwrap() {
            None => assert!(
                !published && state.occupant.is_none(),
                "{context}: link is absent"
            ),
            Some(id) if published => link_bytes(volume, id, &other_max_target(), context),
            Some(id) => {
                assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                file_bytes(volume, id, OCCUPANT, context);
            }
        }
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::AlreadyExists)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut MaxLinkState) -> u64 {
        volume.delete_file_in_root("link", ts(40)).unwrap();
        state.occupant = None;
        1
    }
}

struct MaxSymlinkRename;

impl Family for MaxSymlinkRename {
    type State = MaxLinkState;

    fn name(&self) -> &'static str {
        "symlink rename at the maximum target length"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> MaxLinkState {
        max_link_fixture(volume, Variant::Plain, true)
    }

    fn captured(&self, _state: &MaxLinkState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MaxLinkState,
    ) -> Result<(), CoreError> {
        volume.rename(OBJECT_ROOT, "link", state.directory, "moved", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MaxLinkState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let moved = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            2 - usize::from(moved),
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("link").unwrap(),
            (!moved).then_some(state.link),
            "{context}: root link"
        );
        assert_eq!(
            volume
                .lookup_in_directory(state.directory, "moved")
                .unwrap(),
            moved.then_some(state.link),
            "{context}: moved link"
        );
        link_bytes(volume, state.link, &max_target(), context);
    }
}

struct MaxSymlinkUnlink;

impl Family for MaxSymlinkUnlink {
    type State = MaxLinkState;

    fn name(&self) -> &'static str {
        "symlink unlink at the maximum target length"
    }

    fn format(&self, _variant: Variant) -> Format {
        small()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> MaxLinkState {
        max_link_fixture(volume, Variant::Plain, true)
    }

    fn captured(&self, _state: &MaxLinkState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &MaxLinkState,
    ) -> Result<(), CoreError> {
        volume.unlink_symlink(OBJECT_ROOT, "link", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MaxLinkState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let removed = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            2 - usize::from(removed),
            "{context}: root entries"
        );
        if removed {
            assert_eq!(volume.lookup_root("link").unwrap(), None, "{context}");
            assert!(volume.stat(state.link).unwrap().is_none(), "{context}");
        } else {
            assert_eq!(
                volume.lookup_root("link").unwrap(),
                Some(state.link),
                "{context}"
            );
            link_bytes(volume, state.link, &max_target(), context);
        }
    }
}

/// A target one byte above the representable maximum must be refused without
/// any write or flush, and the maximum-length target must then publish.
fn over_long_symlink_target_refuses(pages: usize) {
    let context = format!("over-long symlink target pages={pages}");
    let mut volume = matrix::open(TraceBackend::new(small().device()), pages);
    let generation = volume.generation();
    volume.device_mut().reset();
    let mut target = max_target();
    target.push('t');
    assert_eq!(target.len(), SymlinkRecord::maximum_target_bytes(BS) + 1);
    assert!(
        matches!(
            volume.create_symlink(OBJECT_ROOT, "link", &target, ts(30)),
            Err(CoreError::InvalidMetadata(_))
        ),
        "{context}: an over-long target must be refused"
    );
    let stats = volume.device_mut().stats();
    assert_eq!(
        (stats.writes, stats.flushes),
        (0, 0),
        "{context}: refusal I/O"
    );
    assert_eq!(volume.generation(), generation, "{context}");
    assert!(volume.list_root().unwrap().is_empty(), "{context}");
    let id = volume
        .create_symlink(OBJECT_ROOT, "link", &max_target(), ts(31))
        .unwrap_or_else(|error| panic!("{context}: retry {error}"));
    link_bytes(&mut volume, id, &max_target(), &context);
    let mut remounted = matrix::open(volume.into_device().into_inner(), pages);
    link_bytes(&mut remounted, id, &max_target(), &context);
    matrix::assert_checker_clean(remounted.device_mut(), &context);
    eprintln!("{context}: refused writes=0 flushes=0 retry=1");
}

// ---------------------------------------------------------------------------
// Generated tests
// ---------------------------------------------------------------------------

/// Seeds of the sampled cut campaigns, one per family part.
const MIXED_BATCH_SEED: u64 = 0x5eed_ba7c_1001;
const WINDOW_BATCH_SEED: u64 = 0x5eed_ba7c_1002;
const DEEP_SPLIT_SEED: u64 = 0x5eed_ba7c_1003;
const DEEP_RENAME_SEED: u64 = 0x5eed_ba7c_1004;

crate::profile_tests!(mixed_batch, |pages| matrix::plain_sampled(
    &MixedBatch,
    pages,
    SAMPLE,
    MIXED_BATCH_SEED
));
crate::profile_tests!(mixed_batch_retained, |pages| matrix::retained_sampled(
    &MixedBatch,
    pages,
    SAMPLE,
    MIXED_BATCH_SEED
));
crate::profile_tests!(window_batch, |pages| window_matrix(
    pages,
    Variant::Plain,
    WINDOW_BATCH_SEED
));
crate::profile_tests!(window_batch_retained, |pages| {
    window_matrix(pages, Variant::Retained, WINDOW_BATCH_SEED);
    matrix::ambiguous(&WindowBatch, pages, Variant::Retained)
});
crate::profile_tests!(window_batch_eviction, |pages| window_matrix(
    pages,
    Variant::Eviction,
    WINDOW_BATCH_SEED
));
crate::profile_tests!(window_batch_spilled_ambiguous, |pages| matrix::ambiguous(
    &WindowBatch,
    pages,
    Variant::Eviction
));
crate::profile_tests!(deep_split, |pages| matrix::plain_sampled(
    &DeepSplit,
    pages,
    SAMPLE,
    DEEP_SPLIT_SEED
));
crate::profile_tests!(deep_cross_rename, |pages| matrix::plain_sampled(
    &DeepCrossRename,
    pages,
    SAMPLE,
    DEEP_RENAME_SEED
));
crate::profile_tests!(single_create, |pages| matrix::plain(
    &SingleCreate,
    pages,
    12
));
crate::profile_tests!(single_create_refusal, |pages| matrix::refusal(
    &SingleCreate,
    pages
));
crate::profile_tests!(mkdir, |pages| matrix::plain(&Mkdir, pages, 12));
crate::profile_tests!(mkdir_refusal, |pages| matrix::refusal(&Mkdir, pages));
crate::profile_tests!(rmdir, |pages| matrix::plain(&Rmdir, pages, 12));
crate::profile_tests!(rmdir_refusal, |pages| matrix::refusal(&Rmdir, pages));
crate::profile_tests!(hard_link, |pages| matrix::plain(&HardLink, pages, 12));
crate::profile_tests!(hard_link_retained, |pages| matrix::retained(
    &HardLink, pages, 12
));
crate::profile_tests!(hard_link_refusal, |pages| matrix::refusal(&HardLink, pages));
crate::profile_tests!(max_symlink_create, |pages| matrix::plain(
    &MaxSymlinkCreate,
    pages,
    12
));
crate::profile_tests!(max_symlink_create_refusal, |pages| matrix::refusal(
    &MaxSymlinkCreate,
    pages
));
crate::profile_tests!(max_symlink_rename, |pages| matrix::plain(
    &MaxSymlinkRename,
    pages,
    12
));
crate::profile_tests!(max_symlink_unlink, |pages| matrix::plain(
    &MaxSymlinkUnlink,
    pages,
    12
));
crate::profile_tests!(over_long_symlink_target, |pages| {
    over_long_symlink_target_refuses(pages)
});
