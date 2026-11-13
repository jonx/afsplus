//! Random positional writes, reads, truncations, opens, creations and
//! deletions on a few files, checked against a model of what each block
//! must hold: its own pattern or zeros. The lab's stress test
//! (wincompat-aros tests/linux/iostress.c) read another file's block
//! through the handler; this drives the same calls in one thread the way
//! the handler does: runs of blocks per device request, a delayed commit
//! window, a cache small enough to evict.
mod common;
use std::ffi::c_void;
use std::mem::size_of;
use std::{ptr, slice};

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use common::format;

const BLOCK: usize = 4096;
const BLOCKS: u64 = 256;
const FILES: usize = 4;
const OWNERS: usize = 2;
const DEVICE: u64 = 65536;

unsafe extern "C" fn read_run(
    context: *mut c_void,
    lba: u64,
    destination: *mut u8,
    length: u32,
) -> i32 {
    // SAFETY: the test keeps the backend and the buffer alive for the call.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    let destination = unsafe { slice::from_raw_parts_mut(destination, length as usize) };
    device.read_blocks(lba, destination).map_or(5, |()| 0)
}
unsafe extern "C" fn write_run(
    context: *mut c_void,
    lba: u64,
    source: *const u8,
    length: u32,
) -> i32 {
    // SAFETY: as above.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    let source = unsafe { slice::from_raw_parts(source, length as usize) };
    device.write_blocks(lba, source).map_or(5, |()| 0)
}
unsafe extern "C" fn flush(context: *mut c_void) -> i32 {
    // SAFETY: as above.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    device.flush().map_or(5, |()| 0)
}

fn mount(device: &mut MemoryBackend) -> *mut AfsplusAros {
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(device).cast::<c_void>(),
        block_size: BLOCK as u32,
        max_transfer_blocks: 256,
        total_blocks: DEVICE,
        read_block: Some(read_run),
        write_block: Some(write_run),
        flush: Some(flush),
    };
    let config = AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: b"AFS+".as_ptr(),
        volume_name_length: 4,
        max_file_handles: 64,
        max_locks: 64,
        max_file_info_name_bytes: 107,
        flags: 0,
        snapshot_max_edit_records: 0,
        snapshot_max_views: 0,
        snapshot_reclaim_records: 0,
    };
    let mut filesystem = ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut filesystem), 0);
    filesystem
}

fn pattern(file: usize, block: u64) -> [u8; BLOCK] {
    let mut b = [0u8; BLOCK];
    for unit in b.chunks_exact_mut(16) {
        unit[..8].copy_from_slice(&block.to_le_bytes());
        unit[8..].copy_from_slice(&(0x1057e55u64 * (file as u64 + 1)).to_le_bytes());
    }
    b
}

struct File {
    name: String,
    handle: u64,
    object: u64,
    size: u64,
    written: Vec<bool>,
}

struct Run {
    fs: *mut AfsplusAros,
    /// The lock of the directory the files live in; 0 is the root.
    base: u64,
    first_owner: usize,
    seed: u64,
    step: usize,
    seconds: i64,
    millis: u32,
}

impl Run {
    fn open(&self, name: &str, mode: u32) -> u64 {
        let mut handle = 0;
        assert_eq!(
            afsplus_aros_open(
                self.fs,
                self.base,
                name.as_ptr(),
                name.len() as u32,
                mode,
                self.seconds,
                0,
                &mut handle
            ),
            0,
            "seed {} step {}: open {name} mode {mode}",
            self.seed,
            self.step
        );
        handle
    }
    fn create(&self, slot: usize, generation: usize) -> File {
        let name = format!("iostress-{}-{slot}.dat", self.first_owner + generation * 7);
        // A new file is held exclusively; the stress test shares its files.
        let handle = self.open(&name, AFSPLUS_AROS_OPEN_NEW_FILE);
        assert_eq!(afsplus_aros_close(self.fs, handle), 0);
        let handle = self.open(&name, AFSPLUS_AROS_OPEN_READ_WRITE);
        // SAFETY: plain integers, written by the call.
        let mut info: AfsplusArosFileInfo = unsafe { std::mem::zeroed() };
        let mut stored = [0u8; 108];
        assert_eq!(
            afsplus_aros_examine_file(self.fs, handle, &mut info, stored.as_mut_ptr(), 108),
            0
        );
        File {
            name,
            handle,
            object: info.object_id,
            size: 0,
            written: vec![false; BLOCKS as usize],
        }
    }
    fn check(&self, slot: usize, file: &File, block: u64, buffer: &[u8], count: u32) {
        let (seed, step) = (self.seed, self.step);
        if block >= file.size {
            assert_eq!(
                count, 0,
                "seed {seed} step {step}: read beyond the end of {} returned data",
                file.name
            );
            return;
        }
        assert_eq!(
            count as usize, BLOCK,
            "seed {seed} step {step}: short read of {} block {block}",
            file.name
        );
        let want = if file.written[block as usize] {
            pattern(slot, block)
        } else {
            [0u8; BLOCK]
        };
        if buffer != want {
            let unit = buffer
                .chunks_exact(16)
                .zip(want.chunks_exact(16))
                .position(|(a, b)| a != b)
                .unwrap();
            let found = u64::from_le_bytes(buffer[unit * 16..unit * 16 + 8].try_into().unwrap());
            let tag = u64::from_le_bytes(buffer[unit * 16 + 8..unit * 16 + 16].try_into().unwrap());
            panic!(
                "seed {seed} step {step}: {} block {block} (written {}) holds block {found} tag {tag:x} at byte {}",
                file.name, file.written[block as usize], unit * 16
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    seed: u64,
    operations: usize,
    cache: u32,
    delayed: bool,
    fillers: usize,
    step_ms: u64,
    busy_ticks: bool,
) {
    let mut device = format(MemoryBackend::new(BLOCK, DEVICE), true);
    let fs = mount(&mut device);
    let mut granted = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, cache, &mut granted), 0);
    if delayed {
        assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    }
    exercise(
        fs, 0, 4000, seed, operations, delayed, fillers, step_ms, 50, busy_ticks,
    );
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

#[allow(clippy::too_many_arguments)]
fn exercise(
    fs: *mut AfsplusAros,
    base: u64,
    first_owner: usize,
    seed: u64,
    operations: usize,
    delayed: bool,
    fillers: usize,
    step_ms: u64,
    turnover: u64,
    busy_ticks: bool,
) {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state >> 8
    };
    let mut run = Run {
        fs,
        base,
        first_owner,
        seed,
        step: 0,
        seconds: 10,
        millis: 0,
    };
    let mut generation = 0;
    // Neighbours, so that the directory is a tree of several leaves that
    // split and merge as the files come and go.
    for filler in 0..fillers {
        let name = format!("iostress-{}-{}.log", 3000 + filler * 3, filler % 5);
        let handle = run.open(&name, AFSPLUS_AROS_OPEN_NEW_FILE);
        assert_eq!(afsplus_aros_close(fs, handle), 0);
    }
    let mut files: Vec<File> = Vec::new();
    for owner in 0..OWNERS {
        for slot in 0..FILES {
            files.push(run.create(slot, generation + owner));
        }
    }
    generation += OWNERS;
    let mut buffer = [0u8; BLOCK];
    let (mut pending, mut quiet) = (0u32, 0u32);
    for step in 0..operations {
        run.step = step;
        // Time passes as it does under load: a few milliseconds a call.
        run.millis += (next() % step_ms) as u32;
        while run.millis >= 1000 {
            run.millis -= 1000;
            run.seconds += 1;
        }
        let (seconds, nanos) = (run.seconds, run.millis * 1_000_000);
        let index = next() as usize % files.len();
        let slot = index % FILES;
        let block = next() % BLOCKS;
        // Owners end once in `turnover` operations; the rest keep the
        // shares of the stress test.
        let what = if next() % turnover == 0 {
            999
        } else {
            next() % 980
        };
        if what < 450 {
            let file = &files[index];
            let mut count = 0;
            assert_eq!(
                afsplus_aros_read_at(
                    fs,
                    file.handle,
                    block * BLOCK as u64,
                    buffer.as_mut_ptr(),
                    BLOCK as u32,
                    &mut count
                ),
                0,
                "read"
            );
            run.check(slot, file, block, &buffer, count);
        } else if what < 850 {
            let file = &mut files[index];
            // As the controller does: a write beyond the end first extends the file.
            if block > file.size {
                let mut size = 0;
                assert_eq!(
                    afsplus_aros_set_file_size(
                        fs,
                        file.handle,
                        (block * BLOCK as u64) as i64,
                        AFSPLUS_AROS_SEEK_BEGINNING,
                        seconds,
                        nanos,
                        &mut size
                    ),
                    0,
                    "extend"
                );
                file.size = block;
            }
            let data = pattern(slot, block);
            let mut count = 0;
            assert_eq!(
                afsplus_aros_write_at(
                    fs,
                    file.handle,
                    block * BLOCK as u64,
                    data.as_ptr(),
                    BLOCK as u32,
                    seconds,
                    nanos,
                    &mut count
                ),
                0,
                "write"
            );
            assert_eq!(count as usize, BLOCK);
            file.written[block as usize] = true;
            file.size = file.size.max(block + 1);
        } else if what < 900 {
            let file = &mut files[index];
            let blocks = next() % BLOCKS;
            let mut size = 0;
            assert_eq!(
                afsplus_aros_set_file_size(
                    fs,
                    file.handle,
                    (blocks * BLOCK as u64) as i64,
                    AFSPLUS_AROS_SEEK_BEGINNING,
                    seconds,
                    nanos,
                    &mut size
                ),
                0,
                "truncate"
            );
            for cut in blocks..BLOCKS {
                file.written[cut as usize] = false;
            }
            file.size = blocks;
        } else if what < 955 {
            // a handle of its own, read from the start and closed
            let file = &files[index];
            let own = run.open(&file.name, AFSPLUS_AROS_OPEN_READ_WRITE);
            for k in 0..8u64.min(file.size) {
                let mut count = 0;
                assert_eq!(
                    afsplus_aros_read(fs, own, buffer.as_mut_ptr(), BLOCK as u32, &mut count),
                    0,
                    "sequential read"
                );
                run.check(slot, file, k, &buffer, count);
            }
            assert_eq!(afsplus_aros_close(fs, own), 0);
        } else if what < 980 {
            // one more handle on the file, written through and closed
            let more = run.open(&files[index].name, AFSPLUS_AROS_OPEN_READ_WRITE);
            let file = &mut files[index];
            if block <= file.size {
                let data = pattern(slot, block);
                let mut count = 0;
                assert_eq!(
                    afsplus_aros_write_at(
                        fs,
                        more,
                        block * BLOCK as u64,
                        data.as_ptr(),
                        BLOCK as u32,
                        seconds,
                        nanos,
                        &mut count
                    ),
                    0
                );
                file.written[block as usize] = true;
                file.size = file.size.max(block + 1);
            }
            assert_eq!(afsplus_aros_close(fs, more), 0);
        } else {
            // An owner ends: its files are closed and, every other time,
            // deleted (a killed process leaves them). New ones replace them.
            let owner = index / FILES;
            let remove = next() % 2 == 0;
            for slot in 0..FILES {
                let file = &files[owner * FILES + slot];
                assert_eq!(afsplus_aros_close(fs, file.handle), 0);
                if remove {
                    assert_eq!(
                        afsplus_aros_delete_object(
                            fs,
                            base,
                            file.name.as_ptr(),
                            file.name.len() as u32,
                            seconds,
                            nanos
                        ),
                        0,
                        "delete"
                    );
                }
            }
            for slot in 0..FILES {
                files[owner * FILES + slot] = run.create(slot, generation);
            }
            generation += 1;
            for (index, file) in files.iter().enumerate() {
                for other in &files[index + 1..] {
                    assert_ne!(
                        file.object, other.object,
                        "seed {seed} step {step}: {} and {} are one object",
                        file.name, other.name
                    );
                }
            }
        }
        // The handler's rule (native/aros/afsplus_commit_tick.h): ask after
        // a packet that may have changed the volume, while the library says
        // work waits, and otherwise once in 64 packets.
        quiet += 1;
        let asked = if busy_ticks {
            // A handler under load: every batch of packets holds a write,
            // so the library is never asked while its window is closed.
            (450..850).contains(&what)
        } else {
            pending != 0 || what >= 450 || quiet >= 64
        };
        if delayed && asked {
            quiet = 0;
            assert_eq!(
                afsplus_aros_commit_due(fs, seconds, nanos, &mut pending),
                0,
                "seed {seed} step {step}: commit"
            );
        }
    }
    if std::env::var("RANDOM_IO_REPORT").is_ok() {
        eprintln!(
            "random_io: {generation} owners, objects up to {}, {} s of clock",
            files.iter().map(|file| file.object).max().unwrap_or(0),
            run.seconds
        );
    }
    for file in &files {
        assert_eq!(afsplus_aros_close(fs, file.handle), 0);
    }
}

/// An image file read where it lies, with what is written kept in memory:
/// the image is not changed.
struct Overlay {
    image: std::fs::File,
    written: std::collections::HashMap<u64, Box<[u8; BLOCK]>>,
}

unsafe extern "C" fn overlay_read(
    context: *mut c_void,
    lba: u64,
    destination: *mut u8,
    length: u32,
) -> i32 {
    use std::os::unix::fs::FileExt;
    // SAFETY: the test keeps the overlay and the buffer alive for the call.
    let overlay = unsafe { &mut *context.cast::<Overlay>() };
    let destination = unsafe { slice::from_raw_parts_mut(destination, length as usize) };
    for (index, block) in destination.chunks_exact_mut(BLOCK).enumerate() {
        let at = lba + index as u64;
        match overlay.written.get(&at) {
            Some(data) => block.copy_from_slice(&data[..]),
            None => {
                if overlay
                    .image
                    .read_exact_at(block, at * BLOCK as u64)
                    .is_err()
                {
                    return 5;
                }
            }
        }
    }
    0
}
unsafe extern "C" fn overlay_write(
    context: *mut c_void,
    lba: u64,
    source: *const u8,
    length: u32,
) -> i32 {
    // SAFETY: as above.
    let overlay = unsafe { &mut *context.cast::<Overlay>() };
    let source = unsafe { slice::from_raw_parts(source, length as usize) };
    for (index, block) in source.chunks_exact(BLOCK).enumerate() {
        overlay
            .written
            .insert(lba + index as u64, Box::new(block.try_into().unwrap()));
    }
    0
}
unsafe extern "C" fn overlay_flush(_context: *mut c_void) -> i32 {
    0
}

/// The same exercise on a copy of an existing volume, in one of its
/// directories: RANDOM_IO_IMAGE names the image, RANDOM_IO_DIRECTORY the
/// directory (the root without it), RANDOM_IO_OPERATIONS how long,
/// RANDOM_IO_TURNOVER how many operations between two ends of an owner.
/// With RANDOM_IO_BUSY the library is asked for its commits only after a
/// write, as by a handler that is never idle: four million operations of
/// it on a 32 GiB volume filled the reclaim queue and lost a window
/// (afsplus-vfs tests/busy_volume.rs). A volume that has lived has trees
/// and maps a new one has not.
#[test]
fn random_io_on_an_existing_volume() {
    let Ok(path) = std::env::var("RANDOM_IO_IMAGE") else {
        return;
    };
    let image = std::fs::File::open(&path).unwrap();
    let total_blocks = image.metadata().unwrap().len() / BLOCK as u64;
    let mut overlay = Overlay {
        image,
        written: Default::default(),
    };
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(&mut overlay).cast::<c_void>(),
        block_size: BLOCK as u32,
        max_transfer_blocks: 256,
        total_blocks,
        read_block: Some(overlay_read),
        write_block: Some(overlay_write),
        flush: Some(overlay_flush),
    };
    let config = AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: b"AFS+".as_ptr(),
        volume_name_length: 4,
        max_file_handles: 1024,
        max_locks: 1024,
        max_file_info_name_bytes: 107,
        flags: 0,
        snapshot_max_edit_records: 0,
        snapshot_max_views: 0,
        snapshot_reclaim_records: 0,
    };
    let mut fs = ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut fs), 0);
    let mut granted = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 65536, &mut granted), 0);
    assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    let mut taken = 0;
    assert_eq!(afsplus_aros_set_window_ops(fs, 512, &mut taken), 0);
    let mut base = 0;
    if let Ok(directory) = std::env::var("RANDOM_IO_DIRECTORY") {
        assert_eq!(
            afsplus_aros_locate_path(
                fs,
                0,
                directory.as_ptr(),
                directory.len() as u32,
                AFSPLUS_AROS_LOCK_SHARED,
                &mut base
            ),
            0
        );
    }
    let operations = std::env::var("RANDOM_IO_OPERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60000);
    let turnover = std::env::var("RANDOM_IO_TURNOVER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000);
    let busy = std::env::var("RANDOM_IO_BUSY").is_ok();
    exercise(
        fs,
        base,
        101,
        0x2545_f491_4f6c_dd1d,
        operations,
        true,
        0,
        12,
        turnover,
        busy,
    );
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

/// Minutes of the handler's clock, so that the window closes many times
/// while files come and go; the second seed as a handler that is never idle.
#[test]
fn random_io_over_many_commits() {
    for seed in 1..=2u64 {
        run(
            seed.wrapping_mul(0x2545_f491_4f6c_dd1d),
            30000,
            16384,
            true,
            100,
            60,
            seed == 2,
        );
    }
}

#[test]
fn random_io_matches_the_model() {
    let rounds: u64 = std::env::var("RANDOM_IO_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    for seed in 1..=rounds {
        for cache in [0u32, 64, 1024, 16384] {
            for delayed in [false, true] {
                for fillers in [0, 1500] {
                    run(
                        seed.wrapping_mul(0x9e37_79b9_7f4a_7c15),
                        8000,
                        cache,
                        delayed,
                        fillers,
                        8,
                        false,
                    );
                }
            }
        }
    }
}
