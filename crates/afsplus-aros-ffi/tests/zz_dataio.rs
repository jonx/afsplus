//! File data as the AROS handler reads it: how many device requests and
//! bytes one read call costs. The device callbacks take runs of blocks, as
//! the handler's do, and the mount is given the handler's transfer bound.
//! Ignored by default.
//!
//!   cargo test --release -p afsplus-aros-ffi --test zz_dataio -- --ignored --nocapture
mod common;
use std::ffi::c_void;
use std::time::Instant;

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use common::format;

const BLOCK: usize = 4096;
/// 320 MiB, sparse.
const BLOCKS: u64 = 81920;
/// Blocks a device request may carry: 1 MiB, the handler's own bound.
const MAX_TRANSFER: u32 = 256;
/// Read cache, in blocks: 16 MiB, a quarter of the large file.
const CACHE: u32 = 4096;

unsafe extern "C" fn read_run(context: *mut c_void, lba: u64, to: *mut u8, length: u32) -> i32 {
    // SAFETY: the test keeps the backend and the buffer alive for the call.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    let to = unsafe { std::slice::from_raw_parts_mut(to, length as usize) };
    for (index, block) in to.chunks_exact_mut(BLOCK).enumerate() {
        if device.read_block(lba + index as u64, block).is_err() {
            return 5;
        }
    }
    0
}

unsafe extern "C" fn write_run(
    context: *mut c_void,
    lba: u64,
    from: *const u8,
    length: u32,
) -> i32 {
    // SAFETY: as above.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    let from = unsafe { std::slice::from_raw_parts(from, length as usize) };
    for (index, block) in from.chunks_exact(BLOCK).enumerate() {
        if device.write_block(lba + index as u64, block).is_err() {
            return 5;
        }
    }
    0
}

unsafe extern "C" fn flush(_: *mut c_void) -> i32 {
    0
}

fn mount(device: &mut MemoryBackend) -> *mut AfsplusAros {
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: std::mem::size_of::<AfsplusArosDevice>() as u32,
        context: std::ptr::from_mut(device).cast::<c_void>(),
        block_size: BLOCK as u32,
        max_transfer_blocks: MAX_TRANSFER,
        total_blocks: BLOCKS,
        read_block: Some(read_run),
        write_block: Some(write_run),
        flush: Some(flush),
    };
    let config = AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: std::mem::size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: b"AFS+".as_ptr(),
        volume_name_length: 4,
        max_file_handles: 8,
        max_locks: 8,
        max_file_info_name_bytes: 107,
        flags: 0,
        snapshot_max_edit_records: 0,
        snapshot_max_views: 0,
        snapshot_reclaim_records: 0,
    };
    let mut filesystem = std::ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut filesystem), 0);
    let mut granted = 0;
    assert_eq!(
        afsplus_aros_set_cache_blocks(filesystem, CACHE, &mut granted),
        0
    );
    assert_eq!(afsplus_aros_set_commit_policy(filesystem, 5000, 1000), 0);
    filesystem
}

fn counters(fs: *mut AfsplusAros) -> AfsplusArosCounters {
    let mut counters = AfsplusArosCounters {
        struct_size: std::mem::size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(fs, &mut counters), 0);
    counters
}

fn open(fs: *mut AfsplusAros, name: &str, mode: u32) -> u64 {
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            0,
            name.as_ptr(),
            name.len() as u32,
            mode,
            10,
            0,
            &mut file
        ),
        0
    );
    file
}

/// What one call of `body` costs the device: requests, bytes, and time.
fn measure(fs: *mut AfsplusAros, name: &str, calls: usize, mut body: impl FnMut(usize)) {
    let before = counters(fs);
    let start = Instant::now();
    for i in 0..calls {
        body(i);
    }
    let ns = start.elapsed().as_nanos() as f64 / calls as f64;
    let after = counters(fs);
    println!(
        "DATA {name:<34} {ns:>9.0} ns  {:>7.3} device reads  {:>9.0} bytes read  {:>7.3} device writes  {:>7.3} flushes  ({calls} calls)",
        (after.device_reads - before.device_reads) as f64 / calls as f64,
        (after.device_read_bytes - before.device_read_bytes) as f64 / calls as f64,
        (after.device_writes - before.device_writes) as f64 / calls as f64,
        (after.device_flushes - before.device_flushes) as f64 / calls as f64,
    );
}

fn read_at(fs: *mut AfsplusAros, file: u64, offset: u64, buffer: &mut [u8]) {
    let mut count = 0;
    assert_eq!(
        afsplus_aros_read_at(
            fs,
            file,
            offset,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut count
        ),
        0
    );
    assert_eq!(count as usize, buffer.len());
}

/// The byte a file holds at `offset`, so that a read can be checked.
fn pattern(offset: u64) -> u8 {
    (offset / 4096 * 31 + offset % 251) as u8
}

fn check(offset: u64, buffer: &[u8]) {
    for at in [0, buffer.len() / 2, buffer.len() - 1] {
        assert_eq!(
            buffer[at],
            pattern(offset + at as u64),
            "byte {}",
            offset + at as u64
        );
    }
}

#[test]
#[ignore]
fn data_paths() {
    const SMALL: u64 = 4 << 20;
    const LARGE: u64 = 64 << 20;
    const CALL: usize = 64 << 10;
    let mut device = format(MemoryBackend::new(BLOCK, BLOCKS), false);
    let mut fs = mount(&mut device);

    // The files, written in 64 KiB calls and committed.
    let mut chunk = vec![0u8; CALL];
    for (name, size) in [("four.bin", SMALL), ("sixtyfour.bin", LARGE)] {
        let file = open(fs, name, AFSPLUS_AROS_OPEN_NEW_FILE);
        let calls = (size / CALL as u64) as usize;
        measure(fs, &format!("write {name} 64K/call"), calls, |i| {
            let offset = (i * CALL) as u64;
            for (at, byte) in chunk.iter_mut().enumerate() {
                *byte = pattern(offset + at as u64);
            }
            let mut count = 0;
            assert_eq!(
                afsplus_aros_write_at(
                    fs,
                    file,
                    offset,
                    chunk.as_ptr(),
                    CALL as u32,
                    20 + i as i64,
                    0,
                    &mut count
                ),
                0
            );
        });
        assert_eq!(afsplus_aros_close(fs, file), 0);
        let before = counters(fs);
        assert_eq!(afsplus_aros_flush(fs), 0);
        let after = counters(fs);
        println!(
            "DATA commit of {name:<24} {:>5} device writes  {:>3} flushes",
            after.device_writes - before.device_writes,
            after.device_flushes - before.device_flushes
        );
    }

    let mut state = 0x9E37_79B9u32;
    let mut random = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        u64::from(state)
    };
    // Each cold pass follows a remount: nothing of the file is in memory.
    let mut small = [0u8; 4096];
    for pass in ["cold", "again"] {
        if pass == "cold" {
            assert_eq!(afsplus_aros_unmount(fs), 0);
            fs = mount(&mut device);
        }
        let four = open(fs, "four.bin", AFSPLUS_AROS_OPEN_OLD_FILE);
        measure(
            fs,
            &format!("sequential 4M in 64K calls, {pass}"),
            64,
            |i| {
                read_at(fs, four, (i * CALL) as u64, &mut chunk);
                check((i * CALL) as u64, &chunk);
            },
        );
        assert_eq!(afsplus_aros_close(fs, four), 0);
    }
    for pass in ["cold", "again"] {
        if pass == "cold" {
            assert_eq!(afsplus_aros_unmount(fs), 0);
            fs = mount(&mut device);
        }
        let four = open(fs, "four.bin", AFSPLUS_AROS_OPEN_OLD_FILE);
        measure(
            fs,
            &format!("sequential 4M in 4K calls, {pass}"),
            1024,
            |i| {
                read_at(fs, four, (i * 4096) as u64, &mut small);
                check((i * 4096) as u64, &small);
            },
        );
        assert_eq!(afsplus_aros_close(fs, four), 0);
    }

    // Random 4 KiB reads over a file four times the cache: cold after a
    // remount, then the same offsets again, then a set that fits the cache.
    assert_eq!(afsplus_aros_unmount(fs), 0);
    fs = mount(&mut device);
    let large = open(fs, "sixtyfour.bin", AFSPLUS_AROS_OPEN_OLD_FILE);
    let offsets: Vec<u64> = (0..4096)
        .map(|_| random() % (LARGE / 4096) * 4096)
        .collect();
    let mut small = [0u8; 4096];
    for pass in ["cold", "again"] {
        measure(
            fs,
            &format!("random 4K of 64M, {pass}"),
            offsets.len(),
            |i| {
                read_at(fs, large, offsets[i], &mut small);
                check(offsets[i], &small);
            },
        );
    }
    let few: Vec<u64> = offsets.iter().copied().take(CACHE as usize / 4).collect();
    for pass in ["within the cache, first", "within the cache, again"] {
        measure(fs, &format!("random 4K, {pass}"), few.len(), |i| {
            read_at(fs, large, few[i], &mut small);
            check(few[i], &small);
        });
    }
    let mut ragged = [0u8; 4096];
    measure(fs, "random 4K unaligned, cold", 1024, |i| {
        let offset = (LARGE / 2 + i as u64 * 64 * 4096 + 1000) % (LARGE - 8192);
        read_at(fs, large, offset, &mut ragged);
        check(offset, &ragged);
    });
    assert_eq!(afsplus_aros_close(fs, large), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}
