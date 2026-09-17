//! Handler counters against an independent count kept by the test device.

mod common;

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::slice;

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use common::formatted;

#[derive(Default)]
struct Seen {
    reads: u64,
    writes: u64,
    flushes: u64,
    read_bytes: u64,
    written_bytes: u64,
    refused: u64,
}

struct Counting {
    device: MemoryBackend,
    seen: Seen,
    fail_writes: bool,
}

unsafe extern "C" fn read_block(
    context: *mut c_void,
    lba: u64,
    destination: *mut u8,
    length: u32,
) -> i32 {
    // SAFETY: the test keeps the context and destination alive for the call.
    let counting = unsafe { &mut *context.cast::<Counting>() };
    // SAFETY: the bridge supplies exactly one writable block.
    let destination = unsafe { slice::from_raw_parts_mut(destination, length as usize) };
    counting.seen.reads += 1;
    counting.seen.read_bytes += u64::from(length);
    counting
        .device
        .read_block(lba, destination)
        .map_or(5, |()| 0)
}

unsafe extern "C" fn write_block(
    context: *mut c_void,
    lba: u64,
    source: *const u8,
    length: u32,
) -> i32 {
    // SAFETY: the test keeps the context and source alive for the call.
    let counting = unsafe { &mut *context.cast::<Counting>() };
    if counting.fail_writes {
        counting.seen.refused += 1;
        return 5;
    }
    // SAFETY: the bridge supplies exactly one readable block.
    let source = unsafe { slice::from_raw_parts(source, length as usize) };
    counting.seen.writes += 1;
    counting.seen.written_bytes += u64::from(length);
    counting.device.write_block(lba, source).map_or(5, |()| 0)
}

unsafe extern "C" fn flush(context: *mut c_void) -> i32 {
    // SAFETY: the test keeps the context alive for the entire mount.
    let counting = unsafe { &mut *context.cast::<Counting>() };
    counting.seen.flushes += 1;
    counting.device.flush().map_or(5, |()| 0)
}

fn counters(filesystem: *mut AfsplusAros) -> AfsplusArosCounters {
    let mut output = AfsplusArosCounters {
        struct_size: size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut output), 0);
    output
}

#[test]
fn counters_match_the_device_and_count_calls_and_failures() {
    let mut counting = Counting {
        device: formatted(true),
        seen: Seen::default(),
        fail_writes: false,
    };
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(&mut counting).cast::<c_void>(),
        block_size: 4096,
        reserved: 0,
        total_blocks: 8192,
        read_block: Some(read_block),
        write_block: Some(write_block),
        flush: Some(flush),
    };
    let config = AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: b"AFS+".as_ptr(),
        volume_name_length: 4,
        max_file_handles: 8,
        max_locks: 8,
        max_file_info_name_bytes: 107,
        flags: 0,
    };
    let mut filesystem = ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut filesystem), 0);

    // Mount traffic is already counted; no call has completed yet.
    let mounted = counters(filesystem);
    assert_eq!(mounted.struct_size, 72);
    assert_eq!((mounted.calls, mounted.failed_calls), (0, 0));
    assert!(mounted.device_reads > 0);
    assert_eq!(mounted.device_reads, counting.seen.reads);

    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            b"data".as_ptr(),
            4,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            1,
            0,
            &mut file
        ),
        0
    );
    let payload = [0x5Au8; 3 * 4096];
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write(
            filesystem,
            file,
            payload.as_ptr(),
            3 * 4096,
            2,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(afsplus_aros_fsync(filesystem, file), 0);
    assert_eq!(afsplus_aros_close(filesystem, file), 0);

    let worked = counters(filesystem);
    // counters, open, write, fsync, close.
    assert_eq!((worked.calls, worked.failed_calls), (5, 0));
    assert_eq!(
        (
            worked.device_reads,
            worked.device_writes,
            worked.device_flushes,
            worked.device_read_bytes,
            worked.device_written_bytes,
            worked.device_failures,
        ),
        (
            counting.seen.reads,
            counting.seen.writes,
            counting.seen.flushes,
            counting.seen.read_bytes,
            counting.seen.written_bytes,
            0,
        )
    );
    // Three payload blocks at least, each a whole 4096-byte block, and a
    // barrier for the fsync.
    assert!(worked.device_writes - mounted.device_writes >= 3);
    assert_eq!(worked.device_written_bytes, worked.device_writes * 4096);
    assert!(worked.device_flushes > mounted.device_flushes);

    // A refused device write is a device failure and a failed call, and it
    // is not counted as a write.
    counting.fail_writes = true;
    let mut lock = 0;
    assert_eq!(
        afsplus_aros_create_directory(filesystem, 0, b"doomed".as_ptr(), 6, 3, 0, &mut lock),
        100
    );
    let failed = counters(filesystem);
    assert_eq!((failed.calls, failed.failed_calls), (7, 1));
    assert_eq!(failed.device_failures, counting.seen.refused);
    assert!(failed.device_failures >= 1);
    assert_eq!(failed.device_writes, counting.seen.writes);
    counting.fail_writes = false;
    let _ = afsplus_aros_unmount(filesystem);
}
