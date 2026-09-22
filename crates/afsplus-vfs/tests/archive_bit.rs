//! The archive bit says "backed up since the last change". A backup program
//! sets it once it holds a copy; the file system clears it when the object
//! changes after that, as SFS, PFS3 and the RAM handler do. A file loses it
//! when its content is written, truncated or extended; a drawer loses it when
//! it gains or loses an entry. Setting the protection, the date or the comment
//! is not such a change, so a restore can put the bit back and keep it.
//!
//! Every case runs on a SYNC mount and on a delayed one (ADR-121), because
//! the two reach the disk by different code: an immediate transaction, or a
//! window materialised at its commit.

use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::posix::PROTECTION_ARCHIVE;
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Durability, ObjectId, Vfs};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds: 1_000 + seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 16_384);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xA4; 16],
            label: "Archive".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    device
}

fn mounted(delayed: bool) -> Vfs<MemoryBackend> {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    if delayed {
        vfs.set_durability(Durability::DELAYED).unwrap();
    }
    vfs
}

/// What the disk says, after a commit and a remount, with the image checked.
fn archived_after_remount(vfs: Vfs<MemoryBackend>, objects: &[ObjectId]) -> Vec<bool> {
    let mut vfs = vfs;
    vfs.sync_filesystem().unwrap();
    let mut device = vfs.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    objects.iter().map(|id| archived(&mut vfs, *id)).collect()
}

fn archived(vfs: &mut Vfs<MemoryBackend>, object: ObjectId) -> bool {
    vfs.stat(object).unwrap().protection as u32 & PROTECTION_ARCHIVE != 0
}

/// What a backup program does once it holds a copy.
fn mark_archived(vfs: &mut Vfs<MemoryBackend>, object: ObjectId, now: Timespec) {
    let protection = vfs.stat(object).unwrap().protection as u32;
    vfs.set_protection(object, protection | PROTECTION_ARCHIVE, now)
        .unwrap();
    assert!(archived(vfs, object));
}

fn write(vfs: &mut Vfs<MemoryBackend>, file: ObjectId, offset: u64, bytes: &[u8], now: Timespec) {
    let handle = vfs.open_file(file, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, offset, bytes, now).unwrap();
    vfs.close(handle).unwrap();
}

/// A file with content, backed up: the starting point of every case.
fn backed_up_file(vfs: &mut Vfs<MemoryBackend>, parent: ObjectId, name: &str) -> ObjectId {
    let file = vfs.create_file(parent, name, at(1)).unwrap();
    write(vfs, file, 0, b"the first version", at(2));
    vfs.sync_filesystem().unwrap();
    mark_archived(vfs, file, at(3));
    vfs.sync_filesystem().unwrap();
    file
}

fn drawer(vfs: &mut Vfs<MemoryBackend>, name: &str) -> ObjectId {
    let drawer = vfs.create_directory(OBJECT_ROOT, name, at(1)).unwrap();
    vfs.sync_filesystem().unwrap();
    mark_archived(vfs, drawer, at(3));
    vfs.sync_filesystem().unwrap();
    drawer
}

fn both(case: impl Fn(bool)) {
    case(false);
    case(true);
}

#[test]
fn a_write_clears_the_bit_of_the_file() {
    both(|delayed| {
        let mut vfs = mounted(delayed);
        let overwritten = backed_up_file(&mut vfs, OBJECT_ROOT, "overwritten");
        let appended = backed_up_file(&mut vfs, OBJECT_ROOT, "appended");
        let untouched = backed_up_file(&mut vfs, OBJECT_ROOT, "untouched");

        write(&mut vfs, overwritten, 0, b"THE", at(10));
        write(&mut vfs, appended, 17, b", and more", at(11));

        assert!(!archived(&mut vfs, overwritten), "delayed={delayed}");
        assert!(!archived(&mut vfs, appended), "delayed={delayed}");
        assert!(archived(&mut vfs, untouched), "delayed={delayed}");
        assert_eq!(
            archived_after_remount(vfs, &[overwritten, appended, untouched]),
            [false, false, true],
            "delayed={delayed}"
        );
    });
}

#[test]
fn a_truncate_clears_the_bit_of_the_file() {
    both(|delayed| {
        let mut vfs = mounted(delayed);
        let shortened = backed_up_file(&mut vfs, OBJECT_ROOT, "shortened");
        let extended = backed_up_file(&mut vfs, OBJECT_ROOT, "extended");

        let handle = vfs.open_file(shortened, AccessMode::WriteOnly).unwrap();
        vfs.truncate(handle, 4, at(10)).unwrap();
        vfs.close(handle).unwrap();
        let handle = vfs.open_file(extended, AccessMode::WriteOnly).unwrap();
        vfs.truncate(handle, 9_000, at(11)).unwrap();
        vfs.close(handle).unwrap();

        assert_eq!(
            archived_after_remount(vfs, &[shortened, extended]),
            [false, false],
            "delayed={delayed}"
        );
    });
}

#[test]
fn a_drawer_that_gains_or_loses_an_entry_loses_its_bit() {
    both(|delayed| {
        let mut vfs = mounted(delayed);
        let gains_file = drawer(&mut vfs, "gains-file");
        let gains_drawer = drawer(&mut vfs, "gains-drawer");
        let loses = drawer(&mut vfs, "loses");
        let source = drawer(&mut vfs, "source");
        let target = drawer(&mut vfs, "target");
        let linked = drawer(&mut vfs, "linked");
        let quiet = drawer(&mut vfs, "quiet");
        let leaving = vfs.create_file(loses, "leaving", at(4)).unwrap();
        let moving = vfs.create_file(source, "moving", at(4)).unwrap();
        let shared = vfs.create_file(quiet, "shared", at(4)).unwrap();
        vfs.sync_filesystem().unwrap();
        for object in [loses, source, quiet] {
            mark_archived(&mut vfs, object, at(5));
        }
        mark_archived(&mut vfs, moving, at(5));
        vfs.sync_filesystem().unwrap();

        // A link is not staged in the window: it commits what waits and then
        // itself. It goes first, so the four changes after it are still in
        // the window when the drawers are read below.
        vfs.link_file(shared, linked, "shared", at(9)).unwrap();
        assert!(!archived(&mut vfs, linked), "delayed={delayed}");
        vfs.create_file(gains_file, "new", at(10)).unwrap();
        vfs.create_directory(gains_drawer, "new", at(10)).unwrap();
        vfs.unlink_file(loses, "leaving", at(10)).unwrap();
        vfs.rename(source, "moving", target, "moved", false, at(10))
            .unwrap();
        let _ = leaving;

        // Before any commit, as a program on a delayed mount reads it: the
        // drawers already show the change, with its date (ADR-121 decision 4).
        for changed in [gains_file, gains_drawer, loses, source, target] {
            let stat = vfs.stat(changed).unwrap();
            assert!(
                stat.protection as u32 & PROTECTION_ARCHIVE == 0,
                "delayed={delayed}: a changed drawer showed the bit before the commit"
            );
            assert_eq!(stat.modified, at(10), "delayed={delayed}");
        }
        assert!(archived(&mut vfs, quiet), "delayed={delayed}");

        assert_eq!(
            archived_after_remount(
                vfs,
                &[
                    gains_file,
                    gains_drawer,
                    loses,
                    source,
                    target,
                    linked,
                    quiet,
                    moving
                ]
            ),
            // The drawers whose entries changed lose the bit. The drawer the
            // link came from is untouched, and so is the moved file: its
            // content did not change, and the two drawers that did say where
            // a backup has to look again.
            [false, false, false, false, false, false, true, true],
            "delayed={delayed}"
        );
    });
}

#[test]
fn a_rename_inside_one_drawer_clears_that_drawer_once() {
    both(|delayed| {
        let mut vfs = mounted(delayed);
        let only = drawer(&mut vfs, "only");
        vfs.create_file(only, "before", at(4)).unwrap();
        vfs.sync_filesystem().unwrap();
        mark_archived(&mut vfs, only, at(5));
        vfs.sync_filesystem().unwrap();

        vfs.rename(only, "before", only, "after", false, at(10))
            .unwrap();

        assert_eq!(
            archived_after_remount(vfs, &[only]),
            [false],
            "delayed={delayed}"
        );
    });
}

#[test]
fn setting_protection_date_or_comment_keeps_the_bit() {
    both(|delayed| {
        let mut vfs = mounted(delayed);
        let file = backed_up_file(&mut vfs, OBJECT_ROOT, "restored");
        let folder = drawer(&mut vfs, "folder");

        // What a restore does after it has written the content back: the
        // date, the comment, then the protection word with ARCHIVE set.
        vfs.set_times(file, at(-500), at(10)).unwrap();
        vfs.set_comment(file, "restored from tape", at(11)).unwrap();
        let protection = vfs.stat(file).unwrap().protection as u32;
        vfs.set_protection(file, protection | 0x2, at(12)).unwrap();
        vfs.set_times(folder, at(-500), at(10)).unwrap();
        vfs.set_comment(folder, "restored", at(11)).unwrap();

        assert_eq!(
            archived_after_remount(vfs, &[file, folder]),
            [true, true],
            "delayed={delayed}"
        );
    });
}

#[test]
fn a_new_object_starts_without_the_bit() {
    both(|delayed| {
        let mut vfs = mounted(delayed);
        let file = vfs.create_file(OBJECT_ROOT, "new-file", at(1)).unwrap();
        write(&mut vfs, file, 0, b"never backed up", at(2));
        let folder = vfs
            .create_directory(OBJECT_ROOT, "new-drawer", at(3))
            .unwrap();

        assert_eq!(
            archived_after_remount(vfs, &[file, folder]),
            [false, false],
            "delayed={delayed}"
        );
    });
}
