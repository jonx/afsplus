use afsplus_aros::{
    ArosAdapter, ArosConfig, ArosError, EntryType, LockAccess, NameEncoding, OpenMode, SeekMode,
};
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
            uuid: [0xA2; 16],
            label: "ArosAdapter".into(),
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

#[test]
fn dos_semantics_cover_the_mountable_alpha_operation_slice() {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(vfs, ArosConfig::default());

    let work = adapter
        .create_directory(None, b"work", timestamp(1))
        .unwrap();
    let draft = adapter
        .open(Some(work), b"draft", OpenMode::NewFile, timestamp(2))
        .unwrap();
    assert_eq!(adapter.write(draft, b"hello", timestamp(3)).unwrap(), 5);
    assert_eq!(adapter.seek(draft, 8192, SeekMode::Beginning).unwrap(), 5);
    assert_eq!(adapter.write(draft, b"tail", timestamp(4)).unwrap(), 4);
    assert_eq!(adapter.seek(draft, 0, SeekMode::Beginning).unwrap(), 8196);
    let mut visible_before_fsync = [0u8; 5];
    assert_eq!(adapter.read(draft, &mut visible_before_fsync).unwrap(), 5);
    assert_eq!(&visible_before_fsync, b"hello");
    adapter.fsync(draft).unwrap();
    assert_eq!(
        adapter
            .set_file_size(draft, 5, SeekMode::Beginning, timestamp(5))
            .unwrap(),
        5
    );
    let parent = adapter.parent_of_file(draft).unwrap();
    assert!(adapter.same_lock(Some(work), Some(parent)).unwrap());
    adapter.free_lock(parent).unwrap();
    let draft_lock = adapter.lock_from_file(draft).unwrap();
    assert_eq!(adapter.examine_lock(draft_lock).unwrap().name, b"draft");
    adapter.free_lock(draft_lock).unwrap();
    adapter.close(draft).unwrap();

    adapter
        .rename(Some(work), b"draft", None, b"final", timestamp(6))
        .unwrap();
    let final_lock = adapter.locate(None, b"final", LockAccess::Shared).unwrap();
    adapter
        .make_hard_link(Some(work), b"linked", final_lock, timestamp(7))
        .unwrap();
    adapter.delete_object(None, b"final", timestamp(8)).unwrap();

    let linked = adapter
        .open(Some(work), b"linked", OpenMode::OldFile, timestamp(9))
        .unwrap();
    let mut contents = [0u8; 8];
    assert_eq!(adapter.read(linked, &mut contents).unwrap(), 5);
    assert_eq!(&contents[..5], b"hello");
    assert_eq!(adapter.file_size(linked).unwrap(), 5);
    adapter.close(linked).unwrap();

    let info = adapter.examine_lock(work).unwrap();
    assert_eq!(info.entry_type, EntryType::Directory);
    let entry = adapter.examine_next(work).unwrap();
    assert_eq!(entry.name, b"linked");
    assert_eq!(entry.entry_type, EntryType::File);
    assert_eq!(adapter.examine_next(work), Err(ArosError::NoMoreEntries));
    adapter.rewind_directory(work).unwrap();

    let disk = adapter.disk_info();
    assert!(!disk.write_protected);
    assert!(disk.in_use);
    assert_eq!(disk.bytes_per_block, BLOCK_SIZE as u32);
    adapter.flush().unwrap();
    adapter.free_lock(final_lock).unwrap();
    adapter.free_lock(work).unwrap();

    let vfs = adapter.into_vfs().unwrap();
    let device = vfs.into_volume().into_device();
    let remounted = Vfs::mount(device, MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(remounted, ArosConfig::default());
    let work = adapter.locate(None, b"work", LockAccess::Shared).unwrap();
    let linked = adapter
        .open(Some(work), b"linked", OpenMode::OldFile, timestamp(10))
        .unwrap();
    let mut contents = [0u8; 5];
    assert_eq!(adapter.read(linked, &mut contents).unwrap(), 5);
    assert_eq!(&contents, b"hello");
    adapter.close(linked).unwrap();
    adapter.free_lock(work).unwrap();

    let mut device = adapter.into_vfs().unwrap().into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn latin1_names_round_trip_and_lock_conflicts_are_explicit() {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            name_encoding: NameEncoding::Latin1,
            ..ArosConfig::default()
        },
    );
    let file = adapter
        .open(None, b"caf\xe9", OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.close(file).unwrap();
    let folded = adapter
        .locate(None, b"CAF\xc9", LockAccess::Shared)
        .unwrap();
    adapter.free_lock(folded).unwrap();
    // MODE_NEWFILE follows DOS semantics and truncates an existing folded
    // match; it must not create a second directory entry.
    let reopened = adapter
        .open(None, b"CAF\xc9", OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.close(reopened).unwrap();
    let shared = adapter
        .locate(None, b"caf\xe9", LockAccess::Shared)
        .unwrap();
    assert_eq!(
        adapter.locate(None, b"caf\xe9", LockAccess::Exclusive),
        Err(ArosError::ObjectInUse)
    );
    adapter.free_lock(shared).unwrap();
    let exclusive = adapter
        .locate(None, b"caf\xe9", LockAccess::Exclusive)
        .unwrap();
    adapter.free_lock(exclusive).unwrap();

    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    assert_eq!(adapter.examine_next(root).unwrap().name, b"caf\xe9");
    adapter.free_lock(root).unwrap();

    let directory = adapter
        .create_directory(None, b"directory", timestamp(2))
        .unwrap();
    let child = adapter
        .create_directory(Some(directory), b"child", timestamp(3))
        .unwrap();
    adapter.free_lock(directory).unwrap();
    let exclusive_parent = adapter
        .parent_lock_with_access(child, LockAccess::Exclusive)
        .unwrap()
        .unwrap();
    assert_eq!(
        adapter.locate(None, b"directory", LockAccess::Shared),
        Err(ArosError::ObjectInUse)
    );
    adapter.free_lock(exclusive_parent).unwrap();
    adapter.free_lock(child).unwrap();
}

#[test]
fn read_only_mount_and_invalid_offsets_map_to_dos_errors() {
    let vfs = Vfs::mount(
        formatted(),
        MountOptions {
            mode: MountMode::ReadOnly,
        },
    )
    .unwrap();
    let mut adapter = ArosAdapter::new(vfs, ArosConfig::default());
    assert_eq!(
        adapter.open(None, b"new", OpenMode::NewFile, timestamp(1)),
        Err(ArosError::DiskWriteProtected)
    );
    assert!(adapter.disk_info().write_protected);

    let writable = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(writable, ArosConfig::default());
    let file = adapter
        .open(None, b"file", OpenMode::NewFile, timestamp(1))
        .unwrap();
    assert_eq!(
        adapter.seek(file, -1, SeekMode::Beginning),
        Err(ArosError::SeekError)
    );
    assert_eq!(
        adapter.locate(None, b"bad/name", LockAccess::Shared),
        Err(ArosError::InvalidComponentName)
    );
    assert_eq!(ArosError::NoMoreEntries.io_error(), 232);
}

#[test]
fn examine_reports_the_stored_spelling_on_a_case_insensitive_volume() {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(vfs, ArosConfig::default());

    let file = adapter
        .open(None, b"CaseTest", OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.close(file).unwrap();

    let lock = adapter
        .locate(None, b"CASETEST", LockAccess::Shared)
        .unwrap();
    assert_eq!(adapter.examine_lock(lock).unwrap().name, b"CaseTest");

    let reopened = adapter
        .open(None, b"casetest", OpenMode::OldFile, timestamp(2))
        .unwrap();
    assert_eq!(adapter.examine_file(reopened).unwrap().name, b"CaseTest");
    let from_file = adapter.lock_from_file(reopened).unwrap();
    assert_eq!(adapter.examine_lock(from_file).unwrap().name, b"CaseTest");
    adapter.close(reopened).unwrap();

    let dir = adapter
        .create_directory(None, b"Work", timestamp(3))
        .unwrap();
    let inner = adapter
        .open(Some(dir), b"Draft", OpenMode::NewFile, timestamp(4))
        .unwrap();
    adapter.close(inner).unwrap();
    let inner_lock = adapter.locate(None, b"work", LockAccess::Shared).unwrap();
    let draft = adapter
        .locate(Some(inner_lock), b"DRAFT", LockAccess::Shared)
        .unwrap();
    assert_eq!(adapter.examine_lock(draft).unwrap().name, b"Draft");
    let parent = adapter.parent_lock(draft).unwrap().unwrap();
    assert_eq!(adapter.examine_lock(parent).unwrap().name, b"Work");
}

#[test]
fn set_protection_and_set_date_persist_through_examine() {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(vfs, ArosConfig::default());

    let file = adapter
        .open(None, b"prot", OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.write(file, b"p", timestamp(1)).unwrap();
    adapter.close(file).unwrap();
    let dir = adapter
        .create_directory(None, b"dir", timestamp(1))
        .unwrap();

    const FIBF_SCRIPT_PURE: u32 = (1 << 6) | (1 << 5);
    adapter
        .set_protection(None, b"prot", FIBF_SCRIPT_PURE, timestamp(2))
        .unwrap();
    adapter
        .set_protection(Some(dir), b"", 0x0f, timestamp(2))
        .unwrap();
    let modified = Timespec {
        seconds: 1_634_906_064,
        nanoseconds: 680_000_000,
    };
    adapter
        .set_date(None, b"PROT", modified, timestamp(3))
        .unwrap();

    let lock = adapter.locate(None, b"prot", LockAccess::Shared).unwrap();
    let info = adapter.examine_lock(lock).unwrap();
    assert_eq!(info.protection, FIBF_SCRIPT_PURE);
    assert_eq!(info.modified, modified);
    assert_eq!(adapter.examine_lock(dir).unwrap().protection, 0x0f);

    assert_eq!(
        adapter
            .set_protection(None, b"missing", 0, timestamp(4))
            .unwrap_err(),
        ArosError::ObjectNotFound
    );

    // Survives a remount.
    adapter.free_lock(lock).unwrap();
    adapter.free_lock(dir).unwrap();
    let device = adapter.into_vfs().unwrap().into_volume().into_device();
    let vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(vfs, ArosConfig::default());
    let lock = adapter.locate(None, b"prot", LockAccess::Shared).unwrap();
    let info = adapter.examine_lock(lock).unwrap();
    assert_eq!(info.protection, FIBF_SCRIPT_PURE);
    assert_eq!(info.modified, modified);
}
