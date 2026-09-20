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

/// A count of the allocations the meter sees and a histogram of their sizes,
/// so that a measurement can say how many allocations an operation costs and
/// how large the largest of them are. It is a build of its own, behind the
/// `heap-profile` feature, because an atomic increment per allocation would
/// otherwise sit in the hottest path the library has.
#[cfg(feature = "heap-profile")]
pub mod profile {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

    /// One bucket per power of two of the size asked for, the last one
    /// holding everything from 2^30 upwards.
    pub const BUCKETS: usize = 32;

    static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static FREES: AtomicU64 = AtomicU64::new(0);
    static REALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static LARGEST: AtomicUsize = AtomicUsize::new(0);
    static HISTOGRAM: [AtomicU64; BUCKETS] = [const { AtomicU64::new(0) }; BUCKETS];

    pub(super) fn note_alloc(bytes: usize) {
        ALLOCATIONS.fetch_add(1, Relaxed);
        record(bytes);
    }

    pub(super) fn note_free() {
        FREES.fetch_add(1, Relaxed);
    }

    pub(super) fn note_realloc(bytes: usize) {
        REALLOCATIONS.fetch_add(1, Relaxed);
        record(bytes);
    }

    fn record(bytes: usize) {
        LARGEST.fetch_max(bytes, Relaxed);
        HISTOGRAM[bucket(bytes)].fetch_add(1, Relaxed);
    }

    fn bucket(bytes: usize) -> usize {
        ((usize::BITS - bytes.leading_zeros()) as usize).min(BUCKETS - 1)
    }

    /// A reading of the counters: allocations, frees, reallocations, the
    /// largest size asked for, and the histogram of the sizes asked for by
    /// an allocation or a reallocation.
    pub struct Profile {
        pub allocations: u64,
        pub frees: u64,
        pub reallocations: u64,
        pub largest: usize,
        pub histogram: [u64; BUCKETS],
    }

    pub fn sample() -> Profile {
        let mut histogram = [0u64; BUCKETS];
        for (slot, counter) in histogram.iter_mut().zip(HISTOGRAM.iter()) {
            *slot = counter.load(Relaxed);
        }
        Profile {
            allocations: ALLOCATIONS.load(Relaxed),
            frees: FREES.load(Relaxed),
            reallocations: REALLOCATIONS.load(Relaxed),
            largest: LARGEST.load(Relaxed),
            histogram,
        }
    }

    /// The bytes held now and the peak, as `afsplus_aros_counters` reports
    /// them, but readable without a mount: a measurement needs the heap
    /// before the volume is mounted to subtract what its test disk holds.
    pub fn heap() -> (u64, u64) {
        super::sample()
    }

    pub fn reset() {
        ALLOCATIONS.store(0, Relaxed);
        FREES.store(0, Relaxed);
        REALLOCATIONS.store(0, Relaxed);
        LARGEST.store(0, Relaxed);
        for counter in HISTOGRAM.iter() {
            counter.store(0, Relaxed);
        }
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

/// The largest alignment the system allocator gives through `malloc`.
/// Beyond it Rust's `System` asks `posix_memalign`, which on AROS lives in
/// posixc.library: a library a boot volume's handler cannot open, since it
/// needs dos.library to start and dos.library waits for the boot volume.
/// Larger alignments are therefore served here from a `malloc` block.
const MALLOC_ALIGN: usize = 16;

/// A block for an alignment above [`MALLOC_ALIGN`]: `align` bytes more than
/// asked for, the address handed out rounded up to `align`, and the block's
/// own address kept in the word before it.
fn over_aligned(layout: Layout) -> Option<Layout> {
    let size = layout.size().checked_add(layout.align())?;
    Layout::from_size_align(size, MALLOC_ALIGN).ok()
}

unsafe fn alloc_over_aligned(layout: Layout, zeroed: bool) -> *mut u8 {
    let Some(outer) = over_aligned(layout) else {
        return std::ptr::null_mut();
    };
    // SAFETY: `outer` has a non-zero size and the malloc alignment.
    let base = unsafe {
        if zeroed {
            System.alloc_zeroed(outer)
        } else {
            System.alloc(outer)
        }
    };
    if base.is_null() {
        return base;
    }
    // At least MALLOC_ALIGN bytes lie between `base` and the aligned address,
    // so the word before it is inside the block.
    let aligned = (base as usize + layout.align()) & !(layout.align() - 1);
    let pointer = aligned as *mut u8;
    // SAFETY: `pointer - size_of::<usize>()` is within the block and aligned.
    unsafe { (pointer as *mut usize).sub(1).write(base as usize) };
    pointer
}

unsafe fn dealloc_over_aligned(pointer: *mut u8, layout: Layout) {
    // SAFETY: `pointer` came from `alloc_over_aligned` with this layout.
    let base = unsafe { (pointer as *mut usize).sub(1).read() } as *mut u8;
    let outer = over_aligned(layout).expect("the layout was allocated");
    // SAFETY: `base` is the block `alloc_over_aligned` got for `outer`.
    unsafe { System.dealloc(base, outer) };
}

// SAFETY: every call is forwarded to the system allocator, through `malloc`
// for every alignment; the meter only counts the sizes of the calls that
// succeeded.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = if layout.align() > MALLOC_ALIGN {
            // SAFETY: the caller's layout contract holds.
            unsafe { alloc_over_aligned(layout, false) }
        } else {
            // SAFETY: the caller's layout contract is passed on unchanged.
            unsafe { System.alloc(layout) }
        };
        if !pointer.is_null() {
            self.acquire(layout.size());
            #[cfg(feature = "heap-profile")]
            profile::note_alloc(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = if layout.align() > MALLOC_ALIGN {
            // SAFETY: as for `alloc`.
            unsafe { alloc_over_aligned(layout, true) }
        } else {
            // SAFETY: as for `alloc`.
            unsafe { System.alloc_zeroed(layout) }
        };
        if !pointer.is_null() {
            self.acquire(layout.size());
            #[cfg(feature = "heap-profile")]
            profile::note_alloc(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if layout.align() > MALLOC_ALIGN {
            // SAFETY: the caller returns a live allocation of this layout.
            unsafe { dealloc_over_aligned(pointer, layout) };
        } else {
            // SAFETY: the caller returns a live allocation of this layout.
            unsafe { System.dealloc(pointer, layout) };
        }
        self.release(layout.size());
        #[cfg(feature = "heap-profile")]
        profile::note_free();
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.align() > MALLOC_ALIGN {
            // SAFETY: the caller's live pointer and a valid new layout.
            let Ok(new_layout) = Layout::from_size_align(new_size, layout.align()) else {
                return std::ptr::null_mut();
            };
            let moved = unsafe { self.alloc(new_layout) };
            if !moved.is_null() {
                // SAFETY: both blocks are live and at least this long.
                unsafe {
                    std::ptr::copy_nonoverlapping(pointer, moved, layout.size().min(new_size));
                    self.dealloc(pointer, layout);
                }
            }
            return moved;
        }
        // SAFETY: the caller's live pointer and new size are passed on.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            #[cfg(feature = "heap-profile")]
            profile::note_realloc(new_size);
            if new_size >= layout.size() {
                self.acquire(new_size - layout.size());
            } else {
                self.release(layout.size() - new_size);
            }
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignments_beyond_malloc_are_served_and_returned() {
        for align in [32usize, 64, 4096] {
            let layout = Layout::from_size_align(100, align).unwrap();
            // SAFETY: a valid non-zero layout, returned below.
            let pointer = unsafe { HEAP.alloc_zeroed(layout) };
            assert!(!pointer.is_null());
            assert_eq!(pointer as usize % align, 0);
            // SAFETY: 100 bytes were allocated zeroed.
            assert!(unsafe { std::slice::from_raw_parts(pointer, 100) }
                .iter()
                .all(|&byte| byte == 0));
            // SAFETY: the pointer is live with this layout.
            let grown = unsafe { HEAP.realloc(pointer, layout, 5000) };
            assert!(!grown.is_null());
            assert_eq!(grown as usize % align, 0);
            let grown_layout = Layout::from_size_align(5000, align).unwrap();
            // SAFETY: the grown block is live with this layout.
            unsafe { HEAP.dealloc(grown, grown_layout) };
        }
    }
}
