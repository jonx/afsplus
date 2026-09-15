//! Q3 allocation-state qualification under low-space and multi-region
//! pressure. These tests complement the existing per-boundary crash matrices:
//! failed allocations must publish nothing, while destructive operations and
//! bounded reclaim must keep making progress close to ENOSPC.

use afsplus_block::{
    for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, mount_with_options, CoreError, MkfsParams, MountOptions, Volume};
use afsplus_format::geometry::MAX_REGION_BLOCKS;
use afsplus_format::Timespec;
use std::num::NonZeroUsize;
use std::time::Instant;

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(total_blocks: u64, region_size: u32) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, total_blocks);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0xA3; 16],
            label: "AllocationPressure".into(),
            region_size,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

fn profile_mount<D: BlockDevice>(device: D, pages: usize) -> Volume<D> {
    let volume = mount_with_options(
        device,
        MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    volume
}

/// These fixtures format no intent log: any checker warning, including a
/// retained-checkpoint finding, fails the oracle.
fn assert_checker_clean<D: BlockDevice>(device: &mut D, context: &str) {
    let report = check_device(device);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
    assert!(
        report.warnings.is_empty(),
        "{context}: {:?}",
        report.warnings
    );
}

#[derive(Default)]
struct SpillEvidence {
    generation: u64,
    spills: u64,
    peak: u64,
}

impl SpillEvidence {
    fn observe<D: BlockDevice>(&mut self, volume: &Volume<D>, pages: usize) {
        if volume.generation() == self.generation {
            return;
        }
        self.generation = volume.generation();
        if let Some(stats) = volume.last_commit_stats() {
            self.spills += stats.tree_mutations.staged_spill_writes;
            self.peak = self
                .peak
                .max(stats.tree_mutations.max_resident_staged_nodes);
            assert!(stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
        }
    }
}

#[test]
fn near_full_enospc_publishes_nothing_and_delete_can_recover_space() {
    for pages in [2, 4, 8, usize::MAX] {
        near_full_enospc_publishes_nothing_and_delete_can_recover_space_profile(pages);
    }
}

fn near_full_enospc_publishes_nothing_and_delete_can_recover_space_profile(pages: usize) {
    let mut evidence = SpillEvidence::default();
    let dev = formatted(512, 512);
    let mut vol = profile_mount(TraceBackend::new(dev), pages);
    vol.set_reclaim_batch_blocks(1);
    let file = vol.create_file_in_root("reserve", b"", ts(1)).unwrap();
    evidence.observe(&vol, pages);
    let before_fill = vol.free_blocks();
    let emergency_headroom = vol.emergency_headroom_blocks();
    assert_eq!(
        vol.available_blocks(),
        before_fill.saturating_sub(emergency_headroom)
    );
    let before_generation = vol.generation();
    let before_metadata = vol.stat(file).unwrap().unwrap();
    let normally_available = vol.available_blocks();
    vol.device_mut().reset();
    assert!(matches!(
        vol.preallocate_file(file, 0, normally_available * BS as u64, ts(2)),
        Err(CoreError::NoSpace)
    ));
    let floor_failure_io = vol.device_mut().stats();
    assert_eq!(floor_failure_io.writes, 0);
    assert_eq!(floor_failure_io.flushes, 0);
    assert_eq!(vol.generation(), before_generation);
    assert_eq!(vol.free_blocks(), before_fill);
    assert_eq!(vol.stat(file).unwrap().unwrap(), before_metadata);

    let fill_blocks = before_fill - 24;
    vol.device_mut().reset();
    vol.preallocate_file(file, 0, fill_blocks * BS as u64, ts(2))
        .unwrap();
    evidence.observe(&vol, pages);
    let near_full = vol.free_blocks();
    assert!(near_full <= 24, "fill left {near_full} free blocks");
    assert!(near_full >= emergency_headroom);
    assert_eq!(vol.available_blocks(), near_full - emergency_headroom);

    let generation = vol.generation();
    let metadata = vol.stat(file).unwrap().unwrap();
    vol.device_mut().reset();
    assert!(matches!(
        vol.preallocate_file(
            file,
            fill_blocks * BS as u64,
            (near_full + 1) * BS as u64,
            ts(3),
        ),
        Err(CoreError::NoSpace)
    ));
    let failed_io = vol.device_mut().stats();
    assert_eq!(failed_io.writes, 0, "ENOSPC issued media writes");
    assert_eq!(failed_io.flushes, 0, "ENOSPC issued a durability barrier");
    assert_eq!(vol.generation(), generation);
    assert_eq!(vol.free_blocks(), near_full);
    assert_eq!(vol.stat(file).unwrap().unwrap(), metadata);

    vol.device_mut().reset();
    vol.delete_file_in_root("reserve", ts(4)).unwrap();
    assert_eq!(vol.lookup_root("reserve").unwrap(), None);
    evidence.observe(&vol, pages);
    let free_after_delete = vol.free_blocks();
    assert!(
        free_after_delete <= near_full,
        "quarantined extents became free before a safe generation"
    );

    vol.set_reclaim_batch_blocks(fill_blocks);
    assert!(vol.reclaim_step(ts(5)).unwrap() > 0);
    assert!(
        vol.free_blocks() > near_full,
        "bounded reclaim did not restore usable space"
    );
    evidence.observe(&vol, pages);
    let retry = vol.create_file_in_root("retry", b"", ts(6)).unwrap();
    evidence.observe(&vol, pages);
    vol.preallocate_file(
        retry,
        fill_blocks * BS as u64,
        (near_full + 1) * BS as u64,
        ts(7),
    )
    .unwrap();
    evidence.observe(&vol, pages);
    vol.write_file_at(retry, 0, b"recovered capacity", ts(8))
        .unwrap();
    evidence.observe(&vol, pages);
    assert_eq!(vol.read_file(retry).unwrap(), b"recovered capacity");
    let mut dev = vol.into_device().into_inner();
    assert_checker_clean(&mut dev, &format!("pages={pages} retry"));
    let mut remounted = profile_mount(dev, pages);
    assert_eq!(remounted.lookup_root("reserve").unwrap(), None);
    assert_eq!(remounted.lookup_root("retry").unwrap(), Some(retry));
    assert_eq!(remounted.read_file(retry).unwrap(), b"recovered capacity");
    assert_checker_clean(remounted.device_mut(), &format!("pages={pages} remount"));
    eprintln!(
        "low-space refusal pages={pages} spills={} peak_staged={}",
        evidence.spills, evidence.peak
    );
}

#[test]
fn near_full_delete_survives_every_modeled_power_cut() {
    let mut vol = mount(formatted(512, 512)).unwrap();
    vol.set_reclaim_batch_blocks(1);
    let file = vol.create_file_in_root("victim", b"", ts(1)).unwrap();
    let fill_blocks = vol.free_blocks() - 24;
    vol.preallocate_file(file, 0, fill_blocks * BS as u64, ts(2))
        .unwrap();
    let base = vol.into_device();
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut recorded = mount(RecordingBackend::new(base.clone())).unwrap();
    recorded.set_reclaim_batch_blocks(1);
    recorded.delete_file_in_root("victim", ts(3)).unwrap();
    let (_, log) = recorded.into_device().into_parts();
    let mut pre_states = 0u64;
    let mut post_states = 0u64;
    for crash_point in 0..=log.len() {
        for_each_crash_state(&base, &log, crash_point, |state| {
            let context = state.description;
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(
                report.is_clean(),
                "{context}: checker findings {:?}",
                report.errors
            );
            let mut recovered =
                mount(image).unwrap_or_else(|error| panic!("{context}: mount failed: {error}"));
            match recovered.generation() {
                generation if generation == pre_generation => {
                    pre_states += 1;
                    assert_eq!(recovered.lookup_root("victim").unwrap(), Some(file));
                }
                generation if generation == pre_generation + 1 => {
                    post_states += 1;
                    assert_eq!(recovered.lookup_root("victim").unwrap(), None);
                }
                generation => panic!(
                    "{context}: recovered disallowed generation {generation}, expected {pre_generation} or {}",
                    pre_generation + 1
                ),
            }
        });
    }
    assert!(
        pre_states > 0,
        "matrix never recovered the pre-delete state"
    );
    assert!(
        post_states > 0,
        "matrix never recovered the post-delete state"
    );
}

#[test]
fn one_transaction_updates_a_multi_node_allocation_root_under_pressure() {
    const REGIONS: u64 = 160;
    const REGION_BLOCKS: u32 = 16;
    let dev = formatted(REGIONS * REGION_BLOCKS as u64, REGION_BLOCKS);
    let mut vol = mount(dev).unwrap();
    vol.set_reclaim_batch_blocks(1);
    let file = vol.create_file_in_root("wide", b"", ts(1)).unwrap();
    // Leave enough ordinary space for the extent tree and commit metadata,
    // but force one transaction to cross the 144-record AFST leaf boundary.
    let normally_available = vol.available_blocks();
    vol.preallocate_file(file, 0, (normally_available - 12) * BS as u64, ts(2))
        .unwrap();
    let stats = vol.last_commit_stats().unwrap();
    assert!(
        stats.allocation_records_updated >= 144,
        "only {} region records changed",
        stats.allocation_records_updated
    );
    assert_eq!(
        stats.allocation_records_updated,
        stats.region_descriptors_written
    );
    assert!(stats.allocation_tree_nodes_written > 1);
    assert!(stats.alloc.allocator_ram_bytes < 64 * 1024);
    assert!(vol.free_blocks() >= vol.emergency_headroom_blocks());

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut remounted = mount(dev).unwrap();
    assert_eq!(remounted.stat(file).unwrap().unwrap().size_bytes, 0);
}

#[test]
#[ignore = "full-size Q3 allocation qualification; run in release with --nocapture"]
fn q3_allocation_qualification() {
    let dev = formatted(u64::from(MAX_REGION_BLOCKS), MAX_REGION_BLOCKS);
    let mut vol = mount(TraceBackend::new(dev)).unwrap();
    vol.set_reclaim_batch_blocks(1);
    let file = vol
        .create_file_in_root("one-gib-region", b"", ts(1))
        .unwrap();
    let initial_free = vol.free_blocks();
    let emergency_headroom = vol.emergency_headroom_blocks();
    let initial_available = vol.available_blocks();
    let reserved_blocks = initial_available - 24;
    vol.device_mut().reset();
    let started = Instant::now();
    vol.preallocate_file(file, 0, reserved_blocks * BS as u64, ts(2))
        .unwrap();
    let elapsed_ns = started.elapsed().as_nanos();
    let commit = vol.last_commit_stats().unwrap();
    let io = vol.device_mut().stats();
    let final_free = vol.free_blocks();
    let final_available = vol.available_blocks();
    let metadata_bytes_written = commit.metadata_blocks_written * BS as u64;
    let total_bytes_per_reserved_block_ppm =
        io.bytes_written.saturating_mul(1_000_000) / reserved_blocks;
    let metadata_bytes_per_reserved_block_ppm =
        metadata_bytes_written.saturating_mul(1_000_000) / reserved_blocks;

    println!(
        "{{\"schema_version\":1,\"workload\":\"one-gib-region-near-full\",\"backend\":\"memory\",\"block_size\":{BS},\"total_blocks\":{},\"region_size\":{},\"emergency_headroom_blocks\":{emergency_headroom},\"initial_free_blocks\":{initial_free},\"initial_available_blocks\":{initial_available},\"logical_blocks_reserved\":{reserved_blocks},\"final_free_blocks\":{final_free},\"final_available_blocks\":{final_available},\"elapsed_ns\":{elapsed_ns},\"reads\":{},\"writes\":{},\"bytes_read\":{},\"bytes_written\":{},\"flushes\":{},\"metadata_blocks_written\":{},\"metadata_bytes_written\":{metadata_bytes_written},\"bitmap_pages_written\":{},\"region_descriptors_written\":{},\"allocation_records_updated\":{},\"allocation_tree_nodes_written\":{},\"allocator_peak_bytes\":{},\"total_bytes_per_reserved_block_ppm\":{total_bytes_per_reserved_block_ppm},\"metadata_bytes_per_reserved_block_ppm\":{metadata_bytes_per_reserved_block_ppm}}}",
        MAX_REGION_BLOCKS,
        MAX_REGION_BLOCKS,
        io.reads,
        io.writes,
        io.bytes_read,
        io.bytes_written,
        io.flushes,
        commit.metadata_blocks_written,
        commit.bitmap_pages_written,
        commit.region_descriptors_written,
        commit.allocation_records_updated,
        commit.allocation_tree_nodes_written,
        commit.alloc.allocator_ram_bytes,
    );
    assert!(final_free >= emergency_headroom);
    assert!(final_available <= 24);
    assert_eq!(commit.region_descriptors_written, 1);
    assert_eq!(commit.allocation_records_updated, 1);
    assert!(commit.bitmap_pages_written <= 9);
    assert!(commit.alloc.allocator_ram_bytes <= 32 * 1024);

    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
#[ignore = "Q3 development probe for the low-space metadata floor"]
fn q3_low_space_headroom_probe() {
    for (label, total_blocks, region_size, headrooms) in [
        (
            "single-region",
            512,
            512,
            &[24u64, 16, 12, 8, 6, 5, 4, 3, 2, 1, 0][..],
        ),
        (
            "multi-node-allocation-root",
            145 * 16,
            16,
            &[
                160u64, 128, 112, 96, 80, 64, 48, 32, 24, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1, 0,
            ][..],
        ),
    ] {
        for &headroom in headrooms {
            let dev = formatted(total_blocks, region_size);
            let mut vol = mount(dev).unwrap();
            vol.set_reclaim_batch_blocks(1);
            let file = vol.create_file_in_root("reserve", b"", ts(1)).unwrap();
            let before = vol.free_blocks();
            let emergency_headroom = vol.emergency_headroom_blocks();
            let result = vol.preallocate_file(file, 0, (before - headroom) * BS as u64, ts(2));
            let after = vol.free_blocks();
            let available_after = vol.available_blocks();
            if result.is_ok() {
                assert!(after >= emergency_headroom);
            }
            let delete_result = if result.is_ok() {
                vol.delete_file_in_root("reserve", ts(3))
            } else {
                Err(CoreError::NoSpace)
            };
            let deleted = delete_result.is_ok();
            let delete_stats = deleted.then(|| vol.last_commit_stats().unwrap());
            println!(
                "{{\"schema_version\":1,\"workload\":\"low-space-delete-progress\",\"geometry\":\"{label}\",\"total_blocks\":{total_blocks},\"region_size\":{region_size},\"emergency_headroom_blocks\":{emergency_headroom},\"requested_headroom\":{headroom},\"fill_ok\":{},\"raw_free_after_fill\":{after},\"available_after_fill\":{available_after},\"delete_ok\":{deleted},\"delete_blocks_allocated\":{},\"delete_blocks_retired\":{},\"delete_metadata_blocks_written\":{},\"delete_reclaim_structure_blocks\":{}}}",
                result.is_ok(),
                delete_stats.map_or(0, |stats| stats.alloc.blocks_allocated),
                delete_stats.map_or(0, |stats| stats.alloc.blocks_retired),
                delete_stats.map_or(0, |stats| stats.metadata_blocks_written),
                delete_stats.map_or(0, |stats| stats.alloc.reclaim.structure_blocks_written),
            );
        }
    }
}

#[test]
fn repeated_near_full_cow_and_reclaim_preserve_shared_survivors() {
    for pages in [2, 4, 8, usize::MAX] {
        repeated_near_full_cow_and_reclaim_preserve_shared_survivors_profile(pages);
    }
}

fn repeated_near_full_cow_and_reclaim_preserve_shared_survivors_profile(pages: usize) {
    let mut evidence = SpillEvidence::default();
    use afsplus_format::OBJECT_ROOT;
    let mut vol = profile_mount(TraceBackend::new(formatted(512, 512)), pages);
    vol.set_reclaim_batch_blocks(1);
    let original: Vec<u8> = (0..4 * BS).map(|i| (i % 251) as u8).collect();
    let keeper = vol.create_file_in_root("keeper", &original, ts(1)).unwrap();
    evidence.observe(&vol, pages);
    let writer = vol
        .clone_file(keeper, OBJECT_ROOT, "writer", ts(2))
        .unwrap();
    evidence.observe(&vol, pages);
    let mut expected = original.clone();
    for cycle in 0..24 {
        let now = ts(10 + cycle);
        let pressure = vol.create_file_in_root("pressure", b"", now).unwrap();
        evidence.observe(&vol, pages);
        let reserve = vol.available_blocks().saturating_sub(24);
        assert!(reserve > 0, "cycle={cycle}: failed to restore capacity");
        vol.preallocate_file(pressure, 0, reserve * BS as u64, now)
            .unwrap();
        evidence.observe(&vol, pages);
        let generation = vol.generation();
        let free = vol.free_blocks();
        let too_large = (free + 1) * BS as u64;
        vol.device_mut().reset();
        assert!(matches!(
            vol.preallocate_file(pressure, reserve * BS as u64, too_large, now),
            Err(CoreError::NoSpace)
        ));
        let refusal_io = vol.device_mut().stats();
        assert_eq!(refusal_io.writes, 0, "pages={pages} cycle={cycle}");
        assert_eq!(refusal_io.flushes, 0, "pages={pages} cycle={cycle}");
        assert_eq!(vol.generation(), generation);
        assert_eq!(vol.free_blocks(), free);
        assert_eq!(vol.stat(pressure).unwrap().unwrap().size_bytes, 0);
        // Write across a block boundary while almost full. Whether admitted
        // or rejected, both owners must retain exactly their promised bytes.
        let offset = (cycle as usize % 3 + 1) * BS - 7;
        let data = vec![cycle as u8; 29];
        match vol.write_file_at(writer, offset as u64, &data, now) {
            Ok(()) => expected[offset..offset + data.len()].copy_from_slice(&data),
            Err(CoreError::NoSpace) => assert_eq!(vol.generation(), generation),
            Err(error) => panic!("cycle={cycle}: {error}"),
        }
        evidence.observe(&vol, pages);
        assert_eq!(vol.read_file(keeper).unwrap(), original, "cycle={cycle}");
        assert_eq!(vol.read_file(writer).unwrap(), expected, "cycle={cycle}");
        vol.delete_file_in_root("pressure", now).unwrap();
        evidence.observe(&vol, pages);
        vol.set_reclaim_batch_blocks(64);
        let mut converged = false;
        for _ in 0..128 {
            let reclaimed = vol.reclaim_step(now).unwrap();
            evidence.observe(&vol, pages);
            if reclaimed == 0 {
                converged = true;
                break;
            }
        }
        assert!(converged, "cycle={cycle}: reclaim did not converge");
        // Retry the same write after bounded reclamation, even when the
        // pressured attempt succeeded; exact survivor bytes remain required.
        vol.write_file_at(writer, offset as u64, &data, now)
            .unwrap();
        expected[offset..offset + data.len()].copy_from_slice(&data);
        evidence.observe(&vol, pages);
        assert_eq!(vol.lookup_root("pressure").unwrap(), None);
        assert_eq!(vol.read_file(keeper).unwrap(), original);
        assert_eq!(vol.read_file(writer).unwrap(), expected);
        let mut image = vol.into_device().into_inner();
        assert_checker_clean(&mut image, &format!("pages={pages} cycle={cycle}"));
        vol = profile_mount(TraceBackend::new(image), pages);
        vol.set_reclaim_batch_blocks(1);
        assert_eq!(vol.read_file(keeper).unwrap(), original);
        assert_eq!(vol.read_file(writer).unwrap(), expected);
    }
    eprintln!(
        "low-space cycles pages={pages} cycles=24 spills={} peak_staged={}",
        evidence.spills, evidence.peak
    );
}
