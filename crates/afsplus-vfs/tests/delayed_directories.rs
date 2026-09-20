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

/// The counter test of lot D2. Removing 91 drawers on a delayed mount costs
/// the flushes of the window commits alone. Before it, each `RemoveDir`
/// committed the open window, ran a transaction of its own and let the
/// maintenance behind it commit again: five checkpoints and ten flushes per
/// drawer, 910 for the delete phase of the hosted benchmark.
#[test]
fn removing_drawers_flushes_what_the_window_commits_flush() {
    const TREES: usize = 10;
    const PER_TREE: usize = 8;
    let device = TraceBackend::new(formatted());
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    for tree in 0..TREES {
        let id = vfs
            .create_directory(OBJECT_ROOT, &format!("t{tree:02}"), ms(0))
            .unwrap();
        for drawer in 0..PER_TREE {
            vfs.create_directory(id, &format!("d{drawer}"), ms(0))
                .unwrap();
        }
    }
    vfs.sync_filesystem().unwrap();

    // The drawers are committed; the counters start from here, so what
    // follows counts the removals and nothing else.
    let mut device = vfs.into_volume().into_device();
    device.reset();
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    let start = vfs.generation();
    for tree in 0..TREES {
        let name = format!("t{tree:02}");
        let id = vfs.lookup(OBJECT_ROOT, &name).unwrap();
        for drawer in 0..PER_TREE {
            vfs.remove_directory(id, &format!("d{drawer}"), ms(10))
                .unwrap();
        }
        vfs.remove_directory(OBJECT_ROOT, &name, ms(10)).unwrap();
    }
    vfs.sync_filesystem().unwrap();
    let commits = vfs.generation() - start;
    let removals = (TREES * PER_TREE + TREES) as u32;
    let device = vfs.into_volume().into_device();
    let flushes = device.stats().flushes;
    // The window bound commits once per 512 removals, the final sync once
    // more, and the maintenance each commit runs afterwards is a transaction
    // of its own.
    let expected_commits = 2 * (removals / DELAYED_WINDOW_OPS_MAX + 2);
    eprintln!(
        "{removals} drawers removed: {commits} generations, {flushes} flushes, \
         {expected_commits} commits expected"
    );
    // Two flushes a commit. The old code paid ten flushes per drawer, 900
    // here, and this assertion fails on it.
    assert!(
        flushes <= u64::from(2 * expected_commits),
        "{flushes} flushes for {expected_commits} commits of two flushes"
    );
    assert!(
        flushes < u64::from(removals),
        "{flushes} flushes is still a flush per drawer or more"
    );
    let mut device = device.into_inner();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    assert!(names(&mut vfs, OBJECT_ROOT).is_empty());
}

/// A drawer made and removed inside one window cancels out: nothing was
/// committed in between, so nothing of it reaches the disk and the block its
/// entry tree was to be rooted at goes back to the transaction.
#[test]
fn a_drawer_made_and_removed_in_one_window_leaves_nothing() {
    let mut vfs = delayed(formatted());
    vfs.sync_filesystem().unwrap();
    let start = vfs.generation();
    let free_before = vfs.statfs().available_blocks;
    let drawer = vfs.create_directory(OBJECT_ROOT, "gone", ms(0)).unwrap();
    assert_eq!(vfs.generation(), start);
    vfs.remove_directory(OBJECT_ROOT, "gone", ms(10)).unwrap();
    // Still no commit: the removal is an operation of the same window.
    assert_eq!(vfs.generation(), start);
    assert!(matches!(vfs.stat(drawer), Err(VfsError::NotFound)));
    assert!(names(&mut vfs, OBJECT_ROOT).is_empty());
    vfs.sync_filesystem().unwrap();
    assert_eq!(vfs.statfs().available_blocks, free_before);
    let mut vfs = remount(vfs);
    assert!(names(&mut vfs, OBJECT_ROOT).is_empty());
    assert!(matches!(vfs.stat(drawer), Err(VfsError::NotFound)));
}

/// A drawer that still holds something is refused, and the window that
/// staged that something is intact afterwards: the delayed path answers what
/// the immediate path answers.
#[test]
fn removing_a_drawer_that_still_holds_a_pending_file_is_refused() {
    let mut vfs = delayed(formatted());
    let drawer = vfs.create_directory(OBJECT_ROOT, "work", ms(0)).unwrap();
    vfs.sync_filesystem().unwrap();
    let start = vfs.generation();

    // A file only in the window: the drawer is not empty, and no commit is
    // allowed to happen to find that out.
    vfs.create_file(drawer, "draft.c", ms(10)).unwrap();
    assert!(matches!(
        vfs.remove_directory(OBJECT_ROOT, "work", ms(20)),
        Err(VfsError::DirectoryNotEmpty)
    ));
    assert_eq!(vfs.generation(), start);
    assert_eq!(names(&mut vfs, drawer), ["draft.c"]);

    // With the file gone, again only in the window, the drawer goes.
    vfs.unlink_file(drawer, "draft.c", ms(30)).unwrap();
    vfs.remove_directory(OBJECT_ROOT, "work", ms(40)).unwrap();
    assert!(names(&mut vfs, OBJECT_ROOT).is_empty());
    let mut vfs = remount(vfs);
    assert!(names(&mut vfs, OBJECT_ROOT).is_empty());

    // A committed drawer holding a committed file is refused the same way.
    vfs.set_durability(Durability::DELAYED).unwrap();
    let drawer = vfs.create_directory(OBJECT_ROOT, "src", ms(50)).unwrap();
    vfs.create_file(drawer, "main.c", ms(60)).unwrap();
    vfs.sync_filesystem().unwrap();
    assert!(matches!(
        vfs.remove_directory(OBJECT_ROOT, "src", ms(70)),
        Err(VfsError::DirectoryNotEmpty)
    ));
    assert_eq!(names(&mut vfs, drawer), ["main.c"]);
}

/// A cut inside a window that removes drawers: the image is clean and the
/// drawers are all present or all gone, per checkpoint.
#[test]
fn a_cut_inside_the_window_leaves_the_drawers_all_present_or_all_gone() {
    let mut vfs = delayed(formatted());
    let mut drawers = Vec::new();
    for drawer in 0..12 {
        let id = vfs
            .create_directory(OBJECT_ROOT, &format!("d{drawer:02}"), ms(0))
            .unwrap();
        vfs.create_file(id, "inside", ms(0)).unwrap();
        drawers.push(id);
    }
    vfs.sync_filesystem().unwrap();
    let present: Vec<String> = (0..12).map(|drawer| format!("d{drawer:02}")).collect();

    // The removals stay in the window; the cut takes the device as it is.
    for (at, id) in drawers.iter().enumerate() {
        vfs.unlink_file(*id, "inside", ms(1_000)).unwrap();
        vfs.remove_directory(OBJECT_ROOT, &format!("d{at:02}"), ms(1_010))
            .unwrap();
    }
    // Listing the root would commit the window first, so the cut comes
    // straight after the removals.
    assert!(vfs.changes_pending());
    let mut vfs = remount(vfs);
    assert_eq!(names(&mut vfs, OBJECT_ROOT), present);
    for id in &drawers {
        assert_eq!(names(&mut vfs, *id), ["inside"]);
    }
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
