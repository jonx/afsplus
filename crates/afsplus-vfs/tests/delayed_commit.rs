//! Delayed group commit at the VFS (ADR-121): changes gather in the window
//! and are committed together, on the clock, at the bound, or when asked;
//! a crash before a commit loses whole trailing changes and nothing else.

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Durability, Vfs, DELAYED_WINDOW_OPS_MAX, DELAYED_WINDOW_OPS_MIN};

fn ms(millis: i64) -> Timespec {
    Timespec {
        seconds: 1_000 + millis / 1_000,
        nanoseconds: (millis % 1_000) as u32 * 1_000_000,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xDC; 16],
            label: "Delayed".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: ms(0),
        },
    )
    .unwrap();
    device
}

fn delayed(device: MemoryBackend) -> Vfs<MemoryBackend> {
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    vfs
}

fn write_file(vfs: &mut Vfs<MemoryBackend>, name: &str, bytes: &[u8], at: i64) -> u64 {
    let id = vfs.create_file(OBJECT_ROOT, name, ms(at)).unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, bytes, ms(at)).unwrap();
    vfs.close(handle).unwrap();
    id
}

fn read_file(vfs: &mut Vfs<MemoryBackend>, name: &str) -> Option<Vec<u8>> {
    let id = vfs.lookup(OBJECT_ROOT, name).ok()?;
    let handle = vfs.open_file(id, AccessMode::ReadOnly).unwrap();
    let mut buffer = vec![0u8; 1 << 16];
    let count = vfs.read(handle, 0, &mut buffer).unwrap();
    vfs.close(handle).unwrap();
    buffer.truncate(count);
    Some(buffer)
}

fn remount(vfs: Vfs<MemoryBackend>) -> Vfs<MemoryBackend> {
    let mut device = vfs.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    Vfs::mount(device, MountOptions::default()).unwrap()
}

#[test]
fn changes_wait_for_the_idle_second_and_then_commit_together() {
    let mut vfs = delayed(formatted());
    let start = vfs.generation();
    write_file(&mut vfs, "a", b"alpha", 0);
    write_file(&mut vfs, "b", b"beta", 100);
    vfs.rename(OBJECT_ROOT, "b", OBJECT_ROOT, "bb", false, ms(200))
        .unwrap();
    // Everything is visible, nothing committed, and closing did not commit.
    assert_eq!(read_file(&mut vfs, "a").unwrap(), b"alpha");
    assert_eq!(read_file(&mut vfs, "bb").unwrap(), b"beta");
    assert_eq!(vfs.generation(), start);
    assert!(vfs.changes_pending());
    // Not yet idle for a second.
    assert!(vfs.commit_if_due(ms(900)).unwrap());
    assert_eq!(vfs.generation(), start);
    // Idle for a second: one commit for all of it, and the reclaim of what
    // it released may follow as a transaction of its own.
    assert!(!vfs.commit_if_due(ms(1_200)).unwrap());
    assert!((start + 1..=start + 2).contains(&vfs.generation()));
    let mut vfs = remount(vfs);
    assert_eq!(read_file(&mut vfs, "a").unwrap(), b"alpha");
    assert_eq!(read_file(&mut vfs, "bb").unwrap(), b"beta");
    assert!(read_file(&mut vfs, "b").is_none());
}

#[test]
fn a_volume_never_idle_still_commits_at_the_maximum_age() {
    let mut vfs = delayed(formatted());
    let start = vfs.generation();
    let mut committed_at = None;
    for step in 0..20 {
        let at = step * 500;
        write_file(&mut vfs, &format!("f{step}"), b"x", at);
        vfs.commit_if_due(ms(at)).unwrap();
        if committed_at.is_none() && vfs.generation() > start {
            committed_at = Some(at);
        }
    }
    assert_eq!(
        committed_at,
        Some(5_000),
        "the oldest change turned five seconds old"
    );
}

#[test]
fn the_window_bound_commits_without_the_clock() {
    let mut vfs = delayed(formatted());
    let start = vfs.generation();
    for index in 0..DELAYED_WINDOW_OPS_MAX {
        vfs.create_file(OBJECT_ROOT, &format!("n{index}"), ms(0))
            .unwrap();
    }
    assert!(vfs.generation() > start);
    assert!(!vfs.changes_pending());
}

/// A mount that cannot spare the peak of a full window asks for a shorter
/// one: the bound it takes is the bound the window then commits at, and a
/// value outside the range is brought into it rather than refused.
#[test]
fn a_mount_may_shorten_the_window_and_the_bound_is_what_commits() {
    let mut vfs = delayed(formatted());
    assert_eq!(vfs.window_ops_max(), DELAYED_WINDOW_OPS_MAX);
    assert_eq!(vfs.set_window_ops_max(64), 64);
    assert_eq!(vfs.window_ops_max(), 64);

    let start = vfs.generation();
    for index in 0..63 {
        vfs.create_file(OBJECT_ROOT, &format!("s{index}"), ms(0))
            .unwrap();
    }
    assert_eq!(vfs.generation(), start, "63 changes are still a window");
    assert!(vfs.changes_pending());
    vfs.create_file(OBJECT_ROOT, "s63", ms(0)).unwrap();
    assert!(vfs.generation() > start, "the 64th commits the window");
    assert!(!vfs.changes_pending());

    // Neither end of the range can be left.
    assert_eq!(vfs.set_window_ops_max(0), DELAYED_WINDOW_OPS_MIN);
    assert_eq!(vfs.set_window_ops_max(u32::MAX), DELAYED_WINDOW_OPS_MAX);
}

#[test]
fn a_crash_before_the_commit_loses_the_trailing_changes_and_only_them() {
    let mut vfs = delayed(formatted());
    write_file(&mut vfs, "first", b"one", 0);
    vfs.commit_if_due(ms(2_000)).unwrap();
    write_file(&mut vfs, "second", b"two", 3_000);
    // fsync is durability asked for: it commits the window it is in.
    let synced = write_file(&mut vfs, "synced", b"three", 3_100);
    let handle = vfs.open_file(synced, AccessMode::ReadOnly).unwrap();
    vfs.fsync(handle).unwrap();
    vfs.close(handle).unwrap();
    write_file(&mut vfs, "lost", b"four", 3_200);
    vfs.unlink_file(OBJECT_ROOT, "first", ms(3_300)).unwrap();
    // Crash.
    let mut vfs = remount(vfs);
    assert_eq!(
        read_file(&mut vfs, "first").unwrap(),
        b"one",
        "the delete was lost"
    );
    assert_eq!(read_file(&mut vfs, "second").unwrap(), b"two");
    assert_eq!(read_file(&mut vfs, "synced").unwrap(), b"three");
    assert!(read_file(&mut vfs, "lost").is_none());
}

#[test]
fn listing_a_directory_shows_what_waits_in_the_window() {
    let mut vfs = delayed(formatted());
    write_file(&mut vfs, "listed", b"l", 0);
    let dir = vfs.open_directory(OBJECT_ROOT).unwrap();
    let page = vfs.read_directory(dir, 0, 64).unwrap();
    let names: Vec<&[u8]> = page
        .entries
        .iter()
        .map(|entry| entry.name.as_slice())
        .collect();
    assert_eq!(names, [b"listed".as_slice()]);
    vfs.close(dir).unwrap();
}

#[test]
fn a_delayed_mount_commits_far_less_than_a_sync_one() {
    let run = |durability: Durability| {
        let device = TraceBackend::new(formatted());
        let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
        vfs.set_durability(durability).unwrap();
        let start = vfs.generation();
        for index in 0..100 {
            let name = format!("s{index}");
            let id = vfs.create_file(OBJECT_ROOT, &name, ms(index)).unwrap();
            let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
            vfs.write(handle, 0, &[index as u8; 900], ms(index))
                .unwrap();
            vfs.close(handle).unwrap();
            vfs.rename(
                OBJECT_ROOT,
                &name,
                OBJECT_ROOT,
                &format!("r{index}"),
                false,
                ms(index),
            )
            .unwrap();
        }
        vfs.sync_filesystem().unwrap();
        let commits = vfs.generation() - start;
        let flushes = vfs.into_volume().into_device().stats().flushes;
        (commits, flushes)
    };
    let (sync_commits, sync_flushes) = run(Durability::Sync);
    let (delayed_commits, delayed_flushes) = run(Durability::DELAYED);
    eprintln!(
        "100 files: sync {sync_commits} commits {sync_flushes} flushes, \
         delayed {delayed_commits} commits {delayed_flushes} flushes"
    );
    assert!(sync_commits >= 200, "{sync_commits}");
    assert!(delayed_commits <= 3, "{delayed_commits}");
    assert!(
        delayed_flushes * 20 < sync_flushes,
        "flushes {delayed_flushes} delayed, {sync_flushes} sync"
    );
}

#[test]
fn leaving_delayed_commits_what_waits_and_sync_commits_each_change() {
    let mut vfs = delayed(formatted());
    let start = vfs.generation();
    write_file(&mut vfs, "w", b"w", 0);
    vfs.set_durability(Durability::Sync).unwrap();
    assert_eq!(vfs.generation(), start + 1);
    assert!(!vfs.changes_pending());
    vfs.create_file(OBJECT_ROOT, "now", ms(10)).unwrap();
    assert_eq!(
        vfs.generation(),
        start + 2,
        "sync: the create commits itself"
    );
}

#[test]
fn deleted_files_are_cleaned_and_their_space_returned_in_idle_time() {
    let mut vfs = delayed(formatted());
    for index in 0..80 {
        write_file(&mut vfs, &format!("d{index}"), &[1u8; 5000], index);
    }
    vfs.commit_if_due(ms(2_000)).unwrap();
    while vfs.commit_if_due(ms(2_000)).unwrap() {}
    let with_files = vfs.statfs().free_blocks;
    for index in 0..80 {
        vfs.unlink_file(OBJECT_ROOT, &format!("d{index}"), ms(3_000 + index))
            .unwrap();
    }
    // Committed at the maximum age while still busy, a change 100 ms ago:
    // the names are gone, the space is not back yet, nothing was cleaned.
    write_file(&mut vfs, "busy", b"b", 8_000);
    vfs.commit_if_due(ms(8_100)).unwrap();
    assert!(!vfs.changes_pending());
    assert_eq!(vfs.pending_orphans().unwrap(), 80);
    assert!(
        vfs.statfs().free_blocks <= with_files,
        "nothing returned yet"
    );
    // Idle ticks clean in batches until nothing is left, then rest.
    let mut ticks = 0;
    while vfs.commit_if_due(ms(10_000 + ticks * 100)).unwrap() {
        ticks += 1;
        assert!(ticks < 20, "idle cleanup did not finish");
    }
    assert!(
        ticks >= 3,
        "80 orphans take more than one batch of 32: {ticks}"
    );
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    assert!(
        vfs.statfs().free_blocks >= with_files + 80,
        "space returned"
    );
    let mut vfs = remount(vfs);
    assert!(read_file(&mut vfs, "d0").is_none());
}

#[test]
fn a_backlog_past_the_cap_is_cleaned_by_the_commit() {
    use afsplus_vfs::DELAYED_ORPHANS_MAX;
    let mut device = MemoryBackend::new(4096, 32_768);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xCA; 16],
            label: "Cap".into(),
            region_size: 8192,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: ms(0),
        },
    )
    .unwrap();
    let mut vfs = delayed(device);
    let total = DELAYED_ORPHANS_MAX + 100;
    for index in 0..total {
        vfs.create_file(OBJECT_ROOT, &format!("c{index}"), ms(index as i64))
            .unwrap();
    }
    vfs.sync_filesystem().unwrap();
    // Never idle: every change is one millisecond after the last.
    for index in 0..total {
        vfs.unlink_file(OBJECT_ROOT, &format!("c{index}"), ms(10_000 + index as i64))
            .unwrap();
        vfs.commit_if_due(ms(10_000 + index as i64)).unwrap();
    }
    // The last changes are committed as any commit is, without the cleanup
    // a sync would add.
    vfs.set_durability(Durability::Sync).unwrap();
    let pending = vfs.pending_orphans().unwrap();
    assert!(pending <= DELAYED_ORPHANS_MAX, "{pending} pending");
}

#[test]
fn a_write_uses_the_space_of_a_delete_still_waiting_for_idle_time() {
    let mut vfs = delayed(formatted());
    let chunk = vec![7u8; 1 << 20];
    let fill = |vfs: &mut Vfs<MemoryBackend>, name: &str, at: i64| {
        let id = vfs.create_file(OBJECT_ROOT, name, ms(at)).unwrap();
        let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
        for index in 0..20 {
            vfs.write(handle, index << 20, &chunk, ms(at)).unwrap();
        }
        vfs.close(handle).unwrap();
    };
    fill(&mut vfs, "first", 0);
    vfs.sync_filesystem().unwrap();
    vfs.unlink_file(OBJECT_ROOT, "first", ms(1_000)).unwrap();
    // Committed while busy: the 20 MiB wait for idle time.
    write_file(&mut vfs, "busy", b"b", 6_000);
    vfs.commit_if_due(ms(6_100)).unwrap();
    assert_eq!(vfs.pending_orphans().unwrap(), 1);
    assert!(
        vfs.statfs().free_blocks < 20 * 256,
        "the 32 MiB volume has no room for a second copy"
    );
    // No idle tick: the write itself asks for the space back.
    fill(&mut vfs, "second", 6_200);
    vfs.sync_filesystem().unwrap();
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    let mut vfs = remount(vfs);
    assert!(read_file(&mut vfs, "first").is_none());
    assert_eq!(read_file(&mut vfs, "second").unwrap().len(), 1 << 16);
}

#[test]
fn deletes_on_a_volume_never_idle_keep_room_for_their_commits() {
    let mut vfs = delayed(formatted());
    let files = 2_560;
    let drawers: Vec<u64> = (0..80)
        .map(|index| {
            vfs.create_directory(OBJECT_ROOT, &format!("d{index}"), ms(0))
                .unwrap()
        })
        .collect();
    for index in 0..files {
        let id = vfs
            .create_file(drawers[index / 32], &format!("f{index}"), ms(0))
            .unwrap();
        let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
        vfs.write(handle, 0, &[3u8; 1_200], ms(0)).unwrap();
        vfs.close(handle).unwrap();
    }
    vfs.sync_filesystem().unwrap();
    // A change every millisecond: no idle tick ever runs. Every delete must
    // still commit, the last ones included.
    for index in 0..files {
        let at = 10_000 + index as i64;
        vfs.unlink_file(drawers[index / 32], &format!("f{index}"), ms(at))
            .unwrap();
        vfs.commit_if_due(ms(at)).unwrap();
    }
    vfs.set_durability(Durability::Sync).unwrap();
    let room = vfs.statfs().available_blocks;
    assert!(room >= 1_024, "{room} blocks available");
}

/// A volume that is only read must not write: once idle maintenance has
/// returned what it can, further idle ticks commit nothing, however many
/// packets wake the handler (measured on the J313: a checkpoint per wake,
/// ten thousand 4 KiB writes during one boot).
#[test]
fn idle_ticks_on_a_settled_volume_commit_nothing() {
    let mut vfs = delayed(formatted());
    for index in 0..8 {
        write_file(&mut vfs, &format!("s{index}"), &[7u8; 9000], index);
    }
    vfs.unlink_file(OBJECT_ROOT, "s3", ms(500)).unwrap();
    // Commit, then give idle maintenance the ticks it needs: a backlog the
    // older checkpoint still protects takes one more transaction to free.
    for tick in 0..10 {
        vfs.commit_if_due(ms(2_000 + tick * 100)).unwrap();
    }
    let settled = vfs.generation();
    let free = vfs.statfs().free_blocks;
    for tick in 0..200 {
        assert_eq!(read_file(&mut vfs, "s1").unwrap().len(), 9000);
        assert!(!vfs.commit_if_due(ms(10_000 + tick * 50)).unwrap());
    }
    assert_eq!(
        vfs.generation(),
        settled,
        "idle ticks on a read-only workload committed checkpoints"
    );
    assert!(
        vfs.reclaim_pending_blocks() <= 2,
        "{}",
        vfs.reclaim_pending_blocks()
    );
    assert!(vfs.statfs().free_blocks >= free);
    // The residue is returned by the next real transaction's reclaim.
    write_file(&mut vfs, "later", b"x", 30_000);
    while vfs.commit_if_due(ms(32_000)).unwrap() {}
    let mut vfs = remount(vfs);
    assert_eq!(read_file(&mut vfs, "s1").unwrap().len(), 9000);
    assert!(read_file(&mut vfs, "s3").is_none());
}

/// A device whose writes fail while the shared switch is on: a transient
/// fault the test turns on for one idle tick and off again.
struct Flaky {
    inner: MemoryBackend,
    failing: std::rc::Rc<std::cell::Cell<bool>>,
}

impl afsplus_block::BlockDevice for Flaky {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), afsplus_block::BlockError> {
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), afsplus_block::BlockError> {
        if self.failing.get() {
            return Err(afsplus_block::BlockError::Injected("transient write fault"));
        }
        self.inner.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), afsplus_block::BlockError> {
        self.inner.flush()
    }
}

/// The rest ends with any transaction, even one that leaves the backlog at
/// the same count, and a remount starts without one.
#[test]
fn the_idle_rest_ends_with_a_transaction_and_with_a_remount() {
    let mut vfs = delayed(formatted());
    write_file(&mut vfs, "a", &[1u8; 9000], 0);
    for tick in 0..10 {
        vfs.commit_if_due(ms(2_000 + tick * 100)).unwrap();
    }
    let rested = vfs.generation();
    let backlog = vfs.reclaim_pending_blocks();
    for tick in 0..20 {
        assert!(!vfs.commit_if_due(ms(4_000 + tick * 100)).unwrap());
    }
    assert_eq!(vfs.generation(), rested, "resting");
    // A real transaction: idle reclaim is tried again afterwards, whatever
    // the backlog count, then rests at the new generation.
    write_file(&mut vfs, "b", b"x", 10_000);
    for tick in 0..10 {
        vfs.commit_if_due(ms(12_000 + tick * 100)).unwrap();
    }
    let after = vfs.generation();
    assert!(
        after >= rested + 2,
        "the change committed and idle reclaim ran again: {rested} -> {after} (backlog {backlog} -> {})",
        vfs.reclaim_pending_blocks()
    );
    for tick in 0..20 {
        assert!(!vfs.commit_if_due(ms(20_000 + tick * 100)).unwrap());
    }
    assert_eq!(vfs.generation(), after, "resting again");
    // A remount carries no rest: its first idle tick tries once.
    let mut vfs = delayed(remount(vfs).into_volume().into_device());
    let mounted = vfs.generation();
    for tick in 0..10 {
        vfs.commit_if_due(ms(40_000 + tick * 100)).unwrap();
    }
    let tried = vfs.generation();
    assert!(tried > mounted, "idle reclaim ran after the remount");
    for tick in 0..20 {
        vfs.commit_if_due(ms(50_000 + tick * 100)).unwrap();
    }
    assert_eq!(vfs.generation(), tried, "and rests again");
}

/// A reclaim step that fails does not start a rest: the backlog it could
/// not touch is tried again on the next tick and drains.
#[test]
fn a_failed_idle_reclaim_step_is_retried() {
    let failing = std::rc::Rc::new(std::cell::Cell::new(false));
    let device = Flaky {
        inner: formatted(),
        failing: failing.clone(),
    };
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    let at = |millis: i64| ms(millis);
    for index in 0..40 {
        let id = vfs
            .create_file(OBJECT_ROOT, &format!("f{index}"), at(index))
            .unwrap();
        let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
        vfs.write(handle, 0, &[3u8; 9000], at(index)).unwrap();
        vfs.close(handle).unwrap();
    }
    for tick in 0..10 {
        vfs.commit_if_due(at(2_000 + tick * 100)).unwrap();
    }
    // Truncate them with maintenance off: the freed blocks wait in the
    // reclaim queue, with no deleted file whose cleanup would be a
    // transaction of its own.
    vfs.set_idle_maintenance(false);
    for index in 0..40 {
        let id = vfs.lookup(OBJECT_ROOT, &format!("f{index}")).unwrap();
        let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
        vfs.truncate(handle, 0, at(3_000 + index)).unwrap();
        vfs.close(handle).unwrap();
    }
    vfs.commit_if_due(at(6_000)).unwrap();
    vfs.set_idle_maintenance(true);
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    assert!(
        vfs.reclaim_pending_blocks() > 40,
        "{}",
        vfs.reclaim_pending_blocks()
    );
    // One idle tick on a device that refuses every write.
    failing.set(true);
    let _ = vfs.commit_if_due(at(8_000));
    failing.set(false);
    let stuck = vfs.reclaim_pending_blocks();
    // The fault is gone: the following ticks must work the backlog down.
    for tick in 0..30 {
        let _ = vfs.commit_if_due(at(9_000 + tick * 100));
    }
    let left = vfs.reclaim_pending_blocks();
    assert!(
        left <= 2,
        "the backlog was abandoned after a transient fault: {stuck} -> {left}"
    );
}
