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

use alloc::vec::Vec;

use crate::chain::{self, ChainKind, ChainSegment};
use crate::header::block_type;
use crate::FormatError;

/// Bound on one descriptor, so a corrupt reference cannot demand an
/// unbounded chain walk. 64 KiB is the largest Windows security descriptor.
pub const MAX_SECURITY_DESCRIPTOR_BYTES: u32 = 65_536;

/// The descriptor chain as an owned chain: "AFSX" blocks.
pub const SECURITY_CHAIN: ChainKind = ChainKind {
    block_type: block_type::SECURITY_DESCRIPTOR,
    max_bytes: MAX_SECURITY_DESCRIPTOR_BYTES,
    label: "security",
    length_out_of_range: "security descriptor length out of range",
    owner_or_format_zero: "security segment owner or format is zero",
    position_inconsistent: "security segment position inconsistent",
    link_inconsistent: "security segment chain link inconsistent",
    length_not_exact: "security segment length is not exact",
    flags_nonzero: "security segment header flags are nonzero",
    payload_too_short: "security segment payload too short",
    reserved_nonzero: "security segment reserved field is nonzero",
};

/// Descriptor bytes one segment holds at `block_size`.
pub fn segment_capacity(block_size: usize) -> usize {
    chain::segment_capacity(block_size)
}

/// Segments a descriptor of `total_len` bytes occupies, or `None` when the
/// length is zero, above the bound, or the block cannot hold any byte.
pub fn segment_count(total_len: u32, block_size: usize) -> Option<u16> {
    chain::segment_count(&SECURITY_CHAIN, total_len, block_size)
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
    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        ChainSegment {
            object_id: self.object_id,
            format: self.format,
            version: self.version,
            total_len: self.total_len,
            index: self.index,
            count: self.count,
            next: self.next,
            bytes: self.bytes,
        }
        .encode(&SECURITY_CHAIN, block_size, generation)
    }

    /// Decode one segment, returning it with its transaction generation.
    pub fn decode(block: &'a [u8]) -> Result<(Self, u64), FormatError> {
        let (s, generation) = ChainSegment::decode(&SECURITY_CHAIN, block)?;
        Ok((
            SecuritySegment {
                object_id: s.object_id,
                format: s.format,
                version: s.version,
                total_len: s.total_len,
                index: s.index,
                count: s.count,
                next: s.next,
                bytes: s.bytes,
            },
            generation,
        ))
    }
}
