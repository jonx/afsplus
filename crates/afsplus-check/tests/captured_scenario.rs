use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_check::scenario::captured::{inspect, Limits};
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{
    mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MkfsParams, MountOptions,
    NamePolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use std::num::NonZeroUsize;

fn work() -> SnapshotWorkLimits {
    SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 16,
        reclaim_records: 8,
    }
}
fn limits() -> Limits {
    Limits {
        entries: 4,
        bytes: 16,
        ranges: 2,
        page_entries: 2,
    }
}

#[test]
fn captured_inspection_preserves_aliases_targets_and_bytes_after_remount() {
    for pages in [2, 4, 8, usize::MAX] {
        let mut image = MemoryBackend::new(4096, 512);
        mkfs_with_options(
            &mut image,
            &MkfsParams {
                uuid: [0x54; 16],
                label: "Captured scenario".into(),
                region_size: 64,
                reclaim_caps: Default::default(),
                log_slots: 8,
                shared_extents: true,
                data_policy: false,
                name_policy: NamePolicy::Sensitive,
                timestamp: Timespec::default(),
            },
            MkfsOptions {
                persistent_snapshots: true,
            },
        )
        .unwrap();
        let options = MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        };
        let mut volume = mount_with_snapshot_limits(image, options, work()).unwrap();
        let now = Timespec::default();
        let file = volume.create_file_in_root("file", b"before", now).unwrap();
        volume.link_file(file, OBJECT_ROOT, "alias", now).unwrap();
        volume.create_directory(OBJECT_ROOT, "dir", now).unwrap();
        volume
            .create_symlink(OBJECT_ROOT, "link", "file", now)
            .unwrap();
        let id = volume.snapshot_create(now).unwrap();
        volume.write_file_at(file, 0, b"after!", now).unwrap();
        let image = volume.into_device();
        let mut volume =
            mount_with_snapshot_limits(TraceBackend::new(image), options, work()).unwrap();
        let handle = volume.snapshot_open(id).unwrap();
        let view = inspect(&mut volume, &handle, limits()).unwrap();
        assert_eq!(view.info, handle.info());
        assert_eq!(
            view.entries
                .iter()
                .map(|e| e.path[0].as_str())
                .collect::<Vec<_>>(),
            ["alias", "dir", "file", "link"]
        );
        for entry in [&view.entries[0], &view.entries[2]] {
            assert_eq!(entry.metadata.object_id, file);
            assert_eq!(entry.metadata.link_count, 2);
            assert_eq!(entry.data, b"before");
            assert_eq!(entry.allocation.len(), 1);
            assert!(!entry.allocation[0].unwritten);
        }
        assert_eq!(view.entries[3].data, b"file");
        assert!(view.entries[1].data.is_empty());
        for budget in [
            Limits {
                entries: 3,
                ..limits()
            },
            Limits {
                bytes: 15,
                ..limits()
            },
            Limits {
                ranges: 1,
                ..limits()
            },
            Limits {
                page_entries: 0,
                ..limits()
            },
        ] {
            assert!(inspect(&mut volume, &handle, budget).is_err());
        }
        assert_eq!(inspect(&mut volume, &handle, limits()).unwrap(), view);
        let trace = volume.into_device();
        assert_eq!(trace.stats().writes, 0);
        assert_eq!(trace.stats().flushes, 0);
    }
}

#[test]
fn snapshot_scenario_reopens_persistent_views_after_remount() {
    use afsplus_check::scenario::Plan;
    use afsplus_core::flight::EventKind;
    for pages in ["2", "4", "8", "unlimited"] {
        let wire = format!("AFSPSC07\nformat 4096 512 64 8 {pages} 256 127 0 none 4096 16 8\ncreate f root 66696c65 6265666f7265\nsnapshot_create s\nsnapshot_open s\nwrite f 0 616674657221\nsnapshot_inspect s\nremount\nsnapshot_open s\nsnapshot_inspect s\nsnapshot_close s\nsnapshot_delete s\n");
        let plan = Plan::parse(wire.as_bytes()).unwrap();
        let run = plan.run().unwrap();
        assert!(run.failure.is_none(), "{:?}", run.failure);
        for index in [4, 7] {
            let events = &run.events[index].flight.as_ref().unwrap().events;
            assert!(events
                .iter()
                .any(|event| event.kind == EventKind::ObjectMapped
                    && event.object.is_some_and(|object| object.view_id == 1)));
        }
        let mut volume =
            mount_with_snapshot_limits(run.result, MountOptions::default(), work()).unwrap();
        assert!(volume.snapshot_list(0, 16).unwrap().entries.is_empty());
        let file = volume.lookup_root("file").unwrap().unwrap();
        assert_eq!(volume.read_file(file).unwrap(), b"after!");
    }
}

#[test]
fn captured_registry_pages_preserve_distinct_history_with_aggregate_budgets() {
    use afsplus_check::scenario::{captured::inspect_all, Plan};
    for pages in ["2", "4", "8", "unlimited"] {
        let wire = format!("AFSPSC07\nformat 4096 512 64 8 {pages} 256 127 0 none 4096 16 8\ncreate f root 66696c65 61\nsnapshot_create first\nwrite f 0 62\nsnapshot_create second\nwrite f 0 63\nsnapshot_create third\nwrite f 0 64\nremount\n");
        let plan = Plan::parse(wire.as_bytes()).unwrap();
        let run = plan.run().unwrap();
        assert!(run.failure.is_none(), "{:?}", run.failure);
        let mut volume = mount_with_snapshot_limits(
            TraceBackend::new(run.result),
            MountOptions::default(),
            work(),
        )
        .unwrap();
        let budget = Limits {
            entries: 3,
            bytes: 3,
            ranges: 3,
            page_entries: 1,
        };
        let views = inspect_all(&mut volume, 3, budget).unwrap();
        assert_eq!(views.len(), 3);
        for (index, view) in views.iter().enumerate() {
            assert_eq!(view.info.id, index as u64 + 1);
            assert_eq!(view.entries.len(), 1);
            assert_eq!(view.entries[0].path, ["file"]);
            assert_eq!(view.entries[0].data, [b'a' + index as u8]);
        }
        assert!(inspect_all(&mut volume, 2, budget).is_err());
        for limited in [
            Limits {
                entries: 2,
                ..budget
            },
            Limits { bytes: 2, ..budget },
            Limits {
                ranges: 2,
                ..budget
            },
        ] {
            assert!(inspect_all(&mut volume, 3, limited).is_err());
        }
        assert_eq!(inspect_all(&mut volume, 3, budget).unwrap(), views);
        let trace = volume.into_device();
        assert_eq!(trace.stats().writes, 0);
        assert_eq!(trace.stats().flushes, 0);
    }
}
