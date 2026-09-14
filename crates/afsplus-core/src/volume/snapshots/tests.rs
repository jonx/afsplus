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
    verify(&mut volume);
    volume
}

fn verify<D: BlockDevice>(volume: &mut Volume<D>) {
    for cp in std::iter::once(&volume.checkpoint).chain(volume.other_checkpoint.iter()) {
        let state =
            crate::verify::load_committed_state(&mut volume.dev, &volume.ident, cp).unwrap();
        let findings = crate::verify::full_sweep(&state, &volume.ident.geometry(), cp);
        assert!(
            findings.is_empty(),
            "generation {}: {findings:?}",
            cp.generation
        );
    }
}

fn formatted(blocks: u64, log_slots: u16) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, blocks);
    crate::mkfs_with_options(
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
        crate::MkfsOptions {
            persistent_snapshots: true,
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
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        0
    );
    volume.write_file_at(clone, 0, &[6; 9000], now(13)).unwrap();
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
        verify(&mut volume);
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

#[test]
fn snapshot_checker_rejects_resealed_ownership_and_historical_namespace_corruption() {
    use afsplus_format::snapshot::{LedgerState, LifetimeRecord, RegistryState};
    use afsplus_format::tree::{key_u64, TreeItem, TreeNode};
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", &[0x61; 4096], now(2))
        .unwrap();
    let id = volume.snapshot_create(now(3)).unwrap();
    let view = volume.snapshot_record(id).unwrap();
    let old_record = object_map::lookup_lba(
        &mut volume.dev,
        &volume.ident.geometry(),
        view.object_map_root,
        view.generation,
        file,
    )
    .unwrap()
    .unwrap();
    volume
        .write_file_at(file, 0, &[0x62; 4096], now(4))
        .unwrap();
    verify(&mut volume);
    let ident = volume.ident.clone();
    let cp = volume.checkpoint.clone();
    let roots = cp.snapshot_roots.unwrap();
    let base = volume.into_device();
    let decode_tree = |dev: &mut MemoryBackend, lba| {
        let mut buf = vec![0; 4096];
        dev.read_block(lba, &mut buf).unwrap();
        let (tree, generation) = TreeNode::decode(&buf).unwrap();
        assert!(tree.is_leaf());
        (tree, generation)
    };
    let reject = |mut dev: MemoryBackend, expected: &str| {
        let error = match crate::verify::load_committed_state(&mut dev, &ident, &cp) {
            Ok(state) => crate::verify::full_sweep(&state, &ident.geometry(), &cp).join("; "),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error:?}"
        );
    };
    let mutate_ledger = |mutate: &dyn Fn(&mut TreeNode)| {
        let mut dev = base.clone();
        let (mut tree, generation) = decode_tree(&mut dev, roots.lifetimes);
        mutate(&mut tree);
        // Preserve canonical adjacent-run packing so each fixture isolates its
        // intended ownership violation rather than failing an earlier check.
        let mut index = 1;
        while index + 1 < tree.items.len() {
            let start = afsplus_format::snapshot::decode_key(&tree.items[index].key).unwrap();
            let next = afsplus_format::snapshot::decode_key(&tree.items[index + 1].key).unwrap();
            let mut left =
                LifetimeRecord::decode(&tree.items[index].value, start, cp.generation, 512)
                    .unwrap();
            let right =
                LifetimeRecord::decode(&tree.items[index + 1].value, next, cp.generation, 512)
                    .unwrap();
            if start + left.blocks == next
                && left.birth == right.birth
                && left.retirement == right.retirement
            {
                left.blocks += right.blocks;
                tree.items[index].value = left.encode(start, cp.generation, 512).unwrap().to_vec();
                tree.items.remove(index + 1);
            } else {
                index += 1;
            }
        }
        tree.subtree_items = tree.items.len() as u64;
        dev.write_block(roots.lifetimes, &tree.encode(4096, generation).unwrap())
            .unwrap();
        dev
    };
    reject(
        mutate_ledger(&|tree| {
            let mut control = LedgerState::decode(&tree.items[0].value, 512).unwrap();
            control.retained_blocks += 1;
            tree.items[0].value = control.encode(512).unwrap().to_vec();
        }),
        "retained total",
    );
    reject(
        mutate_ledger(&|tree| {
            let index = tree
                .items
                .iter()
                .position(|item| {
                    let start = afsplus_format::snapshot::decode_key(&item.key).unwrap();
                    start != 0
                        && LifetimeRecord::decode(&item.value, start, cp.generation, 512)
                            .unwrap()
                            .retirement
                            == 0
                })
                .unwrap();
            tree.items.remove(index);
        }),
        "has no snapshot lifetime",
    );
    reject(
        mutate_ledger(&|tree| {
            tree.items.push(TreeItem {
                key: key_u64(roots.registry).to_vec(),
                value: LifetimeRecord {
                    blocks: 1,
                    birth: 1,
                    retirement: cp.generation,
                }
                .encode(roots.registry, cp.generation, 512)
                .unwrap()
                .to_vec(),
            });
            let mut control = LedgerState::decode(&tree.items[0].value, 512).unwrap();
            control.retained_blocks += 1;
            tree.items[0].value = control.encode(512).unwrap().to_vec();
            tree.items.sort_by(|a, b| a.key.cmp(&b.key));
        }),
        "aliases housekeeping or quarantine",
    );
    reject(
        mutate_ledger(&|tree| {
            let item = tree
                .items
                .iter_mut()
                .find(|item| {
                    let start = afsplus_format::snapshot::decode_key(&item.key).unwrap();
                    start != 0
                        && LifetimeRecord::decode(&item.value, start, cp.generation, 512)
                            .unwrap()
                            .retirement
                            != 0
                })
                .unwrap();
            let start = afsplus_format::snapshot::decode_key(&item.key).unwrap();
            let mut run = LifetimeRecord::decode(&item.value, start, cp.generation, 512).unwrap();
            let blocks = run.blocks;
            run.retirement = 0;
            item.value = run.encode(start, cp.generation, 512).unwrap().to_vec();
            let mut control = LedgerState::decode(&tree.items[0].value, 512).unwrap();
            control.retained_blocks -= blocks;
            tree.items[0].value = control.encode(512).unwrap().to_vec();
        }),
        "inconsistent live ownership",
    );
    reject(
        mutate_ledger(&|tree| {
            let item = tree
                .items
                .iter_mut()
                .find(|item| {
                    let start = afsplus_format::snapshot::decode_key(&item.key).unwrap();
                    start != 0 && {
                        let run =
                            LifetimeRecord::decode(&item.value, start, cp.generation, 512).unwrap();
                        start <= cp.object_map_block && cp.object_map_block < start + run.blocks
                    }
                })
                .unwrap();
            let start = afsplus_format::snapshot::decode_key(&item.key).unwrap();
            let mut run = LifetimeRecord::decode(&item.value, start, cp.generation, 512).unwrap();
            assert!(run.birth > 1);
            run.birth = 1;
            item.value = run.encode(start, cp.generation, 512).unwrap().to_vec();
        }),
        "birth disagrees with header",
    );
    let mut dev = base.clone();
    let (mut tree, generation) = decode_tree(&mut dev, roots.registry);
    tree.items[0].value = RegistryState { next_id: id }.encode().unwrap().to_vec();
    dev.write_block(roots.registry, &tree.encode(4096, generation).unwrap())
        .unwrap();
    reject(dev, "snapshot ID outside");
    let mut dev = base.clone();
    let mut buf = vec![0; 4096];
    dev.read_block(old_record, &mut buf).unwrap();
    let (mut object, generation) = ObjectRecord::decode_with_generation(&buf).unwrap();
    object.link_count += 1;
    dev.write_block(old_record, &object.encode(4096, generation).unwrap())
        .unwrap();
    reject(
        dev,
        &format!("historical object {file} link count mismatch"),
    );
    let mut dev = base.clone();
    let (mut tree, generation) = decode_tree(&mut dev, roots.registry);
    let mut bad_view = view;
    bad_view.generation = 1;
    bad_view.committed_tx_id = 1;
    tree.items[1].value = bad_view.encode(cp.generation, 512).unwrap().to_vec();
    dev.write_block(roots.registry, &tree.encode(4096, generation).unwrap())
        .unwrap();
    reject(dev, "generation");
    let mut dev = base;
    let mut state = crate::verify::load_committed_state(&mut dev, &ident, &cp).unwrap();
    assert!(state.snapshot_owned_blocks.contains(&old_record));
    state.bitmaps.pages[0][0].set_allocated(old_record as u32, false);
    let findings = crate::verify::full_sweep(&state, &ident.geometry(), &cp);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains(&format!("block {old_record} is marked FREE"))),
        "{findings:?}"
    );
}

#[test]
fn checker_rejects_disconnected_directory_cycles_with_matching_link_counts() {
    use afsplus_format::tree::{TreeItem, TreeNode};
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let child = volume.create_directory_in_root("child", now(2)).unwrap();
    let id = volume.snapshot_create(now(3)).unwrap();
    let view = volume.snapshot_record(id).unwrap();
    let root = snapshot::view::object(&mut volume.dev, &volume.ident, view, OBJECT_ROOT)
        .unwrap()
        .unwrap();
    let dir = snapshot::view::object(&mut volume.dev, &volume.ident, view, child)
        .unwrap()
        .unwrap();
    // Give the live namespace independent root/child directory blocks.
    volume.create_directory_in_root("later", now(4)).unwrap();
    volume.create_directory(child, "nested", now(5)).unwrap();
    verify(&mut volume);
    let mut buf = vec![0; 4096];
    volume.dev.read_block(root.data_root, &mut buf).unwrap();
    let (mut root_node, generation) = TreeNode::decode(&buf).unwrap();
    root_node.items.clear();
    root_node.subtree_items = 0;
    volume
        .dev
        .write_block(root.data_root, &root_node.encode(4096, generation).unwrap())
        .unwrap();
    volume.dev.read_block(dir.data_root, &mut buf).unwrap();
    let (mut dir_node, generation) = TreeNode::decode(&buf).unwrap();
    let (key, value) = directory::encode_entry(
        &volume.ident,
        &DirEntry {
            name: b"self".to_vec(),
            key: b"self".to_vec(),
            child_id: child,
            child_type_hint: 2,
        },
    )
    .unwrap();
    dir_node.items.push(TreeItem { key, value });
    dir_node.subtree_items = 1;
    volume
        .dev
        .write_block(dir.data_root, &dir_node.encode(4096, generation).unwrap())
        .unwrap();
    let error = match crate::verify::load_committed_state(
        &mut volume.dev,
        &volume.ident,
        &volume.checkpoint,
    ) {
        Ok(_) => panic!("disconnected cycle passed ownership verification"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("snapshot 1") && error.contains("unreachable from namespace roots"),
        "{error}"
    );
}

#[test]
fn last_snapshot_deletion_preserves_older_selectable_view_during_the_next_write() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"before", now(2))
        .unwrap();
    volume
        .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, now(3))
        .unwrap();
    let id = volume.snapshot_create(now(4)).unwrap();
    let view = volume.snapshot_record(id).unwrap();
    volume.snapshot_delete(id, now(5)).unwrap();
    let ident = volume.ident.clone();
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording.write_file_at(file, 0, b"after!", now(6)).unwrap();
    assert_eq!(
        recording
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        0
    );
    let (_, log) = recording.into_device().into_parts();
    let mut protected = 0;
    let mut released = 0;
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |mut state| {
            let selection = select_checkpoint(&mut state.image, &ident).unwrap();
            let mut reachable = false;
            for cp in std::iter::once(&selection.chosen).chain(selection.other.iter()) {
                let loaded =
                    crate::verify::load_committed_state(&mut state.image, &ident, cp).unwrap();
                assert!(crate::verify::full_sweep(&loaded, &ident.geometry(), cp).is_empty());
                let roots = cp.snapshot_roots.unwrap();
                let page = snapshot::read_registry_page(
                    &mut state.image,
                    &ident.geometry(),
                    roots.registry,
                    cp.generation,
                    id,
                    1,
                )
                .unwrap();
                reachable |= page.records.iter().any(|(found, _)| *found == id);
            }
            if reachable {
                let mut bytes = [0; 6];
                snapshot::view::read_at(&mut state.image, &ident, view, file, 0, &mut bytes)
                    .unwrap();
                assert_eq!(&bytes, b"before");
                protected += 1;
            } else {
                released += 1;
            }
        });
    }
    assert!(protected > 0 && released > 0);
    println!("snapshot_post_delete_write protected={protected} released={released}");
}

#[test]
fn missing_or_unreadable_older_snapshot_roots_cannot_authorize_in_place_writes() {
    for missing in [false, true] {
        let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
        let file = volume
            .create_file_in_root("file", b"before", now(2))
            .unwrap();
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, now(3))
            .unwrap();
        let id = volume.snapshot_create(now(4)).unwrap();
        volume.snapshot_delete(id, now(5)).unwrap();
        let older = volume.other_checkpoint.as_mut().unwrap();
        if missing {
            older.snapshot_roots = None;
            volume
                .dev
                .write_block(
                    volume.ident.checkpoint_slots[1 - volume.current_slot],
                    &older.encode(4096).unwrap(),
                )
                .unwrap();
        } else {
            volume
                .dev
                .write_block(older.snapshot_roots.unwrap().registry, &[0; 4096])
                .unwrap();
        }
        // The chosen checkpoint remains readable. Damage in the other slot
        // cannot grant permission to weaken the current write's byte contract.
        volume.write_file_at(file, 0, b"after!", now(6)).unwrap();
        assert_eq!(
            volume
                .last_commit_stats()
                .unwrap()
                .data_blocks_overwritten_in_place,
            0
        );
        assert_eq!(volume.read_file(file).unwrap(), b"after!");
        verify(&mut volume);
    }
}
