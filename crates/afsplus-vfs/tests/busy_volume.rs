//! A delayed mount that is never idle.
//!
//! The transactions of a delayed window return no superseded block to the
//! free pool, and what they supersede waits in the reclaim queue for idle
//! time. A volume that was written without a pause for two minutes filled
//! the queue's tables: the commit of its window was refused, the window was
//! abandoned with every file created in it, and the files created next were
//! given the identifiers of the lost ones, so that the handles still open on
//! those read and wrote the new files.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::reclaim::ReclaimCaps;
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Durability, Vfs, VfsError};

const BLOCK: usize = 4096;

fn at(milliseconds: u64) -> Timespec {
    Timespec {
        seconds: 100 + (milliseconds / 1000) as i64,
        nanoseconds: (milliseconds % 1000) as u32 * 1_000_000,
    }
}

/// A volume whose reclaim queue is small, so that a test reaches its end in
/// a few hundred commits instead of two minutes.
fn busy(blocks: u64) -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(BLOCK, blocks);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xB5; 16],
            label: "Busy".into(),
            region_size: 4096,
            reclaim_caps: ReclaimCaps {
                inline_entries: 8,
                segment_refs: 4,
                table_refs: 4,
            },
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::Delayed {
        idle_ms: 1_000,
        max_age_ms: 5_000,
    })
    .unwrap();
    vfs.set_window_ops_max(16);
    vfs
}

/// Overwrites and truncations without a pause: `rounds` windows' worth.
/// Says what the first refused operation answered.
fn work(vfs: &mut Vfs<MemoryBackend>, rounds: u64) -> Result<(), VfsError> {
    let id = vfs.create_file(OBJECT_ROOT, "work.dat", at(1))?;
    let handle = vfs.open_file(id, AccessMode::ReadWrite)?;
    let block = [0x5Au8; BLOCK];
    for step in 0..rounds * 16 {
        let now = at(2 + step);
        if step % 64 == 63 {
            vfs.truncate(handle, (step % 7) * BLOCK as u64, now)?;
        } else {
            vfs.write(handle, (step * 5 % 48) * BLOCK as u64, &block, now)?;
        }
    }
    vfs.close(handle)
}

#[test]
fn a_volume_that_is_never_idle_keeps_publishing_its_windows() {
    let mut vfs = busy(65536);
    work(&mut vfs, 4000).expect("a window must not be lost because the volume never rests");
    let (tables, most) = vfs.reclaim_queue_tables();
    assert!(
        tables < most,
        "the reclaim queue is at its end: {tables} of {most} tables"
    );
}

#[test]
fn a_file_created_after_a_lost_window_is_none_of_the_lost_ones() {
    let mut vfs = busy(65536);
    // Nothing empties the queue, so its end is reached and a window is lost.
    vfs.set_inline_maintenance(false);
    let work = vfs.create_file(OBJECT_ROOT, "work.dat", at(1)).unwrap();
    let work = vfs.open_file(work, AccessMode::ReadWrite).unwrap();
    let block = [0x5Au8; BLOCK];
    let mut opened = Vec::new();
    let mut step = 0u64;
    let lost = loop {
        let now = at(2 + step);
        let mut turn = || -> Result<(), VfsError> {
            if step.is_multiple_of(8) {
                let id = vfs.create_file(OBJECT_ROOT, &format!("file{step}.dat"), now)?;
                let handle = vfs.open_file(id, AccessMode::ReadWrite)?;
                opened.push((id, handle));
                vfs.write(handle, 0, &block, now)?;
            }
            vfs.write(work, (step * 5 % 48) * BLOCK as u64, &block, now)?;
            Ok(())
        };
        match turn() {
            Ok(()) => step += 1,
            Err(VfsError::WindowLost(lost)) => break lost,
            Err(other) => panic!("step {step}: {other:?}"),
        }
        assert!(step < 100_000, "the queue never reached its end");
    };
    assert!(lost > 0, "the loss must say how much was lost");
    let highest = opened.iter().map(|(id, _)| *id).max().unwrap();

    vfs.set_inline_maintenance(true);
    let fresh = loop {
        match vfs.create_file(OBJECT_ROOT, "fresh.dat", at(900_000)) {
            Ok(id) => break id,
            // The queue is still at its end until a loss has emptied it.
            Err(VfsError::WindowLost(_)) => {}
            Err(other) => panic!("after the loss: {other:?}"),
        }
    };
    assert!(
        fresh > highest,
        "object {fresh} was given out again: the lost window had reached {highest}"
    );
    let handle = vfs.open_file(fresh, AccessMode::ReadWrite).unwrap();
    let mark = [0xEEu8; BLOCK];
    vfs.write(handle, 0, &mark, at(900_001)).unwrap();
    // No handle of before reads the new file.
    for (id, old) in opened {
        let mut read = [0u8; BLOCK];
        if let Ok(count) = vfs.read(old, 0, &mut read) {
            assert!(
                count == 0 || read != mark,
                "the handle on object {id} reads the file created after the loss"
            );
        }
    }
}
