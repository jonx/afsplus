//! Owned chain: an immutable run of linked blocks that belongs to exactly one
//! object and holds one opaque content, written whole by one commit and
//! replaced whole. The security descriptor container (ADR-101) is the first
//! kind; the layout is the same for every kind and only the block magic, the
//! content bound and the messages differ.
//!
//! Segment payload, after the block header (owner = object id):
//! format u32, version u16, reserved u16 (zero), total length u32, index u16,
//! count u16, next block u64, then the content bytes of this segment. Every
//! segment but the last is full; the last has `next` zero.
use alloc::vec;
use alloc::vec::Vec;

use crate::header::{BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, OBJECT_INVALID};

const FIXED_PAYLOAD: usize = 24;

/// What distinguishes one kind of owned chain from another: the block magic,
/// the bound on the content, and the refusal messages, which name the kind so
/// a report says which container is damaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainKind {
    pub block_type: u32,
    /// Bound on the content, so a corrupt reference cannot demand an
    /// unbounded chain walk.
    pub max_bytes: u32,
    /// Short name used in damage reports.
    pub label: &'static str,
    pub length_out_of_range: &'static str,
    pub owner_or_format_zero: &'static str,
    pub position_inconsistent: &'static str,
    pub link_inconsistent: &'static str,
    pub length_not_exact: &'static str,
    pub flags_nonzero: &'static str,
    pub payload_too_short: &'static str,
    pub reserved_nonzero: &'static str,
    pub tail_nonzero: &'static str,
}

/// Content bytes one segment holds at `block_size`.
pub fn segment_capacity(block_size: usize) -> usize {
    block_size.saturating_sub(HEADER_SIZE + FIXED_PAYLOAD)
}

/// Segments a content of `total_len` bytes occupies, or `None` when the
/// length is zero, above the bound, or the block cannot hold any byte.
pub fn segment_count(kind: &ChainKind, total_len: u32, block_size: usize) -> Option<u16> {
    let capacity = segment_capacity(block_size);
    if total_len == 0 || total_len > kind.max_bytes || capacity == 0 {
        return None;
    }
    u16::try_from((total_len as usize).div_ceil(capacity)).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainSegment<'a> {
    pub object_id: u64,
    pub format: u32,
    pub version: u16,
    pub total_len: u32,
    pub index: u16,
    pub count: u16,
    pub next: u64,
    pub bytes: &'a [u8],
}

impl<'a> ChainSegment<'a> {
    fn validate(&self, kind: &ChainKind, block_size: usize) -> Result<(), FormatError> {
        let capacity = segment_capacity(block_size);
        let count = segment_count(kind, self.total_len, block_size)
            .ok_or(FormatError::Invalid(kind.length_out_of_range))?;
        if self.object_id == OBJECT_INVALID || self.format == 0 {
            return Err(FormatError::Invalid(kind.owner_or_format_zero));
        }
        if self.count != count || self.index >= count {
            return Err(FormatError::Invalid(kind.position_inconsistent));
        }
        let last = self.index + 1 == count;
        if last != (self.next == 0) {
            return Err(FormatError::Invalid(kind.link_inconsistent));
        }
        let expected = if last {
            self.total_len as usize - capacity * (count as usize - 1)
        } else {
            capacity
        };
        if self.bytes.len() != expected {
            return Err(FormatError::Invalid(kind.length_not_exact));
        }
        Ok(())
    }

    pub fn encode(
        &self,
        kind: &ChainKind,
        block_size: usize,
        generation: u64,
    ) -> Result<Vec<u8>, FormatError> {
        self.validate(kind, block_size)?;
        let mut block = vec![0u8; block_size];
        let payload_len = FIXED_PAYLOAD + self.bytes.len();
        let p = &mut block[HEADER_SIZE..HEADER_SIZE + payload_len];
        le::put_u32(&mut p[0..4], self.format);
        le::put_u16(&mut p[4..6], self.version);
        le::put_u32(&mut p[8..12], self.total_len);
        le::put_u16(&mut p[12..14], self.index);
        le::put_u16(&mut p[14..16], self.count);
        le::put_u64(&mut p[16..24], self.next);
        p[FIXED_PAYLOAD..].copy_from_slice(self.bytes);
        BlockHeader {
            block_type: kind.block_type,
            flags: 0,
            owner: self.object_id,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    /// Decode one segment, returning it with its transaction generation.
    pub fn decode(kind: &ChainKind, block: &'a [u8]) -> Result<(Self, u64), FormatError> {
        let header = BlockHeader::verify(block, kind.block_type)?;
        let p = header.payload(block);
        if header.flags != 0 {
            return Err(FormatError::Invalid(kind.flags_nonzero));
        }
        if p.len() < FIXED_PAYLOAD {
            return Err(FormatError::Invalid(kind.payload_too_short));
        }
        if le::get_u16(&p[6..8]) != 0 {
            return Err(FormatError::Invalid(kind.reserved_nonzero));
        }
        if block[HEADER_SIZE + p.len()..].iter().any(|b| *b != 0) {
            return Err(FormatError::Invalid(kind.tail_nonzero));
        }
        let segment = ChainSegment {
            object_id: header.owner,
            format: le::get_u32(&p[0..4]),
            version: le::get_u16(&p[4..6]),
            total_len: le::get_u32(&p[8..12]),
            index: le::get_u16(&p[12..14]),
            count: le::get_u16(&p[14..16]),
            next: le::get_u64(&p[16..24]),
            bytes: &p[FIXED_PAYLOAD..],
        };
        segment.validate(kind, block.len())?;
        Ok((segment, header.generation))
    }
}
