//! Integrated staged-tree cache profiles: exact state, spill accounting and
//! recovery. A staged-image limit is distinct from a total process RAM limit.
use afsplus_block::{
    for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{
    mkfs, mount_with_options, CoreError, MkfsParams, MountOptions, NamePolicy, Volume,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use std::num::NonZeroUsize;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}
fn options(pages: usize) -> MountOptions {
    MountOptions {
        tree_cache_pages: NonZeroUsize::new(pages),
        ..Default::default()
    }
}
fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 4096);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0xca; 16],
            label: "Cache".into(),
            region_size: 64,
            reclaim_caps: Default::default(),
            log_slots: 64,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}
fn names(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| format!("{i:04}-{}", "n".repeat(180)))
        .collect()
}
fn verify<D: BlockDevice>(
    volume: &mut Volume<D>,
    names: &[String],
    deleted_odd: bool,
    payload: &[u8],
) {
    let expected = names
        .iter()
        .enumerate()
        .filter(|(i, _)| !deleted_odd || i % 2 == 0)
        .count();
    assert_eq!(volume.list_root().unwrap().len(), expected);
    for (i, name) in names.iter().enumerate() {
        let id = volume.lookup_root(name).unwrap();
        if deleted_odd && i % 2 != 0 {
            assert!(id.is_none());
        } else {
            assert_eq!(volume.read_file(id.unwrap()).unwrap(), payload);
        }
    }
}
fn check_accounting(volume: &mut Volume<TraceBackend<MemoryBackend>>, pages: usize) -> u64 {
    let stats = volume.last_commit_stats().unwrap();
    let io = volume.device_mut().stats();
    assert_eq!(
        stats.bytes_written, io.bytes_written,
        "spill writes missing from commit bytes"
    );
    assert_eq!(stats.flushes, io.flushes);
    assert!(stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
    assert!(
        stats.tree_mutations.max_staged_nodes_before_eviction <= (pages as u64).saturating_add(1)
    );
    assert!(stats.tree_mutations.final_nodes_written > 0);
    println!(
        "pages={pages} writes={} reads={} spill={} reload={} staged_peak={}",
        io.writes,
        io.reads,
        stats.tree_mutations.staged_spill_writes,
        stats.tree_mutations.staged_spill_reloads,
        stats.tree_mutations.max_resident_staged_nodes
    );
    stats.tree_mutations.staged_spill_writes
}

#[test]
fn batch_create_delete_and_remount_match_at_two_four_eight_and_unlimited_pages() {
    let names = names(192);
    for pages in [2, 4, 8, usize::MAX] {
        let mut volume =
            mount_with_options(TraceBackend::new(formatted()), options(pages)).unwrap();
        assert_eq!(volume.tree_cache_pages(), pages);
        let operations: Vec<_> = names
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content: b"payload",
            })
            .collect();
        volume.device_mut().reset();
        volume.run_batch(&operations, ts(1)).unwrap();
        let spills = check_accounting(&mut volume, pages);
        if pages == usize::MAX {
            assert_eq!(spills, 0);
        } else {
            assert!(spills > 0);
        }
        verify(&mut volume, &names, false, b"payload");
        let deletions: Vec<_> = names
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 2 != 0)
            .map(|(_, name)| BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name,
            })
            .collect();
        volume.device_mut().reset();
        volume.run_batch(&deletions, ts(2)).unwrap();
        check_accounting(&mut volume, pages);
        verify(&mut volume, &names, true, b"payload");
        let mut dev = volume.into_device().into_inner();
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "{pages}: {:?}", report.errors);
        let mut remounted = mount_with_options(dev, options(pages)).unwrap();
        verify(&mut remounted, &names, true, b"payload");
        assert!(check_device(&mut remounted.into_device()).is_clean());
    }
}

#[test]
fn durable_window_recovery_honors_the_mount_profile() {
    let names = names(192);
    for pages in [2, 4, 8, usize::MAX] {
        let mut volume = mount_with_options(formatted(), options(pages)).unwrap();
        let generation = volume.generation();
        assert!(volume.set_tree_cache_pages(0).is_err());
        assert_eq!(volume.tree_cache_pages(), pages);
        for (i, name) in names.iter().enumerate() {
            volume
                .window_op(
                    &BatchOp::CreateFile {
                        parent_id: OBJECT_ROOT,
                        name,
                        content: b"",
                    },
                    ts(1),
                )
                .unwrap();
            assert!(matches!(
                volume.set_tree_cache_pages(1),
                Err(CoreError::Busy)
            ));
            if i % 4 == 3 {
                volume.window_fsync().unwrap();
            }
        }
        let dev = volume.into_device(); // Crash after fsync, before checkpoint.
        let mut volume = mount_with_options(TraceBackend::new(dev), options(pages)).unwrap();
        assert_eq!(volume.generation(), generation + 1);
        let spills = check_accounting(&mut volume, pages);
        if pages == usize::MAX {
            assert_eq!(spills, 0);
        } else {
            assert!(spills > 0);
        }
        verify(&mut volume, &names, false, b"");
        let mut dev = volume.into_device().into_inner();
        assert!(check_device(&mut dev).is_clean());
        let mut volume = mount_with_options(dev, options(pages)).unwrap();
        assert_eq!(
            volume.generation(),
            generation + 1,
            "replay must be idempotent"
        );
        verify(&mut volume, &names, false, b"");
    }
}

#[test]
fn failed_provisional_spills_leave_the_committed_view_and_allow_retry() {
    let names = names(192);
    let operations: Vec<_> = names
        .iter()
        .map(|name| BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name,
            content: b"",
        })
        .collect();
    // The successful recovery/batch fixtures establish more than 31 early
    // spill writes. These faults precede the metadata publication barrier.
    for index in [0, 1, 7, 31] {
        let dev = afsplus_block::FaultBackend::new(
            formatted(),
            afsplus_block::FaultPlan {
                fail_write_index: Some(index),
                ..Default::default()
            },
        );
        let mut volume = mount_with_options(dev, options(2)).unwrap();
        let generation = volume.generation();
        assert!(volume.run_batch(&operations, ts(1)).is_err());
        assert!(volume.device_mut().tripped());
        assert_eq!(volume.generation(), generation);
        assert!(volume.list_root().unwrap().is_empty());
        volume.run_batch(&operations, ts(1)).unwrap();
        verify(&mut volume, &names, false, b"");
        let mut dev = volume.into_device().into_inner();
        assert!(check_device(&mut dev).is_clean());
    }
}

#[test]
fn spilled_directory_split_is_atomic_at_every_modeled_cut() {
    let names = names(40);
    let mut base = formatted();
    let mut selected = None;
    for (i, name) in names.iter().enumerate() {
        let mut volume =
            mount_with_options(RecordingBackend::new(base.clone()), options(2)).unwrap();
        volume
            .create_file_in_root(name, b"", ts(i as i64 + 1))
            .unwrap();
        let spills = volume
            .last_commit_stats()
            .unwrap()
            .tree_mutations
            .staged_spill_writes;
        let (next, log) = volume.into_device().into_parts();
        if spills > 0 {
            selected = Some((i, log));
            break;
        }
        base = next;
    }
    let (index, log) = selected.expect("fixture must force a real staged-node spill");
    let mut before = 0;
    let mut after = 0;
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut dev = state.image;
            let report = check_device(&mut dev);
            assert!(
                report.is_clean(),
                "{}: {:?}",
                state.description,
                report.errors
            );
            let mut volume = mount_with_options(dev, options(2)).unwrap();
            let count = volume.list_root().unwrap().len();
            assert!(
                count == index || count == index + 1,
                "{}",
                state.description
            );
            verify(&mut volume, &names[..count], false, b"");
            if count == index {
                before += 1;
            } else {
                after += 1;
            }
        });
    }
    assert!(before > 0 && after > 0);
}
