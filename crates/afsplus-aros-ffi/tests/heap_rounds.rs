//! The STEADY round of the Hosted DOS probe at the C boundary, repeated:
//! after a warm-up, more rounds hold nothing more between rounds and need no
//! more memory at once. One test in its own binary, since the heap meter is
//! the library's.

mod common;

use std::mem::size_of;

use afsplus_aros_ffi::*;
use common::{formatted, mount};

fn heap(filesystem: *mut AfsplusAros) -> (u64, u64) {
    let mut output = AfsplusArosCounters {
        struct_size: size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut output), 0);
    (output.heap_bytes, output.heap_peak_bytes)
}

/// Create, read, lock and unlock a record, close, lock, examine, unlock,
/// watch and unwatch, set the comment and the protection, delete: each file
/// is a new object.
fn round(filesystem: *mut AfsplusAros, now: i64) {
    let name = b"note";
    let (mut file, mut count, mut lock, mut watch) = (0, 0, 0, 0);
    let open = |mode, file: &mut u64| {
        afsplus_aros_open(filesystem, 0, name.as_ptr(), 4, mode, now, 0, file)
    };
    assert_eq!(open(AFSPLUS_AROS_OPEN_NEW_FILE, &mut file), 0);
    let content = b"0123456789abcdef";
    assert_eq!(
        afsplus_aros_write(filesystem, file, content.as_ptr(), 16, now, 0, &mut count),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
    assert_eq!(open(AFSPLUS_AROS_OPEN_OLD_FILE, &mut file), 0);
    let mut buffer = [0u8; 17];
    assert_eq!(
        afsplus_aros_read(filesystem, file, buffer.as_mut_ptr(), 17, &mut count),
        0
    );
    assert_eq!(afsplus_aros_lock_record(filesystem, file, 0, 4, 1), 0);
    assert_eq!(afsplus_aros_free_record(filesystem, file, 0, 4), 0);
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            name.as_ptr(),
            4,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut lock
        ),
        0
    );
    let mut info = AfsplusArosFileInfo::default();
    let mut info_name = [0u8; 108];
    assert_eq!(
        afsplus_aros_examine_lock(filesystem, lock, &mut info, info_name.as_mut_ptr(), 108),
        0
    );
    assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
    assert_eq!(
        afsplus_aros_watch_add(filesystem, 0, name.as_ptr(), 4, &mut watch),
        0
    );
    assert_eq!(afsplus_aros_watch_remove(filesystem, watch), 0);
    let comment = b"round";
    assert_eq!(
        afsplus_aros_set_comment(filesystem, 0, name.as_ptr(), 4, comment.as_ptr(), 5, now, 0),
        0
    );
    assert_eq!(
        afsplus_aros_set_protection(filesystem, 0, name.as_ptr(), 4, 0x10, now, 0),
        0
    );
    assert_eq!(
        afsplus_aros_delete_object(filesystem, 0, name.as_ptr(), 4, now, 0),
        0
    );
}

#[test]
fn rounds_of_paired_operations_hold_nothing_and_do_not_raise_the_peak() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    // Structures reach their working size in the first rounds: measured,
    // the peak settles at the third and stays for a thousand more.
    for now in 1..=5 {
        round(filesystem, now);
    }
    let (held, peak) = heap(filesystem);
    for now in 6..=305 {
        round(filesystem, now);
    }
    let (held_after, peak_after) = heap(filesystem);
    assert_eq!(
        held_after,
        held,
        "300 more rounds hold {} bytes more",
        held_after as i64 - held as i64
    );
    assert_eq!(
        peak_after,
        peak,
        "300 more rounds raised the peak by {} bytes",
        peak_after as i64 - peak as i64
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
