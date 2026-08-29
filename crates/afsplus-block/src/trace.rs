//! Tracing/accounting wrapper.
//!
//! Records every device operation and accumulates I/O statistics so tests and
//! benchmarks can account for reads, writes, flushes and write amplification
//! from the first prototype onward (roadmap Stage A requirement).

use crate::{BlockDevice, BlockError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceEvent {
    Read { lba: u64 },
    Write { lba: u64 },
    Flush,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoStats {
    pub reads: u64,
    pub writes: u64,
    pub flushes: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
}

#[derive(Debug)]
pub struct TraceBackend<D: BlockDevice> {
    inner: D,
    events: Vec<TraceEvent>,
    stats: IoStats,
}

impl<D: BlockDevice> TraceBackend<D> {
    pub fn new(inner: D) -> Self {
        TraceBackend { inner, events: Vec::new(), stats: IoStats::default() }
    }

    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }

    pub fn stats(&self) -> IoStats {
        self.stats
    }

    pub fn reset(&mut self) {
        self.events.clear();
        self.stats = IoStats::default();
    }

    pub fn inner(&self) -> &D {
        &self.inner
    }

    pub fn into_inner(self) -> D {
        self.inner
    }
}

impl<D: BlockDevice> BlockDevice for TraceBackend<D> {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(lba, buf)?;
        self.events.push(TraceEvent::Read { lba });
        self.stats.reads += 1;
        self.stats.bytes_read += buf.len() as u64;
        Ok(())
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, data)?;
        self.events.push(TraceEvent::Write { lba });
        self.stats.writes += 1;
        self.stats.bytes_written += data.len() as u64;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()?;
        self.events.push(TraceEvent::Flush);
        self.stats.flushes += 1;
        Ok(())
    }
}
