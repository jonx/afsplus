//! Shared mount fixture for C-boundary tests. Each test binary uses the part
//! it needs.
#![allow(dead_code)]

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::slice;

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{mkfs, MkfsParams, NamePolicy};
use afsplus_format::Timespec;

const BLOCK_SIZE: usize = 4096;
const TOTAL_BLOCKS: u64 = 8192;

pub fn formatted(shared_extents: bool) -> MemoryBackend {
    format(MemoryBackend::new(BLOCK_SIZE, TOTAL_BLOCKS), shared_extents)
}

/// A formatted disk whose every block is already in memory: the memory disk
/// allocates a block when it is first written, so on a sparse one the heap
/// meter counts the disk filling up as if the library held it.
pub fn materialized(shared_extents: bool) -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, TOTAL_BLOCKS);
    let zeros = [0u8; BLOCK_SIZE];
    for lba in 0..TOTAL_BLOCKS {
        device.apply_raw(lba, &zeros);
    }
    format(device, shared_extents)
}

fn format(mut device: MemoryBackend, shared_extents: bool) -> MemoryBackend {
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC6; 16],
            label: "FfiCommon".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents,
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

pub fn mount(device: &mut MemoryBackend) -> *mut AfsplusAros {
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
