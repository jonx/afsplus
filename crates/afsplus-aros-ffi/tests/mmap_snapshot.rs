// Copyright (C) 2026 John KNIPPER <code@jkn.me>.
// SPDX-License-Identifier: APL-1.1
mod common;
use afsplus_aros_ffi::*;
use afsplus_check::check_device;
use common::{formatted, mount};

fn open(filesystem: *mut AfsplusAros, name: &[u8]) -> u64 {
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            name.as_ptr(),
            name.len() as u32,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            1,
            0,
            &mut file
        ),
        0
    );
    file
}

#[test]
fn snapshot_handle_is_read_only_unnamed_and_keeps_size_after_source_unlink() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    let source = open(filesystem, b"source");
    let bytes = [0x35; 8192 + 111];
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write_at(
            filesystem,
            source,
            0,
            bytes.as_ptr(),
            bytes.len() as u32,
            2,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(count, 8303);
    let mut snapshot = 0;
    assert_eq!(
        afsplus_aros_clone_open_snapshot(filesystem, source, 3, 0, &mut snapshot),
        0
    );
    assert_ne!(snapshot, source);
    let mut size = 0;
    assert_eq!(afsplus_aros_file_size(filesystem, snapshot, &mut size), 0);
    assert_eq!(size, 8303);
    let mut read = [0; 8192 + 111];
    assert_eq!(
        afsplus_aros_read_at(
            filesystem,
            snapshot,
            0,
            read.as_mut_ptr(),
            read.len() as u32,
            &mut count
        ),
        0
    );
    assert_eq!(read, bytes);
    assert_eq!(
        afsplus_aros_write_at(filesystem, snapshot, 0, b"x".as_ptr(), 1, 4, 0, &mut count),
        214
    );
    assert_eq!(
        afsplus_aros_write_at(
            filesystem,
            source,
            0,
            b"changed".as_ptr(),
            7,
            5,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(afsplus_aros_close(filesystem, source), 0);
    assert_eq!(
        afsplus_aros_delete_object(filesystem, 0, b"source".as_ptr(), 6, 6, 0),
        0
    );
    assert_eq!(afsplus_aros_file_size(filesystem, snapshot, &mut size), 0);
    assert_eq!(size, 8303);
    read.fill(0);
    assert_eq!(
        afsplus_aros_read_at(
            filesystem,
            snapshot,
            0,
            read.as_mut_ptr(),
            read.len() as u32,
            &mut count
        ),
        0
    );
    assert_eq!(read, bytes);
    let mut old = 99;
    assert_eq!(
        afsplus_aros_seek(filesystem, snapshot, 0, AFSPLUS_AROS_SEEK_END, &mut old),
        0
    );
    assert_eq!(old, 0);
    let mut position = 0;
    assert_eq!(
        afsplus_aros_file_position(filesystem, snapshot, &mut position),
        0
    );
    assert_eq!(position, 8303);
    assert_eq!(afsplus_aros_close(filesystem, snapshot), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn missing_clone_capability_refuses_without_publishing_a_handle() {
    let mut device = formatted(false);
    let filesystem = mount(&mut device);
    let source = open(filesystem, b"source");
    let mut snapshot = 0xdead_beef;
    assert_eq!(
        afsplus_aros_clone_open_snapshot(filesystem, source, 2, 0, &mut snapshot),
        209
    );
    assert_eq!(snapshot, 0xdead_beef);
    assert_eq!(afsplus_aros_close(filesystem, source), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}
