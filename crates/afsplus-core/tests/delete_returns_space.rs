//! Deleting a file must give its blocks back.
//!
//! Retiring a file's extents moves them to the reclaim ledger; the ledger is
//! drained by `reclaim_step`. Nothing above the core called it, so on a mounted
//! volume no deleted byte ever came back: writing and deleting the same file
//! repeatedly filled a volume until `rm` itself failed for want of space.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::{Timespec, OBJECT_ROOT};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

#[test]
fn the_blocks_of_a_deleted_file_reach_the_reclaim_ledger_and_then_the_free_pool() {
    let mut device = MemoryBackend::new(4096, 16384);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x9D; 16],
            label: "Space".into(),
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
    let mut volume = mount(device).unwrap();

    let empty = volume.free_blocks();
    volume
        .create_file_in_root("big.bin", &vec![3u8; 1 << 20], at(1))
        .unwrap();
    let written = volume.free_blocks();
    assert!(written < empty, "writing must consume blocks");

    volume.delete_file(OBJECT_ROOT, "big.bin", at(3)).unwrap();
    let deleted = volume.free_blocks();
    let pending = volume.reclaim_pending_blocks();
    eprintln!("empty {empty}, written {written}, deleted {deleted}, pending reclaim {pending}");

    let mut steps = 0;
    while volume.reclaim_pending_blocks() > 0 && steps < 200 {
        volume.reclaim_step(at(4 + steps)).unwrap();
        steps += 1;
    }
    let reclaimed = volume.free_blocks();
    eprintln!("after {steps} reclaim steps: {reclaimed}");

    // The ledger never reaches exactly zero: each reclaim step is itself a
    // transaction that retires the blocks of the checkpoint it replaces. What
    // matters is that it drains to a residue instead of growing.
    let residue = volume.reclaim_pending_blocks();
    assert!(
        residue < 8,
        "the ledger must drain to a residue, not hold {residue} blocks"
    );
    let returned = reclaimed - deleted;
    assert!(
        returned >= 256,
        "the megabyte the file held must come back: only {returned} blocks did"
    );
    // Deleting leaves a couple of blocks of directory and object churn behind,
    // which the next transaction reuses. The file's own storage is what matters.
    assert!(
        empty - reclaimed <= 4,
        "the volume must return to within a few blocks of empty, not {reclaimed} of {empty}"
    );
}
