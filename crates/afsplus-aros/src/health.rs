//! Queryable health of a mounted handler: counters, degraded-state flags and
//! a bounded event ring with an explicit loss counter.
//!
//! Errors reach a DOS caller as one `IoErr()` value and are then gone. The
//! log keeps the ones that describe the volume or its device, so a tool can
//! ask a running handler instead of reading a console.

use std::collections::VecDeque;

use afsplus_core::MountMode;
use afsplus_vfs::HealthNote;

use crate::ArosError;

/// A failed device read, write or barrier was observed since mount.
pub const HEALTH_DEVICE_ERROR: u32 = 1 << 0;
/// A structure failed validation since mount.
pub const HEALTH_CORRUPTION: u32 = 1 << 1;
/// The mount exposes a checkpoint while intent-log records await replay.
pub const HEALTH_REPLAY_PENDING: u32 = 1 << 2;
/// An operation ended by an internal fault instead of a result.
pub const HEALTH_INTERNAL_FAULT: u32 = 1 << 3;

/// Numbers are the wire identities of `api/afsplus_aros.h`: appended, never
/// reused, never renumbered.
///
/// The last three describe the volume without failing anything, so they
/// carry no `IoErr()` value and set no degraded-state flag. They are the
/// events of [docs/26 section 17] that no call can report: a caller asks for
/// a file and gets it, and nobody would ever learn that the mount had to
/// skip the newer checkpoint of the pair.
///
/// [docs/26 section 17]: ../../../docs/26-debug-observability.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HealthEventKind {
    DeviceError = 1,
    Corruption = 2,
    NoSpace = 3,
    InternalFault = 4,
    /// The newer checkpoint of the A/B pair was unusable at mount and the
    /// volume reads the older one: the last commit before this mount is not
    /// in what it shows.
    CheckpointFallback = 5,
    /// Retired space waiting to come back crossed the volume's threshold.
    /// Raised once per crossing, never once per commit.
    ReclaimBacklogHigh = 6,
    /// The mounted checkpoint's free-block total disagrees with the sum over
    /// its own allocation-root region records.
    RegionFreeCountMismatch = 7,
}

const KINDS: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthEvent {
    /// Counts every recorded event since mount, including dropped ones, so a
    /// gap in drained sequences is visible loss.
    pub sequence: u64,
    pub kind: HealthEventKind,
    pub dos_error: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub mount_mode: MountMode,
    pub flags: u32,
    pub generation: u64,
    pub pending_intent_records: u32,
    pub pending_orphans: u64,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub available_blocks: u64,
    pub device_errors: u64,
    pub corruption_errors: u64,
    pub no_space_errors: u64,
    pub internal_faults: u64,
    pub checkpoint_fallbacks: u64,
    pub reclaim_backlog_highs: u64,
    pub free_count_mismatches: u64,
    pub events_recorded: u64,
    pub events_dropped: u64,
    /// `IoErr()` value of the most recent recorded event that carries one,
    /// zero when none. An event that failed no call carries none and leaves
    /// this as it was.
    pub last_error: i32,
}

#[derive(Debug)]
pub struct HealthLog {
    events: VecDeque<HealthEvent>,
    capacity: usize,
    recorded: u64,
    dropped: u64,
    counts: [u64; KINDS],
    last_error: i32,
}

impl HealthLog {
    /// A zero capacity keeps the counters and drops every event.
    pub fn new(capacity: usize) -> Self {
        HealthLog {
            events: VecDeque::with_capacity(capacity),
            capacity,
            recorded: 0,
            dropped: 0,
            counts: [0; KINDS],
            last_error: 0,
        }
    }

    /// Records `error` when it describes the volume or its device. Ordinary
    /// results such as a missing object are not health events.
    pub fn record(&mut self, error: ArosError) {
        let kind = match error {
            ArosError::Unknown => HealthEventKind::DeviceError,
            ArosError::NotDosDisk => HealthEventKind::Corruption,
            ArosError::DiskFull => HealthEventKind::NoSpace,
            _ => return,
        };
        self.push(kind, error.io_error());
    }

    /// Records what the VFS observed about the volume. Nothing failed, so
    /// these carry no `IoErr()` value.
    pub fn record_note(&mut self, note: HealthNote) {
        self.push(
            match note {
                HealthNote::CheckpointFallback => HealthEventKind::CheckpointFallback,
                HealthNote::ReclaimBacklogHigh => HealthEventKind::ReclaimBacklogHigh,
                HealthNote::RegionFreeCountMismatch => HealthEventKind::RegionFreeCountMismatch,
            },
            0,
        );
    }

    pub fn record_internal_fault(&mut self) {
        self.push(
            HealthEventKind::InternalFault,
            ArosError::Unknown.io_error(),
        );
    }

    fn push(&mut self, kind: HealthEventKind, dos_error: i32) {
        self.recorded += 1;
        self.counts[kind as usize - 1] += 1;
        if dos_error != 0 {
            self.last_error = dos_error;
        }
        if self.capacity == 0 {
            self.dropped += 1;
            return;
        }
        if self.events.len() == self.capacity {
            // The oldest event gives way: the newest state matters most.
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(HealthEvent {
            sequence: self.recorded,
            kind,
            dos_error,
        });
    }

    /// Moves the oldest retained events into `output`.
    pub fn drain(&mut self, output: &mut [HealthEvent]) -> usize {
        let count = output.len().min(self.events.len());
        for slot in &mut output[..count] {
            *slot = self.events.pop_front().expect("counted");
        }
        count
    }

    pub(crate) fn fill(&self, snapshot: &mut HealthSnapshot) {
        snapshot.device_errors = self.counts[0];
        snapshot.corruption_errors = self.counts[1];
        snapshot.no_space_errors = self.counts[2];
        snapshot.internal_faults = self.counts[3];
        snapshot.checkpoint_fallbacks = self.counts[4];
        snapshot.reclaim_backlog_highs = self.counts[5];
        snapshot.free_count_mismatches = self.counts[6];
        snapshot.events_recorded = self.recorded;
        snapshot.events_dropped = self.dropped;
        snapshot.last_error = self.last_error;
        if self.counts[0] != 0 {
            snapshot.flags |= HEALTH_DEVICE_ERROR;
        }
        if self.counts[1] != 0 {
            snapshot.flags |= HEALTH_CORRUPTION;
        }
        if self.counts[3] != 0 {
            snapshot.flags |= HEALTH_INTERNAL_FAULT;
        }
    }
}
