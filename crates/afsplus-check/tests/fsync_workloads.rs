//! Git-style fsync workload measurements (architecture blocker 2).
//!
//! Every prototype transaction is a full checkpoint commit with its own
//! barriers, so these workloads measure exactly what a small durable
//! operation costs under checkpoint COW — the number an intent log would
//! have to beat. Three patterns dominate Git and package-manager behavior:
//!
//! - `ref update`: create a lock file, remove the old target, rename the
//!   lock over it (three transactions today; an atomic-replace rename would
//!   make it two);
//! - `checkout`: a burst of small-file creates;
//! - `log append`: repeated small appends to one file, each made durable.
//!
//! Run `cargo test -p afsplus-check --test fsync_workloads --release -- \
//! --ignored --nocapture` for the full-size qualification table. The
//! measured baseline and its analysis live in
//! `implementation/fsync-intent-log-baseline.md`.

use std::time::Instant;

use afsplus_block::{IoStats, MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::volume::CommitStats;
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::Timespec;

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec { seconds, nanoseconds: 0 }
}

fn fresh_volume(total_blocks: u64) -> Volume<TraceBackend<MemoryBackend>> {
    let mut dev = MemoryBackend::new(BS, total_blocks);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [55u8; 16],
            label: "FsyncBench".into(),
            region_size: 16_384,
            reclaim_caps: Default::default(),
            timestamp: ts(0),
        },
    )
    .unwrap();
    mount(TraceBackend::new(dev)).unwrap()
}

#[derive(Default)]
struct Totals {
    transactions: u64,
    commits: CommitStats,
    wall_micros: u128,
}

impl Totals {
    fn absorb(&mut self, s: CommitStats) {
        self.transactions += 1;
        self.commits.data_blocks_written += s.data_blocks_written;
        self.commits.metadata_blocks_written += s.metadata_blocks_written;
        self.commits.bitmap_pages_written += s.bitmap_pages_written;
        self.commits.region_descriptors_written += s.region_descriptors_written;
        self.commits.allocation_tree_nodes_written += s.allocation_tree_nodes_written;
        self.commits.checkpoint_blocks_written += s.checkpoint_blocks_written;
        self.commits.flushes += s.flushes;
        self.commits.bytes_written += s.bytes_written;
        self.commits.alloc.reclaim.structure_blocks_written +=
            s.alloc.reclaim.structure_blocks_written;
    }
}

struct WorkloadRow {
    name: &'static str,
    logical_ops: u64,
    totals: Totals,
    io: IoStats,
}

fn print_table(rows: &[WorkloadRow]) {
    println!(
        "\n{:<22} {:>6} {:>5} {:>8} {:>8} {:>7} {:>8} {:>9} {:>9} {:>9} {:>8}",
        "workload",
        "ops",
        "txs",
        "writes",
        "reads",
        "flushes",
        "MiB",
        "wr/op",
        "flush/op",
        "reads/op",
        "ms"
    );
    for row in rows {
        let ops = row.logical_ops as f64;
        println!(
            "{:<22} {:>6} {:>5} {:>8} {:>8} {:>7} {:>8.2} {:>9.1} {:>9.1} {:>9.1} {:>8.1}",
            row.name,
            row.logical_ops,
            row.totals.transactions,
            row.io.writes,
            row.io.reads,
            row.io.flushes,
            row.io.bytes_written as f64 / (1024.0 * 1024.0),
            row.io.writes as f64 / ops,
            row.io.flushes as f64 / ops,
            row.io.reads as f64 / ops,
            row.totals.wall_micros as f64 / 1000.0,
        );
    }
    println!("\nper-transaction write breakdown (averages):");
    println!(
        "{:<22} {:>6} {:>6} {:>6} {:>6} {:>7} {:>9} {:>6}",
        "workload", "data", "meta", "bmap", "desc", "alloc", "reclaim", "ckpt"
    );
    for row in rows {
        let txs = row.totals.transactions.max(1) as f64;
        let c = &row.totals.commits;
        println!(
            "{:<22} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>7.1} {:>9.1} {:>6.1}",
            row.name,
            c.data_blocks_written as f64 / txs,
            c.metadata_blocks_written as f64 / txs,
            c.bitmap_pages_written as f64 / txs,
            c.region_descriptors_written as f64 / txs,
            c.allocation_tree_nodes_written as f64 / txs,
            c.alloc.reclaim.structure_blocks_written as f64 / txs,
            c.checkpoint_blocks_written as f64 / txs,
        );
    }
    // The comparison an intent log has to justify itself against: one log
    // record write plus one barrier per durable op, with the full checkpoint
    // amortized over a configurable window.
    println!("\nintent-log envelope for contrast (1 log write + 1 flush per op,");
    println!("checkpoint amortized every 64 ops at current per-tx cost):");
    for row in rows {
        let ops = row.logical_ops as f64;
        let txs = row.totals.transactions.max(1) as f64;
        let ckpt_writes_per_tx = row.io.writes as f64 / txs;
        let est_writes = 1.0 + ckpt_writes_per_tx / 64.0;
        let est_flushes = 1.0 + 3.0 / 64.0;
        println!(
            "{:<22} est {est_writes:>5.2} wr/op {est_flushes:>5.2} flush/op vs measured {:>5.1} / {:>4.1}",
            row.name,
            row.io.writes as f64 / ops,
            row.io.flushes as f64 / ops,
        );
    }
}

/// One Git-style ref update: write a lock file durably, then publish it over
/// the previous ref. Without atomic-replace rename this is three
/// transactions; the delete is skipped on the first iteration.
fn ref_update(vol: &mut Volume<TraceBackend<MemoryBackend>>, totals: &mut Totals, i: i64) {
    let start = Instant::now();
    vol.create_file_in_root("HEAD.lock", format!("ref {i}\n").as_bytes(), ts(i)).unwrap();
    totals.absorb(vol.last_commit_stats().unwrap());
    if vol.lookup_root("HEAD").unwrap().is_some() {
        vol.delete_file_in_root("HEAD", ts(i)).unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    vol.rename(
        afsplus_format::OBJECT_ROOT,
        "HEAD.lock",
        afsplus_format::OBJECT_ROOT,
        "HEAD",
        ts(i),
    )
    .unwrap();
    totals.absorb(vol.last_commit_stats().unwrap());
    totals.wall_micros += start.elapsed().as_micros();
}

fn run_workloads(updates: u64, files: u64, appends: u64) -> Vec<WorkloadRow> {
    let mut rows = Vec::new();

    // --- ref updates ------------------------------------------------------
    let mut vol = fresh_volume(65_536);
    vol.device_mut().reset();
    let mut totals = Totals::default();
    for i in 0..updates {
        ref_update(&mut vol, &mut totals, i as i64);
    }
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow { name: "git ref update", logical_ops: updates, totals, io });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- checkout burst ---------------------------------------------------
    let mut vol = fresh_volume(65_536);
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..files {
        vol.create_file_in_root(&format!("obj-{i:06}"), &[i as u8; 900], ts(i as i64)).unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow { name: "checkout small files", logical_ops: files, totals, io });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- durable log appends ---------------------------------------------
    let mut vol = fresh_volume(65_536);
    let id = vol.create_file_in_root("log", b"", ts(0)).unwrap();
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..appends {
        vol.write_file_at(id, i * 200, &[i as u8; 200], ts(i as i64)).unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow { name: "durable log append", logical_ops: appends, totals, io });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    rows
}

#[test]
fn fsync_workload_smoke() {
    let rows = run_workloads(40, 120, 120);
    print_table(&rows);
    for row in &rows {
        let txs = row.totals.transactions.max(1) as f64;
        // Guard rails, not targets: silent cost regressions must fail here.
        assert!(row.io.writes as f64 / txs < 20.0, "{}: writes per tx exploded", row.name);
        assert!(row.totals.commits.flushes as f64 / txs <= 3.0, "{}", row.name);
        assert!(row.io.reads as f64 / txs < 40.0, "{}: reads per tx exploded", row.name);
    }
}

#[test]
#[ignore = "full-size fsync workload qualification; run in release with --nocapture"]
fn fsync_workload_qualification() {
    let rows = run_workloads(1_000, 4_000, 4_000);
    print_table(&rows);
}
