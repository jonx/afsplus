//! One logical page of a region allocation bitmap.
//!
//! Large allocation regions are split into independently checksummed pages.
//! Each logical page has three deterministic physical slots; a region
//! descriptor binds the slot and generation selected by a checkpoint.
//! Bit = 1 means allocated. Retired blocks remain 1 until quarantine ends.
//!
//! Bits are LSB-first. Bits beyond `valid_blocks` in the final byte are
//! written as 1 and verified on decode.

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError};

/// Payload identity preceding the bitmap bytes.
pub const BITMAP_FIXED_PAYLOAD: usize = 16;
/// Region blocks represented by one bitmap block at the prototype's 4 KiB
/// logical block size.
pub const BITMAP_PAGE_BLOCKS: u32 = ((4096 - HEADER_SIZE - BITMAP_FIXED_PAYLOAD) * 8) as u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapPage {
    pub region: u32,
    pub page_index: u32,
    /// First region-relative block represented by this page.
    pub first_block: u32,
    pub valid_blocks: u32,
    /// `ceil(valid_blocks / 8)` bytes; trailing bits are 1.
    pub bits: Vec<u8>,
}

impl BitmapPage {
    /// A fresh logical page with every represented block free.
    pub fn all_free(region: u32, page_index: u32, first_block: u32, valid_blocks: u32) -> Self {
        let mut page = BitmapPage {
            region,
            page_index,
            first_block,
            valid_blocks,
            bits: vec![0u8; (valid_blocks as usize).div_ceil(8)],
        };
        page.seal_trailing_bits();
        page
    }

    pub fn is_allocated(&self, local_index: u32) -> bool {
        debug_assert!(local_index < self.valid_blocks);
        self.bits[local_index as usize / 8] & (1 << (local_index % 8)) != 0
    }

    /// Sets a block's state; returns whether the bit changed.
    pub fn set_allocated(&mut self, local_index: u32, allocated: bool) -> bool {
        debug_assert!(local_index < self.valid_blocks);
        let byte = &mut self.bits[local_index as usize / 8];
        let mask = 1 << (local_index % 8);
        let was = *byte & mask != 0;
        if allocated {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        was != allocated
    }

    pub fn free_blocks(&self) -> u32 {
        let ones: u32 = self.bits.iter().map(|b| b.count_ones()).sum();
        let trailing = (self.bits.len() as u32 * 8) - self.valid_blocks;
        self.valid_blocks - (ones - trailing)
    }

    fn seal_trailing_bits(&mut self) {
        let total_bits = self.bits.len() as u32 * 8;
        for index in self.valid_blocks..total_bits {
            self.bits[index as usize / 8] |= 1 << (index % 8);
        }
    }

    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        if self.valid_blocks == 0 || self.valid_blocks > BITMAP_PAGE_BLOCKS {
            return Err(FormatError::Invalid(
                "bitmap page valid block count out of range",
            ));
        }
        let expected_first = self
            .page_index
            .checked_mul(BITMAP_PAGE_BLOCKS)
            .ok_or(FormatError::Overflow("bitmap page first block"))?;
        if self.first_block != expected_first {
            return Err(FormatError::Invalid(
                "bitmap page offset does not match page index",
            ));
        }
        let expected_len = (self.valid_blocks as usize).div_ceil(8);
        if self.bits.len() != expected_len {
            return Err(FormatError::Invalid("bitmap byte length mismatch"));
        }
        let payload_len = BITMAP_FIXED_PAYLOAD + self.bits.len();
        if payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow(
                "region bitmap page exceeds one block",
            ));
        }
        let mut copy = self.clone();
        copy.seal_trailing_bits();

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.region);
        le::put_u32(&mut p[4..8], self.page_index);
        le::put_u32(&mut p[8..12], self.first_block);
        le::put_u32(&mut p[12..16], self.valid_blocks);
        p[16..16 + copy.bits.len()].copy_from_slice(&copy.bits);

        BlockHeader {
            block_type: block_type::BITMAP,
            flags: 0,
            owner: self.region as u64,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    /// Decodes one logical page and returns its header generation.
    pub fn decode(block: &[u8]) -> Result<(BitmapPage, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::BITMAP)?;
        let p = header.payload(block);
        if p.len() < BITMAP_FIXED_PAYLOAD {
            return Err(FormatError::Invalid("bitmap payload too short"));
        }
        let region = le::get_u32(&p[0..4]);
        let page_index = le::get_u32(&p[4..8]);
        let first_block = le::get_u32(&p[8..12]);
        let valid_blocks = le::get_u32(&p[12..16]);
        if valid_blocks == 0 || valid_blocks > BITMAP_PAGE_BLOCKS {
            return Err(FormatError::Invalid(
                "bitmap page valid block count out of range",
            ));
        }
        let expected_first = page_index
            .checked_mul(BITMAP_PAGE_BLOCKS)
            .ok_or(FormatError::Overflow("bitmap page first block"))?;
        if first_block != expected_first {
            return Err(FormatError::Invalid(
                "bitmap page offset does not match page index",
            ));
        }
        let byte_len = (valid_blocks as usize).div_ceil(8);
        if p.len() != BITMAP_FIXED_PAYLOAD + byte_len {
            return Err(FormatError::Invalid("bitmap payload length mismatch"));
        }
        if region as u64 != header.owner {
            return Err(FormatError::Invalid(
                "bitmap region does not match block owner",
            ));
        }
        let page = BitmapPage {
            region,
            page_index,
            first_block,
            valid_blocks,
            bits: p[BITMAP_FIXED_PAYLOAD..].to_vec(),
        };
        let total_bits = page.bits.len() as u32 * 8;
        for index in valid_blocks..total_bits {
            if page.bits[index as usize / 8] & (1 << (index % 8)) == 0 {
                return Err(FormatError::Invalid("bitmap trailing bits not sealed"));
            }
        }
        Ok((page, header.generation))
    }
}
