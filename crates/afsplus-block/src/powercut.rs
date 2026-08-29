//! Power-cut simulation (roadmap Stage A: "power-cut capable block backend").
//!
//! [`RecordingBackend`] captures the exact sequence of writes and flush
//! barriers a workload performs. [`crash_states`] then materializes, for a
//! given crash point, every *full-write subset* of the unflushed tail plus a
//! set of representative torn-write states:
//!
//! - writes before the last completed flush are durable, in order
//! - each write after the last flush may independently be applied or lost;
//!   all such subsets are enumerated, covering loss and device reordering
//!   at whole-block granularity
//! - in-order tear states: each unflushed write applied only partially (a
//!   prefix of the block, at a few representative offsets), with all
//!   earlier unflushed writes applied
//!
//! This is deliberately not "every physically possible state": tears are
//! sampled at fixed offsets and at most one write is torn per state, and
//! sub-block reordering or interleaved tears are not modeled. Checksums make
//! finer-grained tears equivalent to the sampled ones for AFS+ metadata, but
//! the model should be restated, not oversold.
//!
//! Crashing *during* a flush is equivalent to crashing just before it with an
//! arbitrary subset of the pending writes durable, which the enumeration at
//! the previous crash point already covers.

use crate::{BlockDevice, BlockError, MemoryBackend};

#[derive(Debug, Clone)]
pub enum RecordedOp {
    Write { lba: u64, data: Vec<u8> },
    Flush,
}

/// Wraps a device, recording writes (with payloads) and flush barriers.
#[derive(Debug)]
pub struct RecordingBackend<D: BlockDevice> {
    inner: D,
    log: Vec<RecordedOp>,
}

impl<D: BlockDevice> RecordingBackend<D> {
    pub fn new(inner: D) -> Self {
        RecordingBackend { inner, log: Vec::new() }
    }

    pub fn log(&self) -> &[RecordedOp] {
        &self.log
    }

    pub fn into_parts(self) -> (D, Vec<RecordedOp>) {
        (self.inner, self.log)
    }
}

impl<D: BlockDevice> BlockDevice for RecordingBackend<D> {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(lba, buf)
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, data)?;
        self.log.push(RecordedOp::Write { lba, data: data.to_vec() });
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()?;
        self.log.push(RecordedOp::Flush);
        Ok(())
    }
}

/// One durable image the crash model produces for a power cut.
pub struct CrashState {
    pub image: MemoryBackend,
    /// Human-readable description for failure diagnostics.
    pub description: String,
}

/// Prefix-of-block tear offsets exercised for each unflushed write.
const TEAR_OFFSETS: &[usize] = &[64, 2048, 4064];

/// Full subset enumeration is used up to this tail length (2^n states);
/// longer tails would need sampling, which the first prototype does not
/// require. Guarded by an assertion so a silent coverage loss cannot happen.
const MAX_ENUMERATED_TAIL: usize = 12;

/// Enumerates the modeled durable states (full-write subsets plus
/// representative tears; see the module docs) when power is lost immediately
/// after `log[..crash_point]` has been issued (`crash_point` in
/// `0..=log.len()`), starting from the durable image `base`.
pub fn crash_states(base: &MemoryBackend, log: &[RecordedOp], crash_point: usize) -> Vec<CrashState> {
    assert!(crash_point <= log.len());
    let prefix = &log[..crash_point];

    // Split at the last completed flush barrier.
    let last_flush = prefix
        .iter()
        .rposition(|op| matches!(op, RecordedOp::Flush))
        .map(|i| i + 1)
        .unwrap_or(0);

    let mut durable = base.clone();
    for op in &prefix[..last_flush] {
        if let RecordedOp::Write { lba, data } = op {
            durable.apply_raw(*lba, data);
        }
    }

    let tail: Vec<(u64, &Vec<u8>)> = prefix[last_flush..]
        .iter()
        .filter_map(|op| match op {
            RecordedOp::Write { lba, data } => Some((*lba, data)),
            RecordedOp::Flush => None,
        })
        .collect();
    assert!(
        tail.len() <= MAX_ENUMERATED_TAIL,
        "unflushed tail of {} writes exceeds full-enumeration budget; \
         the harness needs a sampling strategy before testing this workload",
        tail.len()
    );

    let mut states = Vec::new();

    // Every subset of the unflushed tail (covers loss and reordering).
    for subset in 0u64..(1u64 << tail.len()) {
        let mut image = durable.clone();
        for (i, (lba, data)) in tail.iter().enumerate() {
            if subset & (1 << i) != 0 {
                image.apply_raw(*lba, data);
            }
        }
        states.push(CrashState {
            image,
            description: format!(
                "crash at op {crash_point}: unflushed subset {subset:#b} of {} writes",
                tail.len()
            ),
        });
    }

    // In-order tears: earlier unflushed writes applied, write `i` torn.
    for (i, (lba, data)) in tail.iter().enumerate() {
        for &tear in TEAR_OFFSETS {
            if tear >= data.len() {
                continue;
            }
            let mut image = durable.clone();
            for (lba_j, data_j) in &tail[..i] {
                image.apply_raw(*lba_j, data_j);
            }
            let mut torn = image.peek(*lba);
            torn[..tear].copy_from_slice(&data[..tear]);
            image.apply_raw(*lba, &torn);
            states.push(CrashState {
                image,
                description: format!(
                    "crash at op {crash_point}: unflushed write {i} (lba {lba}) torn at byte {tear}"
                ),
            });
        }
    }

    states
}
