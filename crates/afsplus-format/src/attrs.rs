//! Extended attributes: the whole attribute set of one object, held as one
//! blob in an owned chain of `"AFSA"` blocks (`chain.rs`). The set is
//! immutable: changing one attribute writes a new chain and retires the old.
//!
//! Set layout, all little-endian:
//!
//! ```text
//! offset size field
//! 0      2    entry count (nonzero)
//! 2      2    reserved (zero)
//! 4      n    entries, in strictly ascending order of name bytes
//! ```
//!
//! One entry:
//!
//! ```text
//! 0      1    name length in bytes (nonzero)
//! 1      1    reserved (zero)
//! 2      2    value length in bytes
//! 4      n    name, UTF-8, no NUL, in a known namespace
//! 4+n    m    value, opaque
//! ```
//!
//! Admission is exact: the entries end where the blob ends. An object without
//! attributes has no chain; an empty set is never stored.
use alloc::vec::Vec;

use crate::chain::ChainKind;
use crate::header::block_type;
use crate::{le, FormatError};

/// Bound on one attribute set, names and framing included.
pub const MAX_ATTRIBUTE_SET_BYTES: u32 = 65_536;
pub const ATTRIBUTE_NAME_MAX_BYTES: usize = 255;
pub const ATTRIBUTE_VALUE_MAX_BYTES: usize = u16::MAX as usize;

/// Format identity and version the chain segments of a set carry.
pub const ATTRIBUTE_SET_FORMAT: u32 = 1;
pub const ATTRIBUTE_SET_VERSION: u16 = 0;

/// Namespaces a name must start with; the rest of the name is nonempty.
pub const ATTRIBUTE_NAMESPACES: [&str; 4] = ["user.", "system.", "security.", "aros."];

const SET_HEADER: usize = 4;
const ENTRY_HEADER: usize = 4;

/// The attribute set chain as an owned chain: "AFSA" blocks.
pub const ATTRIBUTE_CHAIN: ChainKind = ChainKind {
    block_type: block_type::ATTRIBUTE_SET,
    max_bytes: MAX_ATTRIBUTE_SET_BYTES,
    label: "attribute",
    length_out_of_range: "attribute set length out of range",
    owner_or_format_zero: "attribute segment owner or format is zero",
    position_inconsistent: "attribute segment position inconsistent",
    link_inconsistent: "attribute segment chain link inconsistent",
    length_not_exact: "attribute segment length is not exact",
    flags_nonzero: "attribute segment header flags are nonzero",
    payload_too_short: "attribute segment payload too short",
    reserved_nonzero: "attribute segment reserved field is nonzero",
    tail_nonzero: "attribute segment unused tail is nonzero",
};

/// Segments a set of `total_len` bytes occupies, or `None` when the length is
/// zero, above the bound, or the block cannot hold any byte.
pub fn segment_count(total_len: u32, block_size: usize) -> Option<u16> {
    crate::chain::segment_count(&ATTRIBUTE_CHAIN, total_len, block_size)
}

/// A name is 1 to 255 bytes of UTF-8 without NUL, in a known namespace, with
/// something after the namespace.
pub fn validate_attribute_name(name: &str) -> Result<(), FormatError> {
    if name.len() > ATTRIBUTE_NAME_MAX_BYTES {
        return Err(FormatError::Overflow("attribute name"));
    }
    if name.as_bytes().contains(&0) {
        return Err(FormatError::Invalid("attribute name contains NUL"));
    }
    if !ATTRIBUTE_NAMESPACES
        .iter()
        .any(|prefix| name.len() > prefix.len() && name.starts_with(prefix))
    {
        return Err(FormatError::Invalid("attribute name outside a namespace"));
    }
    Ok(())
}

/// Encode a set. `entries` is nonempty and strictly ascending by name bytes;
/// the caller keeps the set sorted, so a duplicate is refused here.
pub fn encode_attribute_set(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, FormatError> {
    if entries.is_empty() {
        return Err(FormatError::Invalid("attribute set is empty"));
    }
    let count =
        u16::try_from(entries.len()).map_err(|_| FormatError::Overflow("attribute count"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    let mut previous: Option<&str> = None;
    for (name, value) in entries {
        validate_attribute_name(name)?;
        if previous.is_some_and(|previous| previous.as_bytes() >= name.as_bytes()) {
            return Err(FormatError::Invalid("attribute names are not ascending"));
        }
        previous = Some(name);
        let value_len =
            u16::try_from(value.len()).map_err(|_| FormatError::Overflow("attribute value"))?;
        out.push(name.len() as u8);
        out.push(0);
        out.extend_from_slice(&value_len.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(value);
        if out.len() > MAX_ATTRIBUTE_SET_BYTES as usize {
            return Err(FormatError::Overflow("attribute set"));
        }
    }
    Ok(out)
}

/// Decode a set into its entries, in stored order.
pub fn decode_attribute_set(bytes: &[u8]) -> Result<Vec<(&str, &[u8])>, FormatError> {
    if bytes.len() > MAX_ATTRIBUTE_SET_BYTES as usize {
        return Err(FormatError::Overflow("attribute set"));
    }
    if bytes.len() < SET_HEADER {
        return Err(FormatError::Invalid("attribute set too short"));
    }
    let count = le::get_u16(&bytes[0..2]) as usize;
    if count == 0 {
        return Err(FormatError::Invalid("attribute set is empty"));
    }
    if le::get_u16(&bytes[2..4]) != 0 {
        return Err(FormatError::Invalid(
            "attribute set reserved field is nonzero",
        ));
    }
    let truncated = FormatError::Invalid("attribute set length is not exact");
    let mut entries: Vec<(&str, &[u8])> = Vec::with_capacity(count.min(bytes.len() / 5));
    let mut at = SET_HEADER;
    for _ in 0..count {
        let header = bytes.get(at..at + ENTRY_HEADER).ok_or(truncated.clone())?;
        let name_len = header[0] as usize;
        if name_len == 0 {
            return Err(FormatError::Invalid("attribute name is empty"));
        }
        if header[1] != 0 {
            return Err(FormatError::Invalid(
                "attribute entry reserved byte is nonzero",
            ));
        }
        let value_len = le::get_u16(&header[2..4]) as usize;
        at += ENTRY_HEADER;
        let name = bytes.get(at..at + name_len).ok_or(truncated.clone())?;
        at += name_len;
        let value = bytes.get(at..at + value_len).ok_or(truncated.clone())?;
        at += value_len;
        let name = core::str::from_utf8(name).map_err(|_| FormatError::InvalidUtf8)?;
        validate_attribute_name(name)?;
        if entries
            .last()
            .is_some_and(|(previous, _)| previous.as_bytes() >= name.as_bytes())
        {
            return Err(FormatError::Invalid("attribute names are not ascending"));
        }
        entries.push((name, value));
    }
    if at != bytes.len() {
        return Err(truncated);
    }
    Ok(entries)
}
