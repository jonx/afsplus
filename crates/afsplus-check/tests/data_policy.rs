//! Q1 data-update policy qualification.
//!
//! Full COW remains the mount default. The runtime-only in-place prototype is
//! deliberately restricted to non-extending writes over proven-private,
//! already-written extents. These tests pin both the eligibility boundary and
//! its weaker crash contract: an older checkpoint can observe old, new, or a
//! torn mixture of the overwritten user-data bytes, while metadata remains
//! structurally valid.

use std::time::Instant;

use afsplus_block::{
    for_each_crash_state, IoStats, MemoryBackend, RecordedOp, RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::volume::{CommitStats, DataUpdatePolicy};
use afsplus_core::{extent_map, Volume};
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 512);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0xD1; 16],
            label: "DataPolicy".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

#[test]
fn full_cow_is_the_mount_default() {
    let mut vol = mount(formatted()).unwrap();
    assert_eq!(vol.data_update_policy(), DataUpdatePolicy::FullCow);
    let file = vol
        .create_file_in_root("private.bin", &vec![0x11; BS], ts(1))
        .unwrap();
    vol.write_file_at(file, 100, &vec![0x22; 500], ts(2))
        .unwrap();
    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.data_blocks_written, 1);
    assert_eq!(stats.data_blocks_overwritten_in_place, 0);
}

#[test]
fn private_non_extending_write_reuses_the_physical_block() {
    let mut vol = mount(formatted()).unwrap();
    let mut expected = vec![0x11; BS];
    let file = vol
        .create_file_in_root("private.bin", &expected, ts(1))
        .unwrap();
    let physical = vol.stat(file).unwrap().unwrap().data_root;
    vol.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
    expected[100..600].fill(0x22);
    vol.write_file_at(file, 100, &vec![0x22; 500], ts(2))
        .unwrap();

    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.data_blocks_written, 1);
    assert_eq!(stats.data_blocks_overwritten_in_place, 1);
    assert_eq!(vol.stat(file).unwrap().unwrap().data_root, physical);
    assert_eq!(vol.read_file(file).unwrap(), expected);

    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
    let remounted = mount(dev).unwrap();
    assert_eq!(
        remounted.data_update_policy(),
        DataUpdatePolicy::FullCow,
        "the experimental policy is runtime-only"
    );
}

#[test]
fn extending_and_shared_writes_fall_back_to_full_cow() {
    let mut vol = mount(formatted()).unwrap();
    let source_bytes = vec![0x31; BS];
    let source = vol
        .create_file_in_root("source.bin", &source_bytes, ts(1))
        .unwrap();
    vol.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);

    vol.write_file_at(source, BS as u64, b"extension", ts(2))
        .unwrap();
    assert_eq!(
        vol.last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        0
    );

    let clone = vol
        .clone_file(source, OBJECT_ROOT, "clone.bin", ts(3))
        .unwrap();
    let source_before = vol.read_file(source).unwrap();
    vol.write_file_at(clone, 0, b"private clone", ts(4))
        .unwrap();
    assert_eq!(
        vol.last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        0,
        "a shared marker must force full COW"
    );
    assert_eq!(vol.read_file(source).unwrap(), source_before);
    assert_ne!(vol.read_file(clone).unwrap(), source_before);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn in_place_crash_matrix_keeps_metadata_clean_but_allows_torn_old_data() {
    let mut vol = mount(formatted()).unwrap();
    let before = vec![0x11; BS];
    let file = vol
        .create_file_in_root("database.page", &before, ts(1))
        .unwrap();
    let physical = vol.stat(file).unwrap().unwrap().data_root;
    let pre_generation = vol.generation();
    let base = vol.into_device();

    let offset = 100usize;
    let length = 3000usize;
    let mut after = before.clone();
    after[offset..offset + length].fill(0xEE);

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
    vol.write_file_at(file, offset as u64, &vec![0xEE; length], ts(2))
        .unwrap();
    assert_eq!(
        vol.last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place,
        1
    );
    let (_, operations) = vol.into_device().into_parts();
    assert!(matches!(
        operations.first(),
        Some(RecordedOp::Write { lba, .. }) if *lba == physical
    ));

    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    let mut mixed_pre_outcomes = 0u64;
    for crash_point in 0..=operations.len() {
        for_each_crash_state(&base, &operations, crash_point, |state| {
            let context = state.description;
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(
                report.is_clean(),
                "{context}: checker findings {:?}",
                report.errors
            );
            let mut recovered = mount(image).unwrap();
            assert!(
                recovered.generation() == pre_generation
                    || recovered.generation() == pre_generation + 1,
                "{context}: unexpected generation {}",
                recovered.generation()
            );
            let bytes = recovered.read_file(file).unwrap();
            if recovered.generation() == pre_generation + 1 {
                post_outcomes += 1;
                assert_eq!(bytes, after, "{context}");
                return;
            }

            pre_outcomes += 1;
            assert_eq!(&bytes[..offset], &before[..offset], "{context}");
            assert_eq!(
                &bytes[offset + length..],
                &before[offset + length..],
                "{context}"
            );
            assert!(
                bytes[offset..offset + length]
                    .iter()
                    .all(|byte| *byte == 0x11 || *byte == 0xEE),
                "{context}: bytes outside the old/new alphabet"
            );
            if bytes != before && bytes != after {
                mixed_pre_outcomes += 1;
            }
        });
    }
    assert!(pre_outcomes > 0);
    assert!(post_outcomes > 0);
    assert!(
        mixed_pre_outcomes > 0,
        "the matrix did not exercise the documented torn-data outcome"
    );
}

#[derive(Default)]
struct PolicyTotals {
    data_blocks: u64,
    in_place_blocks: u64,
    allocated_blocks: u64,
    retired_blocks: u64,
    peak_allocator_ram: u64,
}

impl PolicyTotals {
    fn absorb(&mut self, stats: CommitStats) {
        self.data_blocks += stats.data_blocks_written;
        self.in_place_blocks += stats.data_blocks_overwritten_in_place;
        self.allocated_blocks += stats.alloc.blocks_allocated;
        self.retired_blocks += stats.alloc.blocks_retired;
        self.peak_allocator_ram = self.peak_allocator_ram.max(stats.alloc.allocator_ram_bytes);
    }
}

struct PolicyRow {
    workload: &'static str,
    policy: DataUpdatePolicy,
    operations: u64,
    totals: PolicyTotals,
    io: IoStats,
    extents: usize,
    millis: f64,
}

fn benchmark_volume(total_blocks: u64) -> Volume<TraceBackend<MemoryBackend>> {
    let mut dev = MemoryBackend::new(BS, total_blocks);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0xD2; 16],
            label: "DataPolicyBench".into(),
            region_size: 16_384,
            reclaim_caps: Default::default(),
            log_slots: 64,
            shared_extents: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    mount(TraceBackend::new(dev)).unwrap()
}

fn extent_count(vol: &mut Volume<TraceBackend<MemoryBackend>>, object_id: u64) -> usize {
    let record = vol.stat(object_id).unwrap().unwrap();
    if record.flags & afsplus_format::object::OBJECT_FLAG_EXTENT_TREE == 0 {
        return usize::from(record.data_blocks != 0);
    }
    let geometry = vol.ident().geometry();
    let generation = vol.generation();
    extent_map::load_all(
        vol.device_mut(),
        &geometry,
        record.data_root,
        object_id,
        generation,
    )
    .unwrap()
    .extents
    .len()
}

fn random_rewrite_row(
    workload: &'static str,
    policy: DataUpdatePolicy,
    operations: u64,
    file_blocks: u64,
) -> PolicyRow {
    let mut vol = benchmark_volume(131_072);
    let file = vol
        .create_file_in_root("random.bin", &vec![0x41; file_blocks as usize * BS], ts(1))
        .unwrap();
    vol.set_data_update_policy(policy);
    vol.device_mut().reset();
    let mut totals = PolicyTotals::default();
    let start = Instant::now();
    let mut random = 0x4D59_5DF4_D0F3_3173u64;
    for operation in 0..operations {
        random = random
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let logical = random % file_blocks;
        vol.write_file_at(
            file,
            logical * BS as u64,
            &vec![operation as u8; BS],
            ts(operation as i64 + 2),
        )
        .unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    let millis = start.elapsed().as_secs_f64() * 1000.0;
    let io = vol.device_mut().stats();
    let extents = extent_count(&mut vol, file);
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    PolicyRow {
        workload,
        policy,
        operations,
        totals,
        io,
        extents,
        millis,
    }
}

fn append_row(policy: DataUpdatePolicy, operations: u64) -> PolicyRow {
    let mut vol = benchmark_volume(131_072);
    let file = vol.create_file_in_root("append.log", b"", ts(1)).unwrap();
    vol.set_data_update_policy(policy);
    vol.device_mut().reset();
    let mut totals = PolicyTotals::default();
    let start = Instant::now();
    for operation in 0..operations {
        vol.write_file_at(
            file,
            operation * BS as u64,
            &vec![operation as u8; BS],
            ts(operation as i64 + 2),
        )
        .unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    let millis = start.elapsed().as_secs_f64() * 1000.0;
    let io = vol.device_mut().stats();
    let extents = extent_count(&mut vol, file);
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    PolicyRow {
        workload: "append-4K",
        policy,
        operations,
        totals,
        io,
        extents,
        millis,
    }
}

fn reflink_first_write_row(policy: DataUpdatePolicy, operations: u64) -> PolicyRow {
    let mut vol = benchmark_volume(131_072);
    let original = vec![0x61; operations as usize * BS];
    let source = vol
        .create_file_in_root("source.bin", &original, ts(1))
        .unwrap();
    let clone = vol
        .clone_file(source, OBJECT_ROOT, "clone.bin", ts(2))
        .unwrap();
    vol.set_data_update_policy(policy);
    vol.device_mut().reset();
    let mut totals = PolicyTotals::default();
    let start = Instant::now();
    for operation in 0..operations {
        vol.write_file_at(
            clone,
            operation * BS as u64,
            &vec![operation as u8; BS],
            ts(operation as i64 + 3),
        )
        .unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    let millis = start.elapsed().as_secs_f64() * 1000.0;
    let io = vol.device_mut().stats();
    let extents = extent_count(&mut vol, clone);
    assert_eq!(vol.read_file(source).unwrap(), original);
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    PolicyRow {
        workload: "reflink-first-write",
        policy,
        operations,
        totals,
        io,
        extents,
        millis,
    }
}

fn policy_rows(operations: u64, file_blocks: u64) -> Vec<PolicyRow> {
    let mut rows = Vec::new();
    for policy in [DataUpdatePolicy::FullCow, DataUpdatePolicy::InPlacePrivate] {
        rows.push(random_rewrite_row(
            "random-4K",
            policy,
            operations,
            file_blocks,
        ));
        rows.push(random_rewrite_row(
            "db-hotset-4K",
            policy,
            operations,
            file_blocks.min(32),
        ));
        rows.push(append_row(policy, operations));
        rows.push(reflink_first_write_row(policy, operations));
    }
    rows
}

fn print_policy_rows(rows: &[PolicyRow]) {
    println!(
        "\n{:<20} {:<15} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7} {:>8} {:>8}",
        "workload",
        "policy",
        "ops",
        "writes",
        "reads",
        "flushes",
        "inplace",
        "alloc",
        "retired",
        "extents",
        "RAM KiB",
        "ms"
    );
    for row in rows {
        println!(
            "{:<20} {:<15?} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7} {:>8.1} {:>8.1}",
            row.workload,
            row.policy,
            row.operations,
            row.io.writes,
            row.io.reads,
            row.io.flushes,
            row.totals.in_place_blocks,
            row.totals.allocated_blocks,
            row.totals.retired_blocks,
            row.extents,
            row.totals.peak_allocator_ram as f64 / 1024.0,
            row.millis,
        );
    }
}

#[test]
fn data_policy_workload_smoke() {
    let operations = 16;
    let rows = policy_rows(operations, 64);
    print_policy_rows(&rows);

    let find = |workload, policy| {
        rows.iter()
            .find(|row| row.workload == workload && row.policy == policy)
            .unwrap()
    };
    assert_eq!(
        find("random-4K", DataUpdatePolicy::FullCow)
            .totals
            .in_place_blocks,
        0
    );
    assert_eq!(
        find("random-4K", DataUpdatePolicy::InPlacePrivate)
            .totals
            .in_place_blocks,
        operations
    );
    assert!(
        find("random-4K", DataUpdatePolicy::InPlacePrivate)
            .totals
            .allocated_blocks
            < find("random-4K", DataUpdatePolicy::FullCow)
                .totals
                .allocated_blocks
    );
    assert_eq!(
        find("db-hotset-4K", DataUpdatePolicy::InPlacePrivate)
            .totals
            .in_place_blocks,
        operations
    );
    for policy in [DataUpdatePolicy::FullCow, DataUpdatePolicy::InPlacePrivate] {
        assert_eq!(find("append-4K", policy).totals.in_place_blocks, 0);
        assert_eq!(
            find("reflink-first-write", policy).totals.in_place_blocks,
            0
        );
    }
}

#[test]
#[ignore = "full-size Q1 data-policy qualification; run in release with --nocapture"]
fn data_policy_workload_qualification() {
    let rows = policy_rows(1_000, 4_096);
    print_policy_rows(&rows);
}
