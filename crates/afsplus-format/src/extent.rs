//! Extent-map item: one mapping of a regular file's extent tree (`AFST` nodes
//! of kind `ExtentMap`, owner = the file's object ID).
//!
//! ```text
//! key    8    logical start block, big-endian so byte order is numeric order
//! value  24   offset size field
//!             0      8    physical start block
//!             8      8    block count (nonzero)
//!             16     4    flags
//!             20     4    reserved (zero)
//! ```
//!
//! Flags are a validated namespace: a reader refuses a value that carries a
//! bit it does not know. This codec checks the shape of one item; whether the
//! physical run lies inside the volume and one region, and whether the shared
//! marker is legal on the volume, are contextual checks of the reader.

use crate::{le, FormatError};

pub const EXTENT_KEY_LEN: usize = 8;
pub const EXTENT_VALUE_LEN: usize = 24;

/// Allocated blocks whose contents are not yet part of the logical file: they
/// read as zeros until a write replaces the extent or clears this flag.
pub const EXTENT_UNWRITTEN: u32 = 1 << 0;
/// Conservative marker: the physical run may overlap shared records, and the
/// volume-wide reference tree is the authority (ADR-061).
pub const EXTENT_SHARED: u32 = 1 << 1;
pub const EXTENT_KNOWN_FLAGS: u32 = EXTENT_UNWRITTEN | EXTENT_SHARED;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentItem {
    pub logical_start: u64,
    pub physical_start: u64,
    pub block_count: u64,
    pub flags: u32,
}

impl ExtentItem {
    fn validate(&self) -> Result<(), FormatError> {
        if self.block_count == 0 {
            return Err(FormatError::Invalid("zero-length extent"));
        }
        if self.flags & !EXTENT_KNOWN_FLAGS != 0 {
            return Err(FormatError::Invalid("extent has unsupported flags"));
        }
        if self.logical_start.checked_add(self.block_count).is_none() {
            return Err(FormatError::Overflow("extent logical end"));
        }
        if self.physical_start.checked_add(self.block_count).is_none() {
            return Err(FormatError::Overflow("extent physical end"));
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<([u8; EXTENT_KEY_LEN], [u8; EXTENT_VALUE_LEN]), FormatError> {
        self.validate()?;
        let mut value = [0u8; EXTENT_VALUE_LEN];
        le::put_u64(&mut value[0..8], self.physical_start);
        le::put_u64(&mut value[8..16], self.block_count);
        le::put_u32(&mut value[16..20], self.flags);
        Ok((self.logical_start.to_be_bytes(), value))
    }

    pub fn decode(key: &[u8], value: &[u8]) -> Result<ExtentItem, FormatError> {
        let key: [u8; EXTENT_KEY_LEN] = key
            .try_into()
            .map_err(|_| FormatError::Invalid("extent key is not eight bytes"))?;
        if value.len() != EXTENT_VALUE_LEN {
            return Err(FormatError::Invalid(
                "extent value is not twenty-four bytes",
            ));
        }
        if value[20..24] != [0; 4] {
            return Err(FormatError::Invalid("extent reserved bytes are nonzero"));
        }
        let item = ExtentItem {
            logical_start: u64::from_be_bytes(key),
            physical_start: le::get_u64(&value[0..8]),
            block_count: le::get_u64(&value[8..16]),
            flags: le::get_u32(&value[16..20]),
        };
        item.validate()?;
        Ok(item)
    }
}
