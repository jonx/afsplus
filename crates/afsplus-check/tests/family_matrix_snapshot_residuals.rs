//! Residual persistent-snapshot combinations of the tiny-cache matrix: forced
//! eviction of the maintenance, release and snapshot-aware recovery commits,
//! an exhausted registry identity space planted beneath the mount, and a
//! maintenance pass whose ledger scan wraps. See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{CoreError, Volume};

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
