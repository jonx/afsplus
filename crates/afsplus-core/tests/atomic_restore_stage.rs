// SPDX-License-Identifier: BSD-2-Clause
//! Initial private identity must belong to the same checkpoint as its object.
use afsplus_block::MemoryBackend;
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{
    mkfs_with_options, mount_with_snapshot_limits, CoreError, MkfsOptions, MkfsParams,
    MountOptions, NamePolicy,
};
use afsplus_format::{object::ObjectType, Timespec, OBJECT_ROOT};

fn volume(snapshots: bool) -> afsplus_core::volume::Volume<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 2048);
    mkfs_with_options(
        &mut device,
        &MkfsParams {
            uuid: [0x19; 16],
            label: "AtomicStage".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec::default(),
        },
        MkfsOptions {
            persistent_snapshots: snapshots,
        },
    )
    .unwrap();
    mount_with_snapshot_limits(
        device,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap()
}

#[test]
fn root_and_child_are_born_marked_in_one_checkpoint_each() {
    let mut volume = volume(true);
    let generation = volume.checkpoint().generation;
    let root = volume
        .create_restore_stage(
            OBJECT_ROOT,
            "stage",
            ObjectType::Directory,
            None,
            Timespec::default(),
        )
        .unwrap();
    assert_eq!(volume.checkpoint().generation, generation + 1);
    assert_eq!(
        volume.attribute(root, "system.restore.stage").unwrap(),
        Some(root.to_le_bytes().to_vec())
    );
    let child = volume
        .create_restore_stage(
            root,
            "child",
            ObjectType::File,
            Some(root),
            Timespec::default(),
        )
        .unwrap();
    assert_eq!(volume.checkpoint().generation, generation + 2);
    assert_eq!(
        volume.attribute(child, "system.restore.stage").unwrap(),
        Some(root.to_le_bytes().to_vec())
    );
    let device = volume.into_device();
    let mut volume = mount_with_snapshot_limits(
        device,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap();
    assert_eq!(volume.lookup_root("stage").unwrap(), Some(root));
    assert_eq!(
        volume.lookup_in_directory(root, "child").unwrap(),
        Some(child)
    );
    assert_eq!(
        volume.attribute(child, "system.restore.stage").unwrap(),
        Some(root.to_le_bytes().to_vec())
    );
}

#[test]
fn feature_and_private_parent_admission_fail_before_mutation() {
    let mut plain = volume(false);
    let generation = plain.checkpoint().generation;
    assert!(matches!(
        plain.create_restore_stage(
            OBJECT_ROOT,
            "stage",
            ObjectType::File,
            None,
            Timespec::default()
        ),
        Err(CoreError::FeatureDisabled(_))
    ));
    assert_eq!(plain.checkpoint().generation, generation);
    assert_eq!(plain.lookup_root("stage").unwrap(), None);
    plain
        .create_file_in_root("ordinary", b"ordinary", Timespec::default())
        .unwrap();

    let mut volume = volume(true);
    let root = volume
        .create_restore_stage(
            OBJECT_ROOT,
            "stage",
            ObjectType::Directory,
            None,
            Timespec::default(),
        )
        .unwrap();
    let generation = volume.checkpoint().generation;
    assert!(matches!(
        volume.create_restore_stage(
            OBJECT_ROOT,
            "bad-parent",
            ObjectType::File,
            Some(root),
            Timespec::default()
        ),
        Err(CoreError::InvalidMove(_))
    ));
    assert_eq!(volume.checkpoint().generation, generation);
    assert_eq!(volume.lookup_root("bad-parent").unwrap(), None);
}
