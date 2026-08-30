//! Object map block: object ID -> object record LBA.
//!
//! The prototype's stand-in for the object tree root referenced by the
//! checkpoint. A single sorted block; the real structure becomes a B+ tree.
//! Exceeding one block is a reported prototype limit.
//!
//! Payload layout after the common header:
//!
//! ```text
//! offset size field
//! 0      4    entry count
//! 4      4    reserved (zero)
//! 8      ...  entries: object ID (8), record LBA (8), sorted by object ID
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, OBJECT_INVALID};

const ENTRY_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OmapEntry {
    pub object_id: u64,
    pub block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObjectMap {
    pub entries: Vec<OmapEntry>,
}

impl ObjectMap {
    pub fn lookup(&self, object_id: u64) -> Option<u64> {
        self.entries
            .binary_search_by_key(&object_id, |e| e.object_id)
            .ok()
            .map(|pos| self.entries[pos].block)
    }

    /// Inserts a new mapping or updates an existing one, keeping ID order.
    pub fn upsert(&mut self, object_id: u64, block: u64) -> Result<(), FormatError> {
        if object_id == OBJECT_INVALID {
            return Err(FormatError::Invalid("object ID zero is invalid"));
        }
        match self
            .entries
            .binary_search_by_key(&object_id, |e| e.object_id)
        {
            Ok(pos) => self.entries[pos].block = block,
            Err(pos) => self.entries.insert(pos, OmapEntry { object_id, block }),
        }
        Ok(())
    }

    /// Removes a mapping; returns the record block it pointed at.
    pub fn remove(&mut self, object_id: u64) -> Option<u64> {
        self.entries
            .binary_search_by_key(&object_id, |e| e.object_id)
            .ok()
            .map(|pos| self.entries.remove(pos).block)
    }

    pub fn encode(
        &self,
        block_size: usize,
        transaction_generation: u64,
    ) -> Result<Vec<u8>, FormatError> {
        let payload_len = 8 + self.entries.len() * ENTRY_SIZE;
        if payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow(
                "object map exceeds one block (prototype limit)",
            ));
        }
        validate_entries(&self.entries)?;

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.entries.len() as u32);
        for (i, entry) in self.entries.iter().enumerate() {
            let offset = 8 + i * ENTRY_SIZE;
            le::put_u64(&mut p[offset..offset + 8], entry.object_id);
            le::put_u64(&mut p[offset + 8..offset + 16], entry.block);
        }

        BlockHeader {
            block_type: block_type::OBJECT_MAP,
            flags: 0,
            owner: 0,
            generation: transaction_generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<ObjectMap, FormatError> {
        let header = BlockHeader::verify(block, block_type::OBJECT_MAP)?;
        let p = header.payload(block);
        if p.len() < 8 {
            return Err(FormatError::Invalid("object map payload too short"));
        }
        let count = le::get_u32(&p[0..4]) as usize;
        if count > (p.len() - 8) / ENTRY_SIZE {
            return Err(FormatError::Invalid(
                "object map entry count exceeds payload",
            ));
        }
        if header.payload_len as usize != 8 + count * ENTRY_SIZE {
            return Err(FormatError::Invalid("object map payload length mismatch"));
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let offset = 8 + i * ENTRY_SIZE;
            entries.push(OmapEntry {
                object_id: le::get_u64(&p[offset..offset + 8]),
                block: le::get_u64(&p[offset + 8..offset + 16]),
            });
        }
        validate_entries(&entries)?;
        Ok(ObjectMap { entries })
    }
}

fn validate_entries(entries: &[OmapEntry]) -> Result<(), FormatError> {
    for entry in entries {
        if entry.object_id == OBJECT_INVALID {
            return Err(FormatError::Invalid(
                "object map contains invalid object ID",
            ));
        }
    }
    for pair in entries.windows(2) {
        if pair[0].object_id >= pair[1].object_id {
            return Err(FormatError::Invalid("object map IDs not strictly ordered"));
        }
    }
    Ok(())
}
