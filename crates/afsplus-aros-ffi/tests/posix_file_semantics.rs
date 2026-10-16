// Copyright (C) 2026 John KNIPPER <code@jkn.me>
// Distributed under the AROS Public License 1.1.
mod common;

use afsplus_aros_ffi::*;
use common::{formatted, mount};
use std::collections::BTreeSet;
use std::mem::size_of;

fn create(fs: *mut AfsplusAros, name: &[u8]) {
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            0,
            name.as_ptr(),
            name.len() as u32,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            1_729_000_000,
            0,
            &mut file
        ),
        0
    );
    assert_eq!(afsplus_aros_close(fs, file), 0);
}

fn stat(fs: *mut AfsplusAros, name: &[u8]) -> AfsplusArosStat {
    let mut id = 0;
    assert_eq!(
        afsplus_aros_lookup_id(fs, 0, name.as_ptr(), name.len() as u32, &mut id),
        0
    );
    let mut out = AfsplusArosStat {
        struct_size: size_of::<AfsplusArosStat>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_stat_id(fs, id, &mut out), 0);
    out
}

#[test]
fn real_filesystem_preserves_identity_and_distinct_same_second_nanoseconds() {
    let mut device = formatted(true);
    let fs = mount(&mut device);
    create(fs, b"original");
    let original = stat(fs, b"original").object_id;
    assert_eq!(
        afsplus_aros_rename(
            fs,
            0,
            b"original".as_ptr(),
            8,
            0,
            b"renamed".as_ptr(),
            7,
            1_729_000_001,
            1
        ),
        0
    );
    assert_eq!(stat(fs, b"renamed").object_id, original);
    let mut lock = 0;
    assert_eq!(
        afsplus_aros_locate(
            fs,
            0,
            b"renamed".as_ptr(),
            7,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut lock
        ),
        0
    );
    assert_eq!(
        afsplus_aros_make_hard_link(fs, 0, b"alias".as_ptr(), 5, lock, 1_729_000_001, 2),
        0
    );
    assert_eq!(afsplus_aros_free_lock(fs, lock), 0);
    assert_eq!(stat(fs, b"alias").object_id, original);
    assert_eq!(stat(fs, b"alias").links, 2);

    let mut ids = BTreeSet::new();
    let mut nanoseconds = BTreeSet::new();
    for index in 1..=32_u32 {
        let name = format!("same-second-{index}");
        let bytes = name.as_bytes();
        create(fs, bytes);
        let ns = index * 1_234_567;
        assert_eq!(
            afsplus_aros_set_modified(
                fs,
                0,
                bytes.as_ptr(),
                bytes.len() as u32,
                1_729_000_010,
                ns,
                1_729_000_011,
                0
            ),
            0
        );
        let out = stat(fs, bytes);
        assert_eq!(out.modified_seconds, 1_729_000_010);
        assert_eq!(out.modified_nanoseconds, ns);
        assert_ne!(out.object_id, original);
        assert!(ids.insert(out.object_id));
        assert!(nanoseconds.insert(out.modified_nanoseconds));
    }
    assert_eq!(ids.len(), 32);
    assert_eq!(nanoseconds.len(), 32);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

#[test]
fn native_lookup_uses_mount_encoding_without_changing_utf8_object_api() {
    let mut device = formatted(true);
    let fs = common::mount_with_encoding(&mut device, AFSPLUS_AROS_ENCODING_LATIN1);
    let native = b"caf\xe9";
    let utf8 = "café".as_bytes();
    create(fs, native);
    let (mut first, mut second) = (0, 0);
    assert_eq!(
        afsplus_aros_lookup_dos_id(fs, 0, native.as_ptr(), native.len() as u32, &mut first),
        0
    );
    assert_eq!(
        afsplus_aros_lookup_id(fs, 0, utf8.as_ptr(), utf8.len() as u32, &mut second),
        0
    );
    assert_eq!(first, second);
    assert_eq!(
        afsplus_aros_lookup_id(fs, 0, native.as_ptr(), native.len() as u32, &mut second),
        210
    );
    assert_eq!(afsplus_aros_unmount(fs), 0);
}
