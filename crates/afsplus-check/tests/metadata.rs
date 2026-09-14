use afsplus_block::{
    for_each_crash_state, BlockDevice, FaultBackend, FaultPlan, MemoryBackend, RecordingBackend,
    TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::volume::{
    DataUpdatePolicy, ObjectMetadata, PreservedMetadata, SnapshotWorkLimits,
};
use afsplus_core::{
    mkfs_with_options, mount_with_snapshot_limits, CoreError, MkfsOptions, MkfsParams, MountMode,
    MountOptions, NamePolicy, Volume,
};
use afsplus_format::{Timespec, OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 123,
    }
}
fn wanted() -> PreservedMetadata {
    PreservedMetadata {
        protection: 0x8000_00ff,
        created: Timespec {
            seconds: i64::MIN,
            nanoseconds: 0,
        },
        modified: Timespec {
            seconds: i64::MAX,
            nanoseconds: 999_999_999,
        },
        changed: time(-1234),
    }
}
fn formatted(snapshots: bool, log_slots: u16) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 512);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [83; 16],
            label: "Metadata".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
        MkfsOptions {
            persistent_snapshots: snapshots,
        },
    )
    .unwrap();
    dev
}
fn open<D: BlockDevice>(dev: D, mode: MountMode) -> Volume<D> {
    mount_with_snapshot_limits(
        dev,
        MountOptions { mode },
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 128,
            reclaim_records: 8,
        },
    )
    .unwrap()
}
fn preserved<D: BlockDevice>(volume: &mut Volume<D>, object: u64) -> PreservedMetadata {
    ObjectMetadata::from(volume.stat(object).unwrap().unwrap()).into()
}
fn clean<D: BlockDevice>(volume: Volume<D>) {
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn metadata_restore_preserves_layout_identity_and_captured_history() {
    for snapshots in [false, true] {
        let mut volume = open(formatted(snapshots, 0), MountMode::ReadWrite);
        let file = volume
            .create_file_in_root("file", &[0x39; 8192], time(2))
            .unwrap();
        volume
            .write_file_at(file, 16384, &[0x72; 4096], time(3))
            .unwrap();
        let mut expected_data = vec![0; 20480];
        expected_data[..8192].fill(0x39);
        expected_data[16384..].fill(0x72);
        let clone = volume
            .clone_file(file, OBJECT_ROOT, "clone", time(3))
            .unwrap();
        let old_clone = volume.stat(clone).unwrap().unwrap();
        volume
            .link_file(file, OBJECT_ROOT, "alias", time(3))
            .unwrap();
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, time(4))
            .unwrap();
        let dir = volume.create_directory_in_root("dir", time(5)).unwrap();
        let old = volume.stat(file).unwrap().unwrap();
        let snapshot = snapshots.then(|| volume.snapshot_create(time(6)).unwrap());
        let shared_root = volume.checkpoint().shared_extent_root_block;
        volume.restore_object_metadata(file, wanted()).unwrap();
        assert_eq!(volume.checkpoint().shared_extent_root_block, shared_root);
        let cost = volume.last_commit_stats().unwrap();
        eprintln!(
            "metadata restore snapshots={snapshots}: metadata_blocks={} bytes={} flushes={}",
            cost.metadata_blocks_written, cost.bytes_written, cost.flushes
        );
        let new = volume.stat(file).unwrap().unwrap();
        assert_eq!(
            new,
            afsplus_format::object::ObjectRecord {
                protection: wanted().protection,
                created: wanted().created,
                modified: wanted().modified,
                changed: wanted().changed,
                ..old
            }
        );
        assert_eq!(volume.last_commit_stats().unwrap().data_blocks_written, 0);
        assert_eq!(volume.lookup_root("alias").unwrap(), Some(file));
        volume.restore_object_metadata(dir, wanted()).unwrap();
        volume
            .restore_object_metadata(OBJECT_ROOT, wanted())
            .unwrap();
        let mut volume = open(volume.into_device(), MountMode::ReadWrite);
        for object in [file, dir, OBJECT_ROOT] {
            assert_eq!(preserved(&mut volume, object), wanted());
        }
        assert_eq!(volume.read_file(file).unwrap(), expected_data);
        assert_eq!(volume.read_file(clone).unwrap(), expected_data);
        assert_eq!(volume.stat(clone).unwrap().unwrap(), old_clone);
        if let Some(id) = snapshot {
            let view = volume.snapshot_open(id).unwrap();
            assert_eq!(
                volume.snapshot_stat(&view, file).unwrap().unwrap(),
                ObjectMetadata::from(old)
            );
            let mut bytes = vec![0; expected_data.len()];
            assert_eq!(
                volume
                    .snapshot_read_file_at(&view, file, 0, &mut bytes)
                    .unwrap(),
                expected_data.len()
            );
            assert_eq!(bytes, expected_data);
        }
        clean(volume);
    }
}

#[test]
fn ordinary_protection_changes_only_flags_and_change_time() {
    let mut volume = open(formatted(true, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"bytes", time(2))
        .unwrap();
    let old = volume.stat(file).unwrap().unwrap();
    volume
        .set_object_protection(file, 0xffff_ffff, time(3))
        .unwrap();
    assert_eq!(
        volume.stat(file).unwrap().unwrap(),
        afsplus_format::object::ObjectRecord {
            protection: 0xffff_ffff,
            changed: time(3),
            ..old
        }
    );
    let base = volume.into_device();
    let mut volume = open(TraceBackend::new(base), MountMode::ReadWrite);
    let same = preserved(&mut volume, file);
    volume.restore_object_metadata(file, same).unwrap();
    volume
        .set_object_protection(file, same.protection, time(99))
        .unwrap();
    assert_eq!(preserved(&mut volume, file), same);
    let trace = volume.into_device();
    assert_eq!(trace.stats().writes, 0);
    assert_eq!(trace.stats().flushes, 0);
}

#[test]
fn invalid_metadata_and_hidden_objects_refuse_before_writes() {
    let mut volume = open(formatted(true, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"bytes", time(2))
        .unwrap();
    let mut volume = open(
        TraceBackend::new(volume.into_device()),
        MountMode::ReadWrite,
    );
    let original = preserved(&mut volume, file);
    let generation = volume.generation();
    for field in 0..3 {
        let mut bad = wanted();
        let timestamp = match field {
            0 => &mut bad.created,
            1 => &mut bad.modified,
            _ => &mut bad.changed,
        };
        timestamp.nanoseconds = 1_000_000_000;
        assert!(matches!(
            volume.restore_object_metadata(file, bad),
            Err(CoreError::InvalidMetadata(_))
        ));
    }
    let bad_time = Timespec {
        seconds: 1,
        nanoseconds: u32::MAX,
    };
    assert!(matches!(
        volume.set_object_protection(file, 1, bad_time),
        Err(CoreError::InvalidMetadata(_))
    ));
    assert!(matches!(
        volume.set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, bad_time),
        Err(CoreError::InvalidMetadata(_))
    ));
    for object in [0, OBJECT_ORPHAN_DIRECTORY, u64::MAX] {
        assert!(matches!(
            volume.restore_object_metadata(object, wanted()),
            Err(CoreError::NotFound)
        ));
        assert!(matches!(
            volume.set_object_protection(object, 7, time(3)),
            Err(CoreError::NotFound)
        ));
    }
    assert_eq!(volume.generation(), generation);
    assert_eq!(preserved(&mut volume, file), original);
    let trace = volume.into_device();
    assert_eq!(trace.stats().writes, 0);
    assert_eq!(trace.stats().flushes, 0);
    for mode in [
        MountMode::ReadOnly,
        MountMode::NoChanges,
        MountMode::Recovery,
    ] {
        let mut volume = open(TraceBackend::new(trace.inner().clone()), mode);
        assert!(matches!(
            volume.restore_object_metadata(file, wanted()),
            Err(CoreError::ReadOnly)
        ));
        assert!(matches!(
            volume.set_object_protection(file, 7, time(3)),
            Err(CoreError::ReadOnly)
        ));
        let dev = volume.into_device();
        assert_eq!(dev.stats().writes, 0);
        assert_eq!(dev.stats().flushes, 0);
    }
}

#[test]
fn metadata_publication_cuts_preserve_exact_old_or_new_state_and_snapshot() {
    let mut volume = open(formatted(true, 0), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"stable", time(2))
        .unwrap();
    let old = preserved(&mut volume, file);
    let snapshot = volume.snapshot_create(time(3)).unwrap();
    let base = volume.into_device();
    let mut recording = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recording.restore_object_metadata(file, wanted()).unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut outcomes = [0, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image, MountMode::ReadWrite);
            let current = preserved(&mut volume, file);
            if current == old {
                outcomes[0] += 1;
            } else {
                assert_eq!(current, wanted());
                outcomes[1] += 1;
            }
            assert_eq!(volume.read_file(file).unwrap(), b"stable");
            let view = volume.snapshot_open(snapshot).unwrap();
            assert_eq!(
                PreservedMetadata::from(volume.snapshot_stat(&view, file).unwrap().unwrap()),
                old
            );
            drop(view);
            clean(volume);
        });
    }
    assert!(outcomes.iter().all(|&n| n > 0));
    eprintln!("metadata restore crash states old/new: {outcomes:?}");
}

#[test]
fn metadata_refuses_open_windows_and_uncertain_publication_requires_remount() {
    let mut volume = open(formatted(true, 8), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"before", time(2))
        .unwrap();
    volume
        .window_write_file_at(file, 0, b"after!", time(3))
        .unwrap();
    assert!(matches!(
        volume.restore_object_metadata(file, wanted()),
        Err(CoreError::WindowOpen)
    ));
    volume.window_commit(time(3)).unwrap();
    let base = volume.into_device();
    let mut recorded = open(RecordingBackend::new(base.clone()), MountMode::ReadWrite);
    recorded.restore_object_metadata(file, wanted()).unwrap();
    let flushes = recorded.last_commit_stats().unwrap().flushes;
    let mut failing = open(
        FaultBackend::new(
            base,
            FaultPlan {
                fail_flush_index: Some(flushes - 1),
                ..Default::default()
            },
        ),
        MountMode::ReadWrite,
    );
    assert!(failing.restore_object_metadata(file, wanted()).is_err());
    assert!(matches!(
        failing.restore_object_metadata(file, wanted()),
        Err(CoreError::WindowPoisoned)
    ));
    assert!(matches!(
        failing.set_object_protection(file, 7, time(4)),
        Err(CoreError::WindowPoisoned)
    ));
    let mut volume = open(failing.into_device().into_inner(), MountMode::ReadWrite);
    assert_eq!(preserved(&mut volume, file), wanted());
    assert_eq!(volume.read_file(file).unwrap(), b"after!");
    clean(volume);
}

#[test]
fn invalid_operation_times_never_enter_namespace_or_log_windows() {
    use afsplus_core::volume::BatchOp;
    let mut volume = open(formatted(true, 8), MountMode::ReadWrite);
    let file = volume
        .create_file_in_root("file", b"before", time(2))
        .unwrap();
    let snapshot = volume.snapshot_create(time(3)).unwrap();
    let mut volume = open(
        TraceBackend::new(volume.into_device()),
        MountMode::ReadWrite,
    );
    let bad = Timespec {
        seconds: 0,
        nanoseconds: 1_000_000_000,
    };
    let generation = volume.generation();
    macro_rules! rejected {
        ($call:expr) => {
            assert!(matches!($call, Err(CoreError::InvalidMetadata(_))));
        };
    }
    rejected!(volume.create_file_in_root("bad", b"data", bad));
    rejected!(volume.create_directory_in_root("bad-dir", bad));
    rejected!(volume.write_file_at(file, 0, b"bad", bad));
    rejected!(volume.truncate_file(file, 1, bad));
    rejected!(volume.clone_file(file, OBJECT_ROOT, "clone", bad));
    rejected!(volume.link_file(file, OBJECT_ROOT, "alias", bad));
    rejected!(volume.delete_file_in_root("file", bad));
    rejected!(volume.window_op(
        &BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "logged",
            content: b"x"
        },
        bad
    ));
    rejected!(volume.window_write_file_at(file, 0, b"bad", bad));
    rejected!(volume.window_truncate_file(file, 1, bad));
    rejected!(volume.window_commit(bad));
    rejected!(volume.snapshot_create(bad));
    rejected!(volume.snapshot_delete(snapshot, bad));
    rejected!(volume.snapshot_maintenance_step(bad));
    rejected!(volume.reclaim_step(bad));
    assert_eq!(volume.generation(), generation);
    assert_eq!(volume.read_file(file).unwrap(), b"before");
    assert_eq!(volume.lookup_root("bad").unwrap(), None);
    assert_eq!(volume.lookup_root("logged").unwrap(), None);
    let trace = volume.into_device();
    assert_eq!(trace.stats().writes, 0);
    assert_eq!(trace.stats().flushes, 0);
    let mut volume = open(trace.into_inner(), MountMode::ReadWrite);
    assert_eq!(volume.generation(), generation);
    assert_eq!(volume.pending_intent_records(), 0);
    volume.snapshot_open(snapshot).unwrap();
    volume
        .create_file_in_root("valid", b"works", time(4))
        .unwrap();
    clean(volume);
}
