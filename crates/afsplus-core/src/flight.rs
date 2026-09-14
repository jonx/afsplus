//! Optional bounded diagnostics for core API calls and checkpoint publication.
//!
//! These are runtime observations, not on-disk records or a durability oracle.
//! Ring construction allocates once; ring emission neither allocates nor reads
//! a clock. An optional trusted live adapter must obey the same bounded contract.

use std::collections::{TryReserveError, VecDeque};
use std::num::NonZeroUsize;

/// Categories emitted by API observation and the commit tail. Other coverage has
/// separate integration gates; a category name alone is not that evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Transaction,
    Checkpoint,
    Io,
    Error,
    Api,
    Window,
}

/// Runtime selection, independent of ring capacity and event identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Categories(u8);

impl Categories {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(63);

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
    ApiBegin,
    ApiSucceeded,
    ApiFailed,
    ApiUnwound,
    WindowOpened,
    WindowAttached,
    WindowLogBegin,
    WindowLogDurable,
    WindowLogFailed,
    WindowFailed,
    WindowClosed,
    WindowDetached,
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
            Self::ApiBegin | Self::ApiSucceeded | Self::ApiFailed | Self::ApiUnwound => {
                Category::Api
            }
            Self::WindowOpened
            | Self::WindowAttached
            | Self::WindowLogBegin
            | Self::WindowLogDurable
            | Self::WindowLogFailed
            | Self::WindowFailed
            | Self::WindowClosed
            | Self::WindowDetached => Category::Window,
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

/// Core API method identities. Append new IDs; never renumber or reuse them.
/// These are diagnostic identifiers, not filesystem API v2 ABI ordinals.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiMethod {
    CleanupOrphan = 1,
    CloneFile = 2,
    CloneRange = 3,
    CreateDirectory = 4,
    CreateDirectoryInRoot = 5,
    CreateFileInDirectory = 6,
    CreateFileInRoot = 7,
    CreateSymlink = 8,
    DeleteFile = 9,
    DeleteFileInRoot = 10,
    FileAllocationPage = 11,
    FileDataPolicy = 12,
    FirstOrphan = 13,
    LinkFile = 14,
    ListDirectory = 15,
    ListRoot = 16,
    LookupInDirectory = 17,
    LookupRoot = 18,
    OrphanCount = 19,
    OrphanFile = 20,
    OrphanObject = 21,
    PreallocateFile = 22,
    PreallocateFileBounded = 23,
    QuarantineContains = 24,
    ReadDirectoryPage = 25,
    ReadFile = 26,
    ReadFileAt = 27,
    ReadLink = 28,
    ReclaimStep = 29,
    RemoveDirectory = 30,
    Rename = 31,
    RenameReplace = 32,
    RenameReplaceOrphanTarget = 33,
    RestoreObjectMetadata = 34,
    RunBatch = 35,
    SetDataUpdatePolicy = 36,
    SetFileDataPolicy = 37,
    SetObjectProtection = 38,
    SetOrphanCleanupExtentBudget = 39,
    SetReclaimBatchBlocks = 40,
    SetSnapshotWorkLimits = 41,
    SetTreeCachePages = 42,
    SnapshotAllocationPage = 43,
    SnapshotCreate = 44,
    SnapshotDelete = 45,
    SnapshotList = 46,
    SnapshotLookup = 47,
    SnapshotMaintenanceStep = 48,
    SnapshotOpen = 49,
    SnapshotReadDirectoryPage = 50,
    SnapshotReadFileAt = 51,
    SnapshotReadLink = 52,
    SnapshotStat = 53,
    Stat = 54,
    Sync = 55,
    TruncateFile = 56,
    TruncateFileBounded = 57,
    UnlinkSymlink = 58,
    VisibleMetadata = 59,
    WindowCommit = 60,
    WindowFsync = 61,
    WindowOp = 62,
    WindowTruncateFile = 63,
    WindowWriteFileAt = 64,
    WriteFileAt = 65,
    WriteFileAtBounded = 66,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApiContext {
    pub operation: u64,
    pub span: u64,
    pub parent_span: u64,
    pub method: Option<ApiMethod>,
}

pub(crate) struct ApiToken {
    previous: ApiContext,
    active: ApiContext,
}

#[derive(Clone, Copy)]
pub(crate) enum ApiOutcome {
    Succeeded,
    Failed,
    Unwound,
}

/// An attempt is unique within this recorder, including retries that reuse a
/// checkpoint generation. It starts at the common commit tail, not API entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub attempt: u64,
    pub generation: u64,
    pub kind: EventKind,
    /// The volume requires remount after unsafe window mutation or publication.
    /// API events sample this flag at entry/exit; it is not a durability verdict
    /// or a claim that unwinding restored all in-memory filesystem state.
    pub requires_remount: bool,
    /// Zero context denotes no observed API (including legacy commit-only scope).
    pub api: ApiContext,
    /// Recorder-local deferred-window identity, zero outside observed windows.
    pub window: u64,
    /// Last observed durable group, except WindowLogBegin/WindowLogFailed
    /// identify the attempted group. Zero means no group is represented.
    pub log_sequence: u32,
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
    api_enabled: bool,
    api_next: u64,
    api_context: ApiContext,
    api_exhausted: bool,
    window_next: u64,
    window: u64,
    window_log_sequence: u32,
    window_exhausted: bool,
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
            .field("api_enabled", &self.api_enabled)
            .field("api_context", &self.api_context)
            .field("api_exhausted", &self.api_exhausted)
            .field("window", &self.window)
            .field("window_log_sequence", &self.window_log_sequence)
            .field("window_exhausted", &self.window_exhausted)
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
            api_enabled: false,
            api_next: 0,
            api_context: ApiContext::default(),
            api_exhausted: false,
            window_next: 0,
            window: 0,
            window_log_sequence: 0,
            window_exhausted: false,
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

    /// Opt in to core API and deferred-window spans. Commit-only profiles do not enable
    /// this scope, preserving their event sequences and historical wire bytes.
    pub fn enable_api_observation(&mut self) {
        self.api_enabled = true;
    }

    pub fn api_observation_enabled(&self) -> bool {
        self.api_enabled
    }

    pub(crate) fn observe_window(
        &mut self,
        generation: u64,
        log_sequence: u32,
        attached: bool,
        requires_remount: bool,
    ) {
        if !self.api_enabled {
            return;
        }
        if self.window != 0 {
            if attached {
                return;
            }
            // A fresh engine window cannot inherit a prior observation whose
            // final event was interrupted (for example by provider unwinding).
            self.window_event(
                generation,
                EventKind::WindowDetached,
                None,
                requires_remount,
            );
        }
        if self.window_next == u64::MAX || self.window_exhausted {
            self.window_exhausted = true;
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.window_next += 1;
        self.window = self.window_next;
        self.window_log_sequence = log_sequence;
        self.emit(
            generation,
            if attached {
                EventKind::WindowAttached
            } else {
                EventKind::WindowOpened
            },
            requires_remount,
        );
    }

    pub(crate) fn window_event(
        &mut self,
        generation: u64,
        kind: EventKind,
        attempted_sequence: Option<u32>,
        requires_remount: bool,
    ) {
        if !self.api_enabled || self.window == 0 {
            return;
        }
        let previous_sequence = self.window_log_sequence;
        if let Some(sequence) = attempted_sequence {
            self.window_log_sequence = sequence;
        }
        self.emit(generation, kind, requires_remount);
        if kind != EventKind::WindowLogDurable {
            self.window_log_sequence = previous_sequence;
        }
        if matches!(kind, EventKind::WindowClosed | EventKind::WindowDetached) {
            self.window = 0;
            self.window_log_sequence = 0;
        }
    }

    pub(crate) fn begin_api(
        &mut self,
        method: ApiMethod,
        generation: u64,
        requires_remount: bool,
    ) -> Option<ApiToken> {
        if !self.api_enabled {
            return None;
        }
        if self.api_next == u64::MAX || self.api_exhausted {
            self.api_exhausted = true;
            self.dropped = self.dropped.saturating_add(1);
            return None;
        }
        self.api_next += 1;
        let previous = self.api_context;
        self.api_context = ApiContext {
            operation: if previous.span == 0 {
                self.api_next
            } else {
                previous.operation
            },
            span: self.api_next,
            parent_span: previous.span,
            method: Some(method),
        };
        let token = ApiToken {
            previous,
            active: self.api_context,
        };
        self.emit(generation, EventKind::ApiBegin, requires_remount);
        Some(token)
    }

    pub(crate) fn end_api(
        &mut self,
        token: ApiToken,
        generation: u64,
        outcome: ApiOutcome,
        requires_remount: bool,
    ) {
        if self.api_context != token.active {
            // Never attribute a stale guard to a different active scope.
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.emit(
            generation,
            match outcome {
                ApiOutcome::Succeeded => EventKind::ApiSucceeded,
                ApiOutcome::Failed => EventKind::ApiFailed,
                ApiOutcome::Unwound => EventKind::ApiUnwound,
            },
            requires_remount,
        );
        self.api_context = token.previous;
    }

    pub(crate) fn emit(&mut self, generation: u64, kind: EventKind, requires_remount: bool) {
        if self.api_exhausted
            || self.window_exhausted
            || self.sequence == u64::MAX
            || (kind == EventKind::Begin && self.attempt == u64::MAX)
        {
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
            attempt: if matches!(kind.category(), Category::Api | Category::Window) {
                0
            } else {
                self.attempt
            },
            generation,
            kind,
            requires_remount,
            api: self.api_context,
            window: self.window,
            log_sequence: self.window_log_sequence,
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

#[cfg(test)]
mod api_tests {
    use super::*;

    #[test]
    fn nested_api_spans_share_a_request_and_bind_only_commits_to_attempts() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(32).unwrap()).unwrap();
        assert!(ring.begin_api(ApiMethod::Sync, 1, false).is_none());
        assert_eq!(ring.sequence(), 0);
        ring.enable_api_observation();
        let outer = ring
            .begin_api(ApiMethod::CreateFileInRoot, 1, false)
            .unwrap();
        let inner = ring
            .begin_api(ApiMethod::CreateFileInDirectory, 1, false)
            .unwrap();
        ring.emit(2, EventKind::Begin, false);
        ring.emit(2, EventKind::Adopted, false);
        ring.end_api(inner, 2, ApiOutcome::Succeeded, false);
        ring.end_api(outer, 2, ApiOutcome::Failed, false);
        let events: Vec<_> = ring.events().copied().collect();
        assert!(events.iter().all(|e| e.api.operation == 1));
        assert_eq!((events[1].api.span, events[1].api.parent_span), (2, 1));
        assert_eq!(events[2].api, events[1].api);
        assert_eq!((events[2].attempt, events[3].attempt), (1, 1));
        assert_eq!(events[5].api.span, 1);
        assert_eq!(events[5].kind, EventKind::ApiFailed);
        assert_eq!(events[5].attempt, 0);
        assert_eq!(ring.api_context, ApiContext::default());
        let next = ring.begin_api(ApiMethod::Stat, 2, false).unwrap();
        assert_eq!(
            (ring.api_context.operation, ring.api_context.parent_span),
            (3, 0)
        );
        ring.end_api(next, 2, ApiOutcome::Succeeded, false);
    }

    #[test]
    fn filtered_api_spans_keep_context_and_exhaustion_never_reuses_it() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.enable_api_observation();
        ring.set_categories(Categories::NONE.with(Category::Checkpoint));
        let token = ring.begin_api(ApiMethod::Sync, 1, false).unwrap();
        ring.emit(2, EventKind::Begin, false);
        ring.emit(2, EventKind::CheckpointDurable, false);
        ring.end_api(token, 2, ApiOutcome::Succeeded, false);
        let event = *ring.events().last().unwrap();
        assert_eq!(
            (event.api.operation, event.api.span, event.sequence),
            (1, 1, 3)
        );
        assert_eq!(ring.filtered(), 3);
        assert_eq!(ring.dropped(), 0);
        ring.api_next = u64::MAX;
        assert!(ring.begin_api(ApiMethod::Sync, 2, false).is_none());
        ring.emit(3, EventKind::Begin, false);
        assert_eq!(*ring.events().last().unwrap(), event);
        assert_eq!(ring.dropped(), 2);
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn window_context_survives_filtering_and_stops_before_identity_reuse() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(16).unwrap()).unwrap();
        ring.observe_window(2, 0, false, false);
        assert_eq!((ring.window, ring.sequence()), (0, 0));
        ring.enable_api_observation();
        ring.set_categories(Categories::NONE.with(Category::Checkpoint));
        ring.observe_window(2, 0, false, false);
        ring.window_event(2, EventKind::WindowLogBegin, Some(1), false);
        assert_eq!(ring.window_log_sequence, 0);
        ring.window_event(2, EventKind::WindowLogDurable, Some(1), false);
        ring.emit(2, EventKind::Begin, false);
        ring.emit(2, EventKind::CheckpointDurable, false);
        ring.window_event(2, EventKind::WindowClosed, None, false);
        assert_eq!((ring.window, ring.window_log_sequence), (0, 0));
        let event = *ring.events().next().unwrap();
        assert_eq!((event.window, event.log_sequence, event.attempt), (1, 1, 1));
        assert_eq!((ring.filtered(), ring.dropped()), (5, 0));
        ring.window_next = u64::MAX;
        ring.observe_window(3, 0, false, false);
        ring.emit(3, EventKind::Begin, false);
        assert_eq!(ring.dropped(), 2);
        assert_eq!(ring.events().len(), 1);
        assert_eq!(ring.window, 0);
    }

    #[test]
    fn fresh_window_detaches_an_interrupted_context_and_failed_group_is_not_acknowledged() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(16).unwrap()).unwrap();
        ring.enable_api_observation();
        ring.observe_window(2, 3, true, false);
        ring.window_event(2, EventKind::WindowLogFailed, Some(4), true);
        assert_eq!(ring.window_log_sequence, 3);
        assert_eq!(ring.events().last().unwrap().log_sequence, 4);
        ring.observe_window(2, 0, false, false);
        let events: Vec<_> = ring.events().copied().collect();
        assert_eq!(events[2].kind, EventKind::WindowDetached);
        assert_eq!((events[2].window, events[2].log_sequence), (1, 3));
        assert_eq!(events[3].kind, EventKind::WindowOpened);
        assert_eq!((events[3].window, events[3].log_sequence), (2, 0));
    }
}
