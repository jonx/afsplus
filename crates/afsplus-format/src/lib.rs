//! On-disk structure encoding and decoding for AFS+.
//!
//! Scope and status: this implements the *prototype* wire structures needed by
//! the first-contributor plan in `implementation/peer-review-prototype-plan.md`
//! (identification block, A/B checkpoints, object records, shared typed COW
//! trees, allocation metadata, plus transitional legacy codecs). Nothing here
//! is a frozen epoch-1 commitment; see `spec/disk-layout.md`.
//!
//! Rules followed from `docs/03-on-disk-format.md` and `docs/21-security-and-corruption.md`:
//!
//! - all multi-byte integers are little-endian, decoded with explicit helpers
//! - structures are byte encodings, never native structs
//! - every independently addressable metadata block carries the common header
//!   (magic/type, version, flags, owner, generation, payload length, CRC32C)
//! - decoding is bounds-first: sizes, offsets, counts and UTF-8 are validated
//!   before use, and corrupted or truncated input yields an error, never a panic
//!
//! The crate is `no_std` + `alloc` so the same codecs can serve host tools,
//! FUSE, and constrained environments (ADR-028).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod bitmap;
pub mod checkpoint;
pub mod crc32c;
pub mod dir;
pub mod extent;
pub mod geometry;
pub mod header;
pub mod ident;
pub mod intent_log;
pub mod le;
pub mod object;
pub mod omap;
pub mod reclaim;
pub mod region;
pub mod retired;
pub mod security;
pub mod snapshot;
pub mod tree;

use core::fmt;

/// Format epoch implemented by this crate.
pub const FORMAT_EPOCH: u32 = 1;

/// Default logical block shift (4 KiB). The prototype supports only this size.
pub const DEFAULT_BLOCK_SHIFT: u8 = 12;
pub const DEFAULT_BLOCK_SIZE: usize = 1 << DEFAULT_BLOCK_SHIFT;

/// Filesystem magic stored in the identification block ("AFSPLUS1" as LE u64).
pub const FS_MAGIC: u64 = 0x3153_554C_5053_4641;

/// Object ID constants (`spec/afsplus_format.h`).
pub const OBJECT_INVALID: u64 = 0;
pub const OBJECT_ROOT: u64 = 1;
/// Reserved internal directory containing files whose last user-visible link
/// was removed while a handle remained open (ADR-066).
pub const OBJECT_ORPHAN_DIRECTORY: u64 = 2;
/// Object IDs below this value are reserved for internal objects
/// (`spec/disk-layout.md` rule 3). Dynamic allocation starts here.
pub const OBJECT_FIRST_DYNAMIC: u64 = 16;

/// Maximum name component length in UTF-8 bytes.
pub const NAME_MAX_UTF8_BYTES: usize = 255;

/// Errors produced when decoding or encoding on-disk structures.
///
/// A `FormatError` from a decoder means the block must be treated as invalid
/// in its entirety; callers must not use partial results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The buffer length does not match the expected block size.
    WrongBufferSize { expected: usize, actual: usize },
    /// The block type magic does not match the expected structure.
    WrongBlockType { expected: u32, actual: u32 },
    /// The header version is not supported by this implementation.
    UnsupportedHeaderVersion(u16),
    /// The stored CRC32C does not match the block contents.
    ChecksumMismatch { stored: u32, computed: u32 },
    /// The payload length field exceeds the space available in the block.
    PayloadTooLarge { payload_len: u32, max: usize },
    /// A structure-specific field is out of range or inconsistent.
    Invalid(&'static str),
    /// A name is not valid UTF-8.
    InvalidUtf8,
    /// The structure being encoded does not fit in one block.
    Overflow(&'static str),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::WrongBufferSize { expected, actual } => {
                write!(f, "wrong buffer size: expected {expected}, got {actual}")
            }
            FormatError::WrongBlockType { expected, actual } => {
                write!(
                    f,
                    "wrong block type: expected {expected:#010x}, got {actual:#010x}"
                )
            }
            FormatError::UnsupportedHeaderVersion(v) => {
                write!(f, "unsupported header version {v}")
            }
            FormatError::ChecksumMismatch { stored, computed } => {
                write!(
                    f,
                    "checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
                )
            }
            FormatError::PayloadTooLarge { payload_len, max } => {
                write!(
                    f,
                    "payload length {payload_len} exceeds block capacity {max}"
                )
            }
            FormatError::Invalid(what) => write!(f, "invalid structure: {what}"),
            FormatError::InvalidUtf8 => write!(f, "name is not valid UTF-8"),
            FormatError::Overflow(what) => write!(f, "structure does not fit in one block: {what}"),
        }
    }
}

impl core::error::Error for FormatError {}

/// A timestamp: signed Unix-epoch seconds in UTC plus nanoseconds.
///
/// Host-local or classic Amiga time bases are adapter behavior
/// (`implementation/peer-review-prototype-plan.md`, accepted corrections).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timespec {
    pub seconds: i64,
    pub nanoseconds: u32,
}

impl Timespec {
    pub const WIRE_SIZE: usize = 12;

    pub fn validate(&self) -> Result<(), FormatError> {
        if self.nanoseconds >= 1_000_000_000 {
            Err(FormatError::Invalid("timestamp nanoseconds out of range"))
        } else {
            Ok(())
        }
    }

    pub fn write(&self, buf: &mut [u8]) {
        le::put_i64(&mut buf[0..8], self.seconds);
        le::put_u32(&mut buf[8..12], self.nanoseconds);
    }

    pub fn read(buf: &[u8]) -> Result<Self, FormatError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(FormatError::WrongBufferSize {
                expected: Self::WIRE_SIZE,
                actual: buf.len(),
            });
        }
        let seconds = le::get_i64(&buf[0..8]);
        let nanoseconds = le::get_u32(&buf[8..12]);
        let time = Timespec {
            seconds,
            nanoseconds,
        };
        time.validate()?;
        Ok(time)
    }
}

/// Validates a directory-entry name at the format layer.
///
/// AFS+ names are valid UTF-8, 1..=255 bytes (`docs/05-directories-and-names.md`).
/// NUL and `/` are rejected here as a prototype restriction until the
/// namespace-layer separator policy is specified.
pub fn validate_name(name: &[u8]) -> Result<(), FormatError> {
    if name.is_empty() || name.len() > NAME_MAX_UTF8_BYTES {
        return Err(FormatError::Invalid("name length out of range"));
    }
    let s = core::str::from_utf8(name).map_err(|_| FormatError::InvalidUtf8)?;
    if s.chars().any(|c| c == '\0' || c == '/') {
        return Err(FormatError::Invalid("name contains reserved character"));
    }
    Ok(())
}
