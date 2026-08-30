//! Deterministic fault-injection wrapper.
//!
//! Fails the Nth write or the Nth flush with `BlockError::Injected`. When
//! `fail_hard` is set, every subsequent operation also fails, modeling a
//! device that disappears rather than one transient error.

use crate::{BlockDevice, BlockError};

#[derive(Debug, Clone, Copy, Default)]
pub struct FaultPlan {
    /// Zero-based index of the write operation that fails.
    pub fail_write_index: Option<u64>,
    /// Zero-based index of the flush operation that fails.
    pub fail_flush_index: Option<u64>,
    /// After the first injected fault, fail every subsequent operation.
    pub fail_hard: bool,
}

#[derive(Debug)]
pub struct FaultBackend<D: BlockDevice> {
    inner: D,
    plan: FaultPlan,
    writes_seen: u64,
    flushes_seen: u64,
    tripped: bool,
}

impl<D: BlockDevice> FaultBackend<D> {
    pub fn new(inner: D, plan: FaultPlan) -> Self {
        FaultBackend {
            inner,
            plan,
            writes_seen: 0,
            flushes_seen: 0,
            tripped: false,
        }
    }

    /// Whether any fault has been injected so far.
    pub fn tripped(&self) -> bool {
        self.tripped
    }

    pub fn into_inner(self) -> D {
        self.inner
    }

    fn dead(&self) -> bool {
        self.tripped && self.plan.fail_hard
    }
}

impl<D: BlockDevice> BlockDevice for FaultBackend<D> {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if self.dead() {
            return Err(BlockError::Injected("device gone after fault"));
        }
        self.inner.read_block(lba, buf)
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        if self.dead() {
            return Err(BlockError::Injected("device gone after fault"));
        }
        let index = self.writes_seen;
        self.writes_seen += 1;
        if self.plan.fail_write_index == Some(index) {
            self.tripped = true;
            return Err(BlockError::Injected("write fault"));
        }
        self.inner.write_block(lba, data)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        if self.dead() {
            return Err(BlockError::Injected("device gone after fault"));
        }
        let index = self.flushes_seen;
        self.flushes_seen += 1;
        if self.plan.fail_flush_index == Some(index) {
            self.tripped = true;
            return Err(BlockError::Injected("flush fault"));
        }
        self.inner.flush()
    }
}
