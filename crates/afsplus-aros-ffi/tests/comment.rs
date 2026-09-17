//! Comment group through the exported C functions.

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

fn set(filesystem: *mut AfsplusAros, name: &[u8], comment: &[u8]) -> i32 {
    afsplus_aros_set_comment(
        filesystem,
        0,
        name.as_ptr(),
        name.len() as u32,
        comment.as_ptr(),
        comment.len() as u32,
        5,
        0,
    )
}

/// Reads into a guarded buffer: bytes past the reported length stay 0xA5.
fn get(filesystem: *mut AfsplusAros, name: &[u8], capacity: usize) -> Result<Vec<u8>, i32> {
    let mut buffer = [0xA5u8; 300];
    let mut length = u32::MAX;
    let status = afsplus_aros_comment(
        filesystem,
        0,
        name.as_ptr(),
        name.len() as u32,
        buffer.as_mut_ptr(),
        capacity as u32,
        &mut length,
    );
    if status != 0 {
        assert_eq!(length, u32::MAX, "a failed call wrote its output");
        return Err(status);
    }
    assert!(length as usize <= capacity);
    assert!(buffer[length as usize..].iter().all(|byte| *byte == 0xA5));
    Ok(buffer[..length as usize].to_vec())
}

#[test]
fn comment_round_trips_and_survives_a_remount() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    create(filesystem, b"note");
    assert_eq!(get(filesystem, b"note", 79), Ok(Vec::new()));
    assert_eq!(set(filesystem, b"note", b"from the C side"), 0);
    assert_eq!(
        get(filesystem, b"note", 79),
        Ok(b"from the C side".to_vec())
    );
    // An open file reports the same comment; a stale handle is refused.
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            AFSPLUS_AROS_OPEN_OLD_FILE,
            6,
            0,
            &mut file
        ),
        0
    );
    let mut buffer = [0xA5u8; 8];
    let mut length = 0;
    assert_eq!(
        afsplus_aros_file_comment(filesystem, file, buffer.as_mut_ptr(), 8, &mut length),
        0
    );
    assert_eq!((length, &buffer), (8, b"from the"));
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
    assert_eq!(
        afsplus_aros_file_comment(filesystem, file, buffer.as_mut_ptr(), 8, &mut length),
        211
    );
    // A short buffer receives a prefix, never an error, and nothing past it.
    assert_eq!(get(filesystem, b"note", 4), Ok(b"from".to_vec()));
    assert_eq!(get(filesystem, b"note", 0), Ok(Vec::new()));
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    assert!(check_device(&mut device).is_clean());

    let filesystem = mount(&mut device);
    assert_eq!(
        get(filesystem, b"note", 79),
        Ok(b"from the C side".to_vec())
    );
    assert_eq!(set(filesystem, b"note", b""), 0);
    assert_eq!(get(filesystem, b"note", 79), Ok(Vec::new()));
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    assert!(check_device(&mut device).is_clean());
}

#[test]
fn refusals_carry_dos_codes_and_change_nothing() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    create(filesystem, b"note");
    assert_eq!(set(filesystem, b"note", b"kept"), 0);

    assert_eq!(set(filesystem, b"note", &[b'x'; 256]), 81);
    assert_eq!(set(filesystem, b"absent", b"x"), 205);
    assert_eq!(set(filesystem, b"note", &[0xff]), 210);
    assert_eq!(get(filesystem, b"absent", 79), Err(205));
    // Null pointers with a nonzero length, and a null output.
    assert_eq!(
        afsplus_aros_set_comment(filesystem, 0, b"note".as_ptr(), 4, ptr::null(), 3, 5, 0),
        210
    );
    let mut length = 0;
    assert_eq!(
        afsplus_aros_comment(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            ptr::null_mut(),
            8,
            &mut length
        ),
        210
    );
    assert_eq!(
        afsplus_aros_comment(
            filesystem,
            0,
            b"note".as_ptr(),
            4,
            ptr::null_mut(),
            0,
            ptr::null_mut()
        ),
        210
    );
    assert_eq!(get(filesystem, b"note", 79), Ok(b"kept".to_vec()));
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
