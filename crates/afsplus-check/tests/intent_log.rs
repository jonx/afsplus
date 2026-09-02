//! Intent-log qualification (ADR-037): exactly the fsynced prefix survives
//! a crash, fsync groups replay all-or-nothing, stale records are inert —
//! and the blocker-2 bake-off gate: beat 3 barriers / 10 writes per durable
//! ref update.

use afsplus_block::{crash_states, MemoryBackend, RecordingBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{mkfs, mount, CoreError, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(total: u64, log_slots: u16) -> MemoryBackend {
    formatted_with_policy(total, log_slots, NamePolicy::Sensitive)
}

fn formatted_with_policy(total: u64, log_slots: u16, name_policy: NamePolicy) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, total);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [88u8; 16],
            label: "LogVol".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents: true,
            name_policy,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

#[test]
fn case_only_rename_replays_as_one_spelling_preserving_operation() {
    let base = {
        let mut vol = mount(formatted_with_policy(4096, 8, NamePolicy::Insensitive)).unwrap();
        vol.create_file_in_root("ReadMe", b"content", ts(1))
            .unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.window_op(&publish("readme", "README"), ts(2)).unwrap();
    vol.window_fsync().unwrap();
    let (_, log) = vol.into_device().into_parts();

    let mut old_spelling = 0;
    let mut new_spelling = 0;
    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            let context = state.description.clone();
            let mut vol = mount(state.image).unwrap_or_else(|error| panic!("{context}: {error}"));
            let object = vol
                .lookup_root("rEaDmE")
                .unwrap()
                .unwrap_or_else(|| panic!("{context}: folded lookup must succeed"));
            assert_eq!(vol.read_file(object).unwrap(), b"content", "{context}");
            let entries = vol.list_root().unwrap();
            assert_eq!(entries.len(), 1, "{context}");
            match entries[0].0.as_bytes() {
                b"ReadMe" => {
                    old_spelling += 1;
                    assert_eq!(vol.generation(), pre_generation, "{context}");
                }
                b"README" => {
                    new_spelling += 1;
                    assert_eq!(vol.generation(), pre_generation + 1, "{context}");
                }
                spelling => panic!("{context}: unexpected spelling {spelling:?}"),
            }
            let mut dev = vol.into_device();
            let report = check_device(&mut dev);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
        }
    }
    assert!(old_spelling > 0 && new_spelling > 0);
}

fn create<'a>(name: &'a str, content: &'a [u8]) -> BatchOp<'a> {
    BatchOp::CreateFile {
        parent_id: OBJECT_ROOT,
        name,
        content,
    }
}

fn publish<'a>(source: &'a str, target: &'a str) -> BatchOp<'a> {
    BatchOp::Rename {
        source_parent_id: OBJECT_ROOT,
        source_name: source,
        target_parent_id: OBJECT_ROOT,
        target_name: target,
        replace: true,
    }
}

#[test]
fn crash_recovers_exactly_the_fsynced_prefix() {
    let dev = formatted(4096, 8);
    let mut vol = mount(dev).unwrap();

    // Group 1 (fsynced): v1 published to HEAD.
    vol.window_op(&create("HEAD.lock", b"v1"), ts(1)).unwrap();
    vol.window_op(&publish("HEAD.lock", "HEAD"), ts(1)).unwrap();
    vol.window_fsync().unwrap();
    // Group 2 (fsynced): durable side file.
    vol.window_op(&create("durable.txt", b"kept"), ts(2))
        .unwrap();
    vol.window_fsync().unwrap();
    // Unlogged tail: must vanish in the crash.
    vol.window_op(&create("volatile.txt", b"lost"), ts(3))
        .unwrap();
    assert_eq!(vol.window_unlogged_ops(), 1);

    // Crash: drop the in-memory window; the device keeps data + records.
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    assert_eq!(report.volume.unwrap().log_records_pending, 2);

    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 2, "replay publishes one checkpoint");
    let head = vol.lookup_root("HEAD").unwrap().expect("fsynced group 1");
    assert_eq!(vol.read_file(head).unwrap(), b"v1");
    let durable = vol
        .lookup_root("durable.txt")
        .unwrap()
        .expect("fsynced group 2");
    assert_eq!(vol.read_file(durable).unwrap(), b"kept");
    assert_eq!(vol.lookup_root("HEAD.lock").unwrap(), None);
    assert_eq!(
        vol.lookup_root("volatile.txt").unwrap(),
        None,
        "unlogged op must vanish"
    );

    // The replay checkpoint made the log stale: remounting is idempotent.
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    assert_eq!(report.volume.unwrap().log_records_pending, 0);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 2, "stale records must not replay twice");
    assert!(vol.lookup_root("durable.txt").unwrap().is_some());
}

#[test]
fn every_crash_state_of_a_logged_ref_update_is_all_or_nothing() {
    // One fsync group = one record: create the lock, atomically publish it.
    // Every modeled crash state (including durable-record-over-torn-data,
    // which the content CRC rejects) recovers to the old or the new ref.
    let base = {
        let mut vol = mount(formatted(4096, 8)).unwrap();
        vol.create_file_in_root("HEAD", &[0x01u8; 2000], ts(1))
            .unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.window_op(&create("HEAD.lock", &[0x02u8; 2000]), ts(2))
        .unwrap();
    vol.window_op(&publish("HEAD.lock", "HEAD"), ts(2)).unwrap();
    vol.window_fsync().unwrap();
    let (_, log) = vol.into_device().into_parts();

    let mut pre = 0u64;
    let mut post = 0u64;
    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let mut vol = mount(image).unwrap_or_else(|e| panic!("{context}: {e}"));
            assert_eq!(vol.lookup_root("HEAD.lock").unwrap(), None, "{context}");
            let head = vol
                .lookup_root("HEAD")
                .unwrap()
                .unwrap_or_else(|| panic!("{context}: HEAD must always exist"));
            let content = vol.read_file(head).unwrap();
            match vol.generation() {
                g if g == pre_generation => {
                    pre += 1;
                    assert_eq!(content, vec![0x01u8; 2000], "{context}");
                }
                g if g == pre_generation + 1 => {
                    post += 1;
                    assert_eq!(content, vec![0x02u8; 2000], "{context}");
                }
                g => panic!("{context}: disallowed generation {g}"),
            }
            // Post-replay volumes must pass the checker again.
            let mut dev = vol.into_device();
            let after = check_device(&mut dev);
            assert!(
                after.is_clean(),
                "{context}: after replay {:?}",
                after.errors
            );
        }
    }
    assert!(pre > 0 && post > 0, "matrix must produce both outcomes");
}

#[test]
fn successive_fsync_groups_recover_as_monotone_prefixes() {
    let base = {
        let mut vol = mount(formatted(4096, 8)).unwrap();
        vol.create_file_in_root("HEAD", b"v0", ts(0)).unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    for (i, version) in [b"v1", b"v2", b"v3"].iter().enumerate() {
        vol.window_op(&create("HEAD.lock", *version), ts(i as i64 + 1))
            .unwrap();
        vol.window_op(&publish("HEAD.lock", "HEAD"), ts(i as i64 + 1))
            .unwrap();
        vol.window_fsync().unwrap();
    }
    let (_, log) = vol.into_device().into_parts();

    let mut seen = std::collections::BTreeSet::new();
    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let mut vol = mount(image).unwrap_or_else(|e| panic!("{context}: {e}"));
            assert!(
                vol.generation() == pre_generation || vol.generation() == pre_generation + 1,
                "{context}"
            );
            assert_eq!(vol.lookup_root("HEAD.lock").unwrap(), None, "{context}");
            let head = vol.lookup_root("HEAD").unwrap().expect("HEAD exists");
            let content = vol.read_file(head).unwrap();
            assert!(
                [&b"v0"[..], b"v1", b"v2", b"v3"].contains(&content.as_slice()),
                "{context}: HEAD holds a non-prefix state {content:?}"
            );
            seen.insert(content);
        }
    }
    assert!(
        seen.len() >= 3,
        "matrix should surface several prefixes, saw {seen:?}"
    );
}

#[test]
fn cancelled_window_ops_replay_cleanly() {
    let dev = formatted(4096, 8);
    let mut vol = mount(dev).unwrap();
    vol.window_op(&create("tmp", &[9u8; 2 * BS]), ts(1))
        .unwrap();
    vol.window_op(
        &BatchOp::DeleteFile {
            parent_id: OBJECT_ROOT,
            name: "tmp",
        },
        ts(1),
    )
    .unwrap();
    vol.window_op(&create("kept", b"stay"), ts(1)).unwrap();
    vol.window_fsync().unwrap();

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.lookup_root("tmp").unwrap(), None);
    let kept = vol.lookup_root("kept").unwrap().expect("kept survives");
    assert_eq!(vol.read_file(kept).unwrap(), b"stay");
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn window_rules_are_enforced() {
    let dev = formatted(4096, 2);
    let mut vol = mount(dev).unwrap();
    vol.window_op(&create("a", b"1"), ts(1)).unwrap();

    // Immediate-commit operations are refused while a window is open.
    assert!(matches!(
        vol.create_file_in_root("direct", b"x", ts(1)),
        Err(CoreError::WindowOpen)
    ));
    assert!(matches!(
        vol.reclaim_step(ts(1)),
        Err(CoreError::WindowOpen)
    ));

    // A validation error leaves the window usable.
    assert!(matches!(
        vol.window_op(&create("a", b"dup"), ts(1)),
        Err(CoreError::AlreadyExists)
    ));
    vol.window_op(&create("b", b"2"), ts(1)).unwrap();
    vol.window_fsync().unwrap();

    // Two log slots: the third fsync group must be refused until commit.
    vol.window_op(&create("c", b"3"), ts(2)).unwrap();
    vol.window_fsync().unwrap();
    vol.window_op(&create("d", b"4"), ts(3)).unwrap();
    assert!(matches!(
        vol.window_fsync(),
        Err(CoreError::PrototypeLimit(_))
    ));
    vol.window_commit(ts(4)).unwrap();
    assert_eq!(vol.generation(), 2);
    for name in ["a", "b", "c", "d"] {
        assert!(vol.lookup_root(name).unwrap().is_some(), "{name}");
    }

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    assert_eq!(
        report.volume.unwrap().log_records_pending,
        0,
        "commit staled the log"
    );
}

/// The blocker-2 bake-off gate (ADR-037): a durable ref update through the
/// log must beat the group-commit floor of 10 writes and 3 barriers.
#[test]
fn gate_logged_ref_updates_beat_the_checkpoint_floor() {
    let dev = formatted(65_536, 64);
    let mut vol = mount(TraceBackend::new(dev)).unwrap();
    vol.device_mut().reset();

    let updates = 256u64;
    let window = 64u64;
    for i in 0..updates {
        let content = format!("ref {i}\n");
        vol.window_op(&create("HEAD.lock", content.as_bytes()), ts(i as i64))
            .unwrap();
        vol.window_op(&publish("HEAD.lock", "HEAD"), ts(i as i64))
            .unwrap();
        vol.window_fsync().unwrap();
        if (i + 1) % window == 0 {
            vol.window_commit(ts(i as i64)).unwrap();
        }
    }
    let io = vol.device_mut().stats();
    let writes_per_update = io.writes as f64 / updates as f64;
    let flushes_per_update = io.flushes as f64 / updates as f64;
    println!(
        "logged ref update: {writes_per_update:.2} writes/op, \
         {flushes_per_update:.2} flushes/op ({} writes, {} flushes, {} reads)",
        io.writes, io.flushes, io.reads
    );
    assert!(
        writes_per_update < 5.0,
        "gate: {writes_per_update:.2} writes per durable update (floor was 10)"
    );
    assert!(
        flushes_per_update < 1.5,
        "gate: {flushes_per_update:.2} flushes per durable update (floor was 3)"
    );

    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    let head = vol.lookup_root("HEAD").unwrap().expect("last ref");
    assert_eq!(
        vol.read_file(head).unwrap(),
        format!("ref {}\n", updates - 1).as_bytes()
    );
}
