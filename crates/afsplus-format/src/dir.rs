//! Legacy directory block (`docs/05-directories-and-names.md`).
//!
//! This codec preserves the pre-AFST prototype format for transition tests.
//! Newly formatted volumes use typed leaves in the shared COW tree and never
//! reference this one-block representation. Exceeding one legacy block is a
//! reported limit, never silent truncation.
//!
//! Each entry carries both the normalized comparison key and the original
//! UTF-8 name, per the accepted normalization-preserving correction: original
//! bytes are what readers get back; the key is only for lookup and ordering.
//! This retired codec's key encoding remains identity and is used only by
//! legacy transition tests. Current typed directory trees derive their key
//! from the versioned policy in the identification block.
//!
//! Entries are ordered by byte-wise comparison of their keys — never host
//! locale collation.
//!
//! Payload layout after the common header (header.owner = directory object ID):
//!
//! ```text
//! offset size field
//! 0      4    entry count
//! 4      4    reserved (zero)
//! 8      ...  entries
//! ```
//!
//! Entry layout (unaligned, explicitly decoded):
//!
//! ```text
//! offset size field
//! 0      2    key length in bytes
//! 2      2    name length in bytes
//! 4      1    child type hint (object type wire value)
//! 5      3    reserved (zero)
//! 8      8    child object ID
//! 16     key  normalized comparison key
//! ...    name original UTF-8 name
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, validate_name, FormatError, NAME_MAX_UTF8_BYTES, OBJECT_INVALID};

const ENTRY_FIXED: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub key: Vec<u8>,
    pub name: Vec<u8>,
    pub child_type_hint: u8,
    pub child_id: u64,
}

/// Computes the normalized comparison key for a name.
///
/// Legacy identity mapping used only by the retired one-block codec.
///
/// Current typed directory trees use the volume's pinned Unicode
/// normalization/casefold algorithm instead (ADR-052).
pub fn comparison_key(name: &[u8]) -> Vec<u8> {
    name.to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirBlock {
    pub owner: u64,
    pub entries: Vec<DirEntry>,
}

impl DirBlock {
    pub fn new(owner: u64) -> Self {
        DirBlock {
            owner,
            entries: Vec::new(),
        }
    }

    /// Inserts an entry keeping key order. Fails on duplicate keys.
    pub fn insert(&mut self, entry: DirEntry) -> Result<(), FormatError> {
        match self
            .entries
            .binary_search_by(|e| e.key.as_slice().cmp(&entry.key))
        {
            Ok(_) => Err(FormatError::Invalid("duplicate directory key")),
            Err(pos) => {
                self.entries.insert(pos, entry);
                Ok(())
            }
        }
    }

    /// Removes the entry with this key; returns it, or None if absent.
    pub fn remove(&mut self, key: &[u8]) -> Option<DirEntry> {
        self.entries
            .binary_search_by(|e| e.key.as_slice().cmp(key))
            .ok()
            .map(|pos| self.entries.remove(pos))
    }

    pub fn lookup(&self, key: &[u8]) -> Option<&DirEntry> {
        self.entries
            .binary_search_by(|e| e.key.as_slice().cmp(key))
            .ok()
            .map(|pos| &self.entries[pos])
    }

    pub fn encode(
        &self,
        block_size: usize,
        transaction_generation: u64,
    ) -> Result<Vec<u8>, FormatError> {
        let mut payload_len = 8usize;
        for entry in &self.entries {
            validate_entry(entry)?;
            payload_len += ENTRY_FIXED + entry.key.len() + entry.name.len();
        }
        if block_size < HEADER_SIZE || payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow(
                "directory exceeds one block (prototype limit)",
            ));
        }
        validate_ordering(&self.entries)?;

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.entries.len() as u32);
        let mut offset = 8usize;
        for entry in &self.entries {
            le::put_u16(&mut p[offset..offset + 2], entry.key.len() as u16);
            le::put_u16(&mut p[offset + 2..offset + 4], entry.name.len() as u16);
            p[offset + 4] = entry.child_type_hint;
            le::put_u64(&mut p[offset + 8..offset + 16], entry.child_id);
            offset += ENTRY_FIXED;
            p[offset..offset + entry.key.len()].copy_from_slice(&entry.key);
            offset += entry.key.len();
            p[offset..offset + entry.name.len()].copy_from_slice(&entry.name);
            offset += entry.name.len();
        }

        BlockHeader {
            block_type: block_type::DIRECTORY,
            flags: 0,
            owner: self.owner,
            generation: transaction_generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<DirBlock, FormatError> {
        let header = BlockHeader::verify(block, block_type::DIRECTORY)?;
        let p = header.payload(block);
        if p.len() < 8 {
            return Err(FormatError::Invalid("directory payload too short"));
        }
        if p[4..8].iter().any(|&byte| byte != 0) {
            return Err(FormatError::Invalid("legacy reserved bytes are nonzero"));
        }
        let count = le::get_u32(&p[0..4]) as usize;
        // Bounds-first: each entry needs at least its fixed part.
        if count > (p.len() - 8) / ENTRY_FIXED {
            return Err(FormatError::Invalid(
                "directory entry count exceeds payload",
            ));
        }
        let mut entries = Vec::with_capacity(count);
        let mut offset = 8usize;
        for _ in 0..count {
            if p.len() - offset < ENTRY_FIXED {
                return Err(FormatError::Invalid("truncated directory entry"));
            }
            if p[offset + 5..offset + 8].iter().any(|&byte| byte != 0) {
                return Err(FormatError::Invalid(
                    "directory entry reserved bytes are nonzero",
                ));
            }
            let key_len = le::get_u16(&p[offset..offset + 2]) as usize;
            let name_len = le::get_u16(&p[offset + 2..offset + 4]) as usize;
            let child_type_hint = p[offset + 4];
            let child_id = le::get_u64(&p[offset + 8..offset + 16]);
            offset += ENTRY_FIXED;
            if key_len > p.len() - offset || name_len > p.len() - offset - key_len {
                return Err(FormatError::Invalid(
                    "directory entry lengths exceed payload",
                ));
            }
            let key = p[offset..offset + key_len].to_vec();
            offset += key_len;
            let name = p[offset..offset + name_len].to_vec();
            offset += name_len;
            let entry = DirEntry {
                key,
                name,
                child_type_hint,
                child_id,
            };
            validate_entry(&entry)?;
            entries.push(entry);
        }
        // Trailing garbage inside the declared payload is a format error.
        if offset != header.payload_len as usize {
            return Err(FormatError::Invalid("directory payload length mismatch"));
        }
        validate_ordering(&entries)?;
        Ok(DirBlock {
            owner: header.owner,
            entries,
        })
    }
}

fn validate_entry(entry: &DirEntry) -> Result<(), FormatError> {
    validate_name(&entry.name)?;
    if entry.key.is_empty() || entry.key.len() > NAME_MAX_UTF8_BYTES * 4 {
        return Err(FormatError::Invalid("directory key length out of range"));
    }
    if entry.child_id == OBJECT_INVALID {
        return Err(FormatError::Invalid(
            "directory entry references invalid object",
        ));
    }
    Ok(())
}

fn validate_ordering(entries: &[DirEntry]) -> Result<(), FormatError> {
    for pair in entries.windows(2) {
        if pair[0].key >= pair[1].key {
            return Err(FormatError::Invalid("directory keys not strictly ordered"));
        }
    }
    Ok(())
}
