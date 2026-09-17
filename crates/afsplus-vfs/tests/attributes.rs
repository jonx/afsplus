//! Extended attributes through the portable interface.

use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{
    mkfs, mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MkfsParams, MountOptions,
};
use afsplus_format::Timespec;
use afsplus_vfs::{AttributeWriteMode, Capabilities, Vfs, VfsError};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn params(log_slots: u16) -> MkfsParams {
    MkfsParams {
        uuid: [0xA7; 16],
        label: "Attributes".into(),
        region_size: 4096,
        reclaim_caps: Default::default(),
        log_slots,
        shared_extents: true,
        data_policy: false,
        name_policy: afsplus_core::NamePolicy::Insensitive,
        timestamp: at(0),
    }
}

fn mounted() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(&mut device, &params(8)).unwrap();
    Vfs::mount(device, MountOptions::default()).unwrap()
}

#[test]
fn attributes_persist_in_byte_order_and_batches_are_whole() {
    let mut vfs = mounted();
    assert!(vfs
        .capabilities()
        .contains(Capabilities::EXTENDED_ATTRIBUTES));
    let file = vfs.create_file(vfs.root_object(), "note", at(1)).unwrap();
    assert_eq!(vfs.attribute_names(file).unwrap(), Vec::<String>::new());
    assert_eq!(vfs.attribute(file, "user.kind").unwrap(), None);

    vfs.set_attributes(
        file,
        &[
            ("user.kind", Some(b"text".as_slice())),
            ("aros.tooltype", Some(b"PUBSCREEN=Workbench".as_slice())),
            ("user.empty", Some(b"".as_slice())),
        ],
        AttributeWriteMode::Upsert,
        at(2),
    )
    .unwrap();
    assert_eq!(vfs.stat(file).unwrap().changed, at(2));

    let mut device = vfs.into_volume().into_device();
    assert!(check_device(&mut device).is_clean());
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    let file = vfs.lookup(vfs.root_object(), "note").unwrap();
    assert_eq!(
        vfs.attribute_names(file).unwrap(),
        ["aros.tooltype", "user.empty", "user.kind"]
    );
    assert_eq!(
        vfs.attribute(file, "user.kind").unwrap(),
        Some(b"text".to_vec())
    );
    // Present and empty is not absent.
    assert_eq!(vfs.attribute(file, "user.empty").unwrap(), Some(Vec::new()));

    // One refused change refuses the whole batch: nothing of it is visible.
    assert_eq!(
        vfs.set_attributes(
            file,
            &[
                ("user.kind", Some(b"changed".as_slice())),
                ("user.absent", None),
            ],
            AttributeWriteMode::Upsert,
            at(3),
        ),
        Err(VfsError::NotFound)
    );
    assert_eq!(
        vfs.attribute(file, "user.kind").unwrap(),
        Some(b"text".to_vec())
    );
    assert_eq!(vfs.stat(file).unwrap().changed, at(2));
}

#[test]
fn write_modes_and_name_rules_answer_distinctly() {
    let mut vfs = mounted();
    let file = vfs.create_file(vfs.root_object(), "note", at(1)).unwrap();
    let one = [("user.a", Some(b"1".as_slice()))];
    assert_eq!(
        vfs.set_attributes(file, &one, AttributeWriteMode::Replace, at(2)),
        Err(VfsError::NotFound)
    );
    vfs.set_attributes(file, &one, AttributeWriteMode::Create, at(2))
        .unwrap();
    assert_eq!(
        vfs.set_attributes(file, &one, AttributeWriteMode::Create, at(3)),
        Err(VfsError::AlreadyExists)
    );
    vfs.set_attributes(
        file,
        &[("user.a", Some(b"2".as_slice()))],
        AttributeWriteMode::Replace,
        at(3),
    )
    .unwrap();
    assert_eq!(vfs.attribute(file, "user.a").unwrap(), Some(b"2".to_vec()));
    vfs.set_attributes(file, &[("user.a", None)], AttributeWriteMode::Upsert, at(4))
        .unwrap();
    assert_eq!(vfs.attribute_names(file).unwrap(), Vec::<String>::new());

    // An unregistered namespace never becomes a stored name.
    for name in ["com.apple.quarantine", "user.", "plain", "trusted.x"] {
        assert_eq!(
            vfs.set_attributes(
                file,
                &[(name, Some(b"x".as_slice()))],
                AttributeWriteMode::Upsert,
                at(5),
            ),
            Err(VfsError::Invalid),
            "{name}"
        );
    }
    assert_eq!(
        vfs.attribute(file + 1000, "user.a"),
        Err(VfsError::NotFound)
    );
}

#[test]
fn a_snapshot_bearing_volume_reads_attributes_and_cannot_write_them() {
    let mut device = MemoryBackend::new(4096, 1024);
    mkfs_with_options(
        &mut device,
        &params(0),
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    let volume = mount_with_snapshot_limits(device, MountOptions::default(), limits).unwrap();
    let mut vfs = Vfs::new(volume);
    assert!(!vfs
        .capabilities()
        .contains(Capabilities::EXTENDED_ATTRIBUTES));
    let file = vfs.create_file(vfs.root_object(), "note", at(1)).unwrap();
    assert_eq!(
        vfs.set_attributes(
            file,
            &[("user.a", Some(b"1".as_slice()))],
            AttributeWriteMode::Upsert,
            at(2),
        ),
        Err(VfsError::NotSupported)
    );
    assert_eq!(vfs.attribute_names(file).unwrap(), Vec::<String>::new());
}
