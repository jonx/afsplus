//! Intent-log placement, scanning, and validation (ADR-037).
//!
//! The log area is `log_slots` blocks at deterministic LBAs directly after
//! the allocation-root pool, permanently marked allocated by mkfs and never
//! owned by the allocator: appending a record allocates nothing.
//!
//! [`scan`] returns the valid record prefix for one base checkpoint
//! generation: records must decode, bind to the volume UUID and that exact
//! generation, carry contiguous sequences from 1, reference in-bounds
//! extents, and (for creates) match their content CRC. The first failure
//! ends the prefix — with barriers ordered as written, a durable record can
//! never follow a lost one, so everything past the first invalid slot
//! belongs to an fsync that never completed.

use afsplus_block::BlockDevice;
use afsplus_format::crc32c::Hasher;
use afsplus_format::geometry::Geometry;
use afsplus_format::intent_log::{LogOp, LogRecord};

use crate::allocation_root;
use crate::CoreError;

/// Deterministic LBAs of the log area.
pub fn log_slot_lbas(geo: &Geometry, log_slots: u16) -> Result<Vec<u64>, CoreError> {
    if log_slots == 0 {
        return Ok(Vec::new());
    }
    let skip = allocation_root::reserved_span_before_log(geo)?;
    allocation_root::derive_reserved_lbas(geo, skip, log_slots as usize)
}

pub struct ScannedLog {
    /// Valid record prefix, sequence order.
    pub records: Vec<LogRecord>,
    /// Why the prefix ended (None: clean end or empty slot).
    pub tail_note: Option<String>,
}

/// Reads the valid record prefix bound to `base_generation`.
pub fn scan<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    log_slots: u16,
    uuid: &[u8; 16],
    base_generation: u64,
) -> Result<ScannedLog, CoreError> {
    let slots = log_slot_lbas(geo, log_slots)?;
    let mut buf = vec![0u8; geo.block_size];
    let mut records = Vec::new();
    let mut tail_note = None;
    for (index, lba) in slots.iter().enumerate() {
        dev.read_block(*lba, &mut buf)?;
        let record = match LogRecord::decode(&buf) {
            Ok(record) => record,
            Err(_) => break, // empty, stale-torn, or foreign slot: prefix ends
        };
        if record.uuid != *uuid || record.base_generation != base_generation {
            break; // stale binding from an earlier generation
        }
        if record.sequence as usize != index + 1 {
            tail_note = Some(format!(
                "log slot {index} carries sequence {}, expected {}",
                record.sequence,
                index + 1
            ));
            break;
        }
        match verify_record_contents(dev, geo, &record) {
            Ok(()) => records.push(record),
            Err(reason) => {
                // A durable record over torn data: that fsync never
                // completed. Legal crash artifact; the prefix ends here.
                tail_note = Some(reason);
                break;
            }
        }
    }
    Ok(ScannedLog { records, tail_note })
}

/// Bounds- and CRC-checks one record's referenced content.
fn verify_record_contents<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    record: &LogRecord,
) -> Result<(), String> {
    let mut buf = vec![0u8; geo.block_size];
    for op in &record.ops {
        let LogOp::Create {
            extents,
            size_bytes,
            content_crc,
            ..
        } = op
        else {
            continue;
        };
        let mut remaining = *size_bytes as usize;
        let mut hasher = Hasher::new();
        for (start, blocks) in extents {
            let end = start + *blocks as u64;
            if end > geo.total_blocks
                || !geo.is_allocatable(*start)
                || geo.region_of(*start) != geo.region_of(end.saturating_sub(1))
            {
                return Err(format!(
                    "log record {} references out-of-bounds extent {start}+{blocks}",
                    record.sequence
                ));
            }
            for lba in *start..end {
                if remaining == 0 {
                    break;
                }
                dev.read_block(lba, &mut buf).map_err(|e| e.to_string())?;
                let take = remaining.min(geo.block_size);
                hasher.update(&buf[..take]);
                remaining -= take;
            }
        }
        if remaining != 0 {
            return Err(format!(
                "log record {} content larger than its extents",
                record.sequence
            ));
        }
        if hasher.finalize() != *content_crc {
            return Err(format!(
                "log record {} content CRC mismatch (torn data)",
                record.sequence
            ));
        }
    }
    Ok(())
}
