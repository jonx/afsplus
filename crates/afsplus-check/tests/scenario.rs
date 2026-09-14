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
