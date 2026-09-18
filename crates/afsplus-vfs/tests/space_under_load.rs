//! Space a delete gave back is there for the next write, even before the
//! background has reclaimed it.
//!
//! A host that runs maintenance in the background returns a deleted file's
//! blocks some time after the delete. Under steady load that time grew: three
//! programs on one mounted volume drove its free space from four hundred
//! megabytes to forty and back, and a write that arrived in a trough failed
//! with no space although most of the volume was deleted data waiting to be
//! reclaimed. The failure lost the write window, and with it data that had
//! been acknowledged.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Vfs};

const BLOCK: usize = 4096;
const BLOCKS: u64 = 4096;

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

/// A volume run the way the macOS driver runs it: no maintenance inside
/// operations, every write made durable before it is answered.
fn volume() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(BLOCK, BLOCKS);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x5A; 16],
            label: "Load".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_inline_maintenance(false);
    vfs
}

fn content(seed: u8, length: usize) -> Vec<u8> {
    (0..length).map(|index| seed ^ (index as u8)).collect()
}

fn write_file(vfs: &mut Vfs<MemoryBackend>, name: &str, data: &[u8], now: i64) {
    let id = vfs.create_file(OBJECT_ROOT, name, at(now)).unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    for (index, piece) in data.chunks(256 * 1024).enumerate() {
        vfs.write(handle, (index * 256 * 1024) as u64, piece, at(now))
            .unwrap_or_else(|error| {
                let stats = vfs.statfs();
                panic!("writing {name} at piece {index}: {error}; free {} available {} headroom {} orphans {} reclaim {}", stats.free_blocks, stats.available_blocks, stats.emergency_headroom_blocks, vfs.pending_orphans().unwrap(), vfs.reclaim_pending_blocks())
            });
        vfs.fsync(handle)
            .unwrap_or_else(|error| panic!("making {name} durable at piece {index}: {error}"));
    }
    vfs.close(handle).unwrap();
}

fn read_file(vfs: &mut Vfs<MemoryBackend>, name: &str) -> Vec<u8> {
    let id = vfs.lookup(OBJECT_ROOT, name).unwrap();
    let handle = vfs.open_file(id, AccessMode::ReadOnly).unwrap();
    let mut data = Vec::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = vfs.read(handle, data.len() as u64, &mut buffer).unwrap();
        if read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..read]);
    }
    vfs.close(handle).unwrap();
    data
}

#[test]
fn a_write_can_use_the_space_of_a_file_deleted_before_it() {
    let mut vfs = volume();
    let size = (BLOCKS as usize * BLOCK) * 6 / 10;
    write_file(&mut vfs, "first", &content(1, size), 1);
    vfs.unlink_file(OBJECT_ROOT, "first", at(2)).unwrap();
    assert!(
        vfs.pending_orphans().unwrap() + vfs.reclaim_pending_blocks() > 0,
        "the delete left its space for maintenance, as a background host does"
    );

    let second = content(2, size);
    write_file(&mut vfs, "second", &second, 3);
    assert_eq!(read_file(&mut vfs, "second"), second);

    vfs.sync_filesystem().unwrap();
    let device = vfs.into_volume().into_device();
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    assert_eq!(
        read_file(&mut vfs, "second"),
        second,
        "and it is on the volume"
    );
}
