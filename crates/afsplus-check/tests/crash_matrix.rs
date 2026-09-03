//! Crash matrix over single transactions, plus the mandated negative test:
//! a deliberately mis-ordered commit (checkpoint written before the metadata
//! barrier) MUST make the matrix find at least one invalid state.
//!
//! The model: power is cut after every recorded operation; every full-write
//! subset of the unflushed tail plus representative torn-write states must
//! remount to *exactly* the pre-commit or post-commit state and pass the
//! full checker. See `afsplus_block::powercut` for what the model does and
//! does not cover.

use std::collections::BTreeSet;

use afsplus_block::{
    crash_states, for_each_crash_state, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend,
};
use afsplus_check::check_device;
use afsplus_core::mount::select_checkpoint;
use afsplus_core::verify::load_committed_state;
use afsplus_core::{allocation_root, directory, mkfs, mount, MkfsParams};
use afsplus_format::ident::Identification;
use afsplus_format::object::ObjectType;
use afsplus_format::region::RegionDescriptor;
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn params(label: &str) -> MkfsParams {
    MkfsParams {
        uuid: [42u8; 16],
        label: label.into(),
        region_size: 64,
        reclaim_caps: Default::default(),
        log_slots: 8,
        shared_extents: true,
        data_policy: false,
        name_policy: afsplus_core::NamePolicy::Sensitive,
        timestamp: Timespec {
            seconds: 1_780_000_000,
            nanoseconds: 0,
        },
    }
}

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

/// Blocks a correct transaction must never write: everything reachable from
/// the committed state, its quarantined blocks, the identification block,
/// the current checkpoint slot, and every descriptor/bitmap slot referenced
/// by a retained checkpoint.
fn forbidden_targets(base: &MemoryBackend) -> BTreeSet<u64> {
    let mut dev = base.clone();
    let mut buf = vec![0u8; BS];
    dev.read_block(0, &mut buf).unwrap();
    let ident = Identification::decode(&buf).unwrap();
    let geo = ident.geometry();
    let selection = select_checkpoint(&mut dev, &ident).unwrap();
    let state = load_committed_state(&mut dev, &ident, &selection.chosen).unwrap();

    let mut forbidden: BTreeSet<u64> = BTreeSet::new();
    forbidden.insert(0);
    forbidden.insert(ident.checkpoint_slots[selection.chosen_slot]);
    forbidden.extend(state.metadata_blocks.iter().copied());
    forbidden.extend(state.data_blocks.iter().copied());
    // Quarantined blocks may be *reused by allocation* in the next
    // transaction — that is the whole point — so they are not forbidden.
    for ckpt in std::iter::once(&selection.chosen).chain(selection.other.iter()) {
        let allocation =
            allocation_root::load_all(&mut dev, &geo, ckpt.allocation_root_block, ckpt.generation)
                .unwrap();
        forbidden.extend(allocation.tree_blocks);
        for (r, record) in allocation.records.iter().enumerate() {
            let region = r as u32;
            let descriptor_lba = geo.descriptor_slot_lba(region, record.descriptor_slot);
            forbidden.insert(descriptor_lba);
            dev.read_block(descriptor_lba, &mut buf).unwrap();
            let (descriptor, _) = RegionDescriptor::decode(&buf).unwrap();
            for (page_index, binding) in descriptor.pages.iter().enumerate() {
                forbidden.insert(geo.bitmap_slot_lba(region, page_index as u32, binding.slot));
            }
        }
    }
    forbidden
}

fn run_matrix(
    base: &MemoryBackend,
    log: &[RecordedOp],
    pre_generation: u64,
    mut verify: impl FnMut(&str, afsplus_core::Volume<MemoryBackend>),
) {
    for crash_point in 0..=log.len() {
        for_each_crash_state(base, log, crash_point, |state| {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(
                report.is_clean(),
                "{context}: checker findings {:?}",
                report.errors
            );
            let vol = mount(image).unwrap_or_else(|e| panic!("{context}: mount failed: {e}"));
            assert!(
                vol.generation() == pre_generation || vol.generation() == pre_generation + 1,
                "{context}: recovered to disallowed generation {}",
                vol.generation()
            );
            verify(&context, vol);
        });
    }
}

fn root_directory_height<D: BlockDevice>(vol: &mut afsplus_core::Volume<D>) -> u8 {
    let root = vol.stat(OBJECT_ROOT).unwrap().unwrap();
    let geo = vol.ident().geometry();
    let ident = vol.ident().clone();
    let generation = vol.generation();
    directory::load_all(
        vol.device_mut(),
        &geo,
        root.data_root,
        OBJECT_ROOT,
        generation,
        &ident,
    )
    .unwrap()
    .summary
    .height
}

#[test]
fn every_crash_state_of_a_create_transaction_recovers_to_an_allowed_state() {
    let mut base = MemoryBackend::new(BS, 64);
    mkfs(&mut base, &params("CrashVol")).unwrap();

    let forbidden = forbidden_targets(&base);
    let pre_free = mount(base.clone()).unwrap().free_blocks();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    let file_id = vol
        .create_file_in_root("hello.txt", &[0x5Au8; 4000], ts(1))
        .unwrap();
    let (_, log) = vol.into_device().into_parts();

    // --- Structural discipline of the commit sequence -------------------
    // data, barrier, 6 metadata (including allocation-root COW) + 1 bitmap
    // page + 1 region descriptor,
    // barrier, checkpoint, barrier.
    let shape: Vec<&'static str> = log
        .iter()
        .map(|op| match op {
            RecordedOp::Write { .. } => "w",
            RecordedOp::Flush => "F",
        })
        .collect();
    assert_eq!(
        shape,
        ["w", "F", "w", "w", "w", "w", "w", "w", "w", "w", "F", "w", "F"],
        "commit sequence changed"
    );
    for op in &log {
        if let RecordedOp::Write { lba, .. } = op {
            assert!(
                !forbidden.contains(lba),
                "transaction wrote committed block {lba}"
            );
        }
    }

    // --- The matrix ------------------------------------------------------
    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    run_matrix(&base, &log, 1, |context, mut vol| {
        if vol.generation() == 1 {
            pre_outcomes += 1;
            assert!(
                vol.list_root().unwrap().is_empty(),
                "{context}: pre state shows the new file"
            );
            assert_eq!(vol.free_blocks(), pre_free, "{context}");
        } else {
            post_outcomes += 1;
            assert_eq!(
                vol.lookup_root("hello.txt").unwrap(),
                Some(file_id),
                "{context}"
            );
            let record = vol.stat(file_id).unwrap().unwrap();
            assert_eq!(record.object_type, ObjectType::File);
            assert_eq!(
                vol.read_file(file_id).unwrap(),
                vec![0x5Au8; 4000],
                "{context}"
            );
        }
    });
    assert!(
        pre_outcomes > 0,
        "matrix never produced a pre-commit recovery"
    );
    assert!(
        post_outcomes > 0,
        "matrix never produced a post-commit recovery"
    );
}

#[test]
fn every_crash_state_of_cross_directory_rename_is_atomic() {
    let mut initial = MemoryBackend::new(BS, 256);
    mkfs(
        &mut initial,
        &MkfsParams {
            uuid: [77u8; 16],
            label: "RenameCrash".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut vol = mount(initial).unwrap();
    let left = vol.create_directory_in_root("left", ts(1)).unwrap();
    let right = vol.create_directory_in_root("right", ts(2)).unwrap();
    let file = vol
        .create_file_in_directory(left, "before.txt", b"rename payload", ts(3))
        .unwrap();
    let base = vol.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let forbidden = forbidden_targets(&base);

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.rename(left, "before.txt", right, "after.txt", ts(4))
        .unwrap();
    let (_, log) = vol.into_device().into_parts();
    for operation in &log {
        if let RecordedOp::Write { lba, .. } = operation {
            assert!(
                !forbidden.contains(lba),
                "rename transaction wrote committed block {lba}"
            );
        }
    }

    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    run_matrix(&base, &log, pre_generation, |context, mut vol| {
        if vol.generation() == pre_generation {
            pre_outcomes += 1;
            assert_eq!(
                vol.lookup_in_directory(left, "before.txt").unwrap(),
                Some(file),
                "{context}"
            );
            assert_eq!(
                vol.lookup_in_directory(right, "after.txt").unwrap(),
                None,
                "{context}"
            );
        } else {
            post_outcomes += 1;
            assert_eq!(
                vol.lookup_in_directory(left, "before.txt").unwrap(),
                None,
                "{context}"
            );
            assert_eq!(
                vol.lookup_in_directory(right, "after.txt").unwrap(),
                Some(file),
                "{context}"
            );
        }
        assert_eq!(vol.read_file(file).unwrap(), b"rename payload", "{context}");
    });
    assert!(pre_outcomes > 0, "matrix never produced a pre-rename state");
    assert!(
        post_outcomes > 0,
        "matrix never produced a post-rename state"
    );
}

#[test]
fn directory_root_split_and_collapse_are_crash_atomic() {
    let mut initial = MemoryBackend::new(BS, 2_048);
    mkfs(
        &mut initial,
        &MkfsParams {
            uuid: [0x5c; 16],
            label: "DirectoryHeightCrash".into(),
            region_size: 2_048,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut vol = mount(initial).unwrap();
    let mut boundary = None;
    for index in 0..200u64 {
        let height_before = root_directory_height(&mut vol);
        let base = vol.device_mut().clone();
        let name = format!("height-{index:04}");
        let object_id = vol
            .create_file_in_root(&name, b"", ts(index as i64 + 1))
            .unwrap();
        let height_after = root_directory_height(&mut vol);
        if height_after > height_before {
            boundary = Some((base, name, object_id));
            break;
        }
    }
    let (split_base, boundary_name, boundary_id) =
        boundary.expect("directory did not split within the qualification bound");
    let mut pre = mount(split_base.clone()).unwrap();
    let split_generation = pre.generation();
    let entries_before = pre.list_root().unwrap().len();
    assert_eq!(root_directory_height(&mut pre), 1);

    let forbidden = forbidden_targets(&split_base);
    let mut split = mount(RecordingBackend::new(split_base.clone())).unwrap();
    let replayed_id = split
        .create_file_in_root(&boundary_name, b"", ts(1_000))
        .unwrap();
    assert_eq!(replayed_id, boundary_id);
    assert_eq!(root_directory_height(&mut split), 2);
    let (collapse_base, split_log) = split.into_device().into_parts();
    for operation in &split_log {
        if let RecordedOp::Write { lba, .. } = operation {
            assert!(
                !forbidden.contains(lba),
                "directory split overwrote committed block {lba}"
            );
        }
    }
    let mut split_pre = 0u64;
    let mut split_post = 0u64;
    run_matrix(
        &split_base,
        &split_log,
        split_generation,
        |context, mut recovered| {
            if recovered.generation() == split_generation {
                split_pre += 1;
                assert_eq!(
                    recovered.lookup_root(&boundary_name).unwrap(),
                    None,
                    "{context}"
                );
                assert_eq!(root_directory_height(&mut recovered), 1, "{context}");
                assert_eq!(
                    recovered.list_root().unwrap().len(),
                    entries_before,
                    "{context}"
                );
            } else {
                split_post += 1;
                assert_eq!(
                    recovered.lookup_root(&boundary_name).unwrap(),
                    Some(boundary_id),
                    "{context}"
                );
                assert_eq!(root_directory_height(&mut recovered), 2, "{context}");
                assert_eq!(
                    recovered.list_root().unwrap().len(),
                    entries_before + 1,
                    "{context}"
                );
            }
        },
    );
    assert!(split_pre > 0 && split_post > 0);

    let collapse_generation = split_generation + 1;
    let forbidden = forbidden_targets(&collapse_base);
    let mut collapse = mount(RecordingBackend::new(collapse_base.clone())).unwrap();
    collapse
        .delete_file_in_root(&boundary_name, ts(2_000))
        .unwrap();
    assert_eq!(root_directory_height(&mut collapse), 1);
    let (_, collapse_log) = collapse.into_device().into_parts();
    for operation in &collapse_log {
        if let RecordedOp::Write { lba, .. } = operation {
            assert!(
                !forbidden.contains(lba),
                "directory collapse overwrote committed block {lba}"
            );
        }
    }
    let mut collapse_pre = 0u64;
    let mut collapse_post = 0u64;
    run_matrix(
        &collapse_base,
        &collapse_log,
        collapse_generation,
        |context, mut recovered| {
            if recovered.generation() == collapse_generation {
                collapse_pre += 1;
                assert_eq!(
                    recovered.lookup_root(&boundary_name).unwrap(),
                    Some(boundary_id),
                    "{context}"
                );
                assert_eq!(root_directory_height(&mut recovered), 2, "{context}");
            } else {
                collapse_post += 1;
                assert_eq!(
                    recovered.lookup_root(&boundary_name).unwrap(),
                    None,
                    "{context}"
                );
                assert_eq!(root_directory_height(&mut recovered), 1, "{context}");
            }
        },
    );
    assert!(collapse_pre > 0 && collapse_post > 0);
}

#[test]
fn every_crash_state_of_a_sparse_write_is_atomic() {
    let mut initial = MemoryBackend::new(BS, 256);
    mkfs(
        &mut initial,
        &MkfsParams {
            uuid: [88u8; 16],
            label: "SparseCrash".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut vol = mount(initial).unwrap();
    let before = vec![0x31; BS + 37];
    let file = vol
        .create_file_in_root("atomic.bin", &before, ts(1))
        .unwrap();
    let base = vol.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();
    let forbidden = forbidden_targets(&base);

    let offset = BS as u64 * 3 + 11;
    let mut after = before.clone();
    after.resize(offset as usize, 0);
    after.extend_from_slice(b"after");
    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.write_file_at(file, offset, b"after", ts(2)).unwrap();
    let (_, log) = vol.into_device().into_parts();
    for operation in &log {
        if let RecordedOp::Write { lba, .. } = operation {
            assert!(
                !forbidden.contains(lba),
                "sparse write transaction overwrote committed block {lba}"
            );
        }
    }

    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    run_matrix(&base, &log, pre_generation, |context, mut vol| {
        let recovered = vol.read_file(file).unwrap();
        if vol.generation() == pre_generation {
            pre_outcomes += 1;
            assert_eq!(recovered, before, "{context}");
        } else {
            post_outcomes += 1;
            assert_eq!(recovered, after, "{context}");
        }
    });
    assert!(
        pre_outcomes > 0,
        "matrix never produced the pre-write state"
    );
    assert!(
        post_outcomes > 0,
        "matrix never produced the post-write state"
    );
}

#[test]
fn multi_node_allocation_root_commit_has_an_exhaustive_crash_matrix() {
    // 4 KiB AFST allocation leaves hold 144 region records. Crossing that
    // boundary forces a two-level authoritative allocation root while keeping
    // the sparse in-memory base small enough for an exhaustive matrix.
    const REGIONS: u64 = 145;
    const REGION_BLOCKS: u32 = 16;
    let mut base = MemoryBackend::new(BS, REGIONS * REGION_BLOCKS as u64);
    mkfs(
        &mut base,
        &MkfsParams {
            uuid: [99u8; 16],
            label: "MultiAllocCrash".into(),
            region_size: REGION_BLOCKS,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut inspection = base.clone();
    let mounted = mount(inspection.clone()).unwrap();
    let pre_generation = mounted.generation();
    let allocation = allocation_root::load_all(
        &mut inspection,
        &mounted.ident().geometry(),
        mounted.checkpoint().allocation_root_block,
        pre_generation,
    )
    .unwrap();
    assert!(allocation.summary.nodes > 1);
    let forbidden = forbidden_targets(&base);

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    let file = vol
        .create_file_in_root("multi-root.txt", b"", ts(1))
        .unwrap();
    let (_, log) = vol.into_device().into_parts();
    for operation in &log {
        if let RecordedOp::Write { lba, .. } = operation {
            assert!(
                !forbidden.contains(lba),
                "multi-node allocation transaction overwrote committed block {lba}"
            );
        }
    }

    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    run_matrix(&base, &log, pre_generation, |context, mut vol| {
        if vol.generation() == pre_generation {
            pre_outcomes += 1;
            assert_eq!(
                vol.lookup_root("multi-root.txt").unwrap(),
                None,
                "{context}"
            );
        } else {
            post_outcomes += 1;
            assert_eq!(
                vol.lookup_root("multi-root.txt").unwrap(),
                Some(file),
                "{context}"
            );
        }
    });
    assert!(pre_outcomes > 0);
    assert!(post_outcomes > 0);
}

#[test]
fn misordered_commit_checkpoint_before_metadata_barrier_is_caught() {
    // Negative control for the whole harness: replay the recorded commit
    // with the checkpoint write moved BEFORE the metadata barrier. The
    // matrix must now find at least one durable state where the newest
    // structurally valid checkpoint references metadata that never became
    // durable — and mount must report it as corruption rather than silently
    // falling back to the older checkpoint.
    let mut base = MemoryBackend::new(BS, 64);
    mkfs(&mut base, &params("BadOrder")).unwrap();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.create_file_in_root("hello.txt", b"", ts(1)).unwrap();
    let (_, log) = vol.into_device().into_parts();

    // Move the alternate-checkpoint write before the metadata barrier.
    let mut bad_log = log.clone();
    let checkpoint_index = bad_log
        .iter()
        .position(|op| matches!(op, RecordedOp::Write { lba: 2, .. }))
        .unwrap();
    let ckpt_write = bad_log.remove(checkpoint_index);
    assert!(matches!(ckpt_write, RecordedOp::Write { .. }));
    let first_flush = bad_log
        .iter()
        .position(|op| matches!(op, RecordedOp::Flush))
        .unwrap();
    bad_log.insert(first_flush, ckpt_write);

    let mut invalid_states = 0u64;
    let mut root_rejections = 0u64;
    let mut deferred_rejections = 0u64;
    for crash_point in 0..=bad_log.len() {
        for state in crash_states(&base, &bad_log, crash_point) {
            let mut image = state.image;
            let report = check_device(&mut image);
            match mount(image) {
                Ok(mut vol) => {
                    if !report.is_clean() {
                        invalid_states += 1;
                        // Bounded mount may have read intact roots while a
                        // descendant record is absent. Accessing every root
                        // child must then surface corruption; the full
                        // checker remains the exhaustive transaction oracle.
                        let mut access_failed = false;
                        for (_, object_id) in vol.list_root().unwrap() {
                            if vol.stat(object_id).is_err() {
                                access_failed = true;
                                break;
                            }
                        }
                        if access_failed {
                            deferred_rejections += 1;
                        }
                    }
                }
                Err(e) => {
                    invalid_states += 1;
                    root_rejections += 1;
                    assert!(
                        !report.is_clean(),
                        "{}: mount rejected the state but the checker passed it: {e}",
                        state.description
                    );
                }
            }
        }
    }
    assert!(
        invalid_states > 0,
        "the matrix failed to catch the mis-ordered commit"
    );
    assert!(
        root_rejections > 0,
        "bad ordering never damaged a bounded mount root"
    );
    assert!(
        deferred_rejections > 0,
        "matrix never exercised corruption discovered by on-demand access"
    );
}

#[test]
fn crash_matrix_across_a_second_transaction() {
    // Same property for a transaction that starts from a non-trivial state
    // (with quarantined blocks to promote) and commits back into slot A.
    let mut base = MemoryBackend::new(BS, 64);
    mkfs(&mut base, &params("CrashVol2")).unwrap();
    let mut vol = mount(base).unwrap();
    let first_id = vol.create_file_in_root("first.txt", b"one", ts(1)).unwrap();
    let base = vol.into_device();

    let forbidden = forbidden_targets(&base);
    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    let second_id = vol
        .create_file_in_root("second.txt", b"two", ts(2))
        .unwrap();
    let (_, log) = vol.into_device().into_parts();

    for op in &log {
        if let RecordedOp::Write { lba, .. } = op {
            assert!(
                !forbidden.contains(lba),
                "transaction wrote committed block {lba}"
            );
        }
    }

    run_matrix(&base, &log, 2, |context, mut vol| {
        assert_eq!(
            vol.lookup_root("first.txt").unwrap(),
            Some(first_id),
            "{context}"
        );
        if vol.generation() == 2 {
            assert_eq!(vol.lookup_root("second.txt").unwrap(), None, "{context}");
        } else {
            assert_eq!(
                vol.lookup_root("second.txt").unwrap(),
                Some(second_id),
                "{context}"
            );
        }
    });
}
