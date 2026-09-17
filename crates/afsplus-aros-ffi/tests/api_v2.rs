//! API v2 entry-point group through the exported C functions.

mod common;

use std::ptr;

use afsplus_aros_ffi::*;
use afsplus_check::check_device;
use common::{formatted, mount};

fn open(filesystem: *mut AfsplusAros, name: &[u8], mode: u32) -> u64 {
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            filesystem,
            0,
            name.as_ptr(),
            name.len() as u32,
            mode,
            1,
            0,
            &mut file
        ),
        0
    );
    file
}

fn blocks(filesystem: *mut AfsplusAros, file: u64) -> u64 {
    let mut info = AfsplusArosFileInfo::default();
    let mut name = [0u8; 107];
    assert_eq!(
        afsplus_aros_examine_file(filesystem, file, &mut info, name.as_mut_ptr(), 107),
        0
    );
    info.blocks
}

#[test]
fn c_boundary_carries_positioned_io_clone_preallocate_replace_and_advise() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    let source = open(filesystem, b"source", AFSPLUS_AROS_OPEN_NEW_FILE);
    let mut count = 0;

    // Positioned I/O beyond 4 GiB; the DOS position stays at zero.
    assert_eq!(
        afsplus_aros_write_at(
            filesystem,
            source,
            0x1_0000_0000,
            b"beyond".as_ptr(),
            6,
            2,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(count, 6);
    let mut position = 99;
    assert_eq!(
        afsplus_aros_file_position(filesystem, source, &mut position),
        0
    );
    assert_eq!(position, 0);
    let mut size = 0;
    assert_eq!(afsplus_aros_file_size(filesystem, source, &mut size), 0);
    assert_eq!(size, 0x1_0000_0006);
    let mut bytes = [0xEEu8; 8];
    assert_eq!(
        afsplus_aros_read_at(
            filesystem,
            source,
            0x1_0000_0002,
            bytes.as_mut_ptr(),
            8,
            &mut count
        ),
        0
    );
    assert_eq!(count, 4);
    assert_eq!(&bytes, b"yond\xEE\xEE\xEE\xEE");

    // Preallocate two blocks: size unchanged, allocation grows by two.
    let before = blocks(filesystem, source);
    assert_eq!(before, 1);
    assert_eq!(
        afsplus_aros_preallocate(filesystem, source, 0, 8192, 3, 0),
        0
    );
    assert_eq!(blocks(filesystem, source), 3);
    assert_eq!(afsplus_aros_file_size(filesystem, source, &mut size), 0);
    assert_eq!(size, 0x1_0000_0006);
    // The fixture mount keeps the default 4096-block budget.
    assert_eq!(
        afsplus_aros_preallocate(filesystem, source, 1 << 40, 4097 * 4096, 3, 0),
        207
    );
    assert_eq!(blocks(filesystem, source), 3);

    // Every defined hint is admitted with an explicit no-effect answer.
    for hint in 0..=11 {
        let mut effect = 77;
        assert_eq!(
            afsplus_aros_advise(filesystem, source, 0, 4096, hint, &mut effect),
            0
        );
        assert_eq!(effect, AFSPLUS_AROS_ADVICE_NO_EFFECT);
    }
    let mut effect = 77;
    assert_eq!(
        afsplus_aros_advise(filesystem, source, 0, 4096, 12, &mut effect),
        115
    );
    assert_eq!(effect, 77);
    assert_eq!(
        afsplus_aros_advise(filesystem, source, 0, 4096, 0, ptr::null_mut()),
        210
    );

    // Clone range into a second file, then clone the whole file by lock.
    let target = open(filesystem, b"target", AFSPLUS_AROS_OPEN_NEW_FILE);
    assert_eq!(
        afsplus_aros_clone_range(filesystem, source, 0x1_0000_0000, target, 0, 6, 4, 0),
        0
    );
    assert_eq!(
        afsplus_aros_read_at(filesystem, target, 0, bytes.as_mut_ptr(), 8, &mut count),
        0
    );
    assert_eq!(count, 6);
    assert_eq!(&bytes[..6], b"beyond");
    assert_eq!(afsplus_aros_close(filesystem, target), 0);
    assert_eq!(afsplus_aros_close(filesystem, source), 0);

    let mut lock = 0;
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            b"target".as_ptr(),
            6,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut lock
        ),
        0
    );
    assert_eq!(
        afsplus_aros_clone_file(filesystem, lock, 0, b"twin".as_ptr(), 4, 5, 0),
        0
    );
    assert_eq!(
        afsplus_aros_clone_file(filesystem, lock, 0, b"twin".as_ptr(), 4, 5, 0),
        203
    );
    assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);

    // Replace "target" with "twin" atomically.
    assert_eq!(
        afsplus_aros_replace(
            filesystem,
            0,
            b"twin".as_ptr(),
            4,
            0,
            b"target".as_ptr(),
            6,
            6,
            0
        ),
        0
    );
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            b"twin".as_ptr(),
            4,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut lock
        ),
        205
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);

    // Control: without shared extents both clone calls are unknown actions.
    let mut device = formatted(false);
    let filesystem = mount(&mut device);
    let source = open(filesystem, b"source", AFSPLUS_AROS_OPEN_NEW_FILE);
    let target = open(filesystem, b"target", AFSPLUS_AROS_OPEN_NEW_FILE);
    assert_eq!(
        afsplus_aros_clone_range(filesystem, source, 0, target, 0, 4096, 4, 0),
        209
    );
    assert_eq!(afsplus_aros_close(filesystem, target), 0);
    assert_eq!(afsplus_aros_close(filesystem, source), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

/// (status, extents stored, complete, next offset).
fn map(
    filesystem: *mut AfsplusAros,
    file: u64,
    offset: u64,
    length: u64,
    extents: &mut [AfsplusArosExtent],
) -> (i32, u32, u32, u64) {
    let (mut stored, mut complete, mut next) = (9, 9, 9);
    let status = afsplus_aros_extent_map(
        filesystem,
        file,
        offset,
        length,
        extents.as_mut_ptr(),
        extents.len() as u32,
        &mut stored,
        &mut complete,
        &mut next,
    );
    (status, stored, complete, next)
}

#[test]
fn c_boundary_maps_extents_of_a_sparse_preallocated_file() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    let file = open(filesystem, b"paged", AFSPLUS_AROS_OPEN_NEW_FILE);
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write_at(
            filesystem,
            file,
            0x1_0000_0000,
            [7u8; 4096].as_ptr(),
            4096,
            2,
            0,
            &mut count
        ),
        0
    );
    let mut extents = [AfsplusArosExtent::default(); 4];
    // Unpublished write: refused by value, nothing stored.
    assert_eq!(
        map(filesystem, file, 0, u64::MAX, &mut extents),
        (202, 9, 9, 9)
    );
    assert_eq!(extents[0], AfsplusArosExtent::default());
    assert_eq!(afsplus_aros_flush(filesystem), 0);
    assert_eq!(
        afsplus_aros_preallocate(filesystem, file, 8192, 8192, 3, 0),
        0
    );

    assert_eq!(
        map(filesystem, file, 4096, u64::MAX - 4096, &mut extents),
        (0, 2, 1, u64::MAX)
    );
    assert_eq!(
        extents[..2],
        [
            AfsplusArosExtent {
                offset: 8192,
                length: 8192,
                flags: AFSPLUS_AROS_EXTENT_UNWRITTEN,
                reserved: 0,
            },
            AfsplusArosExtent {
                offset: 0x1_0000_0000,
                length: 4096,
                flags: 0,
                reserved: 0,
            },
        ]
    );
    assert_eq!(extents[2], AfsplusArosExtent::default());

    // Capacity one: incomplete, and the next offset continues it.
    let mut one = [AfsplusArosExtent::default(); 1];
    assert_eq!(
        map(filesystem, file, 0, u64::MAX, &mut one),
        (0, 1, 0, 16384)
    );
    assert_eq!(one[0].offset, 8192);
    assert_eq!(
        map(filesystem, file, 16384, u64::MAX - 16384, &mut one),
        (0, 1, 1, u64::MAX)
    );
    assert_eq!(one[0].offset, 0x1_0000_0000);
    assert_eq!(map(filesystem, file, 0, 0, &mut one).0, 115);
    assert_eq!(afsplus_aros_close(filesystem, file), 0);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
