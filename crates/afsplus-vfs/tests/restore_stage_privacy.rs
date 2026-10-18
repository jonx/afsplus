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
fn every_actual_write_failure_checks_empty_and_nonempty_stage_ids_after_remount() {
    let control = CONTROL.with(Clone::clone);
    control.armed.set(false);
    let (vfs, _, _, _, _) = fixture();
    let original = vfs.into_volume().checkpoint().next_object_id;
    // Measure this real workload once; every issued write receives a failure
    // case. The failure persists during cleanup, rather than disappearing.
    let (mut v, reader, source, _, _) = fixture();
    control.kind.set(2);
    control.at.set(usize::MAX);
    control.count.set(0);
    control.armed.set(true);
    v.snapshot_restore_file(reader, source, OBJECT_ROOT, "Final", Timespec::default())
        .unwrap();
    control.armed.set(false);
    let writes = control.count.get();
    assert!(writes > 20);
    let mut private = 0;
    let mut empty = 0;
    let mut nonempty = 0;
    for at in 1..=writes {
        control.armed.set(false);
        let (mut v, reader, source, bytes, metadata) = fixture();
        control.kind.set(2);
        control.at.set(at);
        control.count.set(0);
        control.hit.set(false);
        control.armed.set(true);
        let result =
            v.snapshot_restore_file(reader, source, OBJECT_ROOT, "Final", Timespec::default());
        control.armed.set(false);
        assert!(control.hit.get(), "write {at}/{writes} was not reached");
        assert!(result.is_err(), "injected write {at} did not fail restore");
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
        let next = volume.checkpoint().next_object_id;
        let mut allocated = Vec::new();
        for object in original..next {
            if let Some(stat) = volume.visible_metadata(object).unwrap() {
                allocated.push((object, stat.size_bytes));
            }
        }
        let mut v =
            Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits)
                .unwrap();
        let published = match v.lookup(OBJECT_ROOT, "Final") {
            Ok(object) => {
                let stat = v.stat(object).unwrap();
                assert_eq!(
                    (stat.created, stat.modified, stat.changed),
                    (metadata.created, metadata.modified, metadata.changed)
                );
                assert_eq!(
                    (stat.owner_uid, stat.owner_gid),
                    (metadata.owner_uid, metadata.owner_gid)
                );
                assert_eq!(v.comment(object).unwrap(), "saved café");
                assert_eq!(
                    v.attribute(object, "user.test").unwrap(),
                    Some(b"opaque\0value".to_vec())
                );
                let file = v
                    .open_file(object, afsplus_vfs::AccessMode::ReadOnly)
                    .unwrap();
                let mut actual = vec![0; bytes.len() + 1];
                assert_eq!(v.read(file, 0, &mut actual).unwrap(), bytes.len());
                assert_eq!(&actual[..bytes.len()], bytes.as_slice());
                v.close(file).unwrap();
                Some(object)
            }
            Err(VfsError::NotFound) => None,
            other => panic!("unexpected final state at write{at}: {other:?}"),
        };
        for (object, size) in allocated {
            if Some(object) != published {
                private += 1;
                empty += usize::from(size == 0);
                nonempty += usize::from(size != 0);
                assert_eq!(
                    v.stat(object),
                    Err(VfsError::NotFound),
                    "direct ID leak at write {at}, id{object}, size{size}"
                );
                assert_eq!(
                    v.open_file(object, afsplus_vfs::AccessMode::ReadOnly),
                    Err(VfsError::NotFound)
                );
                assert_eq!(
                    v.create_file(object, "Injected", Timespec::default()),
                    Err(VfsError::NotFound)
                );
                assert_eq!(
                    v.set_protection(object, 0, Timespec::default()),
                    Err(VfsError::NotFound)
                );
                assert_eq!(
                    v.clone_file(object, OBJECT_ROOT, "Escaped", Timespec::default()),
                    Err(VfsError::NotFound)
                );
            }
        }
        let h = v.open_directory(OBJECT_ROOT).unwrap();
        let page = v.read_directory(h, 0, 64).unwrap();
        assert!(page.eof);
        assert!(page
            .entries
            .iter()
            .all(|entry| entry.name == b"Captured" || entry.name == b"Final"));
        v.close(h).unwrap();
    }
    assert!(private > 0 && empty > 0 && nonempty > 0);
    println!("PASS all{writes} persistent write failures: {private} private ID checks, {empty} empty, {nonempty} nonempty");
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

#[test]
fn malformed_root_identifiers_fail_closed_but_a_deleted_allocated_root_is_inactive() {
    let (v, _, _, _, _) = fixture();
    let mut volume = v.into_volume();
    let object = volume
        .create_file_in_root("Marked", b"payload", Timespec::default())
        .unwrap();
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 64,
    };
    let never_allocated = volume.checkpoint().next_object_id;
    for root in [
        0,
        OBJECT_ROOT,
        afsplus_format::OBJECT_FIRST_DYNAMIC - 1,
        never_allocated,
        u64::MAX,
    ] {
        volume
            .set_attributes(
                object,
                &[("system.restore.stage", Some(root.to_le_bytes().as_slice()))],
                AttributeWriteMode::Upsert,
                Timespec::default(),
            )
            .unwrap();
        let mut v =
            Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits)
                .unwrap();
        assert!(
            matches!(v.stat(object), Err(VfsError::Corrupt(_))),
            "root {root} must fail closed"
        );
        assert!(matches!(
            v.open_file(object, afsplus_vfs::AccessMode::ReadOnly),
            Err(VfsError::Corrupt(_))
        ));
        volume = v.into_volume();
    }
    let root = volume
        .create_directory(OBJECT_ROOT, "ExpiredRoot", Timespec::default())
        .unwrap();
    volume
        .set_attributes(
            object,
            &[("system.restore.stage", Some(root.to_le_bytes().as_slice()))],
            AttributeWriteMode::Upsert,
            Timespec::default(),
        )
        .unwrap();
    volume
        .remove_directory(OBJECT_ROOT, "ExpiredRoot", Timespec::default())
        .unwrap();
    let mut v =
        Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits)
            .unwrap();
    let h = v
        .open_file(object, afsplus_vfs::AccessMode::ReadOnly)
        .unwrap();
    let mut bytes = [0; 7];
    assert_eq!(v.read(h, 0, &mut bytes).unwrap(), 7);
    assert_eq!(&bytes, b"payload");
    assert!(v.attribute_names(object).unwrap().is_empty());
}

#[test]
fn malformed_child_marker_does_not_make_healthy_siblings_unlistable() {
    let (v, _, _, _, _) = fixture();
    let mut volume = v.into_volume();
    let bad = volume
        .create_directory(OBJECT_ROOT, "A-malformed", Timespec::default())
        .unwrap();
    volume
        .set_attributes(
            bad,
            &[("system.restore.stage", Some(&[1, 2, 3]))],
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
    volume
        .create_file_in_root("B-visible", b"healthy", Timespec::default())
        .unwrap();
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 64,
    };
    let mut v =
        Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits)
            .unwrap();
    assert!(matches!(v.stat(bad), Err(VfsError::Corrupt(_))));
    assert!(matches!(v.open_directory(bad), Err(VfsError::Corrupt(_))));
    assert!(matches!(
        v.lookup(OBJECT_ROOT, "A-malformed"),
        Err(VfsError::Corrupt(_))
    ));
    assert!(matches!(
        v.create_file(bad, "Injected", Timespec::default()),
        Err(VfsError::Corrupt(_))
    ));
    let handle = v.open_directory(OBJECT_ROOT).unwrap();
    let first = v.read_directory(handle, 0, 1).unwrap();
    assert_eq!(
        first
            .entries
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<_>>(),
        vec![b"B-visible".to_vec()]
    );
    assert!(!first.eof);
    let second = v.read_directory(handle, first.next_cookie, 1).unwrap();
    assert_eq!(
        second
            .entries
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<_>>(),
        vec![b"Captured".to_vec()]
    );
    assert!(second.eof);
    v.close(handle).unwrap();
}

#[test]
fn nonsnapshot_stat_does_not_read_restore_marker_attributes() {
    use afsplus_block::TraceBackend;
    let mut image = Inner::new(4096, 4096);
    afsplus_core::mkfs(
        &mut image,
        &MkfsParams {
            uuid: [0x86; 16],
            label: "NoSnapshotGuard".into(),
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
    let mut volume = afsplus_core::mount_with_options(image, MountOptions::default()).unwrap();
    let file = volume
        .create_file_in_root("Ordinary", b"live", Timespec::default())
        .unwrap();
    let keys: Vec<_> = (0..32).map(|n| format!("user.inventory-{n:02}")).collect();
    let value = [42u8; 128];
    let attributes: Vec<_> = keys
        .iter()
        .map(|key| (key.as_str(), Some(value.as_slice())))
        .collect();
    volume
        .set_attributes(
            file,
            &attributes,
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
    let image = volume.into_device();
    let mut core =
        afsplus_core::mount_with_options(TraceBackend::new(image.clone()), MountOptions::default())
            .unwrap();
    core.device_mut().reset();
    assert!(!core.orphan_object(file).unwrap());
    core.visible_metadata(file).unwrap().unwrap();
    let expected = core.device_mut().stats();
    let mut core =
        afsplus_core::mount_with_options(TraceBackend::new(image), MountOptions::default())
            .unwrap();
    core.device_mut().reset();
    let mut vfs = Vfs::new(core);
    assert_eq!(vfs.stat(file).unwrap().size, 4);
    let actual = vfs.into_volume().into_device().stats();
    assert_eq!(
        actual, expected,
        "disabled snapshots must add no attribute-guard block I/O"
    );
    println!("PASS nonsnapshot stat I/O equals core orphan+metadata path: reads={} bytes={} writes={} flushes={}",actual.reads,actual.bytes_read,actual.writes,actual.flushes);
}
