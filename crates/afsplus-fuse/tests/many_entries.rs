use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{FuseAdapter, FuseConfig};
use afsplus_vfs::Vfs;

fn at(s: i64) -> Timespec {
    Timespec {
        seconds: s,
        nanoseconds: 0,
    }
}

fn adapter() -> FuseAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 32768);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [7; 16],
            label: "Many".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig::default(),
    )
}

#[test]
fn a_directory_of_a_thousand_entries_lists_all_of_them() {
    let mut a = adapter();
    let dir = a
        .create_directory(OBJECT_ROOT, b"thousand", None, at(1))
        .unwrap()
        .object_id;
    for i in 0..1000 {
        a.create_directory(dir, format!("entry-{i}").as_bytes(), None, at(2))
            .unwrap();
    }
    // Read the whole directory the way a host does: pages until eof.
    let handle = a.open_directory(dir).unwrap();
    let mut seen = 0usize;
    let mut offset = 0u64;
    for _ in 0..4000 {
        let page = a.read_directory(dir, handle, offset, 32).unwrap();
        if page.is_empty() {
            break;
        }
        offset = page.last().unwrap().next_offset;
        seen += page
            .iter()
            .filter(|e| e.name != b"." && e.name != b"..")
            .count();
    }
    a.close(handle).unwrap();
    assert_eq!(seen, 1000, "every entry a person made must be listed");
}
