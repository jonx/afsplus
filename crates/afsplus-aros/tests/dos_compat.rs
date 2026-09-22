//! DOS compatibility semantics beyond the Alpha-0 slice: metadata setters,
//! soft links, and the interaction of locks, open modes and delete.

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, EntryType, LockAccess, OpenMode};
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountMode, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

const BLOCK_SIZE: usize = 4096;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC2; 16],
            label: "DosCompat".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: timestamp(0),
        },
    )
    .unwrap();
    device
}

fn adapter(device: MemoryBackend) -> ArosAdapter<MemoryBackend> {
    ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig::default(),
    )
}

fn remount(adapter: ArosAdapter<MemoryBackend>) -> ArosAdapter<MemoryBackend> {
    let mut device = adapter.into_vfs().unwrap().into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    self::adapter(device)
}

fn create(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8], content: &[u8], at: i64) {
    let file = adapter
        .open(None, name, OpenMode::NewFile, timestamp(at))
        .unwrap();
    assert_eq!(
        adapter.write(file, content, timestamp(at)).unwrap(),
        content.len()
    );
    adapter.close(file).unwrap();
}

#[test]
fn set_protect_and_set_date_persist_exact_values() {
    let mut adapter = adapter(formatted());
    create(&mut adapter, b"note", b"text", 10);
    let directory = adapter
        .create_directory(None, b"drawer", timestamp(11))
        .unwrap();

    // DOS word: script + pure + archive set, delete denied (inverted bit 0).
    adapter
        .set_protection(None, b"note", 0x0000_0071, timestamp(20))
        .unwrap();
    // Empty name addresses the base lock's own object.
    adapter
        .set_protection(Some(directory), b"", 0x0000_000F, timestamp(21))
        .unwrap();
    adapter
        .set_modified(
            None,
            b"note",
            Timespec {
                seconds: 252_460_800,
                nanoseconds: 500_000_000,
            },
            timestamp(22),
        )
        .unwrap();
    adapter.free_lock(directory).unwrap();

    let mut adapter = remount(adapter);
    let note = adapter.locate(None, b"note", LockAccess::Shared).unwrap();
    let info = adapter.examine_lock(note).unwrap();
    assert_eq!(info.protection, 0x71);
    assert_eq!(
        info.modified,
        Timespec {
            seconds: 252_460_800,
            nanoseconds: 500_000_000,
        }
    );
    assert_eq!(info.size, 4);
    let drawer = adapter.locate(None, b"drawer", LockAccess::Shared).unwrap();
    let info = adapter.examine_lock(drawer).unwrap();
    assert_eq!(info.protection, 0xF);
    // The directory's date was never set: it keeps its creation-time value,
    // so the setter above did not leak onto a neighbour.
    assert_eq!(info.modified, timestamp(11));

    assert_eq!(
        adapter.set_protection(None, b"absent", 1, timestamp(30)),
        Err(ArosError::ObjectNotFound)
    );
    assert_eq!(
        adapter.set_modified(
            None,
            b"note",
            Timespec {
                seconds: 0,
                nanoseconds: 1_000_000_000,
            },
            timestamp(31),
        ),
        Err(ArosError::InvalidComponentName)
    );
}

#[test]
fn metadata_setters_are_refused_on_a_read_only_mount() {
    let mut writable = adapter(formatted());
    create(&mut writable, b"note", b"text", 10);
    let device = writable.into_vfs().unwrap().into_volume().into_device();
    let vfs = Vfs::mount(
        device,
        MountOptions {
            mode: MountMode::ReadOnly,
            ..MountOptions::default()
        },
    )
    .unwrap();
    let mut adapter = ArosAdapter::new(vfs, ArosConfig::default());
    assert_eq!(
        adapter.set_protection(None, b"note", 0x71, timestamp(20)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.set_modified(None, b"note", timestamp(5), timestamp(21)),
        Err(ArosError::DiskWriteProtected)
    );
    let note = adapter.locate(None, b"note", LockAccess::Shared).unwrap();
    let info = adapter.examine_lock(note).unwrap();
    assert_eq!(info.protection, 0);
    assert_eq!(info.modified, timestamp(10));
}

#[test]
fn soft_links_store_opaque_targets_and_defer_resolution_to_dos() {
    let mut adapter = adapter(formatted());
    create(&mut adapter, b"real", b"payload", 10);
    adapter
        .make_soft_link(None, b"alias", b"Work:dir/real", timestamp(11))
        .unwrap();

    // Traversal and open report the link; dos.library resolves it.
    assert_eq!(
        adapter.locate(None, b"alias", LockAccess::Shared),
        Err(ArosError::IsSoftLink)
    );
    assert_eq!(
        adapter.open(None, b"alias", OpenMode::OldFile, timestamp(12)),
        Err(ArosError::IsSoftLink)
    );
    assert_eq!(ArosError::IsSoftLink.io_error(), 233);

    let mut target = [0xEEu8; 32];
    assert_eq!(
        adapter.read_soft_link(None, b"alias", &mut target).unwrap(),
        13
    );
    assert_eq!(&target[..13], b"Work:dir/real");
    assert_eq!(target[13], 0xEE);
    // A short buffer learns the size and stays untouched.
    let mut short = [0xEEu8; 12];
    assert_eq!(
        adapter.read_soft_link(None, b"alias", &mut short).unwrap(),
        13
    );
    assert_eq!(short, [0xEE; 12]);
    assert_eq!(
        adapter.read_soft_link(None, b"real", &mut target),
        Err(ArosError::ObjectWrongType)
    );
    assert_eq!(
        adapter.make_soft_link(None, b"alias", b"other", timestamp(13)),
        Err(ArosError::ObjectExists)
    );
    assert_eq!(
        adapter.make_soft_link(None, b"empty", b"", timestamp(13)),
        Err(ArosError::InvalidComponentName)
    );

    // Enumeration reports ST_SOFTLINK for the link and a file for its sibling.
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    let mut seen = Vec::new();
    loop {
        match adapter.examine_next(root) {
            Ok(info) => seen.push((info.name, info.entry_type)),
            Err(ArosError::NoMoreEntries) => break,
            Err(error) => panic!("{error:?}"),
        }
    }
    seen.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        seen,
        vec![
            (b"alias".to_vec(), EntryType::SoftLink),
            (b"real".to_vec(), EntryType::File),
        ]
    );
    adapter.free_lock(root).unwrap();

    // Deleting the link removes the link only.
    let mut adapter = remount(adapter);
    adapter
        .delete_object(None, b"alias", timestamp(20))
        .unwrap();
    assert_eq!(
        adapter.read_soft_link(None, b"alias", &mut target),
        Err(ArosError::ObjectNotFound)
    );
    let real = adapter
        .open(None, b"real", OpenMode::OldFile, timestamp(21))
        .unwrap();
    let mut content = [0u8; 7];
    assert_eq!(adapter.read(real, &mut content).unwrap(), 7);
    assert_eq!(&content, b"payload");
    adapter.close(real).unwrap();
    remount(adapter);
}

#[test]
fn open_modes_lock_like_dos_and_held_objects_cannot_be_deleted() {
    let mut adapter = adapter(formatted());
    create(&mut adapter, b"data", b"abc", 10);

    // MODE_OLDFILE shares the object with locks and other readers.
    let reader = adapter
        .open(None, b"data", OpenMode::OldFile, timestamp(11))
        .unwrap();
    let shared = adapter.locate(None, b"data", LockAccess::Shared).unwrap();
    let updater = adapter
        .open(None, b"data", OpenMode::ReadWrite, timestamp(12))
        .unwrap();
    // MODE_NEWFILE and an exclusive lock both need sole ownership.
    assert_eq!(
        adapter.open(None, b"data", OpenMode::NewFile, timestamp(13)),
        Err(ArosError::ObjectInUse)
    );
    assert_eq!(
        adapter.locate(None, b"data", LockAccess::Exclusive),
        Err(ArosError::ObjectInUse)
    );
    // The refused MODE_NEWFILE did not truncate.
    assert_eq!(adapter.file_size(reader).unwrap(), 3);
    assert_eq!(
        adapter.delete_object(None, b"data", timestamp(14)),
        Err(ArosError::ObjectInUse)
    );

    adapter.close(updater).unwrap();
    adapter.free_lock(shared).unwrap();
    assert_eq!(
        adapter.delete_object(None, b"data", timestamp(15)),
        Err(ArosError::ObjectInUse)
    );
    adapter.close(reader).unwrap();

    // Sole ownership: MODE_NEWFILE excludes every other holder until closed.
    let writer = adapter
        .open(None, b"data", OpenMode::NewFile, timestamp(16))
        .unwrap();
    assert_eq!(
        adapter.open(None, b"data", OpenMode::OldFile, timestamp(17)),
        Err(ArosError::ObjectInUse)
    );
    assert_eq!(
        adapter.locate(None, b"data", LockAccess::Shared),
        Err(ArosError::ObjectInUse)
    );
    assert_eq!(adapter.lock_from_file(writer), Err(ArosError::ObjectInUse));
    adapter.close(writer).unwrap();

    // Control: with every holder released the same delete succeeds, so the
    // refusals above came from the holders and not from the delete path.
    assert!(!adapter.disk_info().in_use);
    adapter.delete_object(None, b"data", timestamp(18)).unwrap();
    assert_eq!(
        adapter.locate(None, b"data", LockAccess::Shared),
        Err(ArosError::ObjectNotFound)
    );
    remount(adapter);
}

fn names(prefix: &str, count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|index| format!("{prefix}{index:03}").into_bytes())
        .collect()
}

#[test]
fn exnext_continues_across_deletes_creates_and_renames() {
    let mut adapter = adapter(formatted());
    let all = names("f", 40);
    for (index, name) in all.iter().enumerate() {
        create(&mut adapter, name, b"x", 10 + index as i64);
    }

    // `Delete #?`: delete every entry right after ExNext returns it.
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    let mut returned = Vec::new();
    loop {
        match adapter.examine_next(root) {
            Ok(info) => {
                adapter
                    .delete_object(None, &info.name, timestamp(100))
                    .unwrap();
                returned.push(info.name);
            }
            Err(ArosError::NoMoreEntries) => break,
            Err(error) => panic!("{error:?}"),
        }
    }
    // Every name exactly once, in comparison-key order, none skipped.
    assert_eq!(returned, all);
    adapter.rewind_directory(root).unwrap();
    assert_eq!(adapter.examine_next(root), Err(ArosError::NoMoreEntries));

    // Mutations around the cursor: entries ordered after it appear once,
    // entries created before it do not, a rename of the last entry is inert.
    for name in [&b"b"[..], b"d", b"f", b"h"] {
        create(&mut adapter, name, b"x", 200);
    }
    adapter.rewind_directory(root).unwrap();
    assert_eq!(adapter.examine_next(root).unwrap().name, b"b");
    assert_eq!(adapter.examine_next(root).unwrap().name, b"d");
    create(&mut adapter, b"a", b"x", 201);
    create(&mut adapter, b"e", b"x", 202);
    adapter
        .rename(None, b"d", None, b"c", timestamp(203))
        .unwrap();
    adapter.delete_object(None, b"f", timestamp(204)).unwrap();
    let mut rest = Vec::new();
    loop {
        match adapter.examine_next(root) {
            Ok(info) => rest.push(info.name),
            Err(ArosError::NoMoreEntries) => break,
            Err(error) => panic!("{error:?}"),
        }
    }
    assert_eq!(rest, vec![b"e".to_vec(), b"h".to_vec()]);

    // What holds the pass above is the resume, and it now lives in the VFS
    // handle rather than in this adapter, so that the FUSE path gets it too:
    // a host keeping one handle on a directory used to see every commit
    // anywhere on the volume turn its next read into ESTALE. This asserts
    // the moved rule at its new home. The control that the rule is load
    // bearing is in the VFS's own tests, where removing it fails the walk;
    // asserting Stale here would only re-assert that the adapter no longer
    // does the work.
    adapter.free_lock(root).unwrap();
    let mut vfs = adapter.into_vfs().unwrap();
    let handle = vfs.open_directory(vfs.root_object()).unwrap();
    let page = vfs.read_directory(handle, 0, 1).unwrap();
    let first = page.entries[0].name.clone();
    vfs.create_file(vfs.root_object(), "z", timestamp(300))
        .unwrap();
    let after = vfs.read_directory(handle, page.next_cookie, 1).unwrap();
    assert_eq!(after.entries.len(), 1);
    assert_ne!(after.entries[0].name, first, "an entry came back twice");
    assert_eq!(vfs.resume_directory_after(handle, Some(b"a")).unwrap(), 1);
    assert_eq!(vfs.resume_directory_after(handle, Some(b"zz")).unwrap(), 6);
    assert_eq!(vfs.resume_directory_after(handle, Some(b"0")).unwrap(), 0);
    assert_eq!(vfs.resume_directory_after(handle, None).unwrap(), 0);
    vfs.close(handle).unwrap();
    let mut device = vfs.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn open_from_lock_consumes_the_lock_and_change_mode_respects_other_holders() {
    let mut adapter = adapter(formatted());
    create(&mut adapter, b"data", b"abcdef", 10);
    let drawer = adapter
        .create_directory(None, b"drawer", timestamp(11))
        .unwrap();

    // A directory lock is not a file; the lock survives the refusal.
    assert_eq!(
        adapter.open_from_lock(drawer),
        Err(ArosError::ObjectWrongType)
    );
    assert_eq!(adapter.examine_lock(drawer).unwrap().name, b"drawer");
    adapter.free_lock(drawer).unwrap();

    // The shared lock becomes the handle: the lock id dies, the hold stays.
    let lock = adapter.locate(None, b"data", LockAccess::Shared).unwrap();
    let file = adapter.open_from_lock(lock).unwrap();
    assert_eq!(adapter.free_lock(lock), Err(ArosError::InvalidLock));
    assert_eq!(adapter.examine_lock(lock), Err(ArosError::InvalidLock));
    assert_eq!(
        adapter.delete_object(None, b"data", timestamp(12)),
        Err(ArosError::ObjectInUse)
    );
    // Position zero, existing content kept, writable without truncation.
    let mut content = [0u8; 8];
    assert_eq!(adapter.read(file, &mut content).unwrap(), 6);
    assert_eq!(&content[..6], b"abcdef");
    assert_eq!(adapter.write_at(file, 0, b"AB", timestamp(13)).unwrap(), 2);
    assert_eq!(adapter.file_size(file).unwrap(), 6);

    // Shared to exclusive is refused while a second holder exists...
    let second = adapter.locate(None, b"data", LockAccess::Shared).unwrap();
    assert_eq!(
        adapter.change_file_mode(file, LockAccess::Exclusive),
        Err(ArosError::ObjectInUse)
    );
    assert_eq!(
        adapter.change_lock_mode(second, LockAccess::Exclusive),
        Err(ArosError::ObjectInUse)
    );
    // ...and the refusal changed nothing: a third shared lock still works.
    let third = adapter.locate(None, b"data", LockAccess::Shared).unwrap();
    adapter.free_lock(third).unwrap();
    adapter.free_lock(second).unwrap();

    // Sole holder: exclusive now excludes, and going back shares again.
    adapter
        .change_file_mode(file, LockAccess::Exclusive)
        .unwrap();
    assert_eq!(
        adapter.locate(None, b"data", LockAccess::Shared),
        Err(ArosError::ObjectInUse)
    );
    adapter
        .change_file_mode(file, LockAccess::Exclusive)
        .unwrap();
    adapter.change_file_mode(file, LockAccess::Shared).unwrap();
    let again = adapter.locate(None, b"data", LockAccess::Shared).unwrap();
    adapter.change_lock_mode(again, LockAccess::Shared).unwrap();
    adapter.free_lock(again).unwrap();
    adapter.close(file).unwrap();

    // An exclusive lock carries its exclusivity into the handle, and closing
    // that handle releases the object completely.
    let exclusive = adapter
        .locate(None, b"data", LockAccess::Exclusive)
        .unwrap();
    let file = adapter.open_from_lock(exclusive).unwrap();
    assert_eq!(
        adapter.open(None, b"data", OpenMode::OldFile, timestamp(14)),
        Err(ArosError::ObjectInUse)
    );
    adapter.close(file).unwrap();
    assert!(!adapter.disk_info().in_use);
    adapter.delete_object(None, b"data", timestamp(15)).unwrap();
    assert_eq!(
        adapter.change_lock_mode(999, LockAccess::Shared),
        Err(ArosError::InvalidLock)
    );
    remount(adapter);
}

#[test]
fn write_protect_refuses_every_mutation_until_the_key_unlocks_it() {
    let mut adapter = adapter(formatted());
    create(&mut adapter, b"kept", b"before", 10);
    let open_writer = adapter
        .open(None, b"kept", OpenMode::ReadWrite, timestamp(11))
        .unwrap();
    adapter.set_write_protect(true, 0x5EC2E7).unwrap();
    assert!(adapter.disk_info().write_protected);
    // The same key again is idempotent; another key cannot re-lock or unlock.
    adapter.set_write_protect(true, 0x5EC2E7).unwrap();
    assert_eq!(
        adapter.set_write_protect(true, 1),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.set_write_protect(false, 1),
        Err(ArosError::InvalidComponentName)
    );

    let refused = Err(ArosError::DiskWriteProtected);
    assert_eq!(adapter.write(open_writer, b"x", timestamp(12)), refused);
    assert_eq!(
        adapter
            .write_at(open_writer, 0, b"x", timestamp(12))
            .map(|_| ()),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter
            .set_file_size(
                open_writer,
                0,
                afsplus_aros::SeekMode::Beginning,
                timestamp(12)
            )
            .map(|_| ()),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.preallocate(open_writer, 0, 4096, timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter
            .open(None, b"new", OpenMode::NewFile, timestamp(12))
            .map(|_| ()),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter
            .create_directory(None, b"dir", timestamp(12))
            .map(|_| ()),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.delete_object(None, b"kept", timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.rename(None, b"kept", None, b"moved", timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.replace(None, b"kept", None, b"moved", timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.set_protection(None, b"kept", 1, timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.set_modified(None, b"kept", timestamp(1), timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.make_soft_link(None, b"alias", b"kept", timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    let lock = adapter.locate(None, b"kept", LockAccess::Shared).unwrap();
    assert_eq!(
        adapter.make_hard_link(None, b"hard", lock, timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    assert_eq!(
        adapter.clone_file(lock, None, b"twin", timestamp(12)),
        Err(ArosError::DiskWriteProtected)
    );
    // Reading stays possible.
    let mut content = [0u8; 6];
    assert_eq!(adapter.read_at(open_writer, 0, &mut content).unwrap(), 6);
    assert_eq!(&content, b"before");
    adapter.free_lock(lock).unwrap();

    // Control: with the right key the very same calls succeed.
    adapter.set_write_protect(false, 0x5EC2E7).unwrap();
    assert!(!adapter.disk_info().write_protected);
    assert_eq!(adapter.write(open_writer, b"AFTER!", timestamp(13)), Ok(6));
    adapter.close(open_writer).unwrap();
    adapter
        .rename(None, b"kept", None, b"moved", timestamp(14))
        .unwrap();

    // A zero key is the keyless lock: any key unlocks it.
    adapter.set_write_protect(true, 0).unwrap();
    adapter.set_write_protect(false, 77).unwrap();
    adapter.set_write_protect(false, 0).unwrap();

    let mut adapter = remount(adapter);
    // Nothing of the refused calls reached the volume, and the state itself
    // does not survive the mount.
    assert!(!adapter.disk_info().write_protected);
    let moved = adapter
        .open(None, b"moved", OpenMode::OldFile, timestamp(20))
        .unwrap();
    assert_eq!(adapter.read(moved, &mut content).unwrap(), 6);
    assert_eq!(&content, b"AFTER!");
    for absent in [&b"new"[..], b"dir", b"alias", b"hard", b"twin", b"kept"] {
        assert_eq!(
            adapter.locate(None, absent, LockAccess::Shared),
            Err(ArosError::ObjectNotFound)
        );
    }
}

#[test]
fn record_locks_collide_by_range_mode_and_handle_and_die_with_the_handle() {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            max_record_locks: 4,
            ..ArosConfig::default()
        },
    );
    create(&mut adapter, b"db", b"0123456789", 10);
    create(&mut adapter, b"other", b"0123456789", 10);
    let first = adapter
        .open(None, b"db", OpenMode::ReadWrite, timestamp(11))
        .unwrap();
    let second = adapter
        .open(None, b"db", OpenMode::ReadWrite, timestamp(11))
        .unwrap();
    let elsewhere = adapter
        .open(None, b"other", OpenMode::ReadWrite, timestamp(11))
        .unwrap();

    // [100, 200) exclusive for the first handle.
    adapter.lock_record(first, 100, 100, true).unwrap();
    // Overlap by one byte at either end collides, for shared requests too.
    assert_eq!(
        adapter.lock_record(second, 199, 10, false),
        Err(ArosError::LockCollision)
    );
    assert_eq!(
        adapter.lock_record(second, 50, 51, true),
        Err(ArosError::LockCollision)
    );
    assert_eq!(ArosError::LockCollision.io_error(), 241);
    // Touching ranges do not overlap; another file is another space; the
    // owner does not collide with itself.
    adapter.lock_record(second, 200, 10, true).unwrap();
    adapter.lock_record(second, 50, 50, false).unwrap();
    adapter.lock_record(elsewhere, 100, 100, true).unwrap();
    // Table of four is full.
    assert_eq!(
        adapter.lock_record(first, 150, 10, true),
        Err(ArosError::NoFreeStore)
    );
    adapter.free_record(elsewhere, 100, 100).unwrap();
    adapter.lock_record(first, 150, 10, true).unwrap();

    // Shared ranges coexist; an exclusive request over them collides.
    adapter.free_record(first, 150, 10).unwrap();
    adapter.lock_record(first, 60, 10, false).unwrap();
    adapter.free_record(first, 60, 10).unwrap();
    assert_eq!(
        adapter.lock_record(first, 60, 10, true),
        Err(ArosError::LockCollision)
    );

    // Free needs the exact range and the owning handle.
    assert_eq!(
        adapter.free_record(first, 100, 99),
        Err(ArosError::RecordNotLocked)
    );
    assert_eq!(
        adapter.free_record(second, 100, 100),
        Err(ArosError::RecordNotLocked)
    );
    assert_eq!(ArosError::RecordNotLocked.io_error(), 240);
    assert_eq!(
        adapter.lock_record(first, 0, 0, true),
        Err(ArosError::BadNumber)
    );
    assert_eq!(
        adapter.lock_record(first, u64::MAX, 2, true),
        Err(ArosError::BadNumber)
    );
    assert_eq!(
        adapter.lock_record(999, 0, 1, true),
        Err(ArosError::InvalidLock)
    );

    // Closing the first handle releases [100, 200): the control shows the
    // collision above came from that record and nothing else.
    adapter.close(first).unwrap();
    adapter.lock_record(second, 199, 10, false).unwrap();
    // Record locks are advisory: the locked bytes stay readable and writable.
    assert_eq!(
        adapter.write_at(elsewhere, 100, b"zz", timestamp(12)),
        Ok(2)
    );
    adapter.close(second).unwrap();
    adapter.close(elsewhere).unwrap();
}

#[test]
fn a_protected_volume_is_not_changed_by_flush_or_by_protecting_it() {
    // Three orphans left by a crash: unlinked while open, never closed.
    let now = timestamp(1);
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let root = vfs.root_object();
    for name in ["one", "two", "three"] {
        let object = vfs.create_file(root, name, now).unwrap();
        let handle = vfs
            .open_file(object, afsplus_vfs::AccessMode::ReadWrite)
            .unwrap();
        vfs.write(handle, 0, &[0x5A; 3 * 4096], now).unwrap();
        vfs.fsync(handle).unwrap();
        vfs.unlink_file(root, name, now).unwrap();
    }
    let device = vfs.into_volume().into_device();

    // A read-write mount resumes one orphan; two stay pending.
    let mut adapter = adapter(device);
    assert_eq!(adapter.health().unwrap().pending_orphans, 2);
    let free_before = adapter.health().unwrap().free_blocks;
    let generation_before = adapter.health().unwrap().generation;

    adapter.set_write_protect(true, 7).unwrap();
    adapter.flush().unwrap();
    adapter.flush().unwrap();
    let protected = adapter.health().unwrap();
    assert_eq!(protected.pending_orphans, 2);
    assert_eq!(protected.free_blocks, free_before);
    assert_eq!(protected.generation, generation_before);

    // Control: unprotected, the same flush resumes work and frees blocks, so
    // the stillness above was the protection and not an empty queue.
    //
    // How MUCH it resumes is not the subject. It used to be exactly one orphan
    // per flush, and this asserted that number; a flush now drains the queue
    // instead of taking one slice off it, because taking one slice meant the
    // space of a deleted file could stay outstanding for the whole life of a
    // mount. Pinning the count here made this test fail for a change it was
    // never about, and bumping the number would have left it asserting the new
    // arithmetic rather than the protection.
    adapter.set_write_protect(false, 7).unwrap();
    adapter.flush().unwrap();
    let resumed = adapter.health().unwrap();
    assert!(
        resumed.pending_orphans < protected.pending_orphans,
        "an unprotected flush must resume orphan work: still {} pending",
        resumed.pending_orphans
    );
    assert!(resumed.free_blocks > free_before);
    remount(adapter);
}

#[test]
fn rename_disk_relabels_in_one_commit_and_root_locks_follow() {
    let mut adapter = adapter(formatted());
    // The label given at format time is the volume name until renamed.
    assert_eq!(adapter.volume_label().unwrap(), b"DosCompat");
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();

    adapter.set_volume_label(b"Work", timestamp(10)).unwrap();
    assert_eq!(adapter.volume_label().unwrap(), b"Work");
    assert_eq!(adapter.examine_lock(root).unwrap().name, b"Work");
    let again = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    assert_eq!(adapter.examine_lock(again).unwrap().name, b"Work");
    adapter.free_lock(again).unwrap();
    adapter.free_lock(root).unwrap();

    // DOS naming rules and the format bound; a refusal changes nothing.
    for bad in [&b""[..], b"a:b", b"a/b", b"nul\0"] {
        assert_eq!(
            adapter.set_volume_label(bad, timestamp(11)),
            Err(ArosError::InvalidComponentName)
        );
    }
    assert_eq!(
        adapter.set_volume_label(&[b'x'; 65], timestamp(11)),
        Err(ArosError::ObjectTooLarge)
    );
    adapter
        .set_volume_label(&[b'y'; 64], timestamp(12))
        .unwrap();
    adapter.set_volume_label(b"Work", timestamp(13)).unwrap();
    adapter.set_write_protect(true, 0).unwrap();
    assert_eq!(
        adapter.set_volume_label(b"Locked", timestamp(14)),
        Err(ArosError::DiskWriteProtected)
    );
    adapter.set_write_protect(false, 0).unwrap();

    // The new label is on the volume, and the info document reports it.
    let mut adapter = remount(adapter);
    assert_eq!(adapter.volume_label().unwrap(), b"Work");
    assert!(adapter.info_json().unwrap().contains("\"label\":\"Work\","));
}

#[test]
fn a_latin1_mount_bounds_the_stored_form_of_the_label() {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            name_encoding: afsplus_aros::NameEncoding::Latin1,
            ..ArosConfig::default()
        },
    );
    // 32 times e-acute: 32 Latin-1 bytes, 64 stored bytes. 33 do not fit.
    adapter.set_volume_label(&[0xE9; 32], timestamp(1)).unwrap();
    assert_eq!(adapter.volume_label().unwrap(), vec![0xE9; 32]);
    assert_eq!(
        adapter.set_volume_label(&[0xE9; 33], timestamp(2)),
        Err(ArosError::ObjectTooLarge)
    );
    assert_eq!(adapter.volume_label().unwrap(), vec![0xE9; 32]);
}

#[test]
fn the_volume_is_named_after_its_label_at_every_mount() {
    let unnamed = || ArosConfig {
        volume_name: Vec::new(),
        ..ArosConfig::default()
    };
    let mut adapter = ArosAdapter::new(
        Vfs::mount(formatted(), MountOptions::default()).unwrap(),
        unnamed(),
    );
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    assert_eq!(adapter.examine_lock(root).unwrap().name, b"DosCompat");
    adapter.free_lock(root).unwrap();
    adapter.set_volume_label(b"Work", timestamp(1)).unwrap();

    // The renamed volume comes back under its new name, never a constant.
    let device = adapter.into_vfs().unwrap().into_volume().into_device();
    let mut adapter = ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        unnamed(),
    );
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    assert_eq!(adapter.examine_lock(root).unwrap().name, b"Work");
    adapter.free_lock(root).unwrap();

    // Control: an explicit name still overrides the label for that mount.
    let device = adapter.into_vfs().unwrap().into_volume().into_device();
    let mut adapter = ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig {
            volume_name: b"Override".to_vec(),
            ..ArosConfig::default()
        },
    );
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    assert_eq!(adapter.examine_lock(root).unwrap().name, b"Override");
    assert_eq!(adapter.volume_label().unwrap(), b"Work");
}

fn read_all(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8]) -> Vec<u8> {
    let file = adapter
        .open(None, name, OpenMode::OldFile, timestamp(90))
        .unwrap();
    let mut bytes = vec![0u8; 256];
    let count = adapter.read(file, &mut bytes).unwrap();
    adapter.close(file).unwrap();
    bytes.truncate(count);
    bytes
}

/// MODE_OLDFILE opens an existing file that the handle may also write: the
/// Fast File System and SFS let a program update a file in place this way.
/// While the volume is write-protected the write is refused, and it goes
/// through once the protection is lifted, on the same handle.
#[test]
fn an_old_file_handle_writes_as_it_does_on_the_fast_file_system() {
    let mut adapter = adapter(formatted());
    create(&mut adapter, b"data", b"abcdef", 10);

    let file = adapter
        .open(None, b"data", OpenMode::OldFile, timestamp(11))
        .unwrap();
    adapter
        .seek(file, 2, afsplus_aros::SeekMode::Beginning)
        .unwrap();
    assert_eq!(adapter.write(file, b"XY", timestamp(12)).unwrap(), 2);

    adapter.set_write_protect(true, 7).unwrap();
    assert_eq!(
        adapter.write(file, b"!", timestamp(13)),
        Err(ArosError::DiskWriteProtected)
    );
    adapter.set_write_protect(false, 7).unwrap();
    assert_eq!(adapter.write(file, b"Z", timestamp(14)).unwrap(), 1);
    adapter.close(file).unwrap();

    assert_eq!(read_all(&mut adapter, b"data"), b"abXYZf");
    let mut adapter = remount(adapter);
    assert_eq!(read_all(&mut adapter, b"data"), b"abXYZf");
}

/// On a mount that takes no write at all, MODE_OLDFILE still opens the file
/// for reading, and a write through it is refused as write-protected.
#[test]
fn an_old_file_handle_on_a_read_only_mount_reads_and_refuses_writes() {
    let mut writable = adapter(formatted());
    create(&mut writable, b"note", b"text", 10);
    let device = writable.into_vfs().unwrap().into_volume().into_device();
    let vfs = Vfs::mount(
        device,
        MountOptions {
            mode: MountMode::ReadOnly,
            ..MountOptions::default()
        },
    )
    .unwrap();
    let mut adapter = ArosAdapter::new(vfs, ArosConfig::default());
    let file = adapter
        .open(None, b"note", OpenMode::OldFile, timestamp(20))
        .unwrap();
    assert_eq!(
        adapter.write(file, b"x", timestamp(21)),
        Err(ArosError::DiskWriteProtected)
    );
    let mut bytes = [0u8; 8];
    assert_eq!(adapter.read(file, &mut bytes).unwrap(), 4);
    assert_eq!(&bytes[..4], b"text");
    adapter.close(file).unwrap();
}
