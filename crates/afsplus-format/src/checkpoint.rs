//! Checkpoint record (ADR-020).
//!
//! Two alternating single-block slots. A commit writes the *other* slot with
//! generation + 1 after all referenced COW metadata is durable, so the newest
//! valid checkpoint is never overwritten. A torn or incomplete checkpoint
//! fails its CRC and mount falls back to the previous slot.
//!
//! The `next_free_block` / `next_object_id` fields carry the state of the
//! bootstrap bump allocator. This is explicitly pre-blocker-3 scaffolding
//! (`implementation/peer-review-prototype-plan.md`, architecture blocker 3):
//! it never reuses storage, which trivially satisfies the retired-block
//! quarantine invariant at the cost of leaking all freed space. The real
//! allocation-state representation is decided by the allocator prototype.
//!
//! Payload layout after the common header (header.generation mirrors
//! `generation` for tooling):
//!
//! ```text
//! offset size field
//! 0      16   filesystem UUID (binds the record to its volume)
//! 16     8    checkpoint generation
//! 24     8    root object ID
//! 32     8    object map LBA
//! 40     8    next free metadata/data LBA (bootstrap allocator high-water)
//! 48     8    next dynamic object ID
//! 56     8    committed transaction ID
//! 64     8    flags (zero; reserved)
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, OBJECT_INVALID};

const PAYLOAD_LEN: usize = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    pub uuid: [u8; 16],
    pub generation: u64,
    pub root_object_id: u64,
    pub object_map_block: u64,
    pub next_free_block: u64,
    pub next_object_id: u64,
    pub committed_tx_id: u64,
    pub flags: u64,
}

impl Checkpoint {
    pub fn encode(&self, block_size: usize) -> Result<Vec<u8>, FormatError> {
        if self.generation == 0 {
            return Err(FormatError::Invalid("checkpoint generation must be nonzero"));
        }
        if self.root_object_id == OBJECT_INVALID {
            return Err(FormatError::Invalid("root object ID is invalid"));
        }
        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        p[0..16].copy_from_slice(&self.uuid);
        le::put_u64(&mut p[16..24], self.generation);
        le::put_u64(&mut p[24..32], self.root_object_id);
        le::put_u64(&mut p[32..40], self.object_map_block);
        le::put_u64(&mut p[40..48], self.next_free_block);
        le::put_u64(&mut p[48..56], self.next_object_id);
        le::put_u64(&mut p[56..64], self.committed_tx_id);
        le::put_u64(&mut p[64..72], self.flags);

        BlockHeader {
            block_type: block_type::CHECKPOINT,
            flags: 0,
            owner: 0,
            generation: self.generation,
            payload_len: PAYLOAD_LEN as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    /// Decodes and validates one checkpoint slot.
    ///
    /// `expected_uuid` binds the slot to the mounted volume: a checkpoint from
    /// another AFS+ image (for example after an image copy onto a reused
    /// device) must never be selected.
    pub fn decode(block: &[u8], expected_uuid: &[u8; 16]) -> Result<Checkpoint, FormatError> {
        let header = BlockHeader::verify(block, block_type::CHECKPOINT)?;
        let p = header.payload(block);
        if p.len() < PAYLOAD_LEN {
            return Err(FormatError::Invalid("checkpoint payload too short"));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&p[0..16]);
        if &uuid != expected_uuid {
            return Err(FormatError::Invalid("checkpoint UUID does not match volume"));
        }
        let generation = le::get_u64(&p[16..24]);
        if generation == 0 || generation != header.generation {
            return Err(FormatError::Invalid("checkpoint generation invalid or inconsistent"));
        }
        let checkpoint = Checkpoint {
            uuid,
            generation,
            root_object_id: le::get_u64(&p[24..32]),
            object_map_block: le::get_u64(&p[32..40]),
            next_free_block: le::get_u64(&p[40..48]),
            next_object_id: le::get_u64(&p[48..56]),
            committed_tx_id: le::get_u64(&p[56..64]),
            flags: le::get_u64(&p[64..72]),
        };
        if checkpoint.root_object_id == OBJECT_INVALID {
            return Err(FormatError::Invalid("root object ID is invalid"));
        }
        Ok(checkpoint)
    }
}
