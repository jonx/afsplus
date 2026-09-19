//! The reclaim backlog as a health note: one note per crossing, and the next
//! one only after the backlog has fallen back to half the threshold.
//!
//! The mount drives its own maintenance here, as a host with a maintenance
//! timer does (the AROS handler, the macOS driver): inline and idle
//! maintenance are off, so nothing cleans behind the test's back. The
//! per-transaction reclaim budget is bounded too, so what a delete retires
//! stays in the queue over several observations instead of disappearing into
//! the next transaction.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, mount_with_options, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{reclaim_backlog_high_blocks, AccessMode, Durability, HealthNote, Vfs, VfsError};

const TOTAL_BLOCKS: u64 = 32_768;

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn volume() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, TOTAL_BLOCKS);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x7B; 16],
            label: "Backlog".into(),
            region_size: 8192,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    let mut volume = mount_with_options(device, MountOptions::default()).unwrap();
    volume.set_reclaim_batch_blocks(64);
    let mut vfs = Vfs::new(volume);
    vfs.set_durability(Durability::DELAYED).unwrap();
    vfs.set_inline_maintenance(false);
    vfs.set_idle_maintenance(false);
    vfs
}

/// Writes `mebibytes` and commits them, then deletes the file and publishes
/// the delete: the orphan is waiting, and nothing has been retired yet.
fn write_then_delete(vfs: &mut Vfs<MemoryBackend>, name: &str, mebibytes: u64, at_second: i64) {
    let chunk = vec![0xA5u8; 1 << 20];
    let object = vfs.create_file(OBJECT_ROOT, name, at(at_second)).unwrap();
    let handle = vfs.open_file(object, AccessMode::WriteOnly).unwrap();
    for index in 0..mebibytes {
        vfs.write(handle, index << 20, &chunk, at(at_second))
            .unwrap();
    }
    vfs.close(handle).unwrap();
    vfs.sync_filesystem().unwrap();
    vfs.unlink_file(OBJECT_ROOT, name, at(at_second + 1))
        .unwrap();
    vfs.sync_filesystem().unwrap();
}

fn drain_below(vfs: &mut Vfs<MemoryBackend>, blocks: u64) -> Result<usize, VfsError> {
    let mut rounds = 0;
    while vfs.reclaim_pending_blocks() >= blocks {
        assert!(rounds < 400, "the queue is not draining");
        if vfs.reclaim_space(8, at(500))? == 0 {
            break;
        }
        rounds += 1;
    }
    Ok(rounds)
}

#[test]
fn the_reclaim_backlog_is_noted_once_per_crossing() {
    let threshold = reclaim_backlog_high_blocks(TOTAL_BLOCKS);
    assert_eq!(threshold, 4_096, "the floor, on a volume of this size");
    let mut vfs = volume();
    assert_eq!(vfs.take_health_notes(), [], "a fresh mount is healthy");

    // An ordinary delete is not a backlog: one mebibyte is far below it.
    write_then_delete(&mut vfs, "small", 1, 1);
    vfs.cleanup_orphans(4, at(10)).unwrap();
    assert!(vfs.reclaim_pending_blocks() < threshold);
    assert_eq!(vfs.take_health_notes(), []);
    assert!(!vfs.reclaim_backlog_high());
    drain_below(&mut vfs, 1).unwrap();

    // Retiring a large file at once puts more than the threshold into the
    // queue, and that is one note.
    write_then_delete(&mut vfs, "big", 24, 20);
    vfs.cleanup_orphans(4, at(30)).unwrap();
    assert!(vfs.reclaim_pending_blocks() >= threshold);
    assert_eq!(vfs.take_health_notes(), [HealthNote::ReclaimBacklogHigh]);
    assert!(vfs.reclaim_backlog_high());

    // A volume that stays high says so once: further maintenance and commits
    // over the same backlog add nothing.
    for round in 0..4 {
        vfs.cleanup_orphans(1, at(40 + round)).unwrap();
        vfs.commit_if_due(at(40 + round)).unwrap();
        assert_eq!(vfs.take_health_notes(), [], "round {round}");
    }
    assert!(vfs.reclaim_backlog_high());

    // Returning the space below half the threshold arms the note again, and
    // the draining itself notes nothing.
    let rounds = drain_below(&mut vfs, threshold / 2).unwrap();
    assert!(rounds > 0, "the queue took several steps to drain");
    assert_eq!(vfs.take_health_notes(), []);
    assert!(!vfs.reclaim_backlog_high());

    // Past the threshold a second time: a second note.
    write_then_delete(&mut vfs, "again", 24, 60);
    vfs.cleanup_orphans(4, at(70)).unwrap();
    assert!(vfs.reclaim_pending_blocks() >= threshold);
    assert_eq!(vfs.take_health_notes(), [HealthNote::ReclaimBacklogHigh]);
}

/// The threshold follows the volume, with a floor under it.
#[test]
fn the_backlog_threshold_is_a_sixteenth_of_the_volume_above_its_floor() {
    assert_eq!(reclaim_backlog_high_blocks(0), 4_096);
    assert_eq!(reclaim_backlog_high_blocks(65_536), 4_096);
    assert_eq!(reclaim_backlog_high_blocks(1 << 20), 65_536);
}
