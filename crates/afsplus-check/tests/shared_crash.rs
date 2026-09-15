//! Crash-atomicity qualification for ADR-061 shared extents.
//!
//! Each checkpoint transaction is recorded at the block-device boundary and
//! cut after every write/flush.  Every modeled image must select exactly the
//! pre- or post-generation, pass the exhaustive checker, preserve contents,
//! and keep reference records synchronized with the selected namespace.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use afsplus_block::{
    for_each_crash_state, BlockDevice, FaultBackend, FaultPlan, MemoryBackend, RecordedOp,
    RecordingBackend,
};
use afsplus_check::check_device;
use afsplus_core::mount::select_checkpoint;
use afsplus_core::shared_extents::{self, SharedRun};
use afsplus_core::verify::load_committed_state;
use afsplus_core::volume::BatchOp;
use afsplus_core::{mkfs, mount, mount_with_options, MkfsParams, MountMode, MountOptions, Volume};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(label: &str, log_slots: u16) -> MemoryBackend {
    let mut device = MemoryBackend::new(BS, 256);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x63; 16],
            label: label.into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(1),
        },
    )
    .unwrap();
    device
}

fn shared_records<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<SharedRun> {
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

/// All data and metadata reachable from the selected checkpoint.  The next
/// transaction may recycle blocks that are reachable only from the older
/// checkpoint because it replaces that checkpoint slot when it publishes;
/// the selected state itself must remain intact until then.
fn protected_blocks(base: &MemoryBackend) -> BTreeSet<u64> {
    let mut device = base.clone();
    let ident = Identification::decode(&device.peek(0)).unwrap();
    let selection = select_checkpoint(&mut device, &ident).unwrap();
    let state = load_committed_state(&mut device, &ident, &selection.chosen).unwrap();
    let mut protected: BTreeSet<u64> = state.metadata_blocks.into_iter().collect();
    protected.extend(state.data_blocks);
    protected
}

fn assert_cow_targets(base: &MemoryBackend, operations: &[RecordedOp]) {
    let protected = protected_blocks(base);
    for operation in operations {
        if let RecordedOp::Write { lba, .. } = operation {
            assert!(
                !protected.contains(lba),
                "transaction overwrote block {lba} reachable from a selectable checkpoint"
            );
        }
    }
}

fn profile_mount<D: BlockDevice>(device: D, pages: usize) -> Volume<D> {
    let volume = mount_with_options(
        device,
        MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    volume
}

/// Rejects checker errors and retained-checkpoint warnings. A stopped
/// intent-log tail is the only admissible crash artifact.
fn assert_checker_clean<D: BlockDevice>(device: &mut D, context: &str) {
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

fn run_checkpoint_matrix_profile(
    base: &MemoryBackend,
    operations: &[RecordedOp],
    pre_generation: u64,
    pages: usize,
    mut verify: impl FnMut(&str, bool, &mut Volume<MemoryBackend>),
) {
    assert_cow_targets(base, operations);
    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    for crash_point in 0..=operations.len() {
        for_each_crash_state(base, operations, crash_point, |state| {
            let context = format!("pages={pages}: {}", state.description);
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(
                report.is_clean(),
                "{context}: checker findings {:?}",
                report.errors
            );
            let mut volume = mount_with_options(
                image,
                MountOptions {
                    tree_cache_pages: NonZeroUsize::new(pages),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{context}: mount failed: {error}"));
            assert_eq!(volume.tree_cache_pages(), pages);
            let post = match volume.generation() {
                generation if generation == pre_generation => {
                    pre_outcomes += 1;
                    false
                }
                generation if generation == pre_generation + 1 => {
                    post_outcomes += 1;
                    true
                }
                generation => panic!(
                    "{context}: generation {generation} is neither {pre_generation} nor {}",
                    pre_generation + 1
                ),
            };
            verify(&context, post, &mut volume);
        });
    }
    assert!(pre_outcomes > 0, "matrix produced no pre-state");
    assert!(post_outcomes > 0, "matrix produced no post-state");
    eprintln!("checkpoint pages={pages} old={pre_outcomes} new={post_outcomes}");
}

fn record_transaction(
    base: &MemoryBackend,
    pages: usize,
    operation: impl FnOnce(&mut Volume<RecordingBackend<MemoryBackend>>),
) -> Vec<RecordedOp> {
    let mut volume = profile_mount(RecordingBackend::new(base.clone()), pages);
    operation(&mut volume);
    volume.into_device().into_parts().1
}

/// Records the automatic recovery transaction produced when a durable intent
/// record is replayed at mount.  The returned base already contains the
/// durable record but still exposes the old checkpoint in NoChanges mode.
fn record_replay(logged: &MemoryBackend, pages: usize) -> Vec<RecordedOp> {
    let volume = profile_mount(RecordingBackend::new(logged.clone()), pages);
    volume.into_device().into_parts().1
}

fn run_replay_matrix(
    logged: &MemoryBackend,
    operations: &[RecordedOp],
    pre_generation: u64,
    pages: usize,
    mut verify_recovered: impl FnMut(&str, &mut Volume<MemoryBackend>),
) {
    assert_cow_targets(logged, operations);
    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    for crash_point in 0..=operations.len() {
        for_each_crash_state(logged, operations, crash_point, |state| {
            let context = format!("pages={pages}: {}", state.description);
            let mut raw_image = state.image.clone();
            let report = check_device(&mut raw_image);
            assert!(
                report.is_clean(),
                "{context}: pre-recovery checker findings {:?}",
                report.errors
            );
            let raw = mount_with_options(
                raw_image,
                MountOptions {
                    mode: MountMode::NoChanges,
                    tree_cache_pages: NonZeroUsize::new(pages),
                },
            )
            .unwrap_or_else(|error| panic!("{context}: no-changes mount failed: {error}"));
            assert_eq!(raw.tree_cache_pages(), pages);
            match raw.generation() {
                generation if generation == pre_generation => {
                    pre_outcomes += 1;
                    assert!(raw.pending_intent_records() > 0, "{context}");
                }
                generation if generation == pre_generation + 1 => {
                    post_outcomes += 1;
                    assert_eq!(raw.pending_intent_records(), 0, "{context}");
                }
                generation => panic!(
                    "{context}: raw generation {generation} is neither {pre_generation} nor {}",
                    pre_generation + 1
                ),
            }

            // A normal RW mount must either retry the still-pending group or
            // observe the already-published replay.  Both paths converge to
            // the same post-state and make the intent record stale.
            let mut recovered = profile_mount(state.image, pages);
            assert_eq!(recovered.generation(), pre_generation + 1, "{context}");
            assert_eq!(recovered.pending_intent_records(), 0, "{context}");
            verify_recovered(&context, &mut recovered);
            let mut recovered_image = recovered.into_device();
            let recovered_report = check_device(&mut recovered_image);
            assert!(
                recovered_report.is_clean(),
                "{context}: post-recovery checker findings {:?}",
                recovered_report.errors
            );
            let mut again = profile_mount(recovered_image, pages);
            assert_eq!(again.generation(), pre_generation + 1, "{context}");
            assert_eq!(again.pending_intent_records(), 0, "{context}");
            verify_recovered(&context, &mut again);
        });
    }
    assert!(pre_outcomes > 0, "replay matrix produced no pre-state");
    assert!(post_outcomes > 0, "replay matrix produced no post-state");
    eprintln!("replay pages={pages} old={pre_outcomes} new={post_outcomes}");
}

#[test]
fn first_clone_is_crash_atomic() {
    first_clone_profile(usize::MAX);
}
#[test]
fn first_clone_two_pages() {
    first_clone_profile(2);
}
#[test]
fn first_clone_four_pages() {
    first_clone_profile(4);
}
#[test]
fn first_clone_eight_pages() {
    first_clone_profile(8);
}
fn first_clone_profile(pages: usize) {
    let options = MountOptions {
        tree_cache_pages: NonZeroUsize::new(pages),
        ..Default::default()
    };
    let mut setup = mount_with_options(formatted("CrashFirstClone", 0), options).unwrap();
    assert_eq!(setup.tree_cache_pages(), pages);
    let content = vec![0x11u8; 2 * BS];
    let source = setup
        .create_file_in_root("source", &content, ts(2))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let mut volume = mount_with_options(RecordingBackend::new(base.clone()), options).unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    let clone_id = volume
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    let operations = volume.into_device().into_parts().1;

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            assert_eq!(volume.read_file(source).unwrap(), content, "{context}");
            if post {
                assert_eq!(
                    volume.lookup_root("clone").unwrap(),
                    Some(clone_id),
                    "{context}"
                );
                assert_eq!(volume.read_file(clone_id).unwrap(), content, "{context}");
                let records = shared_records(volume);
                assert_eq!(records.len(), 1, "{context}: {records:?}");
                assert_eq!(records[0].reference_count, 2, "{context}");
                assert_eq!(records[0].block_count, 2, "{context}");
            } else {
                assert_eq!(volume.lookup_root("clone").unwrap(), None, "{context}");
                assert_eq!(volume.checkpoint().shared_extent_root_block, 0, "{context}");
            }
        },
    );
}

#[test]
fn clone_range_with_multiple_reference_boundaries_is_crash_atomic() {
    clone_range_profile(usize::MAX);
}
#[test]
fn clone_range_boundaries_two_pages() {
    clone_range_profile(2);
}
#[test]
fn clone_range_boundaries_four_pages() {
    clone_range_profile(4);
}
#[test]
fn clone_range_boundaries_eight_pages() {
    clone_range_profile(8);
}
fn clone_range_profile(pages: usize) {
    let options = MountOptions {
        tree_cache_pages: NonZeroUsize::new(pages),
        ..Default::default()
    };
    let mut setup = mount_with_options(formatted("CrashCloneRange", 0), options).unwrap();
    assert_eq!(setup.tree_cache_pages(), pages);
    let source_bytes: Vec<u8> = (0..4 * BS).map(|index| (index / BS) as u8 + 1).collect();
    let old_destination = vec![0x92u8; 4 * BS];
    let source = setup
        .create_file_in_root("source", &source_bytes, ts(2))
        .unwrap();
    let peer = setup.create_file_in_root("peer", b"", ts(3)).unwrap();
    setup
        .clone_range(source, BS as u64, peer, 0, (2 * BS) as u64, ts(4))
        .unwrap();
    let destination = setup
        .create_file_in_root("destination", &old_destination, ts(5))
        .unwrap();
    let pre_records = shared_records(&mut setup);
    assert_eq!(pre_records.len(), 1);
    assert_eq!(pre_records[0].block_count, 2);
    assert_eq!(pre_records[0].reference_count, 2);
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let mut recorded = mount_with_options(RecordingBackend::new(base.clone()), options).unwrap();
    assert_eq!(recorded.tree_cache_pages(), pages);
    {
        let volume = &mut recorded;
        volume
            .clone_range(source, 0, destination, 0, (4 * BS) as u64, ts(6))
            .unwrap();
    }
    let operations = recorded.into_device().into_parts().1;

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            assert_eq!(volume.read_file(source).unwrap(), source_bytes, "{context}");
            assert_eq!(
                volume.read_file(destination).unwrap().as_slice(),
                if post {
                    source_bytes.as_slice()
                } else {
                    old_destination.as_slice()
                },
                "{context}"
            );
            assert_eq!(
                volume.read_file(peer).unwrap(),
                source_bytes[BS..3 * BS],
                "{context}: peer"
            );
            let records = shared_records(volume);
            if post {
                assert_eq!(records.len(), 3, "{context}: {records:?}");
                assert_eq!(records[0].reference_count, 2, "{context}");
                assert_eq!(records[0].block_count, 1, "{context}");
                assert_eq!(records[1].reference_count, 3, "{context}");
                assert_eq!(records[1].block_count, 2, "{context}");
                assert_eq!(records[2].reference_count, 2, "{context}");
                assert_eq!(records[2].block_count, 1, "{context}");
            } else {
                assert_eq!(records, pre_records, "{context}");
            }
        },
    );
}

#[test]
fn shared_write_split_is_crash_atomic() {
    for pages in [2, 4, 8, usize::MAX] {
        shared_write_split_is_crash_atomic_profile(pages);
    }
}

fn shared_write_split_is_crash_atomic_profile(pages: usize) {
    let mut setup = profile_mount(formatted("CrashSharedWrite", 0), pages);
    let original = vec![0x22u8; 4 * BS];
    let source = setup
        .create_file_in_root("source", &original, ts(2))
        .unwrap();
    let clone = setup
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();
    let replacement = vec![0x77u8; BS];
    let operations = record_transaction(&base, pages, |volume| {
        volume
            .write_file_at(clone, BS as u64, &replacement, ts(4))
            .unwrap();
    });
    let mut changed = original.clone();
    changed[BS..2 * BS].copy_from_slice(&replacement);

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            assert_eq!(volume.read_file(source).unwrap(), original, "{context}");
            let expected = if post {
                changed.as_slice()
            } else {
                original.as_slice()
            };
            assert_eq!(
                volume.read_file(clone).unwrap().as_slice(),
                expected,
                "{context}"
            );
            let records = shared_records(volume);
            assert!(records.iter().all(|run| run.reference_count == 2));
            assert_eq!(
                records.iter().map(|run| run.block_count).sum::<u64>(),
                if post { 3 } else { 4 },
                "{context}: {records:?}"
            );
        },
    );
}

#[test]
fn unlink_at_count_three_is_crash_atomic() {
    for pages in [2, 4, 8, usize::MAX] {
        unlink_at_count_three_is_crash_atomic_profile(pages);
    }
}

fn unlink_at_count_three_is_crash_atomic_profile(pages: usize) {
    let mut setup = profile_mount(formatted("CrashUnlinkThree", 0), pages);
    let content = vec![0x33u8; BS];
    let source = setup
        .create_file_in_root("source", &content, ts(2))
        .unwrap();
    let first = setup
        .clone_file(source, OBJECT_ROOT, "first", ts(3))
        .unwrap();
    let second = setup
        .clone_file(source, OBJECT_ROOT, "second", ts(4))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();
    let operations = record_transaction(&base, pages, |volume| {
        volume.delete_file(OBJECT_ROOT, "source", ts(5)).unwrap();
    });

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            assert_eq!(
                volume.lookup_root("source").unwrap().is_none(),
                post,
                "{context}"
            );
            if !post {
                assert_eq!(volume.read_file(source).unwrap(), content, "{context}");
            }
            assert_eq!(volume.read_file(first).unwrap(), content, "{context}");
            assert_eq!(volume.read_file(second).unwrap(), content, "{context}");
            let records = shared_records(volume);
            assert_eq!(records.len(), 1, "{context}: {records:?}");
            assert_eq!(records[0].reference_count, if post { 2 } else { 3 });
        },
    );
}

#[test]
fn unlink_at_count_two_never_reclaims_the_survivor() {
    for pages in [2, 4, 8, usize::MAX] {
        unlink_at_count_two_never_reclaims_the_survivor_profile(pages);
    }
}

fn unlink_at_count_two_never_reclaims_the_survivor_profile(pages: usize) {
    let mut setup = profile_mount(formatted("CrashUnlinkTwo", 0), pages);
    let content = vec![0x44u8; 2 * BS];
    let source = setup
        .create_file_in_root("source", &content, ts(2))
        .unwrap();
    let clone = setup
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    let shared_start = shared_records(&mut setup)[0].physical_start;
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();
    let operations = record_transaction(&base, pages, |volume| {
        volume.delete_file(OBJECT_ROOT, "source", ts(4)).unwrap();
    });

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            assert_eq!(volume.read_file(clone).unwrap(), content, "{context}");
            if post {
                assert_eq!(volume.lookup_root("source").unwrap(), None, "{context}");
                assert!(shared_records(volume).is_empty(), "{context}");
                assert!(
                    !volume.quarantine_contains(shared_start).unwrap(),
                    "{context}: rc=2 -> rc=1 prematurely quarantined survivor data"
                );
            } else {
                assert_eq!(
                    volume.lookup_root("source").unwrap(),
                    Some(source),
                    "{context}"
                );
                assert_eq!(volume.read_file(source).unwrap(), content, "{context}");
                assert_eq!(shared_records(volume)[0].reference_count, 2, "{context}");
            }
        },
    );
}

#[test]
fn truncate_across_private_and_shared_subruns_is_crash_atomic() {
    for pages in [2, 4, 8, usize::MAX] {
        truncate_across_private_and_shared_subruns_is_crash_atomic_profile(pages);
    }
}

fn truncate_across_private_and_shared_subruns_is_crash_atomic_profile(pages: usize) {
    let mut setup = profile_mount(formatted("CrashSharedTruncate", 0), pages);
    let original = vec![0x55u8; 4 * BS];
    let source = setup
        .create_file_in_root("source", &original, ts(2))
        .unwrap();
    let clone = setup
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    setup
        .write_file_at(clone, BS as u64, &vec![0x99u8; BS], ts(4))
        .unwrap();
    let mut clone_bytes = original.clone();
    clone_bytes[BS..2 * BS].fill(0x99);
    assert_eq!(setup.read_file(clone).unwrap(), clone_bytes);
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();
    let operations = record_transaction(&base, pages, |volume| {
        volume.truncate_file(source, BS as u64, ts(5)).unwrap();
    });

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            assert_eq!(volume.read_file(clone).unwrap(), clone_bytes, "{context}");
            let expected = if post {
                &original[..BS]
            } else {
                original.as_slice()
            };
            assert_eq!(
                volume.read_file(source).unwrap().as_slice(),
                expected,
                "{context}"
            );
        },
    );
}

#[test]
fn rename_replace_of_a_shared_target_is_crash_atomic() {
    for pages in [2, 4, 8, usize::MAX] {
        rename_replace_of_a_shared_target_is_crash_atomic_profile(pages);
    }
}

fn rename_replace_of_a_shared_target_is_crash_atomic_profile(pages: usize) {
    let mut setup = profile_mount(formatted("CrashSharedReplace", 0), pages);
    let victim_bytes = vec![0x66u8; BS];
    let incoming_bytes = vec![0xaau8; BS];
    let victim = setup
        .create_file_in_root("victim", &victim_bytes, ts(2))
        .unwrap();
    let peer = setup
        .clone_file(victim, OBJECT_ROOT, "peer", ts(3))
        .unwrap();
    let incoming = setup
        .create_file_in_root("incoming", &incoming_bytes, ts(4))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();
    let operations = record_transaction(&base, pages, |volume| {
        volume
            .rename_replace(OBJECT_ROOT, "incoming", OBJECT_ROOT, "victim", ts(5))
            .unwrap();
    });

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            assert_eq!(volume.read_file(peer).unwrap(), victim_bytes, "{context}");
            if post {
                assert_eq!(
                    volume.lookup_root("victim").unwrap(),
                    Some(incoming),
                    "{context}"
                );
                assert_eq!(volume.lookup_root("incoming").unwrap(), None, "{context}");
                assert_eq!(
                    volume.read_file(incoming).unwrap(),
                    incoming_bytes,
                    "{context}"
                );
                assert!(volume.stat(victim).unwrap().is_none(), "{context}");
                assert!(shared_records(volume).is_empty(), "{context}");
            } else {
                assert_eq!(
                    volume.lookup_root("victim").unwrap(),
                    Some(victim),
                    "{context}"
                );
                assert_eq!(
                    volume.lookup_root("incoming").unwrap(),
                    Some(incoming),
                    "{context}"
                );
                assert_eq!(volume.read_file(victim).unwrap(), victim_bytes, "{context}");
                assert_eq!(
                    volume.read_file(incoming).unwrap(),
                    incoming_bytes,
                    "{context}"
                );
                assert_eq!(shared_records(volume)[0].reference_count, 2, "{context}");
            }
        },
    );
}

#[test]
fn durable_shared_unlink_replay_is_crash_atomic_and_idempotent() {
    for pages in [2, 4, 8, usize::MAX] {
        durable_shared_unlink_replay_is_crash_atomic_and_idempotent_profile(pages);
    }
}

fn durable_shared_unlink_replay_is_crash_atomic_and_idempotent_profile(pages: usize) {
    let mut setup = profile_mount(formatted("ReplaySharedUnlink", 8), pages);
    let content = vec![0xbbu8; 2 * BS];
    let source = setup
        .create_file_in_root("source", &content, ts(2))
        .unwrap();
    let survivor = setup
        .clone_file(source, OBJECT_ROOT, "survivor", ts(3))
        .unwrap();
    let shared_start = shared_records(&mut setup)[0].physical_start;
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();

    let logged = {
        let mut volume = profile_mount(base, pages);
        volume
            .window_op(
                &BatchOp::DeleteFile {
                    parent_id: OBJECT_ROOT,
                    name: "source",
                },
                ts(4),
            )
            .unwrap();
        volume.window_fsync().unwrap();
        volume.into_device()
    };
    let operations = record_replay(&logged, pages);

    run_replay_matrix(
        &logged,
        &operations,
        pre_generation,
        pages,
        |context, volume| {
            assert_eq!(volume.lookup_root("source").unwrap(), None, "{context}");
            assert_eq!(volume.read_file(survivor).unwrap(), content, "{context}");
            assert!(volume.orphan_object(source).unwrap(), "{context}");
            assert_eq!(volume.read_file(source).unwrap(), content, "{context}");
            assert_eq!(shared_records(volume)[0].reference_count, 2, "{context}");
            assert!(
                !volume.quarantine_contains(shared_start).unwrap(),
                "{context}: replay of rc=2 -> rc=1 reclaimed survivor data"
            );
        },
    );
}

#[test]
fn durable_rename_replace_replay_preserves_the_shared_survivor() {
    for pages in [2, 4, 8, usize::MAX] {
        durable_rename_replace_replay_preserves_the_shared_survivor_profile(pages);
    }
}

fn durable_rename_replace_replay_preserves_the_shared_survivor_profile(pages: usize) {
    let mut setup = profile_mount(formatted("ReplaySharedReplace", 8), pages);
    let old_bytes = vec![0xccu8; BS];
    let new_bytes = vec![0xddu8; BS];
    let target = setup
        .create_file_in_root("target", &old_bytes, ts(2))
        .unwrap();
    let survivor = setup
        .clone_file(target, OBJECT_ROOT, "survivor", ts(3))
        .unwrap();
    let incoming = setup
        .create_file_in_root("incoming", &new_bytes, ts(4))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();

    let logged = {
        let mut volume = profile_mount(base, pages);
        volume
            .window_op(
                &BatchOp::Rename {
                    source_parent_id: OBJECT_ROOT,
                    source_name: "incoming",
                    target_parent_id: OBJECT_ROOT,
                    target_name: "target",
                    replace: true,
                },
                ts(5),
            )
            .unwrap();
        volume.window_fsync().unwrap();
        volume.into_device()
    };
    let operations = record_replay(&logged, pages);

    run_replay_matrix(
        &logged,
        &operations,
        pre_generation,
        pages,
        |context, volume| {
            assert_eq!(
                volume.lookup_root("target").unwrap(),
                Some(incoming),
                "{context}"
            );
            assert_eq!(volume.lookup_root("incoming").unwrap(), None, "{context}");
            assert!(volume.orphan_object(target).unwrap(), "{context}");
            assert_eq!(volume.read_file(target).unwrap(), old_bytes, "{context}");
            assert_eq!(volume.read_file(incoming).unwrap(), new_bytes, "{context}");
            assert_eq!(volume.read_file(survivor).unwrap(), old_bytes, "{context}");
            assert_eq!(shared_records(volume)[0].reference_count, 2, "{context}");
        },
    );
}

#[test]
fn shared_storage_is_reused_only_after_the_last_owner_disappears() {
    for pages in [2, 4, 8, usize::MAX] {
        shared_storage_is_reused_only_after_the_last_owner_disappears_profile(pages);
    }
}

fn shared_storage_is_reused_only_after_the_last_owner_disappears_profile(pages: usize) {
    let mut setup = profile_mount(formatted("CrashSharedReuse", 0), pages);
    let old_bytes = vec![0xeeu8; BS];
    let source = setup
        .create_file_in_root("source", &old_bytes, ts(2))
        .unwrap();
    let survivor = setup
        .clone_file(source, OBJECT_ROOT, "survivor", ts(3))
        .unwrap();
    let shared_start = shared_records(&mut setup)[0].physical_start;

    setup.delete_file(OBJECT_ROOT, "source", ts(4)).unwrap();
    assert_eq!(setup.read_file(survivor).unwrap(), old_bytes);
    assert!(shared_records(&mut setup).is_empty());
    assert!(
        !setup.quarantine_contains(shared_start).unwrap(),
        "the rc=2 -> rc=1 transition must leave survivor storage allocated"
    );

    setup.delete_file(OBJECT_ROOT, "survivor", ts(5)).unwrap();
    assert!(
        setup.quarantine_contains(shared_start).unwrap(),
        "the last owner must retire the formerly shared storage"
    );
    let retirement_generation = setup.generation();
    let mut base = setup.into_device();
    let mut reuse_case = None;
    for index in 0..32 {
        let name = format!("replacement-{index}");
        let new_bytes = vec![0xf1u8.wrapping_add(index as u8); BS];
        let pre_generation = profile_mount(base.clone(), pages).generation();
        let mut replacement = 0;
        let mut reused = false;
        let operations = record_transaction(&base, pages, |volume| {
            replacement = volume
                .create_file_in_root(&name, &new_bytes, ts(6 + index))
                .unwrap();
            let ident = volume.ident().clone();
            let cp = volume.checkpoint().clone();
            let state = load_committed_state(volume.device_mut(), &ident, &cp).unwrap();
            reused = state.data_blocks.contains(&shared_start)
                || state.metadata_blocks.contains(&shared_start);
        });
        if reused {
            assert!(
                pre_generation > retirement_generation,
                "reuse crossed the protected slot boundary early"
            );
            reuse_case = Some((
                base.clone(),
                pre_generation,
                operations,
                name,
                new_bytes,
                replacement,
            ));
            break;
        }

        let mut advance = profile_mount(base, pages);
        advance
            .create_file_in_root(&name, &new_bytes, ts(6 + index))
            .unwrap();
        base = advance.into_device();
    }
    let (base, pre_generation, operations, replacement_name, new_bytes, replacement) = reuse_case
        .expect("allocator did not reach the formerly shared block within 32 transactions");

    run_checkpoint_matrix_profile(
        &base,
        &operations,
        pre_generation,
        pages,
        |context, post, volume| {
            if post {
                assert_eq!(
                    volume.lookup_root(&replacement_name).unwrap(),
                    Some(replacement),
                    "{context}"
                );
                assert_eq!(
                    volume.read_file(replacement).unwrap(),
                    new_bytes,
                    "{context}"
                );
                assert!(
                    !volume.quarantine_contains(shared_start).unwrap(),
                    "{context}: reused block remains quarantined"
                );
            } else {
                assert_eq!(
                    volume.lookup_root(&replacement_name).unwrap(),
                    None,
                    "{context}"
                );
                assert!(
                    volume.quarantine_contains(shared_start).unwrap(),
                    "{context}: pre-state lost the retired shared block"
                );
            }
        },
    );
}

#[test]
fn logged_write_replay_splits_shared_data_and_survives_replay_crashes() {
    for pages in [2, 4, 8, usize::MAX] {
        logged_write_replay_splits_shared_data_and_survives_replay_crashes_profile(pages);
    }
}

fn logged_write_replay_splits_shared_data_and_survives_replay_crashes_profile(pages: usize) {
    let old = vec![0x41u8; 3 * BS];
    let patch = vec![0xD2u8; BS];
    let mut setup = profile_mount(formatted("LoggedSharedWrite", 8), pages);
    let source = setup.create_file_in_root("source", &old, ts(2)).unwrap();
    let clone = setup
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = profile_mount(base.clone(), pages).generation();

    let mut logger = profile_mount(base, pages);
    logger
        .window_write_file_at(source, BS as u64, &patch, ts(4))
        .unwrap();
    logger.window_fsync().unwrap();
    let logged = logger.into_device();
    let replay = record_replay(&logged, pages);

    let mut expected_source = old.clone();
    expected_source[BS..2 * BS].copy_from_slice(&patch);
    run_replay_matrix(
        &logged,
        &replay,
        pre_generation,
        pages,
        |context, volume| {
            assert_eq!(
                volume.read_file(source).unwrap(),
                expected_source,
                "{context}"
            );
            assert_eq!(volume.read_file(clone).unwrap(), old, "{context}");
            let records = shared_records(volume);
            assert_eq!(records.len(), 2, "{context}: {records:?}");
            assert!(
                records.iter().all(|record| record.reference_count == 2),
                "{context}"
            );
        },
    );
}

#[test]
fn first_clone_io_failures_preserve_ownership_in_all_profiles() {
    for pages in [2, 4, 8, usize::MAX] {
        let options = MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        };
        let mut setup = mount_with_options(formatted("CloneFaults", 0), options).unwrap();
        let content = vec![0x51; 2 * BS];
        let source = setup
            .create_file_in_root("source", &content, ts(2))
            .unwrap();
        let before = setup.generation();
        let base = setup.into_device();
        let mut reference =
            mount_with_options(RecordingBackend::new(base.clone()), options).unwrap();
        reference
            .clone_file(source, OBJECT_ROOT, "clone", ts(3))
            .unwrap();
        let (_, log) = reference.into_device().into_parts();
        let writes = log
            .iter()
            .filter(|op| matches!(op, RecordedOp::Write { .. }))
            .count() as u64;
        let flushes = log
            .iter()
            .filter(|op| matches!(op, RecordedOp::Flush))
            .count() as u64;
        assert!(writes > 0 && flushes > 0);
        let plans = (0..writes)
            .map(|i| FaultPlan {
                fail_write_index: Some(i),
                ..Default::default()
            })
            .chain((0..flushes).map(|i| FaultPlan {
                fail_flush_index: Some(i),
                ..Default::default()
            }));
        for plan in plans {
            let context = format!("pages={pages} plan={plan:?}");
            let mut volume =
                mount_with_options(FaultBackend::new(base.clone(), plan), options).unwrap();
            assert!(
                volume
                    .clone_file(source, OBJECT_ROOT, "clone", ts(3))
                    .is_err(),
                "{context}"
            );
            assert!(volume.device_mut().tripped(), "{context}");
            if plan.fail_flush_index == Some(flushes - 1) {
                assert!(
                    matches!(
                        volume.clone_file(source, OBJECT_ROOT, "clone", ts(3)),
                        Err(afsplus_core::CoreError::WindowPoisoned)
                    ),
                    "{context}"
                );
            } else {
                assert_eq!(volume.read_file(source).unwrap(), content, "{context}");
                assert_eq!(volume.lookup_root("clone").unwrap(), None, "{context}");
                assert_eq!(volume.generation(), before, "{context}");
            }
            let mut image = volume.into_device().into_inner();
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let mut recovered = mount_with_options(image, options).unwrap();
            assert_eq!(recovered.tree_cache_pages(), pages);
            assert_eq!(recovered.read_file(source).unwrap(), content, "{context}");
            if recovered.generation() == before {
                assert_eq!(recovered.lookup_root("clone").unwrap(), None, "{context}");
                assert!(shared_records(&mut recovered).is_empty(), "{context}");
                recovered
                    .clone_file(source, OBJECT_ROOT, "clone", ts(3))
                    .unwrap();
            }
            assert_eq!(recovered.generation(), before + 1, "{context}");
            let clone = recovered.lookup_root("clone").unwrap().unwrap();
            assert_eq!(recovered.read_file(clone).unwrap(), content, "{context}");
            assert_eq!(recovered.read_file(source).unwrap(), content, "{context}");
            let records = shared_records(&mut recovered);
            assert_eq!(records.len(), 1, "{context}");
            assert_eq!(
                (records[0].block_count, records[0].reference_count),
                (2, 2),
                "{context}"
            );
            let report = check_device(recovered.device_mut());
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
        }
        eprintln!("first-clone faults pages={pages} writes={writes} flushes={flushes}");
    }
}

#[derive(Clone, Copy, Debug)]
enum SharedFaultFamily {
    Write,
    Replace,
}

struct SharedFaultFixture {
    source: u64,
    peer: u64,
    incoming: u64,
    generation: u64,
    original: Vec<u8>,
    incoming_bytes: Vec<u8>,
    shared: SharedRun,
}

impl SharedFaultFixture {
    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        family: SharedFaultFamily,
    ) -> Result<(), afsplus_core::CoreError> {
        match family {
            SharedFaultFamily::Write => {
                volume.write_file_at(self.peer, BS as u64, &vec![0x97; BS], ts(5))
            }
            SharedFaultFamily::Replace => {
                volume.rename_replace(OBJECT_ROOT, "incoming", OBJECT_ROOT, "source", ts(5))
            }
        }
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        family: SharedFaultFamily,
        post: bool,
        context: &str,
    ) {
        let replaced = post && matches!(family, SharedFaultFamily::Replace);
        assert_eq!(
            volume.generation(),
            self.generation + u64::from(post),
            "{context}"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            if replaced { 2 } else { 3 },
            "{context}"
        );
        assert_eq!(
            volume.lookup_root("source").unwrap(),
            Some(if replaced { self.incoming } else { self.source }),
            "{context}"
        );
        assert_eq!(
            volume.lookup_root("peer").unwrap(),
            Some(self.peer),
            "{context}"
        );
        assert_eq!(
            volume.lookup_root("incoming").unwrap(),
            (!replaced).then_some(self.incoming),
            "{context}"
        );
        assert_eq!(
            volume.read_file(self.incoming).unwrap(),
            self.incoming_bytes,
            "{context}"
        );
        if replaced {
            assert!(volume.stat(self.source).unwrap().is_none(), "{context}");
        } else {
            assert_eq!(
                volume.read_file(self.source).unwrap(),
                self.original,
                "{context}"
            );
        }
        let mut peer_bytes = self.original.clone();
        let expected_records = if post && matches!(family, SharedFaultFamily::Write) {
            peer_bytes[BS..2 * BS].fill(0x97);
            vec![
                SharedRun {
                    block_count: 1,
                    ..self.shared
                },
                SharedRun {
                    physical_start: self.shared.physical_start + 2,
                    block_count: 2,
                    ..self.shared
                },
            ]
        } else if replaced {
            Vec::new()
        } else {
            vec![self.shared]
        };
        assert_eq!(
            volume.read_file(self.peer).unwrap(),
            peer_bytes,
            "{context}"
        );
        assert_eq!(shared_records(volume), expected_records, "{context}");
        for offset in 0..self.shared.block_count {
            assert!(
                !volume
                    .quarantine_contains(self.shared.physical_start + offset)
                    .unwrap(),
                "{context}: reachable shared storage quarantined"
            );
        }
    }
}

fn shared_fault_fixture(pages: usize) -> (SharedFaultFixture, MemoryBackend) {
    let mut setup = profile_mount(formatted("SharedMutationFaults", 0), pages);
    let original = vec![0x36; 4 * BS];
    let incoming_bytes = vec![0x71; BS + 29];
    let source = setup
        .create_file_in_root("source", &original, ts(2))
        .unwrap();
    let peer = setup
        .clone_file(source, OBJECT_ROOT, "peer", ts(3))
        .unwrap();
    let incoming = setup
        .create_file_in_root("incoming", &incoming_bytes, ts(4))
        .unwrap();
    let records = shared_records(&mut setup);
    assert_eq!(records.len(), 1);
    assert_eq!(
        (
            records[0].block_count,
            records[0].reference_count,
            records[0].flags
        ),
        (4, 2, 0)
    );
    let fixture = SharedFaultFixture {
        source,
        peer,
        incoming,
        generation: setup.generation(),
        original,
        incoming_bytes,
        shared: records[0],
    };
    (fixture, setup.into_device())
}

fn shared_mutation_io_failures(family: SharedFaultFamily) {
    for pages in [2, 4, 8, usize::MAX] {
        let (fixture, base) = shared_fault_fixture(pages);
        fixture.verify(
            &mut profile_mount(base.clone(), pages),
            family,
            false,
            "initial fixture",
        );
        let mut reference = profile_mount(RecordingBackend::new(base.clone()), pages);
        fixture.apply(&mut reference, family).unwrap();
        fixture.verify(&mut reference, family, true, "successful reference");
        let staged = reference.last_commit_stats().unwrap().tree_mutations;
        assert!(
            staged.max_resident_staged_nodes <= pages as u64,
            "{family:?} pages={pages}: resident staged nodes exceed the profile"
        );
        let checkpoint_slots = reference.ident().checkpoint_slots;
        let (_, log) = reference.into_device().into_parts();
        let write_targets: Vec<_> = log
            .iter()
            .filter_map(|op| match op {
                RecordedOp::Write { lba, .. } => Some(*lba),
                _ => None,
            })
            .collect();
        assert!(
            checkpoint_slots.contains(write_targets.last().unwrap()),
            "{family:?} pages={pages}: final write must publish a checkpoint"
        );
        assert_eq!(
            write_targets
                .iter()
                .filter(|lba| checkpoint_slots.contains(lba))
                .count(),
            1,
            "{family:?} pages={pages}: fixture must have exactly one checkpoint write"
        );
        let writes = log
            .iter()
            .filter(|op| matches!(op, RecordedOp::Write { .. }))
            .count() as u64;
        let flushes = log
            .iter()
            .filter(|op| matches!(op, RecordedOp::Flush))
            .count() as u64;
        assert!(writes > 0 && flushes > 0);
        let plans = (0..writes)
            .map(|i| FaultPlan {
                fail_write_index: Some(i),
                ..Default::default()
            })
            .chain((0..flushes).map(|i| FaultPlan {
                fail_flush_index: Some(i),
                ..Default::default()
            }));
        let mut same_handle_cases = 0;
        for plan in plans {
            let context = format!("{family:?} pages={pages} plan={plan:?}");
            let mut volume = profile_mount(FaultBackend::new(base.clone(), plan), pages);
            assert!(fixture.apply(&mut volume, family).is_err(), "{context}");
            assert!(volume.device_mut().tripped(), "{context}");
            let published = plan.fail_flush_index == Some(flushes - 1);
            if !published {
                fixture.verify(&mut volume, family, false, &context);
            }
            // The checkpoint is the last recorded write. Its failed write or
            // barrier leaves publication uncertain to the mounted writer.
            if published || plan.fail_write_index == Some(writes - 1) {
                assert!(
                    matches!(
                        fixture.apply(&mut volume, family),
                        Err(afsplus_core::CoreError::WindowPoisoned)
                    ),
                    "{context}"
                );
            }
            // Inspect before any retry: before-write failures retain old state;
            // a failed final barrier follows a complete readable checkpoint.
            let mut recovered = profile_mount(volume.into_device().into_inner(), pages);
            fixture.verify(&mut recovered, family, published, &context);
            assert_checker_clean(recovered.device_mut(), &context);
            if !published {
                fixture.apply(&mut recovered, family).unwrap();
            }
            fixture.verify(&mut recovered, family, true, &context);
            let mut again = profile_mount(recovered.into_device(), pages);
            fixture.verify(&mut again, family, true, &context);
            assert_checker_clean(again.device_mut(), &context);

            if !published && plan.fail_write_index != Some(writes - 1) {
                // A separate failure instance preserves the untouched pre-retry
                // remount oracle above while qualifying retry on the same handle.
                let mut retry = profile_mount(FaultBackend::new(base.clone(), plan), pages);
                assert!(
                    fixture.apply(&mut retry, family).is_err(),
                    "{context}: retry instance"
                );
                assert!(retry.device_mut().tripped(), "{context}: retry instance");
                fixture.verify(&mut retry, family, false, &context);
                fixture
                    .apply(&mut retry, family)
                    .unwrap_or_else(|error| panic!("{context}: same-handle retry failed: {error}"));
                fixture.verify(&mut retry, family, true, &context);
                let mut remounted = profile_mount(retry.into_device().into_inner(), pages);
                fixture.verify(&mut remounted, family, true, &context);
                assert_checker_clean(remounted.device_mut(), &context);
                same_handle_cases += 1;
            }
        }
        assert_eq!(same_handle_cases, writes + flushes - 2);
        eprintln!(
            "shared mutation faults family={family:?} pages={pages} writes={writes} flushes={flushes} independent_same_handle_cases={same_handle_cases} spills={} peak_staged={}",
            staged.staged_spill_writes, staged.max_resident_staged_nodes
        );
    }
}

#[test]
fn shared_write_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles() {
    shared_mutation_io_failures(SharedFaultFamily::Write);
}

#[test]
fn shared_replace_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles() {
    shared_mutation_io_failures(SharedFaultFamily::Replace);
}

/// Matches the completed-write and adoption-read fault model in faults.rs.
struct SharedAmbiguousDevice {
    inner: MemoryBackend,
    checkpoints: [u64; 2],
    published: bool,
    fail_write: bool,
    tripped: bool,
    writes: u64,
    flushes: u64,
}

impl BlockDevice for SharedAmbiguousDevice {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), afsplus_block::BlockError> {
        if self.published && !self.fail_write {
            self.tripped = true;
            return Err(afsplus_block::BlockError::Injected("post-publication read"));
        }
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), afsplus_block::BlockError> {
        self.writes += 1;
        self.inner.write_block(lba, data)?;
        if self.checkpoints.contains(&lba) {
            self.published = true;
            if self.fail_write {
                self.tripped = true;
                return Err(afsplus_block::BlockError::Injected(
                    "completed checkpoint write",
                ));
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), afsplus_block::BlockError> {
        self.flushes += 1;
        self.inner.flush()
    }
}

fn shared_ambiguous_publication(family: SharedFaultFamily) {
    for pages in [2, 4, 8, usize::MAX] {
        for fail_write in [true, false] {
            let context = format!("{family:?} pages={pages} completed_write={fail_write}");
            let (mut fixture, base) = shared_fault_fixture(pages);
            let checkpoints = Identification::decode(&base.peek(0))
                .unwrap()
                .checkpoint_slots;
            let mut volume = profile_mount(
                SharedAmbiguousDevice {
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
            assert!(fixture.apply(&mut volume, family).is_err(), "{context}");
            assert!(
                volume.device_mut().published && volume.device_mut().tripped,
                "{context}"
            );
            let io = (volume.device_mut().writes, volume.device_mut().flushes);
            assert!(
                matches!(
                    fixture.apply(&mut volume, family),
                    Err(afsplus_core::CoreError::WindowPoisoned)
                ),
                "{context}: same operation must refuse"
            );
            assert!(
                matches!(
                    volume.create_file_in_root("blocked", b"must not appear", ts(6)),
                    Err(afsplus_core::CoreError::WindowPoisoned)
                ),
                "{context}: independent mutation must refuse"
            );
            assert_eq!(
                (volume.device_mut().writes, volume.device_mut().flushes),
                io,
                "{context}: poisoned mutations performed writes or flushes"
            );
            let mut recovered = profile_mount(volume.into_device().inner, pages);
            fixture.verify(&mut recovered, family, true, &context);
            assert_eq!(recovered.lookup_root("blocked").unwrap(), None, "{context}");
            assert_checker_clean(recovered.device_mut(), &context);
            // Mutate an independent private file after reconciliation, allocating
            // fresh data while every shared source/survivor remains exact.
            recovered
                .write_file_at(fixture.incoming, 17, &[0xa4; 93], ts(7))
                .unwrap();
            fixture.incoming_bytes[17..110].fill(0xa4);
            fixture.generation += 1;
            fixture.verify(&mut recovered, family, true, &context);
            let mut again = profile_mount(recovered.into_device(), pages);
            fixture.verify(&mut again, family, true, &context);
            assert_eq!(again.lookup_root("blocked").unwrap(), None, "{context}");
            assert_checker_clean(again.device_mut(), &context);
            eprintln!("ambiguous shared publication {context}");
        }
    }
}

#[test]
fn shared_write_ambiguous_publication_requires_remount_in_all_profiles() {
    shared_ambiguous_publication(SharedFaultFamily::Write);
}

#[test]
fn shared_replace_ambiguous_publication_requires_remount_in_all_profiles() {
    shared_ambiguous_publication(SharedFaultFamily::Replace);
}
