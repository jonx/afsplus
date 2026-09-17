//! Interface-revision and capability query of the AROS C boundary.

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

fn formatted(
    log_slots: u16,
    shared_extents: bool,
    data_policy: bool,
    name_policy: NamePolicy,
) -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, TOTAL_BLOCKS);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC1; 16],
            label: "Boundary".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents,
            data_policy,
            name_policy,
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

fn mount(device: &mut MemoryBackend, mount_mode: u32) -> *mut AfsplusAros {
    let mut filesystem = ptr::null_mut();
    assert_eq!(mount_with_flags(device, mount_mode, 0, &mut filesystem), 0);
    filesystem
}

fn mount_with_flags(
    device: &mut MemoryBackend,
    mount_mode: u32,
    flags: u32,
    filesystem: &mut *mut AfsplusAros,
) -> i32 {
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
        mount_mode,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: b"AFS+".as_ptr(),
        volume_name_length: 4,
        max_file_handles: 8,
        max_locks: 8,
        max_file_info_name_bytes: 107,
        flags,
    };
    afsplus_aros_mount(&callbacks, &config, filesystem)
}

#[test]
fn mount_flags_admit_only_the_two_defined_security_bits() {
    let mut device = formatted(8, true, false, NamePolicy::Insensitive);
    assert_eq!(AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE, 1);
    assert_eq!(AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION, 2);
    for flags in [0, 1, 2, 3] {
        let mut filesystem = ptr::null_mut();
        assert_eq!(mount_with_flags(&mut device, 0, flags, &mut filesystem), 0);
        assert_eq!(afsplus_aros_unmount(filesystem), 0);
    }
    // Undefined bits are refused before any device access.
    let mut filesystem = ptr::null_mut();
    assert_eq!(mount_with_flags(&mut device, 0, 4, &mut filesystem), 210);
    assert!(filesystem.is_null());
    assert_eq!(mount_with_flags(&mut device, 0, 5, &mut filesystem), 210);
    assert!(filesystem.is_null());
}

fn query(filesystem: *mut AfsplusAros) -> AfsplusArosCapabilities {
    let mut output = AfsplusArosCapabilities {
        struct_size: size_of::<AfsplusArosCapabilities>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_capabilities(filesystem, &mut output), 0);
    output
}

#[test]
fn interface_query_needs_no_mount_and_names_its_revision_and_groups() {
    let mut output = AfsplusArosInterface {
        struct_size: 24,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_interface(&mut output), 0);
    assert_eq!(
        output,
        AfsplusArosInterface {
            struct_size: 24,
            abi_version: 1,
            interface_revision: 13,
            reserved: 0,
            groups: 0x3FFF,
        }
    );
    assert_eq!(afsplus_aros_interface(ptr::null_mut()), 210);
}

#[test]
fn sized_output_refuses_short_callers_and_never_writes_past_a_long_one() {
    // A caller below the first published layout is refused and untouched.
    let mut short = AfsplusArosInterface {
        struct_size: 23,
        abi_version: 0xAAAA_AAAA,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_interface(&mut short), 115);
    assert_eq!(short.struct_size, 23);
    assert_eq!(short.abi_version, 0xAAAA_AAAA);

    // A caller built against a longer future layout keeps its tail bytes and
    // learns how many bytes this library filled.
    #[repr(C)]
    struct Future {
        known: AfsplusArosInterface,
        tail: [u8; 16],
    }
    let mut future = Future {
        known: AfsplusArosInterface {
            struct_size: 40,
            ..Default::default()
        },
        tail: [0x5A; 16],
    };
    assert_eq!(
        afsplus_aros_interface(ptr::from_mut(&mut future).cast::<AfsplusArosInterface>()),
        0
    );
    assert_eq!(future.known.struct_size, 24);
    assert_eq!(future.known.interface_revision, 13);
    assert_eq!(future.tail, [0x5A; 16]);
}

#[test]
fn capabilities_use_published_c_identities_and_follow_the_volume() {
    // Logged, reflink-capable, case-insensitive volume.
    let mut device = formatted(8, true, false, NamePolicy::Insensitive);
    let filesystem = mount(&mut device, AFSPLUS_AROS_MOUNT_READ_WRITE);
    let full = query(filesystem);
    assert_eq!(full.struct_size, 64);
    assert_eq!(full.mount_mode, 0);
    // Bits 0,1,2,3,5,6,9,10,11,12,13,15,16,17 of api/filesystem_v2.h.
    assert_eq!(full.capabilities, 0x3_BE6F);
    assert_eq!(full.block_size, 4096);
    assert_eq!(full.max_name_bytes, 255);
    assert_eq!(full.case_sensitive, 0);
    assert_eq!(
        (
            full.unicode_version_major,
            full.unicode_version_minor,
            full.unicode_version_patch
        ),
        (16, 0, 0)
    );
    assert_eq!(full.pending_intent_records, 0);
    assert_eq!(full.total_blocks, 8192);
    assert!(full.available_blocks <= full.free_blocks);
    assert!(full.free_blocks < full.total_blocks);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);

    // Control volume: no log, no shared extents, data policy, case-sensitive,
    // mounted read-only. A constant answer cannot satisfy both volumes: clone
    // and logged-fsync bits 11,12,13 disappear and bit 14 appears.
    let mut device = formatted(0, false, true, NamePolicy::Sensitive);
    let filesystem = mount(&mut device, AFSPLUS_AROS_MOUNT_READ_ONLY);
    let plain = query(filesystem);
    assert_eq!(plain.capabilities, 0x3_C66F);
    assert_eq!(plain.capabilities & 0x3800, 0);
    assert_eq!(plain.mount_mode, 1);
    assert_eq!(plain.case_sensitive, 1);
    assert_ne!(plain.capabilities, full.capabilities);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);

    // Null filesystem and short caller are refused by value.
    let mut output = AfsplusArosCapabilities {
        struct_size: 64,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_capabilities(ptr::null_mut(), &mut output), 211);
    let mut device = formatted(8, true, false, NamePolicy::Insensitive);
    let filesystem = mount(&mut device, AFSPLUS_AROS_MOUNT_READ_WRITE);
    output.struct_size = 63;
    assert_eq!(afsplus_aros_capabilities(filesystem, &mut output), 115);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
