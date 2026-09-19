//! A directory joins the open window as a file does (ADR-121 decision 8):
//! `CreateDir` no longer commits the window and then a transaction of its
//! own. A directory made this way is usable at once, and a cut before the
//! commit loses the whole group, never a part of it.

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{
    AccessMode, Durability, NodeKind, ObjectId, Vfs, VfsError, DELAYED_WINDOW_OPS_MAX,
};

const DRAWERS: usize = 90;
const FILES: usize = 32;

fn ms(millis: i64) -> Timespec {
    Timespec {
        seconds: 1_000 + millis / 1_000,
        nanoseconds: (millis % 1_000) as u32 * 1_000_000,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 65_536);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xD1; 16],
            label: "Drawers".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: ms(0),
        },
    )
    .unwrap();
    device
}

fn delayed(device: MemoryBackend) -> Vfs<MemoryBackend> {
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    vfs
}

fn remount(vfs: Vfs<MemoryBackend>) -> Vfs<MemoryBackend> {
    let mut device = vfs.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    Vfs::mount(device, MountOptions::default()).unwrap()
}

fn names(vfs: &mut Vfs<MemoryBackend>, directory: ObjectId) -> Vec<String> {
    let handle = vfs.open_directory(directory).unwrap();
    let mut collected = Vec::new();
    let mut cookie = 0;
    loop {
        let page = vfs.read_directory(handle, cookie, 64).unwrap();
        for entry in &page.entries {
            collected.push(String::from_utf8(entry.name.clone()).unwrap());
        }
        cookie = page.next_cookie;
        if page.eof {
            break;
        }
    }
    vfs.close(handle).unwrap();
    collected.sort();
    collected
}

/// The counter test of lot I. Before it, each of the 90 drawers cost four
/// flushes: the open window was committed and `create_directory` then ran a
/// transaction of its own. Now the drawers are operations of the window like
/// the files, so the whole phase costs the flushes of the commits the window
/// bound and the final sync make, and nothing per drawer.
#[test]
fn drawers_full_of_files_flush_what_a_few_commits_flush() {
    let device = TraceBackend::new(formatted());
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    vfs.sync_filesystem().unwrap();
    let start = vfs.generation();
    for drawer in 0..DRAWERS {
        let id = vfs
            .create_directory(OBJECT_ROOT, &format!("d{drawer}"), ms(0))
            .unwrap();
        for file in 0..FILES {
            vfs.create_file(id, &format!("f{file}"), ms(0)).unwrap();
        }
    }
    vfs.sync_filesystem().unwrap();
    let commits = vfs.generation() - start;
    let flushes = vfs.into_volume().into_device().stats().flushes;

    // 90 drawers and 2,880 files are 2,970 window operations: the window
    // bound commits every 512 of them, five times, and the final sync
    // commits once more. Six commits of two flushes each. The maintenance a
    // commit runs afterwards is a transaction of its own, so allow twice
    // that: measured, 18 flushes over 9 generations. The old code committed
    // the window and then a transaction per drawer, four flushes each, 360
    // for the drawers alone, and this assertion fails on it.
    let operations = (DRAWERS + DRAWERS * FILES) as u32;
    let expected_commits = operations / DELAYED_WINDOW_OPS_MAX + 1;
    eprintln!(
        "{DRAWERS} drawers of {FILES} files: {operations} operations, \
         {commits} generations, {flushes} flushes, {expected_commits} commits expected"
    );
    assert!(
        flushes <= u64::from(4 * expected_commits),
        "{flushes} flushes for {expected_commits} commits of two flushes"
    );
    assert!(
        flushes < DRAWERS as u64,
        "{flushes} flushes is still a flush per drawer or more"
    );
}

/// Everything a drawer must answer while it is still only in the window, and
/// the same answers from the committed volume afterwards.
#[test]
fn a_drawer_made_in_the_window_is_usable_before_the_commit() {
    let mut vfs = delayed(formatted());
    vfs.sync_filesystem().unwrap();
    let start = vfs.generation();
    let drawer = vfs.create_directory(OBJECT_ROOT, "work", ms(10)).unwrap();
    let file = vfs.create_file(drawer, "note", ms(20)).unwrap();
    let handle = vfs.open_file(file, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, b"inside", ms(20)).unwrap();
    vfs.close(handle).unwrap();
    // Nothing has been committed: the drawer exists only in the window.
    assert_eq!(vfs.generation(), start);
    assert!(vfs.changes_pending());

    assert_eq!(vfs.lookup(OBJECT_ROOT, "work").unwrap(), drawer);
    assert_eq!(vfs.lookup(drawer, "note").unwrap(), file);
    let stat = vfs.stat(drawer).unwrap();
    assert_eq!(stat.kind, NodeKind::Directory);
    assert_eq!(stat.links, 1);
    assert_eq!(stat.size, 0);
    assert_eq!(stat.created, ms(10));
    // The parent's own stat: the immediate path moves its modified time to
    // the moment of the create, and so does the window.
    let parent = vfs.stat(OBJECT_ROOT).unwrap();
    assert_eq!(parent.kind, NodeKind::Directory);
    assert_eq!(vfs.stat(file).unwrap().kind, NodeKind::File);
    // A read of either directory commits the window first, as it does for a
    // directory whose entries wait: the entry trees are written by the
    // commit. One commit answers both listings.
    assert_eq!(names(&mut vfs, OBJECT_ROOT), ["work"]);
    assert!(vfs.generation() > start, "the listing committed the window");
    assert_eq!(names(&mut vfs, drawer), ["note"]);
    assert!(!vfs.changes_pending());

    vfs.sync_filesystem().unwrap();
    let mut vfs = remount(vfs);
    assert_eq!(vfs.lookup(OBJECT_ROOT, "work").unwrap(), drawer);
    assert_eq!(vfs.lookup(drawer, "note").unwrap(), file);
    let stat = vfs.stat(drawer).unwrap();
    assert_eq!(stat.kind, NodeKind::Directory);
    assert_eq!(stat.links, 1);
    assert_eq!(stat.created, ms(10));
    assert_eq!(names(&mut vfs, OBJECT_ROOT), ["work"]);
    assert_eq!(names(&mut vfs, drawer), ["note"]);
    let handle = vfs.open_file(file, AccessMode::ReadOnly).unwrap();
    let mut buffer = [0u8; 16];
    let count = vfs.read(handle, 0, &mut buffer).unwrap();
    assert_eq!(&buffer[..count], b"inside");
    vfs.close(handle).unwrap();
}

/// A file renamed into a drawer the window made, in the same window.
#[test]
fn a_file_renames_into_a_drawer_the_window_made() {
    let mut vfs = delayed(formatted());
    let moved = vfs.create_file(OBJECT_ROOT, "loose", ms(0)).unwrap();
    let drawer = vfs.create_directory(OBJECT_ROOT, "tidy", ms(10)).unwrap();
    vfs.rename(OBJECT_ROOT, "loose", drawer, "kept", false, ms(20))
        .unwrap();
    assert_eq!(vfs.lookup(drawer, "kept").unwrap(), moved);
    assert!(matches!(
        vfs.lookup(OBJECT_ROOT, "loose"),
        Err(VfsError::NotFound)
    ));
    vfs.sync_filesystem().unwrap();
    let mut vfs = remount(vfs);
    assert_eq!(vfs.lookup(drawer, "kept").unwrap(), moved);
    assert_eq!(names(&mut vfs, drawer), ["kept"]);
    assert_eq!(names(&mut vfs, OBJECT_ROOT), ["tidy"]);
}

/// Removing a drawer the window made is not a window operation: the
/// immediate path commits the window first and then removes it. That is
/// correct, only slower, and it is what ADR-121 decision 8 allows.
#[test]
fn removing_a_drawer_the_window_made_commits_it_first() {
    let mut vfs = delayed(formatted());
    vfs.sync_filesystem().unwrap();
    let start = vfs.generation();
    let drawer = vfs.create_directory(OBJECT_ROOT, "gone", ms(0)).unwrap();
    assert_eq!(vfs.generation(), start);
    vfs.remove_directory(OBJECT_ROOT, "gone", ms(10)).unwrap();
    assert!(vfs.generation() > start);
    assert!(matches!(vfs.stat(drawer), Err(VfsError::NotFound)));
    assert!(names(&mut vfs, OBJECT_ROOT).is_empty());
    let mut vfs = remount(vfs);
    assert!(names(&mut vfs, OBJECT_ROOT).is_empty());
}

/// A cut inside the window: the whole group is lost, never a part of it, and
/// the image is clean. The committed prefix before it stays.
#[test]
fn a_cut_inside_the_window_loses_the_whole_group_of_drawers() {
    let mut vfs = delayed(formatted());
    let kept = vfs.create_directory(OBJECT_ROOT, "kept", ms(0)).unwrap();
    vfs.create_file(kept, "inside", ms(10)).unwrap();
    vfs.sync_filesystem().unwrap();

    // A second group, and a third nested inside it, none of it committed.
    let lost = vfs
        .create_directory(OBJECT_ROOT, "lost", ms(1_000))
        .unwrap();
    let nested = vfs.create_directory(lost, "deeper", ms(1_010)).unwrap();
    for file in 0..8 {
        vfs.create_file(nested, &format!("f{file}"), ms(1_020))
            .unwrap();
    }
    vfs.create_file(kept, "also-lost", ms(1_030)).unwrap();
    assert!(vfs.changes_pending());

    // Crash: the device is taken as it stands, with the window unwritten.
    let mut vfs = remount(vfs);
    assert_eq!(vfs.lookup(OBJECT_ROOT, "kept").unwrap(), kept);
    assert_eq!(names(&mut vfs, kept), ["inside"]);
    assert!(matches!(
        vfs.lookup(OBJECT_ROOT, "lost"),
        Err(VfsError::NotFound)
    ));
    assert!(matches!(vfs.stat(lost), Err(VfsError::NotFound)));
    assert!(matches!(vfs.stat(nested), Err(VfsError::NotFound)));
    assert_eq!(names(&mut vfs, OBJECT_ROOT), ["kept"]);
}

/// The window holding a directory cannot be written to the intent log: the
/// log has no record for one. An fsync therefore commits it as a checkpoint
/// (ADR-121 decision 5), and what was fsynced survives the cut.
#[test]
fn an_fsync_over_a_staged_drawer_commits_the_window() {
    let mut vfs = delayed(formatted());
    vfs.sync_filesystem().unwrap();
    let start = vfs.generation();
    let drawer = vfs.create_directory(OBJECT_ROOT, "drawer", ms(0)).unwrap();
    let file = vfs.create_file(drawer, "synced", ms(10)).unwrap();
    let handle = vfs.open_file(file, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, b"durable", ms(10)).unwrap();
    assert_eq!(vfs.generation(), start, "nothing committed yet");
    vfs.fsync(handle).unwrap();
    assert!(
        vfs.generation() > start,
        "the fsync could not log the drawer, so it checkpointed"
    );
    vfs.close(handle).unwrap();
    // Cut right after the fsync.
    let mut vfs = remount(vfs);
    assert_eq!(vfs.lookup(OBJECT_ROOT, "drawer").unwrap(), drawer);
    assert_eq!(names(&mut vfs, drawer), ["synced"]);
}

/// The fallback: a mount that never set a durability stays `SYNC`, and a
/// directory made there is durable when the call returns.
#[test]
fn a_sync_mount_still_makes_a_drawer_durable_at_once() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    assert_eq!(vfs.durability(), Durability::Sync);
    let start = vfs.generation();
    let drawer = vfs.create_directory(OBJECT_ROOT, "sync", ms(0)).unwrap();
    assert!(vfs.generation() > start, "the create committed itself");
    assert!(!vfs.changes_pending());
    // No sync of any kind before the cut.
    let mut vfs = remount(vfs);
    assert_eq!(vfs.lookup(OBJECT_ROOT, "sync").unwrap(), drawer);
    assert_eq!(vfs.stat(drawer).unwrap().kind, NodeKind::Directory);
}
