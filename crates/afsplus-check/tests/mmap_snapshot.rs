// Copyright (C) 2026 John KNIPPER <code@jkn.me>.
// SPDX-License-Identifier: APL-1.1
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};

fn time(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 17,
    }
}

#[test]
fn unnamed_mmap_snapshot_shares_data_and_survives_source_rewrite_and_unlink() {
    let mut device = MemoryBackend::new(4096, 4096);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xA7; 16],
            label: "MmapSnapshot".into(),
            region_size: 2048,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(0),
        },
    )
    .unwrap();
    let mut volume = mount(device).unwrap();
    let source = volume
        .create_file_in_root("source", &[0x35; 3 * 4096], time(1))
        .unwrap();
    let clone = volume.clone_file_unlinked(source, time(2)).unwrap();
    assert_ne!(source, clone);
    let stats = volume.last_commit_stats().unwrap();
    assert_eq!(
        stats.data_blocks_written, 0,
        "snapshot must not copy file data"
    );
    assert_eq!(stats.shared_refs.blocks_newly_shared, 3);
    assert_eq!(volume.list_root().unwrap(), vec![("source".into(), source)]);
    assert!(volume.orphan_object(clone).unwrap());
    assert_eq!(volume.orphan_count().unwrap(), 1);
    volume
        .write_file_at(source, 4096, &[0x6a; 4096], time(3))
        .unwrap();
    volume.delete_file(OBJECT_ROOT, "source", time(4)).unwrap();
    assert_eq!(volume.read_file(clone).unwrap(), vec![0x35; 3 * 4096]);
    let mut device = volume.into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut volume = mount(device).unwrap();
    assert_eq!(volume.read_file(clone).unwrap(), vec![0x35; 3 * 4096]);
    volume.cleanup_orphan(clone, time(5)).unwrap();
    assert_eq!(volume.orphan_count().unwrap(), 0);
    let mut device = volume.into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}
