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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use afsplus_aros::health::HealthEvent;
use afsplus_aros::{
    ArosAdapter, ArosConfig, ArosError, DiskInfo, FileInfo, LockAccess, NameEncoding, OpenMode,
    SeekMode,
};
use afsplus_block::{BlockDevice, BlockError};
use afsplus_core::flight::{Categories, Category, Event, FlightRecorder, LiveSink, SinkResult};
use afsplus_core::{MountMode, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::{Capabilities, Vfs};

pub const AFSPLUS_AROS_ABI_VERSION: u32 = 1;
pub const AFSPLUS_AROS_INTERFACE_REVISION: u32 = 14;
pub const AFSPLUS_AROS_GROUP_BASE: u64 = 0x1;
pub const AFSPLUS_AROS_GROUP_INTERFACE_QUERY: u64 = 0x2;
pub const AFSPLUS_AROS_GROUP_DOS_METADATA: u64 = 0x4;
pub const AFSPLUS_AROS_GROUP_SOFT_LINKS: u64 = 0x8;
pub const AFSPLUS_AROS_GROUP_API_V2: u64 = 0x10;
pub const AFSPLUS_AROS_GROUP_NOTIFY: u64 = 0x20;
pub const AFSPLUS_AROS_GROUP_OBSERVE: u64 = 0x40;
pub const AFSPLUS_AROS_GROUP_MANAGE: u64 = 0x80;
pub const AFSPLUS_AROS_GROUP_COUNTERS: u64 = 0x100;
pub const AFSPLUS_AROS_GROUP_DOS_HANDLES: u64 = 0x200;
pub const AFSPLUS_AROS_GROUP_DOS_RECORDS: u64 = 0x400;
pub const AFSPLUS_AROS_GROUP_OBJECT_IDS: u64 = 0x800;
pub const AFSPLUS_AROS_GROUP_EXTENT_MAP: u64 = 0x1000;
pub const AFSPLUS_AROS_GROUP_VOLUME_LABEL: u64 = 0x2000;
pub const AFSPLUS_AROS_GROUP_DOS_COMMENT: u64 = 0x4000;
pub const AFSPLUS_AROS_EXTENT_UNWRITTEN: u32 = 1;
pub const AFSPLUS_AROS_DIR_RECORD_MAX: u32 = 280;
pub const AFSPLUS_AROS_KIND_FILE: u32 = 1;
pub const AFSPLUS_AROS_KIND_DIRECTORY: u32 = 2;
pub const AFSPLUS_AROS_KIND_SYMLINK: u32 = 3;
pub const AFSPLUS_AROS_HEALTH_DEVICE_ERROR: u32 = afsplus_aros::health::HEALTH_DEVICE_ERROR;
pub const AFSPLUS_AROS_HEALTH_CORRUPTION: u32 = afsplus_aros::health::HEALTH_CORRUPTION;
pub const AFSPLUS_AROS_HEALTH_REPLAY_PENDING: u32 = afsplus_aros::health::HEALTH_REPLAY_PENDING;
pub const AFSPLUS_AROS_HEALTH_INTERNAL_FAULT: u32 = afsplus_aros::health::HEALTH_INTERNAL_FAULT;
pub const AFSPLUS_AROS_ADVICE_NO_EFFECT: u32 = 0;
const AFSPLUS_AROS_GROUPS: u64 = AFSPLUS_AROS_GROUP_BASE
    | AFSPLUS_AROS_GROUP_INTERFACE_QUERY
    | AFSPLUS_AROS_GROUP_DOS_METADATA
    | AFSPLUS_AROS_GROUP_SOFT_LINKS
    | AFSPLUS_AROS_GROUP_API_V2
    | AFSPLUS_AROS_GROUP_NOTIFY
    | AFSPLUS_AROS_GROUP_OBSERVE
    | AFSPLUS_AROS_GROUP_MANAGE
    | AFSPLUS_AROS_GROUP_COUNTERS
    | AFSPLUS_AROS_GROUP_DOS_HANDLES
    | AFSPLUS_AROS_GROUP_DOS_RECORDS
    | AFSPLUS_AROS_GROUP_OBJECT_IDS
    | AFSPLUS_AROS_GROUP_EXTENT_MAP
    | AFSPLUS_AROS_GROUP_VOLUME_LABEL
    | AFSPLUS_AROS_GROUP_DOS_COMMENT;

// Published C capability identities of `api/filesystem_v2.h`. They are
// independent of the Rust mask and never renumbered.
pub const FSV2_CAP_64BIT_IO: u64 = 1 << 0;
pub const FSV2_CAP_UTF8_NAMES: u64 = 1 << 1;
pub const FSV2_CAP_SYMLINKS: u64 = 1 << 2;
pub const FSV2_CAP_HARDLINKS: u64 = 1 << 3;
pub const FSV2_CAP_XATTRS: u64 = 1 << 4;
pub const FSV2_CAP_ATOMIC_REPLACE: u64 = 1 << 5;
pub const FSV2_CAP_OBJECT_IDS: u64 = 1 << 6;
pub const FSV2_CAP_FAST_ENUMERATION: u64 = 1 << 7;
pub const FSV2_CAP_CHANGE_STREAM: u64 = 1 << 8;
pub const FSV2_CAP_SPARSE: u64 = 1 << 9;
pub const FSV2_CAP_FSYNC: u64 = 1 << 10;
pub const FSV2_CAP_CLONE_FILE: u64 = 1 << 11;
pub const FSV2_CAP_CLONE_RANGE: u64 = 1 << 12;
pub const FSV2_CAP_LOGGED_DATA_FSYNC: u64 = 1 << 13;
pub const FSV2_CAP_DATA_POLICY: u64 = 1 << 14;
pub const FSV2_CAP_OPEN_UNLINKED: u64 = 1 << 15;
pub const FSV2_CAP_PAGED_DIRECTORIES: u64 = 1 << 16;
pub const FSV2_CAP_PREALLOCATE: u64 = 1 << 17;

/// Rust capability bit to published C identity. A Rust bit without a row is
/// not advertised at the C boundary.
const CAPABILITY_MAP: [(u64, u64); 15] = [
    (Capabilities::PREALLOCATE, FSV2_CAP_PREALLOCATE),
    (Capabilities::IO_64BIT, FSV2_CAP_64BIT_IO),
    (Capabilities::UTF8_NAMES, FSV2_CAP_UTF8_NAMES),
    (Capabilities::HARD_LINKS, FSV2_CAP_HARDLINKS),
    (Capabilities::ATOMIC_REPLACE, FSV2_CAP_ATOMIC_REPLACE),
    (Capabilities::OBJECT_IDS, FSV2_CAP_OBJECT_IDS),
    (Capabilities::PAGED_DIRECTORIES, FSV2_CAP_PAGED_DIRECTORIES),
    (Capabilities::SPARSE_FILES, FSV2_CAP_SPARSE),
    (Capabilities::FSYNC, FSV2_CAP_FSYNC),
    (Capabilities::CLONE_FILE, FSV2_CAP_CLONE_FILE),
    (Capabilities::CLONE_RANGE, FSV2_CAP_CLONE_RANGE),
    (Capabilities::LOGGED_DATA_FSYNC, FSV2_CAP_LOGGED_DATA_FSYNC),
    (Capabilities::DATA_POLICY, FSV2_CAP_DATA_POLICY),
    (Capabilities::OPEN_UNLINKED, FSV2_CAP_OPEN_UNLINKED),
    (Capabilities::SYMLINKS, FSV2_CAP_SYMLINKS),
];

fn published_capabilities(capabilities: Capabilities) -> u64 {
    CAPABILITY_MAP
        .iter()
        .filter(|(rust, _)| capabilities.contains(*rust))
        .fold(0, |bits, (_, published)| bits | published)
}
pub const AFSPLUS_AROS_MOUNT_READ_WRITE: u32 = 0;
pub const AFSPLUS_AROS_MOUNT_READ_ONLY: u32 = 1;
pub const AFSPLUS_AROS_MOUNT_NO_CHANGES: u32 = 2;
pub const AFSPLUS_AROS_MOUNT_RECOVERY: u32 = 3;
/// `AfsplusArosMountConfig::flags`: the explicit request to let a classic
/// protection write replace security metadata the container does not hold,
/// where preserving is impossible. A zero field keeps the refusal.
pub const AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE: u32 = 1;
/// `AfsplusArosMountConfig::flags`: refuse a classic protection write on an
/// object that carries an on-disk security descriptor. A zero field takes the
/// handler default, which applies the write, keeps every descriptor byte and
/// marks the projection as diverged.
pub const AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION: u32 = 2;

/// Every defined bit of `AfsplusArosMountConfig::flags`.
const MOUNT_FLAGS: u32 =
    AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE | AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION;
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
    pub flags: u32,
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosInterface {
    pub struct_size: u32,
    pub abi_version: u32,
    pub interface_revision: u32,
    pub reserved: u32,
    pub groups: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosCapabilities {
    pub struct_size: u32,
    pub mount_mode: u32,
    pub capabilities: u64,
    pub block_size: u32,
    pub max_name_bytes: u32,
    pub case_sensitive: u32,
    pub unicode_version_major: u8,
    pub unicode_version_minor: u8,
    pub unicode_version_patch: u8,
    pub reserved0: u8,
    pub pending_intent_records: u32,
    pub reserved1: u32,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub available_blocks: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosHealth {
    pub struct_size: u32,
    pub mount_mode: u32,
    pub flags: u32,
    pub pending_intent_records: u32,
    pub generation: u64,
    pub pending_orphans: u64,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub available_blocks: u64,
    pub device_errors: u64,
    pub corruption_errors: u64,
    pub no_space_errors: u64,
    pub internal_faults: u64,
    pub events_recorded: u64,
    pub events_dropped: u64,
    pub last_error: i32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosHealthEvent {
    pub sequence: u64,
    pub kind: u32,
    pub dos_error: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosTraceCounters {
    pub struct_size: u32,
    pub attached: u32,
    pub delivered: u64,
    pub missed: u64,
    pub filtered: u64,
    pub dropped: u64,
}

/// `struct afsp_trace_event` of `api/debug_observability.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfspTraceEvent {
    pub sequence: u64,
    pub timestamp: u64,
    pub transaction_id: u64,
    pub object_id: u64,
    pub block: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub task_id: u32,
    pub category: u16,
    pub event: u16,
}

pub type AfspTraceSinkFn = unsafe extern "C" fn(context: *mut c_void, event: *const AfspTraceEvent);

/// `struct afsp_trace_sink`.
#[repr(C)]
pub struct AfspTraceSink {
    pub emit: Option<AfspTraceSinkFn>,
    pub ctx: *mut c_void,
    pub category_mask: u64,
}

pub const AFSP_TRACE_TX: u64 = 1 << 0;
pub const AFSP_TRACE_CHECKPOINT: u64 = 1 << 1;
pub const AFSP_TRACE_IO: u64 = 1 << 2;
pub const AFSP_TRACE_ALLOC: u64 = 1 << 3;
pub const AFSP_TRACE_BTREE: u64 = 1 << 5;
pub const AFSP_TRACE_OBJECT: u64 = 1 << 6;
pub const AFSP_TRACE_RECLAIM: u64 = 1 << 10;
pub const AFSP_TRACE_ERROR: u64 = 1 << 12;
pub const AFSP_TRACE_API: u64 = 1 << 13;
pub const AFSP_TRACE_WINDOW: u64 = 1 << 14;
pub const AFSP_TRACE_LIFECYCLE: u64 = 1 << 15;

/// Core category to published trace bit. Data staging reports as I/O, view
/// descents as tree activity, and mount, format and verify as lifecycle.
const TRACE_CATEGORY_MAP: [(Category, u64); 15] = [
    (Category::Transaction, AFSP_TRACE_TX),
    (Category::Checkpoint, AFSP_TRACE_CHECKPOINT),
    (Category::Io, AFSP_TRACE_IO),
    (Category::Data, AFSP_TRACE_IO),
    (Category::Allocator, AFSP_TRACE_ALLOC),
    (Category::Tree, AFSP_TRACE_BTREE),
    (Category::View, AFSP_TRACE_BTREE),
    (Category::Object, AFSP_TRACE_OBJECT),
    (Category::Reclaim, AFSP_TRACE_RECLAIM),
    (Category::Error, AFSP_TRACE_ERROR),
    (Category::Api, AFSP_TRACE_API),
    (Category::Window, AFSP_TRACE_WINDOW),
    (Category::Mount, AFSP_TRACE_LIFECYCLE),
    (Category::Format, AFSP_TRACE_LIFECYCLE),
    (Category::Verify, AFSP_TRACE_LIFECYCLE),
];

fn trace_bit(category: Category) -> u64 {
    TRACE_CATEGORY_MAP
        .iter()
        .find(|(candidate, _)| *candidate == category)
        .map_or(0, |(_, bit)| *bit)
}

struct CallbackSink {
    emit: AfspTraceSinkFn,
    context: *mut c_void,
}

// SAFETY: one AFS+ instance is single-task by the boundary contract; the
// recorder holding this sink never leaves the handler task that installed it.
unsafe impl Send for CallbackSink {}
// SAFETY: as above; no shared access exists.
unsafe impl Sync for CallbackSink {}

impl LiveSink for CallbackSink {
    fn try_event(&mut self, event: Event) -> SinkResult {
        let (object_id, mut block) = event
            .object
            .map_or((0, 0), |object| (object.object_id, object.record_block));
        if let Some(tree) = event.tree {
            block = tree.block;
        }
        if let Some(allocation) = event.allocation {
            block = allocation.start;
        }
        let translated = AfspTraceEvent {
            sequence: event.sequence,
            timestamp: 0,
            transaction_id: event.attempt,
            object_id,
            block,
            arg0: event.generation,
            arg1: event.api.operation,
            task_id: 0,
            category: trace_bit(event.kind.category()) as u16,
            event: event.kind as u16,
        };
        // SAFETY: the installer guarantees the callback and context outlive
        // the attachment and that the callback does not unwind or reenter.
        unsafe { (self.emit)(self.context, ptr::from_ref(&translated)) };
        SinkResult::Accepted
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosCounters {
    pub struct_size: u32,
    pub reserved: u32,
    pub calls: u64,
    pub failed_calls: u64,
    pub device_reads: u64,
    pub device_writes: u64,
    pub device_flushes: u64,
    pub device_read_bytes: u64,
    pub device_written_bytes: u64,
    pub device_failures: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosStat {
    pub struct_size: u32,
    pub kind: u32,
    pub object_id: u64,
    pub size: u64,
    pub allocated_size: u64,
    pub protection: u64,
    pub links: u32,
    pub reserved0: u32,
    pub created_seconds: i64,
    pub modified_seconds: i64,
    pub changed_seconds: i64,
    pub created_nanoseconds: u32,
    pub modified_nanoseconds: u32,
    pub changed_nanoseconds: u32,
    pub reserved1: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosDirEntry {
    pub object_id: u64,
    pub kind: u32,
    pub name_length: u32,
    pub record_length: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfsplusArosExtent {
    pub offset: u64,
    pub length: u64,
    pub flags: u32,
    pub reserved: u32,
}

const _: [(); 24] = [(); std::mem::size_of::<AfsplusArosExtent>()];
const _: [(); 88] = [(); std::mem::size_of::<AfsplusArosStat>()];
const _: [(); 24] = [(); std::mem::size_of::<AfsplusArosDirEntry>()];
const _: [(); 72] = [(); std::mem::size_of::<AfsplusArosCounters>()];
const _: [(); 112] = [(); std::mem::size_of::<AfsplusArosHealth>()];
const _: [(); 16] = [(); std::mem::size_of::<AfsplusArosHealthEvent>()];
const _: [(); 40] = [(); std::mem::size_of::<AfsplusArosTraceCounters>()];
const _: [(); 64] = [(); std::mem::size_of::<AfspTraceEvent>()];
const _: [(); 24] = [(); std::mem::size_of::<AfsplusArosInterface>()];
const _: [(); 64] = [(); std::mem::size_of::<AfsplusArosCapabilities>()];
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

/// Device traffic of one mount, shared between the block adapter buried in
/// the volume and the boundary that reports it.
#[derive(Debug, Default)]
struct DeviceCounters {
    reads: AtomicU64,
    writes: AtomicU64,
    flushes: AtomicU64,
    read_bytes: AtomicU64,
    written_bytes: AtomicU64,
    failures: AtomicU64,
}

impl DeviceCounters {
    fn finish(
        &self,
        result: &Result<(), BlockError>,
        count: &AtomicU64,
        bytes: Option<(&AtomicU64, usize)>,
    ) {
        match result {
            Ok(()) => {
                count.fetch_add(1, Ordering::Relaxed);
                if let Some((total, length)) = bytes {
                    total.fetch_add(length as u64, Ordering::Relaxed);
                }
            }
            Err(_) => {
                self.failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

struct CallbackDevice {
    counters: Arc<DeviceCounters>,
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
        let result = callback_status(status);
        self.counters.finish(
            &result,
            &self.counters.reads,
            Some((&self.counters.read_bytes, buffer.len())),
        );
        result
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.validate_access(lba, data.len())?;
        let write = self.write_block.ok_or_else(|| callback_error(-1))?;
        let length = u32::try_from(data.len()).map_err(|_| callback_error(-1))?;
        // SAFETY: same lifetime contract as `read_block`; `data` is readable
        // for exactly `length` bytes and is not retained by the callback.
        let status = unsafe { write(self.context, lba, data.as_ptr(), length) };
        let result = callback_status(status);
        self.counters.finish(
            &result,
            &self.counters.writes,
            Some((&self.counters.written_bytes, data.len())),
        );
        result
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let flush = self.flush.ok_or_else(|| callback_error(-1))?;
        // SAFETY: the callback/context lifetime is the mount lifetime.
        let result = callback_status(unsafe { flush(self.context) });
        self.counters.finish(&result, &self.counters.flushes, None);
        result
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
    device: Arc<DeviceCounters>,
    calls: u64,
    failed_calls: u64,
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

/// `ffi_status` for a mounted instance: a failure that describes the volume
/// or its device, and any internal fault, also enters the health log.
fn bridge_status<F>(filesystem: *mut AfsplusAros, operation: F) -> i32
where
    F: FnOnce() -> Result<(), ArosError>,
{
    let outcome = catch_unwind(AssertUnwindSafe(operation));
    let Ok(bridge) = bridge_mut(filesystem) else {
        return ArosError::InvalidLock.io_error();
    };
    bridge.calls += 1;
    match outcome {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            bridge.failed_calls += 1;
            bridge.adapter.health_log().record(error);
            error.io_error()
        }
        Err(_) => {
            bridge.failed_calls += 1;
            bridge.adapter.health_log().record_internal_fault();
            ArosError::Unknown.io_error()
        }
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

/// Growth rule of the query structures: read the caller's size from the
/// leading `u32`, refuse a size below the first published layout, copy at most
/// that many bytes and report the count written in the same field.
/// The size each size-negotiated query struct had when it was first
/// published. A caller may declare any size from here up; the struct may
/// grow, these numbers never do, so a client built against the first layout
/// keeps working against every later library.
pub const AFSPLUS_AROS_INTERFACE_FIRST_LAYOUT: usize = 24;
pub const AFSPLUS_AROS_CAPABILITIES_FIRST_LAYOUT: usize = 64;
pub const AFSPLUS_AROS_HEALTH_FIRST_LAYOUT: usize = 112;
pub const AFSPLUS_AROS_TRACE_COUNTERS_FIRST_LAYOUT: usize = 40;
pub const AFSPLUS_AROS_COUNTERS_FIRST_LAYOUT: usize = 72;
pub const AFSPLUS_AROS_STAT_FIRST_LAYOUT: usize = 88;

// No struct may be smaller than the layout it was first published with.
const _: () = {
    assert!(std::mem::size_of::<AfsplusArosInterface>() >= AFSPLUS_AROS_INTERFACE_FIRST_LAYOUT);
    assert!(
        std::mem::size_of::<AfsplusArosCapabilities>() >= AFSPLUS_AROS_CAPABILITIES_FIRST_LAYOUT
    );
    assert!(std::mem::size_of::<AfsplusArosHealth>() >= AFSPLUS_AROS_HEALTH_FIRST_LAYOUT);
    assert!(
        std::mem::size_of::<AfsplusArosTraceCounters>() >= AFSPLUS_AROS_TRACE_COUNTERS_FIRST_LAYOUT
    );
    assert!(std::mem::size_of::<AfsplusArosCounters>() >= AFSPLUS_AROS_COUNTERS_FIRST_LAYOUT);
    assert!(std::mem::size_of::<AfsplusArosStat>() >= AFSPLUS_AROS_STAT_FIRST_LAYOUT);
};

fn write_sized_output<T: Copy>(
    output: *mut T,
    minimum: usize,
    mut value: T,
    set_size: impl Fn(&mut T, u32),
) -> Result<(), ArosError> {
    require_output(output)?;
    // SAFETY: every query structure starts with its `u32` size, and the
    // caller provides at least that aligned field.
    let caller = unsafe { ptr::read(output.cast::<u32>()) } as usize;
    if caller < minimum {
        return Err(ArosError::BadNumber);
    }
    let count = caller.min(std::mem::size_of::<T>());
    set_size(&mut value, count as u32);
    // SAFETY: the caller declared `caller >= count` writable bytes; `value`
    // is a plain `repr(C)` structure without padding-sensitive invariants.
    unsafe {
        ptr::copy_nonoverlapping(
            ptr::from_ref(&value).cast::<u8>(),
            output.cast::<u8>(),
            count,
        );
    }
    Ok(())
}

fn mount_mode_value(mode: MountMode) -> u32 {
    match mode {
        MountMode::ReadWrite => AFSPLUS_AROS_MOUNT_READ_WRITE,
        MountMode::ReadOnly => AFSPLUS_AROS_MOUNT_READ_ONLY,
        MountMode::NoChanges => AFSPLUS_AROS_MOUNT_NO_CHANGES,
        MountMode::Recovery => AFSPLUS_AROS_MOUNT_RECOVERY,
    }
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
            || config.flags & !MOUNT_FLAGS != 0
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
        // No name: the volume is named after its committed label.
        let volume_name = if config.volume_name_length == 0 {
            Vec::new()
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
        let counters = Arc::new(DeviceCounters::default());
        let callback_device = CallbackDevice {
            counters: Arc::clone(&counters),
            context: device.context,
            block_size: device.block_size as usize,
            total_blocks: device.total_blocks,
            read_block,
            write_block: device.write_block,
            flush: device.flush,
        };
        let vfs = Vfs::mount(
            callback_device,
            MountOptions {
                mode,
                ..Default::default()
            },
        )?;
        let adapter = ArosAdapter::new(
            vfs,
            ArosConfig {
                name_encoding: name_encoding(config.name_encoding)?,
                volume_name,
                max_file_handles,
                max_locks,
                max_file_info_name_bytes,
                allow_security_downgrade: config.flags & AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE
                    != 0,
                strict_security_projection: config.flags
                    & AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION
                    != 0,
                ..ArosConfig::default()
            },
        );
        let raw = Box::into_raw(Box::new(NativeBridge {
            adapter,
            max_file_info_name_bytes,
            device: counters,
            calls: 0,
            failed_calls: 0,
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
        require_output(output_lock)?;
        let parent = bridge_mut(filesystem)?.adapter.parent_lock(lock)?;
        write_output(output_lock, parent.unwrap_or(0))
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_parent_lock_with_access(
    filesystem: *mut AfsplusAros,
    lock: u64,
    access: u32,
    output_lock: *mut u64,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_lock)?;
        let parent = bridge_mut(filesystem)?
            .adapter
            .parent_lock_with_access(lock, lock_access(access)?)?;
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
    bridge_status(filesystem, || {
        require_output(output_same)?;
        let same = bridge_mut(filesystem)?
            .adapter
            .same_lock(optional_lock(first_lock), optional_lock(second_lock))?;
        write_output(output_same, u32::from(same))
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_free_lock(filesystem: *mut AfsplusAros, lock: u64) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?.adapter.free_lock(lock)
    })
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
        require_output(output_lock)?;
        let lock = bridge_mut(filesystem)?.adapter.parent_of_file(file)?;
        write_output(output_lock, lock)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_lock_from_file(
    filesystem: *mut AfsplusAros,
    file: u64,
    output_lock: *mut u64,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_lock)?;
        let lock = bridge_mut(filesystem)?.adapter.lock_from_file(file)?;
        write_output(output_lock, lock)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_close(filesystem: *mut AfsplusAros, file: u64) -> i32 {
    bridge_status(filesystem, || bridge_mut(filesystem)?.adapter.close(file))
}

#[no_mangle]
pub extern "C" fn afsplus_aros_read(
    filesystem: *mut AfsplusAros,
    file: u64,
    destination: *mut u8,
    length: u32,
    output_count: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || bridge_mut(filesystem)?.adapter.fsync(file))
}

#[no_mangle]
pub extern "C" fn afsplus_aros_flush(filesystem: *mut AfsplusAros) -> i32 {
    bridge_status(filesystem, || bridge_mut(filesystem)?.adapter.flush())
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
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
    bridge_status(filesystem, || {
        let bridge = bridge_mut(filesystem)?;
        validate_file_info_output(bridge, output, name, name_capacity)?;
        let info = bridge.adapter.examine_next(lock)?;
        copy_file_info(bridge, info, output, name, name_capacity)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_rewind_directory(filesystem: *mut AfsplusAros, lock: u64) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?.adapter.rewind_directory(lock)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_disk_info(
    filesystem: *mut AfsplusAros,
    output: *mut AfsplusArosDiskInfo,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output)?;
        copy_disk_info(bridge_mut(filesystem)?.adapter.disk_info(), output)
    })
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn afsplus_aros_interface(output: *mut AfsplusArosInterface) -> i32 {
    ffi_status(|| {
        write_sized_output(
            output,
            AFSPLUS_AROS_INTERFACE_FIRST_LAYOUT,
            AfsplusArosInterface {
                struct_size: 0,
                abi_version: AFSPLUS_AROS_ABI_VERSION,
                interface_revision: AFSPLUS_AROS_INTERFACE_REVISION,
                reserved: 0,
                groups: AFSPLUS_AROS_GROUPS,
            },
            |value, size| value.struct_size = size,
        )
    })
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn afsplus_aros_capabilities(
    filesystem: *mut AfsplusAros,
    output: *mut AfsplusArosCapabilities,
) -> i32 {
    bridge_status(filesystem, || {
        let policy = bridge_mut(filesystem)?.adapter.volume_policy();
        write_sized_output(
            output,
            AFSPLUS_AROS_CAPABILITIES_FIRST_LAYOUT,
            AfsplusArosCapabilities {
                struct_size: 0,
                mount_mode: mount_mode_value(policy.mount_mode),
                capabilities: published_capabilities(policy.capabilities),
                block_size: policy.statfs.block_size,
                max_name_bytes: policy.statfs.max_name_bytes,
                case_sensitive: u32::from(policy.statfs.case_sensitive),
                unicode_version_major: policy.statfs.unicode_version[0],
                unicode_version_minor: policy.statfs.unicode_version[1],
                unicode_version_patch: policy.statfs.unicode_version[2],
                reserved0: 0,
                pending_intent_records: policy.pending_intent_records,
                reserved1: 0,
                total_blocks: policy.statfs.total_blocks,
                free_blocks: policy.statfs.free_blocks,
                available_blocks: policy.statfs.available_blocks,
            },
            |value, size| value.struct_size = size,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_set_protection(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    protection: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        let name = input_bytes(name, name_length)?;
        bridge_mut(filesystem)?.adapter.set_protection(
            optional_lock(base_lock),
            name,
            protection,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn afsplus_aros_set_modified(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    modified_seconds: i64,
    modified_nanoseconds: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        let name = input_bytes(name, name_length)?;
        bridge_mut(filesystem)?.adapter.set_modified(
            optional_lock(base_lock),
            name,
            timestamp(modified_seconds, modified_nanoseconds)?,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn afsplus_aros_make_soft_link(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    target: *const u8,
    target_length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        let name = input_bytes(name, name_length)?;
        let target = input_bytes(target, target_length)?;
        bridge_mut(filesystem)?.adapter.make_soft_link(
            optional_lock(base_lock),
            name,
            target,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_read_soft_link(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    target: *mut u8,
    target_capacity: u32,
    output_required: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_required)?;
        let name = input_bytes(name, name_length)?;
        let target = output_slice(target, target_capacity)?;
        let required = bridge_mut(filesystem)?.adapter.read_soft_link(
            optional_lock(base_lock),
            name,
            target,
        )?;
        write_output(
            output_required,
            u32::try_from(required).map_err(|_| ArosError::ObjectTooLarge)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_set_comment(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    comment: *const u8,
    comment_length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        let name = input_bytes(name, name_length)?;
        let comment = input_bytes(comment, comment_length)?;
        bridge_mut(filesystem)?.adapter.set_comment(
            optional_lock(base_lock),
            name,
            comment,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_comment(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    comment: *mut u8,
    comment_capacity: u32,
    output_length: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_length)?;
        let name = input_bytes(name, name_length)?;
        let destination = output_slice(comment, comment_capacity)?;
        let text = bridge_mut(filesystem)?.adapter.comment(
            optional_lock(base_lock),
            name,
            destination.len(),
        )?;
        destination[..text.len()].copy_from_slice(&text);
        write_output(output_length, text.len() as u32)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_file_comment(
    filesystem: *mut AfsplusAros,
    file: u64,
    comment: *mut u8,
    comment_capacity: u32,
    output_length: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_length)?;
        let destination = output_slice(comment, comment_capacity)?;
        let text = bridge_mut(filesystem)?
            .adapter
            .file_comment(file, destination.len())?;
        destination[..text.len()].copy_from_slice(&text);
        write_output(output_length, text.len() as u32)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_read_at(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    destination: *mut u8,
    length: u32,
    output_count: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_count)?;
        let destination = output_slice(destination, length)?;
        let count = bridge_mut(filesystem)?
            .adapter
            .read_at(file, offset, destination)?;
        write_output(output_count, count as u32)
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn afsplus_aros_write_at(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    source: *const u8,
    length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
    output_count: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_count)?;
        let source = input_bytes(source, length)?;
        let count = bridge_mut(filesystem)?.adapter.write_at(
            file,
            offset,
            source,
            timestamp(now_seconds, now_nanoseconds)?,
        )?;
        write_output(output_count, count as u32)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_clone_file(
    filesystem: *mut AfsplusAros,
    source_lock: u64,
    target_base_lock: u64,
    target_name: *const u8,
    target_name_length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        let name = input_bytes(target_name, target_name_length)?;
        bridge_mut(filesystem)?.adapter.clone_file(
            source_lock,
            optional_lock(target_base_lock),
            name,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn afsplus_aros_clone_range(
    filesystem: *mut AfsplusAros,
    source_file: u64,
    source_offset: u64,
    target_file: u64,
    target_offset: u64,
    length: u64,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?.adapter.clone_range(
            source_file,
            source_offset,
            target_file,
            target_offset,
            length,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_preallocate(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    length: u64,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?.adapter.preallocate(
            file,
            offset,
            length,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn afsplus_aros_replace(
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
    bridge_status(filesystem, || {
        let source = input_bytes(source_name, source_name_length)?;
        let target = input_bytes(target_name, target_name_length)?;
        bridge_mut(filesystem)?.adapter.replace(
            optional_lock(source_base_lock),
            source,
            optional_lock(target_base_lock),
            target,
            timestamp(now_seconds, now_nanoseconds)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_advise(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    length: u64,
    hint: u32,
    output_effect: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_effect)?;
        let effect = bridge_mut(filesystem)?
            .adapter
            .advise(file, offset, length, hint)?;
        write_output(output_effect, effect as u32)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_watch_add(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    output_watch: *mut u64,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_watch)?;
        let name = input_bytes(name, name_length)?;
        let watch = bridge_mut(filesystem)?
            .adapter
            .add_watch(optional_lock(base_lock), name)?;
        write_output(output_watch, watch)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_watch_remove(filesystem: *mut AfsplusAros, watch: u64) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?.adapter.remove_watch(watch)
    })
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn afsplus_aros_watch_drain(
    filesystem: *mut AfsplusAros,
    watches: *mut u64,
    capacity: u32,
    output_count: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_count)?;
        let bridge = bridge_mut(filesystem)?;
        if capacity == 0 {
            return write_output(output_count, 0);
        }
        if watches.is_null() {
            return Err(ArosError::InvalidComponentName);
        }
        // SAFETY: the caller provides `capacity` aligned writable slots.
        let output = unsafe { slice::from_raw_parts_mut(watches, capacity as usize) };
        let count = bridge.adapter.drain_watches(output);
        write_output(output_count, count as u32)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_health(
    filesystem: *mut AfsplusAros,
    output: *mut AfsplusArosHealth,
) -> i32 {
    bridge_status(filesystem, || {
        let health = bridge_mut(filesystem)?.adapter.health()?;
        write_sized_output(
            output,
            AFSPLUS_AROS_HEALTH_FIRST_LAYOUT,
            AfsplusArosHealth {
                struct_size: 0,
                mount_mode: mount_mode_value(health.mount_mode),
                flags: health.flags,
                pending_intent_records: health.pending_intent_records,
                generation: health.generation,
                pending_orphans: health.pending_orphans,
                total_blocks: health.total_blocks,
                free_blocks: health.free_blocks,
                available_blocks: health.available_blocks,
                device_errors: health.device_errors,
                corruption_errors: health.corruption_errors,
                no_space_errors: health.no_space_errors,
                internal_faults: health.internal_faults,
                events_recorded: health.events_recorded,
                events_dropped: health.events_dropped,
                last_error: health.last_error,
                reserved: 0,
            },
            |value, size| value.struct_size = size,
        )
    })
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn afsplus_aros_health_events(
    filesystem: *mut AfsplusAros,
    events: *mut AfsplusArosHealthEvent,
    capacity: u32,
    output_count: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_count)?;
        let bridge = bridge_mut(filesystem)?;
        if capacity != 0 && events.is_null() {
            return Err(ArosError::InvalidComponentName);
        }
        let mut written = 0u32;
        while written < capacity {
            let mut one = [HealthEvent {
                sequence: 0,
                kind: afsplus_aros::health::HealthEventKind::InternalFault,
                dos_error: 0,
            }];
            if bridge.adapter.health_log().drain(&mut one) == 0 {
                break;
            }
            // SAFETY: `written < capacity` writable aligned slots exist.
            unsafe {
                ptr::write(
                    events.add(written as usize),
                    AfsplusArosHealthEvent {
                        sequence: one[0].sequence,
                        kind: one[0].kind as u32,
                        dos_error: one[0].dos_error,
                    },
                );
            }
            written += 1;
        }
        write_output(output_count, written)
    })
}

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn afsplus_aros_set_trace_sink(
    filesystem: *mut AfsplusAros,
    sink: *const AfspTraceSink,
) -> i32 {
    bridge_status(filesystem, || {
        let bridge = bridge_mut(filesystem)?;
        // SAFETY: a non-null sink is a readable structure for this call.
        let Some(sink) = (unsafe { sink.as_ref() }) else {
            bridge.adapter.replace_flight_recorder(None);
            return Ok(());
        };
        let emit = sink.emit.ok_or(ArosError::InvalidComponentName)?;
        let mut categories = Categories::NONE;
        for (category, bit) in TRACE_CATEGORY_MAP {
            if sink.category_mask & bit != 0 {
                categories = categories.with(category);
            }
        }
        let capacity = std::num::NonZeroUsize::new(64).expect("nonzero literal");
        let mut recorder = FlightRecorder::new(capacity).map_err(|_| ArosError::NoFreeStore)?;
        recorder.set_categories(categories);
        if categories.contains(Category::Api) {
            recorder.enable_api_observation();
        }
        if categories.contains(Category::Object) {
            recorder.enable_object_observation();
        }
        if categories.contains(Category::Allocator)
            || categories.contains(Category::Tree)
            || categories.contains(Category::Reclaim)
        {
            recorder.enable_subsystem_observation();
        }
        if categories.contains(Category::Data) {
            recorder.enable_data_observation();
        }
        if categories.contains(Category::View) {
            recorder.enable_view_observation();
        }
        recorder.replace_sink(Some(Box::new(CallbackSink {
            emit,
            context: sink.ctx,
        })));
        bridge.adapter.replace_flight_recorder(Some(recorder));
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_trace_counters(
    filesystem: *mut AfsplusAros,
    output: *mut AfsplusArosTraceCounters,
) -> i32 {
    bridge_status(filesystem, || {
        let counters = bridge_mut(filesystem)?
            .adapter
            .with_flight_recorder(|recorder| {
                // The ring only mirrors what the sink already received.
                let _ = recorder.drain().count();
                AfsplusArosTraceCounters {
                    struct_size: 0,
                    attached: 1,
                    delivered: recorder.delivered(),
                    missed: recorder.missed(),
                    filtered: recorder.filtered(),
                    dropped: recorder.dropped(),
                }
            })
            .unwrap_or_default();
        write_sized_output(
            output,
            AFSPLUS_AROS_TRACE_COUNTERS_FIRST_LAYOUT,
            counters,
            |value, size| value.struct_size = size,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_info_json(
    filesystem: *mut AfsplusAros,
    buffer: *mut u8,
    capacity: u32,
    output_required: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_required)?;
        let destination = output_slice(buffer, capacity)?;
        let document = bridge_mut(filesystem)?.adapter.info_json()?;
        let bytes = document.as_bytes();
        if bytes.len() <= destination.len() {
            destination[..bytes.len()].copy_from_slice(bytes);
        }
        write_output(
            output_required,
            u32::try_from(bytes.len()).map_err(|_| ArosError::ObjectTooLarge)?,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_counters(
    filesystem: *mut AfsplusAros,
    output: *mut AfsplusArosCounters,
) -> i32 {
    bridge_status(filesystem, || {
        let bridge = bridge_mut(filesystem)?;
        let device = &bridge.device;
        write_sized_output(
            output,
            AFSPLUS_AROS_COUNTERS_FIRST_LAYOUT,
            AfsplusArosCounters {
                struct_size: 0,
                reserved: 0,
                calls: bridge.calls,
                failed_calls: bridge.failed_calls,
                device_reads: device.reads.load(Ordering::Relaxed),
                device_writes: device.writes.load(Ordering::Relaxed),
                device_flushes: device.flushes.load(Ordering::Relaxed),
                device_read_bytes: device.read_bytes.load(Ordering::Relaxed),
                device_written_bytes: device.written_bytes.load(Ordering::Relaxed),
                device_failures: device.failures.load(Ordering::Relaxed),
            },
            |value, size| value.struct_size = size,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_open_from_lock(
    filesystem: *mut AfsplusAros,
    lock: u64,
    output_file: *mut u64,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_file)?;
        let file = bridge_mut(filesystem)?.adapter.open_from_lock(lock)?;
        write_output(output_file, file)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_change_lock_mode(
    filesystem: *mut AfsplusAros,
    lock: u64,
    access: u32,
) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?
            .adapter
            .change_lock_mode(lock, lock_access(access)?)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_change_file_mode(
    filesystem: *mut AfsplusAros,
    file: u64,
    access: u32,
) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?
            .adapter
            .change_file_mode(file, lock_access(access)?)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_set_write_protect(
    filesystem: *mut AfsplusAros,
    protect: u32,
    key: u32,
) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?
            .adapter
            .set_write_protect(protect != 0, key)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_lock_record(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    length: u64,
    exclusive: u32,
) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?
            .adapter
            .lock_record(file, offset, length, exclusive != 0)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_free_record(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    length: u64,
) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?
            .adapter
            .free_record(file, offset, length)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_lookup_id(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    name: *const u8,
    name_length: u32,
    output_object_id: *mut u64,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_object_id)?;
        let name = std::str::from_utf8(input_bytes(name, name_length)?)
            .map_err(|_| ArosError::InvalidComponentName)?;
        let object = bridge_mut(filesystem)?
            .adapter
            .lookup_id(optional_lock(base_lock), name)?;
        write_output(output_object_id, object)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_stat_id(
    filesystem: *mut AfsplusAros,
    object_id: u64,
    output: *mut AfsplusArosStat,
) -> i32 {
    bridge_status(filesystem, || {
        let stat = bridge_mut(filesystem)?.adapter.stat_id(object_id)?;
        write_sized_output(
            output,
            AFSPLUS_AROS_STAT_FIRST_LAYOUT,
            AfsplusArosStat {
                struct_size: 0,
                kind: stat.kind as u32,
                object_id: stat.object_id,
                size: stat.size,
                allocated_size: stat.allocated_size,
                protection: stat.protection,
                links: stat.links,
                reserved0: 0,
                created_seconds: stat.created.seconds,
                modified_seconds: stat.modified.seconds,
                changed_seconds: stat.changed.seconds,
                created_nanoseconds: stat.created.nanoseconds,
                modified_nanoseconds: stat.modified.nanoseconds,
                changed_nanoseconds: stat.changed.nanoseconds,
                reserved1: 0,
            },
            |value, size| value.struct_size = size,
        )
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_dir_open(
    filesystem: *mut AfsplusAros,
    base_lock: u64,
    output_dir: *mut u64,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_dir)?;
        let enumerator = bridge_mut(filesystem)?
            .adapter
            .open_enumerator(optional_lock(base_lock))?;
        write_output(output_dir, enumerator)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_dir_read(
    filesystem: *mut AfsplusAros,
    dir: u64,
    buffer: *mut u8,
    capacity: u32,
    max_entries: u32,
    output_count: *mut u32,
    output_eof: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_count)?;
        require_output(output_eof)?;
        // Read no more entries than the buffer is certain to hold: an entry
        // taken from the walk and then not delivered would be lost.
        let certain = capacity / AFSPLUS_AROS_DIR_RECORD_MAX;
        if certain == 0 || max_entries == 0 {
            return Err(ArosError::BadNumber);
        }
        let limit = max_entries.min(certain).min(64) as usize;
        let destination = output_slice(buffer, capacity)?;
        let page = bridge_mut(filesystem)?
            .adapter
            .read_enumerator(dir, limit)?;
        let header = std::mem::size_of::<AfsplusArosDirEntry>();
        let mut at = 0usize;
        for entry in &page.entries {
            let record = (header + entry.name.len() + 7) & !7;
            let fixed = AfsplusArosDirEntry {
                object_id: entry.object_id,
                kind: entry.kind as u32,
                name_length: entry.name.len() as u32,
                record_length: record as u32,
                reserved: 0,
            };
            let slot = destination
                .get_mut(at..at + record)
                .ok_or(ArosError::ObjectTooLarge)?;
            slot.fill(0);
            // SAFETY: `slot` holds at least `header` bytes and `fixed` is a
            // plain `repr(C)` value; the copy is byte-wise, so the caller's
            // buffer alignment does not matter.
            unsafe {
                ptr::copy_nonoverlapping(
                    ptr::from_ref(&fixed).cast::<u8>(),
                    slot.as_mut_ptr(),
                    header,
                );
            }
            slot[header..header + entry.name.len()].copy_from_slice(&entry.name);
            at += record;
        }
        write_output(output_count, page.entries.len() as u32)?;
        write_output(output_eof, u32::from(page.eof))
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_dir_close(filesystem: *mut AfsplusAros, dir: u64) -> i32 {
    bridge_status(filesystem, || {
        bridge_mut(filesystem)?.adapter.close_enumerator(dir)
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments, clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn afsplus_aros_extent_map(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    length: u64,
    extents: *mut AfsplusArosExtent,
    capacity: u32,
    output_count: *mut u32,
    output_complete: *mut u32,
    output_next_offset: *mut u64,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_count)?;
        require_output(output_complete)?;
        require_output(output_next_offset)?;
        if extents.is_null() {
            return Err(ArosError::InvalidComponentName);
        }
        let map =
            bridge_mut(filesystem)?
                .adapter
                .extent_map(file, offset, length, capacity as usize)?;
        for (index, range) in map.ranges.iter().enumerate() {
            // SAFETY: the adapter returns at most `capacity` ranges and the
            // caller provides that many aligned writable slots.
            unsafe {
                ptr::write(
                    extents.add(index),
                    AfsplusArosExtent {
                        offset: range.offset,
                        length: range.length,
                        flags: if range.unwritten {
                            AFSPLUS_AROS_EXTENT_UNWRITTEN
                        } else {
                            0
                        },
                        reserved: 0,
                    },
                );
            }
        }
        write_output(output_count, map.ranges.len() as u32)?;
        write_output(output_complete, u32::from(map.complete))?;
        write_output(output_next_offset, map.next_offset)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_volume_label(
    filesystem: *mut AfsplusAros,
    label: *mut u8,
    capacity: u32,
    output_required: *mut u32,
) -> i32 {
    bridge_status(filesystem, || {
        require_output(output_required)?;
        let destination = output_slice(label, capacity)?;
        let current = bridge_mut(filesystem)?.adapter.volume_label()?;
        if current.len() <= destination.len() {
            destination[..current.len()].copy_from_slice(&current);
        }
        write_output(output_required, current.len() as u32)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_set_volume_label(
    filesystem: *mut AfsplusAros,
    label: *const u8,
    label_length: u32,
    now_seconds: i64,
    now_nanoseconds: u32,
) -> i32 {
    bridge_status(filesystem, || {
        let label = input_bytes(label, label_length)?;
        bridge_mut(filesystem)?
            .adapter
            .set_volume_label(label, timestamp(now_seconds, now_nanoseconds)?)
    })
}

#[cfg(test)]
mod sized_output_tests {
    use super::*;

    /// A query struct after it has grown: its first layout was the first
    /// sixteen bytes.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Grown {
        struct_size: u32,
        first: u32,
        second: u64,
        added_later: u64,
    }

    const GROWN_FIRST_LAYOUT: usize = 16;

    fn ask(declared: u32) -> (Result<(), ArosError>, [u8; 32]) {
        let mut memory = [0xA5u8; 32];
        memory[..4].copy_from_slice(&declared.to_ne_bytes());
        let value = Grown {
            struct_size: 0,
            first: 0x1111_1111,
            second: 0x2222_2222_2222_2222,
            added_later: 0x3333_3333_3333_3333,
        };
        let result = write_sized_output(
            memory.as_mut_ptr().cast::<Grown>(),
            GROWN_FIRST_LAYOUT,
            value,
            |value, size| value.struct_size = size,
        );
        (result, memory)
    }

    #[test]
    fn a_client_of_the_first_layout_is_served_after_the_struct_grew() {
        // The floor is the first layout, not the current size of the struct.
        let (result, memory) = ask(16);
        assert_eq!(result, Ok(()));
        assert_eq!(memory[..4], 16u32.to_ne_bytes());
        assert_eq!(memory[4..8], 0x1111_1111u32.to_ne_bytes());
        assert_eq!(memory[8..16], 0x2222_2222_2222_2222u64.to_ne_bytes());
        assert!(memory[16..].iter().all(|byte| *byte == 0xA5));

        // Below the first layout nothing is written.
        let (result, memory) = ask(15);
        assert_eq!(result, Err(ArosError::BadNumber));
        assert!(memory[4..].iter().all(|byte| *byte == 0xA5));

        // A current client gets everything, a future one no more than exists.
        let (result, memory) = ask(24);
        assert_eq!(result, Ok(()));
        assert_eq!(memory[16..24], 0x3333_3333_3333_3333u64.to_ne_bytes());
        let (result, memory) = ask(32);
        assert_eq!(result, Ok(()));
        assert_eq!(memory[..4], 24u32.to_ne_bytes());
        assert!(memory[24..].iter().all(|byte| *byte == 0xA5));
    }

    #[test]
    fn floors_are_the_published_numbers() {
        // Literal values: a struct that grows must not move them.
        assert_eq!(AFSPLUS_AROS_INTERFACE_FIRST_LAYOUT, 24);
        assert_eq!(AFSPLUS_AROS_CAPABILITIES_FIRST_LAYOUT, 64);
        assert_eq!(AFSPLUS_AROS_HEALTH_FIRST_LAYOUT, 112);
        assert_eq!(AFSPLUS_AROS_TRACE_COUNTERS_FIRST_LAYOUT, 40);
        assert_eq!(AFSPLUS_AROS_COUNTERS_FIRST_LAYOUT, 72);
        assert_eq!(AFSPLUS_AROS_STAT_FIRST_LAYOUT, 88);
    }
}
