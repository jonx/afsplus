use super::*;
use crate::mount::select_checkpoint;
use crate::verify::load_mount_state;
use crate::{MkfsParams, NamePolicy};
use afsplus_block::{for_each_crash_state, MemoryBackend, RecordingBackend};

fn now(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 123,
    }
}
fn limits() -> SnapshotWorkLimits {
    SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 128,
        reclaim_records: 8,
    }
}

// Deliberately test-only until namespace/checker/resource qualification permits
// enabling the feature in normal mount negotiation.
fn open<D: BlockDevice>(mut dev: D, mode: MountMode) -> Volume<D> {
    let mut buf = vec![0; 4096];
    dev.read_block(0, &mut buf).unwrap();
    let ident = Identification::decode(&buf).unwrap();
    let selection = select_checkpoint(&mut dev, &ident).unwrap();
    let state = load_mount_state(&mut dev, &ident, &selection.chosen).unwrap();
    let mut volume = Volume::new(dev, ident, selection, state, mode);
    volume.set_snapshot_work_limits(limits()).unwrap();
    if mode == MountMode::ReadWrite || mode == MountMode::Recovery {
        volume.recover_intent_log().unwrap();
    } else {
        volume.inspect_intent_log().unwrap();
    }
    volume
}

fn formatted(blocks: u64, log_slots: u16) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, blocks);
    crate::mkfs::mkfs_snapshots(
        &mut dev,
        &MkfsParams {
            uuid: [73; 16],
            label: "SnapshotVolume".into(),
            region_size: blocks as u32,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: now(1),
        },
    )
    .unwrap();
    dev
}

fn bytes<D: BlockDevice>(volume: &mut Volume<D>, handle: &SnapshotHandle, id: u64) -> Vec<u8> {
    let len = volume
        .snapshot_stat(handle, id)
        .unwrap()
        .unwrap()
        .size_bytes as usize;
    let mut out = vec![0x5a; len];
    // Cross block and caller-buffer boundaries with a non-power-of-two chunk.
    let mut offset = 0;
    while offset < len {
        let end = (offset + 3001).min(len);
        let got = volume
            .snapshot_read_file_at(handle, id, offset as u64, &mut out[offset..end])
            .unwrap();
        assert_eq!(got, end - offset);
        offset = end;
    }
    out
}

#[test]
fn snapshot_namespace_handles_and_cursors_survive_live_mutations_and_remount() {
    let mut volume = open(formatted(2048, 0), MountMode::ReadWrite);
    let original: Vec<_> = (0..12000).map(|i| (i * 17) as u8).collect();
    let alpha = volume
        .create_file_in_root("alpha", &original, now(2))
        .unwrap();
    let beta = volume
        .create_file_in_root("beta", b"second", now(3))
        .unwrap();
    let id = volume.snapshot_create(now(4)).unwrap();
    let handle = volume.snapshot_open(id).unwrap();
    let metadata = volume.snapshot_stat(&handle, alpha).unwrap().unwrap();
    let first = volume
        .snapshot_read_directory_page(&handle, OBJECT_ROOT, None, 1)
        .unwrap();
    assert_eq!(first.entries[0].name, b"alpha");
    assert!(!first.eof);
    volume.write_file_at(alpha, 0, b"changed", now(5)).unwrap();
    volume
        .rename(OBJECT_ROOT, "beta", OBJECT_ROOT, "renamed", now(6))
        .unwrap();
    volume
        .create_file_in_root("later", b"later", now(7))
        .unwrap();
    assert_eq!(bytes(&mut volume, &handle, alpha), original);
    assert_eq!(
        volume.snapshot_stat(&handle, alpha).unwrap().unwrap(),
        metadata
    );
    assert_eq!(
        volume
            .snapshot_lookup(&handle, OBJECT_ROOT, "beta")
            .unwrap(),
        Some(beta)
    );
    assert_eq!(
        volume
            .snapshot_lookup(&handle, OBJECT_ROOT, "renamed")
            .unwrap(),
        None
    );
    let second = volume
        .snapshot_read_directory_page(&handle, OBJECT_ROOT, Some(first.next), 1)
        .unwrap();
    assert_eq!(second.entries[0].name, b"beta");
    assert!(second.eof);
    let second_id = volume.snapshot_create(now(8)).unwrap();
    let second_handle = volume.snapshot_open(second_id).unwrap();
    assert!(matches!(
        volume.snapshot_read_directory_page(&second_handle, OBJECT_ROOT, Some(first.next), 1),
        Err(CoreError::Stale)
    ));
    let duplicate = handle.clone();
    assert!(matches!(
        volume.snapshot_delete(id, now(9)),
        Err(CoreError::Busy)
    ));
    drop(duplicate);
    assert!(matches!(
        volume.snapshot_delete(id, now(9)),
        Err(CoreError::Busy)
    ));
    let dev = volume.into_device();
    assert!(matches!(
        crate::mount(dev.clone()),
        Err(CoreError::UnsupportedIncompatFeatures(_))
    ));
    let mut volume = open(dev, MountMode::ReadWrite);
    assert!(matches!(
        volume.snapshot_stat(&handle, alpha),
        Err(CoreError::Stale)
    ));
    let reopened = volume.snapshot_open(id).unwrap();
    assert_eq!(bytes(&mut volume, &reopened, alpha), original);
    let resumed = volume
        .snapshot_read_directory_page(&reopened, OBJECT_ROOT, Some(first.next), 1)
        .unwrap();
    assert_eq!(resumed.entries[0].name, b"beta");
    drop(reopened);
    volume.snapshot_delete(id, now(10)).unwrap();
    assert!(matches!(volume.snapshot_open(id), Err(CoreError::NotFound)));
    // Old-mount handles do not pin a new mount; IDs are never reused.
    volume.snapshot_delete(second_id, now(11)).unwrap();
    assert!(volume.snapshot_create(now(12)).unwrap() > second_id);
    let mut readonly = open(volume.into_device(), MountMode::ReadOnly);
    assert!(matches!(
        readonly.snapshot_create(now(13)),
        Err(CoreError::ReadOnly)
    ));
}

#[test]
fn registered_views_force_cow_for_private_and_shared_files() {
    let mut volume = open(formatted(2048, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("private", &[3; 9000], now(2))
        .unwrap();
    volume
        .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, now(3))
        .unwrap();
    volume.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
    let id = volume.snapshot_create(now(4)).unwrap();
    let handle = volume.snapshot_open(id).unwrap();
    volume.write_file_at(file, 17, &[9; 6000], now(5)).unwrap();
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        0
    );
    assert_eq!(bytes(&mut volume, &handle, file), vec![3; 9000]);
    let clone = volume
        .clone_file(file, OBJECT_ROOT, "clone", now(6))
        .unwrap();
    let second_id = volume.snapshot_create(now(7)).unwrap();
    let second = volume.snapshot_open(second_id).unwrap();
    let captured = volume.read_file(clone).unwrap();
    volume.delete_file_in_root("private", now(8)).unwrap();
    volume.write_file_at(clone, 0, &[4; 9000], now(9)).unwrap();
    assert_eq!(bytes(&mut volume, &handle, file), vec![3; 9000]);
    assert_eq!(bytes(&mut volume, &second, clone), captured);
    drop(handle);
    drop(second);
    volume.snapshot_delete(id, now(10)).unwrap();
    volume.snapshot_delete(second_id, now(11)).unwrap();
    volume.write_file_at(clone, 0, &[5; 9000], now(12)).unwrap();
    assert!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place
            > 0
    );
}

#[test]
fn snapshot_creation_closes_the_log_window_and_reads_ignore_later_overlay() {
    let mut volume = open(formatted(4096, 16), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"original", now(2))
        .unwrap();
    volume
        .window_write_file_at(file, 0, b"captured", now(3))
        .unwrap();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "from-window",
                content: b"present",
            },
            now(3),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    let id = volume.snapshot_create(now(4)).unwrap();
    assert!(volume.window.is_none());
    let handle = volume.snapshot_open(id).unwrap();
    assert_eq!(bytes(&mut volume, &handle, file), b"captured");
    assert!(volume
        .snapshot_lookup(&handle, OBJECT_ROOT, "from-window")
        .unwrap()
        .is_some());
    volume
        .window_write_file_at(file, 0, b"overlaid", now(5))
        .unwrap();
    volume.window_fsync().unwrap();
    assert_eq!(volume.read_file(file).unwrap(), b"overlaid");
    assert_eq!(bytes(&mut volume, &handle, file), b"captured");
    let mut volume = open(volume.into_device(), MountMode::ReadWrite);
    assert_eq!(volume.read_file(file).unwrap(), b"overlaid");
    let handle = volume.snapshot_open(id).unwrap();
    assert_eq!(bytes(&mut volume, &handle, file), b"captured");
}

#[test]
fn an_old_view_does_not_retain_unrelated_churn_and_release_makes_progress() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    volume.set_reclaim_batch_blocks(64);
    let keep = volume
        .create_file_in_root("keep", &[0x71; 4096], now(2))
        .unwrap();
    let id = volume.snapshot_create(now(3)).unwrap();
    let handle = volume.snapshot_open(id).unwrap();
    volume.delete_file_in_root("keep", now(4)).unwrap();
    let mut minimum_free = volume.free_blocks();
    let mut maximum_retired = 0;
    for cycle in 0..160 {
        volume
            .create_file_in_root("temporary", &[cycle as u8; 4096], now(5 + cycle))
            .unwrap();
        volume
            .delete_file_in_root("temporary", now(5 + cycle))
            .unwrap();
        assert_eq!(bytes(&mut volume, &handle, keep), vec![0x71; 4096]);
        minimum_free = minimum_free.min(volume.free_blocks());
        maximum_retired =
            maximum_retired.max(volume.snapshot_state().unwrap().ledger.retained_blocks);
        if cycle % 16 == 0 {
            for _ in 0..8 {
                volume.snapshot_maintenance_step(now(5 + cycle)).unwrap();
            }
        }
    }
    drop(handle);
    volume.snapshot_delete(id, now(200)).unwrap();
    let mut transferred = 0;
    for _ in 0..256 {
        let progress = volume.snapshot_maintenance_step(now(201)).unwrap();
        assert!(progress.records_scanned <= 8);
        transferred += progress.blocks_transferred;
        if progress.ledger_retired_blocks == 0 {
            break;
        }
    }
    assert_eq!(volume.snapshot_state().unwrap().ledger.retained_blocks, 0);
    assert!(transferred > 0);
    assert!(volume.free_blocks() > minimum_free);
    println!("snapshot_volume_churn cycles=160 blocks=512 minimum_free={minimum_free} maximum_retired={maximum_retired} released={transferred}");
}

#[test]
fn snapshot_create_and_delete_publication_cuts_preserve_exact_membership_and_bytes() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"before", now(2))
        .unwrap();
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    let id = recording.snapshot_create(now(3)).unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut counts = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadWrite);
            assert_eq!(volume.read_file(file).unwrap(), b"before");
            let list = volume.snapshot_list(0, 64).unwrap();
            assert!(list.entries.len() <= 1);
            counts[list.entries.len()] += 1;
            if !list.entries.is_empty() {
                assert_eq!(list.entries[0].id, id);
                let handle = volume.snapshot_open(id).unwrap();
                assert_eq!(bytes(&mut volume, &handle, file), b"before");
            }
        });
    }
    assert!(counts.iter().all(|&count| count > 0));
    let mut volume = open(base, MountMode::ReadWrite);
    let id = volume.snapshot_create(now(3)).unwrap();
    volume.write_file_at(file, 0, b"after!", now(4)).unwrap();
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording.snapshot_delete(id, now(5)).unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut deleted = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadWrite);
            assert_eq!(volume.read_file(file).unwrap(), b"after!");
            let list = volume.snapshot_list(0, 64).unwrap();
            deleted[list.entries.len()] += 1;
            if !list.entries.is_empty() {
                let handle = volume.snapshot_open(id).unwrap();
                assert_eq!(bytes(&mut volume, &handle, file), b"before");
            }
        });
    }
    assert!(deleted.iter().all(|&count| count > 0));
    println!("snapshot_volume_cuts create_absent={} create_present={} delete_absent={} delete_present={}", counts[0], counts[1], deleted[0], deleted[1]);
}

#[test]
fn snapshot_admission_preserves_emergency_space_and_deletion_can_use_it() {
    let mut volume = open(formatted(128, 0), MountMode::ReadWrite);
    let keep = volume
        .create_file_in_root("keep", b"retained", now(2))
        .unwrap();
    let first = volume.snapshot_create(now(3)).unwrap();
    volume
        .set_snapshot_work_limits(SnapshotWorkLimits {
            max_views: 1,
            ..limits()
        })
        .unwrap();
    let generation = volume.generation();
    assert!(matches!(
        volume.snapshot_create(now(4)),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert_eq!(volume.generation(), generation);
    volume.set_snapshot_work_limits(limits()).unwrap();
    // Pause ordinary promotion to make the admission reserve observable.
    volume.set_reclaim_batch_blocks(0);
    let mut full = false;
    for i in 0..128 {
        match volume.create_file_in_root(&format!("fill-{i}"), &[0x41; 4096], now(5)) {
            Ok(_) => (),
            Err(CoreError::NoSpace) => {
                full = true;
                break;
            }
            Err(error) => panic!("unexpected fill error: {error}"),
        }
    }
    assert!(full);
    let mut refused = false;
    for _ in 0..8 {
        let generation = volume.generation();
        match volume.snapshot_create(now(6)) {
            Ok(_) => (),
            Err(CoreError::NoSpace) => {
                assert_eq!(volume.generation(), generation);
                refused = true;
                break;
            }
            Err(error) => panic!("unexpected admission error: {error}"),
        }
    }
    assert!(refused);
    assert!(volume.free_blocks() >= volume.emergency_headroom_blocks());
    let handle = volume.snapshot_open(first).unwrap();
    assert_eq!(bytes(&mut volume, &handle, keep), b"retained");
    drop(handle);
    // Deletion is destructive maintenance and may consume the reserved floor.
    volume.snapshot_delete(first, now(7)).unwrap();
    assert!(matches!(
        volume.snapshot_open(first),
        Err(CoreError::NotFound)
    ));
    volume.set_reclaim_batch_blocks(64);
    for _ in 0..16 {
        volume.snapshot_maintenance_step(now(8)).unwrap();
    }
    assert_eq!(volume.read_file(keep).unwrap(), b"retained");
}

#[test]
fn uncertain_snapshot_publication_blocks_mutation_and_remount_resolves_membership() {
    use afsplus_block::{FaultBackend, FaultPlan};
    for deletion in [false, true] {
        let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
        let file = volume
            .create_file_in_root("file", b"stable", now(2))
            .unwrap();
        if deletion {
            assert_eq!(volume.snapshot_create(now(3)).unwrap(), 1);
        }
        let base = volume.into_device();
        let dev = FaultBackend::new(
            base,
            FaultPlan {
                fail_flush_index: Some(1),
                ..Default::default()
            },
        );
        let mut volume = open(dev, MountMode::ReadWrite);
        let result = if deletion {
            volume.snapshot_delete(1, now(4))
        } else {
            volume.snapshot_create(now(4)).map(|_| ())
        };
        assert!(result.is_err());
        assert!(volume.window_poisoned);
        assert!(matches!(
            volume.snapshot_create(now(5)),
            Err(CoreError::WindowPoisoned)
        ));
        assert!(matches!(
            volume.create_file_in_root("blocked", b"", now(5)),
            Err(CoreError::WindowPoisoned)
        ));
        let mut volume = open(volume.into_device().into_inner(), MountMode::ReadWrite);
        assert_eq!(volume.read_file(file).unwrap(), b"stable");
        let list = volume.snapshot_list(0, 64).unwrap();
        assert_eq!(list.entries.len(), usize::from(!deletion));
        if !deletion {
            let handle = volume.snapshot_open(1).unwrap();
            assert_eq!(bytes(&mut volume, &handle, file), b"stable");
        }
        assert_eq!(volume.snapshot_create(now(6)).unwrap(), 2);
    }
}

#[test]
fn exhausted_snapshot_ids_do_not_publish_a_pending_namespace_window() {
    let mut dev = formatted(512, 16);
    let mut initial = open(dev.clone(), MountMode::ReadWrite);
    let root = initial.checkpoint.snapshot_roots.unwrap().registry;
    let (mut node, generation) = afsplus_format::tree::TreeNode::decode(&dev.peek(root)).unwrap();
    node.items[0].value = afsplus_format::snapshot::RegistryState { next_id: u64::MAX }
        .encode()
        .unwrap()
        .to_vec();
    dev.write_block(root, &node.encode(4096, generation).unwrap())
        .unwrap();
    let mut volume = open(dev, MountMode::ReadWrite);
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "pending",
                content: b"pending",
            },
            now(2),
        )
        .unwrap();
    let before = volume.generation();
    assert!(matches!(
        volume.snapshot_create(now(3)),
        Err(CoreError::Format(_))
    ));
    assert_eq!(volume.generation(), before);
    assert!(volume.window.is_some());
    assert!(initial.lookup_root("pending").unwrap().is_none());
}
