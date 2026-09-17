//! `Volume::file_allocation_from`: the committed allocation of a file read
//! from a byte offset in one tree descent.

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_core::volume::FileAllocationRange;
use afsplus_core::{mkfs, mount_with_options, MkfsParams, MountOptions, NamePolicy, Volume};
use afsplus_format::Timespec;

const BLOCK: u64 = 4096;
/// Written blocks 0, 2, 4, ...: every one is its own extent, because the odd
/// blocks between them are holes.
const FRAGMENTS: u64 = 600;

fn now() -> Timespec {
    Timespec {
        seconds: 1,
        nanoseconds: 0,
    }
}

fn mounted(blocks: u64) -> Volume<TraceBackend<MemoryBackend>> {
    let mut device = TraceBackend::new(MemoryBackend::new(4096, blocks));
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xE5; 16],
            label: "Seek".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: false,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: now(),
        },
    )
    .unwrap();
    mount_with_options(device, MountOptions::default()).unwrap()
}

fn written(block: u64) -> FileAllocationRange {
    FileAllocationRange {
        offset: block * BLOCK,
        length: BLOCK,
        unwritten: false,
    }
}

#[test]
fn a_far_offset_in_a_fragmented_file_is_one_descent() {
    let mut volume = mounted(32768);
    let file = volume
        .create_file_in_root("fragmented", b"", now())
        .unwrap();
    for index in 0..FRAGMENTS {
        volume
            .write_file_at(file, 2 * index * BLOCK, &[index as u8; 4096], now())
            .unwrap();
        // The trace keeps every event; only the counters matter here.
        volume.device_mut().reset();
    }

    // Far into the file, starting inside a written block: fragment 500 is
    // block 1000, the next ones are every second block.
    let far = volume
        .file_allocation_from(file, 1000 * BLOCK + 17, 3)
        .unwrap();
    assert_eq!(
        far.ranges,
        vec![written(1000), written(1002), written(1004)]
    );
    assert_eq!((far.next, far.eof), (1005 * BLOCK, false));
    // Starting in the hole after a fragment returns the next fragment.
    let hole = volume.file_allocation_from(file, 1001 * BLOCK, 1).unwrap();
    assert_eq!(hole.ranges, vec![written(1002)]);
    // The last fragment, then nothing: inside EOF, at EOF, far past it.
    let last = 2 * (FRAGMENTS - 1);
    let tail = volume.file_allocation_from(file, last * BLOCK, 8).unwrap();
    assert_eq!((tail.ranges, tail.eof), (vec![written(last)], true));
    for beyond in [(last + 1) * BLOCK, (last + 2) * BLOCK, u64::MAX - 5] {
        let none = volume.file_allocation_from(file, beyond, 8).unwrap();
        assert_eq!((none.ranges.len(), none.eof, none.next), (0, true, beyond));
    }

    // Cross-check: the seek walk over the whole file, resumed from `next`,
    // equals the ordinal walk entry for entry.
    let mut by_ordinal = Vec::new();
    let mut cursor = 0;
    loop {
        let page = volume.file_allocation_page(file, cursor, 64).unwrap();
        by_ordinal.extend(page.ranges);
        cursor = page.next;
        if page.eof {
            break;
        }
    }
    let mut by_offset = Vec::new();
    let mut offset = 0;
    loop {
        let page = volume.file_allocation_from(file, offset, 64).unwrap();
        by_offset.extend(page.ranges);
        offset = page.next;
        if page.eof {
            break;
        }
    }
    assert_eq!(by_ordinal.len(), FRAGMENTS as usize);
    assert_eq!(by_offset, by_ordinal);

    // Cost: the far query by offset reads a handful of tree blocks; reaching
    // the same place by ordinal reads every leaf before it.
    volume.device_mut().reset();
    volume.file_allocation_from(file, 1000 * BLOCK, 3).unwrap();
    let seek_reads = volume.device_mut().stats().reads;
    volume.device_mut().reset();
    let mut cursor = 0;
    while cursor < 500 {
        cursor = volume.file_allocation_page(file, cursor, 64).unwrap().next;
    }
    let walk_reads = volume.device_mut().stats().reads;
    assert!(seek_reads <= 8, "seek read {seek_reads} blocks");
    assert!(
        walk_reads >= 4 * seek_reads,
        "walk {walk_reads} against seek {seek_reads}"
    );

    assert!(volume.file_allocation_from(file, 0, 0).is_err());
    assert!(volume.file_allocation_from(file, 0, 65).is_err());
    assert!(volume.file_allocation_from(0xDEAD, 0, 1).is_err());
}

#[test]
fn files_without_an_extent_tree_and_empty_files_answer_the_same_question() {
    let mut volume = mounted(8192);
    let direct = volume.create_file_in_root("direct", b"", now()).unwrap();
    volume
        .write_file_at(direct, 0, &[7u8; 3 * 4096], now())
        .unwrap();
    let whole = FileAllocationRange {
        offset: 0,
        length: 3 * BLOCK,
        unwritten: false,
    };
    // One contiguous run, whatever byte inside it is asked for.
    for offset in [0, 1, 2 * BLOCK + 9] {
        let page = volume.file_allocation_from(direct, offset, 4).unwrap();
        assert_eq!(
            (page.ranges, page.next, page.eof),
            (vec![whole], 3 * BLOCK, true)
        );
    }
    let past = volume.file_allocation_from(direct, 3 * BLOCK, 4).unwrap();
    assert_eq!((past.ranges.len(), past.eof), (0, true));
    // The ordinal reader agrees.
    assert_eq!(
        volume.file_allocation_page(direct, 0, 4).unwrap().ranges,
        vec![whole]
    );

    let empty = volume.create_file_in_root("empty", b"", now()).unwrap();
    let page = volume.file_allocation_from(empty, 0, 4).unwrap();
    assert_eq!((page.ranges.len(), page.eof, page.next), (0, true, 0));
    let root = afsplus_format::OBJECT_ROOT;
    assert!(volume.file_allocation_from(root, 0, 4).is_err());
}
