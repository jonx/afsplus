//! Optional live block-I/O activity notifications.
//!
//! This wrapper is deliberately separate from [`crate::BlockDevice`]. A
//! normal device therefore pays no branch, callback, allocation, or clock
//! cost. Front-ends which want a virtual drive LED opt in by wrapping their
//! device in [`ActivityBackend`].

use crate::{BlockDevice, BlockError};

/// The class of device operation in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityOperation {
    Read,
    Write,
    /// A durability barrier. Consumers normally count this as write activity.
    Flush,
}

/// Whether an operation is starting or has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityPhase {
    Begin,
    End { success: bool },
}

/// One allocation-free activity notification.
///
/// `lba` is `None` and `block_count` is zero for a flush. No data payload,
/// object name, timestamp, or filesystem metadata is copied into this event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityEvent {
    pub operation: ActivityOperation,
    pub phase: ActivityPhase,
    pub lba: Option<u64>,
    pub block_count: u64,
}

impl ActivityEvent {
    fn begin(operation: ActivityOperation, lba: Option<u64>, block_count: u64) -> Self {
        ActivityEvent {
            operation,
            phase: ActivityPhase::Begin,
            lba,
            block_count,
        }
    }

    fn end(
        operation: ActivityOperation,
        lba: Option<u64>,
        block_count: u64,
        success: bool,
    ) -> Self {
        ActivityEvent {
            operation,
            phase: ActivityPhase::End { success },
            lba,
            block_count,
        }
    }
}

/// Receives synchronous activity notifications.
///
/// The callback must stay fast and must not re-enter the same block device.
/// Queueing, timestamping, and visual pulse stretching belong in the consumer.
pub trait ActivitySink {
    /// Lets a sink suppress unneeded operation classes before event creation.
    fn is_enabled(&self, _operation: ActivityOperation) -> bool {
        true
    }

    fn on_activity(&mut self, event: ActivityEvent);
}

impl<F> ActivitySink for F
where
    F: FnMut(ActivityEvent),
{
    fn on_activity(&mut self, event: ActivityEvent) {
        self(event);
    }
}

/// A zero-allocation activity wrapper for a block device.
#[derive(Debug)]
pub struct ActivityBackend<D: BlockDevice, S: ActivitySink> {
    inner: D,
    sink: S,
}

impl<D: BlockDevice, S: ActivitySink> ActivityBackend<D, S> {
    pub fn new(inner: D, sink: S) -> Self {
        ActivityBackend { inner, sink }
    }

    pub fn inner(&self) -> &D {
        &self.inner
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn into_parts(self) -> (D, S) {
        (self.inner, self.sink)
    }

    fn notify_result<T>(
        &mut self,
        enabled: bool,
        operation: ActivityOperation,
        lba: Option<u64>,
        block_count: u64,
        result: Result<T, BlockError>,
    ) -> Result<T, BlockError> {
        if enabled {
            self.sink.on_activity(ActivityEvent::end(
                operation,
                lba,
                block_count,
                result.is_ok(),
            ));
        }
        result
    }

    fn notify_begin(
        &mut self,
        operation: ActivityOperation,
        lba: Option<u64>,
        block_count: u64,
    ) -> bool {
        let enabled = self.sink.is_enabled(operation);
        if enabled {
            self.sink
                .on_activity(ActivityEvent::begin(operation, lba, block_count));
        }
        enabled
    }
}

impl<D: BlockDevice, S: ActivitySink> BlockDevice for ActivityBackend<D, S> {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let operation = ActivityOperation::Read;
        let enabled = self.notify_begin(operation, Some(lba), 1);
        let result = self.inner.read_block(lba, buf);
        self.notify_result(enabled, operation, Some(lba), 1, result)
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        let operation = ActivityOperation::Write;
        let enabled = self.notify_begin(operation, Some(lba), 1);
        let result = self.inner.write_block(lba, data);
        self.notify_result(enabled, operation, Some(lba), 1, result)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let operation = ActivityOperation::Flush;
        let enabled = self.notify_begin(operation, None, 0);
        let result = self.inner.flush();
        self.notify_result(enabled, operation, None, 0, result)
    }
}
