//! Single-threaded, memory-only host qualification. No filesystem paths accepted.
mod heap;

use afsplus_block::{trace::IoStats, BlockDevice, BlockError};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};
use std::cell::Cell;
use std::time::Instant;

#[global_allocator]
static HEAP: heap::Meter = heap::Meter::new();
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
    before: heap::Sample,
    after: heap::Sample,
    io: IoStats,
    wall_ns: u128,
    payload_written: u64,
    payload_read: u64,
}

fn phase<T>(name: &'static str, io: &Cell<IoStats>, work: impl FnOnce() -> T) -> (Row, T) {
    io.set(IoStats::default());
    let start = Instant::now();
    let before = HEAP.begin();
    let result = work();
    let after = HEAP.sample();
    let wall_ns = start.elapsed().as_nanos();
    (
        Row {
            name,
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
    if std::env::args_os().len() != 1 {
        eprintln!("usage: afsplus-measure (fixed memory-only workload, no arguments)");
        std::process::exit(1);
    }
    let io = Cell::new(IoStats::default());
    let mut rows = Vec::with_capacity(12);
    let mut dev = Image {
        bytes: vec![0; BLOCKS as usize * BS],
        io: &io,
    };
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
    let (row, mut dev) = phase("final-unmount", &io, || volume.into_device());
    rows.push(row);
    let (row, ()) = phase("recovered-check", &io, || {
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "recovered checker: {:?}", report.errors);
    });
    rows.push(row);
    let image_crc = afsplus_format::crc32c::crc32c(&dev.bytes);
    // Reporting occurs after all samples so JSON formatting cannot inflate a phase.
    println!("{{\"version\":1,\"workload\":\"small-files-v1\",\"outcome\":\"pass\",\"backend\":\"fixed-memory\",\"image_bytes\":{},\"image_crc32c\":{},\"last_edit_bitmap_payload_peak_bytes\":{},\"phases\":[", dev.bytes.len(), image_crc, bitmap_peak);
    for (i, row) in rows.iter().enumerate() {
        if i != 0 {
            println!(",");
        }
        println!("{{\"name\":\"{}\",\"wall_ns\":{},\"heap_start_bytes\":{},\"heap_end_bytes\":{},\"heap_peak_bytes\":{},\"heap_peak_above_start_bytes\":{},\"heap_acquired_bytes\":{},\"heap_released_bytes\":{},\"reads\":{},\"writes\":{},\"bytes_read\":{},\"bytes_written\":{},\"flushes\":{},\"logical_payload_written_bytes\":{},\"logical_payload_read_bytes\":{}}}",
            row.name, row.wall_ns, row.before.live, row.after.live, row.after.peak,
            row.after.peak - row.before.live,
            row.after.acquired.wrapping_sub(row.before.acquired), row.after.released.wrapping_sub(row.before.released),
            row.io.reads, row.io.writes, row.io.bytes_read, row.io.bytes_written, row.io.flushes,
            row.payload_written, row.payload_read);
    }
    println!("]}}");
}
