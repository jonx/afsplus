//! Intent-log qualification (ADR-037): exactly the fsynced prefix survives
//! a crash, fsync groups replay all-or-nothing, stale records are inert —
//! and the blocker-2 bake-off gate: beat 3 barriers / 10 writes per durable
//! ref update.

use afsplus_block::{
    crash_states, for_each_crash_state, MemoryBackend, RecordedOp, RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{
    mkfs, mount, mount_with_options, CoreError, MkfsParams, MountMode, MountOptions, NamePolicy,
};
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
            data_policy: false,
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

#[test]
fn existing_file_write_and_truncate_replay_in_order() {
    let mut original = vec![0x11u8; 3 * BS];
    let base = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        vol.create_file_in_root("database", &original, ts(1))
            .unwrap();
        vol.into_device()
    };

    let mut vol = mount(base).unwrap();
    let object = vol.lookup_root("database").unwrap().unwrap();
    let patch = vec![0xA5u8; BS + 37];
    let offset = BS as u64 - 19;
    vol.window_write_file_at(object, offset, &patch, ts(2))
        .unwrap();
    original[offset as usize..offset as usize + patch.len()].copy_from_slice(&patch);
    vol.window_fsync().unwrap();

    let truncated = BS as u64 + 113;
    vol.window_truncate_file(object, truncated, ts(3)).unwrap();
    original.truncate(truncated as usize);
    vol.window_fsync().unwrap();

    // Drop the live window: mount must rebuild the final layout solely from
    // the two durable records and their prewritten COW data.
    let mut dev = vol.into_device();
    let before = check_device(&mut dev);
    assert!(before.is_clean(), "{:?}", before.errors);
    assert_eq!(before.volume.unwrap().log_records_pending, 2);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.read_file(object).unwrap(), original);

    // The partial truncate tail was zeroed before logging. Growing the file
    // later must not reveal bytes from the old second block.
    vol.truncate_file(object, 2 * BS as u64, ts(4)).unwrap();
    let grown = vol.read_file(object).unwrap();
    assert!(grown[truncated as usize..].iter().all(|byte| *byte == 0));
    let mut dev = vol.into_device();
    let after = check_device(&mut dev);
    assert!(after.is_clean(), "{:?}", after.errors);
}

#[test]
fn every_crash_state_of_an_existing_file_write_is_old_or_new() {
    let old = vec![0x31u8; 2 * BS];
    let patch = vec![0x72u8; BS + 23];
    let offset = 101usize;
    let mut new = old.clone();
    new[offset..offset + patch.len()].copy_from_slice(&patch);
    let base = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        vol.create_file_in_root("vm", &old, ts(1)).unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    let object = vol.lookup_root("vm").unwrap().unwrap();
    vol.window_write_file_at(object, offset as u64, &patch, ts(2))
        .unwrap();
    vol.window_fsync().unwrap();
    let (_, log) = vol.into_device().into_parts();

    let mut saw_old = false;
    let mut saw_new = false;
    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let mut recovered = mount(image).unwrap_or_else(|error| panic!("{context}: {error}"));
            let content = recovered.read_file(object).unwrap();
            match recovered.generation() {
                generation if generation == pre_generation => {
                    saw_old = true;
                    assert_eq!(content, old, "{context}");
                }
                generation if generation == pre_generation + 1 => {
                    saw_new = true;
                    assert_eq!(content, new, "{context}");
                }
                generation => panic!("{context}: disallowed generation {generation}"),
            }
            let mut image = recovered.into_device();
            assert!(check_device(&mut image).is_clean(), "{context}");
        }
    }
    assert!(saw_old && saw_new);
}

#[test]
fn existing_file_write_then_rename_replays_as_one_group() {
    let base = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        vol.create_file_in_root("draft", b"old bytes", ts(1))
            .unwrap();
        vol.into_device()
    };
    let mut vol = mount(base).unwrap();
    let object = vol.lookup_root("draft").unwrap().unwrap();
    vol.window_write_file_at(object, 0, b"published", ts(2))
        .unwrap();
    vol.window_op(&publish("draft", "final"), ts(3)).unwrap();
    vol.window_fsync().unwrap();

    let mut vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.lookup_root("draft").unwrap(), None);
    assert_eq!(vol.lookup_root("final").unwrap(), Some(object));
    assert_eq!(vol.read_file(object).unwrap(), b"published");
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn write_and_truncate_replay_is_restartable_after_every_cut() {
    let old = vec![0x18u8; 3 * BS];
    let patch = vec![0xC7u8; BS + 41];
    let offset = 73usize;
    let final_size = BS + 211;
    let mut expected = old.clone();
    expected[offset..offset + patch.len()].copy_from_slice(&patch);
    expected.truncate(final_size);
    let base = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        vol.create_file_in_root("image", &old, ts(1)).unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut logger = mount(base).unwrap();
    let object = logger.lookup_root("image").unwrap().unwrap();
    logger
        .window_write_file_at(object, offset as u64, &patch, ts(2))
        .unwrap();
    logger
        .window_truncate_file(object, final_size as u64, ts(3))
        .unwrap();
    logger.window_fsync().unwrap();
    let logged = logger.into_device();

    // Record the automatic recovery transaction, then crash that recovery at
    // every device write and barrier. A second mount must either retry the
    // still-live record or observe its published checkpoint.
    let replaying = mount(RecordingBackend::new(logged.clone())).unwrap();
    let (_, replay) = replaying.into_device().into_parts();
    let mut saw_pre = false;
    let mut saw_post = false;
    for crash_point in 0..=replay.len() {
        for_each_crash_state(&logged, &replay, crash_point, |state| {
            let context = state.description;
            let mut raw_image = state.image.clone();
            let report = check_device(&mut raw_image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let raw = mount_with_options(
                raw_image,
                MountOptions {
                    mode: MountMode::NoChanges,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{context}: {error}"));
            match raw.generation() {
                generation if generation == pre_generation => {
                    saw_pre = true;
                    assert!(raw.pending_intent_records() > 0, "{context}");
                }
                generation if generation == pre_generation + 1 => {
                    saw_post = true;
                    assert_eq!(raw.pending_intent_records(), 0, "{context}");
                }
                generation => panic!("{context}: disallowed generation {generation}"),
            }

            let mut recovered =
                mount(state.image).unwrap_or_else(|error| panic!("{context}: {error}"));
            assert_eq!(recovered.generation(), pre_generation + 1, "{context}");
            assert_eq!(recovered.read_file(object).unwrap(), expected, "{context}");
            let mut image = recovered.into_device();
            assert!(check_device(&mut image).is_clean(), "{context}");
        });
    }
    assert!(saw_pre && saw_post);
}

#[test]
fn existing_file_data_is_barriered_before_its_log_record() {
    let base = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        vol.create_file_in_root("ordered", &[0x22; BS], ts(1))
            .unwrap();
        vol.into_device()
    };
    let mut vol = mount(RecordingBackend::new(base)).unwrap();
    let object = vol.lookup_root("ordered").unwrap().unwrap();
    let log_lba =
        afsplus_core::intent_log::log_slot_lbas(&vol.ident().geometry(), vol.ident().log_slots)
            .unwrap()[0];
    vol.window_write_file_at(object, 7, b"replacement", ts(2))
        .unwrap();
    vol.window_fsync().unwrap();
    let (_, operations) = vol.into_device().into_parts();
    let record_index = operations
        .iter()
        .position(|operation| matches!(operation, RecordedOp::Write { lba, .. } if *lba == log_lba))
        .expect("intent record write");
    assert!(record_index >= 2);
    assert!(matches!(operations[record_index - 1], RecordedOp::Flush));
    assert!(matches!(operations[record_index + 1], RecordedOp::Flush));
    assert!(operations[..record_index - 1]
        .iter()
        .any(|operation| matches!(operation, RecordedOp::Write { .. })));
}

#[test]
fn version_two_log_feature_rejects_data_updates_but_keeps_namespace_replay() {
    let mut dev = formatted(8192, 8);
    let mut ident = afsplus_format::ident::Identification::decode(&dev.peek(0)).unwrap();
    ident.features.incompat &= !afsplus_format::ident::INCOMPAT_INTENT_LOG_DATA_UPDATES;
    dev.apply_raw(0, &ident.encode(BS).unwrap());

    let mut vol = mount(dev).unwrap();
    let object = vol
        .create_file_in_root("old-format", b"old", ts(1))
        .unwrap();
    assert!(matches!(
        vol.window_write_file_at(object, 0, b"new", ts(2)),
        Err(CoreError::FeatureDisabled(_))
    ));
    vol.window_op(&create("namespace", b"still works"), ts(3))
        .unwrap();
    vol.window_fsync().unwrap();
    let mut vol = mount(vol.into_device()).unwrap();
    let created = vol.lookup_root("namespace").unwrap().unwrap();
    assert_eq!(vol.read_file(created).unwrap(), b"still works");
}

#[test]
fn version_three_record_without_its_feature_fails_closed() {
    let mut dev = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        let object = vol.create_file_in_root("guarded", b"old", ts(1)).unwrap();
        vol.window_write_file_at(object, 0, b"new", ts(2)).unwrap();
        vol.window_fsync().unwrap();
        vol.into_device()
    };
    let mut ident = afsplus_format::ident::Identification::decode(&dev.peek(0)).unwrap();
    ident.features.incompat &= !afsplus_format::ident::INCOMPAT_INTENT_LOG_DATA_UPDATES;
    dev.apply_raw(0, &ident.encode(BS).unwrap());

    let report = check_device(&mut dev);
    assert!(!report.is_clean());
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("without their INCOMPAT feature")));
    assert!(matches!(mount(dev), Err(CoreError::Corrupt(_))));
}

#[test]
fn data_free_truncates_replay_sparse_growth_and_aligned_shrink() {
    let base = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        vol.create_file_in_root("grow", b"seed", ts(1)).unwrap();
        vol.create_file_in_root("shrink", &[0x99; 2 * BS], ts(1))
            .unwrap();
        vol.into_device()
    };
    let mut vol = mount(base).unwrap();
    let grow = vol.lookup_root("grow").unwrap().unwrap();
    let shrink = vol.lookup_root("shrink").unwrap().unwrap();
    vol.window_truncate_file(grow, 3 * BS as u64 + 17, ts(2))
        .unwrap();
    vol.window_truncate_file(shrink, BS as u64, ts(2)).unwrap();
    vol.window_fsync().unwrap();

    let mut vol = mount(vol.into_device()).unwrap();
    let grown = vol.read_file(grow).unwrap();
    assert_eq!(&grown[..4], b"seed");
    assert!(grown[4..].iter().all(|byte| *byte == 0));
    assert_eq!(vol.read_file(shrink).unwrap(), vec![0x99; BS]);
    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}

#[test]
fn successive_existing_writes_recover_only_monotone_prefixes() {
    let base = {
        let mut vol = mount(formatted(8192, 8)).unwrap();
        vol.create_file_in_root("page", &[0u8; BS], ts(0)).unwrap();
        vol.into_device()
    };
    let mut logger = mount(RecordingBackend::new(base.clone())).unwrap();
    let object = logger.lookup_root("page").unwrap().unwrap();
    for value in 1..=3u8 {
        logger
            .window_write_file_at(object, 0, &vec![value; BS], ts(i64::from(value)))
            .unwrap();
        logger.window_fsync().unwrap();
    }
    let (_, operations) = logger.into_device().into_parts();

    let mut seen = std::collections::BTreeSet::new();
    for crash_point in 0..=operations.len() {
        for state in crash_states(&base, &operations, crash_point) {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let mut recovered = mount(image).unwrap_or_else(|error| panic!("{context}: {error}"));
            let bytes = recovered.read_file(object).unwrap();
            let value = bytes[0];
            assert!([0, 1, 2, 3].contains(&value), "{context}: {value}");
            assert!(bytes.iter().all(|byte| *byte == value), "{context}");
            seen.insert(value);
        }
    }
    assert_eq!(seen, std::collections::BTreeSet::from([0, 1, 2, 3]));
}

#[test]
fn cancellation_preflight_refusals_preserve_the_pending_window() {
    use std::num::NonZeroUsize;
    for pages in [2, 4, 8, usize::MAX] {
        for logged_prefix in [false, true] {
            let mut vol = mount_with_options(
                TraceBackend::new(formatted(4096, 8)),
                MountOptions {
                    tree_cache_pages: NonZeroUsize::new(pages),
                    ..Default::default()
                },
            )
            .unwrap();
            let file = vol
                .window_op(&create("kept", b"keep this"), ts(1))
                .unwrap()
                .unwrap();
            if logged_prefix {
                vol.window_fsync().unwrap();
            }
            vol.window_op(&create("pending", b"pending bytes"), ts(2))
                .unwrap();
            let pending = vol.window_unlogged_ops();
            let before = vol.device_mut().stats();
            assert!(matches!(
                vol.window_op(
                    &BatchOp::DeleteFile {
                        parent_id: file,
                        name: "child"
                    },
                    ts(3)
                ),
                Err(CoreError::NotDirectory)
            ));
            assert_eq!(
                vol.window_unlogged_ops(),
                pending,
                "validation lost the pending window"
            );
            assert!(matches!(
                vol.window_op(
                    &BatchOp::DeleteFile {
                        parent_id: u64::MAX,
                        name: "child"
                    },
                    ts(3)
                ),
                Err(CoreError::NotFound)
            ));
            assert_eq!(vol.window_unlogged_ops(), pending);
            assert!(matches!(
                vol.window_op(
                    &BatchOp::Rename {
                        source_parent_id: OBJECT_ROOT,
                        source_name: "",
                        target_parent_id: OBJECT_ROOT,
                        target_name: "kept",
                        replace: true,
                    },
                    ts(3)
                ),
                Err(CoreError::InvalidName(_))
            ));
            assert_eq!(vol.window_unlogged_ops(), pending);
            let after = vol.device_mut().stats();
            assert_eq!(
                (after.writes, after.flushes),
                (before.writes, before.flushes)
            );
            vol.window_fsync().unwrap();
            let mut recovered = mount_with_options(
                vol.into_device(),
                MountOptions {
                    tree_cache_pages: NonZeroUsize::new(pages),
                    ..Default::default()
                },
            )
            .unwrap();
            for (name, expected) in [
                ("kept", b"keep this".as_slice()),
                ("pending", b"pending bytes".as_slice()),
            ] {
                let id = recovered
                    .lookup_root(name)
                    .unwrap()
                    .expect("durable file survives");
                assert_eq!(recovered.read_file(id).unwrap(), expected);
            }
            let mut dev = recovered.into_device();
            assert!(check_device(&mut dev).is_clean());
        }
    }
}

#[test]
fn cancellation_preflight_read_failure_keeps_the_window_retryable() {
    use afsplus_block::{BlockDevice, BlockError};
    use std::num::NonZeroUsize;
    struct FailRead {
        inner: TraceBackend<MemoryBackend>,
        fail_next_read: bool,
    }
    impl BlockDevice for FailRead {
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }
        fn total_blocks(&self) -> u64 {
            self.inner.total_blocks()
        }
        fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
            if std::mem::take(&mut self.fail_next_read) {
                return Err(BlockError::Injected("cancellation preflight read"));
            }
            self.inner.read_block(lba, buf)
        }
        fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
            self.inner.write_block(lba, data)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            self.inner.flush()
        }
    }
    for pages in [2, 4, 8, usize::MAX] {
        for logged in [false, true] {
            let mut vol = mount_with_options(
                FailRead {
                    inner: TraceBackend::new(formatted(4096, 8)),
                    fail_next_read: false,
                },
                MountOptions {
                    tree_cache_pages: NonZeroUsize::new(pages),
                    ..Default::default()
                },
            )
            .unwrap();
            vol.window_op(&create("kept", b"first"), ts(1)).unwrap();
            if logged {
                vol.window_fsync().unwrap();
            }
            vol.window_op(&create("pending", b"second"), ts(2)).unwrap();
            let pending = vol.window_unlogged_ops();
            let before = vol.device_mut().inner.stats();
            vol.device_mut().fail_next_read = true;
            assert!(matches!(
                vol.window_op(
                    &BatchOp::DeleteFile {
                        parent_id: OBJECT_ROOT,
                        name: "absent",
                    },
                    ts(3)
                ),
                Err(CoreError::Block(BlockError::Injected(
                    "cancellation preflight read"
                )))
            ));
            assert_eq!(vol.window_unlogged_ops(), pending);
            let after = vol.device_mut().inner.stats();
            assert_eq!(
                (after.writes, after.flushes),
                (before.writes, before.flushes)
            );
            assert!(matches!(
                vol.window_op(
                    &BatchOp::DeleteFile {
                        parent_id: OBJECT_ROOT,
                        name: "absent",
                    },
                    ts(3)
                ),
                Err(CoreError::NotFound)
            ));
            vol.window_op(&create("retry", b"third"), ts(4)).unwrap();
            vol.window_fsync().unwrap();
            let mut recovered = mount(vol.into_device().inner.into_inner()).unwrap();
            for (name, data) in [
                ("kept", b"first".as_slice()),
                ("pending", b"second".as_slice()),
                ("retry", b"third".as_slice()),
            ] {
                let id = recovered.lookup_root(name).unwrap().unwrap();
                assert_eq!(recovered.read_file(id).unwrap(), data);
            }
            assert_eq!(recovered.list_root().unwrap().len(), 3);
            assert!(check_device(&mut recovered.into_device()).is_clean());
        }
    }
}
