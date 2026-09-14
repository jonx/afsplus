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
