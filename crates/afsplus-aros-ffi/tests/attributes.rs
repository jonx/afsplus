//! Attribute group through the exported C functions.

mod common;

use std::ptr;

use afsplus_aros_ffi::*;
use afsplus_check::check_device;
use common::{formatted, mount};

fn create(filesystem: *mut AfsplusAros, name: &[u8]) {
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
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
}

fn set(filesystem: *mut AfsplusAros, attribute: &[u8], value: &[u8], mode: u32) -> i32 {
    afsplus_aros_set_attribute(
        filesystem,
        0,
        b"note".as_ptr(),
        4,
        attribute.as_ptr(),
        attribute.len() as u32,
        value.as_ptr(),
        value.len() as u32,
        mode,
        5,
        0,
    )
}

/// Reads into a guarded buffer of `capacity`; returns status, the required
/// size, and the bytes written (empty when it did not fit).
fn get(filesystem: *mut AfsplusAros, attribute: &[u8], capacity: usize) -> (i32, u32, Vec<u8>) {
    let mut buffer = [0xA5u8; 64];
    let mut required = u32::MAX;
    let status = afsplus_aros_get_attribute(
        filesystem,
        0,
        b"note".as_ptr(),
        4,
        attribute.as_ptr(),
        attribute.len() as u32,
        buffer.as_mut_ptr(),
        capacity as u32,
        &mut required,
    );
    let written = buffer.iter().take_while(|byte| **byte != 0xA5).count();
    assert!(buffer[written..].iter().all(|byte| *byte == 0xA5));
    (status, required, buffer[..written].to_vec())
}

#[test]
fn attributes_cross_the_boundary_with_size_probing() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    create(filesystem, b"note");

    assert_eq!(
        set(
            filesystem,
            b"user.kind",
            b"text",
            AFSPLUS_AROS_ATTRIBUTE_CREATE
        ),
        0
    );
    assert_eq!(
        set(
            filesystem,
            b"aros.tooltype",
            b"DONOTWAIT",
            AFSPLUS_AROS_ATTRIBUTE_UPSERT
        ),
        0
    );
    // A zero capacity asks for the size; a short buffer stays untouched.
    assert_eq!(get(filesystem, b"user.kind", 0), (0, 4, Vec::new()));
    assert_eq!(get(filesystem, b"user.kind", 3), (0, 4, Vec::new()));
    assert_eq!(get(filesystem, b"user.kind", 4), (0, 4, b"text".to_vec()));
    assert_eq!(
        get(filesystem, b"user.absent", 8),
        (205, u32::MAX, Vec::new())
    );

    let mut names = [0xA5u8; 64];
    let mut required = 0;
    assert_eq!(
        afsplus_aros_list_attributes(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            names.as_mut_ptr(),
            64,
            &mut required
        ),
        0
    );
    assert_eq!(&names[..required as usize], b"aros.tooltype\0user.kind\0");
    assert_eq!(names[required as usize], 0xA5);

    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    assert!(check_device(&mut device).is_clean());
    let filesystem = mount(&mut device);
    assert_eq!(get(filesystem, b"aros.tooltype", 16).2, b"DONOTWAIT");
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

#[test]
fn modes_refusals_and_the_preserved_namespaces() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    create(filesystem, b"note");
    assert_eq!(
        set(filesystem, b"user.a", b"1", AFSPLUS_AROS_ATTRIBUTE_REPLACE),
        205
    );
    assert_eq!(
        set(filesystem, b"user.a", b"1", AFSPLUS_AROS_ATTRIBUTE_CREATE),
        0
    );
    assert_eq!(
        set(filesystem, b"user.a", b"2", AFSPLUS_AROS_ATTRIBUTE_CREATE),
        203
    );
    assert_eq!(
        set(filesystem, b"user.a", b"2", AFSPLUS_AROS_ATTRIBUTE_REPLACE),
        0
    );
    assert_eq!(get(filesystem, b"user.a", 8).2, b"2");
    // A removal carries no value, and an unassigned mode is refused.
    assert_eq!(
        set(filesystem, b"user.a", b"x", AFSPLUS_AROS_ATTRIBUTE_REMOVE),
        115
    );
    assert_eq!(set(filesystem, b"user.a", b"", 4), 115);
    assert_eq!(
        set(filesystem, b"user.a", b"", AFSPLUS_AROS_ATTRIBUTE_REMOVE),
        0
    );
    assert_eq!(
        set(filesystem, b"user.a", b"", AFSPLUS_AROS_ATTRIBUTE_REMOVE),
        205
    );
    assert_eq!(
        set(
            filesystem,
            b"security.x",
            b"1",
            AFSPLUS_AROS_ATTRIBUTE_UPSERT
        ),
        223
    );
    assert_eq!(
        set(filesystem, b"system.x", b"1", AFSPLUS_AROS_ATTRIBUTE_UPSERT),
        223
    );
    assert_eq!(
        set(filesystem, b"plain", b"1", AFSPLUS_AROS_ATTRIBUTE_UPSERT),
        210
    );
    // A value past the format's bound.
    assert_eq!(
        set(
            filesystem,
            b"user.big",
            &vec![1u8; 65_536],
            AFSPLUS_AROS_ATTRIBUTE_UPSERT
        ),
        210
    );
    let mut required = 0;
    assert_eq!(
        afsplus_aros_get_attribute(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            b"user.a".as_ptr(),
            6,
            ptr::null_mut(),
            4,
            &mut required
        ),
        210
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
