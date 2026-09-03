use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{FuseAdapter, FuseConfig};
use afsplus_vfs::{AccessMode, NodeKind, Vfs, VfsError};

const BLOCK_SIZE: usize = 4096;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn adapter() -> FuseAdapter<MemoryBackend> {
    adapter_with_policy(afsplus_core::NamePolicy::Sensitive)
}

fn adapter_with_policy(policy: afsplus_core::NamePolicy) -> FuseAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(BLOCK_SIZE, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xF5; 16],
            label: "FuseProtocol".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: policy,
            timestamp: timestamp(0),
        },
    )
    .unwrap();
    let vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    FuseAdapter::new(
        vfs,
        FuseConfig {
            uid: 501,
            gid: 20,
            ..FuseConfig::default()
        },
    )
}

fn write_through_adapter() -> FuseAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(BLOCK_SIZE, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xF5; 16],
            label: "FuseWriteThrough".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: timestamp(0),
        },
    )
    .unwrap();
    FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig {
            durable_data_replies: true,
            ..FuseConfig::default()
        },
    )
}

#[test]
fn durable_data_replies_survive_without_a_fuse_fsync_request() {
    let mut fuse = write_through_adapter();
    let (file, handle) = fuse
        .create_file(
            OBJECT_ROOT,
            b"host-file",
            AccessMode::ReadWrite,
            timestamp(1),
        )
        .unwrap();
    fuse.write(handle, 0, b"host durable bytes", timestamp(2))
        .unwrap();

    let device = fuse.into_vfs().into_volume().into_device();
    let mut recovered = FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig {
            durable_data_replies: true,
            ..FuseConfig::default()
        },
    );
    let recovered_file = recovered.lookup(OBJECT_ROOT, b"host-file").unwrap();
    assert_eq!(recovered_file.object_id, file.object_id);
    let recovered_handle = recovered
        .open_file(
            recovered_file.object_id,
            AccessMode::ReadWrite,
            false,
            timestamp(3),
        )
        .unwrap();
    assert_eq!(
        recovered.read(recovered_handle, 0, 32).unwrap(),
        b"host durable bytes"
    );

    recovered
        .truncate(
            recovered_file.object_id,
            Some(recovered_handle),
            4,
            timestamp(4),
        )
        .unwrap();
    let device = recovered.into_vfs().into_volume().into_device();
    let mut recovered = FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig::default(),
    );
    let recovered_file = recovered.lookup(OBJECT_ROOT, b"host-file").unwrap();
    let recovered_handle = recovered
        .open_file(
            recovered_file.object_id,
            AccessMode::ReadOnly,
            false,
            timestamp(5),
        )
        .unwrap();
    assert_eq!(recovered.read(recovered_handle, 0, 32).unwrap(), b"host");

    let mut device = recovered.into_vfs().into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn insensitive_protocol_lookup_rejects_folded_duplicates_and_preserves_spelling() {
    let mut fuse = adapter_with_policy(afsplus_core::NamePolicy::Insensitive);
    let (file, handle) = fuse
        .create_file(
            OBJECT_ROOT,
            "Straße".as_bytes(),
            AccessMode::ReadWrite,
            timestamp(1),
        )
        .unwrap();
    fuse.close(handle).unwrap();
    assert_eq!(
        fuse.lookup(OBJECT_ROOT, b"STRASSE").unwrap().object_id,
        file.object_id
    );
    assert!(matches!(
        fuse.create_file(OBJECT_ROOT, b"strasse", AccessMode::ReadWrite, timestamp(2)),
        Err(VfsError::AlreadyExists)
    ));
    fuse.rename(
        OBJECT_ROOT,
        b"strasse",
        OBJECT_ROOT,
        b"STRASSE",
        false,
        timestamp(3),
    )
    .unwrap();
    let directory = fuse.open_directory(OBJECT_ROOT).unwrap();
    let names: Vec<_> = fuse
        .read_directory(OBJECT_ROOT, directory, 0, 16)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert!(names.contains(&b"STRASSE".to_vec()));
    assert!(!names.contains(&"Straße".as_bytes().to_vec()));
    fuse.close(directory).unwrap();
}

#[test]
fn protocol_slice_survives_remount_and_checker() {
    let mut fuse = adapter();
    let work = fuse
        .create_directory(OBJECT_ROOT, b"work", timestamp(1))
        .unwrap();
    assert_eq!(work.kind, NodeKind::Directory);
    assert_eq!(work.uid, 501);

    let (draft, handle) = fuse
        .create_file(
            work.object_id,
            b"draft",
            AccessMode::ReadWrite,
            timestamp(2),
        )
        .unwrap();
    assert_eq!(draft.kind, NodeKind::File);
    fuse.write(handle, 0, b"hello", timestamp(3)).unwrap();
    fuse.write(handle, 8192, b"tail", timestamp(4)).unwrap();
    assert_eq!(fuse.read(handle, 8188, 8).unwrap(), b"\0\0\0\0tail");
    fuse.fsync(handle).unwrap();
    fuse.truncate(draft.object_id, Some(handle), 5, timestamp(5))
        .unwrap();
    fuse.close(handle).unwrap();

    fuse.rename(
        work.object_id,
        b"draft",
        OBJECT_ROOT,
        b"final",
        false,
        timestamp(6),
    )
    .unwrap();
    let final_file = fuse.lookup(OBJECT_ROOT, b"final").unwrap();
    assert_eq!(final_file.object_id, draft.object_id);

    let second = fuse
        .create_file(
            OBJECT_ROOT,
            b"replacement",
            AccessMode::WriteOnly,
            timestamp(7),
        )
        .unwrap();
    fuse.close(second.1).unwrap();
    fuse.rename(
        OBJECT_ROOT,
        b"replacement",
        OBJECT_ROOT,
        b"final",
        true,
        timestamp(8),
    )
    .unwrap();
    assert_eq!(
        fuse.lookup(OBJECT_ROOT, b"final").unwrap().object_id,
        second.0.object_id
    );
    fuse.sync_filesystem().unwrap();

    let vfs = fuse.into_vfs();
    let mut device = vfs.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);

    let mut remounted = FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig::default(),
    );
    assert_eq!(
        remounted.lookup(OBJECT_ROOT, b"final").unwrap().object_id,
        second.0.object_id
    );
}

#[test]
fn directory_offsets_resume_without_duplicates_and_track_parent() {
    let mut fuse = adapter();
    let directory = fuse
        .create_directory(OBJECT_ROOT, b"directory", timestamp(1))
        .unwrap();
    for (index, name) in [b"alpha".as_slice(), b"beta", b"gamma"]
        .into_iter()
        .enumerate()
    {
        let (_, handle) = fuse
            .create_file(
                directory.object_id,
                name,
                AccessMode::ReadOnly,
                timestamp(index as i64 + 2),
            )
            .unwrap();
        fuse.close(handle).unwrap();
    }

    let handle = fuse.open_directory(directory.object_id).unwrap();
    let mut offset = 0;
    let mut names = Vec::new();
    loop {
        let page = fuse
            .read_directory(directory.object_id, handle, offset, 1)
            .unwrap();
        if page.is_empty() {
            break;
        }
        assert_eq!(page.len(), 1);
        offset = page[0].next_offset;
        names.push(page[0].name.clone());
    }
    assert_eq!(
        names,
        [
            b".".to_vec(),
            b"..".to_vec(),
            b"alpha".to_vec(),
            b"beta".to_vec(),
            b"gamma".to_vec(),
        ]
    );
    assert_eq!(
        fuse.lookup(directory.object_id, b"..").unwrap().object_id,
        OBJECT_ROOT
    );
    fuse.close(handle).unwrap();
}

#[test]
fn names_open_modes_and_stale_handles_fail_explicitly() {
    let mut fuse = adapter();
    assert!(matches!(
        fuse.create_file(OBJECT_ROOT, b"\xff", AccessMode::ReadWrite, timestamp(1)),
        Err(VfsError::Invalid)
    ));

    let (file, handle) = fuse
        .create_file(OBJECT_ROOT, b"file", AccessMode::ReadOnly, timestamp(2))
        .unwrap();
    assert!(matches!(
        fuse.write(handle, 0, b"x", timestamp(3)),
        Err(VfsError::ReadOnly)
    ));
    fuse.close(handle).unwrap();
    assert!(matches!(fuse.read(handle, 0, 1), Err(VfsError::Stale)));

    let write_handle = fuse
        .open_file(file.object_id, AccessMode::WriteOnly, true, timestamp(4))
        .unwrap();
    assert!(matches!(
        fuse.read(write_handle, 0, 1),
        Err(VfsError::Invalid)
    ));
}
