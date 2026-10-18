// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! Restore preserves historical data/metadata and never overwrites a target.
use afsplus_block::MemoryBackend;
use afsplus_core::volume::{AttributeWriteMode, PreservedMetadata, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{Vfs, VfsError};

fn fixture() -> (Vfs<MemoryBackend>, u64, u64, Vec<u8>, PreservedMetadata) {
    let mut image = MemoryBackend::new(4096, 4096);
    mkfs_with_snapshots_and_security_descriptors(
        &mut image,
        &MkfsParams {
            uuid: [0x72; 16],
            label: "Restore".into(),
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
    let vfs = Vfs::mount_with_snapshot_limits(
        image,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap();
    let mut volume = vfs.into_volume();
    let bytes: Vec<_> = (0..130_001).map(|i| (i % 251) as u8).collect();
    let object = volume
        .create_file_in_root("Captured", &bytes, Timespec::default())
        .unwrap();
    volume
        .set_attributes(
            object,
            &[("user.test", Some(b"opaque\0value"))],
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
    volume
        .set_object_comment(object, "saved café", Timespec::default())
        .unwrap();
    let metadata = PreservedMetadata {
        protection: 0x2001,
        owner_uid: 65400,
        owner_gid: 4000000000,
        created: Timespec {
            seconds: 100,
            nanoseconds: 123,
        },
        modified: Timespec {
            seconds: 200,
            nanoseconds: 456,
        },
        changed: Timespec {
            seconds: 300,
            nanoseconds: 789,
        },
    };
    volume.restore_object_metadata(object, metadata).unwrap();
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let mut vfs = Vfs::mount_with_snapshot_limits(
        volume.into_device(),
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    (vfs, reader, object, bytes, metadata)
}
#[test]
fn restore_captured_file_in_fresh_storage_and_preserve_all_supported_metadata() {
    let (vfs, reader, source, bytes, metadata) = fixture();
    // Mutate the live source after capture, remount, then restore the old view.
    let snapshot = vfs.snapshot_info(reader).unwrap().id;
    let mut volume = vfs.into_volume();
    volume
        .write_file_at(source, 0, b"live change", Timespec::default())
        .unwrap();
    volume
        .set_object_comment(source, "live change", Timespec::default())
        .unwrap();
    let mut vfs = Vfs::mount_with_snapshot_limits(
        volume.into_device(),
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    let restored = vfs
        .snapshot_restore_file(
            reader,
            source,
            OBJECT_ROOT,
            "Restored",
            Timespec {
                seconds: 900,
                nanoseconds: 0,
            },
        )
        .unwrap();
    assert_ne!(restored, source);
    let stat = vfs.stat(restored).unwrap();
    assert_eq!(stat.owner_uid, 65400);
    assert_eq!(stat.owner_gid, 4000000000);
    assert_eq!(stat.protection, 0x2001);
    assert_eq!(stat.created, metadata.created);
    assert_eq!(stat.modified, metadata.modified);
    assert_eq!(stat.changed, metadata.changed);
    assert_eq!(vfs.comment(restored).unwrap(), "saved café");
    let mut volume = Vfs::mount_with_snapshot_limits(
        vfs.into_volume().into_device(),
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap()
    .into_volume();
    assert_eq!(
        PreservedMetadata::from(volume.visible_metadata(restored).unwrap().unwrap()),
        metadata
    );
    let old_view = volume.snapshot_open(snapshot).unwrap();
    let mut old_bytes = vec![0; bytes.len()];
    assert_eq!(
        volume
            .snapshot_read_file_at(&old_view, source, 0, &mut old_bytes)
            .unwrap(),
        bytes.len()
    );
    assert_eq!(old_bytes, bytes);
    assert_eq!(
        volume.attribute(restored, "user.test").unwrap().unwrap(),
        b"opaque\0value"
    );
    let mut actual = vec![0; bytes.len()];
    assert_eq!(
        volume.read_file_at(restored, 0, &mut actual).unwrap(),
        bytes.len()
    );
    assert_eq!(actual, bytes);
    let entries = volume
        .read_directory_page(OBJECT_ROOT, None, 64)
        .unwrap()
        .entries;
    assert!(entries
        .iter()
        .all(|entry| !entry.name.starts_with(b".afsplus-restore-")));
}
#[test]
fn existing_destination_and_unrepresentable_types_are_refused_without_publication() {
    let (mut vfs, reader, source, bytes, _) = fixture();
    assert_eq!(
        vfs.snapshot_restore_file(reader, source, OBJECT_ROOT, "CAPTURED", Timespec::default()),
        Err(VfsError::AlreadyExists)
    );
    assert_eq!(
        vfs.snapshot_restore_file(
            reader,
            OBJECT_ROOT,
            OBJECT_ROOT,
            "directory",
            Timespec::default()
        ),
        Err(VfsError::NotSupported)
    );
    assert!(matches!(
        vfs.snapshot_restore_file(reader, source, OBJECT_ROOT, "bad/name", Timespec::default()),
        Err(VfsError::Invalid)
    ));
    assert_eq!(
        vfs.lookup(OBJECT_ROOT, "directory"),
        Err(VfsError::NotFound)
    );
    let mut volume = vfs.into_volume();
    let mut actual = vec![0; bytes.len()];
    volume.read_file_at(source, 0, &mut actual).unwrap();
    assert_eq!(actual, bytes);
}
#[test]
fn stale_reader_is_refused() {
    let (mut vfs, reader, source, _, _) = fixture();
    vfs.snapshot_close(reader).unwrap();
    assert_eq!(
        vfs.snapshot_restore_file(reader, source, OBJECT_ROOT, "new", Timespec::default()),
        Err(VfsError::Stale)
    );
    assert_eq!(vfs.lookup(OBJECT_ROOT, "new"), Err(VfsError::NotFound));
}
#[test]
fn security_descriptor_is_refused_before_any_target_or_stage_exists() {
    let (vfs, _, source, _, _) = fixture();
    let mut volume = vfs.into_volume();
    volume
        .set_security_descriptor(source, 7, 1, b"opaque-security", Timespec::default())
        .unwrap();
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let mut vfs = Vfs::mount_with_snapshot_limits(
        volume.into_device(),
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    assert_eq!(
        vfs.snapshot_restore_file(reader, source, OBJECT_ROOT, "denied", Timespec::default()),
        Err(VfsError::NotSupported)
    );
    let mut volume = vfs.into_volume();
    assert_eq!(
        volume.lookup_in_directory(OBJECT_ROOT, "denied").unwrap(),
        None
    );
    let entries = volume
        .read_directory_page(OBJECT_ROOT, None, 64)
        .unwrap()
        .entries;
    assert!(entries
        .iter()
        .all(|entry| !entry.name.starts_with(b".afsplus-restore-")));
}
#[test]
fn sparse_holes_and_unwritten_reservations_survive_fresh_restore() {
    let (vfs, _, _, _, _) = fixture();
    let mut volume = vfs.into_volume();
    let source = volume
        .create_file_in_root("Sparse", &[], Timespec::default())
        .unwrap();
    volume
        .truncate_file(source, 200000, Timespec::default())
        .unwrap();
    volume
        .preallocate_file(source, 4096, 8192, Timespec::default())
        .unwrap();
    volume
        .preallocate_file(source, 262144, 8192, Timespec::default())
        .unwrap();
    volume
        .write_file_at(source, 16384, b"end", Timespec::default())
        .unwrap();
    let ranges = volume.file_allocation_page(source, 0, 64).unwrap().ranges;
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let mut vfs = Vfs::mount_with_snapshot_limits(
        volume.into_device(),
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    let restored = vfs
        .snapshot_restore_file(
            reader,
            source,
            OBJECT_ROOT,
            "SparseRestored",
            Timespec::default(),
        )
        .unwrap();
    let mut volume = vfs.into_volume();
    assert_eq!(
        volume.file_allocation_page(restored, 0, 64).unwrap().ranges,
        ranges
    );
    assert_eq!(
        volume
            .visible_metadata(restored)
            .unwrap()
            .unwrap()
            .size_bytes,
        200000
    );
}
#[test]
fn final_name_never_aliases_staging_name_under_unicode_casefold() {
    let (mut vfs, reader, source, _, metadata) = fixture();
    // Long-s folds to ASCII s. Stage candidate00 must be skipped by the
    // actual volume comparison key, not an ASCII spelling comparison.
    let name = format!(".afſpluſ-restore-{reader:016x}-{source:016x}-00");
    let restored = vfs
        .snapshot_restore_file(
            reader,
            source,
            OBJECT_ROOT,
            &name,
            Timespec {
                seconds: 900,
                nanoseconds: 0,
            },
        )
        .unwrap();
    assert_eq!(vfs.stat(restored).unwrap().changed, metadata.changed);
    let mut volume = vfs.into_volume();
    let entries = volume
        .read_directory_page(OBJECT_ROOT, None, 64)
        .unwrap()
        .entries;
    let final_entry = entries
        .iter()
        .find(|entry| entry.child_id == restored)
        .unwrap();
    assert_eq!(final_entry.name, name.as_bytes());
    // This success assertion verifies spelling and metadata. The independently
    // recorded crash campaign in restore_powercut additionally witnesses every
    // private durable stage key; its filter-removal control must fail.
}
#[test]
fn every_restore_flush_failure_leaves_final_absent_or_fully_preserved_after_remount() {
    use afsplus_block::{FaultBackend, FaultPlan};
    let (vfs, reader, source, bytes, metadata) = fixture();
    let snapshot = vfs.snapshot_info(reader).unwrap().id;
    let image = vfs.into_volume().into_device();
    let mut failures = 0;
    for index in 0..128 {
        let backend = FaultBackend::new(
            image.clone(),
            FaultPlan {
                fail_flush_index: Some(index),
                ..Default::default()
            },
        );
        let mut vfs = Vfs::mount_with_snapshot_limits(
            backend,
            MountOptions::default(),
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 8,
                reclaim_records: 64,
            },
        )
        .unwrap();
        let reader = vfs.snapshot_open(snapshot, 8).unwrap();
        let result = vfs.snapshot_restore_file(
            reader,
            source,
            OBJECT_ROOT,
            "Final",
            Timespec {
                seconds: 900,
                nanoseconds: 0,
            },
        );
        let backend = vfs.into_volume().into_device();
        let tripped = backend.tripped();
        assert_eq!(result.is_err(), tripped, "flush index {index}");
        let mut vfs = Vfs::mount_with_snapshot_limits(
            backend.into_inner(),
            MountOptions::default(),
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 8,
                reclaim_records: 64,
            },
        )
        .unwrap();
        let reader = vfs.snapshot_open(snapshot, 8).unwrap();
        let mut captured = vec![0; bytes.len()];
        assert_eq!(
            vfs.snapshot_read(reader, source, 0, &mut captured).unwrap(),
            bytes.len()
        );
        assert_eq!(captured, bytes);
        match vfs.lookup(OBJECT_ROOT, "Final") {
            Err(VfsError::NotFound) => assert!(tripped),
            Ok(final_id) => {
                let stat = vfs.stat(final_id).unwrap();
                assert_eq!(stat.owner_uid, metadata.owner_uid);
                assert_eq!(stat.owner_gid, metadata.owner_gid);
                assert_eq!(stat.protection, u64::from(metadata.protection));
                assert_eq!(stat.created, metadata.created);
                assert_eq!(stat.modified, metadata.modified);
                assert_eq!(stat.changed, metadata.changed);
                assert_eq!(vfs.comment(final_id).unwrap(), "saved café");
                let mut volume = vfs.into_volume();
                let mut actual = vec![0; bytes.len()];
                volume.read_file_at(final_id, 0, &mut actual).unwrap();
                assert_eq!(actual, bytes);
                assert_eq!(
                    volume.attribute(final_id, "user.test").unwrap().unwrap(),
                    b"opaque\0value"
                );
            }
            error => panic!("unexpected namespace result {error:?} at flush {index}"),
        }
        if !tripped {
            assert!(
                failures >= 15,
                "exercise data,metadata,publication barriers"
            );
            println!("PASS {failures} injected flush failures and final success");
            return;
        }
        failures = index + 1;
    }
    panic!("restore exceeded128 flushes; adjust modeled bound deliberately");
}
