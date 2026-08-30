//! Allocator/commit measurements demanded by the prototype plan: metadata
//! bytes per allocation, bitmap pages written, flush count, retired blocks,
//! reclaim latency, allocator RAM. Run with `--nocapture` to see the table.
//!
//! The numbers asserted here are prototype characteristics, not format
//! guarantees; the point is that silent regressions get caught and that the
//! delta-log/spacemap alternatives have a baseline to beat.

use afsplus_block::{FileBackend, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::volume::CommitStats;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::Timespec;

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

#[test]
fn per_transaction_resource_accounting() {
    // Four regions of 16 blocks: exercises reserved heads and multi-region
    // allocation, with a deliberately tiny allocator RAM footprint.
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [42u8; 16],
            label: "MeasureVol".into(),
            region_size: 16,
            reclaim_caps: Default::default(),
            log_slots: 8,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut vol = mount(dev).unwrap();

    let mut rows: Vec<(String, CommitStats)> = Vec::new();
    for i in 0..3 {
        vol.create_file_in_root(&format!("file-{i}"), &[i as u8; 6000], ts(i as i64 + 1))
            .unwrap();
        rows.push((
            format!("create file-{i} (2 data blocks)"),
            vol.last_commit_stats().unwrap(),
        ));
    }
    vol.delete_file_in_root("file-1", ts(10)).unwrap();
    rows.push(("delete file-1".into(), vol.last_commit_stats().unwrap()));
    vol.create_file_in_root("file-3", &[9u8; 6000], ts(11))
        .unwrap();
    rows.push((
        "create file-3 (reuses quarantine)".into(),
        vol.last_commit_stats().unwrap(),
    ));
    vol.reclaim_step(ts(12)).unwrap();
    rows.push(("reclaim step".into(), vol.last_commit_stats().unwrap()));

    println!(
        "\n{:<34} {:>4} {:>4} {:>4} {:>4} {:>6} {:>7} {:>8} {:>7} {:>5} {:>5} {:>7} {:>7}",
        "transaction",
        "data",
        "meta",
        "bmap",
        "desc",
        "flush",
        "retired",
        "promoted",
        "latency",
        "qapp",
        "qseal",
        "qpend",
        "bytes"
    );
    for (label, s) in &rows {
        println!(
            "{:<34} {:>4} {:>4} {:>4} {:>4} {:>6} {:>7} {:>8} {:>7} {:>5} {:>5} {:>7} {:>7}",
            label,
            s.data_blocks_written,
            s.metadata_blocks_written,
            s.bitmap_pages_written,
            s.region_descriptors_written,
            s.flushes,
            s.alloc.blocks_retired,
            s.alloc.blocks_promoted,
            s.alloc.reclaim_latency_generations,
            s.alloc.reclaim.entries_appended,
            s.alloc.reclaim.segments_sealed,
            s.alloc.reclaim.pending_blocks_after,
            s.bytes_written,
        );
    }
    println!(
        "allocator RAM: {} bytes for {} regions; pending reclaim: {}; free: {}\n",
        vol.allocator_ram_bytes(),
        vol.ident().geometry().region_count(),
        vol.reclaim_pending_blocks(),
        vol.free_blocks(),
    );

    for (label, s) in &rows {
        // Every transaction: exactly one checkpoint block, and the retired
        // list plus COW'd structures stay bounded. The authoritative
        // allocation-root COW adds one metadata node to the old baseline.
        assert_eq!(s.checkpoint_blocks_written, 1, "{label}");
        assert!(s.metadata_blocks_written <= 6, "{label}");
        // On this deliberately tiny 4-region geometry, the roving allocator
        // plus quarantine promotions may touch every region's single page;
        // the bound is the region count, not a fixed working set.
        assert!(
            s.bitmap_pages_written <= 4,
            "{label}: bitmap write amplification"
        );
        assert_eq!(
            s.region_descriptors_written, s.alloc.region_descriptors_dirty,
            "{label}: descriptor accounting drift"
        );
        assert_eq!(
            s.allocation_records_updated, s.region_descriptors_written,
            "{label}: allocation-root dirty record drift"
        );
        assert!(
            s.region_descriptors_written <= 4,
            "{label}: descriptor write amplification"
        );
        assert!(s.flushes <= 3, "{label}");
        // Reclaim latency: exactly one generation per promoted block.
        assert_eq!(
            s.alloc.reclaim_latency_generations, s.alloc.blocks_promoted,
            "{label}"
        );
    }
    // The last transaction (a reclaim step) touched three of the four
    // 2-byte region pages — promotion targets plus the rebuilt queue root,
    // shifted by the intent-log area — while still avoiding retention of
    // the full 8-byte bitmap.
    assert_eq!(vol.allocator_ram_bytes(), 3 * 2);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
#[ignore = "explicit 1 TiB sparse-image qualification"]
fn one_tib_sparse_image_formats_and_mounts_without_a_block_count_scan() {
    const TOTAL_BLOCKS: u64 = (1u64 << 40) / BS as u64;
    let dir =
        std::env::temp_dir().join(format!("afsplus-1tib-qualification-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("volume.img");
    let started = std::time::Instant::now();
    let mut dev = FileBackend::create(&path, BS, TOTAL_BLOCKS).unwrap();
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x1A; 16],
            label: "OneTiB".into(),
            region_size: afsplus_format::geometry::MAX_REGION_BLOCKS,
            reclaim_caps: Default::default(),
            log_slots: 8,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.ident().geometry().region_count(), 1_024);
    assert!(vol.list_root().unwrap().is_empty());
    let small_commit_started = std::time::Instant::now();
    vol.create_file_in_root("small-a", b"", ts(1)).unwrap();
    let first_commit = small_commit_started.elapsed();
    let first_stats = vol.last_commit_stats().unwrap();
    assert_eq!(first_stats.bitmap_pages_written, 1);
    assert_eq!(first_stats.region_descriptors_written, 1);
    assert_eq!(first_stats.allocation_records_updated, 1);
    assert_eq!(first_stats.alloc.allocation_records_loaded, 1);
    assert!(first_stats.allocation_tree_nodes_written <= 3);
    let second_commit_started = std::time::Instant::now();
    vol.create_file_in_root("small-b", b"", ts(2)).unwrap();
    let second_commit = second_commit_started.elapsed();
    let second_stats = vol.last_commit_stats().unwrap();
    assert_eq!(second_stats.allocation_records_updated, 1);
    assert_eq!(second_stats.alloc.allocation_records_loaded, 2);
    assert!(second_stats.allocation_tree_nodes_written <= 3);
    assert_eq!(vol.list_root().unwrap().len(), 2);
    drop(vol);

    let checker_started = std::time::Instant::now();
    let mut checker_dev = FileBackend::open(&path, BS, TOTAL_BLOCKS).unwrap();
    let report = check_device(&mut checker_dev);
    let checker_elapsed = checker_started.elapsed();
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    drop(checker_dev);

    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.len() > (1u64 << 40) - (2u64 << 30));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let physical_bytes = metadata.blocks() * 512;
        println!(
            "1 TiB sparse format+mount+2 commits: {:?}, commits {:?}/{:?}, checker {:?}, physical bytes: {}",
            started.elapsed(),
            first_commit,
            second_commit,
            checker_elapsed,
            physical_bytes
        );
        // APFS allocation around widely separated writes varied from roughly
        // 112 MiB to 2.13 GiB in consecutive runs. Enforce that the image is
        // still sparse by orders of magnitude without turning host allocation
        // policy into an AFS+ format contract.
        assert!(physical_bytes < (1u64 << 40) / 100);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
