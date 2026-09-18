//! The Rust heap of this library: the bytes its allocations hold now and the
//! most they ever held at once. The handler's own C allocations, allocator
//! overhead and the stack are outside it. The meter is the library's, so a
//! process that runs several instances on one copy of the library sees their
//! sum.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub(crate) struct Meter {
    live: AtomicUsize,
    peak: AtomicUsize,
}

impl Meter {
    const fn new() -> Self {
        Meter {
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    fn acquire(&self, bytes: usize) {
        let live = self.live.fetch_add(bytes, Relaxed).wrapping_add(bytes);
        self.peak.fetch_max(live, Relaxed);
    }

    fn release(&self, bytes: usize) {
        self.live.fetch_sub(bytes, Relaxed);
    }
}

#[global_allocator]
static HEAP: Meter = Meter::new();

/// Bytes held now and the most held at once since the library started.
pub(crate) fn sample() -> (u64, u64) {
    (
        HEAP.live.load(Relaxed) as u64,
        HEAP.peak.load(Relaxed) as u64,
    )
}

// SAFETY: every call is forwarded unchanged to the system allocator; the
// meter only counts the sizes of the calls that succeeded.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout contract is passed on unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            self.acquire(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as for `alloc`.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            self.acquire(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller returns a live allocation of this layout.
        unsafe { System.dealloc(pointer, layout) };
        self.release(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's live pointer and new size are passed on.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            if new_size >= layout.size() {
                self.acquire(new_size - layout.size());
            } else {
                self.release(layout.size() - new_size);
            }
        }
        moved
    }
}
