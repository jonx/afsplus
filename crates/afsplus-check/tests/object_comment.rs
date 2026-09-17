//! The stored comment of an object (ADR-106): durable, carried by every
//! rewrite of its record, copied by a clone, captured by a snapshot, and
//! either old or new after a power cut.
use afsplus_block::{for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::volume::{BatchOp, DataUpdatePolicy, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_options, mkfs_with_security_descriptors, mount, mount_with_snapshot_limits,
    CoreError, MkfsOptions, MkfsParams, MountOptions, NamePolicy, Volume,
};
use afsplus_format::{Timespec, OBJECT_ROOT};

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 9,
    }
}

fn params() -> MkfsParams {
    MkfsParams {
        uuid: [0xc0; 16],
        label: "Comments".into(),
        region_size: 512,
        reclaim_caps: Default::default(),
        log_slots: 4,
        shared_extents: true,
        data_policy: true,
        name_policy: NamePolicy::Sensitive,
        timestamp: time(1),
    }
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_security_descriptors(&mut dev, &params()).unwrap();
    dev
}

fn checked<D: BlockDevice>(volume: Volume<D>) -> D {
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    dev
}

#[test]
fn a_comment_survives_every_rewrite_of_its_object_and_a_clone_copies_it() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &[1; 6000], time(2))
        .unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(2))
        .unwrap();
    let link = volume
        .create_symlink(OBJECT_ROOT, "link", "dir/target", time(2))
        .unwrap();
    assert_eq!(volume.object_comment(file).unwrap(), "");
    volume
        .set_object_comment(file, "Résumé 1992 🜁", time(3))
        .unwrap();
    volume.set_object_comment(dir, "a drawer", time(3)).unwrap();
    volume.set_object_comment(link, "a link", time(3)).unwrap();
    volume
        .set_object_comment(OBJECT_ROOT, "the root", time(3))
        .unwrap();
    // The change time moves, the modification time does not.
    let stat = volume.stat(file).unwrap().unwrap();
    assert_eq!((stat.modified, stat.changed), (time(2), time(3)));

    // Every rewrite path of the file's record, with a descriptor beside it.
    volume
        .set_security_descriptor(file, 0x7fff_0001, 1, b"descriptor", time(4))
        .unwrap();
    volume
        .write_file_at(file, 40_000, &[2; 5000], time(4))
        .unwrap();
    volume.truncate_file(file, 9000, time(4)).unwrap();
    volume
        .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, time(4))
        .unwrap();
    volume.link_file(file, dir, "alias", time(4)).unwrap();
    volume
        .rename(OBJECT_ROOT, "file", dir, "moved", time(4))
        .unwrap();
    volume
        .run_batch(
            &[BatchOp::DeleteFile {
                parent_id: dir,
                name: "alias",
            }],
            time(4),
        )
        .unwrap();
    volume
        .window_write_file_at(file, 0, &[3; 100], time(5))
        .unwrap();
    volume.window_commit(time(5)).unwrap();
    volume
        .create_file_in_directory(dir, "child", b"", time(5))
        .unwrap();
    volume
        .rename(OBJECT_ROOT, "link", OBJECT_ROOT, "link2", time(5))
        .unwrap();
    let clone = volume
        .clone_file(file, OBJECT_ROOT, "clone", time(6))
        .unwrap();

    let mut volume = mount(checked(volume)).unwrap();
    assert_eq!(volume.object_comment(file).unwrap(), "Résumé 1992 🜁");
    assert_eq!(volume.object_comment(dir).unwrap(), "a drawer");
    assert_eq!(volume.object_comment(link).unwrap(), "a link");
    assert_eq!(volume.object_comment(OBJECT_ROOT).unwrap(), "the root");
    // The clone carries the comment; afterwards the two are independent.
    assert_eq!(volume.object_comment(clone).unwrap(), "Résumé 1992 🜁");
    volume.set_object_comment(clone, "", time(7)).unwrap();
    assert_eq!(volume.object_comment(clone).unwrap(), "");
    assert_eq!(volume.object_comment(file).unwrap(), "Résumé 1992 🜁");
    let mut target = [0u8; 32];
    let length = volume.read_link(link, &mut target).unwrap();
    assert_eq!(&target[..length], b"dir/target");
    assert_eq!(
        volume.security_descriptor(file).unwrap().unwrap().bytes,
        b"descriptor"
    );
    checked(volume);
}

#[test]
fn bounds_no_ops_and_refusals() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    let longest = "é".repeat(127) + "x";
    volume.set_object_comment(file, &longest, time(3)).unwrap();
    assert_eq!(volume.object_comment(file).unwrap(), longest);
    let generation = volume.checkpoint().generation;
    volume.set_object_comment(file, &longest, time(4)).unwrap();
    assert_eq!(volume.checkpoint().generation, generation);
    for bad in ["x".repeat(256), "nul\0inside".to_owned()] {
        assert!(matches!(
            volume.set_object_comment(file, &bad, time(4)),
            Err(CoreError::InvalidMetadata(_))
        ));
    }
    assert!(matches!(
        volume.set_object_comment(999, "x", time(4)),
        Err(CoreError::NotFound)
    ));
    assert_eq!(volume.object_comment(file).unwrap(), longest);
    assert_eq!(volume.checkpoint().generation, generation);
    checked(volume);
}

#[test]
fn a_snapshot_keeps_the_comment_it_captured() {
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            log_slots: 0,
            ..params()
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let mut volume = mount_with_snapshot_limits(dev, MountOptions::default(), limits).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    volume
        .set_object_comment(file, "captured", time(3))
        .unwrap();
    let snapshot = volume.snapshot_create(time(4)).unwrap();
    volume.set_object_comment(file, "live", time(5)).unwrap();
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(
        volume
            .snapshot_object_comment(&view, file)
            .unwrap()
            .as_deref(),
        Some("captured")
    );
    drop(view);
    assert_eq!(volume.object_comment(file).unwrap(), "live");
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
}

#[test]
fn every_power_cut_leaves_the_old_comment_or_the_new_one() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"stable", time(2))
        .unwrap();
    volume.set_object_comment(file, "before", time(3)).unwrap();
    let base = volume.into_device();
    for (next, expected_after) in [
        (
            "after, and longer than before",
            "after, and longer than before",
        ),
        ("", ""),
    ] {
        let mut recording = mount(RecordingBackend::new(base.clone())).unwrap();
        recording.set_object_comment(file, next, time(4)).unwrap();
        let (_, log) = recording.into_device().into_parts();
        let mut outcomes = [0u32, 0];
        for cut in 0..=log.len() {
            for_each_crash_state(&base, &log, cut, |state| {
                let mut volume = mount(state.image).unwrap();
                let comment = volume.object_comment(file).unwrap();
                if comment == "before" {
                    outcomes[0] += 1;
                } else {
                    assert_eq!(comment, expected_after);
                    outcomes[1] += 1;
                }
                assert_eq!(volume.read_file(file).unwrap(), b"stable");
                checked(volume);
            });
        }
        assert!(outcomes.iter().all(|count| *count > 0), "{outcomes:?}");
        eprintln!("comment {next:?} crash states old/new: {outcomes:?}");
    }
}
