// SPDX-License-Identifier: BSD-2-Clause
//! Real C callbacks preserve the explicit uncertain result across the FFI.
use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use std::{ffi::c_void, mem::size_of, ptr, slice};
struct Device {
    image: MemoryBackend,
    flushes: usize,
    fail: usize,
}
unsafe extern "C" fn read(context: *mut c_void, lba: u64, out: *mut u8, len: u32) -> i32 {
    // SAFETY: mount and the synchronous caller retain the device and block.
    let (device, out) = unsafe {
        (
            &mut *context.cast::<Device>(),
            slice::from_raw_parts_mut(out, len as usize),
        )
    };
    device.image.read_block(lba, out).map_or(5, |()| 0)
}
unsafe extern "C" fn write(context: *mut c_void, lba: u64, input: *const u8, len: u32) -> i32 {
    // SAFETY: mount and the synchronous caller retain the device and block.
    let (device, input) = unsafe {
        (
            &mut *context.cast::<Device>(),
            slice::from_raw_parts(input, len as usize),
        )
    };
    device.image.write_block(lba, input).map_or(5, |()| 0)
}
unsafe extern "C" fn flush(context: *mut c_void) -> i32 {
    // SAFETY: mount retains the device for the entire callback lifetime.
    let device = unsafe { &mut *context.cast::<Device>() };
    device.flushes += 1;
    if device.flushes == device.fail {
        5
    } else {
        0
    }
}
fn fixture() -> (Box<Device>, *mut AfsplusAros, u64, u64) {
    let mut device = Box::new(Device {
        image: MemoryBackend::new(4096, 2048),
        flushes: 0,
        fail: 0,
    });
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(device.as_mut()).cast(),
        block_size: 4096,
        reserved: 0,
        total_blocks: 2048,
        read_block: Some(read),
        write_block: Some(write),
        flush: Some(flush),
    };
    assert_eq!(
        afsplus_aros_format(
            &callbacks,
            AFSPLUS_AROS_ENCODING_UTF8,
            b"Outcome".as_ptr(),
            7,
            [0x34; 16].as_ptr(),
            0,
            0,
            1
        ),
        0
    );
    let config = AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: ptr::null(),
        volume_name_length: 0,
        max_file_handles: 16,
        max_locks: 16,
        max_file_info_name_bytes: 107,
        flags: 0,
        snapshot_max_edit_records: 4096,
        snapshot_max_views: 8,
        snapshot_reclaim_records: 64,
    };
    let mut fs = ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut fs), 0);
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            0,
            b"source".as_ptr(),
            6,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            0,
            0,
            &mut file
        ),
        0
    );
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write(fs, file, b"saved".as_ptr(), 5, 0, 0, &mut count),
        0
    );
    assert_eq!(count, 5);
    assert_eq!(afsplus_aros_close(fs, file), 0);
    let mut snapshot = 0;
    assert_eq!(afsplus_aros_snapshot_create(fs, 0, 0, &mut snapshot), 0);
    let mut reader = 0;
    assert_eq!(afsplus_aros_snapshot_open(fs, snapshot, 1, &mut reader), 0);
    let mut source = 0;
    assert_eq!(
        afsplus_aros_snapshot_lookup(fs, reader, 1, b"source".as_ptr(), 6, 1, &mut source),
        0
    );
    device.flushes = 0;
    (device, fs, reader, source)
}
fn restore(fs: *mut AfsplusAros, reader: u64, source: u64) -> i32 {
    let mut output = 0;
    afsplus_aros_snapshot_restore_file(
        fs,
        reader,
        source,
        0,
        b"restored".as_ptr(),
        8,
        0,
        0,
        1,
        &mut output,
    )
}
#[test]
fn callback_publication_failure_is_50001_and_prepublication_io_is_100() {
    let (device, fs, reader, source) = fixture();
    assert_eq!(restore(fs, reader, source), 0);
    let total = device.flushes;
    assert_eq!(afsplus_aros_snapshot_close(fs, reader, 1), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
    for (ordinal, expected) in [
        (1, 100),
        (total - 2, 100),
        (total - 1, 50001),
        (total, 50001),
    ] {
        let (mut device, fs, reader, source) = fixture();
        device.fail = ordinal;
        assert_eq!(
            restore(fs, reader, source),
            expected,
            "flush {ordinal}/{total}"
        );
        device.fail = 0;
        assert_eq!(afsplus_aros_snapshot_close(fs, reader, 1), 0);
        // Unmount consumes the bridge even if its poisoned state refuses sync.
        let _ = afsplus_aros_unmount(fs);
    }
}
