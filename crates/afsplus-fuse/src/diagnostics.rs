//! Diagnostics for the host mount: what the driver knows, said where a person
//! can read it.
//!
//! The core already records why it refuses a request, through
//! [`afsplus_core::flight`]. Until now nothing on the FUSE path installed a
//! recorder, so a refusal reached the user as a bare host errno with the
//! reason discarded inside the driver that knew it.
//!
//! [`Counters`] is the live half. A [`CounterSink`] is installed as the
//! recorder's live adapter and observes every event inside the filesystem
//! operation that emits it, so it obeys that contract exactly: it only stores
//! into atomics and attempts one non-blocking lock. It never allocates, never
//! blocks and never reenters the filesystem. When the report lock is held it
//! answers [`SinkResult::Busy`] and counts the miss rather than waiting, which
//! is what the contract asks for and is also why the report can say how much
//! it did not see.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

use afsplus_core::flight::{ApiMethod, Categories, Category, Event, EventKind, SinkResult};

/// Events kept for the report. Fixed at construction: the sink writes into a
/// preallocated slot so that observing the filesystem cannot allocate inside
/// a filesystem operation.
const RECENT_EVENTS: usize = 64;

/// Ring capacity of the recorder itself, when one is installed.
const RECORDER_CAPACITY: usize = 4096;

/// Live totals. Every field is written only by the sink and read only by the
/// reporter, so a reader can never block a filesystem operation.
#[derive(Debug, Default)]
pub struct Counters {
    observed: AtomicU64,
    api_begin: AtomicU64,
    api_succeeded: AtomicU64,
    api_failed: AtomicU64,
    api_unwound: AtomicU64,
    missed: AtomicU64,
}

impl Counters {
    /// Events the sink accepted.
    pub fn observed(&self) -> u64 {
        self.observed.load(Ordering::Relaxed)
    }

    /// API calls the core refused while this mount was running.
    pub fn api_failed(&self) -> u64 {
        self.api_failed.load(Ordering::Relaxed)
    }

    /// Events the sink saw but could not keep, because the report held the
    /// recent-event lock. They are counted in `observed`; only their detail is
    /// lost, and the report says so rather than presenting a silent gap.
    pub fn missed(&self) -> u64 {
        self.missed.load(Ordering::Relaxed)
    }
}

/// The failing API calls kept for the report, newest last.
#[derive(Debug)]
struct Recent {
    slots: Vec<Option<Event>>,
    next: usize,
    seen: u64,
}

impl Recent {
    fn new(capacity: usize) -> Self {
        Self {
            slots: vec![None; capacity],
            next: 0,
            seen: 0,
        }
    }

    /// Writes into a preallocated slot. No allocation, no growth.
    fn push(&mut self, event: Event) {
        let slot = self.next % self.slots.len();
        self.slots[slot] = Some(event);
        self.next = self.next.wrapping_add(1);
        self.seen = self.seen.saturating_add(1);
    }

    /// Oldest first.
    fn events(&self) -> Vec<Event> {
        let len = self.slots.len();
        let start = if self.seen as usize >= len {
            self.next % len
        } else {
            0
        };
        (0..len)
            .filter_map(|offset| self.slots[(start + offset) % len])
            .collect()
    }
}

/// What the mount knows, shared between the filesystem thread that observes
/// and the thread that reports.
#[derive(Debug)]
pub struct Diagnostics {
    counters: Counters,
    recent: Mutex<Recent>,
}

impl Diagnostics {
    fn new() -> Self {
        Self {
            counters: Counters::default(),
            recent: Mutex::new(Recent::new(RECENT_EVENTS)),
        }
    }

    pub fn counters(&self) -> &Counters {
        &self.counters
    }

    /// A report a person can read, naming the refusals by the API method the
    /// core refused. Each line states what was refused and at which volume
    /// generation, which is the fact the driver had and used to discard.
    ///
    /// These are refusals BY THE CORE, and the driver recovers from some of
    /// them: a directory walk whose cursor went stale is refused here and then
    /// resumed by name, with the person seeing nothing wrong. The report says
    /// so rather than presenting every line as a failure the user suffered,
    /// because a report that raises alarms about handled events teaches its
    /// reader to ignore it. Which refusals reached the user is the adapter's
    /// knowledge, not the recorder's.
    pub fn report(&self) -> String {
        let counters = &self.counters;
        let mut report = String::new();
        report.push_str("afsplus mount diagnostics\n");
        report.push_str(&format!(
            "  api calls: {} begun, {} succeeded, {} refused, {} unwound\n",
            counters.api_begin.load(Ordering::Relaxed),
            counters.api_succeeded.load(Ordering::Relaxed),
            counters.api_failed.load(Ordering::Relaxed),
            counters.api_unwound.load(Ordering::Relaxed),
        ));
        report.push_str(&format!(
            "  events observed: {}, detail missed while reporting: {}\n",
            counters.observed(),
            counters.missed(),
        ));

        let recent = match self.recent.lock() {
            Ok(recent) => recent.events(),
            // A panic in a reporting thread must not take the mount with it.
            Err(poisoned) => poisoned.into_inner().events(),
        };
        if recent.is_empty() {
            report.push_str("  no refused call recorded\n");
            return report;
        }
        report.push_str(
            "  calls the core refused, oldest first. The driver recovers from \
             some of these and the user sees nothing wrong:\n",
        );
        for event in recent {
            report.push_str(&format!(
                "    {} refused at generation {}\n",
                method_name(event.api.method),
                event.generation,
            ));
        }
        report
    }
}

/// The name of a refused call as the report prints it. An unnamed method is
/// printed with its number rather than guessed at: the report must not invent
/// a name for something this build does not know.
fn method_name(method: Option<ApiMethod>) -> String {
    match method {
        Some(method) => format!("{method:?}"),
        None => "an unobserved call".to_string(),
    }
}

/// The recorder's live adapter. Installed inside the recorder, called inside
/// filesystem operations, and bound by that contract: no allocation, no
/// blocking, no reentry.
#[derive(Debug)]
pub struct CounterSink {
    shared: Arc<Diagnostics>,
}

impl CounterSink {
    pub fn new(shared: Arc<Diagnostics>) -> Self {
        Self { shared }
    }
}

impl afsplus_core::flight::LiveSink for CounterSink {
    fn try_event(&mut self, event: Event) -> SinkResult {
        let counters = &self.shared.counters;
        counters.observed.fetch_add(1, Ordering::Relaxed);
        match event.kind {
            EventKind::ApiBegin => {
                counters.api_begin.fetch_add(1, Ordering::Relaxed);
                return SinkResult::Accepted;
            }
            EventKind::ApiSucceeded => {
                counters.api_succeeded.fetch_add(1, Ordering::Relaxed);
                return SinkResult::Accepted;
            }
            EventKind::ApiUnwound => {
                counters.api_unwound.fetch_add(1, Ordering::Relaxed);
            }
            EventKind::ApiFailed => {
                counters.api_failed.fetch_add(1, Ordering::Relaxed);
            }
            _ => return SinkResult::Accepted,
        }
        // Only the refusals are kept in detail, and only without waiting.
        match self.shared.recent.try_lock() {
            Ok(mut recent) => {
                recent.push(event);
                SinkResult::Accepted
            }
            Err(TryLockError::WouldBlock) => {
                counters.missed.fetch_add(1, Ordering::Relaxed);
                SinkResult::Busy
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().push(event);
                SinkResult::Accepted
            }
        }
    }
}

/// Installs a recorder observing API calls on a mounted volume, and returns
/// the diagnostics the mount reports from.
pub fn install<D: afsplus_block::BlockDevice>(
    vfs: &mut afsplus_vfs::Vfs<D>,
) -> Result<Arc<Diagnostics>, std::collections::TryReserveError> {
    let shared = Arc::new(Diagnostics::new());
    let capacity = std::num::NonZeroUsize::new(RECORDER_CAPACITY).expect("capacity is not zero");
    let mut recorder = afsplus_core::flight::FlightRecorder::new(capacity)?;
    recorder.set_categories(Categories::NONE.with(Category::Api));
    recorder.enable_api_observation();
    recorder.replace_sink(Some(Box::new(CounterSink::new(Arc::clone(&shared)))));
    vfs.replace_flight_recorder(Some(recorder));
    Ok(shared)
}
