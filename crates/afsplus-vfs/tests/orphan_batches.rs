//! Orphan cleanup in batches (performance programme, lot D): a mount cleans
//! many orphans in one transaction instead of two commits each, an orphan
//! somebody holds open is passed over rather than stopping the rest, and the
//! health record still says what waits.

use std::cell::Cell;
use std::rc::Rc;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Durability, Vfs};

/// A device whose flush count the test can read while the volume is still
/// mounted: the point of this lot is a counter, not an elapsed time.
struct CountingBackend {
    inner: MemoryBackend,
    flushes: Rc<Cell<u64>>,
}

impl CountingBackend {
    fn new(inner: MemoryBackend) -> (Self, Rc<Cell<u64>>) {
        let flushes = Rc::new(Cell::new(0));
        (
            CountingBackend {
                inner,
                flushes: Rc::clone(&flushes),
            },
            flushes,
        )
    }
}

impl BlockDevice for CountingBackend {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(lba, buf)
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, data)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.flushes.set(self.flushes.get() + 1);
        self.inner.flush()
    }
}

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
            uuid: [0xB4; 16],
            label: "Batches".into(),
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

fn write_file<D: BlockDevice>(vfs: &mut Vfs<D>, name: &str, bytes: &[u8], at: i64) -> u64 {
    let id = vfs.create_file(OBJECT_ROOT, name, ms(at)).unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, bytes, ms(at)).unwrap();
    vfs.close(handle).unwrap();
    id
}

/// Sixty-four deleted files waiting for idle time, committed and uncleaned.
fn sixty_four_orphans<D: BlockDevice>(vfs: &mut Vfs<D>) {
    for index in 0..64 {
        write_file(vfs, &format!("d{index:02}"), &[7u8; 900], index);
    }
    vfs.sync_filesystem().unwrap();
    for index in 0..64 {
        vfs.unlink_file(OBJECT_ROOT, &format!("d{index:02}"), ms(3_000 + index))
            .unwrap();
    }
    // Committed while still busy: the names are gone and nothing is cleaned.
    write_file(vfs, "busy", b"b", 8_000);
    vfs.commit_if_due(ms(8_100)).unwrap();
    assert!(!vfs.changes_pending());
    assert_eq!(vfs.pending_orphans().unwrap(), 64);
}

#[test]
fn an_idle_tick_cleans_a_batch_of_orphans_in_one_transaction() {
    let (device, flushes) = CountingBackend::new(formatted());
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    sixty_four_orphans(&mut vfs);

    let before_generation = vfs.generation();
    let before_flushes = flushes.get();
    assert!(vfs.commit_if_due(ms(10_000)).unwrap(), "the tick worked");
    let cleaned = 64 - vfs.pending_orphans().unwrap();
    let commits = vfs.generation() - before_generation;
    let tick_flushes = flushes.get() - before_flushes;
    eprintln!("one idle tick: {cleaned} orphans, {commits} commits, {tick_flushes} flushes");

    // The tick cleans a batch of 32. Before this lot each of them was two
    // transactions of its own: this same test on the code before it read 67
    // commits and 134 flushes for the one tick, against 4 and 8 now, the
    // cleanup transaction plus the reclaim of what it retired.
    assert_eq!(cleaned, 32, "a full batch, not one orphan");
    assert!(
        commits <= 8,
        "one cleanup transaction and the reclaim behind it: {commits} commits"
    );
    assert!(
        tick_flushes <= 2 * commits,
        "{tick_flushes} flushes for {commits} commits"
    );

    // The rest follows in later ticks, and the volume is clean after.
    let mut ticks = 1;
    while vfs.commit_if_due(ms(10_000 + ticks * 100)).unwrap() {
        ticks += 1;
        assert!(ticks < 20, "idle cleanup did not finish");
    }
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    let mut device = vfs.into_volume().into_device().inner;
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn a_sync_cleans_every_orphan_in_batches() {
    let (device, flushes) = CountingBackend::new(formatted());
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    sixty_four_orphans(&mut vfs);

    let before_generation = vfs.generation();
    let before_flushes = flushes.get();
    vfs.sync_filesystem().unwrap();
    let commits = vfs.generation() - before_generation;
    let sync_flushes = flushes.get() - before_flushes;
    eprintln!("sync of 64 orphans: {commits} commits, {sync_flushes} flushes");
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    // Two batches of 32, their reclaim, and the commit of the window. The
    // code before this lot took 131 commits and 262 flushes for the same 64.
    assert!(
        commits <= 6,
        "64 orphans must not be 64 transactions: {commits}"
    );
    let mut device = vfs.into_volume().into_device().inner;
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn an_open_orphan_waits_without_holding_up_the_batch() {
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    vfs.set_durability(Durability::DELAYED).unwrap();
    let mut objects = Vec::new();
    for index in 0..8 {
        objects.push(write_file(
            &mut vfs,
            &format!("d{index}"),
            &[3u8; 900],
            index,
        ));
    }
    vfs.sync_filesystem().unwrap();

    // The lowest object id sorts first in the orphan directory, so the file
    // that is held open is the one a cleanup reaches first.
    let held = vfs.open_file(objects[0], AccessMode::ReadOnly).unwrap();
    for index in 0..8 {
        vfs.unlink_file(OBJECT_ROOT, &format!("d{index}"), ms(3_000 + index))
            .unwrap();
    }
    write_file(&mut vfs, "busy", b"b", 8_000);
    vfs.commit_if_due(ms(8_100)).unwrap();
    assert_eq!(vfs.pending_orphans().unwrap(), 8);

    let cleaned = vfs.cleanup_orphans(8, ms(9_000)).unwrap();
    assert_eq!(cleaned, 7, "the seven behind the open one were cleaned");
    assert_eq!(vfs.pending_orphans().unwrap(), 1);
    // The reader still has its bytes.
    let mut buffer = [0u8; 900];
    assert_eq!(vfs.read(held, 0, &mut buffer).unwrap(), 900);
    assert_eq!(buffer[0], 3);

    // The last close is what cleans it, as before.
    vfs.close(held).unwrap();
    assert_eq!(vfs.pending_orphans().unwrap(), 0);
    let mut device = vfs.into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}
