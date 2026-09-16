//! Residual persistent-snapshot combinations of the tiny-cache matrix: forced
//! eviction of the maintenance, release and snapshot-aware recovery commits,
//! an exhausted registry identity space planted beneath the mount, and a
//! maintenance pass whose ledger scan wraps. See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend, TraceBackend};
use afsplus_core::{CoreError, Volume};
use afsplus_format::tree::{key_u64, TreeNode};

use common::family_matrix::{self as matrix, ts, Family, Format, ReplayFamily, Variant, BS};

/// Long root names of the eviction fixtures.
const POPULATION: usize = 128;
const NAME_LENGTH: usize = 240;

const LIVE: &[u8] = b"live-content-of-the-subject-file";
const HISTORIC: &[u8] = b"historic";
/// Independent spellings of the payloads the oracles expect.
const EXPECTED_LIVE: &[u8] = b"live-content-of-the-subject-file";
const EXPECTED_HISTORIC: &[u8] = b"historic";
const CAPTURED_BYTE: u8 = 0x61;

fn image(variant: Variant) -> Format {
    match variant {
        Variant::Eviction => Format::new(4096, 16),
        _ => Format::new(1024, 256),
    }
}

fn populate(volume: &mut Volume<MemoryBackend>, variant: Variant) -> usize {
    if variant == Variant::Eviction {
        matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
        POPULATION
    } else {
        0
    }
}

/// Registered snapshot identities of the selected checkpoint.
fn registry<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<u64> {
    let page = volume.snapshot_list(0, 64).unwrap();
    assert_eq!(page.next_id, None, "registry page is complete");
    page.entries.into_iter().map(|entry| entry.id).collect()
}

/// Reads the whole file with a sentinel past the end.
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

/// Reads a captured object through the view of `snapshot`.
fn captured_bytes<D: BlockDevice>(
    volume: &mut Volume<D>,
    snapshot: u64,
    id: u64,
    expected: &[u8],
    context: &str,
) {
    let view = volume.snapshot_open(snapshot).unwrap();
    let mut read = vec![0xa5; expected.len() + 1];
    assert_eq!(
        volume
            .snapshot_read_file_at(&view, id, 0, &mut read)
            .unwrap(),
        expected.len(),
        "{context}: captured length of {id}"
    );
    assert_eq!(
        &read[..expected.len()],
        expected,
        "{context}: captured bytes of {id}"
    );
    assert_eq!(
        read[expected.len()],
        0xa5,
        "{context}: captured EOF of {id}"
    );
}

// ------------------------------------------ maintenance and release eviction

struct LedgerState {
    subject: u64,
    keeper: u64,
    snapshot: u64,
    background: usize,
}

/// One bounded maintenance step over a ledger that a captured delete filled,
/// on a root wide enough to measure the staged demand of its commit.
struct SnapshotMaintenance;

impl Family for SnapshotMaintenance {
    type State = LedgerState;

    fn name(&self) -> &'static str {
        "snapshot maintenance step residual"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> LedgerState {
        let background = populate(volume, variant);
        volume.set_reclaim_batch_blocks(1);
        let subject = volume
            .create_file_in_root("subject", &[CAPTURED_BYTE; 3 * BS], ts(1))
            .unwrap();
        let keeper = volume.create_file_in_root("keeper", LIVE, ts(2)).unwrap();
        let snapshot = volume.snapshot_create(ts(3)).unwrap();
        // The captured file leaves the live namespace, so its blocks enter the
        // snapshot ledger and maintenance has work to do.
        volume.delete_file_in_root("subject", ts(4)).unwrap();
        LedgerState {
            subject,
            keeper,
            snapshot,
            background,
        }
    }

    fn snapshot(&self, state: &LedgerState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &LedgerState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.subject, vec![CAPTURED_BYTE; 3 * BS]),
            (state.keeper, LIVE.to_vec()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &LedgerState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(2);
        volume.snapshot_maintenance_step(ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &LedgerState,
        _variant: Variant,
        _delta: u64,
        context: &str,
    ) {
        assert_eq!(
            registry(volume),
            vec![state.snapshot],
            "{context}: registry membership"
        );
        assert_eq!(
            volume.lookup_root("subject").unwrap(),
            None,
            "{context}: deleted entry"
        );
        file_bytes(volume, state.keeper, EXPECTED_LIVE, context);
        captured_bytes(
            volume,
            state.snapshot,
            state.subject,
            &[CAPTURED_BYTE; 3 * BS],
            context,
        );
        captured_bytes(volume, state.snapshot, state.keeper, EXPECTED_LIVE, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + 1,
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        MAINTENANCE_DEMAND
    }
}

/// Deleting the last view releases its ledger while the older selectable
/// checkpoint keeps every captured byte reachable.
struct SnapshotRelease;

impl Family for SnapshotRelease {
    type State = LedgerState;

    fn name(&self) -> &'static str {
        "snapshot release residual"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> LedgerState {
        let background = populate(volume, variant);
        volume.set_reclaim_batch_blocks(1);
        let subject = volume
            .create_file_in_root("subject", &[CAPTURED_BYTE; 3 * BS], ts(1))
            .unwrap();
        let keeper = volume.create_file_in_root("keeper", LIVE, ts(2)).unwrap();
        let snapshot = volume.snapshot_create(ts(3)).unwrap();
        volume.delete_file_in_root("subject", ts(4)).unwrap();
        LedgerState {
            subject,
            keeper,
            snapshot,
            background,
        }
    }

    fn captured(&self, _state: &LedgerState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &LedgerState,
    ) -> Result<(), CoreError> {
        volume.snapshot_delete(state.snapshot, ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &LedgerState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let held = delta == 0;
        assert_eq!(
            registry(volume),
            if held {
                vec![state.snapshot]
            } else {
                Vec::new()
            },
            "{context}: registry membership"
        );
        file_bytes(volume, state.keeper, EXPECTED_LIVE, context);
        assert_eq!(
            volume.lookup_root("subject").unwrap(),
            None,
            "{context}: deleted entry"
        );
        if held {
            captured_bytes(
                volume,
                state.snapshot,
                state.subject,
                &[CAPTURED_BYTE; 3 * BS],
                context,
            );
        } else {
            assert!(
                matches!(
                    volume.snapshot_open(state.snapshot),
                    Err(CoreError::NotFound)
                ),
                "{context}: released view is unreachable"
            );
        }
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + 1,
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        RELEASE_DEMAND
    }
}

// ------------------------------------------- snapshot-aware recovery eviction

/// Two durable groups over a file whose pre-write bytes a snapshot captures,
/// recovered on a volume with a wide root.
struct SnapshotRecovery;

struct RecoveryState {
    file: u64,
    snapshot: u64,
    background: usize,
}

impl ReplayFamily for SnapshotRecovery {
    type State = RecoveryState;

    fn name(&self) -> &'static str {
        "snapshot mount recovery residual"
    }

    fn format(&self, variant: Variant) -> Format {
        Format {
            log_slots: 8,
            ..image(variant)
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> RecoveryState {
        let background = populate(volume, variant);
        let file = volume.create_file_in_root("file", HISTORIC, ts(1)).unwrap();
        let snapshot = volume.snapshot_create(ts(2)).unwrap();
        RecoveryState {
            file,
            snapshot,
            background,
        }
    }

    fn snapshot(&self, state: &RecoveryState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &RecoveryState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, HISTORIC.to_vec())]
    }

    fn groups(&self) -> usize {
        2
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RecoveryState,
        index: usize,
    ) -> Result<(), CoreError> {
        if index == 0 {
            volume.window_write_file_at(state.file, 0, b"replaced", ts(10))?;
        } else {
            volume.window_truncate_file(state.file, 4, ts(11))?;
        }
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RecoveryState,
        _variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let expected: &[u8] = match acknowledged {
            0 => EXPECTED_HISTORIC,
            1 => b"replaced",
            _ => b"repl",
        };
        file_bytes(volume, state.file, expected, context);
        assert_eq!(
            registry(volume),
            vec![state.snapshot],
            "{context}: registry membership"
        );
        captured_bytes(
            volume,
            state.snapshot,
            state.file,
            EXPECTED_HISTORIC,
            context,
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + 1,
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        RECOVERY_DEMAND
    }
}

// --------------------------------------------- exhausted identity space

/// Rewrites the registry control record so the identity space is exhausted:
/// the value of key zero becomes the largest representable next identity and
/// the tree node is resealed with its own generation.
fn plant_exhausted_registry(image: &mut MemoryBackend, registry_block: u64) -> u64 {
    let block = image.peek(registry_block);
    let (mut node, generation) = TreeNode::decode(&block).expect("registry root is a tree node");
    assert!(node.is_leaf(), "the planted registry root must be a leaf");
    let control = node
        .items
        .iter_mut()
        .find(|item| item.key == key_u64(0))
        .expect("the registry holds a control record");
    assert_eq!(control.value.len(), 32, "registry control value length");
    let planted = u64::MAX;
    control.value[..8].copy_from_slice(&planted.to_le_bytes());
    let resealed = node
        .encode(image.block_size(), generation)
        .expect("resealed registry node");
    image.apply_raw(registry_block, &resealed);
    planted
}

/// An exhausted registry identity space refuses every further creation with
/// no write and no flush, live and after a remount, while the registered view
/// keeps its captured bytes and the image passes the checker.
fn exhausted_identity_space(pages: usize) {
    let mut volume = matrix::open(Format::new(1024, 256).device(), pages);
    let subject = volume.create_file_in_root("subject", LIVE, ts(1)).unwrap();
    let kept = volume.snapshot_create(ts(2)).unwrap();
    let registry_block = volume
        .checkpoint()
        .snapshot_roots
        .expect("the snapshot feature keeps roots")
        .registry;
    let generation = volume.generation();
    let mut base = volume.into_device();
    let planted = plant_exhausted_registry(&mut base, registry_block);
    assert_eq!(planted, u64::MAX, "planted next identity");
    matrix::assert_checker_clean(&mut base, "planted identity space");

    let mut volume = matrix::open(TraceBackend::new(base.clone()), pages);
    assert_eq!(volume.generation(), generation, "mount published");
    assert_eq!(registry(&mut volume), vec![kept], "planted membership");
    volume.device_mut().reset();
    let error = volume
        .snapshot_create(ts(3))
        .expect_err("an exhausted identity space admits no creation");
    assert_eq!(
        format!("{error:?}"),
        "Format(Overflow(\"snapshot ID exhausted\"))",
        "refusal error"
    );
    let stats = volume.device_mut().stats();
    assert_eq!((stats.writes, stats.flushes), (0, 0), "refusal issued I/O");
    assert_eq!(volume.generation(), generation, "refusal published");
    assert_eq!(
        registry(&mut volume),
        vec![kept],
        "membership after refusal"
    );
    file_bytes(&mut volume, subject, EXPECTED_LIVE, "refusal");
    captured_bytes(&mut volume, kept, subject, EXPECTED_LIVE, "refusal");

    // Releasing the registered identity frees a slot but not an identity.
    volume.snapshot_delete(kept, ts(4)).unwrap();
    assert!(registry(&mut volume).is_empty(), "membership after release");
    volume.device_mut().reset();
    let error = volume
        .snapshot_create(ts(5))
        .expect_err("releasing a view returns no identity");
    assert_eq!(
        format!("{error:?}"),
        "Format(Overflow(\"snapshot ID exhausted\"))",
        "refusal after release"
    );
    let stats = volume.device_mut().stats();
    assert_eq!(
        (stats.writes, stats.flushes),
        (0, 0),
        "refusal after release issued I/O"
    );
    let released = volume.generation();
    let mut after = volume.into_device().into_inner();
    matrix::assert_checker_clean(&mut after, "after release");

    // The remounted image refuses the same call with the same state.
    let mut remounted = matrix::open(TraceBackend::new(base), pages);
    assert_eq!(remounted.generation(), generation, "remounted generation");
    assert_eq!(registry(&mut remounted), vec![kept], "remounted membership");
    remounted.device_mut().reset();
    let error = remounted
        .snapshot_create(ts(6))
        .expect_err("the remounted image admits no creation");
    assert_eq!(
        format!("{error:?}"),
        "Format(Overflow(\"snapshot ID exhausted\"))",
        "remounted refusal"
    );
    let stats = remounted.device_mut().stats();
    assert_eq!(
        (stats.writes, stats.flushes),
        (0, 0),
        "remounted refusal issued I/O"
    );
    file_bytes(&mut remounted, subject, EXPECTED_LIVE, "remounted refusal");
    captured_bytes(&mut remounted, kept, subject, EXPECTED_LIVE, "remounted");
    // Ordinary namespace work is unaffected by the exhausted identity space.
    remounted
        .create_file_in_root("later", b"later-bytes", ts(7))
        .unwrap();
    assert_eq!(remounted.generation(), generation + 1, "ordinary work");
    let mut worked = remounted.into_device().into_inner();
    matrix::assert_checker_clean(&mut worked, "ordinary work");
    eprintln!(
        "exhausted identity space pages={pages} planted_next_id={planted} released_generation={released} refusals=3"
    );
}

// ------------------------------------------------------- wrapping ledger scan

/// Captured files whose single block the ledger retires. A live neighbour
/// between two of them keeps the retired runs from coalescing.
const RUNS: usize = 3;
/// Lifetime records one bounded scan admits, from the mount admission the
/// family-matrix driver applies.
const RECLAIM_RECORDS: u64 = 8;
/// Bounded steps the fixture runs before the recorded wrapping step.
const STEPS_BEFORE_WRAP: usize = 1;

struct WrapState {
    captured: Vec<(u64, Vec<u8>)>,
    keepers: Vec<u64>,
    snapshot: u64,
}

/// Builds a ledger of non-adjacent retired runs and returns the captured
/// objects: every deleted one-block file sits between two live ones.
fn fill_ledger(volume: &mut Volume<MemoryBackend>) -> WrapState {
    let mut captured = Vec::new();
    let mut keepers = Vec::new();
    for index in 0..RUNS {
        keepers.push(
            volume
                .create_file_in_root(
                    &format!("keep-{index:02}"),
                    &[0x40 + index as u8; BS],
                    ts(1),
                )
                .unwrap(),
        );
        let id = volume
            .create_file_in_root(&format!("run-{index:02}"), &[0xb0 + index as u8; BS], ts(2))
            .unwrap();
        captured.push((id, vec![0xb0 + index as u8; BS]));
    }
    let snapshot = volume.snapshot_create(ts(5)).unwrap();
    for index in 0..RUNS {
        volume
            .delete_file_in_root(&format!("run-{index:02}"), ts(6))
            .unwrap();
    }
    WrapState {
        captured,
        keepers,
        snapshot,
    }
}

fn verify_wrap_state<D: BlockDevice>(volume: &mut Volume<D>, state: &WrapState, context: &str) {
    assert_eq!(
        registry(volume),
        vec![state.snapshot],
        "{context}: registry membership"
    );
    for index in 0..RUNS {
        assert_eq!(
            volume.lookup_root(&format!("run-{index:02}")).unwrap(),
            None,
            "{context}: deleted entry {index}"
        );
        let keeper = state.keepers[index];
        assert_eq!(
            volume.lookup_root(&format!("keep-{index:02}")).unwrap(),
            Some(keeper),
            "{context}: live entry {index}"
        );
        file_bytes(volume, keeper, &[0x40 + index as u8; BS], context);
        captured_bytes(
            volume,
            state.snapshot,
            keeper,
            &[0x40 + index as u8; BS],
            context,
        );
    }
    for (index, (id, _)) in state.captured.iter().enumerate() {
        captured_bytes(
            volume,
            state.snapshot,
            *id,
            &[0xb0 + index as u8; BS],
            context,
        );
    }
    assert_eq!(
        volume.list_root().unwrap().len(),
        RUNS,
        "{context}: root entries"
    );
}

/// The bounded ledger scan stops after its record budget and resumes at the
/// recorded position until one pass wraps.
fn ledger_scan_resumes(pages: usize) {
    let mut volume = matrix::open(Format::new(1024, 256).device(), pages);
    volume.set_reclaim_batch_blocks(1);
    let state = fill_ledger(&mut volume);
    volume.set_reclaim_batch_blocks(2);
    let mut steps = Vec::new();
    let mut wrapped = 0;
    for _ in 0..16 {
        let report = volume.snapshot_maintenance_step(ts(30)).unwrap();
        steps.push((report.records_scanned, report.scan_wrapped));
        if report.scan_wrapped {
            wrapped += 1;
            break;
        }
        assert_eq!(
            report.records_scanned, RECLAIM_RECORDS,
            "a bounded step that does not wrap fills its budget: {steps:?}"
        );
    }
    assert_eq!(wrapped, 1, "the scan must wrap exactly once: {steps:?}");
    assert_eq!(
        steps.len(),
        STEPS_BEFORE_WRAP + 1,
        "bounded steps before the wrap: {steps:?}"
    );
    verify_wrap_state(&mut volume, &state, "wrapped ledger scan");
    let mut device = volume.into_device();
    matrix::assert_checker_clean(&mut device, "wrapped ledger scan");
    let mut remounted = matrix::open(device, pages);
    verify_wrap_state(&mut remounted, &state, "remounted wrapped ledger scan");
    eprintln!("ledger scan pages={pages} steps={steps:?}");
}

/// The maintenance pass that wraps the ledger scan, qualified through the
/// family-matrix driver: the fixture runs every earlier bounded step, so the
/// recorded commit is the one that crosses the resume boundary.
struct WrappingMaintenance;

impl Family for WrappingMaintenance {
    type State = WrapState;

    fn name(&self) -> &'static str {
        "snapshot wrapping ledger scan"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(1024, 256)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> WrapState {
        volume.set_reclaim_batch_blocks(1);
        let state = fill_ledger(volume);
        volume.set_reclaim_batch_blocks(2);
        for _ in 0..STEPS_BEFORE_WRAP {
            let report = volume.snapshot_maintenance_step(ts(20)).unwrap();
            assert!(
                !report.scan_wrapped,
                "the fixture must stop before the scan wraps"
            );
            assert_eq!(report.records_scanned, RECLAIM_RECORDS, "fixture step");
        }
        state
    }

    fn snapshot(&self, state: &WrapState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &WrapState) -> Vec<(u64, Vec<u8>)> {
        state.captured.clone()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &WrapState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(2);
        volume.snapshot_maintenance_step(ts(30)).map(|_| ())
    }

    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &WrapState,
        _variant: Variant,
    ) {
        let stats = volume.last_commit_stats().unwrap().snapshots;
        assert!(stats.scan_wrapped, "the recorded step must wrap the scan");
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &WrapState,
        _variant: Variant,
        _delta: u64,
        context: &str,
    ) {
        verify_wrap_state(volume, state, context);
    }
}

// -------------------------------- measured staged-node demands of the commits

const MAINTENANCE_DEMAND: u64 = 2;
const RELEASE_DEMAND: u64 = 2;
const RECOVERY_DEMAND: u64 = 2;

crate::profile_tests!(maintenance_eviction, |pages| matrix::eviction(
    &SnapshotMaintenance,
    pages,
    None
));
crate::profile_tests!(release_eviction, |pages| matrix::eviction(
    &SnapshotRelease,
    pages,
    None
));
crate::profile_tests!(mount_recovery_eviction, |pages| matrix::replay_eviction(
    &SnapshotRecovery,
    pages
));
crate::profile_tests!(identity_exhaustion, |pages| exhausted_identity_space(pages));
crate::profile_tests!(ledger_scan, |pages| ledger_scan_resumes(pages));
crate::profile_tests!(wrapping_maintenance, |pages| matrix::plain(
    &WrappingMaintenance,
    pages,
    12
));
