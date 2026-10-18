// SPDX-License-Identifier: BSD-2-Clause
//! Deterministic I/O comparison fixture, using a memory device without cache.
use afsplus_block::{BlockDevice, BlockError, MemoryBackend};
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{mkfs_with_options, MkfsOptions, MkfsParams, MountOptions, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{TreeRestoreLimits, Vfs};
use std::{cell::Cell, rc::Rc, time::Instant};
#[derive(Default)]
struct Counts {
    reads: Cell<u64>,
    writes: Cell<u64>,
    flushes: Cell<u64>,
}
struct Device {
    memory: MemoryBackend,
    counts: Rc<Counts>,
}
impl BlockDevice for Device {
    fn block_size(&self) -> usize {
        self.memory.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.memory.total_blocks()
    }
    fn read_block(&mut self, lba: u64, out: &mut [u8]) -> Result<(), BlockError> {
        self.counts.reads.set(self.counts.reads.get() + 1);
        self.memory.read_block(lba, out)
    }
    fn write_block(&mut self, lba: u64, input: &[u8]) -> Result<(), BlockError> {
        self.counts.writes.set(self.counts.writes.get() + 1);
        self.memory.write_block(lba, input)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.counts.flushes.set(self.counts.flushes.get() + 1);
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
fn run(tree: bool) {
    let counts = Rc::new(Counts::default());
    let mut device = Device {
        memory: MemoryBackend::new(4096, 4096),
        counts: counts.clone(),
    };
    mkfs_with_options(
        &mut device,
        &MkfsParams {
            uuid: [0x81; 16],
            label: "IO".into(),
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
    let bytes: Vec<_> = (0..130001).map(|i| (i % 251) as u8).collect();
    let source = if tree {
        let root = volume
            .create_directory(OBJECT_ROOT, "captured", Timespec::default())
            .unwrap();
        volume
            .create_file_in_directory(root, "large", &bytes, Timespec::default())
            .unwrap();
        volume
            .create_file_in_directory(root, "small", &[17; 139], Timespec::default())
            .unwrap();
        volume
            .create_file_in_directory(root, "empty-file", &[], Timespec::default())
            .unwrap();
        volume
            .create_directory(root, "empty-directory", Timespec::default())
            .unwrap();
        root
    } else {
        volume
            .create_file_in_root("captured", &bytes, Timespec::default())
            .unwrap()
    };
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let mut vfs =
        Vfs::mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), limits())
            .unwrap();
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    counts.reads.set(0);
    counts.writes.set(0);
    counts.flushes.set(0);
    let start = Instant::now();
    if tree {
        vfs.snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            "restored",
            TreeRestoreLimits {
                max_nodes: 16,
                max_depth: 8,
                max_metadata_bytes: 4 * 1024 * 1024,
            },
            Timespec::default(),
        )
        .unwrap();
    } else {
        vfs.snapshot_restore_file(reader, source, OBJECT_ROOT, "restored", Timespec::default())
            .unwrap();
    }
    println!("RESTORE_IO kind={} nodes={} payload={} block_size=4096 reads={} writes={} read_bytes={} write_bytes={} flushes={} elapsed_us={}",if tree {"tree"} else {"file"},if tree {5} else {1},if tree {130140} else {130001},counts.reads.get(),counts.writes.get(),counts.reads.get()*4096,counts.writes.get()*4096,counts.flushes.get(),start.elapsed().as_micros());
}
#[test]
fn comparable_standalone_and_five_object_tree_io() {
    run(false);
    run(true);
}
