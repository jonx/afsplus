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
