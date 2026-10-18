//! Snapshot feature admission through the filesystem-neutral mount boundary.
use afsplus_block::MemoryBackend;
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{
    mkfs_with_options, MkfsOptions, MkfsParams, MountMode, MountOptions, NamePolicy,
};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

fn limits() -> SnapshotWorkLimits {
    SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 64,
    }
}
fn image() -> MemoryBackend {
    let mut image = MemoryBackend::new(4096, 4096);
    mkfs_with_options(
        &mut image,
        &MkfsParams {
            uuid: [0x53; 16],
            label: "Snapshots".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec::default(),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    image
}
#[test]
fn explicit_budgets_admit_snapshots_preserve_old_bytes_and_refuse_busy_delete() {
    assert!(Vfs::mount(image(), MountOptions::default()).is_err());
    let vfs = Vfs::mount_with_snapshot_limits(image(), MountOptions::default(), limits()).unwrap();
    let mut volume = vfs.into_volume();
    let file = volume
        .create_file_in_root("prefs", b"boot", Timespec::default())
        .unwrap();
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let view = volume.snapshot_open(snapshot).unwrap();
    assert!(volume
        .snapshot_delete(snapshot, Timespec::default())
        .is_err());
    drop(view);
    let image = volume.into_device();
    let vfs = Vfs::mount_with_snapshot_limits(
        image,
        MountOptions {
            mode: MountMode::ReadOnly,
            ..Default::default()
        },
        limits(),
    )
    .unwrap();
    let mut volume = vfs.into_volume();
    let view = volume.snapshot_open(snapshot).unwrap();
    let mut bytes = [0; 4];
    assert_eq!(
        volume
            .snapshot_read_file_at(&view, file, 0, &mut bytes)
            .unwrap(),
        4
    );
    assert_eq!(bytes, *b"boot");
    assert!(volume.snapshot_create(Timespec::default()).is_err());
}
#[test]
fn invalid_budgets_are_refused() {
    let mut bad = limits();
    bad.reclaim_records = 4097;
    assert!(Vfs::mount_with_snapshot_limits(image(), MountOptions::default(), bad).is_err());
    let mut bad = limits();
    bad.max_views = 0;
    assert!(Vfs::mount_with_snapshot_limits(image(), MountOptions::default(), bad).is_err());
}

#[test]
fn delayed_capture_keeps_pending_bytes_and_retries_a_failed_directory_page() {
    use afsplus_vfs::{AccessMode, Durability, VfsError};
    let mut vfs =
        Vfs::mount_with_snapshot_limits(image(), MountOptions::default(), limits()).unwrap();
    vfs.set_durability(Durability::Delayed {
        idle_ms: 1000,
        max_age_ms: 5000,
    })
    .unwrap();
    let object = vfs.create_file(1, "prefs", Timespec::default()).unwrap();
    let file = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    vfs.write(file, 0, b"boot", Timespec::default()).unwrap();
    assert!(vfs.changes_pending());
    let id = vfs.snapshot_create(Timespec::default()).unwrap();
    let reader = vfs.snapshot_open(id, 1).unwrap();
    assert!(vfs.snapshot_open(id, 1).is_err());
    vfs.write(file, 0, b"live", Timespec::default()).unwrap();
    let mut bytes = [0; 4];
    assert_eq!(vfs.snapshot_read(reader, object, 0, &mut bytes).unwrap(), 4);
    assert_eq!(&bytes, b"boot");
    assert!(vfs.snapshot_delete(id, Timespec::default()).is_err());
    let dir = vfs.snapshot_directory_open(reader, 1, 1).unwrap();
    assert!(vfs
        .snapshot_directory_read_encoded(dir, 1, |_| Err(VfsError::Invalid))
        .is_err());
    let page = vfs.snapshot_directory_read(dir, 1).unwrap();
    assert_eq!(page.entries[0].name, b"prefs");
    vfs.snapshot_close(reader).unwrap();
    assert!(vfs.snapshot_directory_read(dir, 1).is_err());
    assert!(vfs.snapshot_read(reader, object, 0, &mut bytes).is_err());
    vfs.close(file).unwrap();
    vfs.snapshot_delete(id, Timespec::default()).unwrap();
}
