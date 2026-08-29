//! Mount: checkpoint selection with fallback (ADR-020).
//!
//! Mount examines both checkpoint slots and chooses the newest candidate
//! whose *entire reachable state* validates — not merely the newest one whose
//! own checksum passes. A torn or incomplete newer checkpoint (or one whose
//! referenced metadata never became durable) is ignored and the previous
//! checkpoint is used.
//!
//! Ordinary crash recovery reads a bounded number of blocks and never scans
//! the volume (`docs/08-transactions-and-journal.md` §4).

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::ident::Identification;
use afsplus_format::DEFAULT_BLOCK_SIZE;

use crate::verify::validate_checkpoint_reachable;
use crate::volume::Volume;
use crate::{layout, CoreError};

pub fn mount<D: BlockDevice>(mut dev: D) -> Result<Volume<D>, CoreError> {
    if dev.block_size() != DEFAULT_BLOCK_SIZE {
        return Err(CoreError::UnsupportedGeometry("prototype supports only 4 KiB blocks"));
    }

    let mut buf = vec![0u8; dev.block_size()];
    dev.read_block(layout::IDENT_LBA, &mut buf)?;
    let ident = Identification::decode(&buf)?;
    if ident.total_blocks > dev.total_blocks() {
        return Err(CoreError::Corrupt(format!(
            "identification declares {} blocks but device has {}",
            ident.total_blocks,
            dev.total_blocks()
        )));
    }

    // Decode both slots; remember why a slot was rejected for diagnostics.
    let mut candidates: Vec<(usize, Checkpoint)> = Vec::new();
    let mut slot_errors = [String::from("not examined"), String::from("not examined")];
    for (slot, lba) in ident.checkpoint_slots.iter().enumerate() {
        dev.read_block(*lba, &mut buf)?;
        match Checkpoint::decode(&buf, &ident.uuid) {
            Ok(checkpoint) => {
                slot_errors[slot] = String::from("valid");
                candidates.push((slot, checkpoint));
            }
            Err(e) => slot_errors[slot] = e.to_string(),
        }
    }

    // Newest generation first; fall back if its reachable state is invalid.
    candidates.sort_by_key(|(_, c)| core::cmp::Reverse(c.generation));
    for (slot, checkpoint) in candidates {
        match validate_checkpoint_reachable(&mut dev, &ident, &checkpoint) {
            Ok(state) => return Ok(Volume::new(dev, ident, checkpoint, slot, state)),
            Err(e) => slot_errors[slot] = format!("reachable state invalid: {e}"),
        }
    }

    Err(CoreError::NoValidCheckpoint {
        slot_a: std::mem::take(&mut slot_errors[0]),
        slot_b: std::mem::take(&mut slot_errors[1]),
    })
}
