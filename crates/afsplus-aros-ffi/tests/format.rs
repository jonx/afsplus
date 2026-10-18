//! Explicit format from blank media, label conversion and pre-write refusals.
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::slice;

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};

struct Device {
    image: MemoryBackend,
    writes: u32,
    fail_write: bool,
}

unsafe extern "C" fn read(context: *mut c_void, lba: u64, out: *mut u8, len: u32) -> i32 {
    // SAFETY: the synchronous caller retains the device and complete block buffer.
    let (device, out) = unsafe {
        (
            &mut *context.cast::<Device>(),
            slice::from_raw_parts_mut(out, len as usize),
        )
    };
    device.image.read_block(lba, out).map_or(5, |()| 0)
}

unsafe extern "C" fn write(context: *mut c_void, lba: u64, input: *const u8, len: u32) -> i32 {
    // SAFETY: the synchronous caller retains the device and complete block buffer.
    let (device, input) = unsafe {
        (
            &mut *context.cast::<Device>(),
            slice::from_raw_parts(input, len as usize),
        )
    };
    device.writes += 1;
    if device.fail_write {
        return 5;
    }
    device.image.write_block(lba, input).map_or(5, |()| 0)
}

unsafe extern "C" fn flush(_: *mut c_void) -> i32 {
    0
}

fn device() -> Device {
    Device {
        image: MemoryBackend::new(4096, 2048),
        writes: 0,
        fail_write: false,
    }
}

fn callbacks(device: &mut Device) -> AfsplusArosDevice {
    AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(device).cast(),
        block_size: 4096,
        reserved: 0,
        total_blocks: 2048,
        read_block: Some(read),
        write_block: Some(write),
        flush: Some(flush),
    }
}

fn config() -> AfsplusArosMountConfig {
    AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_LATIN1,
        volume_name: ptr::null(),
        volume_name_length: 0,
        max_file_handles: 16,
        max_locks: 16,
        max_file_info_name_bytes: 107,
        flags: 0,
        snapshot_max_edit_records: 0,
        snapshot_max_views: 0,
        snapshot_reclaim_records: 0,
    }
}

#[test]
fn blank_device_formats_mounts_and_preserves_latin1_label_and_case_policy() {
    let mut device = device();
    let callbacks = callbacks(&mut device);
    let mut filesystem = ptr::null_mut();
    assert_eq!(
        afsplus_aros_mount(&callbacks, &config(), &mut filesystem),
        225
    );
    assert!(filesystem.is_null());
    assert_eq!(device.writes, 0);
    let label = b"Syst\xe8me";
    assert_eq!(
        afsplus_aros_format(
            &callbacks,
            AFSPLUS_AROS_ENCODING_LATIN1,
            label.as_ptr(),
            label.len() as u32,
            [0x22; 16].as_ptr(),
            100,
            0,
            0
        ),
        0
    );
    assert_eq!(
        afsplus_aros_mount(&callbacks, &config(), &mut filesystem),
        0
    );
    let mut output = [0u8; 64];
    let mut length = 0;
    assert_eq!(
        afsplus_aros_volume_label(filesystem, output.as_mut_ptr(), 64, &mut length),
        0
    );
    assert_eq!(&output[..length as usize], label);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    let mut volume = afsplus_core::mount(device.image).unwrap();
    assert_eq!(volume.volume_label(), "Système");
    let file = volume
        .create_file_in_root(
            "ReadMe",
            b"formatted under AROS",
            afsplus_format::Timespec {
                seconds: 101,
                nanoseconds: 0,
            },
        )
        .unwrap();
    let mut volume = afsplus_core::mount(volume.into_device()).unwrap();
    assert_eq!(volume.lookup_root("README").unwrap(), Some(file));
    assert_eq!(volume.read_file(file).unwrap(), b"formatted under AROS");
}

#[test]
fn invalid_labels_and_read_only_devices_are_refused_before_writes() {
    let mut device = device();
    let mut callbacks = callbacks(&mut device);
    for label in [
        &b""[..],
        &b"bad/name"[..],
        &b"bad:name"[..],
        &b"bad\0name"[..],
        &[0xe9; 33][..],
    ] {
        assert_ne!(
            afsplus_aros_validate_format_label(
                AFSPLUS_AROS_ENCODING_LATIN1,
                label.as_ptr(),
                label.len() as u32
            ),
            0
        );
        assert_ne!(
            afsplus_aros_format(
                &callbacks,
                AFSPLUS_AROS_ENCODING_LATIN1,
                label.as_ptr(),
                label.len() as u32,
                [1; 16].as_ptr(),
                0,
                0,
                0
            ),
            0
        );
        assert_eq!(device.writes, 0);
    }
    assert_ne!(
        afsplus_aros_format(
            &callbacks,
            AFSPLUS_AROS_ENCODING_UTF8,
            [0xff].as_ptr(),
            1,
            [1; 16].as_ptr(),
            0,
            0,
            0
        ),
        0
    );
    assert_eq!(device.writes, 0);
    callbacks.write_block = None;
    assert_eq!(
        afsplus_aros_format(
            &callbacks,
            AFSPLUS_AROS_ENCODING_LATIN1,
            b"Work".as_ptr(),
            4,
            [1; 16].as_ptr(),
            0,
            0,
            0
        ),
        214
    );
    assert_eq!(device.writes, 0);
}

#[test]
fn rejected_format_preserves_every_block_of_an_existing_volume() {
    let mut device = device();
    let callbacks = callbacks(&mut device);
    assert_eq!(
        afsplus_aros_format(
            &callbacks,
            AFSPLUS_AROS_ENCODING_UTF8,
            b"KeepMe".as_ptr(),
            6,
            [3; 16].as_ptr(),
            123,
            0,
            0
        ),
        0
    );
    let before = device.image.clone();
    device.writes = 0;
    for (encoding, label) in [
        (AFSPLUS_AROS_ENCODING_LATIN1, &[0xe9; 33][..]),
        (AFSPLUS_AROS_ENCODING_UTF8, &[0xff][..]),
        (AFSPLUS_AROS_ENCODING_UTF8, &b"bad/name"[..]),
    ] {
        assert_ne!(
            afsplus_aros_validate_format_label(encoding, label.as_ptr(), label.len() as u32),
            0
        );
        assert_ne!(
            afsplus_aros_format(
                &callbacks,
                encoding,
                label.as_ptr(),
                label.len() as u32,
                [4; 16].as_ptr(),
                124,
                0,
                0
            ),
            0
        );
    }
    assert_eq!(device.writes, 0);
    for lba in 0..2048 {
        assert_eq!(device.image.peek(lba), before.peek(lba));
    }
    assert_eq!(
        afsplus_core::mount(device.image).unwrap().volume_label(),
        "KeepMe"
    );
}

#[test]
fn device_write_failure_is_not_reported_as_a_successful_format() {
    let mut device = device();
    device.fail_write = true;
    let callbacks = callbacks(&mut device);
    assert_ne!(
        afsplus_aros_format(
            &callbacks,
            AFSPLUS_AROS_ENCODING_UTF8,
            b"Work".as_ptr(),
            4,
            [1; 16].as_ptr(),
            0,
            0,
            0
        ),
        0
    );
    assert!(device.writes != 0);
    let mut filesystem = ptr::null_mut();
    assert_ne!(
        afsplus_aros_mount(&callbacks, &config(), &mut filesystem),
        0
    );
    assert!(filesystem.is_null());
}

#[test]
fn snapshot_format_requires_explicit_mount_budgets_and_preserves_feature() {
    let mut device = device();
    let cb = callbacks(&mut device);
    assert_ne!(
        afsplus_aros_format(
            &cb,
            AFSPLUS_AROS_ENCODING_UTF8,
            b"Snapshot".as_ptr(),
            8,
            [0x53; 16].as_ptr(),
            0,
            0,
            2
        ),
        0
    );
    assert_eq!(device.writes, 0);
    assert_eq!(
        afsplus_aros_format(
            &cb,
            AFSPLUS_AROS_ENCODING_UTF8,
            b"Snapshot".as_ptr(),
            8,
            [0x53; 16].as_ptr(),
            0,
            0,
            1
        ),
        0
    );
    let mut fs = ptr::null_mut();
    assert_ne!(afsplus_aros_mount(&cb, &config(), &mut fs), 0);
    let mut cfg = config();
    cfg.snapshot_max_edit_records = 4096;
    cfg.snapshot_max_views = 8;
    cfg.snapshot_reclaim_records = 64;
    assert_eq!(afsplus_aros_mount(&cb, &cfg, &mut fs), 0);
    let mut capabilities = AfsplusArosCapabilities {
        struct_size: size_of::<AfsplusArosCapabilities>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_capabilities(fs, &mut capabilities), 0);
    assert_ne!(capabilities.capabilities & (1 << 18), 0);
    let mut file = 0;
    let mut written = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            0,
            b"prefs".as_ptr(),
            5,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            0,
            0,
            &mut file
        ),
        0
    );
    assert_eq!(
        afsplus_aros_write(fs, file, b"boot".as_ptr(), 4, 0, 0, &mut written),
        0
    );
    assert_eq!(written, 4);
    assert_eq!(afsplus_aros_close(fs, file), 0);
    assert_eq!(afsplus_aros_set_write_protect(fs, 1, 91), 0);
    let mut refused = 0;
    assert_eq!(afsplus_aros_snapshot_create(fs, 1, 0, &mut refused), 214);
    assert_eq!(afsplus_aros_set_write_protect(fs, 0, 91), 0);
    let mut snapshot = 0;
    assert_eq!(afsplus_aros_snapshot_create(fs, 1, 0, &mut snapshot), 0);
    assert_ne!(snapshot, 0);
    let mut reader = 0;
    assert_eq!(afsplus_aros_snapshot_open(fs, snapshot, 1, &mut reader), 0);
    let mut object = 0;
    assert_eq!(
        afsplus_aros_snapshot_lookup(fs, reader, 1, b"PREFS".as_ptr(), 5, 1, &mut object),
        0
    );
    let mut stat = AfsplusArosStat {
        struct_size: size_of::<AfsplusArosStat>() as u32,
        ..Default::default()
    };
    assert_eq!(
        afsplus_aros_snapshot_stat(fs, reader, object, 1, &mut stat),
        0
    );
    assert_eq!(stat.kind, 1);
    assert_eq!(stat.size, 4);
    stat.struct_size = size_of::<AfsplusArosStat>() as u32;
    assert_eq!(afsplus_aros_snapshot_stat(fs, reader, 1, 1, &mut stat), 0);
    assert_eq!(stat.kind, 2);
    let mut directory = 0;
    assert_eq!(
        afsplus_aros_snapshot_dir_open(fs, reader, 1, 1, &mut directory),
        0
    );
    let mut records = [0u8; 512];
    let mut count = 0;
    let mut eof = 0;
    assert_eq!(
        afsplus_aros_snapshot_dir_read(
            fs,
            directory,
            records.as_mut_ptr(),
            512,
            1,
            1,
            &mut count,
            &mut eof
        ),
        0
    );
    assert_eq!(count, 1);
    assert_eq!(u32::from_ne_bytes(records[8..12].try_into().unwrap()), 1);
    // Only the opening task closes a snapshot handle.
    assert_ne!(afsplus_aros_snapshot_dir_close(fs, directory, 2), 0);
    assert_eq!(afsplus_aros_snapshot_dir_close(fs, directory, 1), 0);
    let mut bytes = [0u8; 4];
    // Another task can neither read through the reader nor stat with it.
    assert_ne!(
        afsplus_aros_snapshot_read(fs, reader, object, 0, bytes.as_mut_ptr(), 4, 2, &mut count),
        0
    );
    assert_ne!(afsplus_aros_snapshot_stat(fs, reader, 1, 2, &mut stat), 0);
    assert_eq!(
        afsplus_aros_snapshot_read(fs, reader, object, 0, bytes.as_mut_ptr(), 4, 1, &mut count),
        0
    );
    assert_eq!(count, 4);
    assert_eq!(&bytes, b"boot");
    let mut restored = 0;
    assert_ne!(
        afsplus_aros_snapshot_restore_tree(
            fs,
            reader,
            1,
            0,
            b"tree".as_ptr(),
            4,
            0,
            64,
            4096,
            2,
            0,
            1,
            &mut restored
        ),
        0
    );
    assert_eq!(restored, 0);
    assert_eq!(
        afsplus_aros_snapshot_restore_tree(
            fs,
            reader,
            1,
            0,
            b"tree".as_ptr(),
            4,
            2,
            2,
            4096,
            2,
            0,
            1,
            &mut restored
        ),
        0
    );
    let mut restored_file = 0;
    let mut restored_lock = 0;
    assert_eq!(
        afsplus_aros_locate(
            fs,
            0,
            b"tree".as_ptr(),
            4,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut restored_lock
        ),
        0
    );
    assert_eq!(
        afsplus_aros_lookup_id(fs, restored_lock, b"prefs".as_ptr(), 5, &mut restored_file),
        0
    );
    stat.struct_size = size_of::<AfsplusArosStat>() as u32;
    assert_eq!(afsplus_aros_stat_id(fs, restored_file, &mut stat), 0);
    assert_eq!(stat.size, 4);
    assert_eq!(stat.kind, 1);
    assert_eq!(afsplus_aros_free_lock(fs, restored_lock), 0);
    let mut info = AfsplusArosDiskInfo::default();
    assert_eq!(afsplus_aros_disk_info(fs, &mut info), 0);
    assert_eq!(info.in_use, 1);
    assert_eq!(afsplus_aros_snapshot_delete(fs, snapshot, 2, 0), 202);
    assert_ne!(afsplus_aros_snapshot_close(fs, reader, 2), 0);
    assert_eq!(afsplus_aros_snapshot_close(fs, reader, 1), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
    assert_eq!(afsplus_aros_mount(&cb, &cfg, &mut fs), 0);
    let mut record = [0u8; 24];
    let mut count = 0;
    let mut next = 0;
    assert_eq!(
        afsplus_aros_snapshot_list(fs, 0, record.as_mut_ptr(), 24, 1, &mut count, &mut next),
        0
    );
    assert_eq!(count, 1);
    assert_eq!(next, snapshot + 1);
    assert_eq!(
        u64::from_ne_bytes(record[..8].try_into().unwrap()),
        snapshot
    );
    assert_eq!(afsplus_aros_set_write_protect(fs, 1, 92), 0);
    assert_eq!(afsplus_aros_snapshot_delete(fs, snapshot, 3, 0), 214);
    assert_eq!(afsplus_aros_set_write_protect(fs, 0, 92), 0);
    assert_eq!(afsplus_aros_snapshot_delete(fs, snapshot, 3, 0), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}
