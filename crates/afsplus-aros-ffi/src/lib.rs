//! Stable C boundary between a native AROS handler and the safe AFS+ adapter.
//!
//! This crate deliberately owns every raw pointer and callback invocation.
//! The filesystem engine, VFS and AROS semantics remain safe Rust. A native
//! handler supplies a block device through callbacks and keeps the returned
//! instance on its single packet-processing task.

use std::ffi::c_void;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;

use afsplus_aros::{
    ArosAdapter, ArosConfig, ArosError, DiskInfo, FileInfo, LockAccess, NameEncoding, OpenMode,
    SeekMode,
};
use afsplus_block::{BlockDevice, BlockError};
use afsplus_core::{MountMode, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

pub const AFSPLUS_AROS_ABI_VERSION: u32 = 1;
pub const AFSPLUS_AROS_MOUNT_READ_WRITE: u32 = 0;
pub const AFSPLUS_AROS_MOUNT_READ_ONLY: u32 = 1;
pub const AFSPLUS_AROS_MOUNT_NO_CHANGES: u32 = 2;
pub const AFSPLUS_AROS_MOUNT_RECOVERY: u32 = 3;
pub const AFSPLUS_AROS_ENCODING_UTF8: u32 = 0;
pub const AFSPLUS_AROS_ENCODING_LATIN1: u32 = 1;
pub const AFSPLUS_AROS_LOCK_SHARED: u32 = 0;
pub const AFSPLUS_AROS_LOCK_EXCLUSIVE: u32 = 1;
pub const AFSPLUS_AROS_OPEN_OLD_FILE: u32 = 0;
pub const AFSPLUS_AROS_OPEN_NEW_FILE: u32 = 1;
pub const AFSPLUS_AROS_OPEN_READ_WRITE: u32 = 2;
pub const AFSPLUS_AROS_SEEK_BEGINNING: u32 = 0;
pub const AFSPLUS_AROS_SEEK_CURRENT: u32 = 1;
pub const AFSPLUS_AROS_SEEK_END: u32 = 2;

pub type AfsplusArosReadBlock =
    unsafe extern "C" fn(context: *mut c_void, lba: u64, destination: *mut u8, length: u32) -> i32;
pub type AfsplusArosWriteBlock =
    unsafe extern "C" fn(context: *mut c_void, lba: u64, source: *const u8, length: u32) -> i32;
pub type AfsplusArosFlush = unsafe extern "C" fn(context: *mut c_void) -> i32;

#[repr(C)]
pub struct AfsplusArosDevice {
    pub abi_version: u32,
    pub struct_size: u32,
    pub context: *mut c_void,
    pub block_size: u32,
    pub reserved: u32,
    pub total_blocks: u64,
    pub read_block: Option<AfsplusArosReadBlock>,
    pub write_block: Option<AfsplusArosWriteBlock>,
    pub flush: Option<AfsplusArosFlush>,
}

#[repr(C)]
pub struct AfsplusArosMountConfig {
    pub abi_version: u32,
    pub struct_size: u32,
    pub mount_mode: u32,
    pub name_encoding: u32,
    pub volume_name: *const u8,
    pub volume_name_length: u32,
    pub max_file_handles: u32,
    pub max_locks: u32,
    pub max_file_info_name_bytes: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosFileInfo {
    pub disk_key: u64,
    pub size: u64,
    pub blocks: u64,
    pub modified_seconds: i64,
    pub object_id: u64,
    pub directory_entry_type: i32,
    pub entry_type: i32,
    pub protection: u32,
    pub modified_nanoseconds: u32,
    pub name_length: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosDiskInfo {
    pub total_blocks: u64,
    pub used_blocks: u64,
    pub bytes_per_block: u32,
    pub disk_type: i32,
    pub write_protected: u32,
    pub in_use: u32,
}

const _: [(); 64] = [(); std::mem::size_of::<AfsplusArosFileInfo>()];
const _: [(); 32] = [(); std::mem::size_of::<AfsplusArosDiskInfo>()];
#[cfg(target_pointer_width = "64")]
const _: [(); 56] = [(); std::mem::size_of::<AfsplusArosDevice>()];
#[cfg(target_pointer_width = "64")]
const _: [(); 48] = [(); std::mem::size_of::<AfsplusArosMountConfig>()];
#[cfg(target_pointer_width = "32")]
const _: [(); 40] = [(); std::mem::size_of::<AfsplusArosDevice>()];
#[cfg(target_pointer_width = "32")]
const _: [(); 40] = [(); std::mem::size_of::<AfsplusArosMountConfig>()];

/// Opaque to C. The actual allocation is `NativeBridge`.
#[repr(C)]
pub struct AfsplusAros {
    _private: [u8; 0],
}

struct CallbackDevice {
    context: *mut c_void,
    block_size: usize,
    total_blocks: u64,
    read_block: AfsplusArosReadBlock,
    write_block: Option<AfsplusArosWriteBlock>,
    flush: Option<AfsplusArosFlush>,
}

impl BlockDevice for CallbackDevice {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    fn read_block(&mut self, lba: u64, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.validate_access(lba, buffer.len())?;
        let length = u32::try_from(buffer.len()).map_err(|_| callback_error(-1))?;
        // SAFETY: the caller guarantees that the callback and context remain
        // valid until `afsplus_aros_unmount` returns. `buffer` is writable for
        // exactly `length` bytes for the duration of this call.
        let status = unsafe { (self.read_block)(self.context, lba, buffer.as_mut_ptr(), length) };
        callback_status(status)
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.validate_access(lba, data.len())?;
        let write = self.write_block.ok_or_else(|| callback_error(-1))?;
        let length = u32::try_from(data.len()).map_err(|_| callback_error(-1))?;
        // SAFETY: same lifetime contract as `read_block`; `data` is readable
        // for exactly `length` bytes and is not retained by the callback.
        let status = unsafe { write(self.context, lba, data.as_ptr(), length) };
        callback_status(status)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let flush = self.flush.ok_or_else(|| callback_error(-1))?;
        // SAFETY: the callback/context lifetime is the mount lifetime.
        callback_status(unsafe { flush(self.context) })
    }
}

impl CallbackDevice {
    fn validate_access(&self, lba: u64, length: usize) -> Result<(), BlockError> {
        if lba >= self.total_blocks {
            return Err(BlockError::OutOfBounds {
                lba,
                total_blocks: self.total_blocks,
            });
        }
        if length != self.block_size {
            return Err(BlockError::WrongBufferSize {
                expected: self.block_size,
                actual: length,
            });
        }
        Ok(())
    }
}

struct NativeBridge {
    adapter: ArosAdapter<CallbackDevice>,
    max_file_info_name_bytes: usize,
}

fn callback_status(status: i32) -> Result<(), BlockError> {
    if status == 0 {
        Ok(())
    } else {
        Err(callback_error(status))
    }
}

fn callback_error(status: i32) -> BlockError {
    BlockError::Io(io::Error::from_raw_os_error(status))
}

fn ffi_status<F>(operation: F) -> i32
where
    F: FnOnce() -> Result<(), ArosError>,
{
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.io_error(),
        Err(_) => ArosError::Unknown.io_error(),
    }
}

fn mount_mode(value: u32) -> Result<MountMode, ArosError> {
    match value {
        AFSPLUS_AROS_MOUNT_READ_WRITE => Ok(MountMode::ReadWrite),
        AFSPLUS_AROS_MOUNT_READ_ONLY => Ok(MountMode::ReadOnly),
        AFSPLUS_AROS_MOUNT_NO_CHANGES => Ok(MountMode::NoChanges),
        AFSPLUS_AROS_MOUNT_RECOVERY => Ok(MountMode::Recovery),
        _ => Err(ArosError::InvalidComponentName),
    }
}

fn name_encoding(value: u32) -> Result<NameEncoding, ArosError> {
    match value {
        AFSPLUS_AROS_ENCODING_UTF8 => Ok(NameEncoding::Utf8),
        AFSPLUS_AROS_ENCODING_LATIN1 => Ok(NameEncoding::Latin1),
        _ => Err(ArosError::InvalidComponentName),
    }
}

fn lock_access(value: u32) -> Result<LockAccess, ArosError> {
    match value {
        AFSPLUS_AROS_LOCK_SHARED => Ok(LockAccess::Shared),
        AFSPLUS_AROS_LOCK_EXCLUSIVE => Ok(LockAccess::Exclusive),
        _ => Err(ArosError::InvalidComponentName),
    }
}

fn open_mode(value: u32) -> Result<OpenMode, ArosError> {
    match value {
        AFSPLUS_AROS_OPEN_OLD_FILE => Ok(OpenMode::OldFile),
        AFSPLUS_AROS_OPEN_NEW_FILE => Ok(OpenMode::NewFile),
        AFSPLUS_AROS_OPEN_READ_WRITE => Ok(OpenMode::ReadWrite),
        _ => Err(ArosError::InvalidComponentName),
    }
}

fn seek_mode(value: u32) -> Result<SeekMode, ArosError> {
    match value {
        AFSPLUS_AROS_SEEK_BEGINNING => Ok(SeekMode::Beginning),
        AFSPLUS_AROS_SEEK_CURRENT => Ok(SeekMode::Current),
        AFSPLUS_AROS_SEEK_END => Ok(SeekMode::End),
        _ => Err(ArosError::SeekError),
    }
}

fn timestamp(seconds: i64, nanoseconds: u32) -> Result<Timespec, ArosError> {
    if nanoseconds >= 1_000_000_000 {
        return Err(ArosError::InvalidComponentName);
    }
    Ok(Timespec {
        seconds,
        nanoseconds,
    })
}

fn optional_lock(lock: u64) -> Option<u64> {
    (lock != 0).then_some(lock)
}

fn bridge_mut<'a>(filesystem: *mut AfsplusAros) -> Result<&'a mut NativeBridge, ArosError> {
    // SAFETY: every exported operation requires the live pointer returned by
    // `afsplus_aros_mount`, on the owning handler task, exactly once at a time.
    unsafe { filesystem.cast::<NativeBridge>().as_mut() }.ok_or(ArosError::InvalidLock)
}

fn input_bytes<'a>(bytes: *const u8, length: u32) -> Result<&'a [u8], ArosError> {
    if length == 0 {
        return Ok(&[]);
    }
    if bytes.is_null() {
        return Err(ArosError::InvalidComponentName);
    }
    // SAFETY: the C caller promises a readable range for this call. The slice
    // is never retained by the adapter.
    Ok(unsafe { slice::from_raw_parts(bytes, length as usize) })
}

fn output_slice<'a>(bytes: *mut u8, length: u32) -> Result<&'a mut [u8], ArosError> {
    if length == 0 {
        return Ok(&mut []);
    }
    if bytes.is_null() {
        return Err(ArosError::InvalidComponentName);
    }
    // SAFETY: the C caller promises an exclusive writable range for this call.
    Ok(unsafe { slice::from_raw_parts_mut(bytes, length as usize) })
}

fn require_output<T>(output: *mut T) -> Result<(), ArosError> {
    if output.is_null() {
        Err(ArosError::InvalidComponentName)
    } else {
        Ok(())
    }
}

fn write_output<T>(output: *mut T, value: T) -> Result<(), ArosError> {
    require_output(output)?;
    // SAFETY: the C caller provides an aligned, writable `T` output slot.
    unsafe { ptr::write(output, value) };
    Ok(())
}

fn copy_file_info(
    bridge: &NativeBridge,
    info: FileInfo,
    output: *mut AfsplusArosFileInfo,
    name: *mut u8,
    name_capacity: u32,
) -> Result<(), ArosError> {
    if (name_capacity as usize) < bridge.max_file_info_name_bytes {
        return Err(ArosError::ObjectTooLarge);
    }
    let destination = output_slice(name, name_capacity)?;
    if info.name.len() > destination.len() {
        return Err(ArosError::ObjectTooLarge);
    }
    destination[..info.name.len()].copy_from_slice(&info.name);
    write_output(
        output,
        AfsplusArosFileInfo {
            disk_key: info.disk_key,
            size: info.size,
            blocks: info.blocks,
            modified_seconds: info.modified.seconds,
            object_id: info.object_id,
            directory_entry_type: info.directory_entry_type as i32,
            entry_type: info.entry_type as i32,
            protection: info.protection,
            modified_nanoseconds: info.modified.nanoseconds,
            name_length: info.name.len() as u32,
            reserved: 0,
        },
    )
}

fn validate_file_info_output(
    bridge: &NativeBridge,
    output: *mut AfsplusArosFileInfo,
    name: *mut u8,
    name_capacity: u32,
) -> Result<(), ArosError> {
    require_output(output)?;
    if (name_capacity as usize) < bridge.max_file_info_name_bytes {
        return Err(ArosError::ObjectTooLarge);
    }
    let _ = output_slice(name, name_capacity)?;
    Ok(())
}

fn copy_disk_info(info: DiskInfo, output: *mut AfsplusArosDiskInfo) -> Result<(), ArosError> {
    write_output(
        output,
        AfsplusArosDiskInfo {
            total_blocks: info.total_blocks,
            used_blocks: info.used_blocks,
            bytes_per_block: info.bytes_per_block,
            disk_type: info.disk_type,
            write_protected: u32::from(info.write_protected),
            in_use: u32::from(info.in_use),
        },
    )
}

#[no_mangle]
// C entry points keep an ordinary C signature. Raw-pointer validation and the
// unavoidable dereferences are centralized here instead of making Rust users
// pretend an `unsafe extern` qualifier can protect foreign callers.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn afsplus_aros_mount(
    device: *const AfsplusArosDevice,
    config: *const AfsplusArosMountConfig,
    output: *mut *mut AfsplusAros,
) -> i32 {
    ffi_status(|| {
        if device.is_null() || config.is_null() || output.is_null() {
            return Err(ArosError::InvalidComponentName);
        }
        // SAFETY: non-null pointers above must reference complete ABI structs.
        let device = unsafe { &*device };
        // SAFETY: same contract as `device`.
        let config = unsafe { &*config };
        if device.abi_version != AFSPLUS_AROS_ABI_VERSION
            || config.abi_version != AFSPLUS_AROS_ABI_VERSION
            || device.struct_size as usize != std::mem::size_of::<AfsplusArosDevice>()
            || config.struct_size as usize != std::mem::size_of::<AfsplusArosMountConfig>()
            || device.block_size == 0
            || device.total_blocks == 0
            || device.reserved != 0
            || config.reserved != 0
        {
            return Err(ArosError::InvalidComponentName);
        }
        let mode = mount_mode(config.mount_mode)?;
        let read_block = device.read_block.ok_or(ArosError::NotDosDisk)?;
        if matches!(mode, MountMode::ReadWrite | MountMode::Recovery)
            && (device.write_block.is_none() || device.flush.is_none())
        {
            return Err(ArosError::DiskWriteProtected);
        }
        let volume_name = if config.volume_name_length == 0 {
            b"AFS+".to_vec()
        } else {
            input_bytes(config.volume_name, config.volume_name_length)?.to_vec()
        };
        let max_file_handles =
            usize::try_from(config.max_file_handles).map_err(|_| ArosError::ObjectTooLarge)?;
        let max_locks = usize::try_from(config.max_locks).map_err(|_| ArosError::ObjectTooLarge)?;
        let max_file_info_name_bytes = usize::try_from(config.max_file_info_name_bytes)
            .map_err(|_| ArosError::ObjectTooLarge)?;
        if max_file_handles == 0 || max_locks == 0 || max_file_info_name_bytes == 0 {
            return Err(ArosError::InvalidComponentName);
        }
        let callback_device = CallbackDevice {
            context: device.context,
            block_size: device.block_size as usize,
            total_blocks: device.total_blocks,
            read_block,
            write_block: device.write_block,
            flush: device.flush,
        };
        let vfs = Vfs::mount(callback_device, MountOptions { mode })?;
        let adapter = ArosAdapter::new(
            vfs,
            ArosConfig {
                name_encoding: name_encoding(config.name_encoding)?,
                volume_name,
                max_file_handles,
                max_locks,
                max_file_info_name_bytes,
            },
        );
        let raw = Box::into_raw(Box::new(NativeBridge {
            adapter,
            max_file_info_name_bytes,
        }))
        .cast::<AfsplusAros>();
        write_output(output, raw)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_unmount(filesystem: *mut AfsplusAros) -> i32 {
    ffi_status(|| {
        if filesystem.is_null() {
            return Err(ArosError::InvalidLock);
        }
        // SAFETY: this consumes exactly once the allocation returned by mount.
        let mut bridge = unsafe { Box::from_raw(filesystem.cast::<NativeBridge>()) };
        bridge.adapter.flush()?;
        let _ = bridge.adapter.into_vfs()?;
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_locate(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    access: u32,
    output_lock: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_lock)?;
        let name = input_bytes(name, name_length)?;
        let lock = bridge_mut(filesystem)?.adapter.locate(
            optional_lock(base_lock),
            name,
            lock_access(access)?,
        )?;
        write_output(output_lock, lock)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_duplicate_lock(
    filesystem: *mut AfsplusAros,
    lock: u64,
    output_lock: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_lock)?;
        let copy = bridge_mut(filesystem)?.adapter.duplicate_lock(lock)?;
        write_output(output_lock, copy)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_parent_lock(
    filesystem: *mut AfsplusAros,
    lock: u64,
    output_lock: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_lock)?;
        let parent = bridge_mut(filesystem)?.adapter.parent_lock(lock)?;
        write_output(output_lock, parent.unwrap_or(0))
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_same_lock(
    filesystem: *mut AfsplusAros,
    first_lock: u64,
    second_lock: u64,
    output_same: *mut u32,
) -> i32 {
    ffi_status(|| {
        require_output(output_same)?;
        let same = bridge_mut(filesystem)?
            .adapter
            .same_lock(optional_lock(first_lock), optional_lock(second_lock))?;
        write_output(output_same, u32::from(same))
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_free_lock(filesystem: *mut AfsplusAros, lock: u64) -> i32 {
    ffi_status(|| bridge_mut(filesystem)?.adapter.free_lock(lock))
}

#[no_mangle]
pub extern "C" fn afsplus_aros_open(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    mode: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
    output_file: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_file)?;
        let name = input_bytes(name, name_length)?;
        let file = bridge_mut(filesystem)?.adapter.open(
            optional_lock(base_lock),
            name,
            open_mode(mode)?,
            timestamp(now_seconds, now_nanoseconds)?,
        )?;
        write_output(output_file, file)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_parent_of_file(
    filesystem: *mut AfsplusAros,
    file: u64,
    output_lock: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_lock)?;
        let lock = bridge_mut(filesystem)?.adapter.parent_of_file(file)?;
        write_output(output_lock, lock)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_close(filesystem: *mut AfsplusAros, file: u64) -> i32 {
    ffi_status(|| bridge_mut(filesystem)?.adapter.close(file))
}

#[no_mangle]
pub extern "C" fn afsplus_aros_read(
    filesystem: *mut AfsplusAros,
    file: u64,
    destination: *mut u8,
    length: u32,
    output_count: *mut u32,
) -> i32 {
    ffi_status(|| {
        require_output(output_count)?;
        let destination = output_slice(destination, length)?;
        let count = bridge_mut(filesystem)?.adapter.read(file, destination)?;
        let count = u32::try_from(count).map_err(|_| ArosError::ObjectTooLarge)?;
        write_output(output_count, count)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_write(
    filesystem: *mut AfsplusAros,
    file: u64,
    source: *const u8,
    length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
    output_count: *mut u32,
) -> i32 {
    ffi_status(|| {
        require_output(output_count)?;
        let source = input_bytes(source, length)?;
        let count = bridge_mut(filesystem)?.adapter.write(
            file,
            source,
            timestamp(now_seconds, now_nanoseconds)?,
        )?;
        let count = u32::try_from(count).map_err(|_| ArosError::ObjectTooLarge)?;
        write_output(output_count, count)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_seek(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: i64,
    mode: u32,
    output_old_position: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_old_position)?;
        let old = bridge_mut(filesystem)?
            .adapter
            .seek(file, offset, seek_mode(mode)?)?;
        write_output(output_old_position, old)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_file_position(
    filesystem: *mut AfsplusAros,
    file: u64,
    output_position: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_position)?;
        let position = bridge_mut(filesystem)?.adapter.file_position(file)?;
        write_output(output_position, position)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_file_size(
    filesystem: *mut AfsplusAros,
    file: u64,
    output_size: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_size)?;
        let size = bridge_mut(filesystem)?.adapter.file_size(file)?;
        write_output(output_size, size)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_set_file_size(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: i64,
    mode: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
    output_size: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_size)?;
        let size = bridge_mut(filesystem)?.adapter.set_file_size(
            file,
            offset,
            seek_mode(mode)?,
            timestamp(now_seconds, now_nanoseconds)?,
        )?;
        write_output(output_size, size)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_fsync(filesystem: *mut AfsplusAros, file: u64) -> i32 {
    ffi_status(|| bridge_mut(filesystem)?.adapter.fsync(file))
}

#[no_mangle]
pub extern "C" fn afsplus_aros_flush(filesystem: *mut AfsplusAros) -> i32 {
    ffi_status(|| bridge_mut(filesystem)?.adapter.flush())
}

#[no_mangle]
pub extern "C" fn afsplus_aros_create_directory(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
    output_lock: *mut u64,
) -> i32 {
    ffi_status(|| {
        require_output(output_lock)?;
        let name = input_bytes(name, name_length)?;
        let lock = bridge_mut(filesystem)?.adapter.create_directory(
            optional_lock(base_lock),
            name,
            timestamp(now_seconds, now_nanoseconds)?,
        )?;
        write_output(output_lock, lock)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_delete_object(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    ffi_status(|| {
        let name = input_bytes(name, name_length)?;
        bridge_mut(filesystem)?.adapter.delete_object(
            optional_lock(base_lock),
            name,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_rename(
    filesystem: *mut AfsplusAros,
    source_base_lock: u64,
    source_name: *const u8,
    source_name_length: u32,
    target_base_lock: u64,
    target_name: *const u8,
    target_name_length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    ffi_status(|| {
        let source_name = input_bytes(source_name, source_name_length)?;
        let target_name = input_bytes(target_name, target_name_length)?;
        bridge_mut(filesystem)?.adapter.rename(
            optional_lock(source_base_lock),
            source_name,
            optional_lock(target_base_lock),
            target_name,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_make_hard_link(
    filesystem: *mut AfsplusAros,
    target_base_lock: u64,
    target_name: *const u8,
    target_name_length: u32,
    source_lock: u64,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    ffi_status(|| {
        let target_name = input_bytes(target_name, target_name_length)?;
        bridge_mut(filesystem)?.adapter.make_hard_link(
            optional_lock(target_base_lock),
            target_name,
            source_lock,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_examine_lock(
    filesystem: *mut AfsplusAros,
    lock: u64,
    output: *mut AfsplusArosFileInfo,
    name: *mut u8,
    name_capacity: u32,
) -> i32 {
    ffi_status(|| {
        let bridge = bridge_mut(filesystem)?;
        validate_file_info_output(bridge, output, name, name_capacity)?;
        let info = bridge.adapter.examine_lock(lock)?;
        copy_file_info(bridge, info, output, name, name_capacity)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_examine_file(
    filesystem: *mut AfsplusAros,
    file: u64,
    output: *mut AfsplusArosFileInfo,
    name: *mut u8,
    name_capacity: u32,
) -> i32 {
    ffi_status(|| {
        let bridge = bridge_mut(filesystem)?;
        validate_file_info_output(bridge, output, name, name_capacity)?;
        let info = bridge.adapter.examine_file(file)?;
        copy_file_info(bridge, info, output, name, name_capacity)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_examine_next(
    filesystem: *mut AfsplusAros,
    lock: u64,
    output: *mut AfsplusArosFileInfo,
    name: *mut u8,
    name_capacity: u32,
) -> i32 {
    ffi_status(|| {
        let bridge = bridge_mut(filesystem)?;
        validate_file_info_output(bridge, output, name, name_capacity)?;
        let info = bridge.adapter.examine_next(lock)?;
        copy_file_info(bridge, info, output, name, name_capacity)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_rewind_directory(filesystem: *mut AfsplusAros, lock: u64) -> i32 {
    ffi_status(|| bridge_mut(filesystem)?.adapter.rewind_directory(lock))
}

#[no_mangle]
pub extern "C" fn afsplus_aros_disk_info(
    filesystem: *mut AfsplusAros,
    output: *mut AfsplusArosDiskInfo,
) -> i32 {
    ffi_status(|| {
        require_output(output)?;
        copy_disk_info(bridge_mut(filesystem)?.adapter.disk_info(), output)
    })
}
