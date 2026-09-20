//! The meter counts what was asked for, to the byte. Nothing records a
//! block's size beside it any longer, so a backing allocator that rounded a
//! size, or a meter that counted a rounded one, would show here. One test in
//! its own binary: the meter is the whole binary's allocator, and a second
//! test thread would allocate between the two readings.
#![cfg(feature = "heap-profile")]

use std::alloc::{alloc, dealloc, Layout};

use afsplus_aros_ffi::heap_profile;

#[test]
fn the_meter_and_the_allocator_agree_to_the_byte() {
    // Sizes no allocator serves exactly, and one alignment above the
    // sixteen bytes `malloc` gives, which takes the over-aligned path.
    const BLOCKS: [(usize, usize); 7] = [
        (1, 1),
        (7, 8),
        (13, 8),
        (100, 8),
        (4095, 16),
        (65537, 8),
        (300, 4096),
    ];
    let mut held = Vec::with_capacity(BLOCKS.len());
    let mut asked = 0u64;
    for (size, align) in BLOCKS {
        let layout = Layout::from_size_align(size, align).unwrap();
        // SAFETY: a valid non-zero layout, returned below.
        let pointer = unsafe { alloc(layout) };
        assert!(!pointer.is_null());
        assert_eq!(pointer as usize % align, 0);
        asked += size as u64;
        held.push((pointer, layout));
    }
    // The vector was made at its full size beforehand, so between the two
    // readings nothing moves but these blocks.
    let after_allocating = heap_profile::heap().0;
    for (pointer, layout) in held.drain(..) {
        // SAFETY: each block is live with its own layout.
        unsafe { dealloc(pointer, layout) };
    }
    let after_freeing = heap_profile::heap().0;
    assert_eq!(
        after_allocating - after_freeing,
        asked,
        "the fall is exactly what was asked for: {after_allocating} then {after_freeing}"
    );
}
