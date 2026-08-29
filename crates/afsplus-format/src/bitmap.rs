//! Region bitmap page.
//!
//! One page per region, written to one of the region's three reserved
//! generational slot blocks (`geometry`). Bit = 1 means allocated. Retired
//! blocks stay at 1 until quarantine releases them (see the retired list).
//!
//! Bits are LSB-first within each byte: block `base + i` is bit `i % 8` of
//! byte `i / 8`. Bits beyond the region's valid range are written as 1
//! (conservatively allocated) and verified on decode.
//!
//! Payload layout after the common header (header.owner = region index,
//! header.generation = the checkpoint generation this state belongs to):
//!
//! ```text
//! offset size field
//! 0      4    region index
//! 4      4    valid block count
//! 8      ...  bitmap bytes, ceil(valid/8)
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapPage {
    pub region: u32,
    pub valid_blocks: u32,
    /// `ceil(valid_blocks / 8)` bytes; trailing bits are 1.
    pub bits: Vec<u8>,
}

impl BitmapPage {
    /// A fresh page with every valid block free.
    pub fn all_free(region: u32, valid_blocks: u32) -> Self {
        let mut page = BitmapPage {
            region,
            valid_blocks,
            bits: vec![0u8; (valid_blocks as usize).div_ceil(8)],
        };
        page.seal_trailing_bits();
        page
    }

    pub fn is_allocated(&self, index: u32) -> bool {
        debug_assert!(index < self.valid_blocks);
        self.bits[index as usize / 8] & (1 << (index % 8)) != 0
    }

    /// Sets a block's state; returns whether the bit changed.
    pub fn set_allocated(&mut self, index: u32, allocated: bool) -> bool {
        debug_assert!(index < self.valid_blocks);
        let byte = &mut self.bits[index as usize / 8];
        let mask = 1 << (index % 8);
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
        // Trailing bits are always 1 and outside the valid range.
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
        let expected_len = (self.valid_blocks as usize).div_ceil(8);
        if self.bits.len() != expected_len {
            return Err(FormatError::Invalid("bitmap byte length mismatch"));
        }
        let payload_len = 8 + self.bits.len();
        if payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow("region bitmap exceeds one page"));
        }
        let mut copy = self.clone();
        copy.seal_trailing_bits();

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.region);
        le::put_u32(&mut p[4..8], self.valid_blocks);
        p[8..8 + copy.bits.len()].copy_from_slice(&copy.bits);

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

    /// Decodes a page and returns it with the generation stamped in its
    /// header, so callers can verify it against the checkpoint's record.
    pub fn decode(block: &[u8]) -> Result<(BitmapPage, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::BITMAP)?;
        let p = header.payload(block);
        if p.len() < 8 {
            return Err(FormatError::Invalid("bitmap payload too short"));
        }
        let region = le::get_u32(&p[0..4]);
        let valid_blocks = le::get_u32(&p[4..8]);
        let byte_len = (valid_blocks as usize).div_ceil(8);
        if p.len() != 8 + byte_len {
            return Err(FormatError::Invalid("bitmap payload length mismatch"));
        }
        if region as u64 != header.owner {
            return Err(FormatError::Invalid("bitmap region does not match block owner"));
        }
        let page = BitmapPage { region, valid_blocks, bits: p[8..].to_vec() };
        // Trailing bits must be allocated; a zero there is corruption.
        let total_bits = page.bits.len() as u32 * 8;
        for index in valid_blocks..total_bits {
            if page.bits[index as usize / 8] & (1 << (index % 8)) == 0 {
                return Err(FormatError::Invalid("bitmap trailing bits not sealed"));
            }
        }
        Ok((page, header.generation))
    }
}
