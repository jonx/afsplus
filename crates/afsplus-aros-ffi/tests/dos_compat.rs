//! DOS metadata and soft-link entry points through the exported C functions.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::slice;

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, NamePolicy};
use afsplus_format::Timespec;

const BLOCK_SIZE: usize = 4096;
const TOTAL_BLOCKS: u64 = 8192;

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, TOTAL_BLOCKS);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC3; 16],
            label: "FfiDos".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
        },
    )
    .unwrap();
    device
}

unsafe extern "C" fn read_block(
    context: *mut c_void,
    lba: u64,
    destination: *mut u8,
    length: u32,
) -> i32 {
    // SAFETY: the test keeps this backend and destination alive for the call.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    // SAFETY: the bridge supplies exactly one writable block.
    let destination = unsafe { slice::from_raw_parts_mut(destination, length as usize) };
    device.read_block(lba, destination).map_or(5, |()| 0)
}

unsafe extern "C" fn write_block(
    context: *mut c_void,
    lba: u64,
    source: *const u8,
    length: u32,
) -> i32 {
    // SAFETY: the test keeps this backend and source alive for the call.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    // SAFETY: the bridge supplies exactly one readable block.
    let source = unsafe { slice::from_raw_parts(source, length as usize) };
    device.write_block(lba, source).map_or(5, |()| 0)
}

unsafe extern "C" fn flush(context: *mut c_void) -> i32 {
    // SAFETY: the test keeps this backend alive for the entire mount.
    let device = unsafe { &mut *context.cast::<MemoryBackend>() };
    device.flush().map_or(5, |()| 0)
}

fn mount(device: &mut MemoryBackend) -> *mut AfsplusAros {
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(device).cast::<c_void>(),
        block_size: BLOCK_SIZE as u32,
        reserved: 0,
        total_blocks: TOTAL_BLOCKS,
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
    filesystem
}

fn examine(filesystem: *mut AfsplusAros, name: &[u8]) -> AfsplusArosFileInfo {
    let mut lock = 0;
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            name.as_ptr(),
            name.len() as u32,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut lock,
        ),
        0
    );
    let mut info = AfsplusArosFileInfo::default();
    let mut text = [0u8; 107];
    assert_eq!(
        afsplus_aros_examine_lock(filesystem, lock, &mut info, text.as_mut_ptr(), 107),
        0
    );
    assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
    info
}

#[test]
fn c_boundary_sets_metadata_and_transports_soft_links() {
    let mut device = formatted();
    let filesystem = mount(&mut device);
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            10,
            0,
            &mut file,
        ),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, file), 0);

    assert_eq!(
        afsplus_aros_set_protection(filesystem, 0, b"note".as_ptr(), 4, 0x5A, 20, 0),
        0
    );
    assert_eq!(
        afsplus_aros_set_modified(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            252_460_861,
            40_000_000,
            21,
            0
        ),
        0
    );
    // Invalid nanoseconds and a missing object are refused by value.
    assert_eq!(
        afsplus_aros_set_modified(filesystem, 0, b"note".as_ptr(), 4, 1, 1_000_000_000, 22, 0),
        210
    );
    assert_eq!(
        afsplus_aros_set_protection(filesystem, 0, b"gone".as_ptr(), 4, 1, 23, 0),
        205
    );

    assert_eq!(
        afsplus_aros_make_soft_link(
            filesystem,
            0,
            b"alias".as_ptr(),
            5,
            b"Work:note".as_ptr(),
            9,
            24,
            0,
        ),
        0
    );
    let mut lock = 0;
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            b"alias".as_ptr(),
            5,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut lock,
        ),
        233
    );
    let mut target = [0xEEu8; 16];
    let mut required = 0;
    assert_eq!(
        afsplus_aros_read_soft_link(
            filesystem,
            0,
            b"alias".as_ptr(),
            5,
            target.as_mut_ptr(),
            16,
            &mut required,
        ),
        0
    );
    assert_eq!(required, 9);
    assert_eq!(&target[..10], b"Work:note\xEE");
    let mut short = [0xEEu8; 8];
    required = 0;
    assert_eq!(
        afsplus_aros_read_soft_link(
            filesystem,
            0,
            b"alias".as_ptr(),
            5,
            short.as_mut_ptr(),
            8,
            &mut required,
        ),
        0
    );
    assert_eq!(required, 9);
    assert_eq!(short, [0xEE; 8]);
    assert_eq!(
        afsplus_aros_read_soft_link(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            target.as_mut_ptr(),
            16,
            &mut required,
        ),
        212
    );
    assert_eq!(
        afsplus_aros_read_soft_link(
            filesystem,
            0,
            b"alias".as_ptr(),
            5,
            target.as_mut_ptr(),
            16,
            ptr::null_mut(),
        ),
        210
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);

    // The values survive a remount of a checker-clean image.
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    let filesystem = mount(&mut device);
    let info = examine(filesystem, b"note");
    assert_eq!(info.protection, 0x5A);
    assert_eq!(info.modified_seconds, 252_460_861);
    assert_eq!(info.modified_nanoseconds, 40_000_000);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

#[test]
fn c_boundary_opens_from_a_lock_changes_modes_and_write_protects() {
    let mut device = formatted();
    let filesystem = mount(&mut device);
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
    assert_eq!(afsplus_aros_close(filesystem, file), 0);

    let (mut lock, mut other) = (0, 0);
    for output in [&mut lock, &mut other] {
        assert_eq!(
            afsplus_aros_locate(
                filesystem,
                0,
                b"data".as_ptr(),
                4,
                AFSPLUS_AROS_LOCK_SHARED,
                output
            ),
            0
        );
    }
    // Two holders: no exclusivity, by value, and an unknown mode is refused.
    assert_eq!(
        afsplus_aros_change_lock_mode(filesystem, lock, AFSPLUS_AROS_LOCK_EXCLUSIVE),
        202
    );
    assert_eq!(afsplus_aros_change_lock_mode(filesystem, lock, 7), 210);

    let mut opened = 0;
    assert_eq!(
        afsplus_aros_open_from_lock(filesystem, lock, &mut opened),
        0
    );
    // The lock identifier died with the conversion.
    assert_eq!(afsplus_aros_free_lock(filesystem, lock), 211);
    assert_eq!(afsplus_aros_free_lock(filesystem, other), 0);
    assert_eq!(
        afsplus_aros_change_file_mode(filesystem, opened, AFSPLUS_AROS_LOCK_EXCLUSIVE),
        0
    );
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            b"data".as_ptr(),
            4,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut other
        ),
        202
    );
    assert_eq!(
        afsplus_aros_open_from_lock(filesystem, opened, ptr::null_mut()),
        210
    );
    assert_eq!(afsplus_aros_close(filesystem, opened), 0);

    // Write protection by value: refused mutation, wrong key, right key.
    assert_eq!(afsplus_aros_set_write_protect(filesystem, 1, 0xBEEF), 0);
    let mut info = AfsplusArosDiskInfo::default();
    assert_eq!(afsplus_aros_disk_info(filesystem, &mut info), 0);
    assert_eq!(info.write_protected, 1);
    assert_eq!(
        afsplus_aros_delete_object(filesystem, 0, b"data".as_ptr(), 4, 9, 0),
        214
    );
    assert_eq!(afsplus_aros_set_write_protect(filesystem, 0, 0xBEE0), 214);
    assert_eq!(afsplus_aros_set_write_protect(filesystem, 0, 0xBEEF), 0);
    assert_eq!(
        afsplus_aros_delete_object(filesystem, 0, b"data".as_ptr(), 4, 9, 0),
        0
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn c_boundary_locks_and_frees_records() {
    let mut device = formatted();
    let filesystem = mount(&mut device);
    let mut created = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            b"db".as_ptr(),
            2,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            1,
            0,
            &mut created
        ),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, created), 0);
    // Two shared handles on the same file.
    let (mut first, mut second) = (0, 0);
    for output in [&mut first, &mut second] {
        assert_eq!(
            afsplus_aros_open(
                filesystem,
                0,
                b"db".as_ptr(),
                2,
                AFSPLUS_AROS_OPEN_READ_WRITE,
                1,
                0,
                output
            ),
            0
        );
    }
    // Beyond 4 GiB, exclusive.
    assert_eq!(
        afsplus_aros_lock_record(filesystem, first, 0x1_0000_0000, 16, 1),
        0
    );
    assert_eq!(
        afsplus_aros_lock_record(filesystem, second, 0x1_0000_000F, 1, 0),
        241
    );
    assert_eq!(
        afsplus_aros_lock_record(filesystem, second, 0x1_0000_0010, 1, 0),
        0
    );
    assert_eq!(afsplus_aros_lock_record(filesystem, second, 0, 0, 0), 115);
    assert_eq!(
        afsplus_aros_free_record(filesystem, second, 0x1_0000_0000, 16),
        240
    );
    assert_eq!(
        afsplus_aros_free_record(filesystem, first, 0x1_0000_0000, 16),
        0
    );
    assert_eq!(
        afsplus_aros_lock_record(filesystem, second, 0x1_0000_000F, 1, 0),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, second), 0);
    assert_eq!(afsplus_aros_close(filesystem, first), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
