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
pub mod fault;
pub mod file;
pub mod memory;
pub mod powercut;
pub mod trace;

use std::fmt;

pub use activity::{
    ActivityBackend, ActivityEvent, ActivityOperation, ActivityPhase, ActivitySink,
};
pub use fault::{FaultBackend, FaultPlan};
pub use file::FileBackend;
pub use memory::MemoryBackend;
pub use powercut::{crash_states, for_each_crash_state, CrashState, RecordedOp, RecordingBackend};
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
