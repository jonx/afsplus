//! Directory entries (`docs/05-directories-and-names.md`).
//!
//! A directory is a typed COW tree (`"AFST"` nodes of kind `Directory`,
//! owner = the directory's object ID). This module holds the entry the core
//! puts in a leaf and the codec of its value; the tree itself is `tree.rs`.
//!
//! An entry carries both the comparison key, which is the item's key and is
//! derived from the volume's pinned normalization policy, and the original
//! UTF-8 name, which is what a reader gets back. Keys order entries by
//! byte-wise comparison, never by host locale collation.
//!
//! The one-block prototype directory of the first milestones is retired
//! (ADR-115); no volume ever wrote one outside its own tests.

use alloc::vec;
use alloc::vec::Vec;

use crate::{le, validate_name, FormatError, OBJECT_INVALID};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub key: Vec<u8>,
    pub name: Vec<u8>,
    pub child_type_hint: u8,
    pub child_id: u64,
}

/// Fixed bytes of a directory-tree leaf value, before the name.
pub const TREE_ENTRY_FIXED: usize = 16;

/// Leaf value of a typed directory tree (`AFST` nodes of kind `Directory`,
/// owner = the directory's object ID). The key of the item is the comparison
/// key of the name; the value is:
///
/// ```text
/// offset size field
/// 0      2    name length in bytes
/// 2      1    child type hint (1 file, 2 directory, 3 symlink)
/// 3      5    reserved (zero)
/// 8      8    child object ID (nonzero)
/// 16     n    original UTF-8 name, exactly `name length` bytes
/// ```
///
/// This codec checks the shape of one value. Whether the key is the
/// comparison key of the name under the volume's algorithm is a contextual
/// check of the reader.
pub fn encode_tree_entry_value(entry: &DirEntry) -> Result<Vec<u8>, FormatError> {
    validate_tree_entry(entry)?;
    let mut value = vec![0u8; TREE_ENTRY_FIXED + entry.name.len()];
    le::put_u16(&mut value[0..2], entry.name.len() as u16);
    value[2] = entry.child_type_hint;
    le::put_u64(&mut value[8..16], entry.child_id);
    value[TREE_ENTRY_FIXED..].copy_from_slice(&entry.name);
    Ok(value)
}

/// Decode one leaf value stored under `key`.
pub fn decode_tree_entry_value(key: &[u8], value: &[u8]) -> Result<DirEntry, FormatError> {
    if value.len() < TREE_ENTRY_FIXED {
        return Err(FormatError::Invalid("directory leaf value is truncated"));
    }
    let name_len = le::get_u16(&value[0..2]) as usize;
    if value.len() != TREE_ENTRY_FIXED + name_len {
        return Err(FormatError::Invalid("directory leaf name length mismatch"));
    }
    if value[3..8] != [0; 5] {
        return Err(FormatError::Invalid(
            "directory leaf reserved bytes are nonzero",
        ));
    }
    let entry = DirEntry {
        key: key.to_vec(),
        name: value[TREE_ENTRY_FIXED..].to_vec(),
        child_type_hint: value[2],
        child_id: le::get_u64(&value[8..16]),
    };
    validate_tree_entry(&entry)?;
    Ok(entry)
}

fn validate_tree_entry(entry: &DirEntry) -> Result<(), FormatError> {
    validate_name(&entry.name)?;
    if entry.child_id == OBJECT_INVALID {
        return Err(FormatError::Invalid(
            "directory entry references invalid object",
        ));
    }
    if !matches!(entry.child_type_hint, 1..=3) {
        return Err(FormatError::Invalid(
            "directory entry has invalid child type hint",
        ));
    }
    Ok(())
}
