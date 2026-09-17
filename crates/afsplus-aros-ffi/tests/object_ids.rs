//! Object-ID group through the exported C functions.

mod common;

use std::mem::size_of;
use std::ptr;

use afsplus_aros_ffi::*;
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

/// Decodes the packed records exactly as a C caller walks them.
fn records(buffer: &[u8], count: u32) -> Vec<(u64, u32, String)> {
    let mut at = 0usize;
    let mut out = Vec::new();
    for _ in 0..count {
        let mut header = AfsplusArosDirEntry::default();
        // SAFETY: the record header is plain data; the copy is byte-wise.
        unsafe {
            ptr::copy_nonoverlapping(
                buffer[at..].as_ptr(),
                ptr::from_mut(&mut header).cast::<u8>(),
                size_of::<AfsplusArosDirEntry>(),
            );
        }
        assert_eq!(header.reserved, 0);
        assert_eq!(header.record_length % 8, 0);
        let name = &buffer[at + 24..at + 24 + header.name_length as usize];
        out.push((
            header.object_id,
            header.kind,
            String::from_utf8(name.to_vec()).unwrap(),
        ));
        at += header.record_length as usize;
    }
    out
}

#[test]
fn c_boundary_looks_up_stats_and_pages_a_directory_by_object_id() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    for name in [&b"alpha"[..], b"beta", b"gamma", b"delta"] {
        create(filesystem, name);
    }

    let mut beta = 0;
    assert_eq!(
        afsplus_aros_lookup_id(filesystem, 0, b"BETA".as_ptr(), 4, &mut beta),
        0
    );
    let mut stat = AfsplusArosStat {
        struct_size: size_of::<AfsplusArosStat>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_stat_id(filesystem, beta, &mut stat), 0);
    assert_eq!(
        (
            stat.struct_size,
            stat.kind,
            stat.object_id,
            stat.size,
            stat.links
        ),
        (88, AFSPLUS_AROS_KIND_FILE, beta, 0, 1)
    );
    assert_eq!(stat.created_seconds, 1);
    assert_eq!(
        afsplus_aros_stat_id(filesystem, 0xDEAD_BEEF, &mut stat),
        205
    );
    assert_eq!(
        afsplus_aros_lookup_id(filesystem, 0, b"\xFF".as_ptr(), 1, &mut beta),
        210
    );

    let mut dir = 0;
    assert_eq!(afsplus_aros_dir_open(filesystem, 0, &mut dir), 0);
    let mut buffer = [0xEEu8; 2 * 280];
    let (mut count, mut eof) = (9, 9);
    // The buffer is certain to hold two records, so two entries are read
    // although eight were asked for.
    assert_eq!(
        afsplus_aros_dir_read(
            filesystem,
            dir,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            8,
            &mut count,
            &mut eof
        ),
        0
    );
    assert_eq!((count, eof), (2, 0));
    let first = records(&buffer, count);
    assert_eq!(first[0].2, "alpha");
    assert_eq!(first[1], (beta, AFSPLUS_AROS_KIND_FILE, "beta".into()));
    // "alpha" record: 24 + 5 bytes padded to 32; the byte after the second
    // record (24 + 4 -> 32) is untouched.
    assert_eq!(buffer[64], 0xEE);

    // A namespace change between two pages does not disturb the walk.
    assert_eq!(
        afsplus_aros_delete_object(filesystem, 0, b"beta".as_ptr(), 4, 2, 0),
        0
    );
    create(filesystem, b"aaa");
    assert_eq!(
        afsplus_aros_dir_read(
            filesystem,
            dir,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            8,
            &mut count,
            &mut eof
        ),
        0
    );
    assert_eq!((count, eof), (2, 1));
    let second = records(&buffer, count);
    assert_eq!(
        second
            .iter()
            .map(|entry| entry.2.as_str())
            .collect::<Vec<_>>(),
        ["delta", "gamma"]
    );
    assert_eq!(
        afsplus_aros_dir_read(
            filesystem,
            dir,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            8,
            &mut count,
            &mut eof
        ),
        0
    );
    assert_eq!((count, eof), (0, 1));

    // A buffer that cannot certainly hold one record is refused and nothing
    // is consumed from the walk.
    assert_eq!(
        afsplus_aros_dir_read(
            filesystem,
            dir,
            buffer.as_mut_ptr(),
            279,
            8,
            &mut count,
            &mut eof
        ),
        115
    );
    assert_eq!(afsplus_aros_dir_close(filesystem, dir), 0);
    assert_eq!(afsplus_aros_dir_close(filesystem, dir), 211);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
