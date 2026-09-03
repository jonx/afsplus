use afsplus_block::{MemoryBackend, TraceBackend, TraceEvent};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, MountMode, MountOptions};
use afsplus_format::ident::{Identification, RO_COMPAT_ORPHAN_DIRECTORY};
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
    formatted_with_options(shared_extents, 8)
}

fn formatted_with_options(shared_extents: bool, log_slots: u16) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 8192);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [94u8; 16],
            label: "VfsApi".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

#[test]
fn logged_data_fsync_is_visible_before_checkpoint_and_recovers_on_remount() {
    let base = {
        let mut volume = mount(formatted()).unwrap();
        volume
            .create_file_in_root("database", b"old", ts(1))
            .unwrap();
        volume.into_device()
    };
    let mut vfs = Vfs::mount(TraceBackend::new(base), MountOptions::default()).unwrap();
    assert!(vfs.capabilities().contains(Capabilities::LOGGED_DATA_FSYNC));
    let object = vfs.lookup(OBJECT_ROOT, "database").unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();

    vfs.write(handle, 0, b"new durable bytes", ts(2)).unwrap();
    assert_eq!(vfs.stat(object).unwrap().size, 17);
    let mut visible = [0u8; 17];
    assert_eq!(vfs.read(handle, 0, &mut visible).unwrap(), visible.len());
    assert_eq!(&visible, b"new durable bytes");
    vfs.fsync(handle).unwrap();

    let traced = vfs.into_volume().into_device();
    assert_eq!(traced.stats().flushes, 2, "data and record barriers");
    assert!(
        traced
            .events()
            .iter()
            .all(|event| !matches!(event, TraceEvent::Write { lba: 1 | 2 })),
        "fsync must not publish a checkpoint"
    );

    let mut recovered = Vfs::mount(traced.into_inner(), MountOptions::default()).unwrap();
    let object = recovered.lookup(OBJECT_ROOT, "database").unwrap();
    let handle = recovered.open_file(object, AccessMode::ReadOnly).unwrap();
    let mut durable = [0u8; 17];
    assert_eq!(recovered.read(handle, 0, &mut durable).unwrap(), 17);
    assert_eq!(&durable, b"new durable bytes");
    let mut device = recovered.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn volumes_without_the_data_log_capability_keep_checkpoint_fsync_semantics() {
    let base = {
        let mut volume = mount(formatted_with_options(true, 0)).unwrap();
        volume.create_file_in_root("legacy", b"old", ts(1)).unwrap();
        volume.into_device()
    };
    let mut vfs = Vfs::mount(TraceBackend::new(base), MountOptions::default()).unwrap();
    assert!(!vfs.capabilities().contains(Capabilities::LOGGED_DATA_FSYNC));
    let object = vfs.lookup(OBJECT_ROOT, "legacy").unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    vfs.write(handle, 0, b"checkpoint", ts(2)).unwrap();
    vfs.fsync(handle).unwrap();
    assert_eq!(vfs.stat(object).unwrap().size, 10);
    let traced = vfs.into_volume().into_device();
    assert!(traced
        .events()
        .iter()
        .any(|event| matches!(event, TraceEvent::Write { lba: 1 | 2 })));
}

#[test]
fn full_intent_log_falls_back_to_a_checkpoint_without_exposing_a_limit() {
    let base = {
        let mut volume = mount(formatted()).unwrap();
        volume.create_file_in_root("counter", &[0], ts(1)).unwrap();
        volume.into_device()
    };
    let mut vfs = Vfs::mount(TraceBackend::new(base), MountOptions::default()).unwrap();
    let object = vfs.lookup(OBJECT_ROOT, "counter").unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();

    for value in 1u8..=9 {
        vfs.write(handle, 0, &[value], ts(i64::from(value) + 1))
            .unwrap();
        vfs.fsync(handle).unwrap();
    }
    let mut current = [0u8; 1];
    assert_eq!(vfs.read(handle, 0, &mut current).unwrap(), 1);
    assert_eq!(current, [9]);

    let traced = vfs.into_volume().into_device();
    assert!(traced
        .events()
        .iter()
        .any(|event| matches!(event, TraceEvent::Write { lba: 1 | 2 })));
    let mut recovered = Vfs::mount(traced.into_inner(), MountOptions::default()).unwrap();
    let object = recovered.lookup(OBJECT_ROOT, "counter").unwrap();
    let handle = recovered.open_file(object, AccessMode::ReadOnly).unwrap();
    assert_eq!(recovered.read(handle, 0, &mut current).unwrap(), 1);
    assert_eq!(current, [9]);
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
fn open_unlinked_file_keeps_identity_until_the_last_handle_closes() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    assert!(vfs.capabilities().contains(Capabilities::OPEN_UNLINKED));
    let object = vfs.create_file(OBJECT_ROOT, "live", ts(1)).unwrap();
    let writer = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    let reader = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
    vfs.write(writer, 0, b"before", ts(2)).unwrap();
    vfs.fsync(writer).unwrap();

    vfs.unlink_file(OBJECT_ROOT, "live", ts(3)).unwrap();
    assert!(matches!(
        vfs.lookup(OBJECT_ROOT, "live"),
        Err(VfsError::NotFound)
    ));
    assert!(matches!(vfs.stat(object), Err(VfsError::NotFound)));
    assert!(matches!(
        vfs.open_file(object, AccessMode::ReadOnly),
        Err(VfsError::NotFound)
    ));

    let replacement = vfs.create_file(OBJECT_ROOT, "live", ts(4)).unwrap();
    assert_ne!(replacement, object);
    vfs.write(writer, 0, b"after!", ts(5)).unwrap();
    vfs.fsync(writer).unwrap();
    let mut content = [0u8; 6];
    assert_eq!(vfs.read(reader, 0, &mut content).unwrap(), content.len());
    assert_eq!(&content, b"after!");
    vfs.truncate(writer, 3, ts(6)).unwrap();
    vfs.close(writer).unwrap();
    content.fill(0);
    assert_eq!(vfs.read(reader, 0, &mut content).unwrap(), 3);
    assert_eq!(&content[..3], b"aft");

    vfs.close(reader).unwrap();
    assert_eq!(vfs.lookup(OBJECT_ROOT, "live").unwrap(), replacement);
    let mut volume = vfs.into_volume();
    assert!(volume.visible_metadata(object).unwrap().is_none());

    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut remounted = Vfs::mount(dev, MountOptions::default()).unwrap();
    assert_eq!(remounted.lookup(OBJECT_ROOT, "live").unwrap(), replacement);
    assert!(matches!(remounted.stat(object), Err(VfsError::NotFound)));
}

#[test]
fn atomic_replace_preserves_an_open_target_without_exposing_it() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let target = vfs.create_file(OBJECT_ROOT, "target", ts(1)).unwrap();
    let target_handle = vfs.open_file(target, AccessMode::ReadWrite).unwrap();
    vfs.write(target_handle, 0, b"old target", ts(2)).unwrap();
    vfs.fsync(target_handle).unwrap();
    let source = vfs.create_file(OBJECT_ROOT, "incoming", ts(3)).unwrap();
    let source_handle = vfs.open_file(source, AccessMode::ReadWrite).unwrap();
    vfs.write(source_handle, 0, b"new source", ts(4)).unwrap();
    vfs.fsync(source_handle).unwrap();
    vfs.close(source_handle).unwrap();

    vfs.rename(OBJECT_ROOT, "incoming", OBJECT_ROOT, "target", true, ts(5))
        .unwrap();
    assert_eq!(vfs.lookup(OBJECT_ROOT, "target").unwrap(), source);
    assert!(matches!(
        vfs.lookup(OBJECT_ROOT, "incoming"),
        Err(VfsError::NotFound)
    ));
    assert!(matches!(vfs.stat(target), Err(VfsError::NotFound)));
    let mut old = [0u8; 10];
    assert_eq!(vfs.read(target_handle, 0, &mut old).unwrap(), old.len());
    assert_eq!(&old, b"old target");
    vfs.write(target_handle, 0, b"still old!", ts(6)).unwrap();
    vfs.fsync(target_handle).unwrap();
    vfs.close(target_handle).unwrap();

    let mut dev = vfs.into_volume().into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut remounted = Vfs::mount(dev, MountOptions::default()).unwrap();
    assert_eq!(remounted.lookup(OBJECT_ROOT, "target").unwrap(), source);
    assert!(matches!(remounted.stat(target), Err(VfsError::NotFound)));
}

#[test]
fn hard_links_only_enter_orphan_state_on_the_final_open_unlink() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let object = vfs.create_file(OBJECT_ROOT, "first", ts(1)).unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    vfs.write(handle, 0, b"linked", ts(2)).unwrap();
    vfs.fsync(handle).unwrap();
    vfs.link_file(object, OBJECT_ROOT, "second", ts(3)).unwrap();

    vfs.unlink_file(OBJECT_ROOT, "first", ts(4)).unwrap();
    assert_eq!(vfs.lookup(OBJECT_ROOT, "second").unwrap(), object);
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    vfs.unlink_file(OBJECT_ROOT, "second", ts(5)).unwrap();
    assert_eq!(vfs.pending_orphans().unwrap(), 1);
    let mut content = [0u8; 6];
    assert_eq!(vfs.read(handle, 0, &mut content).unwrap(), 6);
    assert_eq!(&content, b"linked");
    vfs.close(handle).unwrap();
    assert_eq!(vfs.pending_orphans().unwrap(), 0);

    let mut dev = vfs.into_volume().into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn legacy_volume_without_orphan_feature_keeps_immediate_delete_contract() {
    let mut dev = formatted();
    let mut ident = Identification::decode(&dev.peek(0)).unwrap();
    ident.features.ro_compat &= !RO_COMPAT_ORPHAN_DIRECTORY;
    dev.apply_raw(0, &ident.encode(BS).unwrap());
    let mut vfs = Vfs::mount(dev, MountOptions::default()).unwrap();
    assert!(!vfs.capabilities().contains(Capabilities::OPEN_UNLINKED));
    let object = vfs.create_file(OBJECT_ROOT, "legacy", ts(1)).unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
    vfs.unlink_file(OBJECT_ROOT, "legacy", ts(2)).unwrap();
    let mut byte = [0u8; 1];
    assert!(matches!(
        vfs.read(handle, 0, &mut byte),
        Err(VfsError::NotFound)
    ));
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
    vfs.close(handle).unwrap();
    vfs.sync_filesystem().unwrap();

    let traced = vfs.into_volume().into_device();
    assert_eq!(traced.stats().writes, 0);
    assert_eq!(traced.stats().flushes, 0);
}

#[test]
fn persistent_data_policy_is_exposed_and_survives_remount() {
    let mut dev = MemoryBackend::new(BS, 8192);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [95u8; 16],
            label: "PolicyApi".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: false,
            data_policy: true,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut vfs = Vfs::mount(dev, MountOptions::default()).unwrap();
    assert!(vfs.capabilities().contains(Capabilities::DATA_POLICY));

    let object = vfs.create_file(OBJECT_ROOT, "database", ts(1)).unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    assert!(!vfs.data_policy(handle).unwrap());
    vfs.set_data_policy(handle, true, ts(2)).unwrap();
    assert!(vfs.data_policy(handle).unwrap());
    vfs.close(handle).unwrap();

    // A read-only handle cannot change the policy.
    let read_only = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
    assert!(matches!(
        vfs.set_data_policy(read_only, false, ts(3)),
        Err(VfsError::ReadOnly)
    ));
    assert!(vfs.data_policy(read_only).unwrap());
    vfs.close(read_only).unwrap();

    // The opt-in is on disk: a fresh mount still reports it.
    let dev = vfs.into_volume().into_device();
    let mut vfs = Vfs::mount(dev, MountOptions::default()).unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
    assert!(vfs.data_policy(handle).unwrap());
}

#[test]
fn data_policy_capability_is_absent_without_the_feature() {
    let base = {
        let mut volume = mount(formatted()).unwrap();
        volume.create_file_in_root("plain", b"old", ts(1)).unwrap();
        volume.into_device()
    };
    let mut vfs = Vfs::mount(TraceBackend::new(base), MountOptions::default()).unwrap();
    assert!(!vfs.capabilities().contains(Capabilities::DATA_POLICY));
    let object = vfs.lookup(OBJECT_ROOT, "plain").unwrap();
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();

    // Leave a durable intent record pending, then prove the refused call has
    // zero side effects: it must not publish the data window into a
    // checkpoint and clear the log behind the caller's back.
    vfs.write(handle, 0, b"durable", ts(2)).unwrap();
    vfs.fsync(handle).unwrap();
    assert!(matches!(
        vfs.set_data_policy(handle, true, ts(3)),
        Err(VfsError::NotSupported)
    ));

    let traced = vfs.into_volume().into_device();
    assert!(
        traced
            .events()
            .iter()
            .all(|event| !matches!(event, TraceEvent::Write { lba: 1 | 2 })),
        "the unsupported policy request published the intent window"
    );
}
