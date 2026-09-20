//! The Rust heap of this library: the bytes its allocations hold now and the
//! most they ever held at once. The handler's own C allocations, allocator
//! overhead and the stack are outside it. The meter is the library's, so a
//! process that runs several instances on one copy of the library sees their
//! sum.

use std::alloc::{GlobalAlloc, Layout};
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
        trace::note(bytes);
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
        trace::clear();
    }

    /// Where the allocations come from. While tracing is on, every
    /// allocation records the return addresses of the frames above it in a
    /// fixed table, so that a measurement can say which call sites the
    /// allocations of one operation belong to. It is off unless a
    /// measurement turns it on: a stack walk per allocation costs far more
    /// than the allocation.
    pub mod trace {
        use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};

        /// Frames kept per allocation, counted from the caller of the
        /// global allocator upwards.
        pub const DEPTH: usize = 16;
        const SLOTS: usize = 16384;

        extern "C" {
            fn backtrace(buffer: *mut *mut std::ffi::c_void, size: i32) -> i32;
        }

        static ON: AtomicBool = AtomicBool::new(false);
        static INSIDE: AtomicBool = AtomicBool::new(false);
        static COUNTS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
        static BYTES: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
        static FRAMES: [[AtomicUsize; DEPTH]; SLOTS] =
            [const { [const { AtomicUsize::new(0) }; DEPTH] }; SLOTS];

        pub fn on() {
            ON.store(true, Relaxed);
        }

        pub fn off() {
            ON.store(false, Relaxed);
        }

        pub fn clear() {
            for slot in 0..SLOTS {
                COUNTS[slot].store(0, Relaxed);
                BYTES[slot].store(0, Relaxed);
                for frame in FRAMES[slot].iter() {
                    frame.store(0, Relaxed);
                }
            }
        }

        /// The recorded stacks: the frames, how many allocations shared them
        /// and how many bytes those asked for.
        pub fn stacks() -> Vec<([usize; DEPTH], u64, u64)> {
            let mut out = Vec::new();
            for slot in 0..SLOTS {
                let count = COUNTS[slot].load(Relaxed);
                if count == 0 {
                    continue;
                }
                let mut frames = [0usize; DEPTH];
                for (index, frame) in FRAMES[slot].iter().enumerate() {
                    frames[index] = frame.load(Relaxed);
                }
                out.push((frames, count, BYTES[slot].load(Relaxed)));
            }
            out
        }

        pub(in crate::heap) fn note(bytes: usize) {
            if !ON.load(Relaxed) || INSIDE.swap(true, Relaxed) {
                return;
            }
            // The stack walk allocates nothing; the guard above is only
            // there so that a future one could not recurse into the meter.
            let mut raw = [std::ptr::null_mut(); DEPTH + 4];
            // SAFETY: `raw` holds the length passed.
            let taken = unsafe { backtrace(raw.as_mut_ptr(), raw.len() as i32) } as usize;
            let mut frames = [0usize; DEPTH];
            for index in 0..DEPTH {
                // The first frames are the allocator's own and say nothing.
                let source = index + 4;
                frames[index] = if source < taken {
                    raw[source] as usize
                } else {
                    0
                };
            }
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for frame in frames {
                hash ^= frame as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
            let mut slot = (hash as usize) % SLOTS;
            for _ in 0..SLOTS {
                if COUNTS[slot].load(Relaxed) == 0 {
                    for (index, frame) in frames.iter().enumerate() {
                        FRAMES[slot][index].store(*frame, Relaxed);
                    }
                    break;
                }
                if (0..DEPTH).all(|index| FRAMES[slot][index].load(Relaxed) == frames[index]) {
                    break;
                }
                slot = (slot + 1) % SLOTS;
            }
            COUNTS[slot].fetch_add(1, Relaxed);
            BYTES[slot].fetch_add(bytes as u64, Relaxed);
            INSIDE.store(false, Relaxed);
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

/// The allocator under the meter. Rust's `GlobalAlloc` hands the layout to
/// `dealloc` and to `realloc`, so the size of a block is known where it is
/// freed and nothing here keeps a size header.
///
/// On AROS that is what the handler needs: `afsplus_exec_alloc` and
/// `afsplus_exec_free` in `native/aros/afsplus_bootlibc.c` call exec's
/// `AllocMem` and `FreeMem` with the size, and the 16-byte header the
/// handler's `malloc` puts before every block is saved on every Rust
/// allocation. On the development host the pair is `malloc` and `free`,
/// which is what Rust's `System` calls for these alignments anyway, and a
/// debug build checks the size it is given against the block's own.
mod backing {
    #[cfg(target_os = "aros")]
    extern "C" {
        fn afsplus_exec_alloc(size: usize, clear: i32) -> *mut u8;
        fn afsplus_exec_free(pointer: *mut u8, size: usize);
    }

    #[cfg(not(target_os = "aros"))]
    extern "C" {
        fn malloc(size: usize) -> *mut u8;
        fn calloc(count: usize, size: usize) -> *mut u8;
        fn free(pointer: *mut u8);
        #[link_name = "realloc"]
        fn c_realloc(pointer: *mut u8, size: usize) -> *mut u8;
    }

    // The size the host's allocator actually gave a block. Only Apple's C
    // library is asked; elsewhere the check below is not made, and the AROS
    // path has no such call at all.
    #[cfg(all(not(target_os = "aros"), target_vendor = "apple", debug_assertions))]
    extern "C" {
        fn malloc_size(pointer: *const u8) -> usize;
    }

    /// A free whose size is not the size the block was allocated with is a
    /// bug in this file, and on AROS it would hand exec's free list a wrong
    /// length in silence. A debug build stops on it; a release build pays
    /// nothing.
    #[allow(unused_variables)]
    fn check_size(pointer: *mut u8, size: usize) {
        #[cfg(all(not(target_os = "aros"), target_vendor = "apple", debug_assertions))]
        {
            // SAFETY: `pointer` is a live block of this allocator.
            let given = unsafe { malloc_size(pointer) };
            assert!(
                given >= size,
                "a block of {given} bytes is being freed as {size}"
            );
        }
    }

    /// # Safety
    /// `size` is not zero.
    pub unsafe fn alloc(size: usize, zeroed: bool) -> *mut u8 {
        #[cfg(target_os = "aros")]
        // SAFETY: a non-zero size, and the C side answers null on failure.
        unsafe {
            afsplus_exec_alloc(size, i32::from(zeroed))
        }
        #[cfg(not(target_os = "aros"))]
        // SAFETY: as above.
        unsafe {
            if zeroed {
                calloc(1, size)
            } else {
                malloc(size)
            }
        }
    }

    /// # Safety
    /// `pointer` is a live block of this allocator and `size` is the size it
    /// was allocated with.
    pub unsafe fn dealloc(pointer: *mut u8, size: usize) {
        check_size(pointer, size);
        #[cfg(target_os = "aros")]
        // SAFETY: the caller's contract.
        unsafe {
            afsplus_exec_free(pointer, size)
        }
        #[cfg(not(target_os = "aros"))]
        // SAFETY: the caller's contract; `free` does not need the size.
        unsafe {
            let _ = size;
            free(pointer)
        }
    }

    /// A block of `new_size` holding the first bytes of `pointer`. The host
    /// asks its own `realloc`, which grows a block in place where it can;
    /// exec has nothing of the kind, so on AROS the block is moved.
    ///
    /// # Safety
    /// `pointer` is a live block of this allocator of `old_size` bytes, and
    /// `new_size` is not zero.
    pub unsafe fn realloc(pointer: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
        #[cfg(target_os = "aros")]
        // SAFETY: the caller's contract.
        unsafe {
            let moved = alloc(new_size, false);
            if !moved.is_null() {
                core::ptr::copy_nonoverlapping(pointer, moved, old_size.min(new_size));
                dealloc(pointer, old_size);
            }
            moved
        }
        #[cfg(not(target_os = "aros"))]
        // SAFETY: the caller's contract; the check is the free's.
        unsafe {
            check_size(pointer, old_size);
            c_realloc(pointer, new_size)
        }
    }
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
    let base = unsafe { backing::alloc(outer.size(), zeroed) };
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
    unsafe { backing::dealloc(base, outer.size()) };
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
            // SAFETY: a non-zero size, as `GlobalAlloc` requires.
            unsafe { backing::alloc(layout.size(), false) }
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
            unsafe { backing::alloc(layout.size(), true) }
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
            unsafe { backing::dealloc(pointer, layout.size()) };
        }
        self.release(layout.size());
        #[cfg(feature = "heap-profile")]
        profile::note_free();
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.align() > MALLOC_ALIGN {
            // An over-aligned block is moved: its base address sits in the
            // word before it, and only a fresh block re-establishes that.
            let Ok(new_layout) = Layout::from_size_align(new_size, layout.align()) else {
                return std::ptr::null_mut();
            };
            // SAFETY: a layout of its own.
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
        // SAFETY: the caller's live pointer with the size it was given.
        let moved = unsafe { backing::realloc(pointer, layout.size(), new_size) };
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

    /// The negative control of the header-free path: no block records its
    /// own size any more, so a free given the wrong size would hand exec's
    /// free list a length that is not the block's. A debug build catches it.
    #[test]
    #[should_panic(expected = "is being freed as")]
    #[cfg(all(target_vendor = "apple", debug_assertions))]
    fn a_wrong_size_on_free_is_caught() {
        // SAFETY: a non-zero size. The free below is expected to stop the
        // test before the block is returned.
        let pointer = unsafe { backing::alloc(64, false) };
        assert!(!pointer.is_null());
        // SAFETY: the pointer is live; the size is deliberately not its own.
        unsafe { backing::dealloc(pointer, 1 << 20) };
    }
}
