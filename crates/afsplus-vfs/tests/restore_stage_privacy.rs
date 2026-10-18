// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! Restore preserves historical data/metadata and never overwrites a target.
use afsplus_block::{BlockDevice, BlockError, MemoryBackend as Inner};
use afsplus_core::volume::{AttributeWriteMode, PreservedMetadata, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{Vfs, VfsError};

fn fixture() -> (Vfs<ProbeBackend>, u64, u64, Vec<u8>, PreservedMetadata) {
    let mut image = ProbeBackend::new(4096, 4096);
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

use std::cell::Cell;
use std::rc::Rc;
#[derive(Default)]
struct Fault {
    kind: Cell<u8>,
    at: Cell<usize>,
    count: Cell<usize>,
    armed: Cell<bool>,
    hit: Cell<bool>,
}
thread_local! { static CONTROL: Rc<Fault> = Rc::default(); }
struct ProbeBackend {
    inner: Inner,
    fault: Rc<Fault>,
}
impl ProbeBackend {
    fn new(bs: usize, n: u64) -> Self {
        Self {
            inner: Inner::new(bs, n),
            fault: CONTROL.with(Clone::clone),
        }
    }
    fn check(&self, kind: u8) -> Result<(), BlockError> {
        if self.fault.armed.get() && self.fault.kind.get() == kind {
            let n = self.fault.count.get() + 1;
            self.fault.count.set(n);
            if n >= self.fault.at.get() {
                self.fault.hit.set(true);
                return Err(BlockError::Injected("independent review"));
            }
        }
        Ok(())
    }
}
impl BlockDevice for ProbeBackend {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, l: u64, b: &mut [u8]) -> Result<(), BlockError> {
        self.check(1)?;
        self.inner.read_block(l, b)
    }
    fn write_block(&mut self, l: u64, b: &[u8]) -> Result<(), BlockError> {
        self.check(2)?;
        self.inner.write_block(l, b)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.check(3)?;
        self.inner.flush()
    }
}

#[test]
fn actual_copy_and_cleanup_failure_retains_a_private_stage_after_remount() {
    let mut witnessed = false;
    for at in 8..150 {
        let control = CONTROL.with(Clone::clone);
        control.armed.set(false);
        let (mut v, reader, source, _, _) = fixture();
        control.kind.set(2);
        control.at.set(at);
        control.count.set(0);
        control.armed.set(true);
        let result =
            v.snapshot_restore_file(reader, source, OBJECT_ROOT, "Final", Timespec::default());
        control.armed.set(false);
        let limits = SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        };
        let mut volume = Vfs::mount_with_snapshot_limits(
            v.into_volume().into_device(),
            MountOptions::default(),
            limits,
        )
        .unwrap()
        .into_volume();
        let entries = volume
            .read_directory_page(OBJECT_ROOT, None, 64)
            .unwrap()
            .entries;
        let stage = entries
            .into_iter()
            .find(|e| e.name.starts_with(b".afsplus-restore-"));
        let Some(stage) = stage else {
            continue;
        };
        let object = stage.child_id;
        let mut raw = [0; 4096];
        let n = volume.read_file_at(object, 0, &mut raw).unwrap();
        if !raw[..n].iter().any(|b| *b != 0) {
            continue;
        }
        assert!(result.is_err());
        assert_eq!(
            volume.attribute(object, "system.restore.stage").unwrap(),
            Some(object.to_le_bytes().to_vec())
        );
        let mut v =
            Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits)
                .unwrap();
        assert_eq!(v.stat(object), Err(VfsError::NotFound));
        assert_eq!(
            v.open_file(object, afsplus_vfs::AccessMode::ReadOnly),
            Err(VfsError::NotFound)
        );
        assert_eq!(
            v.lookup(OBJECT_ROOT, std::str::from_utf8(&stage.name).unwrap()),
            Err(VfsError::NotFound)
        );
        assert_eq!(
            v.set_protection(object, 0, Timespec::default()),
            Err(VfsError::NotFound)
        );
        assert_eq!(
            v.set_attributes(
                object,
                &[("system.restore.stage", None)],
                AttributeWriteMode::Upsert,
                Timespec::default()
            ),
            Err(VfsError::NotFound)
        );
        assert_eq!(
            v.clone_file(object, OBJECT_ROOT, "Escaped", Timespec::default()),
            Err(VfsError::NotFound)
        );
        assert_eq!(
            v.rename(
                OBJECT_ROOT,
                std::str::from_utf8(&stage.name).unwrap(),
                OBJECT_ROOT,
                "Escaped",
                false,
                Timespec::default()
            ),
            Err(VfsError::NotFound)
        );
        assert_eq!(
            v.unlink_file(
                OBJECT_ROOT,
                std::str::from_utf8(&stage.name).unwrap(),
                Timespec::default()
            ),
            Err(VfsError::NotFound)
        );
        let h = v.open_directory(OBJECT_ROOT).unwrap();
        let mut cookie = 0;
        let mut names = Vec::new();
        loop {
            let page = v.read_directory(h, cookie, 1).unwrap();
            names.extend(page.entries.into_iter().map(|e| e.name));
            if page.eof {
                break;
            }
            assert_ne!(page.next_cookie, cookie);
            cookie = page.next_cookie;
        }
        assert_eq!(names, vec![b"Captured".to_vec()]);
        witnessed = true;
        break;
    }
    assert!(witnessed, "fixture must reach a real nonempty failed stage");
}

#[test]
fn per_object_root_gate_blocks_tree_children_by_id_and_hides_derived_markers_after_publication() {
    let (v, _, _, _, _) = fixture();
    let mut volume = v.into_volume();
    let root = volume
        .create_directory(OBJECT_ROOT, "PrivateTree", Timespec::default())
        .unwrap();
    volume
        .set_attributes(
            root,
            &[("system.restore.stage", Some(root.to_le_bytes().as_slice()))],
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
    let child = volume
        .create_file_in_directory(root, "Child", b"secret", Timespec::default())
        .unwrap();
    volume
        .set_attributes(
            child,
            &[
                ("system.restore.stage", Some(root.to_le_bytes().as_slice())),
                ("user.keep", Some(b"saved")),
            ],
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 64,
    };
    let mut v =
        Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits)
            .unwrap();
    assert_eq!(v.stat(child), Err(VfsError::NotFound));
    assert_eq!(
        v.open_file(child, afsplus_vfs::AccessMode::ReadOnly),
        Err(VfsError::NotFound)
    );
    assert_eq!(
        v.create_file(root, "Injected", Timespec::default()),
        Err(VfsError::NotFound)
    );
    let mut volume = v.into_volume();
    volume
        .publish_restored_object(
            OBJECT_ROOT,
            "PrivateTree",
            "PublishedTree",
            Timespec::default(),
        )
        .unwrap();
    assert_eq!(
        volume.attribute(root, "system.restore.stage").unwrap(),
        None
    );
    assert_eq!(
        volume.attribute(child, "system.restore.stage").unwrap(),
        Some(root.to_le_bytes().to_vec())
    );
    let mut v =
        Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits)
            .unwrap();
    assert_eq!(v.attribute_names(child).unwrap(), vec!["user.keep"]);
    assert_eq!(v.attribute(child, "system.restore.stage").unwrap(), None);
    assert_eq!(
        v.set_attributes(
            child,
            &[("system.restore.stage", None)],
            AttributeWriteMode::Upsert,
            Timespec::default()
        ),
        Err(VfsError::Invalid)
    );
    let h = v
        .open_file(child, afsplus_vfs::AccessMode::ReadOnly)
        .unwrap();
    let mut bytes = [0; 6];
    assert_eq!(v.read(h, 0, &mut bytes).unwrap(), 6);
    assert_eq!(&bytes, b"secret");
}
