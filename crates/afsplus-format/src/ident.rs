//! Identification block (`docs/03-on-disk-format.md` §5).
//!
//! Fixed, checksummed record at a deterministic location (prototype: LBA 0)
//! that lets external tools recognize AFS+ without parsing arbitrary
//! metadata. It is written once by the formatter and is immutable afterwards,
//! so it is never exposed to torn rewrites; mutable committed state
//! (clean/dirty, generations) lives in the checkpoints.
//!
//! Payload layout after the 32-byte common header:
//!
//! ```text
//! offset size field
//! 0      8    filesystem magic ("AFSPLUS1")
//! 8      4    format epoch
//! 12     4    identification record version
//! 16     16   filesystem UUID
//! 32     1    logical block shift
//! 33     1    checksum algorithm identifier
//! 34     6    reserved (zero)
//! 40     8    total logical blocks
//! 48     8    checkpoint slot A LBA
//! 56     8    checkpoint slot B LBA
//! 64     8    first metadata-area LBA
//! 72     1    label length in bytes
//! 73     64   label (UTF-8, zero padded)
//! ```

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::crc32c::CHECKSUM_CRC32C;
use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, DEFAULT_BLOCK_SHIFT, FORMAT_EPOCH, FS_MAGIC};

pub const IDENT_VERSION: u32 = 1;
pub const LABEL_MAX_BYTES: usize = 64;

const PAYLOAD_LEN: usize = 73 + LABEL_MAX_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identification {
    pub uuid: [u8; 16],
    pub block_shift: u8,
    pub checksum_algorithm: u8,
    pub total_blocks: u64,
    pub checkpoint_slots: [u64; 2],
    pub metadata_start: u64,
    pub label: String,
}

impl Identification {
    pub fn block_size(&self) -> usize {
        1usize << self.block_shift
    }

    pub fn encode(&self, block_size: usize) -> Result<Vec<u8>, FormatError> {
        if self.block_shift != DEFAULT_BLOCK_SHIFT {
            return Err(FormatError::Invalid("prototype supports only 4 KiB blocks"));
        }
        if block_size != self.block_size() {
            return Err(FormatError::WrongBufferSize { expected: self.block_size(), actual: block_size });
        }
        let label = self.label.as_bytes();
        if label.len() > LABEL_MAX_BYTES {
            return Err(FormatError::Overflow("volume label"));
        }
        self.validate_geometry()?;

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u64(&mut p[0..8], FS_MAGIC);
        le::put_u32(&mut p[8..12], FORMAT_EPOCH);
        le::put_u32(&mut p[12..16], IDENT_VERSION);
        p[16..32].copy_from_slice(&self.uuid);
        p[32] = self.block_shift;
        p[33] = self.checksum_algorithm;
        le::put_u64(&mut p[40..48], self.total_blocks);
        le::put_u64(&mut p[48..56], self.checkpoint_slots[0]);
        le::put_u64(&mut p[56..64], self.checkpoint_slots[1]);
        le::put_u64(&mut p[64..72], self.metadata_start);
        p[72] = label.len() as u8;
        p[73..73 + label.len()].copy_from_slice(label);

        BlockHeader {
            block_type: block_type::IDENTIFICATION,
            flags: 0,
            owner: 0,
            generation: 0,
            payload_len: PAYLOAD_LEN as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<Identification, FormatError> {
        let header = BlockHeader::verify(block, block_type::IDENTIFICATION)?;
        let p = header.payload(block);
        if p.len() < PAYLOAD_LEN {
            return Err(FormatError::Invalid("identification payload too short"));
        }
        if le::get_u64(&p[0..8]) != FS_MAGIC {
            return Err(FormatError::Invalid("filesystem magic mismatch"));
        }
        let epoch = le::get_u32(&p[8..12]);
        if epoch != FORMAT_EPOCH {
            return Err(FormatError::Invalid("unsupported format epoch"));
        }
        let version = le::get_u32(&p[12..16]);
        if version != IDENT_VERSION {
            return Err(FormatError::Invalid("unsupported identification version"));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&p[16..32]);
        let block_shift = p[32];
        if block_shift != DEFAULT_BLOCK_SHIFT {
            return Err(FormatError::Invalid("prototype supports only 4 KiB blocks"));
        }
        let checksum_algorithm = p[33];
        if checksum_algorithm != CHECKSUM_CRC32C {
            return Err(FormatError::Invalid("unsupported checksum algorithm"));
        }
        let label_len = p[72] as usize;
        if label_len > LABEL_MAX_BYTES {
            return Err(FormatError::Invalid("label length out of range"));
        }
        let label = core::str::from_utf8(&p[73..73 + label_len])
            .map_err(|_| FormatError::InvalidUtf8)?;

        let ident = Identification {
            uuid,
            block_shift,
            checksum_algorithm,
            total_blocks: le::get_u64(&p[40..48]),
            checkpoint_slots: [le::get_u64(&p[48..56]), le::get_u64(&p[56..64])],
            metadata_start: le::get_u64(&p[64..72]),
            label: String::from(label),
        };
        ident.validate_geometry()?;
        Ok(ident)
    }

    /// Bounds-first geometry validation shared by encode and decode.
    fn validate_geometry(&self) -> Result<(), FormatError> {
        if self.total_blocks == 0 {
            return Err(FormatError::Invalid("total_blocks is zero"));
        }
        let [a, b] = self.checkpoint_slots;
        if a == b {
            return Err(FormatError::Invalid("checkpoint slots must differ"));
        }
        for slot in [a, b] {
            if slot >= self.total_blocks {
                return Err(FormatError::Invalid("checkpoint slot out of volume bounds"));
            }
            if slot >= self.metadata_start {
                return Err(FormatError::Invalid("checkpoint slot overlaps metadata area"));
            }
        }
        if self.metadata_start >= self.total_blocks {
            return Err(FormatError::Invalid("metadata area start out of volume bounds"));
        }
        Ok(())
    }
}
