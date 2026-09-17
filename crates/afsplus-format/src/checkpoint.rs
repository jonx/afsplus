//! Checkpoint record (ADR-020, ADR-061).
//!
//! Two alternating single-block slots. A commit writes the *other* slot with
//! generation + 1 after all referenced COW metadata and bitmap pages are
//! durable, so the newest valid checkpoint is never overwritten. A torn or
//! incomplete checkpoint fails its CRC and mount falls back to the previous
//! slot.
//!
//! The checkpoint binds the allocation state through the AFST allocation
//! root (ADR-035): one tree record per region selects a reserved
//! region-descriptor slot, and that descriptor selects the independently
//! versioned bitmap pages. The transitional inline region records that once
//! followed the fixed payload are removed (ADR-061); an allocation root is
//! mandatory.
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
//! 40     8    allocation-root LBA (always nonzero)
//! 48     8    reclaim-queue root LBA
//! 56     8    next dynamic object ID
//! 64     8    committed transaction ID
//! 72     8    total free blocks
//! 80     8    flags (zero; reserved)
//! 88     8    shared-extent reference-tree root LBA (0 = no tree; ADR-061)
//! 96     1    volume label length in bytes (0..=64; ADR-104)
//! 97     7    reserved (zero)
//! 104    64   volume label, UTF-8 without NUL, zero padded
//! 168    8    snapshot registry root (extended payload only; ADR-073)
//! 176    8    lifetime ledger root (extended payload only; ADR-073)
//! ```
//!
//! The label is committed state: a relabel is an ordinary commit, so a power
//! cut leaves the old label or the new one. The identification block keeps
//! the label given at format time and is never rewritten.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::ident::{validate_label, LABEL_MAX_BYTES};

use crate::geometry::Geometry;
use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, OBJECT_FIRST_DYNAMIC, OBJECT_ROOT};

const LABEL_OFFSET: usize = 96;
const FIXED_PAYLOAD: usize = LABEL_OFFSET + 8 + LABEL_MAX_BYTES;
const SNAPSHOT_PAYLOAD: usize = FIXED_PAYLOAD + 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotRoots {
    pub registry: u64,
    pub lifetimes: u64,
}

impl SnapshotRoots {
    fn validate(self) -> Result<(), FormatError> {
        if self.registry == 0 || self.lifetimes == 0 || self.registry == self.lifetimes {
            return Err(FormatError::Invalid(
                "snapshot roots must be nonzero and distinct",
            ));
        }
        Ok(())
    }
}

/// Per-region allocation-state binding, stored as the value of an
/// allocation-root tree record (ADR-035). No longer carried inline by the
/// checkpoint itself.
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
    /// AFST allocation-region root (ADR-035); always nonzero.
    pub allocation_root_block: u64,
    /// Root of the reclaim queue (ADR-036); always nonzero.
    pub reclaim_root_block: u64,
    pub next_object_id: u64,
    pub committed_tx_id: u64,
    pub free_blocks_total: u64,
    pub flags: u64,
    /// Root of the shared-extent reference tree (ADR-061). Zero on a volume
    /// that has never cloned; allocated by the first clone and kept allocated
    /// afterwards, even once the tree is empty again.
    pub shared_extent_root_block: u64,
    /// Current volume label (ADR-104).
    pub label: String,
    /// Negotiated by INCOMPAT_PERSISTENT_SNAPSHOTS; absent uses the short payload.
    pub snapshot_roots: Option<SnapshotRoots>,
}

impl Checkpoint {
    pub fn encode(&self, block_size: usize) -> Result<Vec<u8>, FormatError> {
        if self.generation == 0 {
            return Err(FormatError::Invalid(
                "checkpoint generation must be nonzero",
            ));
        }
        if self.root_object_id != OBJECT_ROOT {
            return Err(FormatError::Invalid("root object ID is invalid"));
        }
        if self.allocation_root_block == 0 {
            return Err(FormatError::Invalid(
                "checkpoint requires an allocation root",
            ));
        }
        validate_label(&self.label)?;
        let payload_len = if let Some(roots) = self.snapshot_roots {
            roots.validate()?;
            SNAPSHOT_PAYLOAD
        } else {
            FIXED_PAYLOAD
        };
        if payload_len > block_size.saturating_sub(HEADER_SIZE) {
            return Err(FormatError::Overflow("checkpoint payload"));
        }

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        p[0..16].copy_from_slice(&self.uuid);
        le::put_u64(&mut p[16..24], self.generation);
        le::put_u64(&mut p[24..32], self.root_object_id);
        le::put_u64(&mut p[32..40], self.object_map_block);
        le::put_u64(&mut p[40..48], self.allocation_root_block);
        le::put_u64(&mut p[48..56], self.reclaim_root_block);
        le::put_u64(&mut p[56..64], self.next_object_id);
        le::put_u64(&mut p[64..72], self.committed_tx_id);
        le::put_u64(&mut p[72..80], self.free_blocks_total);
        le::put_u64(&mut p[80..88], self.flags);
        le::put_u64(&mut p[88..96], self.shared_extent_root_block);
        p[LABEL_OFFSET] = self.label.len() as u8;
        p[LABEL_OFFSET + 8..LABEL_OFFSET + 8 + self.label.len()]
            .copy_from_slice(self.label.as_bytes());
        if let Some(roots) = self.snapshot_roots {
            le::put_u64(&mut p[FIXED_PAYLOAD..FIXED_PAYLOAD + 8], roots.registry);
            le::put_u64(&mut p[FIXED_PAYLOAD + 8..SNAPSHOT_PAYLOAD], roots.lifetimes);
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
        if header.flags != 0 || header.owner != 0 {
            return Err(FormatError::Invalid(
                "checkpoint header reserved fields are nonzero",
            ));
        }
        if !matches!(
            header.payload_len as usize,
            FIXED_PAYLOAD | SNAPSHOT_PAYLOAD
        ) || p.len() < FIXED_PAYLOAD
        {
            return Err(FormatError::Invalid("checkpoint payload length mismatch"));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&p[0..16]);
        if &uuid != expected_uuid {
            return Err(FormatError::Invalid(
                "checkpoint UUID does not match volume",
            ));
        }
        let generation = le::get_u64(&p[16..24]);
        if generation == 0 || generation != header.generation {
            return Err(FormatError::Invalid(
                "checkpoint generation invalid or inconsistent",
            ));
        }
        let label_len = p[LABEL_OFFSET] as usize;
        let label_field = &p[LABEL_OFFSET + 8..FIXED_PAYLOAD];
        if label_len > LABEL_MAX_BYTES
            || p[LABEL_OFFSET + 1..LABEL_OFFSET + 8] != [0; 7]
            || label_field[label_len..].iter().any(|b| *b != 0)
        {
            return Err(FormatError::Invalid(
                "checkpoint label field is not canonical",
            ));
        }
        let label = core::str::from_utf8(&label_field[..label_len])
            .map_err(|_| FormatError::InvalidUtf8)?;
        validate_label(label)?;
        let checkpoint = Checkpoint {
            uuid,
            generation,
            root_object_id: le::get_u64(&p[24..32]),
            object_map_block: le::get_u64(&p[32..40]),
            allocation_root_block: le::get_u64(&p[40..48]),
            reclaim_root_block: le::get_u64(&p[48..56]),
            next_object_id: le::get_u64(&p[56..64]),
            committed_tx_id: le::get_u64(&p[64..72]),
            free_blocks_total: le::get_u64(&p[72..80]),
            flags: le::get_u64(&p[80..88]),
            shared_extent_root_block: le::get_u64(&p[88..96]),
            label: String::from(label),
            snapshot_roots: if p.len() == SNAPSHOT_PAYLOAD {
                let roots = SnapshotRoots {
                    registry: le::get_u64(&p[FIXED_PAYLOAD..FIXED_PAYLOAD + 8]),
                    lifetimes: le::get_u64(&p[FIXED_PAYLOAD + 8..SNAPSHOT_PAYLOAD]),
                };
                roots.validate()?;
                Some(roots)
            } else {
                None
            },
        };
        if checkpoint.root_object_id != OBJECT_ROOT {
            return Err(FormatError::Invalid("root object ID is invalid"));
        }
        if checkpoint.flags != 0 {
            return Err(FormatError::Invalid(
                "checkpoint payload reserved flags are nonzero",
            ));
        }
        Ok(checkpoint)
    }

    /// Everything checkable without any further I/O. This is the whole basis
    /// on which mount *selects* a checkpoint.
    pub fn validate_structural(&self, geo: &Geometry) -> Result<(), FormatError> {
        if let Some(roots) = self.snapshot_roots {
            roots.validate()?;
            if !geo.is_allocatable(roots.registry) || !geo.is_allocatable(roots.lifetimes) {
                return Err(FormatError::Invalid(
                    "snapshot roots outside allocatable bounds",
                ));
            }
        }
        if !geo.is_allocatable(self.allocation_root_block) {
            return Err(FormatError::Invalid(
                "allocation root block out of allocatable bounds",
            ));
        }
        if self.free_blocks_total > geo.total_blocks {
            return Err(FormatError::Invalid(
                "checkpoint free total exceeds volume size",
            ));
        }
        if !geo.is_allocatable(self.object_map_block) {
            return Err(FormatError::Invalid(
                "object map block out of allocatable bounds",
            ));
        }
        if !geo.is_allocatable(self.reclaim_root_block) {
            return Err(FormatError::Invalid(
                "reclaim root block out of allocatable bounds",
            ));
        }
        if self.shared_extent_root_block != 0 && !geo.is_allocatable(self.shared_extent_root_block)
        {
            return Err(FormatError::Invalid(
                "shared-extent root block out of allocatable bounds",
            ));
        }
        if self.next_object_id < OBJECT_FIRST_DYNAMIC {
            return Err(FormatError::Invalid("next object ID below dynamic range"));
        }
        Ok(())
    }
}
