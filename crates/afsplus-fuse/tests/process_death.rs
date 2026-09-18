//! A driver killed at any instant loses nothing it had acknowledged with fsync.
//!
//! When the process serving a volume dies, the machine does not: every block
//! the process handed to the image file is kept, in the order it was written,
//! and nothing after it. So the image a killed driver leaves is exactly the
//! volume it started from plus a prefix of its writes. This drives the adapter
//! the way the macOS driver does, maintenance on its own steps included,
//! records every block write, and at every prefix requires the checker to find
//! the image clean and every file whose fsync had returned to read back byte
//! for byte.
//!
//! It runs twice: with a program's fsync, and as the macOS driver runs, where
//! every write is made durable before it is answered. The mandated negative:
//! the same workload with neither must lose a file at some prefix, or the
//! harness proves nothing.

use std::cell::RefCell;
use std::rc::Rc;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{FuseAdapter, FuseConfig};
use afsplus_vfs::{AccessMode, Vfs};

const BLOCK_SIZE: usize = 4096;

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

/// Every block write, in order: the block and its bytes.
type WriteLog = Vec<(u64, Vec<u8>)>;

/// A device that keeps every block write in a log the test can read while the
/// adapter still owns the device.
struct Recorder {
    inner: MemoryBackend,
    writes: Rc<RefCell<WriteLog>>,
}

impl BlockDevice for Recorder {
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
        self.writes.borrow_mut().push((lba, data.to_vec()));
        self.inner.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

/// A file the workload made durable: it must read back as `content` in every
/// image cut after `from` writes and before `until` writes.
struct Durable {
    name: String,
    content: Vec<u8>,
    from: usize,
    until: usize,
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, 4096);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xD1; 16],
            label: "ProcessDeath".into(),
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
    device
}

/// A deterministic byte pattern of a given length, different for every seed.
fn pattern(seed: u32, length: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

/// How the workload asks for durability.
#[derive(Clone, Copy, PartialEq)]
enum Durability {
    /// A program calls fsync after writing each kept file.
    Fsync,
    /// The macOS driver's configuration: every write is made durable before
    /// it is answered, because FSKit may not forward a program's fsync.
    EveryWrite,
    /// Nobody asks: the negative control.
    Never,
}

/// Runs the workload and returns the starting image, every block write, and
/// what each acknowledgement promised.
fn run_workload(durability: Durability) -> (MemoryBackend, WriteLog, Vec<Durable>) {
    let base = formatted();
    let writes = Rc::new(RefCell::new(Vec::new()));
    let device = Recorder {
        inner: base.clone(),
        writes: Rc::clone(&writes),
    };
    let mut fuse = FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig {
            durable_data_replies: durability == Durability::EveryWrite,
            ..FuseConfig::default()
        },
    );
    fuse.set_inline_maintenance(false);
    let mut durable: Vec<Durable> = Vec::new();
    let mut clock = 10;
    let mut tick = || {
        clock += 1;
        at(clock)
    };

    for round in 0..24u32 {
        // A kept file: written in pieces, fsync'd, closed.
        let name = format!("kept-{round}");
        let content = pattern(round, 1 + (round as usize * 5_311) % 40_000);
        let (_, handle) = fuse
            .create_file(
                OBJECT_ROOT,
                name.as_bytes(),
                AccessMode::ReadWrite,
                None,
                tick(),
            )
            .unwrap();
        for (index, piece) in content.chunks(9_000).enumerate() {
            fuse.write(handle, (index * 9_000) as u64, piece, tick())
                .unwrap();
        }
        if durability == Durability::Fsync {
            fuse.fsync(handle).unwrap();
        }
        durable.push(Durable {
            name,
            content,
            from: writes.borrow().len(),
            until: usize::MAX,
        });
        fuse.close(handle).unwrap();

        // Work nobody made durable around it: a scratch file written and
        // left, renamed, or deleted, and now and then a kept file deleted.
        let scratch = format!("scratch-{round}");
        let (_, handle) = fuse
            .create_file(
                OBJECT_ROOT,
                scratch.as_bytes(),
                AccessMode::ReadWrite,
                None,
                tick(),
            )
            .unwrap();
        fuse.write(handle, 0, &pattern(1000 + round, 12_000), tick())
            .unwrap();
        fuse.close(handle).unwrap();
        match round % 3 {
            0 => fuse
                .rename(
                    OBJECT_ROOT,
                    scratch.as_bytes(),
                    OBJECT_ROOT,
                    format!("renamed-{round}").as_bytes(),
                    false,
                    tick(),
                )
                .unwrap(),
            1 => fuse
                .unlink_file(OBJECT_ROOT, scratch.as_bytes(), tick())
                .unwrap(),
            _ => {}
        }
        if round % 5 == 4 {
            let victim = &mut durable[round as usize - 3];
            victim.until = writes.borrow().len();
            fuse.unlink_file(OBJECT_ROOT, victim.name.as_bytes(), tick())
                .unwrap();
        }
        // The driver's maintenance thread, a few steps between requests.
        for _ in 0..3 {
            fuse.maintenance_step(tick());
        }
    }
    let log = writes.borrow().clone();
    (base, log, durable)
}

fn read_all(vfs: &mut Vfs<MemoryBackend>, name: &str) -> Option<Vec<u8>> {
    let id = vfs.lookup(OBJECT_ROOT, name).ok()?;
    let handle = vfs.open_file(id, AccessMode::ReadOnly).ok()?;
    let mut content = Vec::new();
    let mut buffer = vec![0u8; 16_384];
    loop {
        let read = vfs.read(handle, content.len() as u64, &mut buffer).ok()?;
        if read == 0 {
            break;
        }
        content.extend_from_slice(&buffer[..read]);
    }
    let _ = vfs.close(handle);
    Some(content)
}

/// Every image a kill could leave, and what went wrong in each, if anything.
fn losses(durability: Durability) -> (usize, Vec<String>) {
    let (base, log, durable) = run_workload(durability);
    let mut image = base;
    let mut problems = Vec::new();
    for cut in 0..=log.len() {
        if cut > 0 {
            let (lba, data) = &log[cut - 1];
            image.write_block(*lba, data).unwrap();
        }
        let owed: Vec<&Durable> = durable
            .iter()
            .filter(|file| file.from <= cut && cut < file.until)
            .collect();
        // Only the cuts where a promise is due or just changed need a full
        // look; checking every one would repeat the same image thousands of
        // times over.
        let promise_boundary = durable
            .iter()
            .any(|file| file.from == cut || file.until == cut + 1);
        if !promise_boundary && cut % 16 != 0 && cut != log.len() {
            continue;
        }
        let report = check_device(&mut image.clone());
        if !report.is_clean() {
            problems.push(format!("cut {cut}: checker: {}", report.errors.join("; ")));
            continue;
        }
        let mut vfs = match Vfs::mount(image.clone(), MountOptions::default()) {
            Ok(vfs) => vfs,
            Err(error) => {
                problems.push(format!("cut {cut}: does not mount: {error:?}"));
                continue;
            }
        };
        for file in owed {
            match read_all(&mut vfs, &file.name) {
                Some(content) if content == file.content => {}
                Some(content) => problems.push(format!(
                    "cut {cut}: {} reads {} bytes, not the {} fsync'd",
                    file.name,
                    content.len(),
                    file.content.len()
                )),
                None => problems.push(format!("cut {cut}: {} is gone", file.name)),
            }
        }
    }
    (log.len(), problems)
}

fn assert_no_loss(durability: Durability) {
    let (writes, problems) = losses(durability);
    assert!(writes > 500, "the workload wrote only {writes} blocks");
    assert!(
        problems.is_empty(),
        "{} of the images a kill could leave lose something:\n{}",
        problems.len(),
        problems
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn a_driver_killed_after_any_write_keeps_every_fsyncd_file() {
    assert_no_loss(Durability::Fsync);
}

#[test]
fn as_the_macos_driver_runs_every_acknowledged_write_survives_a_kill() {
    assert_no_loss(Durability::EveryWrite);
}

#[test]
fn without_durability_the_same_harness_finds_a_lost_file() {
    let (_, problems) = losses(Durability::Never);
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("is gone") || problem.contains("not the")),
        "the harness found no loss in a workload that never asked for durability"
    );
}
