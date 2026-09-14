//! Checksummed binding from one allocation region to its bitmap pages.

use alloc::vec;
use alloc::vec::Vec;

use crate::geometry::{Geometry, BITMAP_SLOTS};
use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError};

const FIXED_PAYLOAD: usize = 16;
const PAGE_BINDING_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapBinding {
    pub slot: u8,
    pub free_blocks: u32,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionDescriptor {
    pub region: u32,
    pub valid_blocks: u32,
    pub free_blocks: u32,
    pub pages: Vec<BitmapBinding>,
}

impl RegionDescriptor {
    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        if self.pages.len() > u32::MAX as usize {
            return Err(FormatError::Overflow("region descriptor page count"));
        }
        let payload_len = FIXED_PAYLOAD + self.pages.len() * PAGE_BINDING_SIZE;
        if block_size < HEADER_SIZE || payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow("region descriptor exceeds one block"));
        }
        let sum = self.pages.iter().try_fold(0u32, |total, page| {
            total
                .checked_add(page.free_blocks)
                .ok_or(FormatError::Overflow("region free count"))
        })?;
        if sum != self.free_blocks {
            return Err(FormatError::Invalid(
                "region descriptor free count mismatch",
            ));
        }
        for page in &self.pages {
            if page.slot >= BITMAP_SLOTS || page.generation == 0 || page.generation > generation {
                return Err(FormatError::Invalid("region bitmap binding out of range"));
            }
        }

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.region);
        le::put_u32(&mut p[4..8], self.valid_blocks);
        le::put_u32(&mut p[8..12], self.free_blocks);
        le::put_u32(&mut p[12..16], self.pages.len() as u32);
        for (index, binding) in self.pages.iter().enumerate() {
            let offset = FIXED_PAYLOAD + index * PAGE_BINDING_SIZE;
            p[offset] = binding.slot;
            le::put_u32(&mut p[offset + 4..offset + 8], binding.free_blocks);
            le::put_u64(&mut p[offset + 8..offset + 16], binding.generation);
        }

        BlockHeader {
            block_type: block_type::REGION_DESCRIPTOR,
            flags: 0,
            owner: self.region as u64,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<(RegionDescriptor, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::REGION_DESCRIPTOR)?;
        let p = header.payload(block);
        if p.len() < FIXED_PAYLOAD {
            return Err(FormatError::Invalid("region descriptor payload too short"));
        }
        let region = le::get_u32(&p[0..4]);
        if region as u64 != header.owner {
            return Err(FormatError::Invalid(
                "descriptor region does not match block owner",
            ));
        }
        let page_count = le::get_u32(&p[12..16]) as usize;
        if page_count > (p.len() - FIXED_PAYLOAD) / PAGE_BINDING_SIZE
            || p.len() != FIXED_PAYLOAD + page_count * PAGE_BINDING_SIZE
        {
            return Err(FormatError::Invalid(
                "region descriptor page count exceeds payload",
            ));
        }
        let mut pages = Vec::with_capacity(page_count);
        for index in 0..page_count {
            let offset = FIXED_PAYLOAD + index * PAGE_BINDING_SIZE;
            pages.push(BitmapBinding {
                slot: p[offset],
                free_blocks: le::get_u32(&p[offset + 4..offset + 8]),
                generation: le::get_u64(&p[offset + 8..offset + 16]),
            });
        }
        let descriptor = RegionDescriptor {
            region,
            valid_blocks: le::get_u32(&p[4..8]),
            free_blocks: le::get_u32(&p[8..12]),
            pages,
        };
        Ok((descriptor, header.generation))
    }

    pub fn validate(
        &self,
        geo: &Geometry,
        expected_region: u32,
        descriptor_generation: u64,
    ) -> Result<(), FormatError> {
        if self.region != expected_region
            || self.valid_blocks != geo.region_valid_blocks(expected_region)
            || self.pages.len() != geo.bitmap_page_count(expected_region) as usize
        {
            return Err(FormatError::Invalid("region descriptor geometry mismatch"));
        }
        let mut free = 0u32;
        for (page_index, binding) in self.pages.iter().enumerate() {
            if binding.slot >= BITMAP_SLOTS
                || binding.generation == 0
                || binding.generation > descriptor_generation
                || binding.free_blocks
                    > geo.bitmap_page_valid_blocks(expected_region, page_index as u32)
            {
                return Err(FormatError::Invalid("region bitmap binding out of range"));
            }
            free = free
                .checked_add(binding.free_blocks)
                .ok_or(FormatError::Overflow("region free count"))?;
        }
        if free != self.free_blocks {
            return Err(FormatError::Invalid(
                "region descriptor free count mismatch",
            ));
        }
        Ok(())
    }
}
