//! A host that serves several clients can take maintenance off their requests.
//!
//! Deleting a large or fragmented file leaves work behind: the orphan's extents
//! to release and the reclaim ledger to drain. Done inline, that work runs on
//! the request that happened to close the last handle, and every other client
//! waits behind it. With inline maintenance off, no operation does any of it,
//! and the host drains it one transaction at a time between requests.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Vfs};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn mounted() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 16384);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x2C; 16],
            label: "Background".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    Vfs::mount(device, MountOptions::default()).unwrap()
}

fn drain(vfs: &mut Vfs<MemoryBackend>) -> u32 {
    let mut steps = 0;
    while vfs.maintenance_step(at(500 + i64::from(steps))).unwrap() {
        steps += 1;
        assert!(steps < 100_000, "maintenance never finishes");
    }
    steps
}

const BLOCKS: u64 = 300;

/// Two files written in alternating blocks, so that each one is fragmented.
fn write_fragmented(vfs: &mut Vfs<MemoryBackend>) {
    let big = vfs.create_file(OBJECT_ROOT, "big", at(1)).unwrap();
    let other = vfs.create_file(OBJECT_ROOT, "other", at(1)).unwrap();
    let handles = [big, other].map(|id| vfs.open_file(id, AccessMode::WriteOnly).unwrap());
    for block in 0..BLOCKS {
        for handle in handles {
            vfs.write(handle, block * 4096, &[7u8; 4096], at(2))
                .unwrap();
        }
    }
    for handle in handles {
        vfs.close(handle).unwrap();
    }
}

#[test]
fn with_inline_maintenance_off_no_operation_does_maintenance_and_steps_drain_it() {
    let mut vfs = mounted();
    vfs.set_inline_maintenance(false);
    drain(&mut vfs);
    let before = vfs.statfs().free_blocks;

    write_fragmented(&mut vfs);
    drain(&mut vfs);
    let written = vfs.statfs().free_blocks;
    assert!(written < before - 2 * BLOCKS, "the files take space");

    vfs.unlink_file(OBJECT_ROOT, "big", at(3)).unwrap();
    // The unlink cleaned nothing and returned nothing: all of it is left for
    // the steps.
    assert_eq!(vfs.pending_orphans().unwrap(), 1);
    assert!(vfs.statfs().free_blocks <= written + 2);

    let steps = drain(&mut vfs);
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    assert!(
        vfs.statfs().free_blocks >= written + BLOCKS - 10,
        "the space of the deleted file came back: {} free, {} before",
        vfs.statfs().free_blocks,
        written
    );
    // The work was cut into many short transactions, not done in one.
    assert!(steps > 1, "{steps} steps");
}
