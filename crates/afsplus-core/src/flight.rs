//! Optional bounded diagnostics for the common checkpoint publication tail.
//!
//! These are runtime observations, not on-disk records or a durability oracle.
//! Construction allocates once; emission neither allocates nor reads a clock.

use std::collections::{TryReserveError, VecDeque};
use std::num::NonZeroUsize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Begin,
    /// The tail's queued data writes and their conditional barrier completed.
    /// Earlier writes and intent-log durability have separate owners.
    DataWritesComplete,
    MetadataDurable,
    PublicationBegin,
    CheckpointDurable,
    Adopted,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emission_keeps_allocation_and_stops_before_identity_reuse() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        let allocated = ring.events.capacity();
        for _ in 0..1000 {
            ring.emit(9, EventKind::Begin, false);
            ring.emit(9, EventKind::Failed, false);
        }
        assert_eq!(ring.events.capacity(), allocated);
        assert_eq!(ring.events.len(), 1);
        assert_eq!(ring.dropped(), 1999);
        ring.attempt = u64::MAX;
        let last = *ring.events.back().unwrap();
        ring.emit(9, EventKind::Begin, false);
        ring.emit(9, EventKind::Adopted, false);
        assert_eq!(*ring.events.back().unwrap(), last);
        assert_eq!(ring.dropped(), 2001);

        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.sequence = u64::MAX - 1;
        ring.emit(1, EventKind::Begin, false);
        ring.emit(1, EventKind::Adopted, false);
        assert_eq!(ring.events.back().unwrap().sequence, u64::MAX);
        assert_eq!(ring.events.back().unwrap().kind, EventKind::Begin);
        assert_eq!(ring.dropped(), 1);
    }
}

/// An attempt is unique within this recorder, including retries that reuse a
/// checkpoint generation. It starts at the common commit tail, not API entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub attempt: u64,
    pub generation: u64,
    pub kind: EventKind,
    /// For `Failed`, publication may have started and remount is required.
    /// This does not assert that the checkpoint reached durable storage.
    pub requires_remount: bool,
}

#[derive(Debug)]
pub struct FlightRecorder {
    events: VecDeque<Event>,
    limit: usize,
    sequence: u64,
    attempt: u64,
    dropped: u64,
}

impl FlightRecorder {
    pub fn new(capacity: NonZeroUsize) -> Result<Self, TryReserveError> {
        let mut events = VecDeque::new();
        events.try_reserve_exact(capacity.get())?;
        Ok(Self {
            events,
            limit: capacity.get(),
            sequence: 0,
            attempt: 0,
            dropped: 0,
        })
    }

    pub fn events(&self) -> impl ExactSizeIterator<Item = &Event> {
        self.events.iter()
    }

    pub fn capacity(&self) -> usize {
        self.limit
    }

    /// Overwritten records, or records refused after identifier exhaustion.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub(crate) fn emit(&mut self, generation: u64, kind: EventKind, requires_remount: bool) {
        if self.sequence == u64::MAX || (kind == EventKind::Begin && self.attempt == u64::MAX) {
            self.dropped = self.dropped.saturating_add(1);
            // Refuse all following events rather than reuse an identity.
            self.sequence = u64::MAX;
            return;
        }
        if kind == EventKind::Begin {
            self.attempt += 1;
        }
        self.sequence += 1;
        if self.events.len() == self.limit {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(Event {
            sequence: self.sequence,
            attempt: self.attempt,
            generation,
            kind,
            requires_remount,
        });
    }
}
