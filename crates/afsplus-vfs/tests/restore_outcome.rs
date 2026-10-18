// SPDX-License-Identifier: BSD-2-Clause
//! Public restore uncertainty is separate from failures before publication.
use afsplus_block::{BlockDevice, BlockError, MemoryBackend};
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{mkfs_with_options, MkfsOptions, MkfsParams, MountOptions, NamePolicy};
use afsplus_format::{object::ObjectType, Timespec, OBJECT_ROOT};
use afsplus_vfs::{Vfs, VfsError};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Default)]
struct Fault {
    flushes: Cell<usize>,
    fail: Cell<usize>,
    sticky: Cell<bool>,
}
struct Device {
    memory: MemoryBackend,
    fault: Rc<Fault>,
}
impl BlockDevice for Device {
    fn block_size(&self) -> usize {
        self.memory.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.memory.total_blocks()
    }
    fn read_block(&mut self, lba: u64, bytes: &mut [u8]) -> Result<(), BlockError> {
        self.memory.read_block(lba, bytes)
    }
    fn write_block(&mut self, lba: u64, bytes: &[u8]) -> Result<(), BlockError> {
        self.memory.write_block(lba, bytes)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        let count = self.fault.flushes.get() + 1;
        self.fault.flushes.set(count);
        let fail = self.fault.fail.get();
        if fail != 0 && (count == fail || self.fault.sticky.get() && count >= fail) {
            return Err(BlockError::Injected("outcome witness"));
        }
        self.memory.flush()
    }
}
fn limits() -> SnapshotWorkLimits {
    SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 64,
    }
}
fn fixture() -> (Vfs<Device>, u64, u64, Rc<Fault>) {
    let fault = Rc::new(Fault::default());
    let mut device = Device {
        memory: MemoryBackend::new(4096, 2048),
        fault: fault.clone(),
    };
    mkfs_with_options(
        &mut device,
        &MkfsParams {
            uuid: [0x51; 16],
            label: "Outcome".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec::default(),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let vfs = Vfs::mount_with_snapshot_limits(device, MountOptions::default(), limits()).unwrap();
    let mut volume = vfs.into_volume();
    let source = volume
        .create_file_in_root("source", b"historical bytes", Timespec::default())
        .unwrap();
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let mut vfs =
        Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits())
            .unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    fault.flushes.set(0);
    (vfs, reader, source, fault)
}
fn run(vfs: &mut Vfs<Device>, reader: u64, source: u64) -> Result<u64, VfsError> {
    vfs.snapshot_restore_file(reader, source, OBJECT_ROOT, "restored", Timespec::default())
}

#[test]
fn final_checkpoint_and_following_barrier_report_uncertain_but_prepublication_io_does_not() {
    let (mut baseline, reader, source, fault) = fixture();
    run(&mut baseline, reader, source).unwrap();
    let total = fault.flushes.get();
    assert!(total >= 4);
    // Last flush is the public postpublication hook; the preceding one is
    // the core checkpoint commit barrier, before it adoption has not occurred.
    for ordinal in [total - 1, total] {
        let (mut vfs, reader, source, fault) = fixture();
        fault.fail.set(ordinal);
        assert!(
            matches!(
                run(&mut vfs, reader, source),
                Err(VfsError::PublicationUncertain { .. })
            ),
            "flush {ordinal}/{total}"
        );
        fault.fail.set(0);
        let mut mounted = Vfs::mount_with_snapshot_limits(
            vfs.into_volume().into_device(),
            MountOptions::default(),
            limits(),
        )
        .unwrap();
        let object = mounted.lookup(OBJECT_ROOT, "restored").unwrap();
        let mut volume = mounted.into_volume();
        let mut bytes = [0; 16];
        assert_eq!(volume.read_file_at(object, 0, &mut bytes).unwrap(), 16);
        assert_eq!(&bytes, b"historical bytes");
    }
    for ordinal in [1, total - 2, total - 3] {
        let (mut vfs, reader, source, fault) = fixture();
        fault.fail.set(ordinal);
        fault.sticky.set(true); // cleanup also fails: preserve original error.
        assert!(
            matches!(run(&mut vfs, reader, source), Err(VfsError::Io(_))),
            "flush {ordinal}/{total}"
        );
    }
    eprintln!("restore success flushes={total}; explicit uncertainty at checkpoint/postpublication barriers");
}

#[test]
fn public_restore_rejects_an_active_stage_parent_without_writes() {
    let (vfs, _, source, fault) = fixture();
    let mut volume = vfs.into_volume();
    let private = volume
        .create_restore_stage(
            OBJECT_ROOT,
            "private",
            ObjectType::Directory,
            None,
            Timespec::default(),
        )
        .unwrap();
    let snapshot = volume.snapshot_list(0, 1).unwrap().entries[0].id;
    let mut vfs =
        Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits())
            .unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    fault.flushes.set(0);
    assert!(matches!(
        vfs.snapshot_restore_file(reader, source, private, "child", Timespec::default()),
        Err(VfsError::NotFound)
    ));
    assert!(matches!(
        vfs.snapshot_restore_tree(
            reader,
            source,
            private,
            "tree",
            afsplus_vfs::TreeRestoreLimits {
                max_nodes: 16,
                max_depth: 8,
                max_metadata_bytes: 4 * 1024 * 1024,
            },
            Timespec::default()
        ),
        Err(VfsError::NotFound)
    ));
    assert_eq!(fault.flushes.get(), 0);
}
