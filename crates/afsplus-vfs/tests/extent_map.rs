//! Semantic extent query: written, reserved and hole by byte range.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::{AccessMode, ExtentRange, Vfs, VfsError};

const BLOCK: u64 = 4096;

fn now() -> Timespec {
    Timespec {
        seconds: 1,
        nanoseconds: 0,
    }
}

fn mounted() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xE1; 16],
            label: "Extents".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: now(),
        },
    )
    .unwrap();
    Vfs::mount(device, MountOptions::default()).unwrap()
}

fn written(offset: u64, length: u64) -> ExtentRange {
    ExtentRange {
        offset,
        length,
        unwritten: false,
    }
}

#[test]
fn extent_map_reports_written_reserved_and_holes_clipped_to_the_query() {
    let mut vfs = mounted();
    let object = vfs.create_file(vfs.root_object(), "sparse", now()).unwrap();
    let file = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    // Blocks 0-1 written, 2-9 hole, 10 written, 20-23 reserved.
    vfs.write(file, 0, &[1u8; 2 * 4096], now()).unwrap();
    vfs.write(file, 10 * BLOCK, &[2u8; 100], now()).unwrap();

    // Unpublished writes: the committed map is not served and nothing is
    // committed behind the caller's back.
    assert_eq!(vfs.extent_map(file, 0, 1 << 20, 64), Err(VfsError::Busy));
    vfs.sync_filesystem().unwrap();
    vfs.preallocate(file, 20 * BLOCK, 4 * BLOCK, 64, now())
        .unwrap();

    let whole = vfs.extent_map(file, 0, u64::MAX, 64).unwrap();
    assert!(whole.complete);
    assert_eq!(
        whole.ranges,
        vec![
            written(0, 2 * BLOCK),
            written(10 * BLOCK, BLOCK),
            ExtentRange {
                offset: 20 * BLOCK,
                length: 4 * BLOCK,
                unwritten: true,
            },
        ]
    );

    // Clipping on both sides, inside one mapping and across a hole.
    let clipped = vfs
        .extent_map(file, BLOCK + 10, 9 * BLOCK + 90, 64)
        .unwrap();
    assert!(clipped.complete);
    assert_eq!(
        clipped.ranges,
        vec![written(BLOCK + 10, BLOCK - 10), written(10 * BLOCK, 100)]
    );
    // A query inside the hole is an empty complete map.
    let hole = vfs.extent_map(file, 3 * BLOCK, 5 * BLOCK, 64).unwrap();
    assert_eq!((hole.ranges.len(), hole.complete), (0, true));

    // A range budget of one yields an incomplete map; continuing from its
    // next_offset gives exactly what the unbounded query gave.
    let first = vfs.extent_map(file, 0, u64::MAX, 1).unwrap();
    assert_eq!(
        (first.ranges.clone(), first.complete, first.next_offset),
        (vec![written(0, 2 * BLOCK)], false, 2 * BLOCK)
    );
    let rest = vfs
        .extent_map(file, first.next_offset, u64::MAX - first.next_offset, 64)
        .unwrap();
    assert!(rest.complete);
    assert_eq!(rest.ranges, whole.ranges[1..]);
    assert_eq!(whole.next_offset, u64::MAX);
    assert_eq!(hole.next_offset, 8 * BLOCK);

    assert_eq!(vfs.extent_map(file, 0, 0, 64), Err(VfsError::Invalid));
    assert_eq!(
        vfs.extent_map(file, u64::MAX, 2, 64),
        Err(VfsError::Invalid)
    );
    assert_eq!(vfs.extent_map(file, 0, 1, 65), Err(VfsError::Invalid));
    assert_eq!(vfs.extent_map(999, 0, 1, 64), Err(VfsError::Stale));
}
