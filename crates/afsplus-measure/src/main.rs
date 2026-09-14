//! Single-threaded, memory-only host qualification. No filesystem paths accepted.
mod cache_workload;
mod heap;
mod resident;

use afsplus_block::{trace::IoStats, BlockDevice, BlockError};
use afsplus_check::check_device;
use afsplus_core::allocation_trace::{self as allocation_trace, Domain};
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};
use std::cell::Cell;
use std::time::Instant;

#[cfg(not(feature = "allocation-domains"))]
#[global_allocator]
static HEAP: heap::Meter = heap::Meter::new();
#[cfg(feature = "allocation-domains")]
#[global_allocator]
static HEAP: heap::TaggedMeter = heap::TaggedMeter::new();
const BS: usize = 4096;
const BLOCKS: u64 = 4096;

/// Fixed fixture allocation: device reads/writes allocate no Rust memory.
/// Counters hold no operation log and count only successfully completed I/O.
struct Image<'a> {
    bytes: Vec<u8>,
    io: &'a Cell<IoStats>,
}
impl Image<'_> {
    fn offset(&self, lba: u64, length: usize) -> Result<usize, BlockError> {
        if length != BS {
            return Err(BlockError::WrongBufferSize {
                expected: BS,
                actual: length,
            });
        }
        if lba >= BLOCKS {
            return Err(BlockError::OutOfBounds {
                lba,
                total_blocks: BLOCKS,
            });
        }
        Ok(lba as usize * BS)
    }
}
impl BlockDevice for Image<'_> {
    fn block_size(&self) -> usize {
        BS
    }
    fn total_blocks(&self) -> u64 {
        BLOCKS
    }
    fn read_block(&mut self, lba: u64, output: &mut [u8]) -> Result<(), BlockError> {
        let offset = self.offset(lba, output.len())?;
        output.copy_from_slice(&self.bytes[offset..offset + BS]);
        let mut io = self.io.get();
        io.reads += 1;
        io.bytes_read += BS as u64;
        self.io.set(io);
        Ok(())
    }
    fn write_block(&mut self, lba: u64, input: &[u8]) -> Result<(), BlockError> {
        let offset = self.offset(lba, input.len())?;
        self.bytes[offset..offset + BS].copy_from_slice(input);
        let mut io = self.io.get();
        io.writes += 1;
        io.bytes_written += BS as u64;
        self.io.set(io);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        let mut io = self.io.get();
        io.flushes += 1;
        self.io.set(io);
        Ok(())
    }
}

struct Row {
    name: &'static str,
    #[cfg(feature = "allocation-domains")]
    origins_before: heap::Details,
    #[cfg(feature = "allocation-domains")]
    origins_after: heap::Details,
    resident_start: Option<resident::Snapshot>,
    resident_end: Option<resident::Snapshot>,
    before: heap::Sample,
    after: heap::Sample,
    io: IoStats,
    wall_ns: u128,
    payload_written: u64,
    payload_read: u64,
}

fn phase<T>(name: &'static str, io: &Cell<IoStats>, work: impl FnOnce() -> T) -> (Row, T) {
    let resident_start = resident::snapshot();
    io.set(IoStats::default());
    let start = Instant::now();
    let before = HEAP.begin();
    #[cfg(feature = "allocation-domains")]
    let origins_before = HEAP.details();
    let result = work();
    let after = HEAP.sample();
    #[cfg(feature = "allocation-domains")]
    let origins_after = HEAP.details();
    let wall_ns = start.elapsed().as_nanos();
    let resident_end = resident::snapshot();
    (
        Row {
            name,
            #[cfg(feature = "allocation-domains")]
            origins_before,
            #[cfg(feature = "allocation-domains")]
            origins_after,
            resident_start,
            resident_end,
            before,
            after,
            io: io.get(),
            wall_ns,
            payload_written: 0,
            payload_read: 0,
        },
        result,
    )
}

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn main() {
    #[cfg(feature = "allocation-domains")]
    afsplus_core::allocation_trace::enable();
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let mut pages = None;
    let mut rounds = None;
    let mut valid = true;
    for pair in arguments.chunks(2) {
        if pair.len() != 2 {
            valid = false;
            break;
        }
        match pair[0].to_str() {
            Some("--cache-profile") if pages.is_none() => {
                pages = match pair[1].to_str() {
                    Some("2") => Some(2),
                    Some("4") => Some(4),
                    Some("8") => Some(8),
                    Some("unlimited") => Some(usize::MAX),
                    _ => {
                        valid = false;
                        None
                    }
                };
            }
            Some("--resident-rounds") if rounds.is_none() => {
                rounds = pair[1].to_str().and_then(|s| {
                    if s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    s.parse::<usize>().ok().filter(|n| (3..=32).contains(n))
                });
                if rounds.is_none() {
                    valid = false;
                }
            }
            _ => valid = false,
        }
    }
    if !valid {
        eprintln!("usage: afsplus-measure (no arguments), or [--cache-profile 2|4|8|unlimited] [--resident-rounds 3..32]");
        std::process::exit(1);
    }
    if let Some(count) = rounds {
        resident::configure(count).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
    }
    if let Some(pages) = pages {
        cache_workload::run(pages);
        return;
    }
    let io = Cell::new(IoStats::default());
    let mut rows = allocation_trace::within(Domain::Reporting, || {
        Vec::with_capacity(14 + resident::rounds())
    });
    let mut dev = allocation_trace::within(Domain::Fixture, || Image {
        bytes: vec![0; BLOCKS as usize * BS],
        io: &io,
    });
    // The image and result-row capacity are allocated before the first phase.
    let (row, ()) = phase("format", &io, || {
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [0x4d; 16],
                label: "Measure".into(),
                region_size: 256,
                reclaim_caps: Default::default(),
                log_slots: 8,
                shared_extents: true,
                data_policy: false,
                name_policy: NamePolicy::Sensitive,
                timestamp: ts(0),
            },
        )
        .unwrap();
    });
    rows.push(row);
    let (row, mut volume) = phase("mount", &io, || mount(dev).unwrap());
    rows.push(row);
    let mut ids = [0; 16];
    let (mut row, ()) = phase("create", &io, || {
        for (i, id) in ids.iter_mut().enumerate() {
            *id = volume
                .create_file_in_root(&format!("file-{i:02}"), &[i as u8; 6000], ts(i as i64 + 1))
                .unwrap();
        }
    });
    row.payload_written = 16 * 6000;
    rows.push(row);
    let (mut row, ()) = phase("edit", &io, || {
        for (i, &id) in ids.iter().enumerate() {
            if i % 2 == 0 {
                volume
                    .write_file_at(id, 4090, &[97; 32], ts(32 + i as i64))
                    .unwrap();
            } else {
                volume.truncate_file(id, 1024, ts(32 + i as i64)).unwrap();
            }
        }
        volume
            .rename(OBJECT_ROOT, "file-00", OBJECT_ROOT, "moved", ts(64))
            .unwrap();
        volume.delete_file_in_root("file-15", ts(65)).unwrap();
    });
    row.payload_written = 8 * 32;
    rows.push(row);
    let bitmap_peak = volume.allocator_ram_bytes();
    let (row, ()) = phase("sync", &io, || volume.sync().unwrap());
    rows.push(row);
    let (row, mut dev) = phase("unmount", &io, || volume.into_device());
    rows.push(row);
    let (row, ()) = phase("raw-check", &io, || {
        let _scope = allocation_trace::enter(Domain::Verifier);
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "raw checker: {:?}", report.errors);
    });
    rows.push(row);
    let (row, mut volume) = phase("remount", &io, || mount(dev).unwrap());
    rows.push(row);
    let (mut row, ()) = phase("read-verify", &io, || {
        assert_eq!(volume.list_root().unwrap().len(), 15);
        assert!(volume.lookup_root("file-00").unwrap().is_none());
        assert!(volume.lookup_root("file-15").unwrap().is_none());
        for (i, &id) in ids[..15].iter().enumerate() {
            let name = if i == 0 {
                "moved".to_owned()
            } else {
                format!("file-{i:02}")
            };
            assert_eq!(volume.lookup_root(&name).unwrap(), Some(id));
            let data = volume.read_file(id).unwrap();
            let mut expected = [i as u8; 6000];
            if i % 2 == 0 {
                expected[4090..4122].fill(97);
                assert_eq!(data, expected);
            } else {
                assert_eq!(data, expected[..1024]);
            }
        }
    });
    row.payload_read = 8 * 6000 + 7 * 1024;
    rows.push(row);
    steady_reads(&mut volume, &io, &mut rows);
    let (row, mut dev) = phase("final-unmount", &io, || volume.into_device());
    rows.push(row);
    let (row, ()) = phase("recovered-check", &io, || {
        let _scope = allocation_trace::enter(Domain::Verifier);
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "recovered checker: {:?}", report.errors);
    });
    rows.push(row);
    report("small-files-v1", &dev, bitmap_peak, &rows, None);
}

/// Repeated read-only work after exact initial read verification. The expected
/// names and bytes have separate preparation/release phases, so oracle costs
/// remain visible without being repeated inside each read phase.
fn steady_reads(
    volume: &mut afsplus_core::Volume<Image<'_>>,
    io: &Cell<IoStats>,
    rows: &mut Vec<Row>,
) {
    if resident::rounds() == 0 {
        return;
    }
    let (mut prepare, expected) = phase("steady-prepare", io, || {
        let _scope = allocation_trace::enter(Domain::Oracle);
        volume
            .list_root()
            .unwrap()
            .into_iter()
            .map(|(name, id)| {
                let bytes = volume.read_file(id).unwrap();
                (name, id, bytes)
            })
            .collect::<Vec<_>>()
    });
    prepare.payload_read = expected
        .iter()
        .map(|(_, _, bytes)| bytes.len() as u64)
        .sum();
    rows.push(prepare);
    let generation = volume.generation();
    for _ in 0..resident::rounds() {
        let (mut row, bytes) = phase("steady-read", io, || {
            let names = volume.list_root().unwrap();
            assert_eq!(names.len(), expected.len());
            let mut bytes = 0;
            for ((name, id), (expected_name, expected_id, expected_bytes)) in
                names.iter().zip(&expected)
            {
                assert_eq!((name, id), (expected_name, expected_id));
                assert_eq!(&volume.read_file(*id).unwrap(), expected_bytes);
                bytes += expected_bytes.len() as u64;
            }
            bytes
        });
        assert_eq!(volume.generation(), generation);
        assert_eq!((row.io.writes, row.io.flushes), (0, 0));
        row.payload_read = bytes;
        rows.push(row);
    }
    let (release, ()) = phase("steady-release", io, || drop(expected));
    rows.push(release);
}

fn report(
    workload: &str,
    dev: &Image<'_>,
    bitmap_peak: usize,
    rows: &[Row],
    cache: Option<(usize, [afsplus_core::volume::CommitStats; 2])>,
) {
    let image_crc = afsplus_format::crc32c::crc32c(&dev.bytes);
    // Reporting occurs after all samples so JSON formatting cannot inflate a phase.
    let version = if cfg!(feature = "allocation-domains") {
        3
    } else if resident::rounds() == 0 {
        1
    } else {
        2
    };
    println!("{{\"version\":{version},\"workload\":\"{workload}\",\"outcome\":\"pass\",\"backend\":\"fixed-memory\",\"image_bytes\":{},\"image_crc32c\":{},\"last_edit_bitmap_payload_peak_bytes\":{},", dev.bytes.len(), image_crc, bitmap_peak);
    #[cfg(feature = "allocation-domains")]
    println!("\"allocation_profile\":\"requested-origins-v1\",");
    if resident::rounds() != 0 {
        let samples: Vec<_> = rows
            .iter()
            .filter(|r| r.name == "steady-read")
            .map(|r| r.resident_end.unwrap().bytes)
            .collect();
        println!("\"resident_provider\":\"ps-rss-kib-v1\",\"resident_scope\":\"whole-process phase boundaries\",\"resident_platform\":\"{}\",\"steady_read\":{{\"rounds\":{},\"end_min_bytes\":{},\"end_max_bytes\":{},\"end_first_bytes\":{},\"end_last_bytes\":{},\"plateau_verified\":false}},", std::env::consts::OS,
            samples.len(), samples.iter().min().unwrap(), samples.iter().max().unwrap(), samples[0], samples.last().unwrap());
    }
    if let Some((pages, stats)) = cache {
        let pages = if pages == usize::MAX {
            "\"unlimited\"".to_owned()
        } else {
            pages.to_string()
        };
        println!("\"cache_pages\":{pages},\"tree_phases\":[");
        for (i, stats) in stats.iter().enumerate() {
            if i > 0 {
                println!(",");
            }
            let tree = stats.tree_mutations;
            println!("{{\"phase\":\"{}\",\"spill_writes\":{},\"spill_reloads\":{},\"staged_peak_pages\":{},\"staged_before_eviction_peak_pages\":{},\"decoded_peak_nodes\":{},\"metadata_writes\":{}}}",
                if i == 0 { "batch-create" } else { "batch-delete" }, tree.staged_spill_writes,
                tree.staged_spill_reloads, tree.max_resident_staged_nodes, tree.max_staged_nodes_before_eviction, tree.max_live_decoded_nodes, stats.metadata_blocks_written);
        }
        println!("],");
    }
    println!("\"phases\":[");
    for (i, row) in rows.iter().enumerate() {
        if i != 0 {
            println!(",");
        }
        let rss = match (row.resident_start, row.resident_end) {
            (Some(start), Some(end)) => {
                format!(",\"resident_start_bytes\":{},\"resident_end_bytes\":{},\"resident_start_probe_wall_ns\":{},\"resident_end_probe_wall_ns\":{}", start.bytes, end.bytes, start.probe_wall_ns, end.probe_wall_ns)
            }
            _ => String::new(),
        };
        #[cfg(feature = "allocation-domains")]
        let rss = rss + &allocation_json(row.origins_before, row.origins_after);
        println!("{{\"name\":\"{}\",\"wall_ns\":{},\"heap_start_bytes\":{},\"heap_end_bytes\":{},\"heap_peak_bytes\":{},\"heap_peak_above_start_bytes\":{},\"heap_acquired_bytes\":{},\"heap_released_bytes\":{},\"reads\":{},\"writes\":{},\"bytes_read\":{},\"bytes_written\":{},\"flushes\":{},\"logical_payload_written_bytes\":{},\"logical_payload_read_bytes\":{}{rss}}}",
            row.name, row.wall_ns, row.before.live, row.after.live, row.after.peak,
            row.after.peak - row.before.live,
            row.after.acquired.wrapping_sub(row.before.acquired), row.after.released.wrapping_sub(row.before.released),
            row.io.reads, row.io.writes, row.io.bytes_read, row.io.bytes_written, row.io.flushes,
            row.payload_written, row.payload_read);
    }
    println!("]}}");
}

#[cfg(feature = "allocation-domains")]
fn allocation_json(before: heap::Details, after: heap::Details) -> String {
    use std::fmt::Write;
    fn sample(before: heap::Sample, after: heap::Sample) -> String {
        format!("{{\"start_bytes\":{},\"end_bytes\":{},\"peak_bytes\":{},\"acquired_bytes\":{},\"released_bytes\":{}}}",
            before.live,after.live,after.peak,after.acquired.wrapping_sub(before.acquired),after.released.wrapping_sub(before.released))
    }
    let mut result = String::from(",\"allocation_origins\":{");
    for (i, name) in allocation_trace::NAMES.iter().enumerate() {
        if i != 0 {
            result.push(',');
        }
        write!(
            &mut result,
            "\"{name}\":{}",
            sample(before.origins[i], after.origins[i])
        )
        .unwrap();
    }
    write!(
        &mut result,
        "}},\"tracking_overhead\":{},\"underlying_requests\":{}",
        sample(before.overhead, after.overhead),
        sample(before.system, after.system)
    )
    .unwrap();
    result
}
