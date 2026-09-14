//! Transitional single-block retired list. Superseded by the reclaim queue
//! (ADR-036, `reclaim`); newly formatted volumes do not reference this
//! codec. Kept as legacy format/test coverage only.
//!
//! Retired-block list (ADR-021, `docs/23-pfs3-stage0-review.md` §4).
//!
//! Blocks that became unreachable in a transaction are *retired*, not free:
//! their bitmap bit stays allocated and they are listed here with the
//! generation that retired them. A retired block becomes allocatable only in
//! a later transaction, once no still-selectable checkpoint can reference it
//! (with two checkpoint slots and newest-valid selection, that is the next
//! transaction). On any uncertainty the rule is to keep quarantining, never
//! to reuse early.
//!
//! The list itself is ordinary COW metadata referenced by the checkpoint.
//! Single block in the prototype; overflow is a reported limit (deferred
//! reclamation batching comes with the real implementation).
//!
//! Payload layout after the common header:
//!
//! ```text
//! offset size field
//! 0      4    entry count
//! 4      4    reserved (zero)
//! 8      ...  entries: lba (8), retire generation (8), sorted by lba
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError};

const ENTRY_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredEntry {
    pub lba: u64,
    pub retire_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetiredList {
    pub entries: Vec<RetiredEntry>,
}

impl RetiredList {
    pub fn insert(&mut self, lba: u64, retire_generation: u64) -> Result<(), FormatError> {
        match self.entries.binary_search_by_key(&lba, |e| e.lba) {
            Ok(_) => Err(FormatError::Invalid("block retired twice")),
            Err(pos) => {
                self.entries.insert(
                    pos,
                    RetiredEntry {
                        lba,
                        retire_generation,
                    },
                );
                Ok(())
            }
        }
    }

    pub fn contains(&self, lba: u64) -> bool {
        self.entries.binary_search_by_key(&lba, |e| e.lba).is_ok()
    }

    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        let payload_len = 8 + self.entries.len() * ENTRY_SIZE;
        if block_size < HEADER_SIZE || payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow(
                "retired list exceeds one block (prototype limit)",
            ));
        }
        validate_entries(&self.entries)?;

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.entries.len() as u32);
        for (i, entry) in self.entries.iter().enumerate() {
            let offset = 8 + i * ENTRY_SIZE;
            le::put_u64(&mut p[offset..offset + 8], entry.lba);
            le::put_u64(&mut p[offset + 8..offset + 16], entry.retire_generation);
        }

        BlockHeader {
            block_type: block_type::RETIRED,
            flags: 0,
            owner: 0,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<RetiredList, FormatError> {
        let header = BlockHeader::verify(block, block_type::RETIRED)?;
        let p = header.payload(block);
        if p.len() < 8 {
            return Err(FormatError::Invalid("retired list payload too short"));
        }
        let count = le::get_u32(&p[0..4]) as usize;
        if count > (p.len() - 8) / ENTRY_SIZE {
            return Err(FormatError::Invalid("retired list count exceeds payload"));
        }
        if header.payload_len as usize != 8 + count * ENTRY_SIZE {
            return Err(FormatError::Invalid("retired list payload length mismatch"));
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let offset = 8 + i * ENTRY_SIZE;
            entries.push(RetiredEntry {
                lba: le::get_u64(&p[offset..offset + 8]),
                retire_generation: le::get_u64(&p[offset + 8..offset + 16]),
            });
        }
        validate_entries(&entries)?;
        Ok(RetiredList { entries })
    }
}

fn validate_entries(entries: &[RetiredEntry]) -> Result<(), FormatError> {
    for entry in entries {
        if entry.retire_generation == 0 {
            return Err(FormatError::Invalid("retired entry with zero generation"));
        }
    }
    for pair in entries.windows(2) {
        if pair[0].lba >= pair[1].lba {
            return Err(FormatError::Invalid("retired entries not strictly ordered"));
        }
    }
    Ok(())
}
