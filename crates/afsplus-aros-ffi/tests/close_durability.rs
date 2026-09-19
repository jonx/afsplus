//! A close is not a durability point (ADR-121). On a delayed mount, opening,
//! writing and closing a file asks the device for nothing; an fsync of the
//! handle and a `SYNC` mount still commit at once.

mod common;

use std::mem::size_of;

use afsplus_aros_ffi::*;
use common::{formatted, mount};

const CYCLES: usize = 16;
const PAYLOAD: [u8; 1200] = [0x33; 1200];

fn flushes(filesystem: *mut AfsplusAros) -> u64 {
    let mut output = AfsplusArosCounters {
        struct_size: size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut output), 0);
    output.device_flushes
}

/// What `ACTION_FINDOUTPUT`, `ACTION_WRITE` and `ACTION_END` do for one
/// created file, with the fsync of the handle the caller asks for or not.
fn create_write_close(filesystem: *mut AfsplusAros, name: &str, seconds: i64, fsync: bool) {
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            name.as_ptr(),
            name.len() as u32,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            seconds,
            0,
            &mut file
        ),
        0
    );
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write(
            filesystem,
            file,
            PAYLOAD.as_ptr(),
            PAYLOAD.len() as u32,
            seconds,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(count, PAYLOAD.len() as u32);
    if fsync {
        assert_eq!(afsplus_aros_fsync(filesystem, file), 0);
    }
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
}

#[test]
fn closes_on_a_delayed_mount_cost_no_commit() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    assert_eq!(afsplus_aros_set_commit_policy(filesystem, 5_000, 1_000), 0);

    let before = flushes(filesystem);
    for cycle in 0..CYCLES {
        create_write_close(
            filesystem,
            &format!("f{cycle:02}.c"),
            100 + cycle as i64,
            false,
        );
    }
    let after = flushes(filesystem);

    // Nothing here asks for durability and no clock makes the window due, so
    // at most the window's own bound may have committed once.
    assert!(
        after - before <= 1,
        "{CYCLES} closes flushed the device {} times",
        after - before
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

#[test]
fn an_fsync_of_the_handle_still_commits() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    assert_eq!(afsplus_aros_set_commit_policy(filesystem, 5_000, 1_000), 0);

    let before = flushes(filesystem);
    for cycle in 0..CYCLES {
        create_write_close(
            filesystem,
            &format!("f{cycle:02}.c"),
            100 + cycle as i64,
            true,
        );
    }
    let after = flushes(filesystem);

    // The control for the test above: a mount that stopped committing
    // altogether would pass it and fail here.
    assert!(
        after - before >= CYCLES as u64,
        "{CYCLES} fsyncs flushed the device {} times",
        after - before
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

#[test]
fn a_sync_mount_commits_every_cycle() {
    let mut device = formatted(true);
    // A mount starts SYNC; the handler asks for DELAYED, this one does not.
    let filesystem = mount(&mut device);

    let mut previous = flushes(filesystem);
    for cycle in 0..CYCLES {
        create_write_close(
            filesystem,
            &format!("f{cycle:02}.c"),
            100 + cycle as i64,
            false,
        );
        let now = flushes(filesystem);
        assert!(
            now > previous,
            "cycle {cycle} on a SYNC mount flushed nothing"
        );
        previous = now;
    }
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
