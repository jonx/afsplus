//! Allocation-region geometry and deterministic reserved metadata placement.
//!
//! Each region starts with three region-descriptor slots followed by three
//! physical slots for every logical bitmap page. Region 0 additionally starts
//! with identification and the two checkpoint blocks.

use crate::bitmap::BITMAP_PAGE_BLOCKS;
use crate::FormatError;

pub const BITMAP_SLOTS: u8 = 3;
pub const DESCRIPTOR_SLOTS: u8 = 3;
pub const BOOTSTRAP_BLOCKS: u64 = 3;
pub const MAX_REGION_BLOCKS: u32 = 262_144;
pub const MIN_REGION_BLOCKS: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub block_size: usize,
    pub total_blocks: u64,
    pub region_size: u32,
}

impl Geometry {
    pub fn validate(&self) -> Result<(), FormatError> {
        if self.block_size != 4096 {
            return Err(FormatError::Invalid("prototype supports only 4 KiB blocks"));
        }
        if !self.region_size.is_power_of_two()
            || self.region_size < MIN_REGION_BLOCKS
            || self.region_size > MAX_REGION_BLOCKS
        {
            return Err(FormatError::Invalid("region size out of prototype range"));
        }
        if self.total_blocks == 0 {
            return Err(FormatError::Invalid("total_blocks is zero"));
        }
        let regions = self.total_blocks.div_ceil(self.region_size as u64);
        if regions > u32::MAX as u64 {
            return Err(FormatError::Invalid("too many allocation regions"));
        }
        // Only region 0 and the possibly partial final region differ from a
        // regular full region. Keep identification validation O(1): mount
        // must not loop once per allocation region.
        let last_region = regions as u32 - 1;
        for region in [0, last_region] {
            let valid = self.region_valid_blocks(region);
            let reserved = self.region_reserved_blocks(region)
                + if region == 0 { BOOTSTRAP_BLOCKS } else { 0 };
            if valid as u64 <= reserved {
                return Err(FormatError::Invalid(
                    "allocation region is all reserved metadata",
                ));
            }
        }
        Ok(())
    }

    pub fn region_count(&self) -> u32 {
        self.total_blocks.div_ceil(self.region_size as u64) as u32
    }

    pub fn region_base(&self, region: u32) -> u64 {
        region as u64 * self.region_size as u64
    }

    pub fn region_valid_blocks(&self, region: u32) -> u32 {
        let base = self.region_base(region);
        (self.total_blocks - base).min(self.region_size as u64) as u32
    }

    pub fn region_of(&self, lba: u64) -> u32 {
        (lba / self.region_size as u64) as u32
    }

    pub fn bitmap_page_count(&self, region: u32) -> u32 {
        self.region_valid_blocks(region)
            .div_ceil(BITMAP_PAGE_BLOCKS)
    }

    pub fn bitmap_page_valid_blocks(&self, region: u32, page_index: u32) -> u32 {
        let first = page_index * BITMAP_PAGE_BLOCKS;
        self.region_valid_blocks(region)
            .saturating_sub(first)
            .min(BITMAP_PAGE_BLOCKS)
    }

    pub fn bitmap_page_for_index(&self, region_index: u32) -> (u32, u32) {
        (
            region_index / BITMAP_PAGE_BLOCKS,
            region_index % BITMAP_PAGE_BLOCKS,
        )
    }

    /// Descriptor plus bitmap slots reserved at the start of this region.
    pub fn region_reserved_blocks(&self, region: u32) -> u64 {
        DESCRIPTOR_SLOTS as u64 + self.bitmap_page_count(region) as u64 * BITMAP_SLOTS as u64
    }

    pub fn region0_reserved_blocks(&self) -> u64 {
        BOOTSTRAP_BLOCKS + self.region_reserved_blocks(0)
    }

    pub fn descriptor_slot_lba(&self, region: u32, slot: u8) -> u64 {
        debug_assert!(slot < DESCRIPTOR_SLOTS);
        let head = if region == 0 { BOOTSTRAP_BLOCKS } else { 0 };
        self.region_base(region) + head + slot as u64
    }

    pub fn bitmap_slot_lba(&self, region: u32, page_index: u32, slot: u8) -> u64 {
        debug_assert!(page_index < self.bitmap_page_count(region));
        debug_assert!(slot < BITMAP_SLOTS);
        let head = if region == 0 { BOOTSTRAP_BLOCKS } else { 0 };
        self.region_base(region)
            + head
            + DESCRIPTOR_SLOTS as u64
            + page_index as u64 * BITMAP_SLOTS as u64
            + slot as u64
    }

    pub fn is_reserved(&self, lba: u64) -> bool {
        if lba >= self.total_blocks {
            return false;
        }
        let region = self.region_of(lba);
        let offset = lba - self.region_base(region);
        let reserved =
            self.region_reserved_blocks(region) + if region == 0 { BOOTSTRAP_BLOCKS } else { 0 };
        offset < reserved
    }

    pub fn is_allocatable(&self, lba: u64) -> bool {
        lba < self.total_blocks && !self.is_reserved(lba)
    }
}
