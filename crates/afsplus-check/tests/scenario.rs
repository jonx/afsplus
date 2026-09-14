//! Semantic runner admission, exact state and bounded failure evidence.
use afsplus_block::{BlockDevice, RecordedOp};
use afsplus_check::scenario::{Plan, RecordingLimits};
use afsplus_core::mount;

const LADDER: &[u8] = b"AFSPSC01\nformat 4096 256 64 8\nmkdir d root 737263\ncreate f d 636166c3a9 00ff\nwrite f 4 42\ntruncate f 2\nrename f root 6f7574\nrmdir d\ncreate spare root 74656d70 -\nunlink spare\nsync\nremount\n";

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
