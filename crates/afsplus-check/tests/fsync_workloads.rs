//! Git-style fsync workload measurements (architecture blocker 2).
//!
//! The workloads compare full checkpoint commits, group commit and the
//! experimental intent log under the same traced block backend. Three
//! patterns dominate Git, package-manager and database behavior:
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
use afsplus_core::volume::{BatchOp, CommitStats};
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::Timespec;

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
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
            log_slots: 64,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
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
}

fn assert_existing_file_log_gate(rows: &[WorkloadRow]) {
    let row = |name| {
        rows.iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("missing workload row {name}"))
    };
    let logged_append = row("logged append(64)");
    let logged_db = row("logged db hotset(64)");
    let checkpoint_append = row("durable log append");
    let per_op = |value: u64, row: &WorkloadRow| value as f64 / row.logical_ops.max(1) as f64;

    assert!(
        per_op(logged_append.io.writes, logged_append)
            < per_op(checkpoint_append.io.writes, checkpoint_append),
        "logged append must beat checkpoint-per-fsync write amplification"
    );
    assert!(
        per_op(logged_append.io.reads, logged_append)
            < per_op(checkpoint_append.io.reads, checkpoint_append),
        "logged append must beat checkpoint-per-fsync read amplification"
    );
    for logged in [logged_append, logged_db] {
        assert!(
            per_op(logged.io.writes, logged) < 3.0,
            "{} exceeds the three-write data-log envelope",
            logged.name
        );
        assert!(
            per_op(logged.io.flushes, logged) < 2.1,
            "{} exceeds data + record barriers with bounded checkpoint amortization",
            logged.name
        );
        assert!(
            per_op(logged.io.reads, logged) < 3.0,
            "{} replay-window read amplification regressed",
            logged.name
        );
    }
}

/// One Git-style ref update: write a lock file durably, then publish it over
/// the previous ref. Without atomic-replace rename this is three
/// transactions; the delete is skipped on the first iteration.
fn ref_update(vol: &mut Volume<TraceBackend<MemoryBackend>>, totals: &mut Totals, i: i64) {
    let start = Instant::now();
    vol.create_file_in_root("HEAD.lock", format!("ref {i}\n").as_bytes(), ts(i))
        .unwrap();
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

    // --- ref updates through the intent log: fsync per update, checkpoint
    // every 64 updates (ADR-037) -----------------------------------------
    let mut vol = fresh_volume(65_536);
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..updates {
        let content = format!("ref {i}\n");
        vol.window_op(
            &BatchOp::CreateFile {
                parent_id: afsplus_format::OBJECT_ROOT,
                name: "HEAD.lock",
                content: content.as_bytes(),
            },
            ts(i as i64),
        )
        .unwrap();
        vol.window_op(
            &BatchOp::Rename {
                source_parent_id: afsplus_format::OBJECT_ROOT,
                source_name: "HEAD.lock",
                target_parent_id: afsplus_format::OBJECT_ROOT,
                target_name: "HEAD",
                replace: true,
            },
            ts(i as i64),
        )
        .unwrap();
        vol.window_fsync().unwrap();
        if (i + 1) % 64 == 0 {
            vol.window_commit(ts(i as i64)).unwrap();
            totals.absorb(vol.last_commit_stats().unwrap());
        }
    }
    if !updates.is_multiple_of(64) {
        vol.window_commit(ts(updates as i64)).unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "ref update logged(64)",
        logical_ops: updates,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- ref updates, group-committed: one 2-op batch per durable update --
    let mut vol = fresh_volume(65_536);
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..updates {
        let content = format!("ref {i}\n");
        vol.run_batch(
            &[
                BatchOp::CreateFile {
                    parent_id: afsplus_format::OBJECT_ROOT,
                    name: "HEAD.lock",
                    content: content.as_bytes(),
                },
                BatchOp::Rename {
                    source_parent_id: afsplus_format::OBJECT_ROOT,
                    source_name: "HEAD.lock",
                    target_parent_id: afsplus_format::OBJECT_ROOT,
                    target_name: "HEAD",
                    replace: true,
                },
            ],
            ts(i as i64),
        )
        .unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "ref update batched(2)",
        logical_ops: updates,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- checkout, group-committed in 64-file windows ---------------------
    let mut vol = fresh_volume(65_536);
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    let names: Vec<String> = (0..files).map(|i| format!("obj-{i:06}")).collect();
    let contents: Vec<Vec<u8>> = (0..files).map(|i| vec![i as u8; 900]).collect();
    for window in names.chunks(64).zip(contents.chunks(64)) {
        let ops: Vec<BatchOp<'_>> = window
            .0
            .iter()
            .zip(window.1)
            .map(|(name, content)| BatchOp::CreateFile {
                parent_id: afsplus_format::OBJECT_ROOT,
                name,
                content,
            })
            .collect();
        vol.run_batch(&ops, ts(0)).unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "checkout batched(64)",
        logical_ops: files,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- ref updates ------------------------------------------------------
    let mut vol = fresh_volume(65_536);
    vol.device_mut().reset();
    let mut totals = Totals::default();
    for i in 0..updates {
        ref_update(&mut vol, &mut totals, i as i64);
    }
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "git ref update",
        logical_ops: updates,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- checkout burst ---------------------------------------------------
    let mut vol = fresh_volume(65_536);
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..files {
        vol.create_file_in_root(&format!("obj-{i:06}"), &[i as u8; 900], ts(i as i64))
            .unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "checkout small files",
        logical_ops: files,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- durable appends through existing-file intent records ------------
    let mut vol = fresh_volume(65_536);
    let id = vol
        .create_file_in_root("logged-append", b"", ts(0))
        .unwrap();
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..appends {
        vol.window_write_file_at(id, i * 200, &[i as u8; 200], ts(i as i64))
            .unwrap();
        vol.window_fsync().unwrap();
        if (i + 1).is_multiple_of(64) {
            vol.window_commit(ts(i as i64)).unwrap();
            totals.absorb(vol.last_commit_stats().unwrap());
        }
    }
    if !appends.is_multiple_of(64) {
        vol.window_commit(ts(appends as i64)).unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "logged append(64)",
        logical_ops: appends,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- durable database hot-set writes through the intent log ----------
    let mut vol = fresh_volume(65_536);
    let id = vol
        .create_file_in_root("database", &vec![0u8; 32 * BS], ts(0))
        .unwrap();
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..appends {
        let page = (i * 17) % 32;
        vol.window_write_file_at(id, page * BS as u64, &vec![i as u8; BS], ts(i as i64))
            .unwrap();
        vol.window_fsync().unwrap();
        if (i + 1).is_multiple_of(64) {
            vol.window_commit(ts(i as i64)).unwrap();
            totals.absorb(vol.last_commit_stats().unwrap());
        }
    }
    if !appends.is_multiple_of(64) {
        vol.window_commit(ts(appends as i64)).unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "logged db hotset(64)",
        logical_ops: appends,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    // --- durable checkpointed log appends --------------------------------
    let mut vol = fresh_volume(65_536);
    let id = vol.create_file_in_root("log", b"", ts(0)).unwrap();
    vol.device_mut().reset();
    let mut totals = Totals::default();
    let start = Instant::now();
    for i in 0..appends {
        vol.write_file_at(id, i * 200, &[i as u8; 200], ts(i as i64))
            .unwrap();
        totals.absorb(vol.last_commit_stats().unwrap());
    }
    totals.wall_micros = start.elapsed().as_micros();
    let io = vol.device_mut().stats();
    rows.push(WorkloadRow {
        name: "durable log append",
        logical_ops: appends,
        totals,
        io,
    });
    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    rows
}

#[test]
fn fsync_workload_smoke() {
    let rows = run_workloads(40, 120, 120);
    print_table(&rows);
    assert_existing_file_log_gate(&rows);
    for row in &rows {
        let ops = row.logical_ops.max(1) as f64;
        let txs = row.totals.transactions.max(1) as f64;
        // Guard rails, not targets: silent cost regressions must fail here.
        assert!(
            row.io.writes as f64 / ops < 30.0,
            "{}: writes per op exploded",
            row.name
        );
        assert!(
            row.totals.commits.flushes as f64 / txs <= 3.0,
            "{}",
            row.name
        );
        assert!(
            row.io.reads as f64 / ops < 60.0,
            "{}: reads per op exploded",
            row.name
        );
    }
}

#[test]
#[ignore = "full-size fsync workload qualification; run in release with --nocapture"]
fn fsync_workload_qualification() {
    let rows = run_workloads(1_000, 4_000, 4_000);
    print_table(&rows);
    assert_existing_file_log_gate(&rows);
}
