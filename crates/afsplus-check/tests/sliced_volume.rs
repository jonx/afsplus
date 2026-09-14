//! A real filesystem cycle in an image partition, with surrounding sentinels.
use afsplus_block::{BlockDevice, MemoryBackend, SliceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};

#[test]
fn sliced_volume_mutations_and_remount_preserve_neighboring_blocks() {
    const BS: usize = 4096;
    for start in [1, 7] {
        let blocks = 256;
        let total = start + blocks + 3;
        let mut parent = MemoryBackend::new(BS, total);
        for lba in 0..total {
            parent.write_block(lba, &[0xa5; BS]).unwrap();
        }
        let mut slice = SliceBackend::new(parent, start, blocks).unwrap();
        let now = Timespec {
            seconds: 42,
            nanoseconds: 123,
        };
        mkfs(
            &mut slice,
            &MkfsParams {
                uuid: [9; 16],
                label: "Partition".into(),
                region_size: 64,
                reclaim_caps: Default::default(),
                log_slots: 8,
                shared_extents: true,
                data_policy: false,
                name_policy: NamePolicy::Sensitive,
                timestamp: now,
            },
        )
        .unwrap();
        let mut volume = mount(slice).unwrap();
        let id = volume
            .create_file_in_root("before", b"original", now)
            .unwrap();
        volume.write_file_at(id, 4096, b"tail", now).unwrap();
        volume
            .rename(OBJECT_ROOT, "before", OBJECT_ROOT, "after", now)
            .unwrap();
        volume.sync().unwrap();
        let mut slice = volume.into_device();
        let report = check_device(&mut slice);
        assert!(report.is_clean(), "{:?}", report.errors);
        let mut volume = mount(slice).unwrap();
        assert_eq!(volume.lookup_root("before").unwrap(), None);
        assert_eq!(volume.lookup_root("after").unwrap(), Some(id));
        let mut expected = vec![0; 4100];
        expected[..8].copy_from_slice(b"original");
        expected[4096..].copy_from_slice(b"tail");
        assert_eq!(volume.read_file(id).unwrap(), expected);
        volume.truncate_file(id, 5, now).unwrap();
        volume.sync().unwrap();
        let mut volume = mount(volume.into_device()).unwrap();
        assert_eq!(volume.read_file(id).unwrap(), b"origi");
        let mut slice = volume.into_device();
        assert!(check_device(&mut slice).is_clean());
        let parent = slice.into_inner();
        for lba in (0..start).chain(start + blocks..total) {
            assert_eq!(parent.peek(lba), vec![0xa5; BS], "outside block {lba}");
        }
    }
}
