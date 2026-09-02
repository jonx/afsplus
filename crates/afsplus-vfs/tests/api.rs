use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, MountMode, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Capabilities, NodeKind, Vfs, VfsError};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    formatted_with_shared_extents(true)
}

fn formatted_with_shared_extents(shared_extents: bool) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 8192);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [94u8; 16],
            label: "VfsApi".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

#[test]
fn clone_capabilities_are_volume_gated_and_filesystem_neutral() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let capabilities = vfs.capabilities();
    assert!(capabilities.contains(Capabilities::CLONE_FILE));
    assert!(capabilities.contains(Capabilities::CLONE_RANGE));

    let source = vfs.create_file(OBJECT_ROOT, "source", ts(1)).unwrap();
    let source_handle = vfs.open_file(source, AccessMode::ReadWrite).unwrap();
    let source_bytes = vec![0x37u8; 2 * BS];
    vfs.write(source_handle, 0, &source_bytes, ts(2)).unwrap();
    let clone = vfs.clone_file(source, OBJECT_ROOT, "clone", ts(3)).unwrap();
    let clone_handle = vfs.open_file(clone, AccessMode::ReadWrite).unwrap();
    let replacement = vec![0x81u8; BS];
    let replacement_file = vfs.create_file(OBJECT_ROOT, "replacement", ts(4)).unwrap();
    let replacement_handle = vfs
        .open_file(replacement_file, AccessMode::ReadWrite)
        .unwrap();
    vfs.write(replacement_handle, 0, &replacement, ts(5))
        .unwrap();
    vfs.clone_range(
        replacement_handle,
        0,
        clone_handle,
        BS as u64,
        BS as u64,
        ts(6),
    )
    .unwrap();

    let mut clone_bytes = vec![0u8; 2 * BS];
    assert_eq!(
        vfs.read(clone_handle, 0, &mut clone_bytes).unwrap(),
        clone_bytes.len()
    );
    let mut expected = source_bytes.clone();
    expected[BS..].copy_from_slice(&replacement);
    assert_eq!(clone_bytes, expected);
    let mut original = vec![0u8; 2 * BS];
    vfs.read(source_handle, 0, &mut original).unwrap();
    assert_eq!(
        original, source_bytes,
        "range clone must not modify its source"
    );

    let mut dev = vfs.into_volume().into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    let mut unsupported = Vfs::mount(
        formatted_with_shared_extents(false),
        MountOptions::default(),
    )
    .unwrap();
    assert!(!unsupported
        .capabilities()
        .contains(Capabilities::CLONE_FILE));
    assert!(!unsupported
        .capabilities()
        .contains(Capabilities::CLONE_RANGE));
    let plain = unsupported
        .create_file(OBJECT_ROOT, "plain", ts(1))
        .unwrap();
    assert!(matches!(
        unsupported.clone_file(plain, OBJECT_ROOT, "copy", ts(2)),
        Err(VfsError::NotSupported)
    ));
}

#[test]
fn handle_api_covers_the_mountable_alpha_operation_slice() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    assert!(vfs.capabilities().contains(Capabilities::IO_64BIT));
    assert!(vfs.capabilities().contains(Capabilities::PAGED_DIRECTORIES));
    let stats = vfs.statfs();
    assert_eq!(stats.block_size, BS as u32);
    assert_eq!(stats.total_blocks, 8192);
    assert!(stats.case_sensitive);
    assert_eq!(stats.unicode_version, [16, 0, 0]);

    let directory = vfs.create_directory(OBJECT_ROOT, "work", ts(1)).unwrap();
    let object = vfs.create_file(directory, "draft", ts(2)).unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    assert_eq!(vfs.write(handle, 0, b"hello", ts(3)).unwrap(), 5);
    assert_eq!(vfs.write(handle, 8193, b"tail", ts(4)).unwrap(), 4);

    let mut head = [0u8; 5];
    assert_eq!(vfs.read(handle, 0, &mut head).unwrap(), 5);
    assert_eq!(&head, b"hello");
    let mut sparse = [0xAA; 8];
    assert_eq!(vfs.read(handle, 8189, &mut sparse).unwrap(), 8);
    assert_eq!(&sparse[..4], &[0; 4]);
    assert_eq!(&sparse[4..], b"tail");
    vfs.fsync(handle).unwrap();
    vfs.truncate(handle, 5, ts(5)).unwrap();
    vfs.close(handle).unwrap();
    assert!(matches!(
        vfs.read(handle, 0, &mut head),
        Err(VfsError::Stale)
    ));

    vfs.rename(directory, "draft", OBJECT_ROOT, "final", false, ts(6))
        .unwrap();
    assert_eq!(vfs.lookup(OBJECT_ROOT, "final").unwrap(), object);
    assert_eq!(vfs.stat(object).unwrap().kind, NodeKind::File);

    let root = vfs.open_directory(OBJECT_ROOT).unwrap();
    let page = vfs.read_directory(root, 0, 1).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(!page.eof);
    let tail = vfs.read_directory(root, page.next_cookie, 16).unwrap();
    assert_eq!(tail.entries.len(), 1);
    assert!(tail.eof);
    vfs.close(root).unwrap();

    vfs.link_file(object, directory, "linked", ts(7)).unwrap();
    vfs.unlink_file(OBJECT_ROOT, "final", ts(8)).unwrap();
    assert_eq!(vfs.lookup(directory, "linked").unwrap(), object);
    vfs.sync_filesystem().unwrap();

    let mut dev = vfs.into_volume().into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn no_changes_vfs_remains_readable_and_issues_no_writes_or_flushes() {
    let dev = {
        let mut volume = mount(formatted()).unwrap();
        volume
            .create_file_in_root("existing", b"content", ts(1))
            .unwrap();
        volume.into_device()
    };
    let traced = TraceBackend::new(dev);
    let mut vfs = Vfs::mount(
        traced,
        MountOptions {
            mode: MountMode::NoChanges,
        },
    )
    .unwrap();
    let object = vfs.lookup(OBJECT_ROOT, "existing").unwrap();
    assert!(matches!(
        vfs.open_file(object, AccessMode::ReadWrite),
        Err(VfsError::ReadOnly)
    ));
    let handle = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
    let mut data = [0u8; 7];
    assert_eq!(vfs.read(handle, 0, &mut data).unwrap(), data.len());
    assert_eq!(&data, b"content");
    vfs.fsync(handle).unwrap();
    vfs.sync_filesystem().unwrap();

    let traced = vfs.into_volume().into_device();
    assert_eq!(traced.stats().writes, 0);
    assert_eq!(traced.stats().flushes, 0);
}
