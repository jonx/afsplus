//! Group COMMIT through the C boundary (ADR-121): a delayed mount writes
//! nothing to the device until the handler's clock makes its changes due.

mod common;

use std::mem::size_of;

use afsplus_aros_ffi::*;
use common::{formatted, mount};

fn flushes(filesystem: *mut AfsplusAros) -> u64 {
    let mut output = AfsplusArosCounters {
        struct_size: size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(filesystem, &mut output), 0);
    output.device_flushes
}

fn due(filesystem: *mut AfsplusAros, seconds: i64, milliseconds: u32) -> u32 {
    let mut pending = 9;
    assert_eq!(
        afsplus_aros_commit_due(filesystem, seconds, milliseconds * 1_000_000, &mut pending),
        0
    );
    pending
}

fn write_file(filesystem: *mut AfsplusAros, name: &[u8], bytes: &[u8], seconds: i64) {
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
            bytes.as_ptr(),
            bytes.len() as u32,
            seconds,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
}

fn read_file(filesystem: *mut AfsplusAros, name: &[u8]) -> Vec<u8> {
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            name.as_ptr(),
            name.len() as u32,
            AFSPLUS_AROS_OPEN_OLD_FILE,
            0,
            0,
            &mut file
        ),
        0
    );
    let mut buffer = [0u8; 64];
    let mut count = 0;
    assert_eq!(
        afsplus_aros_read(filesystem, file, buffer.as_mut_ptr(), 64, &mut count),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
    buffer[..count as usize].to_vec()
}

#[test]
fn a_delayed_mount_writes_when_its_clock_says_so() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    // Refused policies change nothing.
    assert_eq!(
        afsplus_aros_set_commit_policy(filesystem, 60_001, 1_000),
        115
    );
    assert_eq!(afsplus_aros_set_commit_policy(filesystem, 5_000, 0), 115);
    assert_eq!(
        afsplus_aros_set_commit_policy(filesystem, 5_000, 6_000),
        115
    );
    assert_eq!(afsplus_aros_set_commit_policy(filesystem, 5_000, 1_000), 0);

    let before = flushes(filesystem);
    write_file(filesystem, b"one", b"first", 100);
    write_file(filesystem, b"two", b"second", 100);
    assert_eq!(
        read_file(filesystem, b"one"),
        b"first",
        "visible before any commit"
    );
    assert_eq!(
        flushes(filesystem),
        before,
        "nothing forced to the device yet"
    );
    assert_eq!(due(filesystem, 100, 500), 1, "not idle for a second yet");
    assert_eq!(flushes(filesystem), before);
    assert_eq!(due(filesystem, 101, 200), 0, "idle: committed");
    assert!(flushes(filesystem) > before);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);

    let filesystem = mount(&mut device);
    assert_eq!(read_file(filesystem, b"two"), b"second");
    // A mount starts with every change durable at once.
    write_file(filesystem, b"three", b"third", 200);
    assert_eq!(due(filesystem, 200, 0), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

#[test]
fn a_delayed_change_never_committed_is_lost_whole() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    assert_eq!(afsplus_aros_set_commit_policy(filesystem, 5_000, 1_000), 0);
    write_file(filesystem, b"kept", b"k", 10);
    assert_eq!(due(filesystem, 20, 0), 0);
    write_file(filesystem, b"lost", b"l", 30);
    // A crash: the handler goes without unmounting.
    let snapshot = device.clone();
    let _ = afsplus_aros_unmount(filesystem);
    let mut crashed = snapshot;
    let filesystem = mount(&mut crashed);
    assert_eq!(read_file(filesystem, b"kept"), b"k");
    let mut lock = 0;
    assert_eq!(
        afsplus_aros_locate(filesystem, 0, b"lost".as_ptr(), 4, 0, &mut lock),
        205
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
