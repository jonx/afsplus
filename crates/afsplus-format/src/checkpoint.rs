//! Checkpoint record (ADR-020).
//!
//! Two alternating single-block slots. A commit writes the *other* slot with
//! generation + 1 after all referenced COW metadata and bitmap pages are
//! durable, so the newest valid checkpoint is never overwritten. A torn or
//! incomplete checkpoint fails its CRC and mount falls back to the previous
//! slot.
//!
//! The checkpoint binds the allocation state with one record per region. The
//! record selects a reserved region-descriptor slot; that descriptor selects
//! the independently versioned bitmap pages.
//!
//! Structural validation (`validate_structural`) is everything that can be
//! checked without reading any other block; mount uses it to *select* a
//! checkpoint. Whether the referenced state actually validates is a separate
//! step whose failure is reported as corruption, never silently masked.
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
//! 40     8    retired list LBA (0 = empty list)
//! 48     8    next dynamic object ID
//! 56     8    committed transaction ID
//! 64     8    flags (zero; reserved)
//! 72     4    region record count
//! 76     4    reserved
//! 80     ...  region records: descriptor slot (1), reserved (3), free
//!             blocks (4), descriptor generation (8)
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::geometry::{Geometry, DESCRIPTOR_SLOTS};
use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, OBJECT_FIRST_DYNAMIC, OBJECT_ROOT};

const FIXED_PAYLOAD: usize = 80;
const REGION_RECORD_SIZE: usize = 16;

/// Per-region allocation-state binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionRecord {
    /// Which reserved region-descriptor slot holds this state.
    pub descriptor_slot: u8,
    /// Free blocks in the region at this checkpoint (also cross-checked
    /// against the decoded descriptor and bitmap pages).
    pub free_blocks: u32,
    /// Generation stamped in the region descriptor.
    pub descriptor_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub uuid: [u8; 16],
    pub generation: u64,
    pub root_object_id: u64,
    pub object_map_block: u64,
    /// 0 when no blocks are currently retired.
    pub retired_list_block: u64,
    pub next_object_id: u64,
    pub committed_tx_id: u64,
    pub flags: u64,
    pub regions: Vec<RegionRecord>,
}

impl Checkpoint {
    pub fn encode(&self, block_size: usize) -> Result<Vec<u8>, FormatError> {
        if self.generation == 0 {
            return Err(FormatError::Invalid("checkpoint generation must be nonzero"));
        }
        if self.root_object_id != OBJECT_ROOT {
            return Err(FormatError::Invalid("root object ID is invalid"));
        }
        let payload_len = FIXED_PAYLOAD + self.regions.len() * REGION_RECORD_SIZE;
        if payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow("too many regions for one checkpoint block"));
        }

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        p[0..16].copy_from_slice(&self.uuid);
        le::put_u64(&mut p[16..24], self.generation);
        le::put_u64(&mut p[24..32], self.root_object_id);
        le::put_u64(&mut p[32..40], self.object_map_block);
        le::put_u64(&mut p[40..48], self.retired_list_block);
        le::put_u64(&mut p[48..56], self.next_object_id);
        le::put_u64(&mut p[56..64], self.committed_tx_id);
        le::put_u64(&mut p[64..72], self.flags);
        le::put_u32(&mut p[72..76], self.regions.len() as u32);
        for (i, record) in self.regions.iter().enumerate() {
            let offset = FIXED_PAYLOAD + i * REGION_RECORD_SIZE;
            p[offset] = record.descriptor_slot;
            le::put_u32(&mut p[offset + 4..offset + 8], record.free_blocks);
            le::put_u64(&mut p[offset + 8..offset + 16], record.descriptor_generation);
        }

        BlockHeader {
            block_type: block_type::CHECKPOINT,
            flags: 0,
            owner: 0,
            generation: self.generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    /// Decodes one checkpoint slot.
    ///
    /// `expected_uuid` binds the slot to the mounted volume: a checkpoint from
    /// another AFS+ image (for example after an image copy onto a reused
    /// device) must never be selected.
    pub fn decode(block: &[u8], expected_uuid: &[u8; 16]) -> Result<Checkpoint, FormatError> {
        let header = BlockHeader::verify(block, block_type::CHECKPOINT)?;
        let p = header.payload(block);
        if p.len() < FIXED_PAYLOAD {
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
        let region_count = le::get_u32(&p[72..76]) as usize;
        if region_count > (p.len() - FIXED_PAYLOAD) / REGION_RECORD_SIZE {
            return Err(FormatError::Invalid("checkpoint region count exceeds payload"));
        }
        if header.payload_len as usize != FIXED_PAYLOAD + region_count * REGION_RECORD_SIZE {
            return Err(FormatError::Invalid("checkpoint payload length mismatch"));
        }
        let mut regions = Vec::with_capacity(region_count);
        for i in 0..region_count {
            let offset = FIXED_PAYLOAD + i * REGION_RECORD_SIZE;
            regions.push(RegionRecord {
                descriptor_slot: p[offset],
                free_blocks: le::get_u32(&p[offset + 4..offset + 8]),
                descriptor_generation: le::get_u64(&p[offset + 8..offset + 16]),
            });
        }
        let checkpoint = Checkpoint {
            uuid,
            generation,
            root_object_id: le::get_u64(&p[24..32]),
            object_map_block: le::get_u64(&p[32..40]),
            retired_list_block: le::get_u64(&p[40..48]),
            next_object_id: le::get_u64(&p[48..56]),
            committed_tx_id: le::get_u64(&p[56..64]),
            flags: le::get_u64(&p[64..72]),
            regions,
        };
        if checkpoint.root_object_id != OBJECT_ROOT {
            return Err(FormatError::Invalid("root object ID is invalid"));
        }
        Ok(checkpoint)
    }

    /// Everything checkable without any further I/O. This is the whole basis
    /// on which mount *selects* a checkpoint.
    pub fn validate_structural(&self, geo: &Geometry) -> Result<(), FormatError> {
        if self.regions.len() != geo.region_count() as usize {
            return Err(FormatError::Invalid("checkpoint region count does not match geometry"));
        }
        for (i, record) in self.regions.iter().enumerate() {
            if record.descriptor_slot >= DESCRIPTOR_SLOTS {
                return Err(FormatError::Invalid("region descriptor slot out of range"));
            }
            if record.descriptor_generation == 0
                || record.descriptor_generation > self.generation
            {
                return Err(FormatError::Invalid("region descriptor generation out of range"));
            }
            if record.free_blocks > geo.region_valid_blocks(i as u32) {
                return Err(FormatError::Invalid("region free count exceeds region size"));
            }
        }
        if !geo.is_allocatable(self.object_map_block) {
            return Err(FormatError::Invalid("object map block out of allocatable bounds"));
        }
        if self.retired_list_block != 0 && !geo.is_allocatable(self.retired_list_block) {
            return Err(FormatError::Invalid("retired list block out of allocatable bounds"));
        }
        if self.next_object_id < OBJECT_FIRST_DYNAMIC {
            return Err(FormatError::Invalid("next object ID below dynamic range"));
        }
        Ok(())
    }
}
