//! Semantic runner admission, exact state and bounded failure evidence.
use afsplus_block::{BlockDevice, RecordedOp};
use afsplus_check::scenario::{Plan, RecordingLimits};
use afsplus_core::mount;

const LADDER: &[u8] = b"AFSPSC01\nformat 4096 256 64 8\nmkdir d root 737263\ncreate f d 636166c3a9 00ff\nwrite f 4 42\ntruncate f 2\nrename f root 6f7574\nrmdir d\ncreate spare root 74656d70 -\nunlink spare\nsync\nremount\n";

#[test]
fn internal_flight_correlates_operations_without_changing_profile_results() {
    use afsplus_core::flight::EventKind;
    for profile in ["2", "4", "8", "unlimited"] {
        let wire = format!("AFSPSC02\nformat 4096 256 64 8 {profile}\ncreate a root 61 01\nremount\ncreate b root 62 02\n");
        let plan = Plan::parse(wire.as_bytes()).unwrap();
        let plain = plan.run().unwrap();
        assert!(plain.events.iter().all(|e| e.flight.is_none()));
        for capacity in [1, 6, 256] {
            let observed = plan
                .run_with_flight(RecordingLimits::default(), capacity)
                .unwrap();
            assert!(observed.failure.is_none());
            assert_eq!(plain.log.len(), observed.log.len());
            for (a, b) in plain.log.iter().zip(&observed.log) {
                match (a, b) {
                    (RecordedOp::Flush, RecordedOp::Flush) => (),
                    (
                        RecordedOp::Write { lba: a, data: x },
                        RecordedOp::Write { lba: b, data: y },
                    ) => {
                        assert_eq!(a, b);
                        assert_eq!(x, y);
                    }
                    _ => panic!("diagnostics changed block operations"),
                }
            }
            for lba in 0..256 {
                assert_eq!(plain.result.peek(lba), observed.result.peek(lba));
            }
            let batches: Vec<_> = observed
                .events
                .iter()
                .map(|e| e.flight.as_ref().unwrap())
                .collect();
            assert!(
                batches[1].events.is_empty(),
                "remount does not invent coverage"
            );
            let first = batches[0].events.last().unwrap();
            let last = batches[2].events.last().unwrap();
            assert_eq!((first.sequence, first.attempt), (6, 1));
            assert_eq!((last.sequence, last.attempt), (12, 2));
            assert_eq!(last.kind, EventKind::Adopted);
            assert_eq!(batches[2].dropped_total, if capacity == 1 { 10 } else { 0 });
            assert_eq!(batches[0].events.len(), capacity.min(6));
            assert_eq!(batches[2].events.len(), capacity.min(6));
        }
        for capacity in [0, 257, usize::MAX] {
            assert!(plan
                .run_with_flight(RecordingLimits::default(), capacity)
                .is_err());
        }
    }
}

#[test]
fn diagnostic_capture_preserves_failed_commit_evidence() {
    use afsplus_core::flight::EventKind;
    let plan = Plan::parse(b"AFSPSC01\nformat 4096 256 64 8\ncreate a root 61 01\n").unwrap();
    let run = plan
        .run_with_flight(
            RecordingLimits {
                operations: 0,
                payload_bytes: 0,
            },
            8,
        )
        .unwrap();
    assert!(run.failure.is_some());
    assert!(!run.events[0].success);
    let trace = run.events[0].flight.as_ref().unwrap();
    assert_eq!(trace.events.first().unwrap().kind, EventKind::Begin);
    assert_eq!(trace.events.last().unwrap().kind, EventKind::Failed);
    assert!(!trace.events.last().unwrap().requires_remount);
    assert_eq!(trace.dropped_total, 0);
    for lba in 0..256 {
        assert_eq!(run.base.peek(lba), run.result.peek(lba));
    }
}

#[test]
fn version_two_ladder_and_checked_observation_preserve_every_cache_profile() {
    for (profile, pages) in [("2", 2), ("4", 4), ("8", 8), ("unlimited", usize::MAX)] {
        let wire = std::str::from_utf8(LADDER).unwrap().replacen(
            "AFSPSC01\nformat 4096 256 64 8",
            &format!("AFSPSC02\nformat 4096 256 64 8 {profile}"),
            1,
        );
        let plan = Plan::parse(wire.as_bytes()).unwrap();
        assert_eq!(plan.cache_profile(), Some(pages));
        let run = plan.run().unwrap();
        assert!(run.failure.is_none());
        let observed = plan.inspect_checked(run.result, 1024);
        assert!(observed.is_clean());
        assert_eq!(observed.cache_pages, Some(pages));
        let entries = observed.entries.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, ["out"]);
        assert_eq!(entries[0].data, Some(vec![0, 255]));
    }
    for profile in ["0", "1", "3", "02", "2.0", "Unlimited", "8 extra"] {
        assert!(
            Plan::parse(format!("AFSPSC02\nformat 4096 256 64 8 {profile}\n").as_bytes()).is_err()
        );
    }
    assert!(Plan::parse(b"AFSPSC01\nformat 4096 256 64 8 2\n").is_err());
}

#[test]
fn constrained_profile_changes_real_spill_order_before_and_after_remount() {
    fn run(profile: &str) -> afsplus_check::scenario::Run {
        use std::fmt::Write;
        let mut wire = format!("AFSPSC02\nformat 4096 1024 256 8 {profile}\n");
        for i in 0..120 {
            if i == 60 {
                wire.push_str("remount\n");
            }
            let name = format!("{i:04}-{}", "n".repeat(180));
            let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
            writeln!(wire, "create f{i} root {hex} 01").unwrap();
        }
        let plan = Plan::parse(wire.as_bytes()).unwrap();
        let run = plan.run().unwrap();
        assert!(run.failure.is_none(), "{:?}", run.failure);
        let observed = plan.inspect_checked(run.result.clone(), 1024);
        assert!(observed.is_clean());
        let entries = observed.entries.unwrap();
        assert_eq!(entries.len(), 120);
        assert!(entries.iter().all(|entry| entry.data == Some(vec![1])));
        run
    }
    let tiny = run("2");
    let normal = run("unlimited");
    fn signature(run: &afsplus_check::scenario::Run, index: usize) -> Vec<Option<(u64, u32)>> {
        let event = &run.events[index];
        run.log[event.first_block_operation..event.end_block_operation]
            .iter()
            .map(|op| match op {
                RecordedOp::Write { lba, data } => {
                    Some((*lba, afsplus_format::crc32c::crc32c(data)))
                }
                RecordedOp::Flush => None,
            })
            .collect()
    }
    for range in [0..60, 61..121] {
        assert!(
            range
                .into_iter()
                .any(|index| signature(&tiny, index) != signature(&normal, index)),
            "configured cache must affect actual I/O on both sides of remount"
        );
    }
}

#[test]
fn image_ladder_and_recorded_replay_have_exact_namespace_and_data() {
    let run = Plan::parse(LADDER).unwrap().run().unwrap();
    assert_eq!(run.failure, None);
    let mut replay = run.base;
    for op in &run.log {
        match op {
            RecordedOp::Write { lba, data } => replay.write_block(*lba, data).unwrap(),
            RecordedOp::Flush => replay.flush().unwrap(),
        }
    }
    for lba in 0..replay.total_blocks() {
        assert_eq!(replay.peek(lba), run.result.peek(lba), "block {lba}");
    }
    for image in [replay, run.result] {
        let mut volume = mount(image).unwrap();
        let id = volume.lookup_root("out").unwrap().unwrap();
        assert_eq!(volume.read_file(id).unwrap(), [0, 255]);
        let entries = volume.list_directory(afsplus_format::OBJECT_ROOT).unwrap();
        assert_eq!(entries, vec![("out".into(), id)]);
    }
}

#[test]
fn recording_refusal_preserves_exact_written_prefix_across_remounts() {
    let plan = Plan::parse(LADDER).unwrap();
    for operations in [0, 1, 4, 16, 40] {
        let run = plan
            .run_with_limits(RecordingLimits {
                operations,
                payload_bytes: 16384,
            })
            .unwrap();
        assert!(run.failure.is_some());
        assert!(run.log.len() <= operations);
        let bytes: usize = run
            .log
            .iter()
            .map(|op| match op {
                RecordedOp::Write { data, .. } => data.len(),
                RecordedOp::Flush => 0,
            })
            .sum();
        assert!(bytes <= 16384);
        let mut replay = run.base;
        for op in run.log {
            if let RecordedOp::Write { lba, data } = op {
                replay.write_block(lba, &data).unwrap();
            }
        }
        for lba in 0..replay.total_blocks() {
            assert_eq!(replay.peek(lba), run.result.peek(lba));
        }
    }
}

#[test]
fn filesystem_failure_retains_prior_success_and_stops_following_operations() {
    let plan = Plan::parse(b"AFSPSC01\nformat 4096 256 64 8\ncreate a root 61 01\nremount\ncreate b root 61 02\ncreate c root 63 03\n").unwrap();
    let run = plan.run().unwrap();
    assert_eq!(run.failure.as_ref().map(|f| f.0), Some(2));
    assert!(!run.log.is_empty());
    let mut volume = mount(run.result).unwrap();
    let id = volume.lookup_root("a").unwrap().unwrap();
    assert_eq!(volume.read_file(id).unwrap(), [1]);
    assert_eq!(volume.lookup_root("c").unwrap(), None);
}

#[test]
fn malformed_protocol_never_reaches_execution() {
    for wire in [
        "AFSPSC01\nformat 4096 256 64 8\nshell false\n",
        "AFSPSC02\nformat 4096 256 64 8\n",
        "AFSPSC01\nformat 512 256 64 8\n",
        "AFSPSC01\nformat 4096 0256 64 8\n",
        "AFSPSC01\nformat 4096 256 63 8\n",
        "AFSPSC01\nformat 4096 256 64 8\ncreate x root 612f62 -\n",
        "AFSPSC01\nformat 4096 256 64 8\ncreate x root 61 FF\n",
        "AFSPSC01\nformat 4096 256 64 8\nwrite x 16777216 01\n",
        "AFSPSC01\nformat 4096 256 64 8\nsync",
    ] {
        assert!(Plan::parse(wire.as_bytes()).is_err(), "{wire}");
    }
}

#[test]
fn exact_observation_refuses_insufficient_content_budget() {
    use afsplus_check::scenario::{inspect, Entry};
    let run = Plan::parse(LADDER).unwrap().run().unwrap();
    assert!(inspect(run.result.clone(), 1).is_err());
    assert_eq!(
        inspect(run.result, 2).unwrap(),
        vec![Entry {
            path: vec!["out".into()],
            data: Some(vec![0, 255]),
        }]
    );
}

#[test]
fn event_ranges_and_recording_budget_span_remounts() {
    let prefix = b"AFSPSC01\nformat 4096 256 64 8\ncreate a root 61 01\nremount\n";
    let full = b"AFSPSC01\nformat 4096 256 64 8\ncreate a root 61 01\nremount\ncreate b root 62 02\nsync\n";
    let prefix_run = Plan::parse(prefix).unwrap().run().unwrap();
    let run = Plan::parse(full)
        .unwrap()
        .run_with_limits(RecordingLimits {
            operations: prefix_run.log.len(),
            payload_bytes: 64 * 1024 * 1024,
        })
        .unwrap();
    assert_eq!(run.failure.as_ref().map(|f| f.0), Some(2));
    assert_eq!(run.log.len(), prefix_run.log.len());
    assert_eq!(run.events.len(), 3);
    let mut end = 0;
    for (index, event) in run.events.iter().enumerate() {
        assert_eq!(event.operation, index);
        assert_eq!(event.first_block_operation, end);
        assert!(event.end_block_operation >= end);
        end = event.end_block_operation;
    }
    assert!(run.events[0].object_id > 0);
    assert!(run.events[0].success);
    assert!(run.events[1].success);
    assert!(!run.events[2].success);
    assert_eq!(end, run.log.len());
    for lba in 0..run.result.total_blocks() {
        assert_eq!(run.result.peek(lba), prefix_run.result.peek(lba));
    }
}

#[test]
fn structural_findings_cannot_hide_behind_a_readable_namespace() {
    use afsplus_check::scenario::inspect_checked;
    use afsplus_core::{allocation_root, object_map};
    use afsplus_format::{
        bitmap::{BitmapPage, BITMAP_PAGE_BLOCKS},
        checkpoint::Checkpoint,
        ident::Identification,
        region::RegionDescriptor,
    };
    // Use a fully synchronized fixture: pending log recovery could otherwise
    // reuse the deliberately misclassified block before namespace observation.
    let run = Plan::parse(LADDER).unwrap().run().unwrap();
    let mut volume = mount(run.result).unwrap();
    volume.sync().unwrap();
    let object = volume.lookup_root("out").unwrap().unwrap();
    let mut image = volume.into_device();
    let ident = Identification::decode(&image.peek(0)).unwrap();
    let geo = ident.geometry();
    let checkpoint = [1, 2]
        .into_iter()
        .filter_map(|lba| Checkpoint::decode(&image.peek(lba), &ident.uuid).ok())
        .max_by_key(|cp| cp.generation)
        .unwrap();
    let object_lba = object_map::lookup_lba(
        &mut image,
        &geo,
        checkpoint.object_map_block,
        checkpoint.generation,
        object,
    )
    .unwrap()
    .unwrap();
    let region = (object_lba / u64::from(geo.region_size)) as u32;
    let local = (object_lba % u64::from(geo.region_size)) as u32;
    let page = local / BITMAP_PAGE_BLOCKS;
    let allocation = allocation_root::lookup_record(
        &mut image,
        &geo,
        checkpoint.allocation_root_block,
        checkpoint.generation,
        region,
    )
    .unwrap()
    .unwrap();
    let (descriptor, _) = RegionDescriptor::decode(
        &image.peek(geo.descriptor_slot_lba(region, allocation.descriptor_slot)),
    )
    .unwrap();
    let lba = geo.bitmap_slot_lba(region, page, descriptor.pages[page as usize].slot);
    let (mut bitmap, generation) = BitmapPage::decode(&image.peek(lba)).unwrap();
    let target = local - bitmap.first_block;
    assert!(bitmap.is_allocated(target));
    let other = (0..bitmap.valid_blocks)
        .find(|&index| !bitmap.is_allocated(index))
        .unwrap();
    bitmap.set_allocated(target, false);
    bitmap.set_allocated(other, true);
    image
        .write_block(lba, &bitmap.encode(4096, generation).unwrap())
        .unwrap();
    let original = image.clone();
    let observed = inspect_checked(image.clone(), 1024 * 1024);
    assert!(observed.entries.is_ok(), "{:?}", observed.entries);
    assert!(!observed.is_clean());
    assert!(!observed.raw.is_clean());
    assert!(!observed.recovered.unwrap().is_clean());
    for lba in 0..original.total_blocks() {
        assert_eq!(image.peek(lba), original.peek(lba));
    }
}

#[test]
fn checked_inspection_requires_clean_raw_and_recovered_views() {
    use afsplus_check::scenario::inspect_checked;
    let run = Plan::parse(LADDER).unwrap().run().unwrap();
    let observed = inspect_checked(run.result, 2);
    assert!(observed.is_clean(), "{}", observed.raw.render_text());
    assert_eq!(observed.entries.unwrap()[0].data, Some(vec![0, 255]));
    let run = Plan::parse(LADDER).unwrap().run().unwrap();
    let refused = inspect_checked(run.result, 1);
    assert!(!refused.is_clean());
    assert!(refused.raw.is_clean());
    assert!(refused.recovered.unwrap().is_clean());
}

#[test]
fn selected_diagnostics_retain_endpoints_and_deterministic_delivery_across_remounts() {
    for profile in ["2", "4", "8", "unlimited"] {
        for capacity in [1, 32] {
            let operations = "create a root 61 01\nremount\ncreate b root 62 02\n";
            let baseline = Plan::parse(
                format!("AFSPSC03\nformat 4096 256 64 8 {profile} {capacity}\n{operations}")
                    .as_bytes(),
            )
            .unwrap()
            .run()
            .unwrap();
            assert!(baseline.failure.is_none());
            for mask in 0u8..16 {
                let selected = 2 * usize::from(mask & 1 != 0)
                    + 3 * usize::from(mask & 2 != 0)
                    + usize::from(mask & 4 != 0);
                for (sink_capacity, disconnect) in
                    [(0, "none"), (1, "none"), (8, "none"), (1, "0"), (1, "2")]
                {
                    let plan = Plan::parse(format!("AFSPSC04\nformat 4096 256 64 8 {profile} {capacity} {mask} {sink_capacity} {disconnect}\n{operations}").as_bytes()).unwrap();
                    let run = plan.run().unwrap();
                    assert!(run.failure.is_none());
                    let first = run.events[0].flight.as_ref().unwrap();
                    let middle = run.events[1].flight.as_ref().unwrap();
                    let last = run.events[2].flight.as_ref().unwrap();
                    assert_eq!((first.sequence_total, first.attempt_total), (6, 1));
                    assert_eq!((middle.sequence_total, middle.attempt_total), (6, 1));
                    assert!(middle.events.is_empty());
                    assert_eq!((last.sequence_total, last.attempt_total), (12, 2));
                    assert_eq!(last.filtered_total, (12 - 2 * selected) as u64);
                    assert_eq!(
                        last.dropped_total,
                        (2 * selected.saturating_sub(capacity)) as u64
                    );
                    assert_eq!(last.events.len(), selected.min(capacity));
                    let delivered = if sink_capacity == 0 || disconnect == "0" {
                        0
                    } else {
                        selected.min(sink_capacity) * if disconnect == "2" { 1 } else { 2 }
                    };
                    assert_eq!(last.delivered_total, delivered as u64);
                    assert_eq!(
                        last.missed_total,
                        if sink_capacity == 0 {
                            0
                        } else {
                            (2 * selected - delivered) as u64
                        }
                    );
                    assert_eq!(last.sink_closed, disconnect != "none" && selected != 0);
                    assert_eq!(baseline.log.len(), run.log.len());
                    for (a, b) in baseline.log.iter().zip(&run.log) {
                        match (a, b) {
                            (RecordedOp::Flush, RecordedOp::Flush) => (),
                            (
                                RecordedOp::Write { lba: a, data: x },
                                RecordedOp::Write { lba: b, data: y },
                            ) => {
                                assert_eq!(a, b);
                                assert_eq!(x, y);
                            }
                            _ => panic!("selected diagnostics changed block operations"),
                        }
                    }
                    for lba in 0..256 {
                        assert_eq!(baseline.result.peek(lba), run.result.peek(lba));
                    }
                }
            }
        }
    }
}

#[test]
fn selected_diagnostic_header_refuses_invalid_configuration() {
    for suffix in [
        "1 16 0 none",
        "1 1 257 none",
        "1 1 0 0",
        "1 1 1 1025",
        "0 1 1 none",
        "1 1 1 -1",
        "1 1 1",
    ] {
        assert!(
            Plan::parse(format!("AFSPSC04\nformat 4096 256 64 8 2 {suffix}\nsync\n").as_bytes())
                .is_err(),
            "{suffix}"
        );
    }
}

#[test]
fn extended_diagnostic_commands_are_version_bound_and_range_checked() {
    for version in ["AFSPSC04", "AFSPSC05"] {
        let mask = if version == "AFSPSC05" { 63 } else { 15 };
        for command in [
            "window_write f 0 42",
            "window_truncate f 1",
            "window_fsync",
            "window_commit",
        ] {
            let wire = format!("{version}\nformat 4096 256 64 8 2 256 {mask} 0 none\n{command}\n");
            assert_eq!(Plan::parse(wire.as_bytes()).is_ok(), version == "AFSPSC05");
        }
    }
    for suffix in ["256 64 0 none", "0 63 0 none", "256 63 0 1"] {
        assert!(Plan::parse(
            format!("AFSPSC05\nformat 4096 256 64 8 2 {suffix}\nsync\n").as_bytes()
        )
        .is_err());
    }
    assert!(Plan::parse(
        b"AFSPSC05\nformat 4096 256 64 8 2 256 63 0 none\nwindow_write f 16777216 42\n"
    )
    .is_err());
}

#[test]
fn object_observation_is_an_explicit_version_six_profile() {
    for (version, mask, valid) in [
        ("AFSPSC05", 64, false),
        ("AFSPSC06", 127, true),
        ("AFSPSC06", 128, false),
    ] {
        let wire = format!("{version}\nformat 4096 256 64 8 2 256 {mask} 0 none\nsync\n");
        let plan = Plan::parse(wire.as_bytes());
        assert_eq!(plan.is_ok(), valid);
        if let Ok(plan) = plan {
            assert!(plan.object_observation());
            assert!(plan.api_observation());
        }
    }
}

const LINKED_HEADER: &str = "format 4096 256 64 8 2 32 127 0 none 4096 16 8 0 1";

#[test]
fn linked_commands_are_version_nine_and_admission_bounded() {
    for command in [
        "link l f root 61",
        "symlink s root 73 2e2e",
        "clone_file c f root 63",
        "clone_range f 0 g 0 1",
        "set_protection f 7",
        "unlink_symlink s",
    ] {
        for (version, valid) in [("AFSPSC07", false), ("AFSPSC08", false), ("AFSPSC09", true)] {
            let wire = format!("{version}\n{LINKED_HEADER}\n{command}\n");
            assert_eq!(
                Plan::parse(wire.as_bytes()).is_ok(),
                valid,
                "{version} {command}"
            );
        }
    }
    let long_target = format!("symlink s root 73 {}", "61".repeat(1025));
    for command in [
        "symlink s root 73 -",
        "symlink s root 73 610062",
        "symlink s root 73 ff",
        long_target.as_str(),
        "set_protection f 4294967296",
        "clone_range f 16777216 g 0 1",
        "clone_range f 0 g 1 16777216",
        "link l f root",
    ] {
        let wire = format!("AFSPSC09\n{LINKED_HEADER}\n{command}\n");
        assert!(Plan::parse(wire.as_bytes()).is_err(), "{command}");
    }
    let plan = Plan::parse(format!("AFSPSC09\n{LINKED_HEADER}\nsync\n").as_bytes()).unwrap();
    assert!(plan.linked_observation() && plan.object_observation());
    assert!(plan.snapshot_limits().is_some());
}

#[test]
fn linked_observation_reports_aliases_targets_protection_and_clone_bytes() {
    use afsplus_check::scenario::LinkedKind;
    for profile in ["2", "4", "8", "unlimited"] {
        let wire = format!(
            "AFSPSC09\nformat 4096 512 64 8 {profile} 32 127 0 none 4096 16 8 0 1\n\
             mkdir d root 64\ncreate f d 66 000102\nlink l f root 6c\n\
             symlink s d 73 2e2e2fcf84\nset_protection f 7\nclone_file c l root 63\n\
             write f 0 ff\nclone_range l 1 c 4097 2\nrename d root 6532\nunlink f\nremount\n"
        );
        let plan = Plan::parse(wire.as_bytes()).unwrap();
        let run = plan.run().unwrap();
        assert!(run.failure.is_none(), "{:?}", run.failure);
        let observed = plan.inspect_checked(run.result, 1 << 20);
        assert!(observed.is_clean());
        assert!(observed.entries.is_err());
        let linked = observed.linked.unwrap().unwrap();
        let summary: Vec<_> = linked
            .iter()
            .map(|e| (e.path.join("/"), e.kind, e.link_count, e.protection))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("c".to_owned(), LinkedKind::File, 1, 7),
                ("e2".to_owned(), LinkedKind::Directory, 1, 0),
                ("e2/s".to_owned(), LinkedKind::Symlink, 1, 0),
                ("l".to_owned(), LinkedKind::File, 1, 7),
            ]
        );
        let mut clone = vec![0, 1, 2];
        clone.resize(4097, 0);
        clone.extend([1, 2]);
        assert_eq!(linked[0].data, clone);
        assert_eq!(linked[2].data, "../τ".as_bytes());
        assert_eq!(linked[3].data, [0xff, 1, 2]);
        assert_ne!(linked[0].object_id, linked[3].object_id);
    }
    let plan = Plan::parse(
        format!("AFSPSC09\n{LINKED_HEADER}\ncreate f root 66 01\nlink l f root 6c\n").as_bytes(),
    )
    .unwrap();
    let run = plan.run().unwrap();
    let linked = plan
        .inspect_checked(run.result, 1024)
        .linked
        .unwrap()
        .unwrap();
    assert_eq!(linked.len(), 2);
    assert_eq!(linked[0].object_id, linked[1].object_id);
    assert!(linked.iter().all(|e| e.link_count == 2 && e.data == [1]));
    // Invalid linked operations are captured failures, never repaired sequences.
    for command in [
        "unlink_symlink f",
        "clone_range f 0 f 0 1",
        "clone_range f 0 g 1 1",
        "set_protection root 1",
        "link x root root 78",
    ] {
        let wire = format!(
            "AFSPSC09\n{LINKED_HEADER}\ncreate f root 66 01\ncreate g root 67 02\n{command}\n"
        );
        let run = Plan::parse(wire.as_bytes()).unwrap().run().unwrap();
        assert_eq!(run.failure.as_ref().map(|f| f.0), Some(2), "{command}");
    }
}

const V7_HEADER: &str = "format 4096 256 64 8 2 32 127 0 none 4096 16 8";

#[test]
fn version_nine_replacement_orphan_and_reservation_commands_are_admission_bounded() {
    for command in [
        "rename_replace f g root 67",
        "rename_replace_orphan f g root 67",
        "orphan_file f",
        "cleanup_orphan f",
        "preallocate f 0 4096",
        "preallocate_bounded f 0 4096 1 1",
        "set_data_policy f 1",
        "restore_metadata f 7 1 2 3",
        "reclaim_step",
        "snapshot_maintenance_step",
        "batch create:b:root:62:01",
        "window_batch create:b:root:62:01,rename:f:root:67",
    ] {
        for (version, header, valid) in [
            ("AFSPSC07", V7_HEADER, false),
            ("AFSPSC09", LINKED_HEADER, true),
        ] {
            let wire = format!("{version}\n{header}\n{command}\n");
            assert_eq!(
                Plan::parse(wire.as_bytes()).is_ok(),
                valid,
                "{version} {command}"
            );
        }
    }
    for command in [
        "preallocate f 0 16777217",
        "preallocate f 16777216 1",
        "preallocate_bounded f 0 4096 4097 1",
        "preallocate_bounded f 0 4096 1",
        "set_data_policy f 2",
        "restore_metadata f 7 1 2 2147483649",
        "restore_metadata f 4294967296 1 2 3",
        "rename_replace f g root",
        "batch ",
        "batch unknown:b",
        "batch create:b:root:62",
        "batch create:b:root:62:01,,delete:f",
        "cleanup_orphan 1f",
    ] {
        let wire = format!("AFSPSC09\n{LINKED_HEADER}\n{command}\n");
        assert!(Plan::parse(wire.as_bytes()).is_err(), "{command}");
    }
    for header in [
        "format 4096 256 64 8 2 32 127 0 none 4096 16 8 2 1",
        "format 4096 256 64 8 2 32 127 0 none 4096 16 8 0 0",
        "format 4096 256 64 8 2 32 127 0 none 4096 16 8 0 65",
        "format 4096 256 64 8 2 32 127 0 none 4096 16 8 0",
    ] {
        let wire = format!("AFSPSC09\n{header}\nsync\n");
        assert!(Plan::parse(wire.as_bytes()).is_err(), "{header}");
    }
    let wire = "AFSPSC09\nformat 4096 256 64 8 2 32 127 0 none 4096 16 8 1 7\nsync\n";
    let plan = Plan::parse(wire.as_bytes()).unwrap();
    assert!(plan.data_policy());
    assert_eq!(plan.orphan_extents(), 7);
}

#[test]
fn version_nine_observes_replacement_reservation_policy_and_reserved_directory() {
    let wire = "AFSPSC09\nformat 4096 512 64 8 2 32 127 0 none 4096 16 8 1 2\n\
                create f root 66 0102\ncreate g root 67 03\nrename_replace f g root 67\n\
                preallocate f 4096 4096\nset_data_policy f 1\n\
                create h root 68 040506\norphan_file h\n\
                batch create:b:root:62:07\nbatch rename:b:root:6232\nremount\n";
    let plan = Plan::parse(wire.as_bytes()).unwrap();
    let run = plan.run().unwrap();
    assert!(run.failure.is_none(), "{:?}", run.failure);
    let candidates = run.orphan_candidates.clone();
    let observed = plan.inspect_checked_with_orphans(run.result, 1 << 20, &candidates);
    assert!(observed.is_clean());
    // The reserved directory names the orphaned object and sums its size.
    assert_eq!(observed.orphans.unwrap().unwrap(), (1, 3));
    let linked = observed.linked.unwrap().unwrap();
    let paths: Vec<_> = linked.iter().map(|e| e.path.join("/")).collect();
    assert_eq!(paths, vec!["b2".to_owned(), "g".to_owned()]);
    let replaced = &linked[1];
    assert!(replaced.in_place);
    assert_eq!(replaced.data, [1, 2]);
    // A reservation keeps the size and adds unwritten logical coverage.
    assert_eq!(
        replaced.allocation,
        vec![(0, 4096, false), (4096, 4096, true)]
    );
    assert!(!linked[0].in_place);
}

#[test]
fn version_nine_refusals_are_captured_operation_failures() {
    // A base of one file, one directory and one symlink, then one command.
    let base = "create f root 66 0102\nmkdir d root 64\nsymlink s root 73 2e2e\n";
    for (policy, command) in [
        (1, "rename_replace s f root 66"),
        (1, "rename_replace f d root 64"),
        (1, "rename_replace_orphan s f root 66"),
        (1, "orphan_file d"),
        (1, "cleanup_orphan f"),
        (1, "preallocate d 0 4096"),
        (1, "preallocate_bounded f 0 8192 1 4096"),
        (1, "preallocate_bounded f 0 4096 1 0"),
        (1, "set_data_policy d 1"),
        (1, "restore_metadata root 1 1 1 1"),
        (0, "set_data_policy f 1"),
        (1, "batch create:x:f:62:01"),
        (1, "batch replace:f:s:root:73"),
        (1, "window_batch create:x:root:62:01,delete:x"),
    ] {
        let wire = format!(
            "AFSPSC09\nformat 4096 512 64 8 2 32 127 0 none 4096 16 8 {policy} 2\n{base}{command}\n"
        );
        let run = Plan::parse(wire.as_bytes()).unwrap().run().unwrap();
        assert_eq!(
            run.failure.as_ref().map(|failure| failure.0),
            Some(3),
            "{command}"
        );
    }
}

const V8_HEADER: &str = "format 4096 1024 64 8 2 256 32767 0 none 64 4 64";

/// One version-8 scenario reaching the allocator, the mutable trees, reclaim,
/// mount recovery, a refused mount, formatting, verification, staged window
/// writes and captured-view reads.
fn version_eight_wire(pages: &str, mask: u32, capacity: usize, sink: &str) -> Vec<u8> {
    let mut wire = format!(
        "AFSPSC08\nformat 4096 1024 64 8 {pages} {capacity} {mask} {sink} 64 4 64\n\
         format_fault write 0\nmkdir d root 646972\n"
    );
    for index in 0..24 {
        wire.push_str(&format!(
            "create f{index} d {:02x}{:02x} {}\n",
            0x61 + index / 10,
            0x30 + index % 10,
            "5a".repeat(200)
        ));
    }
    wire.push_str("write f0 0 ");
    wire.push_str(&"ab".repeat(5000));
    wire.push_str("\nsync\ntruncate f0 100\nwindow_write f1 0 ");
    wire.push_str(&"cd".repeat(3000));
    wire.push_str("\nwindow_fsync\nwindow_write f2 0 ");
    wire.push_str(&"ef".repeat(3000));
    wire.push_str(
        "\nremount\nsnapshot_create s\nsnapshot_open s\nsnapshot_inspect s\n\
         snapshot_close s\nsnapshot_delete s\nsync\nverify\nremount_refused\n",
    );
    wire.into_bytes()
}

#[test]
fn version_eight_selects_every_category_bit_and_binds_lifecycle_commands() {
    for (version, header, valid) in [
        ("AFSPSC07", V7_HEADER, false),
        ("AFSPSC08", V8_HEADER, true),
        ("AFSPSC09", LINKED_HEADER, false),
    ] {
        for command in ["verify", "remount_refused"] {
            let wire = format!("{version}\n{header}\n{command}\n");
            assert_eq!(
                Plan::parse(wire.as_bytes()).is_ok(),
                valid,
                "{version} {command}"
            );
        }
    }
    for (mask, valid) in [(0, true), (32767, true), (32768, false)] {
        let wire = format!("AFSPSC08\nformat 4096 256 64 8 2 256 {mask} 0 none 64 4 64\nsync\n");
        assert_eq!(Plan::parse(wire.as_bytes()).is_ok(), valid, "mask {mask}");
    }
    // Version 7 keeps its own ceiling, so version 8 widens nothing behind it.
    let wire = "AFSPSC07\nformat 4096 256 64 8 2 256 128 0 none 64 4 64\nsync\n";
    assert!(Plan::parse(wire.as_bytes()).is_err());
    let plan =
        Plan::parse(format!("AFSPSC08\n{V8_HEADER}\nsync\n").as_bytes()).expect("version 8 plan");
    assert!(plan.lifecycle_observation() && plan.object_observation() && plan.api_observation());
    assert!(!plan.linked_observation() && plan.snapshot_limits().is_some());
}

#[test]
fn version_eight_observation_changes_no_image_byte_and_no_block_operation() {
    for pages in ["2", "4", "8", "unlimited"] {
        for (mask, capacity) in [(0u32, 1usize), (32767, 1), (32767, 256)] {
            let plan = Plan::parse(&version_eight_wire(pages, mask, capacity, "0 none"))
                .expect("version 8 plan");
            let observed = plan.run_with_limits(RecordingLimits::default()).unwrap();
            let plain = plan.run_unobserved(RecordingLimits::default()).unwrap();
            assert!(observed.failure.is_none(), "{pages} {mask} {capacity}");
            assert_eq!(plain.failure.is_none(), observed.failure.is_none());
            assert_eq!(plain.log.len(), observed.log.len());
            for (left, right) in plain.log.iter().zip(&observed.log) {
                match (left, right) {
                    (RecordedOp::Flush, RecordedOp::Flush) => (),
                    (
                        RecordedOp::Write { lba: a, data: x },
                        RecordedOp::Write { lba: b, data: y },
                    ) => assert!(a == b && x == y, "block write differs"),
                    _ => panic!("block operation differs"),
                }
            }
            let mut left = plain.result.clone();
            let mut right = observed.result.clone();
            let mut a = vec![0u8; left.block_size()];
            let mut b = vec![0u8; right.block_size()];
            for lba in 0..left.total_blocks() {
                left.read_block(lba, &mut a).unwrap();
                right.read_block(lba, &mut b).unwrap();
                assert_eq!(a, b, "image block {lba} differs");
            }
            assert!(plain.pre_mount.is_none());
            assert!(observed.pre_mount.is_some());
        }
    }
}

#[test]
fn version_eight_injected_faults_reproduce_without_observation() {
    // A fault is a scenario input, so an observed run and an observation-free
    // run must fail at the same operation with the same error and the same I/O.
    for command in [
        "fault write 0\nwindow_fsync\nwindow_commit\n",
        "fault read 0\nsnapshot_create s\nsync\n",
        "fault flush 0\nwindow_fsync\nwindow_commit\n",
    ] {
        let wire = format!(
            "AFSPSC08\n{V8_HEADER}\ncreate f root 61 {}\nsync\nwindow_write f 0 {}\n{command}",
            "ab".repeat(3000),
            "cd".repeat(2000)
        );
        let plan = Plan::parse(wire.as_bytes()).expect("version 8 plan");
        let observed = plan.run_with_limits(RecordingLimits::default()).unwrap();
        let plain = plan.run_unobserved(RecordingLimits::default()).unwrap();
        let failure = observed.failure.as_ref().expect("the armed fault trips");
        assert!(failure.1.contains("injected"), "{failure:?}");
        assert_eq!(
            plain.failure.as_ref().map(|f| (f.0, f.1.clone())),
            Some(failure.clone())
        );
        assert_eq!(plain.log.len(), observed.log.len());
        let mut left = plain.result.clone();
        let mut right = observed.result.clone();
        let mut a = vec![0u8; left.block_size()];
        let mut b = vec![0u8; right.block_size()];
        for lba in 0..left.total_blocks() {
            left.read_block(lba, &mut a).unwrap();
            right.read_block(lba, &mut b).unwrap();
            assert_eq!(a, b, "image block {lba} differs");
        }
    }
    // An armed fault no device operation reaches is refused.
    let wire = format!("AFSPSC08\n{V8_HEADER}\nfault flush 60000\nsync\n");
    let plan = Plan::parse(wire.as_bytes()).expect("version 8 plan");
    assert!(plan.run().is_err());
    // A format fault belongs to the first operation alone.
    let wire = format!("AFSPSC08\n{V8_HEADER}\nsync\nformat_fault write 0\n");
    assert!(Plan::parse(wire.as_bytes()).is_err());
}

#[test]
fn version_eight_batches_carry_the_format_mount_and_subsystem_scopes() {
    use afsplus_core::flight::{Category, EventKind, LifecycleContext};
    let plan = Plan::parse(&version_eight_wire("2", 32767, 256, "0 none")).expect("version 8 plan");
    let run = plan.run_with_limits(RecordingLimits::default()).unwrap();
    assert!(run.failure.is_none(), "{:?}", run.failure);
    let pre_mount = run.pre_mount.as_ref().expect("explicit pre-mount batch");
    let staged: Vec<_> = pre_mount.events.iter().map(|event| event.kind).collect();
    assert_eq!(staged.first(), Some(&EventKind::FormatBegin));
    // The refused first format precedes the fresh one in the same batch.
    assert!(staged.contains(&EventKind::FormatFailed));
    assert!(staged.contains(&EventKind::FormatCheckpointDurable));
    assert!(staged.contains(&EventKind::MountBegin));
    assert!(staged.contains(&EventKind::MountComplete));
    let mut kinds = std::collections::BTreeSet::new();
    let mut categories = std::collections::BTreeSet::new();
    for batch in
        std::iter::once(pre_mount).chain(run.events.iter().filter_map(|e| e.flight.as_ref()))
    {
        for event in &batch.events {
            kinds.insert(format!("{:?}", event.kind));
            categories.insert(format!("{:?}", event.kind.category()));
            // A payload is present exactly when its category names one.
            let payload = event.allocation.is_some() as u8
                + event.tree.is_some() as u8
                + event.reclaim.is_some() as u8
                + event.lifecycle.is_some() as u8;
            let expected = matches!(
                event.kind.category(),
                Category::Allocator
                    | Category::Tree
                    | Category::Reclaim
                    | Category::Mount
                    | Category::Format
                    | Category::Verify
                    | Category::Data
                    | Category::View
            );
            assert_eq!(payload, u8::from(expected), "{:?}", event.kind);
            if let Some(LifecycleContext::Mount(context)) = event.lifecycle {
                assert!(
                    event.generation != 0
                        || event.kind == EventKind::MountBegin
                        || event.kind == EventKind::MountFailed
                );
                assert!(context.slot <= 1);
            }
        }
    }
    for expected in [
        "AllocationGranted",
        "AllocationRetired",
        "TreeReadBegin",
        "ReclaimPromoted",
        "MountIntentReplayed",
        "MountFailed",
        "FormatBegin",
        "FormatFailed",
        "VerifyBegin",
        "VerifyPhase",
        "VerifyComplete",
        "DataWriteBegin",
        "DataWriteComplete",
        "IntentDataDurable",
        "ViewReadBegin",
        "ViewReadComplete",
    ] {
        assert!(kinds.contains(expected), "missing {expected}: {kinds:?}");
    }
    for expected in [
        "Allocator",
        "Tree",
        "Reclaim",
        "Mount",
        "Format",
        "Verify",
        "Data",
        "View",
    ] {
        assert!(categories.contains(expected), "missing category {expected}");
    }
}

#[test]
fn version_eight_category_mask_selects_each_scope_alone() {
    for (bit, name) in [
        (7u32, "Allocation"),
        (8, "Tree"),
        (9, "Reclaim"),
        (10, "Mount"),
        (11, "Format"),
        (12, "Verify"),
        (13, "Data"),
        (14, "View"),
    ] {
        let plan =
            Plan::parse(&version_eight_wire("2", 1 << bit, 256, "0 none")).expect("version 8 plan");
        let run = plan.run_with_limits(RecordingLimits::default()).unwrap();
        assert!(run.failure.is_none(), "{name}");
        let mut seen = false;
        for batch in std::iter::once(run.pre_mount.as_ref().unwrap())
            .chain(run.events.iter().filter_map(|e| e.flight.as_ref()))
        {
            for event in &batch.events {
                let category = format!("{:?}", event.kind.category());
                assert_eq!(
                    1u16 << bit,
                    match category.as_str() {
                        "Allocator" => 1 << 7,
                        "Tree" => 1 << 8,
                        "Reclaim" => 1 << 9,
                        "Mount" => 1 << 10,
                        "Format" => 1 << 11,
                        "Verify" => 1 << 12,
                        "Data" => 1 << 13,
                        "View" => 1 << 14,
                        other => panic!("unselected category {other}"),
                    },
                    "{name}"
                );
                seen = true;
            }
        }
        assert!(seen, "no {name} record was retained");
    }
}
