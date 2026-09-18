//! Delayed group commit at the VFS (ADR-121): changes gather in the window
//! and are committed together, on the clock, at the bound, or when asked;
//! a crash before a commit loses whole trailing changes and nothing else.

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Durability, Vfs, DELAYED_WINDOW_OPS_MAX};

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
