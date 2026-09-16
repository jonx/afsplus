//! Persistent-snapshot families through the family-matrix driver: registry
//! create and delete, ledger maintenance and release, the admission refusals
//! that write nothing, and snapshot-aware mount recovery through the replay
//! runner. See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend, TraceBackend};
use afsplus_core::volume::{DataUpdatePolicy, SnapshotWorkLimits};
use afsplus_core::{CoreError, MountMode, MountOptions, Volume};

use common::family_matrix::{self as matrix, ts, Family, Format, ReplayFamily, Variant, BS};

/// Long root names of the eviction fixtures.
const POPULATION: usize = 300;
const NAME_LENGTH: usize = 240;
/// Seeded full-write subsets drawn per oversized flush segment.
const SAMPLE: usize = 128;

fn image(variant: Variant) -> Format {
    match variant {
        Variant::Eviction => Format::new(16 * 1024, 16),
        Variant::Refusal => Format::new(256, 256),
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

const LIVE: &[u8] = b"live-content-of-the-subject-file";
const HISTORIC: &[u8] = b"historic";

// ------------------------------------------------------- registry create

/// One snapshot creation: the registry gains exactly one identity.
struct SnapshotCreate;

struct RegistryState {
    subject: u64,
    background: usize,
    existing: Vec<u64>,
    /// Identity the operation adds or removes.
    id: u64,
}

impl Family for SnapshotCreate {
    type State = RegistryState;

    fn name(&self) -> &'static str {
        "snapshot registry create"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> RegistryState {
        let background = populate(volume, variant);
        let subject = volume.create_file_in_root("subject", LIVE, ts(1)).unwrap();
        // The retained variant keeps an older view the driver captures.
        let existing = if variant == Variant::Retained {
            vec![volume.snapshot_create(ts(2)).unwrap()]
        } else {
            Vec::new()
        };
        RegistryState {
            subject,
            background,
            id: existing.len() as u64 + 1,
            existing,
        }
    }

    fn snapshot(&self, state: &RegistryState) -> Option<u64> {
        state.existing.first().copied()
    }

    fn captured(&self, state: &RegistryState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.subject, LIVE.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &RegistryState,
    ) -> Result<(), CoreError> {
        volume.snapshot_create(ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RegistryState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let mut expected = state.existing.clone();
        if delta == 1 {
            expected.push(state.id);
        }
        assert_eq!(registry(volume), expected, "{context}: registry membership");
        file_bytes(volume, state.subject, LIVE, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + 1,
            "{context}: root entries"
        );
        if delta == 1 {
            captured_bytes(volume, state.id, state.subject, LIVE, context);
        }
    }

    /// Registry root and allocation root: the measured demand of the
    /// registry commit, which fits every bounded profile.
    fn eviction_demand(&self) -> u64 {
        2
    }
}

// ------------------------------------------------------- registry delete

/// One snapshot deletion: the registry loses exactly that identity while an
/// older retained view keeps its captured bytes.
struct SnapshotDelete;

impl Family for SnapshotDelete {
    type State = RegistryState;

    fn name(&self) -> &'static str {
        "snapshot registry delete"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> RegistryState {
        let background = populate(volume, variant);
        let subject = volume.create_file_in_root("subject", LIVE, ts(1)).unwrap();
        let existing = if variant == Variant::Retained {
            vec![volume.snapshot_create(ts(2)).unwrap()]
        } else {
            Vec::new()
        };
        let id = volume.snapshot_create(ts(3)).unwrap();
        RegistryState {
            subject,
            background,
            existing,
            id,
        }
    }

    fn snapshot(&self, state: &RegistryState) -> Option<u64> {
        state.existing.first().copied()
    }

    fn captured(&self, state: &RegistryState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.subject, LIVE.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RegistryState,
    ) -> Result<(), CoreError> {
        volume.snapshot_delete(state.id, ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RegistryState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let mut expected = state.existing.clone();
        if delta == 0 {
            expected.push(state.id);
        }
        assert_eq!(registry(volume), expected, "{context}: registry membership");
        file_bytes(volume, state.subject, LIVE, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + 1,
            "{context}: root entries"
        );
        if delta == 0 {
            captured_bytes(volume, state.id, state.subject, LIVE, context);
        } else {
            assert!(
                matches!(volume.snapshot_open(state.id), Err(CoreError::NotFound)),
                "{context}: deleted view is unreachable"
            );
        }
    }

    fn eviction_demand(&self) -> u64 {
        3
    }
}

// ------------------------------------------ lifetime maintenance and release

/// One bounded maintenance step over a ledger that a captured delete filled.
struct SnapshotMaintenance;

struct LedgerState {
    subject: u64,
    keeper: u64,
    snapshot: u64,
}

impl Family for SnapshotMaintenance {
    type State = LedgerState;

    fn name(&self) -> &'static str {
        "snapshot maintenance step"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(1024, 256)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> LedgerState {
        volume.set_reclaim_batch_blocks(1);
        let subject = volume
            .create_file_in_root("subject", &[0x61; 3 * BS], ts(1))
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
        }
    }

    fn snapshot(&self, state: &LedgerState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &LedgerState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.subject, vec![0x61; 3 * BS]),
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
        file_bytes(volume, state.keeper, LIVE, context);
        captured_bytes(
            volume,
            state.snapshot,
            state.subject,
            &[0x61; 3 * BS],
            context,
        );
        captured_bytes(volume, state.snapshot, state.keeper, LIVE, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            1,
            "{context}: root entries"
        );
    }
}

/// Deleting the last snapshot releases its ledger while the older selectable
/// checkpoint keeps every captured byte reachable.
struct SnapshotRelease;

impl Family for SnapshotRelease {
    type State = LedgerState;

    fn name(&self) -> &'static str {
        "snapshot release"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(1024, 256)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> LedgerState {
        volume.set_reclaim_batch_blocks(1);
        let subject = volume
            .create_file_in_root("subject", &[0x61; 3 * BS], ts(1))
            .unwrap();
        let keeper = volume.create_file_in_root("keeper", LIVE, ts(2)).unwrap();
        let snapshot = volume.snapshot_create(ts(3)).unwrap();
        volume.delete_file_in_root("subject", ts(4)).unwrap();
        LedgerState {
            subject,
            keeper,
            snapshot,
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
        file_bytes(volume, state.keeper, LIVE, context);
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
                &[0x61; 3 * BS],
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
            1,
            "{context}: root entries"
        );
    }
}

// --------------------------------------------------- snapshot-aware recovery

/// A durable existing-file write recovered on a volume that holds a snapshot
/// of the pre-write bytes.
struct SnapshotRecovery;

struct RecoveryState {
    file: u64,
    snapshot: u64,
}

impl ReplayFamily for SnapshotRecovery {
    type State = RecoveryState;

    fn name(&self) -> &'static str {
        "snapshot mount recovery"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format {
            log_slots: 8,
            ..Format::new(1024, 256)
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> RecoveryState {
        let file = volume.create_file_in_root("file", HISTORIC, ts(1)).unwrap();
        let snapshot = volume.snapshot_create(ts(2)).unwrap();
        RecoveryState { file, snapshot }
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
            0 => HISTORIC,
            1 => b"replaced",
            _ => b"repl",
        };
        file_bytes(volume, state.file, expected, context);
        assert_eq!(
            registry(volume),
            vec![state.snapshot],
            "{context}: registry membership"
        );
        captured_bytes(volume, state.snapshot, state.file, HISTORIC, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            1,
            "{context}: root entries"
        );
    }
}

// --------------------------------------------------------------- refusals

/// The admission view limit refuses a further snapshot with no write and no
/// flush; deleting one identity admits the retry.
fn admission_limit_refusal(pages: usize) {
    let mut volume = matrix::open(TraceBackend::new(Format::new(1024, 256).device()), pages);
    let subject = volume.create_file_in_root("subject", LIVE, ts(1)).unwrap();
    volume
        .set_snapshot_work_limits(SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 2,
            reclaim_records: 8,
        })
        .unwrap();
    let first = volume.snapshot_create(ts(2)).unwrap();
    let second = volume.snapshot_create(ts(3)).unwrap();
    let generation = volume.generation();
    volume.device_mut().reset();
    let error = volume
        .snapshot_create(ts(4))
        .expect_err("the admission limit refuses a third view");
    assert!(matches!(error, CoreError::PrototypeLimit(_)), "{error:?}");
    let stats = volume.device_mut().stats();
    assert_eq!((stats.writes, stats.flushes), (0, 0), "refusal issued I/O");
    assert_eq!(volume.generation(), generation, "refusal published");
    assert_eq!(registry(&mut volume), vec![first, second]);

    // A busy view refuses deletion with no I/O either.
    let view = volume.snapshot_open(second).unwrap();
    volume.device_mut().reset();
    assert!(matches!(
        volume.snapshot_delete(second, ts(5)),
        Err(CoreError::Busy)
    ));
    let stats = volume.device_mut().stats();
    assert_eq!((stats.writes, stats.flushes), (0, 0), "busy refusal I/O");
    drop(view);

    // A missing identity refuses with no I/O.
    volume.device_mut().reset();
    assert!(matches!(
        volume.snapshot_delete(99, ts(5)),
        Err(CoreError::NotFound)
    ));
    let stats = volume.device_mut().stats();
    assert_eq!((stats.writes, stats.flushes), (0, 0), "missing refusal I/O");

    volume.snapshot_delete(first, ts(6)).unwrap();
    let third = volume.snapshot_create(ts(7)).unwrap();
    assert_eq!(registry(&mut volume), vec![second, third]);
    captured_bytes(&mut volume, third, subject, LIVE, "admission retry");
    let mut remounted = matrix::open(volume.into_device().into_inner(), pages);
    assert_eq!(registry(&mut remounted), vec![second, third]);
    matrix::assert_checker_clean(remounted.device_mut(), "admission retry");
    eprintln!("snapshot admission refusals pages={pages} refusals=3");
}

/// Refuses every write while a mount is attempted.
struct ForbidWrites(MemoryBackend);

impl BlockDevice for ForbidWrites {
    fn block_size(&self) -> usize {
        self.0.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.0.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.0.read_block(lba, buf)
    }
    fn write_block(&mut self, _lba: u64, _data: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::Injected("mount wrote before admission"))
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        Err(BlockError::Injected("mount flushed before admission"))
    }
}

/// An invalid snapshot admission is refused before the pending recovery of a
/// durable intent-log group writes anything.
fn admission_precedes_recovery_writes(pages: usize) {
    let format = Format {
        log_slots: 8,
        ..Format::new(1024, 256)
    };
    let mut volume = matrix::open(format.device(), pages);
    let file = volume.create_file_in_root("file", HISTORIC, ts(1)).unwrap();
    volume.snapshot_create(ts(2)).unwrap();
    volume.snapshot_create(ts(3)).unwrap();
    volume
        .window_write_file_at(file, 0, b"replaced", ts(4))
        .unwrap();
    volume.window_fsync().unwrap();
    let base = volume.into_device();
    let mut refusals = 0;
    for mode in [
        MountMode::ReadWrite,
        MountMode::Recovery,
        MountMode::ReadOnly,
        MountMode::NoChanges,
    ] {
        for bad in [
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 1,
                reclaim_records: 8,
            },
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 0,
                reclaim_records: 8,
            },
            SnapshotWorkLimits {
                max_edit_records: 0,
                max_views: 128,
                reclaim_records: 8,
            },
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 128,
                reclaim_records: 0,
            },
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 128,
                reclaim_records: 4097,
            },
        ] {
            let error = afsplus_core::mount_with_snapshot_limits(
                ForbidWrites(base.clone()),
                MountOptions {
                    mode,
                    tree_cache_pages: std::num::NonZeroUsize::new(pages),
                },
                bad,
            )
            .err()
            .expect("invalid admission is refused");
            assert!(
                matches!(error, CoreError::PrototypeLimit(_)),
                "{mode:?}: {error:?}"
            );
            refusals += 1;
        }
    }
    // The valid admission then recovers the acknowledged group.
    let mut recovered = matrix::open_mode(base, pages, MountMode::Recovery);
    assert_eq!(recovered.pending_intent_records(), 0);
    file_bytes(&mut recovered, file, b"replaced", "admission recovery");
    matrix::assert_checker_clean(recovered.device_mut(), "admission recovery");
    eprintln!("snapshot admission before recovery pages={pages} refusals={refusals}");
}

/// Fails every read of one block, so the protection check of the older
/// checkpoint's registry cannot complete.
struct FailRegistryRead {
    inner: MemoryBackend,
    block: u64,
    armed: bool,
    blocked: u64,
}

impl BlockDevice for FailRegistryRead {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if self.armed && lba == self.block {
            self.blocked += 1;
            return Err(BlockError::Injected("older registry read"));
        }
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

/// An unreadable registry in the previous checkpoint slot cannot authorize an
/// optional in-place write: the write falls back to copy on write, keeps every
/// byte exact and leaves the older selectable view intact.
fn previous_slot_protection_failure(pages: usize) {
    let format = Format {
        data_policy: true,
        ..Format::new(1024, 256)
    };
    let mut volume = matrix::open(format.device(), pages);
    let file = volume
        .create_file_in_root("subject", &[0x51; 2 * BS], ts(1))
        .unwrap();
    volume
        .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, ts(2))
        .unwrap();
    let doomed = volume.snapshot_create(ts(3)).unwrap();
    volume.snapshot_delete(doomed, ts(4)).unwrap();
    assert!(registry(&mut volume).is_empty());
    // The empty registry of this checkpoint moves into the previous slot
    // through one more publication, so the protection check reads it there.
    let registry_block = volume
        .checkpoint()
        .snapshot_roots
        .expect("the snapshot feature keeps roots")
        .registry;
    volume.create_file_in_root("scratch", b"", ts(5)).unwrap();
    let device = volume.into_device();
    let mut expected = vec![0x51; 2 * BS];
    expected[100..164].fill(0x9e);

    // A readable empty registry in the previous slot admits the in-place write.
    let mut plain = matrix::open(
        FailRegistryRead {
            inner: device.clone(),
            block: registry_block,
            armed: false,
            blocked: 0,
        },
        pages,
    );
    plain.write_file_at(file, 100, &[0x9e; 64], ts(6)).unwrap();
    assert_eq!(
        plain
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        1,
        "a readable empty previous registry must admit the in-place write"
    );
    file_bytes(&mut plain, file, &expected, "admitted in-place write");
    matrix::assert_checker_clean(&mut plain.into_device().inner, "admitted in-place write");

    // An unreadable one protects it: the write falls back to copy on write.
    let mut guarded = matrix::open(
        FailRegistryRead {
            inner: device,
            block: registry_block,
            armed: false,
            blocked: 0,
        },
        pages,
    );
    // Mount validates both slots, so the fault arms after the mount.
    guarded.device_mut().armed = true;
    let generation = guarded.generation();
    let error = guarded
        .write_file_at(file, 100, &[0x9e; 64], ts(6))
        .expect_err("an unreadable previous registry cannot authorize the write");
    assert!(
        matches!(error, CoreError::Block(BlockError::Injected(_))),
        "{error:?}"
    );
    assert_eq!(guarded.generation(), generation, "the refusal published");
    let blocked = guarded.device_mut().blocked;
    assert!(blocked > 0, "the previous registry was never consulted");
    guarded.device_mut().armed = false;
    let original = vec![0x51; 2 * BS];
    file_bytes(&mut guarded, file, &original, "protected refusal");
    let mut remounted = matrix::open(guarded.into_device().inner, pages);
    assert_eq!(remounted.generation(), generation, "the refusal published");
    file_bytes(&mut remounted, file, &original, "protected refusal");
    matrix::assert_checker_clean(remounted.device_mut(), "protected refusal");
    // The readable registry admits the same write afterwards.
    remounted
        .write_file_at(file, 100, &[0x9e; 64], ts(7))
        .unwrap();
    file_bytes(&mut remounted, file, &expected, "protected retry");
    matrix::assert_checker_clean(remounted.device_mut(), "protected retry");
    eprintln!("previous-slot protection pages={pages} blocked_reads={blocked}");
}

crate::profile_tests!(registry_create, |pages| matrix::plain(
    &SnapshotCreate,
    pages,
    12
));
crate::profile_tests!(registry_create_retained, |pages| matrix::retained(
    &SnapshotCreate,
    pages,
    12
));
crate::profile_tests!(registry_create_eviction, |pages| matrix::eviction_sampled(
    &SnapshotCreate,
    pages,
    SAMPLE,
    0x5eed_50a0_0001
));
crate::profile_tests!(registry_delete, |pages| matrix::plain(
    &SnapshotDelete,
    pages,
    12
));
crate::profile_tests!(registry_delete_retained, |pages| matrix::retained(
    &SnapshotDelete,
    pages,
    12
));
crate::profile_tests!(maintenance_step, |pages| matrix::plain(
    &SnapshotMaintenance,
    pages,
    12
));
crate::profile_tests!(maintenance_ambiguous, |pages| matrix::ambiguous(
    &SnapshotMaintenance,
    pages,
    Variant::Plain
));
crate::profile_tests!(release_last_view, |pages| matrix::plain(
    &SnapshotRelease,
    pages,
    12
));
crate::profile_tests!(release_ambiguous, |pages| matrix::ambiguous(
    &SnapshotRelease,
    pages,
    Variant::Plain
));
crate::profile_tests!(mount_recovery, |pages| matrix::replay_with_faults(
    &SnapshotRecovery,
    pages,
    Variant::Plain,
    12
));
crate::profile_tests!(admission_limits, |pages| admission_limit_refusal(pages));
crate::profile_tests!(admission_before_recovery, |pages| {
    admission_precedes_recovery_writes(pages)
});
crate::profile_tests!(previous_slot_protection, |pages| {
    previous_slot_protection_failure(pages)
});
