//! Optional bounded diagnostics for the common checkpoint publication tail.
//!
//! These are runtime observations, not on-disk records or a durability oracle.
//! Ring construction allocates once; ring emission neither allocates nor reads
//! a clock. An optional trusted live adapter must obey the same bounded contract.

use std::collections::{TryReserveError, VecDeque};
use std::num::NonZeroUsize;

/// Categories emitted by the common commit tail. Other subsystem coverage has
/// separate integration gates; a category name alone is not that evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Transaction,
    Checkpoint,
    Io,
    Error,
}

/// Runtime selection, independent of ring capacity and event identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Categories(u8);

impl Categories {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(15);

    pub const fn with(self, category: Category) -> Self {
        Self(self.0 | (1 << category as u8))
    }

    pub const fn contains(self, category: Category) -> bool {
        self.0 & (1 << category as u8) != 0
    }
}

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

impl EventKind {
    pub const fn category(self) -> Category {
        match self {
            Self::Begin | Self::Adopted => Category::Transaction,
            Self::DataWritesComplete => Category::Io,
            Self::MetadataDurable | Self::PublicationBegin | Self::CheckpointDurable => {
                Category::Checkpoint
            }
            Self::Failed => Category::Error,
        }
    }
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

/// Outcome of a nonblocking live-adapter delivery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkResult {
    Accepted,
    /// This event was not accepted; future events may be attempted.
    Busy,
    /// Stop callbacks until the adapter is explicitly replaced.
    Closed,
}

/// Trusted synchronous adapter, called inside filesystem operations. It must
/// not block, allocate, unwind, reenter the filesystem or change its state.
/// Use a preallocated queue to dispatch arbitrary consumer code elsewhere.
/// Return Busy/Closed for ordinary transport failure; these are observations,
/// never filesystem errors. The core cannot enforce arbitrary host code's
/// execution-time or panic behaviour, just as for a BlockDevice implementation.
pub trait LiveSink: Send + Sync {
    fn try_event(&mut self, event: Event) -> SinkResult;
}

impl<F: FnMut(Event) -> SinkResult + Send + Sync> LiveSink for F {
    fn try_event(&mut self, event: Event) -> SinkResult {
        self(event)
    }
}

pub struct FlightRecorder {
    events: VecDeque<Event>,
    limit: usize,
    sequence: u64,
    attempt: u64,
    dropped: u64,
    categories: Categories,
    filtered: u64,
    sink: Option<Box<dyn LiveSink>>,
    sink_closed: bool,
    delivered: u64,
    missed: u64,
}

impl std::fmt::Debug for FlightRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlightRecorder")
            .field("events", &self.events)
            .field("limit", &self.limit)
            .field("sequence", &self.sequence)
            .field("attempt", &self.attempt)
            .field("dropped", &self.dropped)
            .field("categories", &self.categories)
            .field("filtered", &self.filtered)
            .field("sink_attached", &self.sink.is_some())
            .field("sink_closed", &self.sink_closed)
            .field("delivered", &self.delivered)
            .field("missed", &self.missed)
            .finish()
    }
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
            categories: Categories::ALL,
            filtered: 0,
            sink: None,
            sink_closed: false,
            delivered: 0,
            missed: 0,
        })
    }

    pub fn events(&self) -> impl ExactSizeIterator<Item = &Event> {
        self.events.iter()
    }

    /// Remove retained events without releasing storage or resetting identities
    /// and the cumulative dropped count. Consumed events cannot later count as
    /// overwritten records when a consumer captures the next operation.
    pub fn drain(&mut self) -> impl ExactSizeIterator<Item = Event> + '_ {
        self.events.drain(..)
    }

    pub fn capacity(&self) -> usize {
        self.limit
    }

    /// Overwritten records, or records refused after identifier exhaustion.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Change future admission only. Retained records and cumulative counters
    /// are preserved; filtered Begin events still advance attempt identities.
    pub fn set_categories(&mut self, categories: Categories) -> Categories {
        std::mem::replace(&mut self.categories, categories)
    }

    /// Last assigned identity, including events deliberately filtered out.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn attempt(&self) -> u64 {
        self.attempt
    }

    pub fn categories(&self) -> Categories {
        self.categories
    }

    /// Deliberately unrecorded events, separate from overwrites/exhaustion.
    pub fn filtered(&self) -> u64 {
        self.filtered
    }

    /// Replace the adapter without dropping it during emission. Counts are
    /// cumulative across replacement; only Closed state is cleared. None
    /// detaches live observation without changing local ring collection.
    pub fn replace_sink(&mut self, sink: Option<Box<dyn LiveSink>>) -> Option<Box<dyn LiveSink>> {
        self.sink_closed = false;
        std::mem::replace(&mut self.sink, sink)
    }

    pub fn sink_closed(&self) -> bool {
        self.sink_closed
    }

    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    /// Selected events not accepted while a live adapter is attached, including
    /// events following Closed. Detached and category-filtered events do not
    /// count. Local ring overwrites have their independent dropped counter.
    pub fn missed(&self) -> u64 {
        self.missed
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
        if !self.categories.contains(kind.category()) {
            self.filtered = self.filtered.saturating_add(1);
            return;
        }
        if self.events.len() == self.limit {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        let event = Event {
            sequence: self.sequence,
            attempt: self.attempt,
            generation,
            kind,
            requires_remount,
        };
        self.events.push_back(event);
        if let Some(sink) = &mut self.sink {
            let result = if self.sink_closed {
                SinkResult::Closed
            } else {
                sink.try_event(event)
            };
            match result {
                SinkResult::Accepted => self.delivered = self.delivered.saturating_add(1),
                SinkResult::Busy => self.missed = self.missed.saturating_add(1),
                SinkResult::Closed => {
                    self.missed = self.missed.saturating_add(1);
                    self.sink_closed = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod category_tests {
    use super::*;

    #[test]
    fn filtering_preserves_attempts_sequences_and_loss_accounting() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        let allocated = ring.events.capacity();
        assert_eq!(ring.set_categories(Categories::NONE), Categories::ALL);
        ring.emit(9, EventKind::Begin, false);
        ring.emit(9, EventKind::Failed, false);
        ring.set_categories(Categories::NONE.with(Category::Error));
        ring.emit(9, EventKind::Begin, false);
        ring.emit(9, EventKind::Failed, true);
        let last = *ring.events().last().unwrap();
        assert_eq!((last.sequence, last.attempt), (4, 2));
        assert!(last.requires_remount);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 0));
        ring.set_categories(Categories::ALL);
        ring.emit(9, EventKind::Begin, false);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 1));
        assert_eq!(ring.drain().count(), 1);
        ring.emit(9, EventKind::Adopted, false);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 1));
        assert_eq!(ring.events.capacity(), allocated);
        ring.set_categories(Categories::NONE);
        ring.sequence = u64::MAX;
        ring.emit(9, EventKind::Begin, false);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 2));
        assert_eq!(ring.events().last().unwrap().sequence, 6);
    }

    #[test]
    fn category_mapping_distinguishes_transaction_io_checkpoint_and_error() {
        for (category, kinds) in [
            (
                Category::Transaction,
                vec![EventKind::Begin, EventKind::Adopted],
            ),
            (Category::Io, vec![EventKind::DataWritesComplete]),
            (
                Category::Checkpoint,
                vec![
                    EventKind::MetadataDurable,
                    EventKind::PublicationBegin,
                    EventKind::CheckpointDurable,
                ],
            ),
            (Category::Error, vec![EventKind::Failed]),
        ] {
            let selected = Categories::NONE.with(category);
            for kind in kinds {
                assert_eq!(kind.category(), category);
                assert!(selected.contains(kind.category()));
                assert!(Categories::ALL.contains(kind.category()));
                assert!(!Categories::NONE.contains(kind.category()));
            }
        }
    }
}

#[cfg(test)]
mod sink_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn live_failures_are_counted_without_dropping_or_recalling_closed_adapter() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<FlightRecorder>();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.replace_sink(Some(Box::new(move |_event: Event| {
            match observed.fetch_add(1, Ordering::SeqCst) {
                0 => SinkResult::Accepted,
                1 => SinkResult::Busy,
                2 => SinkResult::Closed,
                _ => panic!("closed adapter called"),
            }
        })));
        for _ in 0..4 {
            ring.emit(1, EventKind::Begin, false);
            ring.emit(1, EventKind::Adopted, false);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!((ring.delivered(), ring.missed(), ring.dropped()), (1, 7, 7));
        assert!(ring.sink_closed());
        ring.set_categories(Categories::NONE);
        ring.emit(2, EventKind::Begin, false);
        assert_eq!((ring.filtered(), ring.missed()), (1, 7));
        let old = ring.replace_sink(None);
        assert!(old.is_some());
        assert!(!ring.sink_closed());
        ring.set_categories(Categories::ALL);
        ring.emit(2, EventKind::Adopted, false);
        assert_eq!((ring.delivered(), ring.missed()), (1, 7));
        ring.replace_sink(Some(Box::new(|_: Event| SinkResult::Accepted)));
        ring.emit(3, EventKind::Begin, false);
        assert_eq!((ring.delivered(), ring.missed()), (2, 7));
        ring.sequence = u64::MAX;
        ring.emit(3, EventKind::Adopted, false);
        assert_eq!((ring.delivered(), ring.missed()), (2, 7));
    }
}
