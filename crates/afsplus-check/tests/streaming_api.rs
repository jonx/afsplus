//! Core primitives required by the portable VFS adapters.

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_core::volume::{BatchOp, DirectoryCursor};
use afsplus_core::{mkfs, mount, CoreError, MkfsParams};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(total: u64) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, total);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [93u8; 16],
            label: "StreamingApi".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

#[test]
fn read_at_handles_boundaries_holes_and_eof_without_a_file_sized_buffer() {
    let mut vol = mount(formatted(8192)).unwrap();
    let original: Vec<u8> = (0..3 * BS).map(|index| (index % 251) as u8).collect();
    let object = vol.create_file_in_root("data", &original, ts(1)).unwrap();

    let mut crossing = [0u8; 19];
    let read = vol
        .read_file_at(object, BS as u64 - 7, &mut crossing)
        .unwrap();
    assert_eq!(read, crossing.len());
    assert_eq!(&crossing, &original[BS - 7..BS - 7 + crossing.len()]);

    let sparse_offset = 6 * BS as u64 + 17;
    vol.write_file_at(object, sparse_offset, b"tail", ts(2))
        .unwrap();
    let mut sparse = vec![0xAA; 3 * BS + 32];
    let read = vol
        .read_file_at(object, 3 * BS as u64, &mut sparse)
        .unwrap();
    assert_eq!(read, 3 * BS + 21);
    assert!(sparse[..3 * BS + 17].iter().all(|byte| *byte == 0));
    assert_eq!(&sparse[3 * BS + 17..3 * BS + 21], b"tail");
    assert_eq!(
        vol.read_file_at(object, sparse_offset + 4, &mut sparse)
            .unwrap(),
        0
    );
}

#[test]
fn directory_pages_are_bounded_complete_and_generation_checked() {
    let mut vol = mount(TraceBackend::new(formatted(16_384))).unwrap();
    let names: Vec<String> = (0..1000).map(|index| format!("file-{index:04}")).collect();
    let ops: Vec<_> = names
        .iter()
        .map(|name| BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name,
            content: b"",
        })
        .collect();
    vol.run_batch(&ops, ts(1)).unwrap();

    vol.device_mut().reset();
    let first = vol.read_directory_page(OBJECT_ROOT, None, 10).unwrap();
    assert_eq!(first.entries.len(), 10);
    assert!(!first.eof);
    assert!(
        vol.device_mut().stats().reads < 20,
        "page read walked the directory"
    );

    let mut all = first
        .entries
        .iter()
        .map(|entry| String::from_utf8(entry.name.clone()).unwrap())
        .collect::<Vec<_>>();
    let mut cursor = first.next;
    loop {
        let page = vol
            .read_directory_page(OBJECT_ROOT, Some(cursor), 37)
            .unwrap();
        all.extend(
            page.entries
                .iter()
                .map(|entry| String::from_utf8(entry.name.clone()).unwrap()),
        );
        cursor = page.next;
        if page.eof {
            break;
        }
    }
    assert_eq!(all, names);

    let stale = DirectoryCursor {
        generation: cursor.generation,
        ordinal: 0,
    };
    vol.create_file_in_root("later", b"", ts(2)).unwrap();
    assert!(matches!(
        vol.read_directory_page(OBJECT_ROOT, Some(stale), 1),
        Err(CoreError::Stale)
    ));
}
