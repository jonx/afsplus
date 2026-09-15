//! Crash-atomicity qualification for ADR-061 shared extents.
//!
//! Each checkpoint transaction is recorded at the block-device boundary and
//! cut after every write/flush.  Every modeled image must select exactly the
//! pre- or post-generation, pass the exhaustive checker, preserve contents,
//! and keep reference records synchronized with the selected namespace.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use afsplus_block::{for_each_crash_state, MemoryBackend, RecordedOp, RecordingBackend};
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

fn shared_records(volume: &mut Volume<MemoryBackend>) -> Vec<SharedRun> {
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

fn run_checkpoint_matrix(
    base: &MemoryBackend,
    operations: &[RecordedOp],
    pre_generation: u64,
    verify: impl FnMut(&str, bool, &mut Volume<MemoryBackend>),
) {
    run_checkpoint_matrix_profile(base, operations, pre_generation, usize::MAX, verify);
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
            let context = state.description;
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
}

fn record_transaction(
    base: &MemoryBackend,
    operation: impl FnOnce(&mut Volume<RecordingBackend<MemoryBackend>>),
) -> Vec<RecordedOp> {
    let mut volume = mount(RecordingBackend::new(base.clone())).unwrap();
    operation(&mut volume);
    volume.into_device().into_parts().1
}

/// Records the automatic recovery transaction produced when a durable intent
/// record is replayed at mount.  The returned base already contains the
/// durable record but still exposes the old checkpoint in NoChanges mode.
fn record_replay(logged: &MemoryBackend) -> Vec<RecordedOp> {
    let volume = mount(RecordingBackend::new(logged.clone())).unwrap();
    volume.into_device().into_parts().1
}

fn run_replay_matrix(
    logged: &MemoryBackend,
    operations: &[RecordedOp],
    pre_generation: u64,
    mut verify_recovered: impl FnMut(&str, &mut Volume<MemoryBackend>),
) {
    assert_cow_targets(logged, operations);
    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    for crash_point in 0..=operations.len() {
        for_each_crash_state(logged, operations, crash_point, |state| {
            let context = state.description;
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
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{context}: no-changes mount failed: {error}"));
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
            let mut recovered = mount(state.image)
                .unwrap_or_else(|error| panic!("{context}: recovery mount failed: {error}"));
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
        });
    }
    assert!(pre_outcomes > 0, "replay matrix produced no pre-state");
    assert!(post_outcomes > 0, "replay matrix produced no post-state");
}

#[test]
fn first_clone_is_crash_atomic() {
    let mut setup = mount(formatted("CrashFirstClone", 0)).unwrap();
    let content = vec![0x11u8; 2 * BS];
    let source = setup
        .create_file_in_root("source", &content, ts(2))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let mut clone_id = 0;
    let operations = record_transaction(&base, |volume| {
        clone_id = volume
            .clone_file(source, OBJECT_ROOT, "clone", ts(3))
            .unwrap();
    });

    run_checkpoint_matrix(
        &base,
        &operations,
        pre_generation,
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
    let mut setup = mount(formatted("CrashSharedWrite", 0)).unwrap();
    let original = vec![0x22u8; 4 * BS];
    let source = setup
        .create_file_in_root("source", &original, ts(2))
        .unwrap();
    let clone = setup
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let replacement = vec![0x77u8; BS];
    let operations = record_transaction(&base, |volume| {
        volume
            .write_file_at(clone, BS as u64, &replacement, ts(4))
            .unwrap();
    });
    let mut changed = original.clone();
    changed[BS..2 * BS].copy_from_slice(&replacement);

    run_checkpoint_matrix(
        &base,
        &operations,
        pre_generation,
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
    let mut setup = mount(formatted("CrashUnlinkThree", 0)).unwrap();
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
    let pre_generation = mount(base.clone()).unwrap().generation();
    let operations = record_transaction(&base, |volume| {
        volume.delete_file(OBJECT_ROOT, "source", ts(5)).unwrap();
    });

    run_checkpoint_matrix(
        &base,
        &operations,
        pre_generation,
        |context, post, volume| {
            assert_eq!(
                volume.lookup_root("source").unwrap().is_none(),
                post,
                "{context}"
            );
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
    let mut setup = mount(formatted("CrashUnlinkTwo", 0)).unwrap();
    let content = vec![0x44u8; 2 * BS];
    let source = setup
        .create_file_in_root("source", &content, ts(2))
        .unwrap();
    let clone = setup
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    let shared_start = shared_records(&mut setup)[0].physical_start;
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let operations = record_transaction(&base, |volume| {
        volume.delete_file(OBJECT_ROOT, "source", ts(4)).unwrap();
    });

    run_checkpoint_matrix(
        &base,
        &operations,
        pre_generation,
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
                assert_eq!(shared_records(volume)[0].reference_count, 2, "{context}");
            }
        },
    );
}

#[test]
fn truncate_across_private_and_shared_subruns_is_crash_atomic() {
    let mut setup = mount(formatted("CrashSharedTruncate", 0)).unwrap();
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
    let clone_bytes = setup.read_file(clone).unwrap();
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let operations = record_transaction(&base, |volume| {
        volume.truncate_file(source, BS as u64, ts(5)).unwrap();
    });

    run_checkpoint_matrix(
        &base,
        &operations,
        pre_generation,
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
    let mut setup = mount(formatted("CrashSharedReplace", 0)).unwrap();
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
    let pre_generation = mount(base.clone()).unwrap().generation();
    let operations = record_transaction(&base, |volume| {
        volume
            .rename_replace(OBJECT_ROOT, "incoming", OBJECT_ROOT, "victim", ts(5))
            .unwrap();
    });

    run_checkpoint_matrix(
        &base,
        &operations,
        pre_generation,
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
                assert_eq!(shared_records(volume)[0].reference_count, 2, "{context}");
            }
        },
    );
}

#[test]
fn durable_shared_unlink_replay_is_crash_atomic_and_idempotent() {
    let mut setup = mount(formatted("ReplaySharedUnlink", 8)).unwrap();
    let content = vec![0xbbu8; 2 * BS];
    let source = setup
        .create_file_in_root("source", &content, ts(2))
        .unwrap();
    let survivor = setup
        .clone_file(source, OBJECT_ROOT, "survivor", ts(3))
        .unwrap();
    let shared_start = shared_records(&mut setup)[0].physical_start;
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();

    let logged = {
        let mut volume = mount(base).unwrap();
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
    let operations = record_replay(&logged);

    run_replay_matrix(&logged, &operations, pre_generation, |context, volume| {
        assert_eq!(volume.lookup_root("source").unwrap(), None, "{context}");
        assert_eq!(volume.read_file(survivor).unwrap(), content, "{context}");
        assert!(volume.orphan_object(source).unwrap(), "{context}");
        assert_eq!(shared_records(volume)[0].reference_count, 2, "{context}");
        assert!(
            !volume.quarantine_contains(shared_start).unwrap(),
            "{context}: replay of rc=2 -> rc=1 reclaimed survivor data"
        );
    });
}

#[test]
fn durable_rename_replace_replay_preserves_the_shared_survivor() {
    let mut setup = mount(formatted("ReplaySharedReplace", 8)).unwrap();
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
    let pre_generation = mount(base.clone()).unwrap().generation();

    let logged = {
        let mut volume = mount(base).unwrap();
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
    let operations = record_replay(&logged);

    run_replay_matrix(&logged, &operations, pre_generation, |context, volume| {
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
    });
}

#[test]
fn shared_storage_is_reused_only_after_the_last_owner_disappears() {
    let mut setup = mount(formatted("CrashSharedReuse", 0)).unwrap();
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
        let pre_generation = mount(base.clone()).unwrap().generation();
        let mut replacement = 0;
        let mut reused = false;
        let operations = record_transaction(&base, |volume| {
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

        let mut advance = mount(base).unwrap();
        advance
            .create_file_in_root(&name, &new_bytes, ts(6 + index))
            .unwrap();
        base = advance.into_device();
    }
    let (base, pre_generation, operations, replacement_name, new_bytes, replacement) = reuse_case
        .expect("allocator did not reach the formerly shared block within 32 transactions");

    run_checkpoint_matrix(
        &base,
        &operations,
        pre_generation,
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
    let old = vec![0x41u8; 3 * BS];
    let patch = vec![0xD2u8; BS];
    let mut setup = mount(formatted("LoggedSharedWrite", 8)).unwrap();
    let source = setup.create_file_in_root("source", &old, ts(2)).unwrap();
    let clone = setup
        .clone_file(source, OBJECT_ROOT, "clone", ts(3))
        .unwrap();
    let base = setup.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut logger = mount(base).unwrap();
    logger
        .window_write_file_at(source, BS as u64, &patch, ts(4))
        .unwrap();
    logger.window_fsync().unwrap();
    let logged = logger.into_device();
    let replay = record_replay(&logged);

    let mut expected_source = old.clone();
    expected_source[BS..2 * BS].copy_from_slice(&patch);
    run_replay_matrix(&logged, &replay, pre_generation, |context, volume| {
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
    });
}
