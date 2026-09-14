//! Family-specific cache-profile oracles; see tiny_cache_matrix.md for scope.
use afsplus_block::{
    for_each_crash_state, BlockDevice, FaultBackend, FaultPlan, MemoryBackend, RecordedOp,
    RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::volume::{
    BatchOp, FileEditLimits, ObjectMetadata, PreservedMetadata, SnapshotWorkLimits,
};
use afsplus_core::{
    mkfs_with_options, mount_with_snapshot_limits, CoreError, MkfsOptions, MkfsParams, MountMode,
    MountOptions, NamePolicy, Volume,
};
use afsplus_format::{object::ObjectRecord, Timespec, OBJECT_ROOT};
use std::{collections::BTreeMap, num::NonZeroUsize};

const PROFILES: [usize; 4] = [2, 4, 8, usize::MAX];
const OLD: &[u8] = b"stable bytes";
const TARGET: &[u8] = b"target bytes";
const LINK: &str = "../original";

fn time(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 123,
    }
}
fn wanted() -> PreservedMetadata {
    PreservedMetadata {
        protection: 0x8000_00ff,
        created: time(-3),
        modified: time(7),
        changed: time(9),
    }
}
fn open<D: BlockDevice>(dev: D, pages: usize) -> Volume<D> {
    open_mode(dev, pages, MountMode::ReadWrite)
}
fn open_mode<D: BlockDevice>(dev: D, pages: usize, mode: MountMode) -> Volume<D> {
    let volume = mount_with_snapshot_limits(
        dev,
        MountOptions {
            mode,
            tree_cache_pages: NonZeroUsize::new(pages),
        },
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 128,
            reclaim_records: 8,
        },
    )
    .unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    volume
}
fn formatted(blocks: u64) -> MemoryBackend {
    formatted_with_log(blocks, 0)
}
fn formatted_with_log(blocks: u64, log_slots: u16) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, blocks);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [0xc1; 16],
            label: "Family matrix".into(),
            region_size: blocks as u32,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(0),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    dev
}

#[derive(Clone, Copy, Debug)]
enum Family {
    Create,
    Mkdir,
    Rmdir,
    HardLink,
    Rename,
    Replace,
    Unlink,
    SymlinkCreate,
    SymlinkRename,
    SymlinkUnlink,
    Protection,
    Restore,
}
const FAMILIES: [Family; 12] = [
    Family::Create,
    Family::Mkdir,
    Family::Rmdir,
    Family::HardLink,
    Family::Rename,
    Family::Replace,
    Family::Unlink,
    Family::SymlinkCreate,
    Family::SymlinkRename,
    Family::SymlinkUnlink,
    Family::Protection,
    Family::Restore,
];
struct Fixture {
    file: u64,
    target: u64,
    dir: u64,
    link: u64,
    snapshot: u64,
    generation: u64,
    records: BTreeMap<u64, ObjectRecord>,
}
fn fixture(pages: usize) -> (MemoryBackend, Fixture) {
    let mut v = open(formatted(512), pages);
    let file = v.create_file_in_root("file", OLD, time(1)).unwrap();
    let target = v.create_file_in_root("target", TARGET, time(2)).unwrap();
    let dir = v.create_directory_in_root("dir", time(3)).unwrap();
    let link = v
        .create_symlink(OBJECT_ROOT, "link", LINK, time(4))
        .unwrap();
    let snapshot = v.snapshot_create(time(5)).unwrap();
    let records = [OBJECT_ROOT, file, target, dir, link]
        .into_iter()
        .map(|id| (id, v.stat(id).unwrap().unwrap()))
        .collect();
    let f = Fixture {
        file,
        target,
        dir,
        link,
        snapshot,
        generation: v.generation(),
        records,
    };
    (v.into_device(), f)
}
fn apply<D: BlockDevice>(v: &mut Volume<D>, f: &Fixture, family: Family) -> Result<(), CoreError> {
    match family {
        Family::Create => {
            v.create_file_in_root("new", b"new bytes", time(6))?;
        }
        Family::Mkdir => {
            v.create_directory_in_root("new", time(6))?;
        }
        Family::Rmdir => v.remove_directory(OBJECT_ROOT, "dir", time(6))?,
        Family::HardLink => v.link_file(f.file, OBJECT_ROOT, "new", time(6))?,
        Family::Rename => v.rename(OBJECT_ROOT, "file", f.dir, "new", time(6))?,
        Family::Replace => v.rename_replace(OBJECT_ROOT, "file", OBJECT_ROOT, "target", time(6))?,
        Family::Unlink => v.delete_file_in_root("file", time(6))?,
        Family::SymlinkCreate => {
            v.create_symlink(OBJECT_ROOT, "new", "SYS:Tools", time(6))?;
        }
        Family::SymlinkRename => v.rename(OBJECT_ROOT, "link", f.dir, "new", time(6))?,
        Family::SymlinkUnlink => v.unlink_symlink(OBJECT_ROOT, "link", time(6))?,
        Family::Protection => v.set_object_protection(f.file, 0x8000_00ff, time(6))?,
        Family::Restore => v.restore_object_metadata(f.file, wanted())?,
    }
    Ok(())
}
fn clean<D: BlockDevice>(dev: &mut D) {
    let report = check_device(dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    // A stopped intent-log tail is an allowed crash artifact. Older selectable
    // checkpoint findings are warnings too, and must not pass this oracle.
    assert!(
        report
            .warnings
            .iter()
            .all(|w| w.starts_with("intent log tail:")),
        "{:?}",
        report.warnings
    );
}
fn link_bytes<D: BlockDevice>(v: &mut Volume<D>, id: u64, expected: &[u8]) {
    let mut bytes = vec![0xa5; expected.len() + 1];
    assert_eq!(v.read_link(id, &mut bytes).unwrap(), expected.len());
    assert_eq!(&bytes[..expected.len()], expected);
    assert_eq!(bytes[expected.len()], 0xa5);
}
fn verify<D: BlockDevice>(v: &mut Volume<D>, f: &Fixture, family: Family, committed: bool) {
    assert_eq!(v.generation(), f.generation + u64::from(committed));
    let mut root: BTreeMap<String, u64> = [
        ("file".into(), f.file),
        ("target".into(), f.target),
        ("dir".into(), f.dir),
        ("link".into(), f.link),
    ]
    .into_iter()
    .collect();
    let mut directory = Vec::new();
    if committed {
        match family {
            Family::Create | Family::Mkdir | Family::SymlinkCreate => {
                let new = v.lookup_root("new").unwrap().expect("published entry");
                assert!(!f.records.contains_key(&new));
                root.insert("new".into(), new);
                match family {
                    Family::Create => assert_eq!(v.read_file(new).unwrap(), b"new bytes"),
                    Family::Mkdir => assert!(v.list_directory(new).unwrap().is_empty()),
                    Family::SymlinkCreate => link_bytes(v, new, b"SYS:Tools"),
                    _ => unreachable!(),
                }
            }
            Family::Rmdir => {
                root.remove("dir");
            }
            Family::HardLink => {
                root.insert("new".into(), f.file);
            }
            Family::Rename => {
                root.remove("file");
                directory.push(("new".into(), f.file));
            }
            Family::Replace => {
                root.remove("file");
                root.insert("target".into(), f.file);
            }
            Family::Unlink => {
                root.remove("file");
            }
            Family::SymlinkRename => {
                root.remove("link");
                directory.push(("new".into(), f.link));
            }
            Family::SymlinkUnlink => {
                root.remove("link");
            }
            Family::Protection | Family::Restore => {}
        }
    }
    assert_eq!(
        v.list_root()
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        root
    );
    if !(committed && matches!(family, Family::Rmdir)) {
        assert_eq!(v.list_directory(f.dir).unwrap(), directory);
    } else {
        assert!(v.stat(f.dir).unwrap().is_none());
    }
    if !(committed && matches!(family, Family::Unlink)) {
        assert_eq!(v.read_file(f.file).unwrap(), OLD);
        let record = v.stat(f.file).unwrap().unwrap();
        assert_eq!(
            record.link_count,
            if committed && matches!(family, Family::HardLink) {
                2
            } else {
                1
            }
        );
        if committed && matches!(family, Family::Restore) {
            assert_eq!(
                PreservedMetadata::from(ObjectMetadata::from(record)),
                wanted()
            );
        } else if committed && matches!(family, Family::Protection) {
            let old = f.records[&f.file];
            assert_eq!(record.protection, 0x8000_00ff);
            assert_eq!(record.created, old.created);
            assert_eq!(record.modified, old.modified);
            assert_eq!(record.changed, time(6));
        }
    } else {
        assert!(v.stat(f.file).unwrap().is_none());
    }
    if !(committed && matches!(family, Family::Replace)) {
        assert_eq!(v.read_file(f.target).unwrap(), TARGET);
    } else {
        assert!(v.stat(f.target).unwrap().is_none());
    }
    if !(committed && matches!(family, Family::SymlinkUnlink)) {
        link_bytes(v, f.link, LINK.as_bytes());
    } else {
        assert!(v.stat(f.link).unwrap().is_none());
    }
    if !committed {
        for (&id, record) in &f.records {
            assert_eq!(v.stat(id).unwrap().as_ref(), Some(record));
        }
    }
    let view = v.snapshot_open(f.snapshot).unwrap();
    for (&id, record) in &f.records {
        assert_eq!(
            ObjectMetadata::from(*record),
            v.snapshot_stat(&view, id).unwrap().unwrap()
        );
    }
    // Enumerate the complete captured namespace, not only known-name lookups.
    // The fixed page count and explicit EOF probe reject extra entries,
    // duplicates, premature EOF and a cursor that never terminates.
    let captured_root = [
        ("dir", f.dir),
        ("file", f.file),
        ("link", f.link),
        ("target", f.target),
    ];
    let mut cursor = None;
    for (index, expected) in captured_root.chunks(2).enumerate() {
        let page = v
            .snapshot_read_directory_page(&view, OBJECT_ROOT, cursor, 2)
            .unwrap();
        let actual: Vec<_> = page
            .entries
            .iter()
            .map(|entry| (entry.name.as_slice(), entry.child_id))
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|(name, id)| (name.as_bytes(), *id))
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(page.eof, index == 1);
        cursor = Some(page.next);
    }
    let end = v
        .snapshot_read_directory_page(&view, OBJECT_ROOT, cursor, 2)
        .unwrap();
    assert!(end.entries.is_empty() && end.eof);
    let child = v
        .snapshot_read_directory_page(&view, f.dir, None, 2)
        .unwrap();
    assert!(child.entries.is_empty() && child.eof);
    for (id, expected) in [(f.file, OLD), (f.target, TARGET)] {
        let mut bytes = vec![0xa5; expected.len() + 1];
        assert_eq!(
            v.snapshot_read_file_at(&view, id, 0, &mut bytes).unwrap(),
            expected.len()
        );
        assert_eq!(&bytes[..expected.len()], expected);
        assert_eq!(bytes[expected.len()], 0xa5);
    }
    let mut bytes = [0xa5; 64];
    assert_eq!(
        v.snapshot_read_link(&view, f.link, &mut bytes).unwrap(),
        LINK.len()
    );
    assert_eq!(&bytes[..LINK.len()], LINK.as_bytes());
    drop(view);
    clean(v.device_mut());
}

#[test]
fn namespace_and_metadata_preserve_exact_live_and_retained_state_in_all_profiles() {
    for pages in PROFILES {
        let (base, f) = fixture(pages);
        for family in FAMILIES {
            let mut v = open(base.clone(), pages);
            apply(&mut v, &f, family).unwrap();
            verify(&mut v, &f, family, true);
            let mut remounted = open(v.into_device(), pages);
            verify(&mut remounted, &f, family, true);
        }
    }
}

fn namespace_and_metadata_cut_matrix(pages: usize) {
    let (base, f) = fixture(pages);
    for family in FAMILIES {
        let mut v = open(RecordingBackend::new(base.clone()), pages);
        apply(&mut v, &f, family).unwrap();
        let (_, log) = v.into_device().into_parts();
        let mut outcomes = [0, 0];
        for cut in 0..=log.len() {
            for_each_crash_state(&base, &log, cut, |state| {
                let mut recovered = open(state.image, pages);
                let committed = recovered.generation() == f.generation + 1;
                verify(&mut recovered, &f, family, committed);
                outcomes[usize::from(committed)] += 1;
            });
        }
        assert!(outcomes.iter().all(|&n| n > 0));
        eprintln!("pages={pages} family={family:?} cuts old/new={outcomes:?}");
    }
}

#[test]
fn namespace_and_metadata_cuts_two_pages() {
    namespace_and_metadata_cut_matrix(2);
}

#[test]
fn namespace_and_metadata_cuts_four_pages() {
    namespace_and_metadata_cut_matrix(4);
}

#[test]
fn namespace_and_metadata_cuts_eight_pages() {
    namespace_and_metadata_cut_matrix(8);
}

#[test]
fn namespace_and_metadata_cuts_unlimited() {
    namespace_and_metadata_cut_matrix(usize::MAX);
}

#[test]
fn namespace_and_metadata_write_and_barrier_failures_preserve_ownership() {
    for pages in PROFILES {
        let (base, f) = fixture(pages);
        for family in FAMILIES {
            let mut reference = open(RecordingBackend::new(base.clone()), pages);
            apply(&mut reference, &f, family).unwrap();
            let (_, log) = reference.into_device().into_parts();
            let writes = log
                .iter()
                .filter(|op| matches!(op, RecordedOp::Write { .. }))
                .count() as u64;
            let flushes = log
                .iter()
                .filter(|op| matches!(op, RecordedOp::Flush))
                .count() as u64;
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
                let mut v = open(FaultBackend::new(base.clone(), plan), pages);
                assert!(
                    apply(&mut v, &f, family).is_err(),
                    "{pages} {family:?} {plan:?}"
                );
                assert!(v.device_mut().tripped());
                // A failed final barrier may have published the new checkpoint.
                let uncertain = plan.fail_flush_index == Some(flushes - 1);
                if uncertain {
                    assert!(matches!(
                        apply(&mut v, &f, family),
                        Err(CoreError::WindowPoisoned)
                    ));
                } else {
                    verify(&mut v, &f, family, false);
                    // A failed checkpoint write can require remount even though
                    // FaultBackend did not apply the failed write.
                    match apply(&mut v, &f, family) {
                        Ok(()) => verify(&mut v, &f, family, true),
                        Err(CoreError::WindowPoisoned) => {}
                        other => panic!("{pages} {family:?} {plan:?}: {other:?}"),
                    }
                }
                let mut recovered = open(v.into_device().into_inner(), pages);
                let committed = recovered.generation() == f.generation + 1;
                verify(&mut recovered, &f, family, committed);
                if !committed {
                    apply(&mut recovered, &f, family).unwrap();
                    verify(&mut recovered, &f, family, true);
                }
            }
            eprintln!(
                "pages={pages} family={family:?} injected writes={writes} barriers={flushes}"
            );
        }
    }
}

#[test]
fn namespace_and_metadata_refusals_issue_no_writes_and_preserve_retained_bytes() {
    for pages in PROFILES {
        let (base, f) = fixture(pages);
        let mut v = open(TraceBackend::new(base), pages);
        v.device_mut().reset();
        assert!(v.create_file_in_root("file", b"wrong", time(6)).is_err());
        assert!(v.create_directory_in_root("file", time(6)).is_err());
        assert!(v.link_file(f.file, OBJECT_ROOT, "file", time(6)).is_err());
        assert!(v
            .rename(OBJECT_ROOT, "file", OBJECT_ROOT, "target", time(6))
            .is_err());
        assert!(v.create_symlink(OBJECT_ROOT, "new", "", time(6)).is_err());
        assert!(v.unlink_symlink(OBJECT_ROOT, "file", time(6)).is_err());
        let mut invalid = wanted();
        invalid.modified.nanoseconds = 1_000_000_000;
        assert!(v.restore_object_metadata(f.file, invalid).is_err());
        let handle = v.snapshot_open(f.snapshot).unwrap();
        assert!(matches!(
            v.snapshot_delete(f.snapshot, time(6)),
            Err(CoreError::Busy)
        ));
        drop(handle);
        assert_eq!(v.device_mut().stats().writes, 0);
        assert_eq!(v.device_mut().stats().flushes, 0);
        verify(&mut v, &f, Family::Create, false);
        apply(&mut v, &f, Family::Create).unwrap();
        verify(&mut v, &f, Family::Create, true);
    }
}

#[test]
fn provisional_spill_failures_retry_at_two_four_and_eight_pages() {
    let names: Vec<_> = (0..192)
        .map(|i| format!("{i:04}-{}", "n".repeat(180)))
        .collect();
    let ops: Vec<_> = names
        .iter()
        .map(|name| BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name,
            content: b"payload",
        })
        .collect();
    for pages in [2, 4, 8] {
        let base = formatted(4096);
        let mut reference = open(base.clone(), pages);
        reference.run_batch(&ops, time(1)).unwrap();
        let stats = reference.last_commit_stats().unwrap().tree_mutations;
        assert!(stats.staged_spill_writes > 31);
        assert!(stats.max_resident_staged_nodes <= pages as u64);
        assert!(stats.max_staged_nodes_before_eviction <= pages as u64 + 1);
        for index in [0, 1, 7, 31] {
            let mut v = open(
                FaultBackend::new(
                    base.clone(),
                    FaultPlan {
                        fail_write_index: Some(index),
                        ..Default::default()
                    },
                ),
                pages,
            );
            let generation = v.generation();
            assert!(v.run_batch(&ops, time(1)).is_err());
            assert!(v.device_mut().tripped());
            assert_eq!(v.generation(), generation);
            assert!(v.list_root().unwrap().is_empty());
            v.run_batch(&ops, time(1)).unwrap();
            let mut remounted = open(v.into_device().into_inner(), pages);
            assert_eq!(remounted.list_root().unwrap().len(), names.len());
            for name in &names {
                let id = remounted.lookup_root(name).unwrap().unwrap();
                assert_eq!(remounted.read_file(id).unwrap(), b"payload");
            }
            clean(remounted.device_mut());
        }
    }
}

fn captured_bytes<D: BlockDevice>(v: &mut Volume<D>, snapshot: u64, file: u64, expected: &[u8]) {
    let handle = v.snapshot_open(snapshot).unwrap();
    let mut bytes = vec![0xa5; expected.len() + 1];
    assert_eq!(
        v.snapshot_read_file_at(&handle, file, 0, &mut bytes)
            .unwrap(),
        expected.len()
    );
    assert_eq!(&bytes[..expected.len()], expected);
    assert_eq!(bytes[expected.len()], 0xa5);
}

#[test]
fn bounded_reservation_refusal_preserves_layout_and_retries_in_all_profiles() {
    for pages in PROFILES {
        let mut v = open(TraceBackend::new(formatted(512)), pages);
        let file = v.create_file_in_root("file", b"", time(1)).unwrap();
        let limits = FileEditLimits {
            max_blocks: 1,
            max_records: 4,
        };
        for block in [6, 0, 3, 9] {
            v.preallocate_file_bounded(file, block * 4096, 4096, time(2), limits)
                .unwrap();
        }
        // Reservations do not change EOF; grow to expose their logical zeros.
        v.truncate_file(file, 10 * 4096, time(3)).unwrap();
        let snapshot = v.snapshot_create(time(4)).unwrap();
        let old = v.stat(file).unwrap().unwrap();
        let allocation = v.file_allocation_page(file, 0, 64).unwrap();
        let generation = v.generation();
        v.device_mut().reset();
        let refusal = v.preallocate_file_bounded(
            file,
            4 * 4096,
            4096,
            time(5),
            FileEditLimits {
                max_blocks: 1,
                max_records: 1,
            },
        );
        assert!(
            matches!(refusal, Err(CoreError::PrototypeLimit(_))),
            "pages={pages}: {refusal:?}"
        );
        assert_eq!(v.generation(), generation);
        assert_eq!(v.stat(file).unwrap(), Some(old));
        assert_eq!(v.file_allocation_page(file, 0, 64).unwrap(), allocation);
        assert_eq!(v.device_mut().stats().writes, 0);
        assert_eq!(v.device_mut().stats().flushes, 0);
        assert_eq!(v.read_file(file).unwrap(), vec![0; 10 * 4096]);
        captured_bytes(&mut v, snapshot, file, &vec![0; 10 * 4096]);
        v.preallocate_file_bounded(file, 4 * 4096, 4096, time(5), limits)
            .unwrap();
        assert_eq!(
            v.stat(file).unwrap().unwrap().data_blocks,
            old.data_blocks + 1
        );
        let mut remounted = open(v.into_device().into_inner(), pages);
        assert_eq!(remounted.read_file(file).unwrap(), vec![0; 10 * 4096]);
        captured_bytes(&mut remounted, snapshot, file, &vec![0; 10 * 4096]);
        clean(remounted.device_mut());
    }
}

#[test]
fn acknowledged_write_truncate_recovery_is_restartable_in_all_profiles() {
    for pages in PROFILES {
        let mut v = open(formatted_with_log(512, 8), pages);
        let old = vec![0x18; 4096 + 211];
        let file = v.create_file_in_root("file", &old, time(1)).unwrap();
        let snapshot = v.snapshot_create(time(2)).unwrap();
        let generation = v.generation();
        let mut expected = old.clone();
        expected[73..284].fill(0xc7);
        expected.truncate(307);
        v.window_write_file_at(file, 73, &[0xc7; 211], time(3))
            .unwrap();
        v.window_truncate_file(file, 307, time(4)).unwrap();
        v.window_fsync().unwrap();
        let logged = v.into_device();
        let reference = open_mode(
            RecordingBackend::new(logged.clone()),
            pages,
            MountMode::Recovery,
        );
        let (_, log) = reference.into_device().into_parts();
        assert!(!log.is_empty());
        let mut outcomes = [0, 0];
        for cut in 0..=log.len() {
            for_each_crash_state(&logged, &log, cut, |state| {
                let mut raw = open_mode(state.image, pages, MountMode::NoChanges);
                let published = raw.generation() == generation + 1;
                assert_eq!(raw.generation(), generation + u64::from(published));
                assert_eq!(raw.pending_intent_records() == 0, published);
                outcomes[usize::from(published)] += 1;
                clean(raw.device_mut());
                let mut recovered = open_mode(raw.into_device(), pages, MountMode::Recovery);
                assert_eq!(recovered.generation(), generation + 1);
                assert_eq!(recovered.pending_intent_records(), 0);
                assert_eq!(recovered.read_file(file).unwrap(), expected);
                captured_bytes(&mut recovered, snapshot, file, &old);
                clean(recovered.device_mut());
                let mut again = open_mode(recovered.into_device(), pages, MountMode::Recovery);
                assert_eq!(again.generation(), generation + 1);
                assert_eq!(again.read_file(file).unwrap(), expected);
                captured_bytes(&mut again, snapshot, file, &old);
            });
        }
        assert!(outcomes.iter().all(|&count| count > 0));
        eprintln!("pages={pages} write/truncate replay pre/post={outcomes:?}");
    }
}
