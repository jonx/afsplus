//! The library's heap counters: what its Rust allocations hold now and at
//! most. One test in its own binary, since the meter is the library's and a
//! second test thread would allocate under it.

mod common;

use std::hint::black_box;
use std::mem::size_of;

use afsplus_aros_ffi::*;
use common::{formatted, mount};

fn counters(filesystem: *mut AfsplusAros) -> AfsplusArosCounters {
    let mut output = AfsplusArosCounters {
        struct_size: size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut output), 0);
    output
}

#[test]
fn heap_counters_follow_what_is_held_and_keep_the_peak() {
    const HELD: u64 = 4 << 20;
    let mut device = formatted(true);
    let filesystem = mount(&mut device);

    let mounted = counters(filesystem);
    assert_eq!(mounted.struct_size, 88);
    assert!(mounted.heap_bytes > 0, "a mounted volume holds memory");
    assert!(mounted.heap_peak_bytes >= mounted.heap_bytes);

    let held = black_box(vec![1u8; HELD as usize]);
    let holding = counters(filesystem);
    assert!(holding.heap_bytes >= mounted.heap_bytes + HELD);
    assert!(holding.heap_peak_bytes >= holding.heap_bytes);

    drop(held);
    let released = counters(filesystem);
    assert!(
        released.heap_bytes + HELD <= holding.heap_bytes,
        "a release lowers what is held: {} then {}",
        holding.heap_bytes,
        released.heap_bytes
    );
    assert!(
        released.heap_peak_bytes >= holding.heap_bytes,
        "the peak stays"
    );

    // A client of the first layout gets the first layout: the heap fields
    // are not written past the size it declared.
    let mut old = AfsplusArosCounters {
        struct_size: AFSPLUS_AROS_COUNTERS_FIRST_LAYOUT as u32,
        heap_bytes: 0x5a5a,
        heap_peak_bytes: 0x5a5a,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut old), 0);
    assert_eq!(old.struct_size, 72);
    assert_eq!((old.heap_bytes, old.heap_peak_bytes), (0x5a5a, 0x5a5a));
    assert!(old.calls > 0);

    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
