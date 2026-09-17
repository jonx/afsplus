use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::slice;

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams};
use afsplus_format::Timespec;

const BLOCK_SIZE: usize = 4096;
const TOTAL_BLOCKS: u64 = 8192;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, TOTAL_BLOCKS);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xA3; 16],
            label: "ArosFfi".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: timestamp(0),
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
        max_file_handles: 64,
        max_locks: 64,
        max_file_info_name_bytes: 107,
        reserved: 0,
    };
    let mut filesystem = ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut filesystem), 0);
    assert!(!filesystem.is_null());
    filesystem
}

#[test]
fn c_boundary_runs_the_alpha_matrix_and_remounts_cleanly() {
    let mut device = formatted();
    let filesystem = mount(&mut device);

    let mut work = 0;
    assert_eq!(
        afsplus_aros_create_directory(filesystem, 0, b"work".as_ptr(), 4, 1, 0, &mut work,),
        0
    );

    let mut draft = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            work,
            b"draft".as_ptr(),
            5,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            2,
            0,
            &mut draft,
        ),
        0
    );
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write(filesystem, draft, b"hello".as_ptr(), 5, 3, 0, &mut count,),
        0
    );
    assert_eq!(count, 5);

    let mut old_position = 0;
    assert_eq!(
        afsplus_aros_seek(
            filesystem,
            draft,
            8192,
            AFSPLUS_AROS_SEEK_BEGINNING,
            &mut old_position,
        ),
        0
    );
    assert_eq!(old_position, 5);
    assert_eq!(
        afsplus_aros_write(filesystem, draft, b"tail".as_ptr(), 4, 4, 0, &mut count,),
        0
    );
    assert_eq!(
        afsplus_aros_seek(
            filesystem,
            draft,
            0,
            AFSPLUS_AROS_SEEK_BEGINNING,
            &mut old_position,
        ),
        0
    );
    assert_eq!(old_position, 8196);
    let mut visible_before_fsync = [0u8; 5];
    assert_eq!(
        afsplus_aros_read(
            filesystem,
            draft,
            visible_before_fsync.as_mut_ptr(),
            visible_before_fsync.len() as u32,
            &mut count,
        ),
        0
    );
    assert_eq!(count, 5);
    assert_eq!(&visible_before_fsync, b"hello");
    assert_eq!(afsplus_aros_fsync(filesystem, draft), 0);

    let mut size = 0;
    assert_eq!(
        afsplus_aros_set_file_size(
            filesystem,
            draft,
            5,
            AFSPLUS_AROS_SEEK_BEGINNING,
            5,
            0,
            &mut size,
        ),
        0
    );
    assert_eq!(size, 5);
    assert_eq!(afsplus_aros_close(filesystem, draft), 0);

    assert_eq!(
        afsplus_aros_rename(
            filesystem,
            work,
            b"draft".as_ptr(),
            5,
            0,
            b"final".as_ptr(),
            5,
            6,
            0,
        ),
        0
    );
    let mut final_lock = 0;
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            b"final".as_ptr(),
            5,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut final_lock,
        ),
        0
    );
    assert_eq!(
        afsplus_aros_make_hard_link(filesystem, work, b"linked".as_ptr(), 6, final_lock, 7, 0,),
        0
    );
    // A held object is not deletable; ERROR_OBJECT_IN_USE until released.
    assert_eq!(
        afsplus_aros_delete_object(filesystem, 0, b"final".as_ptr(), 5, 8, 0),
        202
    );
    assert_eq!(afsplus_aros_free_lock(filesystem, final_lock), 0);
    assert_eq!(
        afsplus_aros_delete_object(filesystem, 0, b"final".as_ptr(), 5, 8, 0),
        0
    );

    let mut linked = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            work,
            b"linked".as_ptr(),
            6,
            AFSPLUS_AROS_OPEN_OLD_FILE,
            9,
            0,
            &mut linked,
        ),
        0
    );
    let mut contents = [0u8; 8];
    assert_ne!(
        afsplus_aros_read(
            filesystem,
            linked,
            contents.as_mut_ptr(),
            contents.len() as u32,
            ptr::null_mut(),
        ),
        0
    );
    let mut position = u64::MAX;
    assert_eq!(
        afsplus_aros_file_position(filesystem, linked, &mut position),
        0
    );
    assert_eq!(position, 0, "rejected read must not advance the file");
    assert_ne!(
        afsplus_aros_seek(
            filesystem,
            linked,
            3,
            AFSPLUS_AROS_SEEK_BEGINNING,
            ptr::null_mut(),
        ),
        0
    );
    assert_eq!(
        afsplus_aros_file_position(filesystem, linked, &mut position),
        0
    );
    assert_eq!(position, 0, "rejected seek must not move the file");
    assert_eq!(
        afsplus_aros_read(
            filesystem,
            linked,
            contents.as_mut_ptr(),
            contents.len() as u32,
            &mut count,
        ),
        0
    );
    assert_eq!(&contents[..count as usize], b"hello");
    assert_eq!(afsplus_aros_close(filesystem, linked), 0);

    let mut info = AfsplusArosFileInfo::default();
    let mut name = [0u8; 107];
    assert_ne!(
        afsplus_aros_examine_next(
            filesystem,
            work,
            ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        ),
        0
    );
    assert_eq!(
        afsplus_aros_examine_next(
            filesystem,
            work,
            &mut info,
            name.as_mut_ptr(),
            name.len() as u32,
        ),
        0
    );
    assert_eq!(&name[..info.name_length as usize], b"linked");
    assert_eq!(info.size, 5);

    let mut disk_info = AfsplusArosDiskInfo::default();
    assert_eq!(afsplus_aros_disk_info(filesystem, &mut disk_info), 0);
    assert_eq!(disk_info.bytes_per_block, BLOCK_SIZE as u32);
    assert_eq!(disk_info.write_protected, 0);
    assert_eq!(disk_info.in_use, 1);
    assert_eq!(afsplus_aros_flush(filesystem), 0);
    assert_eq!(afsplus_aros_free_lock(filesystem, work), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);

    let filesystem = mount(&mut device);
    let mut work = 0;
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            b"work".as_ptr(),
            4,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut work,
        ),
        0
    );
    let mut linked = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            work,
            b"linked".as_ptr(),
            6,
            AFSPLUS_AROS_OPEN_OLD_FILE,
            10,
            0,
            &mut linked,
        ),
        0
    );
    let mut contents = [0u8; 5];
    assert_eq!(
        afsplus_aros_read(
            filesystem,
            linked,
            contents.as_mut_ptr(),
            contents.len() as u32,
            &mut count,
        ),
        0
    );
    assert_eq!(&contents, b"hello");
    assert_eq!(afsplus_aros_close(filesystem, linked), 0);
    assert_eq!(afsplus_aros_free_lock(filesystem, work), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);

    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn abi_version_and_struct_sizes_are_checked_before_mount() {
    let mut device = formatted();
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION + 1,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(&mut device).cast::<c_void>(),
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
        volume_name: ptr::null(),
        volume_name_length: 0,
        max_file_handles: 64,
        max_locks: 64,
        max_file_info_name_bytes: 107,
        reserved: 0,
    };
    let mut filesystem = ptr::null_mut();
    assert_ne!(afsplus_aros_mount(&callbacks, &config, &mut filesystem), 0);
    assert!(filesystem.is_null());
}
