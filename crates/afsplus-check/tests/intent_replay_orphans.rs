//! Intent-log final-link recovery must enter ADR-066 orphan state instead of
//! retiring a victim's complete layout during the replay checkpoint.

use afsplus_block::{crash_states, MemoryBackend, RecordedOp, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{
    mkfs, mount, mount_with_options, MkfsParams, MountMode, MountOptions, NamePolicy,
};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

const BS: usize = DEFAULT_BLOCK_SIZE;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(total_blocks: u64) -> MemoryBackend {
    let mut device = MemoryBackend::new(BS, total_blocks);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x28; 16],
            label: "ReplayOrphans".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    device
}

fn logged_delete(base: MemoryBackend, name: &str, now: Timespec) -> MemoryBackend {
    let mut volume = mount(base).unwrap();
    volume
        .window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name,
            },
            now,
        )
        .unwrap();
    volume.window_fsync().unwrap();
    volume.into_device()
}

fn replay_operations(logged: &MemoryBackend) -> Vec<RecordedOp> {
    let volume = mount(RecordingBackend::new(logged.clone())).unwrap();
    volume.into_device().into_parts().1
}

fn assert_clean(context: &str, device: &mut MemoryBackend) {
    let report = check_device(device);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
}

#[test]
fn fragmented_final_delete_replays_through_orphan_state_near_enospc() {
    const EXTENTS: u64 = 33;
    let mut volume = mount(formatted(4096)).unwrap();
    let victim = volume
        .create_file_in_root("fragmented", b"", ts(1))
        .unwrap();
    for extent in 0..EXTENTS {
        volume
            .write_file_at(
                victim,
                extent * 2 * BS as u64,
                &[extent as u8; BS],
                ts(2 + extent as i64),
            )
            .unwrap();
    }
    let victim_bytes = volume.read_file(victim).unwrap();
    let victim_allocated = volume
        .visible_metadata(victim)
        .unwrap()
        .unwrap()
        .allocated_bytes;

    // Consume ordinary capacity while preserving only a small slice above
    // the emergency floor. Replay is privileged maintenance and must still
    // be able to publish the bounded namespace transition.
    let filler = volume.create_file_in_root("filler", b"", ts(40)).unwrap();
    let fill_blocks = volume.available_blocks().saturating_sub(24);
    volume
        .preallocate_file(filler, 0, fill_blocks * BS as u64, ts(41))
        .unwrap();
    assert!(volume.available_blocks() <= 32);
    let pre_generation = volume.generation();

    let logged = logged_delete(volume.into_device(), "fragmented", ts(42));
    let mut raw = mount_with_options(
        logged.clone(),
        MountOptions {
            mode: MountMode::NoChanges,
        },
    )
    .unwrap();
    assert_eq!(raw.generation(), pre_generation);
    assert_eq!(raw.pending_intent_records(), 1);
    assert_eq!(raw.orphan_count().unwrap(), 0);

    let mut recovered = mount(logged).unwrap();
    assert_eq!(recovered.lookup_root("fragmented").unwrap(), None);
    assert!(recovered.orphan_object(victim).unwrap());
    assert_eq!(recovered.read_file(victim).unwrap(), victim_bytes);
    assert_eq!(
        recovered
            .visible_metadata(victim)
            .unwrap()
            .unwrap()
            .allocated_bytes,
        victim_allocated
    );
    let replay = recovered.last_commit_stats().unwrap();
    assert!(
        replay.alloc.blocks_retired < EXTENTS,
        "replay retired {} blocks for a {EXTENTS}-extent victim",
        replay.alloc.blocks_retired
    );
    assert_eq!(replay.data_blocks_written, 0);

    recovered.set_orphan_cleanup_extent_budget(3);
    let cleanup = recovered.cleanup_orphan(victim, ts(43)).unwrap();
    assert_eq!(cleanup.extents_removed, 3);
    assert!(cleanup.still_pending);
    let mut device = recovered.into_device();
    assert_clean("fragmented low-space replay and first cleanup", &mut device);
}

#[test]
fn one_logged_group_can_lazily_create_many_orphans() {
    const FILES: usize = 64;
    let mut volume = mount(formatted(16_384)).unwrap();
    let mut names = Vec::with_capacity(FILES);
    for index in 0..FILES {
        let name = format!("f{index:02x}");
        volume
            .create_file_in_root(&name, &[index as u8], ts(index as i64 + 1))
            .unwrap();
        names.push(name);
    }
    let pre_generation = volume.generation();
    for (index, name) in names.iter().enumerate() {
        volume
            .window_op(
                &BatchOp::DeleteFile {
                    parent_id: OBJECT_ROOT,
                    name,
                },
                ts(100),
            )
            .unwrap();
        if (index + 1) % 16 == 0 {
            volume.window_fsync().unwrap();
        }
    }

    let mut recovered = mount(volume.into_device()).unwrap();
    assert_eq!(recovered.generation(), pre_generation + 1);
    assert_eq!(recovered.orphan_count().unwrap(), FILES as u64);
    for name in &names {
        assert_eq!(recovered.lookup_root(name).unwrap(), None, "{name}");
    }
    let mut device = recovered.into_device();
    assert_clean("many-orphan lazy replay", &mut device);
}

#[test]
fn logged_write_then_final_delete_preserves_the_final_orphan_bytes() {
    let mut volume = mount(formatted(8192)).unwrap();
    let object = volume
        .create_file_in_root("database", &[0x11; 2 * BS], ts(1))
        .unwrap();
    volume
        .window_write_file_at(object, (BS - 17) as u64, &[0xa5; BS + 31], ts(2))
        .unwrap();
    volume.window_fsync().unwrap();
    let expected = volume.read_file(object).unwrap();
    volume
        .window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "database",
            },
            ts(3),
        )
        .unwrap();
    volume.window_fsync().unwrap();

    let mut recovered = mount(volume.into_device()).unwrap();
    assert_eq!(recovered.lookup_root("database").unwrap(), None);
    assert!(recovered.orphan_object(object).unwrap());
    assert_eq!(recovered.read_file(object).unwrap(), expected);
    let mut device = recovered.into_device();
    assert_clean("write then final delete replay", &mut device);
}

#[test]
fn every_replacing_replay_cut_converges_to_visible_new_and_orphaned_old() {
    let mut setup = mount(formatted(8192)).unwrap();
    let old = setup
        .create_file_in_root("target", b"old-content", ts(1))
        .unwrap();
    for extent in 1..5u64 {
        setup
            .write_file_at(
                old,
                extent * 2 * BS as u64,
                &[0xa0 + extent as u8; BS],
                ts(1 + extent as i64),
            )
            .unwrap();
    }
    let old_bytes = setup.read_file(old).unwrap();
    let incoming = setup
        .create_file_in_root("incoming", b"new-content", ts(7))
        .unwrap();
    let pre_generation = setup.generation();
    setup
        .window_op(
            &BatchOp::Rename {
                source_parent_id: OBJECT_ROOT,
                source_name: "incoming",
                target_parent_id: OBJECT_ROOT,
                target_name: "target",
                replace: true,
            },
            ts(8),
        )
        .unwrap();
    setup.window_fsync().unwrap();
    let logged = setup.into_device();
    let operations = replay_operations(&logged);

    let mut pre = 0u64;
    let mut post = 0u64;
    for cut in 0..=operations.len() {
        for state in crash_states(&logged, &operations, cut) {
            let context = state.description;
            let raw = mount_with_options(
                state.image.clone(),
                MountOptions {
                    mode: MountMode::NoChanges,
                },
            )
            .unwrap_or_else(|error| panic!("{context}: raw mount: {error}"));
            match raw.generation() {
                generation if generation == pre_generation => pre += 1,
                generation if generation == pre_generation + 1 => post += 1,
                generation => panic!("{context}: unexpected generation {generation}"),
            }

            let mut recovered = mount(state.image)
                .unwrap_or_else(|error| panic!("{context}: recovery mount: {error}"));
            assert_eq!(recovered.generation(), pre_generation + 1, "{context}");
            assert_eq!(recovered.lookup_root("target").unwrap(), Some(incoming));
            assert_eq!(recovered.lookup_root("incoming").unwrap(), None);
            assert!(recovered.orphan_object(old).unwrap(), "{context}");
            assert_eq!(recovered.read_file(old).unwrap(), old_bytes, "{context}");
            assert_eq!(recovered.read_file(incoming).unwrap(), b"new-content");
            let mut device = recovered.into_device();
            assert_clean(&context, &mut device);
        }
    }
    assert!(pre > 0 && post > 0);
}
