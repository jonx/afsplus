//! Core object record (`docs/04-object-model.md` §3).
//!
//! One object per block in the prototype; the header's owner field carries
//! the object ID so repair tooling can attribute the block without context.
//!
//! Files either carry one direct data extent (`data_root` = start LBA,
//! `data_blocks` = length) or set [`OBJECT_FLAG_EXTENT_TREE`] and use
//! `data_root` as the root of their typed AFST extent map.
//! Directories use `data_root` for their directory-tree root.
//!
//! `link_count == 0` is invalid: the prototype has no orphan handling yet,
//! so an unreferenced object must not exist at all.
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
//! 80     8    data root LBA (directory: tree root; file: extent start)
//! 88     8    data extent length in blocks (files; 0 = empty file)
//! ```
//!
//! With [`OBJECT_FLAG_SECURITY_REF`] the fixed payload is 112 bytes: a
//! security reference follows, before any inline symlink target.
//!
//! ```text
//! 96     8    first security descriptor segment LBA (nonzero)
//! 104    4    descriptor length in bytes
//! 108    2    descriptor segment count
//! 110    2    reference flags (bit 0: projection diverged)
//! ```
//!
//! With [`OBJECT_FLAG_COMMENT`] a comment follows the fixed payload (after
//! the security reference when both are present, before any inline symlink
//! target): one length byte, 1 to 255, then that many bytes of UTF-8 without
//! NUL (ADR-106). An empty comment is the absent one: the flag is clear.

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, Timespec, OBJECT_INVALID};

const PAYLOAD_LEN: usize = 96;

/// Resource-exhaustion guard (`docs/21-security-and-corruption.md` §5): a
/// corrupt record must not be able to demand absurd extent walks.
pub const MAX_EXTENT_BLOCKS: u64 = 4096;

/// `data_root` references a typed AFST extent map instead of one direct
/// physical extent. This experimental flag is not an epoch-1 commitment.
pub const OBJECT_FLAG_EXTENT_TREE: u16 = 1 << 0;

/// This file is persistently opted into ADR-062 private in-place data
/// updates (ADR-065). Files only; legal only on a volume whose
/// identification carries `COMPAT_DATA_POLICY` — that congruence is
/// enforced by the contextual read paths and the checker, exactly like the
/// shared-extent marker.
pub const OBJECT_FLAG_DATA_IN_PLACE: u16 = 1 << 1;

/// The fixed payload carries a [`SecurityRef`]. Legal only on a volume whose
/// identification carries `INCOMPAT_SECURITY_DESCRIPTORS`; the contextual
/// read paths and the checker enforce that congruence.
pub const OBJECT_FLAG_SECURITY_REF: u16 = 1 << 2;

/// The record carries a comment (ADR-106). No volume feature gates it: every
/// implementation of the format reads the comment field.
pub const OBJECT_FLAG_COMMENT: u16 = 1 << 3;

/// The record carries an [`AttributeRef`] to its extended attribute set. No
/// volume feature gates it: every implementation of the format knows the
/// field, and one that does not read attributes still preserves them.
pub const OBJECT_FLAG_ATTRIBUTES: u16 = 1 << 4;

/// Flags that say which optional metadata fields the record carries; legal
/// on every object type.
const OBJECT_METADATA_FLAGS: u16 =
    OBJECT_FLAG_SECURITY_REF | OBJECT_FLAG_COMMENT | OBJECT_FLAG_ATTRIBUTES;

/// Longest comment, in UTF-8 bytes.
pub const COMMENT_MAX_BYTES: usize = 255;

/// A file comment held inline in the object record. `Copy`, like the record,
/// so every read-modify-write of a record carries it without a second path.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Comment {
    len: u8,
    bytes: [u8; COMMENT_MAX_BYTES],
}

impl Comment {
    pub const EMPTY: Comment = Comment {
        len: 0,
        bytes: [0; COMMENT_MAX_BYTES],
    };

    /// At most 255 bytes of UTF-8 without NUL.
    pub fn new(text: &str) -> Result<Comment, FormatError> {
        if text.len() > COMMENT_MAX_BYTES {
            return Err(FormatError::Overflow("object comment"));
        }
        if text.as_bytes().contains(&0) {
            return Err(FormatError::Invalid("object comment contains NUL"));
        }
        let mut bytes = [0; COMMENT_MAX_BYTES];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Ok(Comment {
            len: text.len() as u8,
            bytes,
        })
    }

    pub fn as_str(&self) -> &str {
        // Construction and decoding both validate UTF-8.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn wire_len(&self) -> usize {
        if self.is_empty() {
            0
        } else {
            1 + self.len as usize
        }
    }
}

impl core::fmt::Debug for Comment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

/// The protection field was edited by a host that did not evaluate the
/// descriptor, so the classic projection and the descriptor may disagree.
pub const SECURITY_REF_PROJECTION_DIVERGED: u16 = 1 << 0;

const SECURITY_REF_LEN: usize = 16;

/// Reference from an object record to its security descriptor chain
/// (`security.rs`). The record owns the chain exclusively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityRef {
    pub first_block: u64,
    pub total_len: u32,
    pub segment_count: u16,
    pub flags: u16,
}

impl SecurityRef {
    fn validate(&self, block_size: usize) -> Result<(), FormatError> {
        if self.first_block == 0
            || self.flags & !SECURITY_REF_PROJECTION_DIVERGED != 0
            || crate::security::segment_count(self.total_len, block_size)
                != Some(self.segment_count)
        {
            return Err(FormatError::Invalid("invalid security reference"));
        }
        Ok(())
    }
}

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
    pub data_blocks: u64,
    /// Present exactly when `flags` carries [`OBJECT_FLAG_SECURITY_REF`].
    pub security: Option<SecurityRef>,
    /// Present exactly when `flags` carries [`OBJECT_FLAG_ATTRIBUTES`].
    pub attributes: Option<AttributeRef>,
    /// Nonempty exactly when `flags` carries [`OBJECT_FLAG_COMMENT`].
    pub comment: Comment,
}

const ATTRIBUTE_REF_LEN: usize = 16;

/// Reference from an object record to its attribute set chain (`attrs.rs`).
/// The record owns the chain exclusively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeRef {
    pub first_block: u64,
    pub total_len: u32,
    pub segment_count: u16,
}

impl AttributeRef {
    fn validate(&self, block_size: usize) -> Result<(), FormatError> {
        if self.first_block == 0
            || crate::attrs::segment_count(self.total_len, block_size) != Some(self.segment_count)
        {
            return Err(FormatError::Invalid("invalid attribute reference"));
        }
        Ok(())
    }
}

impl ObjectRecord {
    /// Length of the fixed payload, before any inline symlink target.
    pub fn fixed_payload_len(&self) -> usize {
        self.attributes_end() + self.comment.wire_len()
    }

    fn attributes_end(&self) -> usize {
        if self.attributes.is_some() {
            self.security_end() + ATTRIBUTE_REF_LEN
        } else {
            self.security_end()
        }
    }

    fn security_end(&self) -> usize {
        if self.security.is_some() {
            PAYLOAD_LEN + SECURITY_REF_LEN
        } else {
            PAYLOAD_LEN
        }
    }

    /// The same record with `comment` set or removed; keeps the flag and
    /// the field congruent.
    pub fn with_comment(mut self, comment: Comment) -> Self {
        self.comment = comment;
        if comment.is_empty() {
            self.flags &= !OBJECT_FLAG_COMMENT;
        } else {
            self.flags |= OBJECT_FLAG_COMMENT;
        }
        self
    }

    /// The same record with `attributes` attached or removed; keeps the flag
    /// and the field congruent.
    pub fn with_attributes(mut self, attributes: Option<AttributeRef>) -> Self {
        self.attributes = attributes;
        if attributes.is_some() {
            self.flags |= OBJECT_FLAG_ATTRIBUTES;
        } else {
            self.flags &= !OBJECT_FLAG_ATTRIBUTES;
        }
        self
    }

    /// The same record with `security` attached or removed; keeps the flag
    /// and the field congruent.
    pub fn with_security(mut self, security: Option<SecurityRef>) -> Self {
        self.security = security;
        if security.is_some() {
            self.flags |= OBJECT_FLAG_SECURITY_REF;
        } else {
            self.flags &= !OBJECT_FLAG_SECURITY_REF;
        }
        self
    }

    pub fn encode(
        &self,
        block_size: usize,
        transaction_generation: u64,
    ) -> Result<Vec<u8>, FormatError> {
        let minimum = HEADER_SIZE + self.fixed_payload_len();
        if block_size < minimum {
            return Err(FormatError::WrongBufferSize {
                expected: minimum,
                actual: block_size,
            });
        }
        self.validate(block_size)?;
        let mut block = vec![0u8; block_size];
        self.write_fields(&mut block);
        self.seal_record(&mut block, transaction_generation, self.fixed_payload_len());
        Ok(block)
    }

    fn write_fields(&self, block: &mut [u8]) {
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
        le::put_u64(&mut p[88..96], self.data_blocks);
        if let Some(security) = self.security {
            le::put_u64(&mut p[96..104], security.first_block);
            le::put_u32(&mut p[104..108], security.total_len);
            le::put_u16(&mut p[108..110], security.segment_count);
            le::put_u16(&mut p[110..112], security.flags);
        }
        if let Some(attributes) = self.attributes {
            let at = self.security_end();
            le::put_u64(&mut p[at..at + 8], attributes.first_block);
            le::put_u32(&mut p[at + 8..at + 12], attributes.total_len);
            le::put_u16(&mut p[at + 12..at + 14], attributes.segment_count);
        }
        if !self.comment.is_empty() {
            let at = self.attributes_end();
            p[at] = self.comment.len;
            p[at + 1..at + 1 + self.comment.len as usize]
                .copy_from_slice(&self.comment.bytes[..self.comment.len as usize]);
        }
    }

    fn seal_record(&self, block: &mut [u8], generation: u64, payload_len: usize) {
        BlockHeader {
            block_type: block_type::OBJECT,
            flags: 0,
            owner: self.object_id,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(block);
    }

    pub fn decode(block: &[u8]) -> Result<ObjectRecord, FormatError> {
        Ok(Self::decode_with_generation(block)?.0)
    }

    /// Like [`ObjectRecord::decode`], but also returns the sealed header's
    /// transaction generation so callers can bound it against the selected
    /// checkpoint, exactly as tree-node access does.
    pub fn decode_with_generation(block: &[u8]) -> Result<(ObjectRecord, u64), FormatError> {
        let (record, header) = Self::decode_fields(block)?;
        record.validate(block.len())?;
        Ok((record, header.generation))
    }

    /// Validate all inline payload bytes, returning metadata for explicit
    /// payload-aware callers. Re-encoding a symlink still requires its target.
    pub fn decode_metadata_with_generation(
        block: &[u8],
    ) -> Result<(ObjectRecord, u64), FormatError> {
        let (record, header) = Self::decode_fields(block)?;
        if record.object_type == ObjectType::Symlink {
            let link = SymlinkRecord::from_verified_fields(block, record, header)?;
            Ok((link.record, header.generation))
        } else {
            record.validate(block.len())?;
            Ok((record, header.generation))
        }
    }

    fn decode_fields(block: &[u8]) -> Result<(ObjectRecord, BlockHeader), FormatError> {
        let header = BlockHeader::verify(block, block_type::OBJECT)?;
        let p = header.payload(block);
        if p.len() < PAYLOAD_LEN {
            return Err(FormatError::Invalid("object record payload too short"));
        }
        // Exact admission: a rewrite re-encodes the decoded fields into a
        // zeroed block, so any byte accepted here without a field would be
        // dropped by the next metadata edit.
        if header.flags != 0 {
            return Err(FormatError::Invalid("object header flags are nonzero"));
        }
        if p[9] != 0 {
            return Err(FormatError::Invalid("object reserved byte is nonzero"));
        }
        let flags = le::get_u16(&p[10..12]);
        let security_end = if flags & OBJECT_FLAG_SECURITY_REF != 0 {
            PAYLOAD_LEN + SECURITY_REF_LEN
        } else {
            PAYLOAD_LEN
        };
        let attributes_end = if flags & OBJECT_FLAG_ATTRIBUTES != 0 {
            security_end + ATTRIBUTE_REF_LEN
        } else {
            security_end
        };
        if p.len() < attributes_end {
            return Err(FormatError::Invalid("object payload length is not exact"));
        }
        let attributes = if attributes_end != security_end {
            let at = security_end;
            if le::get_u16(&p[at + 14..at + 16]) != 0 {
                return Err(FormatError::Invalid(
                    "attribute reference reserved field is nonzero",
                ));
            }
            Some(AttributeRef {
                first_block: le::get_u64(&p[at..at + 8]),
                total_len: le::get_u32(&p[at + 8..at + 12]),
                segment_count: le::get_u16(&p[at + 12..at + 14]),
            })
        } else {
            None
        };
        let comment = if flags & OBJECT_FLAG_COMMENT != 0 {
            let length = *p
                .get(attributes_end)
                .ok_or(FormatError::Invalid("object payload length is not exact"))?
                as usize;
            let text = p
                .get(attributes_end + 1..attributes_end + 1 + length)
                .ok_or(FormatError::Invalid("object payload length is not exact"))?;
            if length == 0 {
                return Err(FormatError::Invalid("object comment flag without comment"));
            }
            let text = core::str::from_utf8(text).map_err(|_| FormatError::InvalidUtf8)?;
            Comment::new(text)?
        } else {
            Comment::EMPTY
        };
        let fixed = attributes_end + comment.wire_len();
        let is_symlink = p[8] == ObjectType::Symlink.to_wire();
        if !is_symlink && p.len() != fixed {
            return Err(FormatError::Invalid("object payload length is not exact"));
        }
        let record = ObjectRecord {
            object_id: le::get_u64(&p[0..8]),
            object_type: ObjectType::from_wire(p[8])?,
            flags,
            link_count: le::get_u32(&p[12..16]),
            size_bytes: le::get_u64(&p[16..24]),
            allocated_bytes: le::get_u64(&p[24..32]),
            created: Timespec::read(&p[32..44])?,
            modified: Timespec::read(&p[44..56])?,
            changed: Timespec::read(&p[56..68])?,
            protection: le::get_u32(&p[68..72]),
            content_generation: le::get_u64(&p[72..80]),
            data_root: le::get_u64(&p[80..88]),
            data_blocks: le::get_u64(&p[88..96]),
            comment,
            attributes,
            security: (security_end != PAYLOAD_LEN).then(|| SecurityRef {
                first_block: le::get_u64(&p[96..104]),
                total_len: le::get_u32(&p[104..108]),
                segment_count: le::get_u16(&p[108..110]),
                flags: le::get_u16(&p[110..112]),
            }),
        };
        if record.object_id != header.owner {
            return Err(FormatError::Invalid("object ID does not match block owner"));
        }
        Ok((record, header))
    }

    fn validate_common(&self, block_size: usize) -> Result<(), FormatError> {
        self.created.validate()?;
        self.modified.validate()?;
        self.changed.validate()?;
        if self.object_id == OBJECT_INVALID {
            return Err(FormatError::Invalid("object ID zero is invalid"));
        }
        if self.link_count == 0 {
            return Err(FormatError::Invalid(
                "link count zero without orphan support",
            ));
        }
        if self.flags
            & !(OBJECT_FLAG_EXTENT_TREE
                | OBJECT_FLAG_DATA_IN_PLACE
                | OBJECT_FLAG_SECURITY_REF
                | OBJECT_FLAG_COMMENT
                | OBJECT_FLAG_ATTRIBUTES)
            != 0
        {
            return Err(FormatError::Invalid("object has unsupported flags"));
        }
        if (self.flags & OBJECT_FLAG_SECURITY_REF != 0) != self.security.is_some() {
            return Err(FormatError::Invalid(
                "security reference flag and field disagree",
            ));
        }
        if let Some(security) = self.security {
            security.validate(block_size)?;
        }
        if (self.flags & OBJECT_FLAG_ATTRIBUTES != 0) != self.attributes.is_some() {
            return Err(FormatError::Invalid(
                "attribute reference flag and field disagree",
            ));
        }
        if let Some(attributes) = self.attributes {
            attributes.validate(block_size)?;
        }
        if (self.flags & OBJECT_FLAG_COMMENT != 0) == self.comment.is_empty() {
            return Err(FormatError::Invalid("comment flag and field disagree"));
        }
        Ok(())
    }

    fn validate(&self, block_size: usize) -> Result<(), FormatError> {
        self.validate_common(block_size)?;
        match self.object_type {
            ObjectType::Directory => {
                if self.flags & !OBJECT_METADATA_FLAGS != 0 {
                    return Err(FormatError::Invalid("directory has file extent flags"));
                }
                if self.data_root == 0 {
                    return Err(FormatError::Invalid(
                        "directory must reference a directory block",
                    ));
                }
                if self.size_bytes != 0 || self.data_blocks != 0 {
                    return Err(FormatError::Invalid("directory size fields must be zero"));
                }
            }
            ObjectType::File => {
                let expected_allocated = self
                    .data_blocks
                    .checked_mul(block_size as u64)
                    .ok_or(FormatError::Overflow("allocated byte count"))?;
                if self.allocated_bytes != expected_allocated {
                    return Err(FormatError::Invalid(
                        "allocated bytes do not match data block count",
                    ));
                }
                if self.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
                    if self.data_root == 0 {
                        return Err(FormatError::Invalid("extent-tree file has no tree root"));
                    }
                } else if self.data_blocks > MAX_EXTENT_BLOCKS {
                    return Err(FormatError::Invalid("file extent exceeds prototype cap"));
                } else if self.data_blocks == 0 {
                    if self.data_root != 0 || self.size_bytes != 0 {
                        return Err(FormatError::Invalid("empty file with data references"));
                    }
                } else {
                    self.data_root
                        .checked_add(self.data_blocks)
                        .ok_or(FormatError::Overflow(
                            "file extent end overflows block address",
                        ))?;
                    let capacity = self.data_blocks * block_size as u64;
                    let minimum = (self.data_blocks - 1) * block_size as u64;
                    if self.data_root == 0
                        || self.size_bytes > capacity
                        || self.size_bytes <= minimum
                    {
                        return Err(FormatError::Invalid("file size inconsistent with extent"));
                    }
                }
            }
            ObjectType::Symlink | ObjectType::Internal => {
                return Err(FormatError::Invalid(
                    "object type not implemented in prototype",
                ));
            }
        }
        Ok(())
    }
}

/// Checksummed inline symlink payload. Fixed-record APIs deliberately reject it
/// until callers explicitly preserve the target through every rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymlinkRecord<'a> {
    pub record: ObjectRecord,
    pub target: &'a str,
}

impl<'a> SymlinkRecord<'a> {
    /// Longest target of a symlink without a security reference. A reference
    /// takes 16 of these bytes.
    pub fn maximum_target_bytes(block_size: usize) -> usize {
        block_size.saturating_sub(HEADER_SIZE + PAYLOAD_LEN)
    }

    fn validate(&self, block_size: usize) -> Result<(), FormatError> {
        self.record.validate_common(block_size)?;
        let fixed = self.record.fixed_payload_len();
        if self.record.object_type != ObjectType::Symlink
            || self.record.flags & !OBJECT_METADATA_FLAGS != 0
            || self.record.data_root != 0
            || self.record.data_blocks != 0
            || self.record.allocated_bytes != 0
            || self.record.size_bytes != self.target.len() as u64
            || self.target.is_empty()
            || self.target.as_bytes().contains(&0)
            || self.target.len() > block_size.saturating_sub(HEADER_SIZE + fixed)
            || self.target.len() > u32::MAX as usize - fixed
        {
            return Err(FormatError::Invalid("invalid inline symlink record"));
        }
        Ok(())
    }

    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        self.validate(block_size)?;
        let fixed = self.record.fixed_payload_len();
        let payload_len = fixed + self.target.len();
        let mut block = vec![0; block_size];
        self.record.write_fields(&mut block);
        block[HEADER_SIZE + fixed..HEADER_SIZE + payload_len]
            .copy_from_slice(self.target.as_bytes());
        self.record.seal_record(&mut block, generation, payload_len);
        Ok(block)
    }

    pub fn decode(block: &'a [u8]) -> Result<(Self, u64), FormatError> {
        let (record, header) = ObjectRecord::decode_fields(block)?;
        Ok((
            Self::from_verified_fields(block, record, header)?,
            header.generation,
        ))
    }

    fn from_verified_fields(
        block: &'a [u8],
        record: ObjectRecord,
        header: BlockHeader,
    ) -> Result<Self, FormatError> {
        let payload = header.payload(block);
        if header.flags != 0 || payload[9] != 0 {
            return Err(FormatError::Invalid("symlink reserved fields are nonzero"));
        }
        let target = core::str::from_utf8(&payload[record.fixed_payload_len()..])
            .map_err(|_| FormatError::Invalid("symlink target is not UTF-8"))?;
        let result = Self { record, target };
        result.validate(block.len())?;
        Ok(result)
    }
}
