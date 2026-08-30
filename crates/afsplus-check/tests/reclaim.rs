//! Reclaim Scale-2 qualification (ADR-036).
//!
//! Crash matrices force sealing, bounded consumption, cursor advancement,
//! and segment disappearance with deliberately tiny root areas and batch
//! budgets; the workload qualifications exceed the legacy 253-entry limit by
//! a wide margin (large COW rewrite, massive truncate, bulk unlink), and an
//! ignored millions-scale run drains ~1.9M quarantined blocks in bounded
//! steps.

use afsplus_block::{crash_states, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::reclaim::ReclaimCaps;
use afsplus_format::Timespec;

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn tiny_caps() -> ReclaimCaps {
    ReclaimCaps {
        inline_entries: 4,
        segment_refs: 3,
        table_refs: 8,
    }
}

fn format_volume(total: u64, region: u32, caps: ReclaimCaps) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, total);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [77u8; 16],
            label: "ReclaimVol".into(),
            region_size: region,
            reclaim_caps: caps,
            log_slots: 0,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

/// Runs reclaim steps until the queue stops shrinking; returns steps taken.
/// The queue never reaches zero: each step retires the previous root, so the
/// steady state keeps one block cycling in quarantine.
fn drain(vol: &mut Volume<MemoryBackend>) -> u64 {
    let mut steps = 0;
    while vol.reclaim_step(ts(1_000 + steps as i64)).unwrap() > 0 {
        steps += 1;
        assert!(steps < 10_000, "reclaim failed to converge");
    }
    steps
}

fn assert_clean(dev: &mut MemoryBackend, context: &str) {
    let report = check_device(dev);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
}

#[test]
fn tiny_caps_seal_segments_and_tables_and_drain_to_steady_state() {
    let dev = format_volume(256, 256, tiny_caps());
    let mut vol = mount(dev).unwrap();
    let initial_free = vol.free_blocks();

    // Every transaction retires ~4 blocks; with a 4-entry inline area the
    // queue seals segments continuously and eventually tables.
    vol.set_reclaim_batch_blocks(1); // almost no reclamation: force growth
    for i in 0..24 {
        vol.create_file_in_root(&format!("f{i}"), &[i as u8; 100], ts(i))
            .unwrap();
    }
    let stats = vol.last_commit_stats().unwrap().alloc.reclaim;
    assert!(vol.reclaim_pending_blocks() > 50, "backlog must have grown");
    let mut sealed_tables_seen = stats.tables_sealed;
    for i in 0..24 {
        vol.delete_file_in_root(&format!("f{i}"), ts(100 + i))
            .unwrap();
        sealed_tables_seen += vol.last_commit_stats().unwrap().alloc.reclaim.tables_sealed;
    }
    assert!(sealed_tables_seen > 0, "tiny caps must force table sealing");

    // Remount mid-backlog: bounded mount, full checker, then drain.
    let mut dev = vol.into_device();
    assert_clean(&mut dev, "mid-backlog");
    let mut vol = mount(dev).unwrap();
    vol.set_reclaim_batch_blocks(7);
    let steps = drain(&mut vol);
    assert!(
        steps > 5,
        "draining a large backlog must take multiple bounded steps"
    );
    // Steady state: the empty namespace occupies the same four metadata
    // blocks as after mkfs, plus one quarantined previous reclaim root.
    assert_eq!(vol.reclaim_pending_blocks(), 1);
    assert_eq!(vol.free_blocks(), initial_free - 1);

    let mut dev = vol.into_device();
    assert_clean(&mut dev, "drained");
}

#[test]
fn reclaim_survives_remount_and_resumes_from_the_cursor() {
    let dev = format_volume(256, 256, tiny_caps());
    let mut vol = mount(dev).unwrap();
    vol.set_reclaim_batch_blocks(1);
    // A multi-block data extent gives the cursor a run to stand inside.
    vol.create_file_in_root("big", &[0xC3u8; 3 * BS], ts(1))
        .unwrap();
    vol.delete_file_in_root("big", ts(2)).unwrap();
    let backlog = vol.reclaim_pending_blocks();
    assert!(backlog > 5);

    // Two 2-block steps leave the cursor mid-run; remount must resume it.
    vol.set_reclaim_batch_blocks(2);
    assert_eq!(vol.reclaim_step(ts(3)).unwrap(), 1); // 2 promoted, 1 re-appended
    let mut dev = vol.into_device();
    assert_clean(&mut dev, "mid-cursor");
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.reclaim_pending_blocks(), backlog - 1);
    vol.set_reclaim_batch_blocks(7);
    drain(&mut vol);
    assert_eq!(vol.reclaim_pending_blocks(), 1);
    let mut dev = vol.into_device();
    assert_clean(&mut dev, "resumed drain");
}

fn run_crash_matrix(
    base: &MemoryBackend,
    log: &[afsplus_block::RecordedOp],
    pre_generation: u64,
    context: &str,
    mut verify: impl FnMut(&str, &mut Volume<MemoryBackend>),
) {
    let mut pre = 0u64;
    let mut post = 0u64;
    for crash_point in 0..=log.len() {
        for state in crash_states(base, log, crash_point) {
            let what = format!("{context}: {}", state.description);
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(
                report.is_clean(),
                "{what}: checker findings {:?}",
                report.errors
            );
            let mut vol = mount(image).unwrap_or_else(|e| panic!("{what}: mount failed: {e}"));
            match vol.generation() {
                g if g == pre_generation => pre += 1,
                g if g == pre_generation + 1 => post += 1,
                g => panic!("{what}: recovered to disallowed generation {g}"),
            }
            verify(&what, &mut vol);
        }
    }
    assert!(
        pre > 0 && post > 0,
        "{context}: matrix must produce both outcomes"
    );
}

#[test]
fn crash_matrix_over_a_sealing_transaction() {
    // Tiny caps: this create seals at least one segment while committing.
    let base = {
        let dev = format_volume(256, 256, tiny_caps());
        let mut vol = mount(dev).unwrap();
        vol.set_reclaim_batch_blocks(1);
        for i in 0..3 {
            vol.create_file_in_root(&format!("seed{i}"), b"seed", ts(i))
                .unwrap();
        }
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();
    let pre_pending = mount(base.clone()).unwrap().reclaim_pending_blocks();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.set_reclaim_batch_blocks(1);
    vol.create_file_in_root("sealer", b"payload", ts(50))
        .unwrap();
    assert!(
        vol.last_commit_stats()
            .unwrap()
            .alloc
            .reclaim
            .segments_sealed
            > 0,
        "test precondition: the recorded transaction must seal a segment"
    );
    let (_, log) = vol.into_device().into_parts();

    run_crash_matrix(&base, &log, pre_generation, "sealing", |what, vol| {
        if vol.generation() == pre_generation {
            assert_eq!(vol.lookup_root("sealer").unwrap(), None, "{what}");
            assert_eq!(vol.reclaim_pending_blocks(), pre_pending, "{what}");
        } else {
            assert!(vol.lookup_root("sealer").unwrap().is_some(), "{what}");
        }
    });
}

#[test]
fn crash_matrix_over_segment_consumption_and_disappearance() {
    // Build a backlog with sealed segments, then record a reclaim step whose
    // batch consumes an entire segment (which therefore leaves the root and
    // is itself retired).
    let base = {
        let dev = format_volume(256, 256, tiny_caps());
        let mut vol = mount(dev).unwrap();
        vol.set_reclaim_batch_blocks(1);
        for i in 0..6 {
            vol.create_file_in_root(&format!("g{i}"), b"x", ts(i))
                .unwrap();
        }
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();
    let pre_pending = mount(base.clone()).unwrap().reclaim_pending_blocks();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.set_reclaim_batch_blocks(9);
    let reclaimed = vol.reclaim_step(ts(60)).unwrap();
    assert!(reclaimed > 0);
    let step_stats = vol.last_commit_stats().unwrap().alloc.reclaim;
    assert!(
        step_stats.structure_blocks_retired > 1,
        "test precondition: the batch must consume at least one whole segment"
    );
    let (_, log) = vol.into_device().into_parts();

    run_crash_matrix(&base, &log, pre_generation, "consumption", |what, vol| {
        if vol.generation() == pre_generation {
            assert_eq!(vol.reclaim_pending_blocks(), pre_pending, "{what}");
        } else {
            assert!(vol.reclaim_pending_blocks() < pre_pending, "{what}");
        }
    });
}

#[test]
fn crash_matrix_over_a_mid_run_cursor_advance() {
    // A 3-block run with a 2-block budget: the recorded step leaves the
    // persistent cursor inside a run.
    let base = {
        let dev = format_volume(256, 256, tiny_caps());
        let mut vol = mount(dev).unwrap();
        vol.set_reclaim_batch_blocks(1);
        vol.create_file_in_root("big", &[0xB7u8; 3 * BS], ts(1))
            .unwrap();
        vol.delete_file_in_root("big", ts(2)).unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.set_reclaim_batch_blocks(2);
    assert!(vol.reclaim_step(ts(3)).unwrap() > 0);
    let (_, log) = vol.into_device().into_parts();

    run_crash_matrix(
        &base,
        &log,
        pre_generation,
        "mid-run cursor",
        |what, vol| {
            // Whatever the outcome, the volume must be able to finish draining.
            vol.set_reclaim_batch_blocks(64);
            while vol.reclaim_step(ts(500)).unwrap() > 0 {}
            assert_eq!(
                vol.reclaim_pending_blocks(),
                1,
                "{what}: drain after recovery"
            );
        },
    );
}

#[test]
fn large_cow_rewrite_exceeds_the_legacy_253_entry_limit() {
    // A full COW rewrite of a 600-block file retires more blocks in one
    // transaction than the retired single-block list could ever record.
    let dev = format_volume(2048, 2048, ReclaimCaps::default());
    let mut vol = mount(dev).unwrap();
    vol.set_reclaim_batch_blocks(1);
    let content = vec![0x11u8; 600 * BS];
    let id = vol.create_file_in_root("data", &content, ts(1)).unwrap();

    let rewritten = vec![0x22u8; 600 * BS];
    vol.write_file_at(id, 0, &rewritten, ts(2)).unwrap();
    assert_eq!(vol.read_file(id).unwrap(), rewritten);
    assert!(
        vol.reclaim_pending_blocks() > 600,
        "the rewrite must quarantine the whole previous content: {} pending",
        vol.reclaim_pending_blocks()
    );

    let mut dev = vol.into_device();
    assert_clean(&mut dev, "after rewrite");
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.read_file(id).unwrap(), rewritten);
    vol.set_reclaim_batch_blocks(128);
    let steps = drain(&mut vol);
    assert!(
        steps >= 5,
        "600 blocks at 128 per step need multiple bounded steps"
    );
    assert_eq!(vol.reclaim_pending_blocks(), 1);
    let mut dev = vol.into_device();
    assert_clean(&mut dev, "after drain");
}

#[test]
fn massive_truncate_and_bulk_unlink_quarantine_and_drain() {
    let dev = format_volume(2048, 2048, ReclaimCaps::default());
    let mut vol = mount(dev).unwrap();
    vol.set_reclaim_batch_blocks(1);
    let initial_free = vol.free_blocks();
    let a = vol
        .create_file_in_root("a", &vec![0xAAu8; 400 * BS], ts(1))
        .unwrap();
    vol.create_file_in_root("b", &vec![0xBBu8; 400 * BS], ts(2))
        .unwrap();

    vol.truncate_file(a, 0, ts(3)).unwrap();
    assert!(vol.reclaim_pending_blocks() >= 400);
    vol.delete_file_in_root("b", ts(4)).unwrap();
    assert!(vol.reclaim_pending_blocks() >= 800);
    assert_eq!(vol.read_file(a).unwrap(), Vec::<u8>::new());

    let mut dev = vol.into_device();
    assert_clean(&mut dev, "after truncate+unlink");
    let mut vol = mount(dev).unwrap();
    vol.set_reclaim_batch_blocks(256);
    drain(&mut vol);
    assert_eq!(vol.reclaim_pending_blocks(), 1);
    // Everything except the empty file's record and the cycling root block
    // is free again.
    assert!(vol.free_blocks() >= initial_free - 3);
    let mut dev = vol.into_device();
    assert_clean(&mut dev, "after drain");
}

#[test]
#[ignore = "explicit millions-scale reclaim qualification"]
fn millions_of_blocks_drain_in_bounded_steps() {
    // 2M-block volume (8 GiB address space, sparse in memory). Preallocation
    // creates unwritten extents, so quarantining ~1.9M blocks costs no data
    // writes; the unlink turns them into a handful of run entries.
    let total: u64 = 1 << 21;
    let dev = format_volume(total, 32_768, ReclaimCaps::default());
    let mut vol = mount(dev).unwrap();
    vol.set_reclaim_batch_blocks(1);
    let prealloc_blocks: u64 = 1_900_000;
    let id = vol.create_file_in_root("huge", b"", ts(1)).unwrap();
    vol.preallocate_file(id, 0, prealloc_blocks * BS as u64, ts(2))
        .unwrap();
    vol.delete_file_in_root("huge", ts(3)).unwrap();
    let pending = vol.reclaim_pending_blocks();
    assert!(pending >= prealloc_blocks, "pending {pending}");

    vol.set_reclaim_batch_blocks(100_000);
    let mut steps = 0u64;
    while vol.reclaim_step(ts(10 + steps as i64)).unwrap() > 0 {
        steps += 1;
        let stats = vol.last_commit_stats().unwrap();
        assert!(stats.alloc.blocks_promoted <= 100_000);
        assert!(
            stats.alloc.reclaim.structure_blocks_written < 32,
            "structure writes per step must stay bounded"
        );
        assert!(steps < 100, "drain must converge in bounded steps");
    }
    assert!(
        steps >= 19,
        "1.9M blocks at 100k per step need many steps, got {steps}"
    );
    assert_eq!(vol.reclaim_pending_blocks(), 1);
    let mut dev = vol.into_device();
    assert_clean(&mut dev, "millions drained");
}

#[test]
fn staged_tree_discards_are_recycled_not_quarantined() {
    // Deleting back across the directory-root height boundary discards a
    // staged node (root collapse); it must return to FREE inside the same
    // transaction rather than entering quarantine.
    let dev = format_volume(1024, 1024, ReclaimCaps::default());
    let mut vol = mount(dev).unwrap();
    let mut names = Vec::new();
    for i in 0..220 {
        let name = format!("entry-with-a-fairly-long-name-{i:04}");
        vol.create_file_in_root(&name, b"", ts(i)).unwrap();
        names.push(name);
    }
    let mut released = 0u64;
    for (i, name) in names.iter().enumerate() {
        vol.delete_file_in_root(name, ts(1_000 + i as i64)).unwrap();
        released += vol.last_commit_stats().unwrap().alloc.blocks_released;
    }
    assert!(released > 0, "root collapses must release staged nodes");
    let mut dev = vol.into_device();
    assert_clean(&mut dev, "after release workload");
}
