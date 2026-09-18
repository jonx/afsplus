//! The STEADY rounds on a delayed mount (ADR-121), measured at a durable
//! point: after a flush, three hundred more rounds hold almost nothing more. One test in its own binary: the heap meter is the
//! library's.

mod common;

use std::mem::size_of;

use afsplus_aros_ffi::*;
use common::{formatted, mount};

fn heap(filesystem: *mut AfsplusAros) -> (u64, u64) {
    assert_eq!(afsplus_aros_flush(filesystem), 0);
    let mut output = AfsplusArosCounters {
        struct_size: size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut output), 0);
    (output.heap_bytes, output.heap_peak_bytes)
}

/// The round of `heap_rounds`: create, read, lock and unlock a record, close, lock, examine, unlock,
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

fn round_with_clock(filesystem: *mut AfsplusAros, now: i64) {
    round(filesystem, now);
    // The handler's clock steps between rounds, as it would on the target.
    let mut pending = 0;
    assert_eq!(afsplus_aros_commit_due(filesystem, now, 0, &mut pending), 0);
}

#[test]
fn delayed_rounds_hold_nothing_more_at_a_durable_point() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    assert_eq!(afsplus_aros_set_commit_policy(filesystem, 5_000, 1_000), 0);
    for now in 1..=5 {
        round_with_clock(filesystem, now);
    }
    let (held, _) = heap(filesystem);
    let mut trail = Vec::new();
    for now in 6..=305 {
        round_with_clock(filesystem, now);
        if now % 50 == 0 {
            trail.push(heap(filesystem).0);
        }
    }
    let (held_after, _) = heap(filesystem);
    // Not to the byte: the commit path grows by a few dozen bytes over
    // hundreds of rounds, in sync mode too (an open finding of C13). A leak
    // of four bytes a round crosses this; the orphans a delete left behind,
    // before the commit cleaned them, cost 1.2 KB a round.
    assert!(
        held_after < held + 1_024,
        "held after the warm-up {held}, then {trail:?}, finally {held_after}"
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
