use super::*;
use crate::mount::select_checkpoint;
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

// Exercise the public opt-in mount path, including configuration before replay.
fn open<D: BlockDevice>(dev: D, mode: MountMode) -> Volume<D> {
    let mut volume =
        crate::mount_with_snapshot_limits(dev, crate::MountOptions { mode }, limits()).unwrap();
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

/// Any accidental recovery write makes a rejected-mount test fail immediately.
struct ForbidWrites(MemoryBackend);
impl BlockDevice for ForbidWrites {
    fn block_size(&self) -> usize {
        self.0.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.0.total_blocks()
    }
    fn read_block(&mut self, lba: u64, out: &mut [u8]) -> Result<(), afsplus_block::BlockError> {
        self.0.read_block(lba, out)
    }
    fn write_block(&mut self, _: u64, _: &[u8]) -> Result<(), afsplus_block::BlockError> {
        panic!("rejected mount wrote to the source")
    }
    fn flush(&mut self) -> Result<(), afsplus_block::BlockError> {
        panic!("rejected mount flushed the source")
    }
}

#[test]
fn public_snapshot_mount_validates_admission_before_pending_recovery_writes() {
    let mut volume = open(formatted(1024, 8), MountMode::ReadWrite);
    volume.snapshot_create(now(2)).unwrap();
    volume.snapshot_create(now(3)).unwrap();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "pending",
                content: b"durable",
            },
            now(4),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    let base = volume.into_device();
    for mode in [
        MountMode::ReadWrite,
        MountMode::Recovery,
        MountMode::ReadOnly,
        MountMode::NoChanges,
    ] {
        assert!(matches!(
            crate::mount_with_options(ForbidWrites(base.clone()), crate::MountOptions { mode }),
            Err(CoreError::UnsupportedIncompatFeatures(_))
        ));
        for bad in [
            SnapshotWorkLimits {
                max_views: 1,
                ..limits()
            },
            SnapshotWorkLimits {
                max_views: 0,
                ..limits()
            },
            SnapshotWorkLimits {
                max_edit_records: 0,
                ..limits()
            },
            SnapshotWorkLimits {
                reclaim_records: 0,
                ..limits()
            },
            SnapshotWorkLimits {
                reclaim_records: usize::MAX,
                ..limits()
            },
            SnapshotWorkLimits {
                reclaim_records: 4097,
                ..limits()
            },
        ] {
            assert!(matches!(
                crate::mount_with_snapshot_limits(
                    ForbidWrites(base.clone()),
                    crate::MountOptions { mode },
                    bad
                ),
                Err(CoreError::PrototypeLimit(_))
            ));
        }
    }
}

#[test]
fn public_snapshot_mount_modes_preserve_history_and_replay_only_when_requested() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(1024, 8), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"before", now(2))
        .unwrap();
    let id = volume.snapshot_create(now(3)).unwrap();
    volume
        .window_write_file_at(file, 0, b"after!", now(4))
        .unwrap();
    volume.window_fsync().unwrap();
    let generation = volume.generation();
    let base = volume.into_device();
    for mode in [
        MountMode::ReadOnly,
        MountMode::NoChanges,
        MountMode::Recovery,
        MountMode::ReadWrite,
    ] {
        let mut volume = open(TraceBackend::new(base.clone()), mode);
        let handle = volume.snapshot_open(id).unwrap();
        assert_eq!(bytes(&mut volume, &handle, file), b"before");
        let replay = matches!(mode, MountMode::ReadWrite | MountMode::Recovery);
        assert_eq!(
            volume.read_file(file).unwrap(),
            if replay { b"after!" } else { b"before" }
        );
        if replay {
            assert!(volume.generation() > generation);
            assert_eq!(volume.pending_intent_records(), 0);
        } else {
            assert_eq!(volume.generation(), generation);
            assert!(volume.pending_intent_records() > 0);
        }
        if mode != MountMode::ReadWrite {
            assert!(matches!(
                volume.snapshot_create(now(5)),
                Err(CoreError::ReadOnly)
            ));
            assert!(matches!(
                volume.create_file_in_root("forbidden", b"x", now(5)),
                Err(CoreError::ReadOnly)
            ));
        }
        let trace = volume.into_device();
        if replay {
            assert!(trace.stats().writes > 0);
            assert!(trace.stats().flushes > 0);
        } else {
            assert_eq!(trace.stats().writes, 0);
            assert_eq!(trace.stats().flushes, 0);
        }
    }
}

#[test]
fn public_snapshot_mount_rejects_unknown_features_and_corrupt_selected_roots_without_writes() {
    let mut volume = open(formatted(1024, 8), MountMode::ReadWrite);
    volume.snapshot_create(now(2)).unwrap();
    let roots = volume.checkpoint.snapshot_roots.unwrap();
    let base = volume.into_device();
    for mode in [
        MountMode::ReadWrite,
        MountMode::Recovery,
        MountMode::ReadOnly,
        MountMode::NoChanges,
    ] {
        for lba in [roots.registry, roots.lifetimes] {
            let mut damaged = base.clone();
            damaged.apply_raw(lba, &[0; 4096]);
            assert!(matches!(
                crate::mount_with_snapshot_limits(
                    ForbidWrites(damaged),
                    crate::MountOptions { mode },
                    limits()
                ),
                Err(CoreError::Corrupt(_))
            ));
        }
        let mut damaged = base.clone();
        let mut ident = Identification::decode(&damaged.peek(0)).unwrap();
        ident.features.incompat |= 1 << 63;
        damaged.apply_raw(0, &ident.encode(4096).unwrap());
        assert!(
            matches!(crate::mount_with_snapshot_limits(ForbidWrites(damaged), crate::MountOptions { mode }, limits()), Err(CoreError::UnsupportedIncompatFeatures(bits)) if bits == 1 << 63)
        );
    }
}

#[test]
fn public_snapshot_mount_bounds_root_reads_across_registry_pages() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(2048, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("stable", b"kept", now(2))
        .unwrap();
    let mut ids = Vec::new();
    let mut reads = Vec::new();
    for count in 1..=90 {
        ids.push(volume.snapshot_create(now(count + 2)).unwrap());
        if count == 1 || count == 90 {
            let mut mounted = crate::mount_with_snapshot_limits(
                TraceBackend::new(volume.into_device()),
                crate::MountOptions {
                    mode: MountMode::NoChanges,
                },
                limits(),
            )
            .unwrap();
            let io = mounted.dev.stats();
            assert_eq!(io.writes, 0);
            assert_eq!(io.flushes, 0);
            reads.push(io.reads);
            for &id in &ids {
                let handle = mounted.snapshot_open(id).unwrap();
                assert_eq!(bytes(&mut mounted, &handle, file), b"kept");
            }
            verify(&mut mounted);
            volume = open(mounted.into_device().into_inner(), MountMode::ReadWrite);
        }
    }
    // One registry split may add root/control path reads; mount must not walk
    // each view or its namespace. Measure before any reader or full checker.
    assert!(reads[1] <= reads[0] + 4, "mount reads: {reads:?}");
    eprintln!("snapshot mount reads at 1 and 90 registered views: {reads:?}");
}

#[test]
fn public_snapshot_mount_recovery_cuts_preserve_acknowledged_live_and_historical_bytes() {
    let mut volume = open(formatted(512, 8), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"before", now(2))
        .unwrap();
    let id = volume.snapshot_create(now(3)).unwrap();
    volume
        .window_write_file_at(file, 0, b"after!", now(4))
        .unwrap();
    volume.window_fsync().unwrap();
    let base = volume.into_device();
    let recording = open(RecordingBackend::new(base.clone()), MountMode::Recovery);
    let (_, log) = recording.into_device().into_parts();
    assert!(!log.is_empty());
    let mut states = 0;
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut recovered = open(state.image, MountMode::Recovery);
            assert_eq!(recovered.read_file(file).unwrap(), b"after!");
            let handle = recovered.snapshot_open(id).unwrap();
            assert_eq!(bytes(&mut recovered, &handle, file), b"before");
            assert_eq!(recovered.pending_intent_records(), 0);
            states += 1;
        });
    }
    eprintln!("snapshot-aware recovery crash states: {states}");
    assert!(states > log.len());
}

#[test]
fn snapshot_allocation_pages_preserve_sparse_and_unwritten_ranges_after_remount() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(4096, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("ranges", &[], now(2)).unwrap();
    let direct = volume
        .create_file_in_root("direct", &[7; 5000], now(2))
        .unwrap();
    let empty = volume.create_file_in_root("empty", &[], now(2)).unwrap();
    let mut expected = Vec::new();
    for i in 0..140u64 {
        let offset = i * 3 * 4096;
        if i % 2 == 0 {
            volume
                .write_file_at(file, offset, &[3; 4096], now(3))
                .unwrap();
        } else {
            volume.preallocate_file(file, offset, 4096, now(3)).unwrap();
        }
        expected.push(SnapshotAllocationRange {
            offset,
            length: 4096,
            unwritten: i % 2 != 0,
        });
    }
    let distant = 1u64 << 40;
    volume
        .preallocate_file(file, distant, 8192, now(4))
        .unwrap();
    // Physical allocation may split a reservation into multiple semantic records.
    let id = volume.snapshot_create(now(5)).unwrap();
    volume.truncate_file(file, 0, now(6)).unwrap();
    let mut volume = open(TraceBackend::new(volume.into_device()), MountMode::ReadOnly);
    let handle = volume.snapshot_open(id).unwrap();
    let baseline = volume.dev.stats();
    for limit in [1, 7, 64] {
        let mut cursor = 0;
        let mut collected = Vec::new();
        loop {
            let before = volume.dev.stats().reads;
            let page = volume
                .snapshot_allocation_page(&handle, file, cursor, limit)
                .unwrap();
            assert!(
                volume.dev.stats().reads - before <= 32,
                "page must seek past huge holes"
            );
            assert!(page.ranges.len() <= limit);
            assert_eq!(page.next, cursor + page.ranges.len() as u64);
            collected.extend(page.ranges);
            if page.eof {
                break;
            }
            assert!(page.next > cursor);
            cursor = page.next;
        }
        assert_eq!(&collected[..140], &expected);
        let tail = &collected[140..];
        assert_eq!(tail[0].offset, distant);
        assert!(tail.iter().all(|r| r.unwritten));
        assert_eq!(tail.iter().map(|r| r.length).sum::<u64>(), 8192);
    }
    let page = volume
        .snapshot_allocation_page(&handle, direct, 0, 1)
        .unwrap();
    assert_eq!(
        page.ranges,
        vec![SnapshotAllocationRange {
            offset: 0,
            length: 8192,
            unwritten: false
        }]
    );
    assert!(page.eof);
    assert!(volume
        .snapshot_allocation_page(&handle, empty, 0, 1)
        .unwrap()
        .ranges
        .is_empty());
    assert!(volume
        .snapshot_allocation_page(&handle, file, 0, 0)
        .is_err());
    assert!(volume
        .snapshot_allocation_page(&handle, file, 0, 65)
        .is_err());
    assert!(volume
        .snapshot_allocation_page(&handle, file, u64::MAX, 1)
        .is_err());
    assert!(volume
        .snapshot_allocation_page(&handle, OBJECT_ROOT, 0, 1)
        .is_err());
    assert_eq!(volume.dev.stats().writes, baseline.writes);
    assert_eq!(volume.dev.stats().flushes, baseline.flushes);
    let mut remounted = open(volume.into_device(), MountMode::ReadOnly);
    assert!(remounted
        .snapshot_allocation_page(&handle, file, 0, 1)
        .is_err());
}

#[test]
fn snapshot_allocation_preserves_the_final_rounded_u64_block() {
    let mut volume = open(formatted(1024, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("last-block", &[], now(2))
        .unwrap();
    let offset = u64::MAX - 4095;
    volume.preallocate_file(file, offset, 4095, now(3)).unwrap();
    let id = volume.snapshot_create(now(4)).unwrap();
    let mut volume = open(volume.into_device(), MountMode::ReadOnly);
    let view = volume.snapshot_open(id).unwrap();
    let page = volume.snapshot_allocation_page(&view, file, 0, 1).unwrap();
    assert_eq!(
        page.ranges,
        vec![SnapshotAllocationRange {
            offset,
            length: 4096,
            unwritten: true
        }]
    );
    assert!(page.eof);
    assert_eq!(
        page.ranges[0].offset as u128 + page.ranges[0].length as u128,
        1u128 << 64
    );
    assert_eq!(
        volume
            .snapshot_stat(&view, file)
            .unwrap()
            .unwrap()
            .size_bytes,
        0
    );
}

#[test]
fn snapshot_preallocation_publication_is_atomic_and_preserves_captured_layout() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("file", b"keep", now(2)).unwrap();
    let snapshot = volume.snapshot_create(now(3)).unwrap();
    let before = volume.stat(file).unwrap().unwrap();
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording
        .preallocate_file(file, 4096, 8192, now(4))
        .unwrap();
    let after = recording.stat(file).unwrap().unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut counts = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadOnly);
            let record = volume.stat(file).unwrap().unwrap();
            assert!(record == before || record == after);
            counts[usize::from(record == after)] += 1;
            assert_eq!(volume.read_file(file).unwrap(), b"keep");
            let view = volume.snapshot_open(snapshot).unwrap();
            let page = volume.snapshot_allocation_page(&view, file, 0, 64).unwrap();
            assert!(page.eof);
            assert_eq!(
                page.ranges,
                vec![SnapshotAllocationRange {
                    offset: 0,
                    length: 4096,
                    unwritten: false
                }]
            );
            assert_eq!(bytes(&mut volume, &view, file), b"keep");
        });
    }
    assert!(counts.iter().all(|n| *n > 0));
    println!(
        "snapshot_preallocation_cuts old={} new={}",
        counts[0], counts[1]
    );
}

#[test]
fn reservation_write_reuses_private_unwritten_blocks_in_a_mixed_write() {
    let mut volume = open(formatted(1024, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("reserved", &[], now(2)).unwrap();
    volume.preallocate_file(file, 0, 4 * 4096, now(3)).unwrap();
    volume.truncate_file(file, 4 * 4096, now(4)).unwrap();
    volume.write_file_at(file, 0, &[7; 4096], now(5)).unwrap();
    let id = volume.snapshot_create(now(6)).unwrap();
    let before_record = volume.stat(file).unwrap().unwrap();
    let (before, _) = volume.load_file_layout(&before_record).unwrap();
    let expected_old = volume.read_file(file).unwrap();
    let mut expected = expected_old.clone();
    let offset = 4096 - 7;
    let data = vec![0x5a; 4096 + 18];
    expected[offset..offset + data.len()].copy_from_slice(&data);
    volume
        .write_file_at(file, offset as u64, &data, now(7))
        .unwrap();
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_initialized_from_reservation,
        2
    );
    let record = volume.stat(file).unwrap().unwrap();
    let (after, _) = volume.load_file_layout(&record).unwrap();
    for logical in [1, 2] {
        let old = extent_at(&before, logical).unwrap();
        let new = extent_at(&after, logical).unwrap();
        assert_eq!(
            old.physical_start + logical - old.logical_start,
            new.physical_start + logical - new.logical_start
        );
        assert_eq!(new.flags, 0);
    }
    assert_ne!(
        extent_at(&before, 0).unwrap().physical_start,
        extent_at(&after, 0).unwrap().physical_start
    );
    assert_eq!(volume.read_file(file).unwrap(), expected);
    let view = volume.snapshot_open(id).unwrap();
    assert_eq!(bytes(&mut volume, &view, file), expected_old);
    verify(&mut volume);
    let mut volume = open(volume.into_device(), MountMode::ReadOnly);
    assert_eq!(volume.read_file(file).unwrap(), expected);
    let view = volume.snapshot_open(id).unwrap();
    assert_eq!(bytes(&mut volume, &view, file), expected_old);
}

#[test]
fn reservation_write_keeps_shared_and_stale_marked_ranges_cow() {
    let mut volume = open(formatted(1024, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("file", &[], now(2)).unwrap();
    volume.preallocate_file(file, 0, 4 * 4096, now(3)).unwrap();
    volume.truncate_file(file, 4 * 4096, now(4)).unwrap();
    let clone = volume
        .clone_file(file, OBJECT_ROOT, "clone", now(5))
        .unwrap();
    let snapshot = volume.snapshot_create(now(6)).unwrap();
    volume.write_file_at(file, 0, b"first", now(7)).unwrap();
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_initialized_from_reservation,
        0
    );
    assert_eq!(volume.read_file(clone).unwrap(), vec![0; 4 * 4096]);
    volume.delete_file_in_root("clone", now(8)).unwrap();
    volume
        .write_file_at(file, 3 * 4096, b"last", now(9))
        .unwrap();
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_initialized_from_reservation,
        0
    );
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(bytes(&mut volume, &view, file), vec![0; 4 * 4096]);
    assert_eq!(bytes(&mut volume, &view, clone), vec![0; 4 * 4096]);
    verify(&mut volume);
}

#[test]
fn reservation_write_progresses_without_replacement_data_capacity() {
    let mut volume = open(formatted(256, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("reserved", &[], now(2)).unwrap();
    volume.preallocate_file(file, 0, 32 * 4096, now(3)).unwrap();
    volume.truncate_file(file, 32 * 4096, now(4)).unwrap();
    let snapshot = volume.snapshot_create(now(5)).unwrap();
    let filler = volume.create_file_in_root("pressure", &[], now(6)).unwrap();
    let fill = volume.available_blocks().saturating_sub(24);
    volume
        .preallocate_file(filler, 0, fill * 4096, now(7))
        .unwrap();
    let available = volume.available_blocks();
    assert!(available < 32);
    volume
        .write_file_at(file, 0, &vec![0x5a; 32 * 4096], now(8))
        .unwrap();
    let stats = volume.last_commit_stats().unwrap();
    assert_eq!(stats.data_blocks_initialized_from_reservation, 32);
    assert_eq!(stats.data_blocks_overwritten_in_place, 0);
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(bytes(&mut volume, &view, file), vec![0; 32 * 4096]);
    assert_eq!(volume.read_file(file).unwrap(), vec![0x5a; 32 * 4096]);
    verify(&mut volume);
    println!("reservation_low_space available_before={available} initialized={} metadata={} bytes={} flushes={}",
        stats.data_blocks_initialized_from_reservation,stats.metadata_blocks_written,stats.bytes_written,stats.flushes);
}

#[test]
fn reservation_write_crashes_preserve_old_zeros_or_complete_new_bytes() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("reserved", &[], now(2)).unwrap();
    volume.preallocate_file(file, 0, 8192, now(3)).unwrap();
    volume.truncate_file(file, 8192, now(4)).unwrap();
    let id = volume.snapshot_create(now(5)).unwrap();
    let base = volume.into_device();
    let mut expected = vec![0; 8192];
    expected[7..5007].fill(0x5a);
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording
        .write_file_at(file, 7, &vec![0x5a; 5000], now(6))
        .unwrap();
    assert_eq!(
        recording
            .last_commit_stats()
            .unwrap()
            .data_blocks_initialized_from_reservation,
        2
    );
    let (_, log) = recording.into_device().into_parts();
    let mut counts = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadOnly);
            let actual = volume.read_file(file).unwrap();
            assert!(actual == vec![0; 8192] || actual == expected);
            counts[usize::from(actual == expected)] += 1;
            let view = volume.snapshot_open(id).unwrap();
            assert_eq!(bytes(&mut volume, &view, file), vec![0; 8192]);
        });
    }
    assert!(counts.iter().all(|n| *n > 0));
    println!(
        "reservation_initialization_cuts old={} new={}",
        counts[0], counts[1]
    );
}

#[test]
fn reservation_write_refuses_false_private_markers_before_data_writes() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("file", &[], now(2)).unwrap();
    volume.preallocate_file(file, 0, 8192, now(3)).unwrap();
    volume.truncate_file(file, 8192, now(4)).unwrap();
    volume
        .clone_file(file, OBJECT_ROOT, "peer", now(5))
        .unwrap();
    let root = volume.stat(file).unwrap().unwrap().data_root;
    let mut dev = volume.into_device();
    let (mut node, generation) = afsplus_format::tree::TreeNode::decode(&dev.peek(root)).unwrap();
    assert!(node.is_leaf());
    for item in &mut node.items {
        item.value[16..20].copy_from_slice(&EXTENT_UNWRITTEN.to_le_bytes());
    }
    dev.write_block(root, &node.encode(4096, generation).unwrap())
        .unwrap();
    let mut volume = crate::mount_with_snapshot_limits(
        TraceBackend::new(dev),
        crate::MountOptions::default(),
        limits(),
    )
    .unwrap();
    let error = volume
        .write_file_at(file, 0, b"denied", now(6))
        .unwrap_err();
    assert!(error.to_string().contains("overlaps shared references"));
    assert_eq!(volume.dev.stats().writes, 0);
    assert_eq!(volume.dev.stats().flushes, 0);
}

#[test]
fn reservation_write_io_errors_preserve_old_logical_zeros_and_require_reconciliation() {
    use afsplus_block::BlockError;
    struct Fault {
        inner: MemoryBackend,
        fail_write: bool,
        fail_flush: Option<u64>,
        flushes: u64,
    }
    impl BlockDevice for Fault {
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }
        fn total_blocks(&self) -> u64 {
            self.inner.total_blocks()
        }
        fn read_block(&mut self, lba: u64, out: &mut [u8]) -> Result<(), BlockError> {
            self.inner.read_block(lba, out)
        }
        fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
            self.inner.write_block(lba, data)?;
            if std::mem::take(&mut self.fail_write) {
                return Err(BlockError::Injected("completed initialization write"));
            }
            Ok(())
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            self.inner.flush()?;
            let index = self.flushes;
            self.flushes += 1;
            if self.fail_flush == Some(index) {
                self.fail_flush = None;
                return Err(BlockError::Injected("initialization barrier"));
            }
            Ok(())
        }
    }
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("reserved", &[], now(2)).unwrap();
    volume.preallocate_file(file, 0, 8192, now(3)).unwrap();
    volume.truncate_file(file, 8192, now(4)).unwrap();
    let id = volume.snapshot_create(now(5)).unwrap();
    let base = volume.into_device();
    for phase in 0..4 {
        let dev = Fault {
            inner: base.clone(),
            fail_write: phase == 0,
            fail_flush: if phase == 0 { None } else { Some(phase - 1) },
            flushes: 0,
        };
        let mut volume = open(dev, MountMode::ReadWrite);
        assert!(volume
            .write_file_at(file, 0, &[0x5a; 8192], now(6))
            .is_err());
        if phase == 3 {
            assert!(matches!(
                volume.write_file_at(file, 0, b"retry", now(7)),
                Err(CoreError::WindowPoisoned)
            ));
        } else {
            assert_eq!(volume.read_file(file).unwrap(), vec![0; 8192]);
        }
        let mut volume = open(volume.into_device().inner, MountMode::ReadOnly);
        assert_eq!(
            volume.read_file(file).unwrap(),
            if phase == 3 {
                vec![0x5a; 8192]
            } else {
                vec![0; 8192]
            }
        );
        let view = volume.snapshot_open(id).unwrap();
        assert_eq!(bytes(&mut volume, &view, file), vec![0; 8192]);
    }
}

#[test]
fn bounded_reservation_edits_skip_unrelated_fragmented_extents() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(8192, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("fragmented", &[], now(2))
        .unwrap();
    for index in 0..600u64 {
        volume
            .preallocate_file(file, index * 3 * 4096, 4096, now(3))
            .unwrap();
    }
    let snapshot = volume.snapshot_create(now(4)).unwrap();
    let base = volume.into_device();
    let mut volume = open(TraceBackend::new(base.clone()), MountMode::ReadWrite);
    let before_reads = volume.dev.stats().reads;
    volume
        .preallocate_file_bounded(
            file,
            901 * 4096,
            4096,
            now(5),
            FileEditLimits {
                max_blocks: 1,
                max_records: 4,
            },
        )
        .unwrap();
    let reads = volume.dev.stats().reads - before_reads;
    let stats = volume.last_commit_stats().unwrap();
    assert!(reads < 150, "range edit read {reads} blocks");
    assert!(
        stats.metadata_blocks_written < 20,
        "rewrote unrelated extent tree nodes"
    );
    assert_eq!(volume.stat(file).unwrap().unwrap().data_blocks, 601);
    assert_eq!(volume.stat(file).unwrap().unwrap().size_bytes, 0);
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(
        volume
            .snapshot_stat(&view, file)
            .unwrap()
            .unwrap()
            .allocated_bytes,
        600 * 4096
    );
    verify(&mut volume);
    let mut rejected = open(TraceBackend::new(base), MountMode::ReadWrite);
    let before = rejected.dev.stats();
    assert!(rejected
        .preallocate_file_bounded(
            file,
            0,
            9 * 4096,
            now(6),
            FileEditLimits {
                max_blocks: 9,
                max_records: 2
            }
        )
        .is_err());
    assert!(rejected
        .preallocate_file_bounded(
            file,
            901 * 4096,
            8192,
            now(6),
            FileEditLimits {
                max_blocks: 1,
                max_records: 4
            }
        )
        .is_err());
    assert_eq!(rejected.dev.stats().writes, before.writes);
    assert_eq!(rejected.dev.stats().flushes, before.flushes);
    assert_eq!(rejected.stat(file).unwrap().unwrap().data_blocks, 600);
    println!(
        "bounded_reservation fragmented_records=600 reads={reads} metadata={} bytes={} flushes={}",
        stats.metadata_blocks_written, stats.bytes_written, stats.flushes
    );
}

#[test]
fn bounded_reservation_refusal_and_boundary_retry_preserve_layout() {
    use afsplus_block::TraceBackend;
    let mut volume = open(TraceBackend::new(formatted(512, 0)), MountMode::ReadWrite);
    let file = volume.create_file_in_root("bounded", &[], now(2)).unwrap();
    let limits = FileEditLimits {
        max_blocks: 1,
        max_records: 4,
    };
    // Empty, before-first, between records, and after-last windows.
    for block in [6, 0, 3, 9] {
        volume
            .preallocate_file_bounded(file, block * 4096, 4096, now(3), limits)
            .unwrap();
    }
    let before = volume.stat(file).unwrap().unwrap();
    let io = volume.dev.stats();
    // Two neighbors fit, but inserting a third distinct run exceeds the result budget.
    assert!(matches!(
        volume.preallocate_file_bounded(
            file,
            4 * 4096,
            4096,
            now(4),
            FileEditLimits {
                max_blocks: 1,
                max_records: 2
            }
        ),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert_eq!(volume.dev.stats().writes, io.writes);
    assert_eq!(volume.dev.stats().flushes, io.flushes);
    assert_eq!(volume.stat(file).unwrap().unwrap(), before);
    volume
        .preallocate_file_bounded(file, 4 * 4096, 4096, now(4), limits)
        .unwrap();
    let io = volume.dev.stats();
    let before = volume.stat(file).unwrap().unwrap();
    volume
        .preallocate_file_bounded(file, 4 * 4096, 4096, now(5), limits)
        .unwrap();
    assert_eq!(volume.dev.stats().writes, io.writes);
    assert_eq!(volume.stat(file).unwrap().unwrap(), before);
    assert_eq!(before.data_blocks, 5);
    verify(&mut volume);
}

#[test]
fn bounded_reservation_tree_publication_preserves_snapshot_at_every_cut() {
    let mut volume = open(formatted(1024, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("tree", &[], now(2)).unwrap();
    for block in 0..130 {
        volume
            .preallocate_file(file, block * 3 * 4096, 4096, now(3))
            .unwrap();
    }
    let snapshot = volume.snapshot_create(now(4)).unwrap();
    let before = volume.stat(file).unwrap().unwrap();
    let old_layout = volume.load_file_layout(&before).unwrap().0;
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording
        .preallocate_file_bounded(
            file,
            196 * 4096,
            4096,
            now(5),
            FileEditLimits {
                max_blocks: 1,
                max_records: 4,
            },
        )
        .unwrap();
    let after = recording.stat(file).unwrap().unwrap();
    let new_layout = recording.load_file_layout(&after).unwrap().0;
    let (_, log) = recording.into_device().into_parts();
    let mut counts = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadOnly);
            let record = volume.stat(file).unwrap().unwrap();
            assert!(record == before || record == after);
            let published = record == after;
            counts[usize::from(published)] += 1;
            assert_eq!(
                volume.load_file_layout(&record).unwrap().0,
                if published {
                    new_layout.clone()
                } else {
                    old_layout.clone()
                }
            );
            let view = volume.snapshot_open(snapshot).unwrap();
            assert_eq!(
                volume
                    .snapshot_stat(&view, file)
                    .unwrap()
                    .unwrap()
                    .allocated_bytes,
                130 * 4096
            );
            let mut cursor = 0;
            loop {
                let page = volume
                    .snapshot_allocation_page(&view, file, cursor, 64)
                    .unwrap();
                for range in &page.ranges {
                    assert_eq!(range.offset, cursor * 3 * 4096);
                    assert_eq!(range.length, 4096);
                    assert!(range.unwritten);
                    cursor += 1;
                }
                if page.eof {
                    break;
                }
            }
            assert_eq!(cursor, 130);
        });
    }
    assert!(counts.iter().all(|n| *n > 0));
    println!(
        "bounded_reservation_cuts old={} new={}",
        counts[0], counts[1]
    );
}

#[test]
fn bounded_writes_preserve_fragmented_layout_and_reject_before_io() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(2048, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("bounded-write", &[], now(2))
        .unwrap();
    for block in 0..130 {
        volume
            .preallocate_file(file, block * 3 * 4096, 4096, now(3))
            .unwrap();
    }
    volume.truncate_file(file, 390 * 4096, now(4)).unwrap();
    let snapshot = volume.snapshot_create(now(5)).unwrap();
    let mut volume = open(
        TraceBackend::new(volume.into_device()),
        MountMode::ReadWrite,
    );
    let before = volume.stat(file).unwrap().unwrap();
    let io = volume.dev.stats();
    assert!(matches!(
        volume.write_file_at_bounded(
            file,
            0,
            &[7; 8192],
            now(6),
            FileEditLimits {
                max_blocks: 1,
                max_records: 8
            }
        ),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert!(matches!(
        volume.write_file_at_bounded(
            file,
            0,
            &[7; 4096],
            now(6),
            FileEditLimits {
                max_blocks: 1,
                max_records: 1
            }
        ),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert_eq!(volume.dev.stats().writes, io.writes);
    assert_eq!(volume.stat(file).unwrap().unwrap(), before);
    let mut expected = vec![0; 390 * 4096];
    for (offset, length, value) in [
        (195 * 4096 + 7, 5000, 0x53),
        (195 * 4096 + 10, 50, 0x71),
        (389 * 4096, 4096, 0x33),
    ] {
        let io = volume.dev.stats();
        volume
            .write_file_at_bounded(
                file,
                offset as u64,
                &vec![value; length],
                now(7),
                FileEditLimits {
                    max_blocks: 2,
                    max_records: 8,
                },
            )
            .unwrap();
        let reads = volume.dev.stats().reads - io.reads;
        assert!(reads < 180, "bounded write used {reads} reads");
        expected[offset..offset + length].fill(value);
        assert_eq!(volume.read_file(file).unwrap(), expected);
    }
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(bytes(&mut volume, &view, file), vec![0; 390 * 4096]);
    verify(&mut volume);
    let mut volume = open(volume.into_device().into_inner(), MountMode::ReadOnly);
    assert_eq!(volume.read_file(file).unwrap(), expected);
}

#[test]
fn bounded_writes_crash_to_exact_bytes_and_preserve_captured_zeros() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("cuts", &[], now(2)).unwrap();
    for block in [0, 3, 6] {
        volume
            .preallocate_file(file, block * 4096, 4096, now(3))
            .unwrap();
    }
    volume.truncate_file(file, 8 * 4096, now(4)).unwrap();
    let snapshot = volume.snapshot_create(now(5)).unwrap();
    let base = volume.into_device();
    let old = vec![0; 8 * 4096];
    let mut new = old.clone();
    new[7..5007].fill(0x5a);
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording
        .write_file_at_bounded(
            file,
            7,
            &[0x5a; 5000],
            now(6),
            FileEditLimits {
                max_blocks: 2,
                max_records: 8,
            },
        )
        .unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut counts = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadOnly);
            let actual = volume.read_file(file).unwrap();
            assert!(actual == old || actual == new);
            counts[usize::from(actual == new)] += 1;
            let view = volume.snapshot_open(snapshot).unwrap();
            assert_eq!(bytes(&mut volume, &view, file), old);
        });
    }
    assert!(counts.iter().all(|n| *n > 0));
    println!("bounded_write_cuts old={} new={}", counts[0], counts[1]);
}

#[test]
fn bounded_writes_keep_shared_peers_and_private_policy_semantics() {
    let mut volume = open(formatted(1024, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("source", &[0x11; 8192], now(2))
        .unwrap();
    let peer = volume
        .clone_file(file, OBJECT_ROOT, "peer", now(3))
        .unwrap();
    volume.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
    volume
        .write_file_at_bounded(
            file,
            7,
            &[0x33; 5000],
            now(4),
            FileEditLimits {
                max_blocks: 2,
                max_records: 8,
            },
        )
        .unwrap();
    assert_eq!(volume.read_file(peer).unwrap(), vec![0x11; 8192]);
    let mut expected = vec![0x11; 8192];
    expected[7..5007].fill(0x33);
    assert_eq!(volume.read_file(file).unwrap(), expected);
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        0
    );
    verify(&mut volume);
}

#[test]
fn bounded_writes_initialize_direct_files_and_keep_opted_in_private_tree_blocks() {
    let mut dev = MemoryBackend::new(4096, 512);
    crate::mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [81; 16],
            label: "BoundedPrivate".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: now(1),
        },
        crate::MkfsOptions {
            persistent_snapshots: false,
        },
    )
    .unwrap();
    let mut volume = open(dev, MountMode::ReadWrite);
    let file = volume.create_file_in_root("private", &[], now(2)).unwrap();
    let limits = FileEditLimits {
        max_blocks: 1,
        max_records: 8,
    };
    volume
        .write_file_at_bounded(file, 0, &[0x11; 4096], now(3), limits)
        .unwrap();
    volume
        .write_file_at_bounded(file, 8192, &[0x22; 4096], now(4), limits)
        .unwrap();
    let record = volume.stat(file).unwrap().unwrap();
    let before = volume.load_file_layout(&record).unwrap().0;
    volume.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
    volume
        .write_file_at_bounded(file, 7, b"private", now(5), limits)
        .unwrap();
    assert_eq!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        1
    );
    let record = volume.stat(file).unwrap().unwrap();
    assert_eq!(volume.load_file_layout(&record).unwrap().0, before);
    let mut expected = vec![0x11; 4096];
    expected.extend(vec![0; 4096]);
    expected.extend(vec![0x22; 4096]);
    expected[7..14].copy_from_slice(b"private");
    assert_eq!(volume.read_file(file).unwrap(), expected);
    verify(&mut volume);
}

#[test]
fn sparse_growth_preserves_fragmented_root_and_final_address_zeros() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(4096, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("grow", b"keep", now(2)).unwrap();
    for block in 1..300 {
        volume
            .preallocate_file(file, block * 3 * 4096, 4096, now(3))
            .unwrap();
    }
    let snapshot = volume.snapshot_create(now(4)).unwrap();
    let before = volume.stat(file).unwrap().unwrap();
    let mut volume = open(
        TraceBackend::new(volume.into_device()),
        MountMode::ReadWrite,
    );
    let io = volume.dev.stats();
    volume.truncate_file(file, u64::MAX, now(5)).unwrap();
    let reads = volume.dev.stats().reads - io.reads;
    assert!(reads < 100, "sparse growth used {reads} reads");
    let after = volume.stat(file).unwrap().unwrap();
    assert_eq!(after.data_root, before.data_root);
    assert_eq!(after.data_blocks, before.data_blocks);
    assert_eq!(after.allocated_bytes, before.allocated_bytes);
    assert_eq!(after.size_bytes, u64::MAX);
    assert!(after.content_generation > before.content_generation);
    let mut tail = [1; 16];
    assert_eq!(
        volume.read_file_at(file, u64::MAX - 16, &mut tail).unwrap(),
        16
    );
    assert_eq!(tail, [0; 16]);
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(bytes(&mut volume, &view, file), b"keep");
    verify(&mut volume);
    let mut volume = open(volume.into_device().into_inner(), MountMode::ReadOnly);
    assert_eq!(
        volume.read_file_at(file, u64::MAX - 16, &mut tail).unwrap(),
        16
    );
    assert_eq!(tail, [0; 16]);
    println!("sparse_growth records=300 reads={reads}");
}

#[test]
fn sparse_growth_publication_is_atomic_with_retained_reservations() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("growth-cuts", b"keep", now(2))
        .unwrap();
    volume.preallocate_file(file, 8192, 4096, now(3)).unwrap();
    let snapshot = volume.snapshot_create(now(4)).unwrap();
    let before = volume.stat(file).unwrap().unwrap();
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording.truncate_file(file, 16384, now(5)).unwrap();
    let after = recording.stat(file).unwrap().unwrap();
    let mut expanded = b"keep".to_vec();
    expanded.resize(16384, 0);
    let (_, log) = recording.into_device().into_parts();
    let mut counts = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadOnly);
            let record = volume.stat(file).unwrap().unwrap();
            assert!(record == before || record == after);
            let published = record == after;
            counts[usize::from(published)] += 1;
            assert_eq!(
                volume.read_file(file).unwrap(),
                if published {
                    expanded.clone()
                } else {
                    b"keep".to_vec()
                }
            );
            assert_eq!(record.data_root, before.data_root);
            assert_eq!(record.data_blocks, before.data_blocks);
            let view = volume.snapshot_open(snapshot).unwrap();
            assert_eq!(bytes(&mut volume, &view, file), b"keep");
            assert_eq!(
                volume
                    .snapshot_stat(&view, file)
                    .unwrap()
                    .unwrap()
                    .allocated_bytes,
                before.allocated_bytes
            );
        });
    }
    assert!(counts.iter().all(|n| *n > 0));
    println!("sparse_growth_cuts old={} new={}", counts[0], counts[1]);
}

#[test]
fn bounded_shrink_edits_fragmented_tail_and_refuses_oversized_retirement() {
    use afsplus_block::TraceBackend;
    let mut volume = open(formatted(4096, 0), MountMode::ReadWrite);
    let file = volume.create_file_in_root("shrink", &[], now(2)).unwrap();
    for block in 0..300 {
        volume
            .preallocate_file(file, block * 3 * 4096, 4096, now(3))
            .unwrap();
    }
    volume
        .write_file_at(file, 897 * 4096, &[0x55; 4096], now(4))
        .unwrap();
    volume.truncate_file(file, 900 * 4096, now(5)).unwrap();
    volume
        .preallocate_file(file, 903 * 4096, 4096, now(6))
        .unwrap();
    let snapshot = volume.snapshot_create(now(7)).unwrap();
    let before = volume.stat(file).unwrap().unwrap();
    let mut original = vec![0; 900 * 4096];
    original[897 * 4096..898 * 4096].fill(0x55);
    let mut volume = open(
        TraceBackend::new(volume.into_device()),
        MountMode::ReadWrite,
    );
    let io = volume.dev.stats();
    assert!(matches!(
        volume.truncate_file_bounded(
            file,
            0,
            now(8),
            FileEditLimits {
                max_blocks: 512,
                max_records: 4
            }
        ),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert!(matches!(
        volume.truncate_file_bounded(
            file,
            897 * 4096 + 7,
            now(8),
            FileEditLimits {
                max_blocks: 1,
                max_records: 4
            }
        ),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert_eq!(volume.dev.stats().writes, io.writes);
    assert_eq!(volume.stat(file).unwrap().unwrap(), before);
    let reads_before = volume.dev.stats().reads;
    volume
        .truncate_file_bounded(
            file,
            897 * 4096 + 7,
            now(8),
            FileEditLimits {
                max_blocks: 2,
                max_records: 4,
            },
        )
        .unwrap();
    let reads = volume.dev.stats().reads - reads_before;
    assert!(reads < 160, "bounded shrink used {reads} reads");
    assert_eq!(volume.stat(file).unwrap().unwrap().data_blocks, 300);
    assert_eq!(volume.read_file(file).unwrap(), original[..897 * 4096 + 7]);
    volume
        .truncate_file_bounded(
            file,
            900 * 4096,
            now(9),
            FileEditLimits {
                max_blocks: 1,
                max_records: 1,
            },
        )
        .unwrap();
    let mut expected = original.clone();
    expected[897 * 4096 + 7..].fill(0);
    assert_eq!(volume.read_file(file).unwrap(), expected);
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(bytes(&mut volume, &view, file), original);
    assert_eq!(
        volume
            .snapshot_stat(&view, file)
            .unwrap()
            .unwrap()
            .allocated_bytes,
        before.allocated_bytes
    );
    verify(&mut volume);
    let mut volume = open(volume.into_device().into_inner(), MountMode::ReadOnly);
    assert_eq!(volume.read_file(file).unwrap(), expected);
    println!("bounded_shrink records=301 reads={reads}");
}

#[test]
fn bounded_shrink_to_empty_tree_preserves_shared_peer() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("shared-tail", &[0x51; 8192], now(2))
        .unwrap();
    let peer = volume
        .clone_file(file, OBJECT_ROOT, "peer", now(3))
        .unwrap();
    volume
        .truncate_file_bounded(
            file,
            0,
            now(4),
            FileEditLimits {
                max_blocks: 2,
                max_records: 2,
            },
        )
        .unwrap();
    assert_eq!(volume.stat(file).unwrap().unwrap().data_blocks, 0);
    assert_eq!(volume.read_file(file).unwrap(), Vec::<u8>::new());
    assert_eq!(volume.read_file(peer).unwrap(), vec![0x51; 8192]);
    verify(&mut volume);
    let mut volume = open(volume.into_device(), MountMode::ReadOnly);
    assert_eq!(volume.read_file(file).unwrap(), Vec::<u8>::new());
    assert_eq!(volume.read_file(peer).unwrap(), vec![0x51; 8192]);
}

#[test]
fn bounded_shrink_crash_preserves_shared_and_captured_bytes() {
    let mut volume = open(formatted(512, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("cuts", &[0x55; 8192], now(2))
        .unwrap();
    volume.preallocate_file(file, 12288, 4096, now(3)).unwrap();
    let peer = volume
        .clone_file(file, OBJECT_ROOT, "peer", now(4))
        .unwrap();
    let snapshot = volume.snapshot_create(now(5)).unwrap();
    let before = volume.stat(file).unwrap().unwrap();
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording
        .truncate_file_bounded(
            file,
            7,
            now(6),
            FileEditLimits {
                max_blocks: 3,
                max_records: 4,
            },
        )
        .unwrap();
    let after = recording.stat(file).unwrap().unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut counts = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadOnly);
            let record = volume.stat(file).unwrap().unwrap();
            assert!(record == before || record == after);
            let published = record == after;
            counts[usize::from(published)] += 1;
            assert_eq!(
                volume.read_file(file).unwrap(),
                vec![0x55; if published { 7 } else { 8192 }]
            );
            assert_eq!(volume.read_file(peer).unwrap(), vec![0x55; 8192]);
            let view = volume.snapshot_open(snapshot).unwrap();
            assert_eq!(bytes(&mut volume, &view, file), vec![0x55; 8192]);
            assert_eq!(
                volume
                    .snapshot_stat(&view, file)
                    .unwrap()
                    .unwrap()
                    .allocated_bytes,
                before.allocated_bytes
            );
        });
    }
    assert!(counts.iter().all(|n| *n > 0));
    println!("bounded_shrink_cuts old={} new={}", counts[0], counts[1]);
}

#[test]
fn repeated_near_full_cycles_preserve_retained_views_and_recover_capacity() {
    use afsplus_block::TraceBackend;
    use std::time::Instant;
    const BS: usize = 4096;
    for scan_budget in [1, 8] {
        let mut volume = open(TraceBackend::new(formatted(512, 0)), MountMode::ReadWrite);
        let work = SnapshotWorkLimits {
            max_edit_records: 512,
            max_views: 4,
            reclaim_records: scan_budget,
        };
        volume.set_snapshot_work_limits(work).unwrap();
        volume.set_reclaim_batch_blocks(1);
        let original: Vec<u8> = (0..4 * BS).map(|i| (i % 251) as u8).collect();
        let keeper = volume
            .create_file_in_root("keeper", &original, now(2))
            .unwrap();
        let writer = volume
            .clone_file(keeper, OBJECT_ROOT, "writer", now(3))
            .unwrap();
        let retained = volume.snapshot_create(now(4)).unwrap();
        let mut expected = original.clone();
        let mut total_scanned = 0u64;
        let mut total_promoted = 0u64;
        let mut total_steps = 0u64;
        let mut max_allocator = 0u64;
        let mut admitted_writes = 0;
        let mut min_free = u64::MAX;
        let mut reads = 0u64;
        let mut writes = 0u64;
        let mut bytes_read = 0u64;
        let mut bytes_written = 0u64;
        let mut flushes = 0u64;
        let mut maintenance_ns = 0u128;
        for cycle in 0..24 {
            let t = now(10 + cycle);
            let rolling_bytes = expected.clone();
            let rolling = volume.snapshot_create(t).unwrap();
            let filler = volume.create_file_in_root("pressure", &[], t).unwrap();
            let reserve = volume.available_blocks().saturating_sub(32);
            assert!(
                reserve > 256,
                "budget={scan_budget} cycle={cycle} reserve={reserve}"
            );
            volume
                .preallocate_file(filler, 0, reserve * BS as u64, t)
                .unwrap();
            let free = volume.free_blocks();
            min_free = min_free.min(free);
            assert!(free <= volume.emergency_headroom_blocks() + 32);
            let generation = volume.generation();
            volume.device_mut().reset();
            assert!(matches!(
                volume.preallocate_file(filler, reserve * BS as u64, (free + 1) * BS as u64, t),
                Err(CoreError::NoSpace)
            ));
            let refused_io = volume.device_mut().stats();
            assert_eq!(refused_io.writes, 0);
            assert_eq!(refused_io.flushes, 0);
            assert_eq!(volume.generation(), generation);
            assert_eq!(volume.free_blocks(), free);
            let offset = (cycle as usize % 3 + 1) * BS - 7;
            let data = vec![cycle as u8; 29];
            match volume.write_file_at(writer, offset as u64, &data, t) {
                Ok(()) => {
                    expected[offset..offset + data.len()].copy_from_slice(&data);
                    admitted_writes += 1;
                }
                Err(CoreError::NoSpace) => assert_eq!(volume.generation(), generation),
                Err(error) => panic!("budget={scan_budget} cycle={cycle}: {error}"),
            }
            assert_eq!(volume.read_file(keeper).unwrap(), original);
            assert_eq!(volume.read_file(writer).unwrap(), expected);
            let historical = volume.snapshot_open(retained).unwrap();
            let recent = volume.snapshot_open(rolling).unwrap();
            assert_eq!(bytes(&mut volume, &historical, writer), original);
            assert_eq!(bytes(&mut volume, &recent, writer), rolling_bytes);
            assert!(matches!(
                volume.snapshot_delete(rolling, t),
                Err(CoreError::Busy)
            ));
            drop(recent);
            volume.snapshot_delete(rolling, t).unwrap();
            volume.delete_file_in_root("pressure", t).unwrap();
            volume.set_reclaim_batch_blocks(64);
            let mut recovered = false;
            volume.device_mut().reset();
            let started = Instant::now();
            for _ in 0..512 {
                let progress = volume.snapshot_maintenance_step(t).unwrap();
                total_steps += 1;
                total_scanned += progress.records_scanned;
                total_promoted += progress.blocks_promoted;
                assert!(progress.records_scanned <= scan_budget as u64);
                if let Some(stats) = volume.last_commit_stats() {
                    max_allocator = max_allocator.max(stats.alloc.allocator_ram_bytes);
                }
                if volume.available_blocks() > 384 {
                    recovered = true;
                    break;
                }
            }
            maintenance_ns += started.elapsed().as_nanos();
            let io = volume.device_mut().stats();
            reads += io.reads;
            writes += io.writes;
            bytes_read += io.bytes_read;
            bytes_written += io.bytes_written;
            flushes += io.flushes;
            assert!(
                recovered,
                "budget={scan_budget} cycle={cycle}: capacity did not recover"
            );
            assert_eq!(bytes(&mut volume, &historical, writer), original);
            drop(historical);
            verify(&mut volume); // Both selectable checkpoints, including their registries.
            volume = open(volume.into_device(), MountMode::ReadWrite);
            volume.set_snapshot_work_limits(work).unwrap();
            volume.set_reclaim_batch_blocks(1);
            assert_eq!(volume.read_file(writer).unwrap(), expected);
            let historical = volume.snapshot_open(retained).unwrap();
            assert_eq!(bytes(&mut volume, &historical, keeper), original);
            assert_eq!(bytes(&mut volume, &historical, writer), original);
            assert!(matches!(
                volume.snapshot_open(rolling),
                Err(CoreError::NotFound)
            ));
        }
        assert!(admitted_writes > 0);
        assert!(total_promoted > 24 * 256);
        assert!(max_allocator < 64 * 1024);
        println!("retained_pressure cycles=24 blocks=512 scan_budget={scan_budget} steps={total_steps} scanned={total_scanned} promoted={total_promoted} admitted_writes={admitted_writes} min_free={min_free} maintenance_allocator_bitmap_peak={max_allocator} maintenance_reads={reads} maintenance_writes={writes} maintenance_bytes_read={bytes_read} maintenance_bytes_written={bytes_written} maintenance_flushes={flushes} maintenance_ns={maintenance_ns}");
    }
}

#[test]
fn live_allocation_pages_are_read_only_and_distinct_from_captured_layouts() {
    use afsplus_block::TraceBackend;
    let mut volume = open(TraceBackend::new(formatted(512, 0)), MountMode::ReadWrite);
    let empty = volume.create_file_in_root("empty", &[], now(2)).unwrap();
    let file = volume
        .create_file_in_root("file", b"written", now(2))
        .unwrap();
    volume.device_mut().reset();
    assert!(volume
        .file_allocation_page(empty, 0, 1)
        .unwrap()
        .ranges
        .is_empty());
    assert_eq!(
        volume.file_allocation_page(file, 0, 1).unwrap().ranges,
        vec![FileAllocationRange {
            offset: 0,
            length: 4096,
            unwritten: false
        }]
    );
    assert_eq!(volume.device_mut().stats().writes, 0);
    assert_eq!(volume.device_mut().stats().flushes, 0);
    volume
        .preallocate_file(file, 4 * 4096, 8192, now(3))
        .unwrap();
    let id = volume.snapshot_create(now(4)).unwrap();
    let view = volume.snapshot_open(id).unwrap();
    let old = volume.snapshot_allocation_page(&view, file, 0, 64).unwrap();
    volume
        .write_file_at(file, 4 * 4096, b"changed", now(5))
        .unwrap();
    volume.device_mut().reset();
    let live = volume.file_allocation_page(file, 0, 64).unwrap();
    assert_eq!(live.ranges.len(), 3);
    assert!(!live.ranges[1].unwritten);
    assert!(live.ranges[2].unwritten);
    assert_eq!(
        volume.snapshot_allocation_page(&view, file, 0, 64).unwrap(),
        old
    );
    for limit in [0, 65] {
        assert!(volume.file_allocation_page(file, 0, limit).is_err());
    }
    assert!(volume.file_allocation_page(file, u64::MAX, 1).is_err());
    assert!(volume.file_allocation_page(empty, 1, 1).is_err());
    assert!(matches!(
        volume.file_allocation_page(OBJECT_ROOT, 0, 1),
        Err(CoreError::IsDirectory)
    ));
    assert_eq!(volume.device_mut().stats().writes, 0);
    assert_eq!(volume.device_mut().stats().flushes, 0);
}
#[test]
fn committed_allocation_readback_refuses_pending_windows_without_flushing() {
    use afsplus_block::TraceBackend;
    let mut volume = open(TraceBackend::new(formatted(1024, 16)), MountMode::ReadWrite);
    let file = volume.create_file_in_root("file", b"old", now(2)).unwrap();
    volume
        .window_write_file_at(file, 8192, b"pending", now(3))
        .unwrap();
    volume.device_mut().reset();
    let generation = volume.generation();
    assert!(matches!(
        volume.file_allocation_page(file, 0, 1),
        Err(CoreError::Busy)
    ));
    assert_eq!(volume.generation(), generation);
    assert_eq!(volume.device_mut().stats().writes, 0);
    assert_eq!(volume.device_mut().stats().flushes, 0);
    volume.window_commit(now(4)).unwrap();
    volume.device_mut().reset();
    let page = volume.file_allocation_page(file, 0, 64).unwrap();
    assert_eq!(page.ranges.len(), 2);
    assert_eq!(page.ranges[1].offset, 8192);
    assert_eq!(volume.device_mut().stats().writes, 0);
    assert_eq!(volume.device_mut().stats().flushes, 0);
}
