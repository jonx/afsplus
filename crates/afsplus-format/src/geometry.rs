//! Volume geometry: allocation regions and reserved block placement.
//!
//! The volume is tiled into fixed-size allocation regions
//! (`docs/07-allocation.md`). Every region owns three *reserved* bitmap slot
//! blocks at deterministic locations, outside the allocator's own domain:
//!
//! ```text
//! region 0:  lba 0 ident | 1 ckpt A | 2 ckpt B | 3,4,5 bitmap slots | data...
//! region r:  base+0,1,2 bitmap slots | data...
//! ```
//!
//! Three generational slots let a commit write the *next* allocation state
//! while the states referenced by both retained checkpoints stay untouched.
//! This deliberately breaks the "allocate a COW copy of the bitmap → the
//! bitmap changes → allocate again" recursion (architecture blocker 3): the
//! bitmap pages are never allocated through the allocator they describe.

use crate::FormatError;

/// Bitmap slots per region.
pub const BITMAP_SLOTS: u8 = 3;
/// Reserved blocks at the head of region 0 (ident, 2 checkpoints, 3 slots).
pub const REGION0_RESERVED: u64 = 6;
/// Reserved blocks at the head of every other region (3 bitmap slots).
pub const REGION_RESERVED: u64 = 3;

/// Maximum region size such that one 4 KiB bitmap page covers the region.
/// (4096 - 32 header - 8 fixed payload) * 8 bits. Larger regions need
/// multi-page bitmaps, which the prototype does not implement.
pub const MAX_REGION_BLOCKS: u32 = 32384;
pub const MIN_REGION_BLOCKS: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub block_size: usize,
    pub total_blocks: u64,
    pub region_size: u32,
}

impl Geometry {
    pub fn validate(&self) -> Result<(), FormatError> {
        if !self.region_size.is_power_of_two()
            || self.region_size < MIN_REGION_BLOCKS
            || self.region_size > MAX_REGION_BLOCKS
        {
            return Err(FormatError::Invalid("region size out of prototype range"));
        }
        if self.total_blocks == 0 {
            return Err(FormatError::Invalid("total_blocks is zero"));
        }
        if self.total_blocks < REGION0_RESERVED + 2 {
            return Err(FormatError::Invalid("volume smaller than reserved area"));
        }
        Ok(())
    }

    pub fn region_count(&self) -> u32 {
        self.total_blocks.div_ceil(self.region_size as u64) as u32
    }

    pub fn region_base(&self, region: u32) -> u64 {
        region as u64 * self.region_size as u64
    }

    /// Number of volume blocks actually covered by a region (the last region
    /// may be partial).
    pub fn region_valid_blocks(&self, region: u32) -> u32 {
        let base = self.region_base(region);
        (self.total_blocks - base).min(self.region_size as u64) as u32
    }

    pub fn region_of(&self, lba: u64) -> u32 {
        (lba / self.region_size as u64) as u32
    }

    /// Deterministic location of a region's bitmap slot block.
    pub fn bitmap_slot_lba(&self, region: u32, slot: u8) -> u64 {
        debug_assert!(slot < BITMAP_SLOTS);
        let head = if region == 0 { REGION0_RESERVED - BITMAP_SLOTS as u64 } else { 0 };
        self.region_base(region) + head + slot as u64
    }

    /// Reserved blocks are outside the allocator: identification, checkpoint
    /// slots, and every region's bitmap slot blocks.
    pub fn is_reserved(&self, lba: u64) -> bool {
        let offset = lba % self.region_size as u64;
        if self.region_of(lba) == 0 {
            offset < REGION0_RESERVED
        } else {
            offset < REGION_RESERVED
        }
    }

    /// A block the allocator may hand out / metadata may live in.
    pub fn is_allocatable(&self, lba: u64) -> bool {
        lba < self.total_blocks && !self.is_reserved(lba)
    }
}
