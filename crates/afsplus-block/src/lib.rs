//! Block-provider abstraction for AFS+ (`docs/02-architecture.md`, layer D)
//! plus the test backends required by Stage A of the roadmap: memory images,
//! sparse host files, tracing/accounting, deterministic fault injection, and
//! power-cut simulation.
//!
//! Durability model: `flush` is a barrier — when it returns, every previously
//! written block is durable. After a power cut, writes issued since the last
//! completed flush may each be applied, lost, or torn (partially applied).
//! The power-cut harness in [`powercut`] enumerates all full-write subsets
//! of that tail plus representative torn-write states.

pub mod activity;
pub mod cache;
pub mod fault;
pub mod file;
pub mod memory;
pub mod overlay;
pub mod powercut;
pub mod slice;
pub mod trace;

use std::fmt;

pub use activity::{
    ActivityBackend, ActivityEvent, ActivityOperation, ActivityPhase, ActivitySink,
};
pub use cache::{CacheControl, CacheStats, CachedDevice};
pub use fault::{FaultBackend, FaultPlan};
pub use file::FileBackend;
pub use memory::MemoryBackend;
pub use overlay::{OverlayBackend, OverlayLimits};
pub use powercut::{
    crash_states, for_each_crash_state, for_each_crash_state_with_budget, CrashState, RecordedOp,
    RecordingBackend,
};
pub use slice::SliceBackend;
pub use trace::{IoStats, TraceBackend, TraceEvent};

#[derive(Debug)]
pub enum BlockError {
    /// Access beyond the device bounds.
    OutOfBounds { lba: u64, total_blocks: u64 },
    /// Buffer size does not match the device block size.
    WrongBufferSize { expected: usize, actual: usize },
    /// A deterministic injected fault (fault-injection backend).
    Injected(&'static str),
    /// Host I/O error.
    Io(std::io::Error),
}

impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockError::OutOfBounds { lba, total_blocks } => {
                write!(
                    f,
                    "block {lba} out of bounds (device has {total_blocks} blocks)"
                )
            }
            BlockError::WrongBufferSize { expected, actual } => {
                write!(f, "wrong buffer size: expected {expected}, got {actual}")
            }
            BlockError::Injected(point) => write!(f, "injected fault at {point}"),
            BlockError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for BlockError {}

impl From<std::io::Error> for BlockError {
    fn from(e: std::io::Error) -> Self {
        BlockError::Io(e)
    }
}

/// The narrow device abstraction the core sees.
///
/// Reads of never-written blocks return zeros on every backend, so freshly
/// created images have deterministic content.
pub trait BlockDevice {
    fn block_size(&self) -> usize;
    fn total_blocks(&self) -> u64;
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError>;
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError>;
    /// Durability barrier: all previously written blocks are durable on return.
    fn flush(&mut self) -> Result<(), BlockError>;

    /// Reads the run of whole blocks that starts at `lba` into `buf`, whose
    /// length is a multiple of the block size. This is the path of file
    /// data: a device that can transfers the run in one request, and a
    /// cache keeps what it reads apart from the metadata that
    /// [`BlockDevice::read_block`] reads. The bytes are those of the same
    /// blocks read one at a time, which is what this default does.
    fn read_blocks(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let block_size = self.block_size();
        check_run(buf.len(), block_size)?;
        for (index, block) in buf.chunks_exact_mut(block_size).enumerate() {
            self.read_block(lba + index as u64, block)?;
        }
        Ok(())
    }

    /// Writes `data`, a whole number of blocks, to the run that starts at
    /// `lba`: the same blocks in the same order as one
    /// [`BlockDevice::write_block`] each, which is what this default does.
    /// A device that transfers the run in one request may, like any write
    /// before the next barrier, have applied any part of it when it fails
    /// or the power is cut.
    fn write_blocks(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        let block_size = self.block_size();
        check_run(data.len(), block_size)?;
        for (index, block) in data.chunks_exact(block_size).enumerate() {
            self.write_block(lba + index as u64, block)?;
        }
        Ok(())
    }

    /// A decoded form of block `lba` that a caller attached with
    /// [`BlockDevice::attach`] and that no write of the block has dropped
    /// since. A device that keeps none answers `None`, and that is always
    /// correct: the caller decodes the bytes itself.
    fn attached(&self, _lba: u64) -> Option<Decoded> {
        None
    }

    /// [`BlockDevice::attached`] for a caller about to use the decoded form
    /// in place of the block's bytes: a device that orders its cache by use
    /// counts this as a use of the block, exactly as a read of it would be.
    fn decoded(&mut self, lba: u64) -> Option<Decoded> {
        self.attached(lba)
    }

    /// How many blocks the device may keep a decoded form for; zero when it
    /// keeps none. A caller sizes what it remembers about kept blocks by
    /// it, so that memory follows the one cache setting.
    fn decoded_capacity(&self) -> usize {
        0
    }

    /// Keeps `value`, decoded from the bytes block `lba` holds now, until the
    /// block is written or leaves the device's cache. A device that keeps
    /// none ignores it.
    fn attach(&mut self, _lba: u64, _value: Decoded) {}
}

/// A decoded block a device may keep beside its bytes; the caller knows the
/// type and downcasts.
pub type Decoded = std::sync::Arc<dyn std::any::Any + Send + Sync>;

/// Refuses a run that is not a whole number of blocks.
pub fn check_run(length: usize, block_size: usize) -> Result<(), BlockError> {
    if block_size == 0 || !length.is_multiple_of(block_size) {
        return Err(BlockError::WrongBufferSize {
            expected: block_size,
            actual: length,
        });
    }
    Ok(())
}

pub(crate) fn check_access(
    lba: u64,
    total_blocks: u64,
    buf_len: usize,
    block_size: usize,
) -> Result<(), BlockError> {
    if lba >= total_blocks {
        return Err(BlockError::OutOfBounds { lba, total_blocks });
    }
    if buf_len != block_size {
        return Err(BlockError::WrongBufferSize {
            expected: block_size,
            actual: buf_len,
        });
    }
    Ok(())
}
