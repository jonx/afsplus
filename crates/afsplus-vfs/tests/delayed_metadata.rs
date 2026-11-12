//! Metadata edits join the open window (ADR-121 decision 8): on a delayed
//! mount, a mode, an owner, a modification time, a comment or an extended
//! attribute no longer commits the window and then a transaction of its own.
//! Copying a tree sets several of them on every file, so each used to cost
//! two commits per edit; now the population commits when the window does.

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, AttributeWriteMode, Durability, Vfs};

const FILES: usize = 200;

fn ms(millis: i64) -> Timespec {
    Timespec {
        seconds: 1_000 + millis / 1_000,
        nanoseconds: (millis % 1_000) as u32 * 1_000_000,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 16_384);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xE3; 16],
            label: "Metadata".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ms(0),
        },
    )
    .unwrap();
    device
}

/// One file as a copy makes it: created, written, then its mode, owner,
/// times, comment and an attribute.
fn copy_file<D: afsplus_block::BlockDevice>(vfs: &mut Vfs<D>, parent: u64, index: usize) -> u64 {
    let id = vfs
        .create_file(parent, &format!("f{index}"), ms(10))
        .unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, format!("file {index}").as_bytes(), ms(11))
        .unwrap();
    vfs.close(handle).unwrap();
    vfs.set_posix_mode(id, 0o640, ms(12)).unwrap();
    vfs.set_owner(id, Some(501), Some(20), ms(12)).unwrap();
    vfs.set_times(id, ms(-500_000), ms(13)).unwrap();
    vfs.set_comment(id, "copied", ms(14)).unwrap();
    vfs.set_attributes(
        id,
        &[("user.com.apple.provenance", Some(&[1, 2, 3][..]))],
        AttributeWriteMode::Upsert,
        ms(15),
    )
    .unwrap();
    id
}

fn check_copied<D: afsplus_block::BlockDevice>(vfs: &mut Vfs<D>, id: u64) {
    let stat = vfs.stat(id).unwrap();
    assert_eq!(stat.mode, 0o640);
    assert_eq!((stat.owner_uid, stat.owner_gid), (501, 20));
    assert_eq!(stat.modified, ms(-500_000));
    assert_eq!(vfs.comment(id).unwrap(), "copied");
    assert_eq!(
        vfs.attribute(id, "user.com.apple.provenance").unwrap(),
        Some(vec![1, 2, 3])
    );
}

#[test]
fn edits_wait_in_the_window_are_visible_at_once_and_survive_the_commit() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    vfs.sync_filesystem().unwrap();
    let start = vfs.generation();
    let id = copy_file(&mut vfs, OBJECT_ROOT, 0);
    // Nothing committed, everything visible.
    assert_eq!(vfs.generation(), start);
    assert!(vfs.changes_pending());
    check_copied(&mut vfs, id);
    // A second value replaces the one the window staged.
    vfs.set_attributes(
        id,
        &[("user.com.apple.provenance", Some(&[9][..]))],
        AttributeWriteMode::Upsert,
        ms(16),
    )
    .unwrap();
    assert_eq!(
        vfs.attribute(id, "user.com.apple.provenance").unwrap(),
        Some(vec![9])
    );
    vfs.sync_filesystem().unwrap();

    let mut device = vfs.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    let stat = vfs.stat(id).unwrap();
    assert_eq!(stat.mode, 0o640);
    assert_eq!(stat.modified, ms(-500_000));
    assert_eq!(
        vfs.attribute(id, "user.com.apple.provenance").unwrap(),
        Some(vec![9])
    );
}

/// The counter test. Two hundred copied files of five edits each cost, with
/// the edits committing on their own, two commits per edit: over 4,000
/// flushes. In the window they cost the flushes of the commits its bound and
/// the final sync make.
#[test]
fn copying_files_with_their_metadata_flushes_per_window_not_per_edit() {
    let mut vfs = Vfs::mount(TraceBackend::new(formatted()), MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    vfs.sync_filesystem().unwrap();
    let drawer = vfs.create_directory(OBJECT_ROOT, "copy", ms(1)).unwrap();
    let ids: Vec<u64> = (0..FILES)
        .map(|index| copy_file(&mut vfs, drawer, index))
        .collect();
    vfs.sync_filesystem().unwrap();
    for id in &ids {
        check_copied(&mut vfs, *id);
    }
    let flushes = vfs.into_volume().into_device().stats().flushes;
    // Eight window operations per file: 1,600 in all, three bound commits
    // and the sync, each two flushes, and the maintenance after them.
    eprintln!("{FILES} copied files: {flushes} flushes");
    assert!(flushes <= 24, "{flushes} flushes for {FILES} copied files");
}

/// A directory's modification time set after its entries changed, as
/// `rsync -a` sets it once the directory is filled, is the time it keeps:
/// the commit does not stamp its own over it.
#[test]
fn a_directory_time_set_after_its_entries_survives_the_commit() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    let drawer = vfs.create_directory(OBJECT_ROOT, "d", ms(1)).unwrap();
    vfs.create_file(drawer, "inside", ms(2)).unwrap();
    vfs.set_times(drawer, ms(-86_400_000), ms(3)).unwrap();
    assert_eq!(vfs.stat(drawer).unwrap().modified, ms(-86_400_000));
    vfs.sync_filesystem().unwrap();
    assert_eq!(vfs.stat(drawer).unwrap().modified, ms(-86_400_000));
    // An entry changed after the time was set moves it again.
    vfs.create_file(drawer, "later", ms(4)).unwrap();
    vfs.sync_filesystem().unwrap();
    assert_eq!(vfs.stat(drawer).unwrap().modified, ms(4));
}

/// The window has no log record for a metadata edit, so an fsync after one
/// commits the window (decision 5), and the edit is on the disk.
#[test]
fn an_fsync_after_a_staged_edit_commits_it() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    let id = vfs.create_file(OBJECT_ROOT, "kept", ms(1)).unwrap();
    vfs.sync_filesystem().unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.set_posix_mode(id, 0o600, ms(2)).unwrap();
    let before = vfs.generation();
    vfs.fsync(handle).unwrap();
    assert!(vfs.generation() > before);
    vfs.close(handle).unwrap();
    // No sync: the device as it is.
    let device = vfs.into_volume().into_device();
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    assert_eq!(vfs.stat(id).unwrap().mode, 0o600);
}

/// A sync mount keeps every edit durable when it returns.
#[test]
fn a_sync_mount_still_commits_each_edit_at_once() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let id = vfs.create_file(OBJECT_ROOT, "f", ms(1)).unwrap();
    let before = vfs.generation();
    vfs.set_posix_mode(id, 0o600, ms(2)).unwrap();
    assert!(vfs.generation() > before);
    assert!(!vfs.changes_pending());
}
