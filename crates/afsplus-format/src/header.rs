//! Common metadata block header (`docs/03-on-disk-format.md` §7).
//!
//! Every independently addressable metadata block begins with this header so
//! repair tools can classify blocks without external context. The CRC32C
//! covers the whole block with the checksum field itself zeroed.
//!
//! Wire layout (32 bytes):
//!
//! ```text
//! offset size field
//! 0      4    block type magic
//! 4      2    header version
//! 6      2    flags
//! 8      8    owner/object identifier (0 where not applicable)
//! 16     8    transaction generation
//! 24     4    payload length (bytes following the header)
//! 28     4    CRC32C of the whole block, checksum field zeroed
//! ```

use crate::{crc32c::crc32c, le, FormatError};

pub const HEADER_SIZE: usize = 32;
pub const HEADER_VERSION: u16 = 1;

const CHECKSUM_OFFSET: usize = 28;

/// Block type magics, readable as ASCII in hex dumps.
pub mod block_type {
    /// Identification block, `"AFSI"`.
    pub const IDENTIFICATION: u32 = u32::from_le_bytes(*b"AFSI");
    /// Checkpoint record, `"AFSC"`.
    pub const CHECKPOINT: u32 = u32::from_le_bytes(*b"AFSC");
    /// Object record, `"AFSO"`.
    pub const OBJECT: u32 = u32::from_le_bytes(*b"AFSO");
    /// Directory block, `"AFSD"`.
    pub const DIRECTORY: u32 = u32::from_le_bytes(*b"AFSD");
    /// Object map block, `"AFSM"`.
    pub const OBJECT_MAP: u32 = u32::from_le_bytes(*b"AFSM");
    /// Region bitmap page, `"AFSB"`.
    pub const BITMAP: u32 = u32::from_le_bytes(*b"AFSB");
    /// Region allocation descriptor, `"AFSG"`.
    pub const REGION_DESCRIPTOR: u32 = u32::from_le_bytes(*b"AFSG");
    /// Shared COW B+ tree node, `"AFST"`.
    pub const TREE_NODE: u32 = u32::from_le_bytes(*b"AFST");
    /// Retired-block list, `"AFSR"` (transitional; replaced by the reclaim queue).
    pub const RETIRED: u32 = u32::from_le_bytes(*b"AFSR");
    /// Reclaim-queue root, `"AFSH"`.
    pub const RECLAIM_ROOT: u32 = u32::from_le_bytes(*b"AFSH");
    /// Sealed reclaim segment, `"AFSS"`.
    pub const RECLAIM_SEGMENT: u32 = u32::from_le_bytes(*b"AFSS");
    /// Sealed reclaim table, `"AFSL"`.
    pub const RECLAIM_TABLE: u32 = u32::from_le_bytes(*b"AFSL");
    /// Security descriptor segment, `"AFSX"`.
    pub const SECURITY_DESCRIPTOR: u32 = u32::from_le_bytes(*b"AFSX");
    /// Intent-log record, `"AFSJ"`.
    pub const INTENT_LOG: u32 = u32::from_le_bytes(*b"AFSJ");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    pub block_type: u32,
    pub flags: u16,
    pub owner: u64,
    pub generation: u64,
    pub payload_len: u32,
}

impl BlockHeader {
    /// Writes the header and seals the block: the payload must already be in
    /// place in `block[HEADER_SIZE..]`, and everything after the payload must
    /// have been zeroed by the caller (blocks are always encoded into
    /// zero-initialized buffers so unused space never leaks stale bytes).
    pub fn seal(&self, block: &mut [u8]) {
        debug_assert!(block.len() >= HEADER_SIZE + self.payload_len as usize);
        le::put_u32(&mut block[0..4], self.block_type);
        le::put_u16(&mut block[4..6], HEADER_VERSION);
        le::put_u16(&mut block[6..8], self.flags);
        le::put_u64(&mut block[8..16], self.owner);
        le::put_u64(&mut block[16..24], self.generation);
        le::put_u32(&mut block[24..28], self.payload_len);
        le::put_u32(&mut block[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4], 0);
        let sum = crc32c(block);
        le::put_u32(&mut block[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4], sum);
    }

    /// Validates and reads the header of `block`, verifying the checksum over
    /// the entire block first. A checksum mismatch is an integrity failure and
    /// the block must not be interpreted further.
    pub fn verify(block: &[u8], expected_type: u32) -> Result<BlockHeader, FormatError> {
        if block.len() < HEADER_SIZE {
            return Err(FormatError::WrongBufferSize {
                expected: HEADER_SIZE,
                actual: block.len(),
            });
        }
        // Checksum first: no field of a corrupted block is trustworthy.
        let stored = le::get_u32(&block[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4]);
        let mut hasher = crate::crc32c::Hasher::new();
        hasher.update(&block[..CHECKSUM_OFFSET]);
        hasher.update(&[0u8; 4]);
        hasher.update(&block[CHECKSUM_OFFSET + 4..]);
        let computed = hasher.finalize();
        if stored != computed {
            return Err(FormatError::ChecksumMismatch { stored, computed });
        }

        let block_type = le::get_u32(&block[0..4]);
        if block_type != expected_type {
            return Err(FormatError::WrongBlockType {
                expected: expected_type,
                actual: block_type,
            });
        }
        let version = le::get_u16(&block[4..6]);
        if version != HEADER_VERSION {
            return Err(FormatError::UnsupportedHeaderVersion(version));
        }
        let payload_len = le::get_u32(&block[24..28]);
        let max = block.len() - HEADER_SIZE;
        if payload_len as usize > max {
            return Err(FormatError::PayloadTooLarge { payload_len, max });
        }
        Ok(BlockHeader {
            block_type,
            flags: le::get_u16(&block[6..8]),
            owner: le::get_u64(&block[8..16]),
            generation: le::get_u64(&block[16..24]),
            payload_len,
        })
    }

    /// Returns the payload slice for a verified header.
    pub fn payload<'a>(&self, block: &'a [u8]) -> &'a [u8] {
        &block[HEADER_SIZE..HEADER_SIZE + self.payload_len as usize]
    }
}
