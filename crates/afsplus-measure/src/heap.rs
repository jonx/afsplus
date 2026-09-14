//! Requested Rust heap bytes, including reallocations. No allocation, formatting,
//! OS queries or locks occur inside allocator callbacks. System allocator
//! overhead, direct C allocations, mappings and stack memory are outside scope.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub struct Meter<A = System> {
    allocator: A,
    live: AtomicUsize,
    peak: AtomicUsize,
    acquired: AtomicUsize,
    released: AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub live: usize,
    pub peak: usize,
    pub acquired: usize,
    pub released: usize,
}

#[cfg(any(not(feature = "allocation-domains"), test))]
impl Meter {
    pub const fn new() -> Self {
        Self::with_allocator(System)
    }
}

impl<A> Meter<A> {
    const fn with_allocator(allocator: A) -> Self {
        Self {
            allocator,
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            acquired: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
        }
    }

    /// Phase boundaries require a quiescent, single-threaded workload. The
    /// counters themselves are thread-safe; a simultaneous sample/reset is not
    /// an atomic cross-counter snapshot and cannot qualify parallel workloads.
    pub fn begin(&self) -> Sample {
        self.peak.store(self.live.load(Relaxed), Relaxed);
        self.sample()
    }

    pub fn sample(&self) -> Sample {
        Sample {
            live: self.live.load(Relaxed),
            peak: self.peak.load(Relaxed),
            acquired: self.acquired.load(Relaxed),
            released: self.released.load(Relaxed),
        }
    }

    fn acquire(&self, bytes: usize) {
        let live = self.live.fetch_add(bytes, Relaxed).wrapping_add(bytes);
        self.peak.fetch_max(live, Relaxed);
        self.acquired.fetch_add(bytes, Relaxed);
    }

    fn release(&self, bytes: usize) {
        self.live.fetch_sub(bytes, Relaxed);
        self.released.fetch_add(bytes, Relaxed);
    }
}

// SAFETY: forwards each valid layout/pointer unchanged to System. Successful
// requests alone change accounting; failure preserves the previous allocation.
// Atomic integer operations cannot allocate or unwind. Realloc measures the
// requested live size after success; System's internal moving-copy transient
// memory is deliberately outside this measurement.
unsafe impl<A: GlobalAlloc> GlobalAlloc for Meter<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller supplies the GlobalAlloc contract's valid layout.
        let pointer = unsafe { self.allocator.alloc(layout) };
        if !pointer.is_null() {
            self.acquire(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same forwarding contract as alloc.
        let pointer = unsafe { self.allocator.alloc_zeroed(layout) };
        if !pointer.is_null() {
            self.acquire(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: caller provides a live allocation with its original layout.
        unsafe { self.allocator.dealloc(pointer, layout) };
        self.release(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: caller meets GlobalAlloc's live pointer and new size contract.
        let result = unsafe { self.allocator.realloc(pointer, layout, size) };
        if !result.is_null() {
            if size >= layout.size() {
                self.acquire(size - layout.size());
            } else {
                self.release(layout.size() - size);
            }
        }
        result
    }
}

/// The tagged variant is separately selected by the allocation-domains feature.
/// Its header/padding cost is explicit; the ordinary forwarding meter is unchanged.
#[cfg(feature = "allocation-domains")]
#[derive(Clone, Copy)]
struct Header {
    origin: afsplus_core::allocation_trace::Domain,
}

#[cfg(feature = "allocation-domains")]
pub struct TaggedMeter<A = System> {
    allocator: A,
    total: Meter<()>,
    origins: [Meter<()>; afsplus_core::allocation_trace::COUNT],
    overhead: Meter<()>,
    system: Meter<()>,
}

#[cfg(feature = "allocation-domains")]
#[derive(Clone, Copy)]
pub struct Details {
    pub origins: [Sample; afsplus_core::allocation_trace::COUNT],
    pub overhead: Sample,
    pub system: Sample,
}

#[cfg(feature = "allocation-domains")]
impl TaggedMeter {
    pub const fn new() -> Self {
        Self::with_allocator(System)
    }
}

#[cfg(feature = "allocation-domains")]
impl<A> TaggedMeter<A> {
    const fn with_allocator(allocator: A) -> Self {
        Self {
            allocator,
            total: Meter::with_allocator(()),
            origins: [const { Meter::with_allocator(()) }; afsplus_core::allocation_trace::COUNT],
            overhead: Meter::with_allocator(()),
            system: Meter::with_allocator(()),
        }
    }
    pub fn sample(&self) -> Sample {
        self.total.sample()
    }
    pub fn begin(&self) -> Sample {
        for counter in &self.origins {
            counter.begin();
        }
        self.overhead.begin();
        self.system.begin();
        self.total.begin()
    }
    pub fn details(&self) -> Details {
        Details {
            origins: std::array::from_fn(|i| self.origins[i].sample()),
            overhead: self.overhead.sample(),
            system: self.system.sample(),
        }
    }
    fn layout(request: Layout) -> Option<(Layout, usize)> {
        let (layout, offset) = Layout::new::<Header>().extend(request).ok()?;
        Some((layout.pad_to_align(), offset))
    }
    fn acquire(&self, size: usize, system: usize, origin: afsplus_core::allocation_trace::Domain) {
        self.total.acquire(size);
        self.origins[origin as usize].acquire(size);
        self.system.acquire(system);
        self.overhead.acquire(system - size);
    }
    fn release(&self, size: usize, system: usize, origin: afsplus_core::allocation_trace::Domain) {
        self.total.release(size);
        self.origins[origin as usize].release(size);
        self.system.release(system);
        self.overhead.release(system - size);
    }
    fn resize_counter(counter: &Meter<()>, old: usize, new: usize) {
        if new >= old {
            counter.acquire(new - old);
        } else {
            counter.release(old - new);
        }
    }
}

// SAFETY: each System allocation contains an aligned private header followed by
// a payload aligned for the caller's layout. The same deterministic extended
// layout and offset recover the System pointer on free/realloc. Realloc preserves
// alignment (and therefore payload offset), and updates counters only on success.
// No allocation, locks, formatting or unwinding occur in these callbacks.
#[cfg(feature = "allocation-domains")]
unsafe impl<A: GlobalAlloc> GlobalAlloc for TaggedMeter<A> {
    unsafe fn alloc(&self, request: Layout) -> *mut u8 {
        let Some((layout, offset)) = Self::layout(request) else {
            return std::ptr::null_mut();
        };
        // SAFETY: the extended layout is validated by Layout::extend.
        let base = unsafe { self.allocator.alloc(layout) };
        if base.is_null() {
            return base;
        }
        let origin = afsplus_core::allocation_trace::current();
        // SAFETY: base is aligned for Header; the payload is within this block.
        unsafe {
            base.cast::<Header>().write(Header { origin });
        }
        self.acquire(request.size(), layout.size(), origin);
        unsafe { base.add(offset) }
    }
    unsafe fn alloc_zeroed(&self, request: Layout) -> *mut u8 {
        let Some((layout, offset)) = Self::layout(request) else {
            return std::ptr::null_mut();
        };
        // SAFETY: valid extended allocation; the header write never touches payload.
        let base = unsafe { self.allocator.alloc_zeroed(layout) };
        if base.is_null() {
            return base;
        }
        let origin = afsplus_core::allocation_trace::current();
        unsafe {
            base.cast::<Header>().write(Header { origin });
        }
        self.acquire(request.size(), layout.size(), origin);
        unsafe { base.add(offset) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, request: Layout) {
        // This layout succeeded at allocation. An impossible mismatch cannot unwind.
        let Some((layout, offset)) = Self::layout(request) else {
            std::process::abort();
        };
        // SAFETY: caller supplies a live payload and its original request layout.
        let base = unsafe { pointer.sub(offset) };
        let origin = unsafe { base.cast::<Header>().read().origin };
        unsafe {
            self.allocator.dealloc(base, layout);
        }
        self.release(request.size(), layout.size(), origin);
    }
    unsafe fn realloc(&self, pointer: *mut u8, request: Layout, size: usize) -> *mut u8 {
        let Ok(new_request) = Layout::from_size_align(size, request.align()) else {
            return std::ptr::null_mut();
        };
        let Some((new_layout, new_offset)) = Self::layout(new_request) else {
            return std::ptr::null_mut();
        };
        let Some((old_layout, offset)) = Self::layout(request) else {
            std::process::abort();
        };
        // SAFETY: valid original payload/layout; header and offset survive a
        // successful System realloc because request alignment never changes.
        let base = unsafe { pointer.sub(offset) };
        let origin = unsafe { base.cast::<Header>().read().origin };
        let result = unsafe { self.allocator.realloc(base, old_layout, new_layout.size()) };
        if result.is_null() {
            return result;
        }
        Self::resize_counter(&self.total, request.size(), size);
        Self::resize_counter(&self.origins[origin as usize], request.size(), size);
        Self::resize_counter(&self.system, old_layout.size(), new_layout.size());
        Self::resize_counter(
            &self.overhead,
            old_layout.size() - request.size(),
            new_layout.size() - size,
        );
        unsafe { result.add(new_offset) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct FailRequests(AtomicBool);
    // SAFETY: test allocator either refuses without touching memory or forwards
    // the original contract to System. Deallocation always releases live memory.
    unsafe impl GlobalAlloc for FailRequests {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if self.0.load(Relaxed) {
                std::ptr::null_mut()
            }
            // SAFETY: forwarded valid layout.
            else {
                unsafe { System.alloc(layout) }
            }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: forwarded live allocation and original layout.
            unsafe { System.dealloc(pointer, layout) }
        }
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            if self.0.load(Relaxed) {
                std::ptr::null_mut()
            }
            // SAFETY: forwarded live allocation, layout and valid new size.
            else {
                unsafe { System.realloc(pointer, layout, size) }
            }
        }
    }

    #[cfg(feature = "allocation-domains")]
    #[test]
    fn tagged_layouts_preserve_alignment_bytes_and_origin_through_resize() {
        use afsplus_core::allocation_trace::{self as trace, Domain};
        trace::enable();
        let meter = TaggedMeter::new();
        for align in [1, 2, 8, 16, 64, 4096] {
            for size in [1, 7, 8, 17, 4097] {
                let layout = Layout::from_size_align(size, align).unwrap();
                // SAFETY: checked allocations, in-bounds reads/writes and exactly
                // one free using the final successful request layout.
                unsafe {
                    let pointer = trace::within(Domain::Tree, || meter.alloc_zeroed(layout));
                    assert!(!pointer.is_null());
                    assert_eq!(pointer as usize % align, 0);
                    assert!(std::slice::from_raw_parts(pointer, size)
                        .iter()
                        .all(|&b| b == 0));
                    pointer.write(42);
                    let bigger = trace::within(Domain::Allocator, || {
                        meter.realloc(pointer, layout, size + 33)
                    });
                    assert!(!bigger.is_null());
                    assert_eq!(bigger.read(), 42);
                    assert_eq!(bigger as usize % align, 0);
                    let details = meter.details();
                    assert_eq!(details.origins[Domain::Tree as usize].live, size + 33);
                    assert_eq!(details.origins[Domain::Allocator as usize].live, 0);
                    assert_eq!(
                        details.system.live,
                        meter.sample().live + details.overhead.live
                    );
                    let smaller = meter.realloc(
                        bigger,
                        Layout::from_size_align(size + 33, align).unwrap(),
                        1,
                    );
                    assert!(!smaller.is_null());
                    assert_eq!(smaller.read(), 42);
                    trace::within(Domain::Oracle, || {
                        meter.dealloc(smaller, Layout::from_size_align(1, align).unwrap())
                    });
                }
                assert_eq!(meter.sample().live, 0);
                let details = meter.details();
                assert_eq!(details.origins.iter().map(|v| v.live).sum::<usize>(), 0);
                assert_eq!((details.system.live, details.overhead.live), (0, 0));
            }
        }
    }

    #[cfg(feature = "allocation-domains")]
    #[test]
    fn tagged_failed_growth_and_header_overflow_preserve_accounting() {
        use afsplus_core::allocation_trace::{self as trace, Domain};
        trace::enable();
        let meter = TaggedMeter::with_allocator(FailRequests(AtomicBool::new(true)));
        let layout = Layout::from_size_align(32, 16).unwrap();
        // SAFETY: failed allocations are never dereferenced; the successful
        // pointer stays valid after refused growth and is freed exactly once.
        unsafe {
            assert!(meter.alloc(layout).is_null());
            assert!(meter.alloc_zeroed(layout).is_null());
            meter.allocator.0.store(false, Relaxed);
            let pointer = trace::within(Domain::Tree, || meter.alloc(layout));
            assert!(!pointer.is_null());
            pointer.write(42);
            let before = meter.details();
            meter.allocator.0.store(true, Relaxed);
            assert!(trace::within(Domain::Batch, || meter.realloc(pointer, layout, 128)).is_null());
            assert_eq!(pointer.read(), 42);
            let after = meter.details();
            for (a, b) in before.origins.iter().zip(after.origins) {
                assert_eq!(
                    (a.live, a.acquired, a.released),
                    (b.live, b.acquired, b.released)
                );
            }
            assert_eq!(before.system.live, after.system.live);
            let huge = Layout::from_size_align(isize::MAX as usize, 1).unwrap();
            assert!(meter.alloc(huge).is_null());
            assert!(meter
                .realloc(pointer, layout, isize::MAX as usize)
                .is_null());
            meter.dealloc(pointer, layout);
        }
        assert_eq!(meter.sample().live, 0);
    }

    #[cfg(feature = "allocation-domains")]
    #[test]
    fn tagged_origin_survives_cross_thread_free_and_scope_unwind() {
        use afsplus_core::allocation_trace::{self as trace, Domain};
        trace::enable();
        let meter = TaggedMeter::new();
        let layout = Layout::from_size_align(100, 64).unwrap();
        // SAFETY: ownership is transferred to the scoped thread through an
        // exposed address; no access occurs on this thread after transfer.
        let address = unsafe { trace::within(Domain::Tree, || meter.alloc(layout)) } as usize;
        assert_ne!(address, 0);
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = trace::enter(Domain::Allocator);
                    assert_eq!(trace::current(), Domain::Allocator);
                    unsafe {
                        meter.dealloc(address as *mut u8, layout);
                    }
                })
                .join()
                .unwrap();
        });
        assert_eq!(meter.details().origins[Domain::Tree as usize].released, 100);
        assert_eq!(
            meter.details().origins[Domain::Allocator as usize].released,
            0
        );
        let _scope = trace::enter(Domain::Oracle);
        let _ = std::panic::catch_unwind(|| trace::within(Domain::Tree, || panic!("scope test")));
        assert_eq!(trace::current(), Domain::Oracle);
        assert_eq!(meter.sample().live, 0);
    }

    #[test]
    fn failure_keeps_live_allocation_and_counters_unchanged() {
        let meter = Meter::with_allocator(FailRequests(AtomicBool::new(true)));
        let layout = Layout::from_size_align(32, 8).unwrap();
        // SAFETY: valid layouts; failed requests are never dereferenced and the
        // one successful allocation is released using its unchanged layout.
        unsafe {
            assert!(meter.alloc(layout).is_null());
            assert!(meter.alloc_zeroed(layout).is_null());
            assert_eq!(meter.sample().acquired, 0);
            meter.allocator.0.store(false, Relaxed);
            let pointer = meter.alloc(layout);
            assert!(!pointer.is_null());
            pointer.write(123);
            meter.allocator.0.store(true, Relaxed);
            assert!(meter.realloc(pointer, layout, 128).is_null());
            assert_eq!(pointer.read(), 123);
            assert_eq!(meter.sample().live, 32);
            assert_eq!(meter.sample().peak, 32);
            assert_eq!(meter.sample().acquired, 32);
            assert_eq!(meter.sample().released, 0);
            meter.dealloc(pointer, layout);
            assert_eq!(meter.sample().live, 0);
        }
    }

    #[test]
    fn tracks_zeroing_growth_shrink_release_and_phase_reset() {
        let meter = Meter::new();
        // SAFETY: each non-null pointer is used within its live layout and is
        // released exactly once using its final successful realloc size.
        unsafe {
            let initial = Layout::from_size_align(32, 16).unwrap();
            let pointer = meter.alloc_zeroed(initial);
            assert!(!pointer.is_null());
            assert_eq!(std::slice::from_raw_parts(pointer, 32), &[0; 32]);
            pointer.write(42);
            let pointer = meter.realloc(pointer, initial, 128);
            assert!(!pointer.is_null());
            assert_eq!(pointer.read(), 42);
            assert_eq!(meter.sample().live, 128);
            assert_eq!(meter.sample().peak, 128);
            let pointer = meter.realloc(pointer, Layout::from_size_align(128, 16).unwrap(), 16);
            assert!(!pointer.is_null());
            assert_eq!(pointer.read(), 42);
            assert_eq!(meter.sample().live, 16);
            assert_eq!(meter.sample().peak, 128);
            assert_eq!(meter.begin().peak, 16);
            meter.dealloc(pointer, Layout::from_size_align(16, 16).unwrap());
        }
        let sample = meter.sample();
        assert_eq!(sample.live, 0);
        assert_eq!(sample.acquired, 128);
        assert_eq!(sample.released, 128);
    }

    #[test]
    fn independent_allocations_and_concurrent_release_balance() {
        let meter = Meter::new();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let meter = &meter;
                scope.spawn(move || {
                    for _ in 0..100 {
                        let layout = Layout::from_size_align(100, 8).unwrap();
                        // SAFETY: allocation is checked and freed once with the
                        // same layout, without sharing its pointer.
                        unsafe {
                            let pointer = meter.alloc(layout);
                            assert!(!pointer.is_null());
                            meter.dealloc(pointer, layout);
                        }
                    }
                });
            }
        });
        let sample = meter.sample();
        assert_eq!(sample.live, 0);
        assert_eq!(sample.acquired, 40_000);
        assert_eq!(sample.released, 40_000);
        assert!((100..=400).contains(&sample.peak));
    }
}
