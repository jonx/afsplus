// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! Whole-tree preflight, preservation and all-or-absent final publication.
use afsplus_block::{FaultBackend, FaultPlan, MemoryBackend};
use afsplus_core::volume::{AttributeWriteMode, PreservedMetadata, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{TreeRestoreLimits, Vfs, VfsError};
fn work() -> SnapshotWorkLimits {
    SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 64,
    }
}
fn limits() -> TreeRestoreLimits {
    TreeRestoreLimits {
        max_nodes: 4096,
        max_depth: 64,
        max_metadata_bytes: 4 * 1024 * 1024,
    }
}
fn metadata(seed: i64) -> PreservedMetadata {
    PreservedMetadata {
        protection: 0x2001,
        owner_uid: 1000 + seed as u32,
        owner_gid: 4000000000,
        created: Timespec {
            seconds: seed,
            nanoseconds: 123,
        },
        modified: Timespec {
            seconds: seed + 100,
            nanoseconds: 456,
        },
        changed: Timespec {
            seconds: seed + 200,
            nanoseconds: 789,
        },
    }
}
fn fixture() -> (MemoryBackend, u64, u64, u64) {
    let mut image = MemoryBackend::new(4096, 8192);
    mkfs_with_snapshots_and_security_descriptors(
        &mut image,
        &MkfsParams {
            uuid: [0x74; 16],
            label: "Tree".into(),
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
    let mut volume = Vfs::mount_with_snapshot_limits(image, MountOptions::default(), work())
        .unwrap()
        .into_volume();
    let root = volume
        .create_directory(OBJECT_ROOT, "Saved", Timespec::default())
        .unwrap();
    let empty = volume
        .create_directory(root, "Empty", Timespec::default())
        .unwrap();
    let sub = volume
        .create_directory(root, "Sub", Timespec::default())
        .unwrap();
    let file = volume
        .create_file_in_directory(sub, "é.cfg", b"historical\0prefs", Timespec::default())
        .unwrap();
    let sparse = volume
        .create_file_in_directory(root, "Sparse", &[], Timespec::default())
        .unwrap();
    volume
        .truncate_file(sparse, 8192, Timespec::default())
        .unwrap();
    volume
        .preallocate_file(sparse, 16384, 4096, Timespec::default())
        .unwrap();
    for (object, seed) in [(file, 4), (sparse, 5), (empty, 2), (sub, 3), (root, 1)] {
        volume
            .set_attributes(
                object,
                &[("user.tree", Some(b"opaque\0tree"))],
                AttributeWriteMode::Create,
                Timespec::default(),
            )
            .unwrap();
        volume
            .set_object_comment(object, &format!("comment-{seed}"), Timespec::default())
            .unwrap();
        volume
            .restore_object_metadata(object, metadata(seed))
            .unwrap();
    }
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    volume
        .write_file_at(file, 0, b"LIVE", Timespec::default())
        .unwrap();
    volume
        .create_file_in_directory(root, "new-live-entry", b"not captured", Timespec::default())
        .unwrap();
    (volume.into_device(), snapshot, root, file)
}
fn verify(vfs: Vfs<MemoryBackend>, root: u64) -> Vfs<MemoryBackend> {
    let mut volume = vfs.into_volume();
    let sub = volume
        .lookup_in_directory(root, "Sub")
        .unwrap()
        .expect("published tree missing captured Sub directory");
    let empty = volume
        .lookup_in_directory(root, "Empty")
        .unwrap()
        .expect("published tree missing captured Empty directory");
    let file = volume.lookup_in_directory(sub, "é.cfg").unwrap().unwrap();
    let sparse = volume.lookup_in_directory(root, "Sparse").unwrap().unwrap();
    assert_eq!(
        volume.lookup_in_directory(root, "new-live-entry").unwrap(),
        None
    );
    assert_eq!(
        volume
            .read_directory_page(root, None, 64)
            .unwrap()
            .entries
            .len(),
        3
    );
    assert_eq!(
        volume
            .read_directory_page(empty, None, 64)
            .unwrap()
            .entries
            .len(),
        0
    );
    assert_eq!(
        volume
            .read_directory_page(sub, None, 64)
            .unwrap()
            .entries
            .len(),
        1
    );
    for (object, seed) in [(file, 4), (sparse, 5), (empty, 2), (sub, 3), (root, 1)] {
        assert_eq!(
            PreservedMetadata::from(volume.visible_metadata(object).unwrap().unwrap()),
            metadata(seed)
        );
        assert_eq!(
            volume.object_comment(object).unwrap(),
            format!("comment-{seed}")
        );
        assert_eq!(
            volume.attribute(object, "user.tree").unwrap().unwrap(),
            b"opaque\0tree"
        );
    }
    let mut bytes = [0; 16];
    assert_eq!(volume.read_file_at(file, 0, &mut bytes).unwrap(), 16);
    assert_eq!(bytes, *b"historical\0prefs");
    let ranges = volume.file_allocation_page(sparse, 0, 64).unwrap().ranges;
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].offset, 16384);
    assert_eq!(ranges[0].length, 4096);
    assert!(ranges[0].unwritten);
    Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), work()).unwrap()
}
#[test]
fn nested_empty_unicode_and_sparse_tree_preserves_every_supported_metadata_after_remount() {
    let (image, snapshot, source, _) = fixture();
    let mut vfs = Vfs::mount_with_snapshot_limits(image, MountOptions::default(), work()).unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    let restored = vfs
        .snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            "Restored",
            limits(),
            Timespec {
                seconds: 900,
                nanoseconds: 0,
            },
        )
        .unwrap();
    assert_ne!(restored, source);
    let vfs = Vfs::mount_with_snapshot_limits(
        vfs.into_volume().into_device(),
        MountOptions::default(),
        work(),
    )
    .unwrap();
    let mut vfs = verify(vfs, restored);
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    assert_eq!(
        vfs.snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            "RESTORED",
            limits(),
            Timespec::default()
        ),
        Err(VfsError::AlreadyExists)
    );
}
#[test]
fn node_depth_and_metadata_budgets_refuse_before_the_first_write() {
    let (image, snapshot, source, _) = fixture();
    for limit in [
        TreeRestoreLimits {
            max_nodes: 0,
            ..limits()
        },
        TreeRestoreLimits {
            max_nodes: 4,
            ..limits()
        },
        TreeRestoreLimits {
            max_depth: 2,
            ..limits()
        },
        TreeRestoreLimits {
            max_metadata_bytes: 10,
            ..limits()
        },
    ] {
        let backend = FaultBackend::new(
            image.clone(),
            FaultPlan {
                fail_write_index: Some(0),
                ..Default::default()
            },
        );
        let mut vfs =
            Vfs::mount_with_snapshot_limits(backend, MountOptions::default(), work()).unwrap();
        let reader = vfs.snapshot_open(snapshot, 8).unwrap();
        assert!(matches!(
            vfs.snapshot_restore_tree(
                reader,
                source,
                OBJECT_ROOT,
                "Denied",
                limit,
                Timespec::default()
            ),
            Err(VfsError::Invalid | VfsError::Limit(_))
        ));
        let backend = vfs.into_volume().into_device();
        assert!(
            !backend.tripped(),
            "preflight performed a write for {limit:?}"
        );
        let mut vfs =
            Vfs::mount_with_snapshot_limits(backend.into_inner(), MountOptions::default(), work())
                .unwrap();
        assert_eq!(vfs.lookup(OBJECT_ROOT, "Denied"), Err(VfsError::NotFound));
    }
}
#[test]
fn unsupported_descendant_is_refused_before_any_stage_write() {
    let (image, _, source, file) = fixture();
    let mut volume = Vfs::mount_with_snapshot_limits(image, MountOptions::default(), work())
        .unwrap()
        .into_volume();
    volume
        .set_security_descriptor(file, 7, 1, b"opaque", Timespec::default())
        .unwrap();
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let backend = FaultBackend::new(
        volume.into_device(),
        FaultPlan {
            fail_write_index: Some(0),
            ..Default::default()
        },
    );
    let mut vfs =
        Vfs::mount_with_snapshot_limits(backend, MountOptions::default(), work()).unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    assert_eq!(
        vfs.snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            "Denied",
            limits(),
            Timespec::default()
        ),
        Err(VfsError::NotSupported)
    );
    assert!(!vfs.into_volume().into_device().tripped());
}
#[test]
fn root_stage_name_excludes_unicode_casefold_equivalent_final_name() {
    let (image, snapshot, source, _) = fixture();
    let mut vfs = Vfs::mount_with_snapshot_limits(image, MountOptions::default(), work()).unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    let name = format!(".afſpluſ-restore-{reader:016x}-{source:016x}-00");
    let restored = vfs
        .snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            &name,
            limits(),
            Timespec {
                seconds: 900,
                nanoseconds: 0,
            },
        )
        .unwrap();
    verify(vfs, restored);
}
fn check_tree_flush_failures(alias: bool) {
    let (image, snapshot, source, file) = fixture();
    for (failures, index) in (0..256).enumerate() {
        let backend = FaultBackend::new(
            image.clone(),
            FaultPlan {
                fail_flush_index: Some(index),
                ..Default::default()
            },
        );
        let mut vfs =
            Vfs::mount_with_snapshot_limits(backend, MountOptions::default(), work()).unwrap();
        let reader = vfs.snapshot_open(snapshot, 8).unwrap();
        let name = if alias {
            format!(".afſpluſ-restore-{reader:016x}-{source:016x}-00")
        } else {
            "Final".into()
        };
        let result = vfs.snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            &name,
            limits(),
            Timespec {
                seconds: 900,
                nanoseconds: 0,
            },
        );
        let backend = vfs.into_volume().into_device();
        let tripped = backend.tripped();
        assert_eq!(result.is_err(), tripped, "index {index}");
        let mut vfs =
            Vfs::mount_with_snapshot_limits(backend.into_inner(), MountOptions::default(), work())
                .unwrap();
        let reader = vfs.snapshot_open(snapshot, 8).unwrap();
        let mut bytes = [0; 16];
        assert_eq!(vfs.snapshot_read(reader, file, 0, &mut bytes).unwrap(), 16);
        assert_eq!(bytes, *b"historical\0prefs");
        match vfs.lookup(OBJECT_ROOT, &name) {
            Err(VfsError::NotFound) => assert!(tripped),
            Ok(root) => {
                verify(vfs, root);
            }
            error => panic!("unexpected namespace result {error:?}"),
        }
        if !tripped {
            assert!(failures >= 40);
            println!("PASS {failures} tree flush failures and final success");
            return;
        }
    }
    panic!("tree exceeds256 flushes; change workload bound deliberately");
}

#[test]
fn every_tree_flush_failure_remounts_to_absent_or_complete_final_namespace() {
    check_tree_flush_failures(false);
}
#[test]
fn stage_alias_cannot_expose_a_partial_tree_at_any_flush_failure() {
    check_tree_flush_failures(true);
}
#[test]
fn symlink_and_hardlinked_descendants_are_refused_before_first_write() {
    for symlink in [true, false] {
        let (image, _, source, file) = fixture();
        let mut volume = Vfs::mount_with_snapshot_limits(image, MountOptions::default(), work())
            .unwrap()
            .into_volume();
        if symlink {
            volume
                .create_symlink(source, "Link", "../outside", Timespec::default())
                .unwrap();
        } else {
            volume
                .link_file(file, source, "Alias", Timespec::default())
                .unwrap();
        }
        let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
        let backend = FaultBackend::new(
            volume.into_device(),
            FaultPlan {
                fail_write_index: Some(0),
                ..Default::default()
            },
        );
        let mut vfs =
            Vfs::mount_with_snapshot_limits(backend, MountOptions::default(), work()).unwrap();
        let reader = vfs.snapshot_open(snapshot, 8).unwrap();
        assert_eq!(
            vfs.snapshot_restore_tree(
                reader,
                source,
                OBJECT_ROOT,
                "Denied",
                limits(),
                Timespec::default()
            ),
            Err(VfsError::NotSupported)
        );
        assert!(!vfs.into_volume().into_device().tripped());
    }
}
#[test]
fn exact_requested_node_and_depth_budgets_admit_the_complete_tree() {
    let (image, snapshot, source, _) = fixture();
    let mut vfs = Vfs::mount_with_snapshot_limits(image, MountOptions::default(), work()).unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    let tight = TreeRestoreLimits {
        max_nodes: 5,
        max_depth: 3,
        max_metadata_bytes: 165,
    };
    let restored = vfs
        .snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            "Tight",
            tight,
            Timespec::default(),
        )
        .unwrap();
    verify(vfs, restored);
}
