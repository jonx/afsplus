// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! A captured private stage stays private in the snapshot, a tree deeper than
//! reclaim can reach is refused, and an attribute set with no room for the
//! stage marker is refused before anything is written.
use afsplus_block::MemoryBackend;
use afsplus_core::volume::{AttributeWriteMode, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::object::ObjectType;
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{TreeRestoreLimits, Vfs, VfsError};

fn fixture() -> Vfs<MemoryBackend> {
    let mut dev = MemoryBackend::new(4096, 4096);
    mkfs_with_snapshots_and_security_descriptors(
        &mut dev,
        &MkfsParams {
            uuid: [0x94; 16],
            label: "Review".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    mount(dev)
}
fn mount(dev: MemoryBackend) -> Vfs<MemoryBackend> {
    Vfs::mount_with_snapshot_limits(
        dev,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap()
}
fn tree(depth: usize) -> TreeRestoreLimits {
    TreeRestoreLimits {
        max_nodes: 64,
        max_depth: depth,
        max_metadata_bytes: 1 << 20,
    }
}

#[test]
fn a_captured_private_stage_is_neither_listed_nor_restored() {
    let now = Timespec::default();
    let mut vfs = fixture();
    let dir = vfs.create_directory(OBJECT_ROOT, "D", now).unwrap();
    vfs.create_file(dir, "kept", now).unwrap();
    let mut volume = vfs.into_volume();
    // What a crash mid-restore leaves: an unpublished stage in D.
    let stage = volume
        .create_restore_stage(dir, ".afsplus-restore-1", ObjectType::File, None, now)
        .unwrap();
    volume
        .write_file_at(stage, 0, b"partial-unfinished", now)
        .unwrap();
    let mut vfs = mount(volume.into_device());
    let id = vfs.snapshot_create(now).unwrap();
    let reader = vfs.snapshot_open(id, 4).unwrap();

    let listing = vfs.snapshot_directory_open(reader, dir, 4).unwrap();
    let page = vfs.snapshot_directory_read(listing, 16).unwrap();
    let names: Vec<_> = page.entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, vec![b"kept".to_vec()]);
    for refused in [
        vfs.snapshot_stat(reader, stage).map(|_| ()),
        vfs.snapshot_lookup(reader, dir, ".afsplus-restore-1")
            .map(|_| ()),
        vfs.snapshot_read(reader, stage, 0, &mut [0; 8]).map(|_| ()),
        vfs.snapshot_restore_file(reader, stage, OBJECT_ROOT, "leak", now)
            .map(|_| ()),
    ] {
        assert!(matches!(refused, Err(VfsError::NotFound)), "{refused:?}");
    }

    let restored = vfs
        .snapshot_restore_tree(reader, dir, OBJECT_ROOT, "R", tree(8), now)
        .unwrap();
    let handle = vfs.open_directory(restored).unwrap();
    let page = vfs.read_directory(handle, 0, 16).unwrap();
    let names: Vec<_> = page.entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, vec![b"kept".to_vec()]);
}

#[test]
fn a_tree_deeper_than_reclaim_reaches_is_refused_before_writing() {
    let now = Timespec::default();
    let mut vfs = fixture();
    vfs.create_directory(OBJECT_ROOT, "D", now).unwrap();
    let id = vfs.snapshot_create(now).unwrap();
    let reader = vfs.snapshot_open(id, 4).unwrap();
    assert!(matches!(
        vfs.snapshot_restore_tree(reader, OBJECT_ROOT, OBJECT_ROOT, "R", tree(65), now),
        Err(VfsError::Limit(_))
    ));
    assert!(vfs
        .snapshot_restore_tree(reader, OBJECT_ROOT, OBJECT_ROOT, "R", tree(64), now)
        .is_ok());
}

#[test]
fn an_attribute_set_without_room_for_the_marker_is_refused_first() {
    let now = Timespec::default();
    let mut vfs = fixture();
    let file = vfs.create_file(OBJECT_ROOT, "f", now).unwrap();
    // Encodes to exactly the 64 KiB set limit.
    let value = vec![7u8; 65_522];
    vfs.set_attributes(
        file,
        &[("user.x", Some(&value))],
        AttributeWriteMode::Create,
        now,
    )
    .unwrap();
    let id = vfs.snapshot_create(now).unwrap();
    let reader = vfs.snapshot_open(id, 4).unwrap();
    assert!(matches!(
        vfs.snapshot_restore_file(reader, file, OBJECT_ROOT, "g", now),
        Err(VfsError::Limit(_))
    ));
    assert!(matches!(
        vfs.snapshot_restore_tree(reader, OBJECT_ROOT, OBJECT_ROOT, "T", tree(8), now),
        Err(VfsError::Limit(_))
    ));
    vfs.snapshot_close(reader).unwrap();
    // Nothing was staged: maintenance finds no private stage to reclaim.
    let mut cursor = Vec::new();
    loop {
        let report = vfs
            .restore_reclaim_step(
                OBJECT_ROOT,
                &cursor,
                afsplus_vfs::RestoreRecoveryLimits {
                    scan_entries: 64,
                    work_steps: 8,
                    max_depth: 64,
                },
                now,
            )
            .unwrap();
        assert_eq!((report.removed, report.advanced, report.blocked), (0, 0, 0));
        if report.done {
            break;
        }
        cursor = report.cursor;
    }
}
