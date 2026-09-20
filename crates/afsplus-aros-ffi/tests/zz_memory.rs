//! What one mounted volume costs in memory: the heap after a mount at three
//! buffer counts, the peak of a full delayed window, the allocations one
//! operation makes, and a histogram of the sizes asked for. It is a
//! measurement, not a gate, so every test here is ignored by default and the
//! whole file needs the `heap-profile` build:
//!
//! ```text
//! cargo test -q -p afsplus-aros-ffi --features heap-profile --test zz_memory -- --ignored --nocapture
//! ```
//!
//! The disk is materialized: the memory disk allocates a block the first
//! time it is written, and on a sparse one the meter would count the disk
//! filling up as the library's own memory.
#![cfg(feature = "heap-profile")]

mod common;

use std::mem::size_of;

use afsplus_aros_ffi::heap_profile;
use afsplus_aros_ffi::*;
use afsplus_block::MemoryBackend;

const BLOCK_SIZE: usize = 4096;

fn disk(total_blocks: u64) -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, total_blocks);
    let zeros = [0u8; BLOCK_SIZE];
    for lba in 0..total_blocks {
        device.apply_raw(lba, &zeros);
    }
    common::format(device, true)
}

fn counters(filesystem: *mut AfsplusAros) -> AfsplusArosCounters {
    let mut output = AfsplusArosCounters {
        struct_size: size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut output), 0);
    output
}

fn now() -> (i64, u32) {
    (1_700_000_000, 0)
}

/// (b) The heap a mount holds, with the cache set to 0, 64 and 1024 blocks.
/// One disk serves all three, so that only the mount differs between them.
#[test]
#[ignore = "measurement harness"]
fn heap_after_mount_at_three_buffer_counts() {
    let mut device = disk(8192);
    for buffers in [0u32, 64, 1024] {
        // The meter is the whole test binary's allocator, so the test disk
        // is on it too: the mount costs the difference.
        let before = heap_profile::heap().0;
        let filesystem = common::mount(&mut device);
        let mut granted = 0;
        assert_eq!(
            afsplus_aros_set_cache_blocks(filesystem, buffers, &mut granted),
            0
        );
        // The cache takes its memory at the next device access, so read the
        // volume once before the reading.
        walk(filesystem);
        let held = counters(filesystem).heap_bytes;
        println!(
            "mount Buffers={buffers} granted={granted}: {} bytes",
            held - before
        );
        assert_eq!(afsplus_aros_unmount(filesystem), 0);
    }
}

/// Reads enough of the volume to fill a read cache: every entry of the root.
fn walk(filesystem: *mut AfsplusAros) {
    let mut lock = 0;
    assert_eq!(afsplus_aros_locate(filesystem, 0, b"".as_ptr(), 0, 0, &mut lock), 0);
    let mut info = AfsplusArosFileInfo::default();
    let mut name = [0u8; 108];
    while afsplus_aros_examine_next(filesystem, lock, &mut info, name.as_mut_ptr(), 108) == 0 {}
    assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
}

/// (c) The peak of a full delayed window: 512 creates, 512 deletes and a
/// 16 MiB write. Every reading is the library's heap less what the test disk
/// held before the mount. `WINDOW_OPS` shortens the window.
#[test]
#[ignore = "measurement harness"]
fn peak_of_a_full_delayed_window() {
    let mut device = disk(32768);
    let baseline = heap_profile::heap().0;
    let filesystem = common::mount_sized(&mut device, 32768);
    let mut granted = 0;
    assert_eq!(
        afsplus_aros_set_cache_blocks(filesystem, 64, &mut granted),
        0
    );
    // The longest window the boundary allows, so that nothing but the
    // window's own bound closes it.
    assert_eq!(
        afsplus_aros_set_commit_policy(filesystem, 60_000, 60_000),
        0
    );
    if let Some(ops) = std::env::var("WINDOW_OPS").ok().and_then(|v| v.parse().ok()) {
        let mut taken = 0;
        assert_eq!(afsplus_aros_set_window_ops(filesystem, ops, &mut taken), 0);
        println!("the window holds {taken} changes");
    }
    let (seconds, nanoseconds) = now();
    // The blocks live at each point, so that what a per-block header would
    // have cost on AROS can be read off the table.
    heap_profile::reset();
    let report = |label: &str| {
        let read = counters(filesystem);
        let profile = heap_profile::sample();
        println!(
            "{label}: holds {}, peak {}, {} blocks live",
            read.heap_bytes - baseline,
            read.heap_peak_bytes - baseline,
            profile.allocations as i64 - profile.frees as i64
        );
    };
    report("mounted");

    for index in 0..512u32 {
        let name = format!("c{index:04}");
        let mut file = 0;
        assert_eq!(
            afsplus_aros_open(
                filesystem,
                0,
                name.as_ptr(),
                name.len() as u32,
                AFSPLUS_AROS_OPEN_NEW_FILE,
                seconds,
                nanoseconds,
                &mut file
            ),
            0
        );
        assert_eq!(afsplus_aros_close(filesystem, file), 0);
    }
    report("512 creates");

    for index in 0..512u32 {
        let name = format!("c{index:04}");
        assert_eq!(
            afsplus_aros_delete_object(
                filesystem,
                0,
                name.as_ptr(),
                name.len() as u32,
                seconds,
                nanoseconds
            ),
            0
        );
    }
    report("512 deletes");

    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            b"big".as_ptr(),
            3,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            seconds,
            nanoseconds,
            &mut file
        ),
        0
    );
    let chunk = vec![0x5au8; 64 * 1024];
    let mut written = 0;
    for _ in 0..(16 * 1024 / 64) {
        assert_eq!(
            afsplus_aros_write(
                filesystem,
                file,
                chunk.as_ptr(),
                chunk.len() as u32,
                seconds,
                nanoseconds,
                &mut written
            ),
            0
        );
    }
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
    report("a 16 MiB write");
    assert_eq!(afsplus_aros_flush(filesystem), 0);
    report("after the commit");
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

/// (d) and (e): the allocations one operation makes, and the sizes asked
/// for. Each phase is measured on its own after a flush, so that the window
/// commit of the phase before it is not charged to this one.
#[test]
#[ignore = "measurement harness"]
fn allocations_per_operation() {
    const OPERATIONS: u64 = 512;
    let mut device = disk(32768);
    let filesystem = common::mount_sized(&mut device, 32768);
    let mut granted = 0;
    assert_eq!(
        afsplus_aros_set_cache_blocks(filesystem, 64, &mut granted),
        0
    );
    let (seconds, nanoseconds) = now();

    let phase = |label: &str, body: &mut dyn FnMut()| {
        assert_eq!(afsplus_aros_flush(filesystem), 0);
        heap_profile::reset();
        body();
        assert_eq!(afsplus_aros_flush(filesystem), 0);
        let profile = heap_profile::sample();
        let per = |count: u64| count as f64 / OPERATIONS as f64;
        println!(
            "{label}: {:.1} allocations, {:.1} frees, {:.1} reallocations per operation; largest {} bytes",
            per(profile.allocations),
            per(profile.frees),
            per(profile.reallocations),
            profile.largest
        );
        let mut histogram = String::new();
        for (bucket, count) in profile.histogram.iter().enumerate() {
            if *count > 0 {
                histogram.push_str(&format!(" <{}:{}", 1u64 << bucket, count));
            }
        }
        println!("  sizes asked for, by power of two:{histogram}");
    };

    phase("create", &mut || {
        for index in 0..OPERATIONS {
            let name = format!("c{index:04}");
            let mut file = 0;
            assert_eq!(
                afsplus_aros_open(
                    filesystem,
                    0,
                    name.as_ptr(),
                    name.len() as u32,
                    AFSPLUS_AROS_OPEN_NEW_FILE,
                    seconds,
                    nanoseconds,
                    &mut file
                ),
                0
            );
            assert_eq!(afsplus_aros_close(filesystem, file), 0);
        }
    });

    phase("lookup", &mut || {
        for index in 0..OPERATIONS {
            let name = format!("c{index:04}");
            let mut lock = 0;
            assert_eq!(
                afsplus_aros_locate(
                    filesystem,
                    0,
                    name.as_ptr(),
                    name.len() as u32,
                    AFSPLUS_AROS_LOCK_SHARED,
                    &mut lock
                ),
                0
            );
            assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
        }
    });

    phase("rename", &mut || {
        for index in 0..OPERATIONS {
            let from = format!("c{index:04}");
            let to = format!("r{index:04}");
            assert_eq!(
                afsplus_aros_rename(
                    filesystem,
                    0,
                    from.as_ptr(),
                    from.len() as u32,
                    0,
                    to.as_ptr(),
                    to.len() as u32,
                    seconds,
                    nanoseconds
                ),
                0
            );
        }
    });

    phase("delete", &mut || {
        for index in 0..OPERATIONS {
            let name = format!("r{index:04}");
            assert_eq!(
                afsplus_aros_delete_object(
                    filesystem,
                    0,
                    name.as_ptr(),
                    name.len() as u32,
                    seconds,
                    nanoseconds
                ),
                0
            );
        }
    });

    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
