//! Same checked wide-name batch under each staged-tree profile.
use super::*;
use afsplus_core::{mount_with_options, volume::BatchOp, MountOptions};

fn name(i: usize) -> String {
    format!("{i:04}-{}", "n".repeat(180))
}

pub fn run(pages: usize) {
    let io = Cell::new(IoStats::default());
    let mut rows = Vec::with_capacity(12);
    let mut dev = Image {
        bytes: vec![0; BLOCKS as usize * BS],
        io: &io,
    };
    let options = MountOptions {
        tree_cache_pages: std::num::NonZeroUsize::new(pages),
        ..Default::default()
    };
    let (row, ()) = phase("format", &io, || {
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [0x4d; 16],
                label: "CacheMeasure".into(),
                region_size: 256,
                reclaim_caps: Default::default(),
                log_slots: 8,
                shared_extents: true,
                data_policy: false,
                name_policy: NamePolicy::Sensitive,
                timestamp: ts(0),
            },
        )
        .unwrap();
    });
    rows.push(row);
    let (row, mut volume) = phase("mount", &io, || mount_with_options(dev, options).unwrap());
    rows.push(row);
    let (mut row, ()) = phase("batch-create", &io, || {
        let names: Vec<_> = (0..192).map(name).collect();
        let operations: Vec<_> = names
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content: b"payload",
            })
            .collect();
        volume.run_batch(&operations, ts(1)).unwrap();
    });
    row.payload_written = 192 * 7;
    let create_stats = volume.last_commit_stats().unwrap();
    assert_eq!(row.io.bytes_written, create_stats.bytes_written);
    assert_eq!(row.io.flushes, create_stats.flushes);
    assert!(create_stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
    if pages != usize::MAX {
        assert!(create_stats.tree_mutations.staged_spill_writes > 0);
    }
    rows.push(row);
    let (row, ()) = phase("batch-delete", &io, || {
        let names: Vec<_> = (1..192).step_by(2).map(name).collect();
        let operations: Vec<_> = names
            .iter()
            .map(|name| BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name,
            })
            .collect();
        volume.run_batch(&operations, ts(2)).unwrap();
    });
    let delete_stats = volume.last_commit_stats().unwrap();
    assert_eq!(row.io.bytes_written, delete_stats.bytes_written);
    assert_eq!(row.io.flushes, delete_stats.flushes);
    assert!(delete_stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
    let bitmap_peak = volume.allocator_ram_bytes();
    rows.push(row);
    let (row, mut dev) = phase("unmount", &io, || volume.into_device());
    rows.push(row);
    let (row, ()) = phase("raw-check", &io, || {
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "raw checker: {:?}", report.errors);
    });
    rows.push(row);
    let (row, mut volume) = phase("remount", &io, || mount_with_options(dev, options).unwrap());
    rows.push(row);
    let (mut row, ()) = phase("read-verify", &io, || {
        assert_eq!(volume.list_root().unwrap().len(), 96);
        for i in 0..192 {
            let id = volume.lookup_root(&name(i)).unwrap();
            if i % 2 == 0 {
                assert_eq!(volume.read_file(id.unwrap()).unwrap(), b"payload");
            } else {
                assert!(id.is_none());
            }
        }
    });
    row.payload_read = 96 * 7;
    rows.push(row);
    let (row, mut dev) = phase("final-unmount", &io, || volume.into_device());
    rows.push(row);
    let (row, ()) = phase("recovered-check", &io, || {
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "recovered checker: {:?}", report.errors);
    });
    rows.push(row);
    report(
        "tree-cache-batch-v1",
        &dev,
        bitmap_peak,
        &rows,
        Some((pages, [create_stats, delete_stats])),
    );
}
