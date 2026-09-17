//! Security descriptor segment (`"AFSX"`): the preservation container for
//! rich security metadata.
//!
//! A descriptor is an opaque byte string tagged with a format identity and a
//! format version. The filesystem stores, returns and removes it; it never
//! interprets the bytes. One descriptor occupies a chain of one or more
//! segments owned by exactly one object (the header's owner field carries the
//! object ID). Segments are immutable: replacing a descriptor writes a new
//! chain and retires the old one.
//!
//! Payload layout after the common header:
//!
//! ```text
//! offset size field
//! 0      4    descriptor format identity (nonzero)
//! 4      2    descriptor format version
//! 6      2    reserved (zero)
//! 8      4    total descriptor length in bytes, across the chain
//! 12     2    segment index (0-based)
//! 14     2    segment count
//! 16     8    next segment LBA (0 on the last segment)
//! 24     n    descriptor bytes of this segment
//! ```
//!
//! Every segment except the last is full. Admission is exact: zero header
//! flags, `payload_len == 24 + n`, zero tail.

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, OBJECT_INVALID};

const FIXED_PAYLOAD: usize = 24;

/// Bound on one descriptor, so a corrupt reference cannot demand an
/// unbounded chain walk. 64 KiB is the largest Windows security descriptor.
pub const MAX_SECURITY_DESCRIPTOR_BYTES: u32 = 65_536;

/// Descriptor bytes one segment holds at `block_size`.
pub fn segment_capacity(block_size: usize) -> usize {
    block_size.saturating_sub(HEADER_SIZE + FIXED_PAYLOAD)
}

/// Segments a descriptor of `total_len` bytes occupies, or `None` when the
/// length is zero, above the bound, or the block cannot hold any byte.
pub fn segment_count(total_len: u32, block_size: usize) -> Option<u16> {
    let capacity = segment_capacity(block_size);
    if total_len == 0 || total_len > MAX_SECURITY_DESCRIPTOR_BYTES || capacity == 0 {
        return None;
    }
    u16::try_from((total_len as usize).div_ceil(capacity)).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecuritySegment<'a> {
    pub object_id: u64,
    pub format: u32,
    pub version: u16,
    pub total_len: u32,
    pub index: u16,
    pub count: u16,
    pub next: u64,
    pub bytes: &'a [u8],
}

impl<'a> SecuritySegment<'a> {
    fn validate(&self, block_size: usize) -> Result<(), FormatError> {
        let capacity = segment_capacity(block_size);
        let count = segment_count(self.total_len, block_size).ok_or(FormatError::Invalid(
            "security descriptor length out of range",
        ))?;
        if self.object_id == OBJECT_INVALID || self.format == 0 {
            return Err(FormatError::Invalid(
                "security segment owner or format is zero",
            ));
        }
        if self.count != count || self.index >= count {
            return Err(FormatError::Invalid(
                "security segment position inconsistent",
            ));
        }
        let last = self.index + 1 == count;
        if last != (self.next == 0) {
            return Err(FormatError::Invalid(
                "security segment chain link inconsistent",
            ));
        }
        let expected = if last {
            self.total_len as usize - capacity * (count as usize - 1)
        } else {
            capacity
        };
        if self.bytes.len() != expected {
            return Err(FormatError::Invalid("security segment length is not exact"));
        }
        Ok(())
    }

    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        self.validate(block_size)?;
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
            block_type: block_type::SECURITY_DESCRIPTOR,
            flags: 0,
            owner: self.object_id,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    /// Decode one segment, returning it with its transaction generation.
    pub fn decode(block: &'a [u8]) -> Result<(Self, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::SECURITY_DESCRIPTOR)?;
        let p = header.payload(block);
        if header.flags != 0 {
            return Err(FormatError::Invalid(
                "security segment header flags are nonzero",
            ));
        }
        if p.len() < FIXED_PAYLOAD {
            return Err(FormatError::Invalid("security segment payload too short"));
        }
        if le::get_u16(&p[6..8]) != 0 {
            return Err(FormatError::Invalid(
                "security segment reserved field is nonzero",
            ));
        }
        if block[HEADER_SIZE + p.len()..].iter().any(|b| *b != 0) {
            return Err(FormatError::Invalid(
                "security segment unused tail is nonzero",
            ));
        }
        let segment = SecuritySegment {
            object_id: header.owner,
            format: le::get_u32(&p[0..4]),
            version: le::get_u16(&p[4..6]),
            total_len: le::get_u32(&p[8..12]),
            index: le::get_u16(&p[12..14]),
            count: le::get_u16(&p[14..16]),
            next: le::get_u64(&p[16..24]),
            bytes: &p[FIXED_PAYLOAD..],
        };
        segment.validate(block.len())?;
        Ok((segment, header.generation))
    }
}
