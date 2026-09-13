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

/// A deterministic byte-vector oracle independent of the extent implementation.
/// Seed and operation number identify failures across sparse, unwritten and COW
/// transitions; every remount also verifies authoritative allocation ownership.
#[test]
fn seeded_mixed_io_preserves_bytes_across_remounts_and_clones() {
    for seed in [0xBF5u64, 0xDEAD_BEEF, 0x1234_5678] {
        let mut state = seed;
        let mut random = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut vol = mount(formatted(8192)).unwrap();
        let object = vol.create_file_in_root("mixed", b"", ts(1)).unwrap();
        let mut expected = Vec::<u8>::new();
        let mut clone = None;
        for step in 0..192 {
            let offset = random() as usize % (32 * BS);
            let length = 1 + random() as usize % (2 * BS + 3);
            let now = ts(step + 2);
            match random() % 3 {
                0 => {
                    let data: Vec<u8> = (0..length).map(|_| random() as u8).collect();
                    vol.write_file_at(object, offset as u64, &data, now)
                        .unwrap_or_else(|error| panic!("seed={seed:#x} step={step}: {error}"));
                    if expected.len() < offset + length {
                        expected.resize(offset + length, 0);
                    }
                    expected[offset..offset + length].copy_from_slice(&data);
                }
                1 => {
                    vol.truncate_file(object, offset as u64, now).unwrap();
                    expected.resize(offset, 0);
                }
                _ => {
                    vol.preallocate_file(object, offset as u64, length as u64, now)
                        .unwrap();
                }
            }
            assert_eq!(
                vol.read_file(object).unwrap(),
                expected,
                "seed={seed:#x} step={step}"
            );
            // Short reads must neither report bytes beyond EOF nor overwrite
            // the unused part of the caller's destination buffer.
            let mut buffer = vec![0xA5; length];
            let count = vol
                .read_file_at(object, offset as u64, &mut buffer)
                .unwrap();
            let wanted = expected.len().saturating_sub(offset).min(length);
            assert_eq!(count, wanted, "seed={seed:#x} step={step}");
            if wanted > 0 {
                assert_eq!(&buffer[..count], &expected[offset..offset + count]);
            }
            assert!(buffer[count..].iter().all(|byte| *byte == 0xA5));
            if step == 63 {
                let id = vol
                    .clone_file(object, OBJECT_ROOT, "snapshot-copy", now)
                    .unwrap();
                clone = Some((id, expected.clone()));
            }
            if let Some((id, bytes)) = &clone {
                assert_eq!(
                    vol.read_file(*id).unwrap(),
                    *bytes,
                    "clone changed: seed={seed:#x} step={step}"
                );
            }
            if step % 16 == 15 {
                let mut dev = vol.into_device();
                let report = afsplus_check::check_device(&mut dev);
                assert!(
                    report.is_clean(),
                    "seed={seed:#x} step={step}: {:?}",
                    report.errors
                );
                vol = mount(dev).unwrap();
                assert_eq!(vol.read_file(object).unwrap(), expected);
            }
        }
    }
}
