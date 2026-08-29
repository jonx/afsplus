//! Core object record (`docs/04-object-model.md` §3).
//!
//! One object per block in the prototype; the header's owner field carries
//! the object ID so repair tooling can attribute the block without context.
//!
//! Payload layout after the common header:
//!
//! ```text
//! offset size field
//! 0      8    object ID
//! 8      1    object type
//! 9      1    reserved (zero)
//! 10     2    object flags
//! 12     4    link count
//! 16     8    logical size in bytes
//! 24     8    allocated size in bytes
//! 32     12   creation timestamp
//! 44     12   modification timestamp
//! 56     12   metadata-change timestamp
//! 68     4    AROS protection flags
//! 72     8    content generation
//! 80     8    data root LBA (directory: directory block; file: 0 = no data)
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, Timespec, OBJECT_INVALID};

const PAYLOAD_LEN: usize = 88;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    File,
    Directory,
    Symlink,
    Internal,
}

impl ObjectType {
    fn to_wire(self) -> u8 {
        match self {
            ObjectType::File => 1,
            ObjectType::Directory => 2,
            ObjectType::Symlink => 3,
            ObjectType::Internal => 4,
        }
    }

    fn from_wire(value: u8) -> Result<Self, FormatError> {
        match value {
            1 => Ok(ObjectType::File),
            2 => Ok(ObjectType::Directory),
            3 => Ok(ObjectType::Symlink),
            4 => Ok(ObjectType::Internal),
            _ => Err(FormatError::Invalid("unknown object type")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectRecord {
    pub object_id: u64,
    pub object_type: ObjectType,
    pub flags: u16,
    pub link_count: u32,
    pub size_bytes: u64,
    pub allocated_bytes: u64,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
    pub protection: u32,
    pub content_generation: u64,
    pub data_root: u64,
}

impl ObjectRecord {
    pub fn encode(&self, block_size: usize, transaction_generation: u64) -> Result<Vec<u8>, FormatError> {
        self.validate()?;
        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u64(&mut p[0..8], self.object_id);
        p[8] = self.object_type.to_wire();
        le::put_u16(&mut p[10..12], self.flags);
        le::put_u32(&mut p[12..16], self.link_count);
        le::put_u64(&mut p[16..24], self.size_bytes);
        le::put_u64(&mut p[24..32], self.allocated_bytes);
        self.created.write(&mut p[32..44]);
        self.modified.write(&mut p[44..56]);
        self.changed.write(&mut p[56..68]);
        le::put_u32(&mut p[68..72], self.protection);
        le::put_u64(&mut p[72..80], self.content_generation);
        le::put_u64(&mut p[80..88], self.data_root);

        BlockHeader {
            block_type: block_type::OBJECT,
            flags: 0,
            owner: self.object_id,
            generation: transaction_generation,
            payload_len: PAYLOAD_LEN as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<ObjectRecord, FormatError> {
        let header = BlockHeader::verify(block, block_type::OBJECT)?;
        let p = header.payload(block);
        if p.len() < PAYLOAD_LEN {
            return Err(FormatError::Invalid("object record payload too short"));
        }
        let record = ObjectRecord {
            object_id: le::get_u64(&p[0..8]),
            object_type: ObjectType::from_wire(p[8])?,
            flags: le::get_u16(&p[10..12]),
            link_count: le::get_u32(&p[12..16]),
            size_bytes: le::get_u64(&p[16..24]),
            allocated_bytes: le::get_u64(&p[24..32]),
            created: Timespec::read(&p[32..44])?,
            modified: Timespec::read(&p[44..56])?,
            changed: Timespec::read(&p[56..68])?,
            protection: le::get_u32(&p[68..72]),
            content_generation: le::get_u64(&p[72..80]),
            data_root: le::get_u64(&p[80..88]),
        };
        if record.object_id != header.owner {
            return Err(FormatError::Invalid("object ID does not match block owner"));
        }
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), FormatError> {
        if self.object_id == OBJECT_INVALID {
            return Err(FormatError::Invalid("object ID zero is invalid"));
        }
        match self.object_type {
            ObjectType::Directory => {
                if self.data_root == 0 {
                    return Err(FormatError::Invalid("directory must reference a directory block"));
                }
                if self.size_bytes != 0 {
                    return Err(FormatError::Invalid("directory logical size must be zero"));
                }
            }
            ObjectType::File => {
                // Prototype: files carry no data extents yet.
                if self.data_root != 0 || self.size_bytes != 0 {
                    return Err(FormatError::Invalid("prototype files must be empty"));
                }
            }
            ObjectType::Symlink | ObjectType::Internal => {
                return Err(FormatError::Invalid("object type not implemented in prototype"));
            }
        }
        Ok(())
    }
}
