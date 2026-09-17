//! Intent-log record (ADR-037, experimental).
//!
//! One block per record, one record per fsync. A record binds to the exact
//! checkpoint generation it extends (records from earlier generations become
//! stale without erasure) and carries the whole fsync group, so a group
//! replays all-or-nothing. Created content is referenced by its already
//! written extents plus a CRC32C of the file bytes; the record never carries
//! the data itself.
//!
//! Payload layout after the common header (header.generation = base
//! checkpoint generation):
//!
//! ```text
//! offset size field
//! 0      16   filesystem UUID
//! 16     8    base checkpoint generation (mirror)
//! 24     4    sequence (slot index + 1)
//! 28     2    operation count
//! 30     2    record version (2 for namespace-only groups, 3 when an
//!             existing-file update is present; zero is the legacy prototype)
//! 32     ...  operations
//! ```
//!
//! Operation entry (unaligned, explicitly decoded):
//!
//! ```text
//! offset size field
//! 0      1    op type (1 create, 2 delete, 3 rename, 4 write, 5 truncate)
//! 1      1    replace flag (rename)
//! 2      2    source/parent name length
//! 4      2    target name length (rename)
//! 6      2    data extent count (create, write, truncate)
//! 8      8    parent / source-parent object ID
//! 16     8    target-parent object ID (rename) / first logical block (write,
//!             truncate tail rewrite)
//! 24     8    expected created object ID (create) / expected file size
//!             (write, truncate)
//! 32     8    content size (create) / resulting file size (write, truncate)
//! 40     4    content CRC32C (create) / complete replacement-block CRC32C
//!             (write, truncate)
//! 44     4    reserved
//! 48     12   operation timestamp (seconds i64, nanoseconds u32)
//! 60     4    reserved
//! 64     ...  data extents: (start u64, blocks u32) × count, then source
//!             name, then target name
//! ```

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, validate_name, FormatError, Timespec};

const FIXED_PAYLOAD: usize = 32;
const OP_FIXED: usize = 64;
const EXTENT_WIRE: usize = 12;
const PREVIOUS_LOG_RECORD_VERSION: u16 = 2;
const LOG_RECORD_VERSION: u16 = 3;

/// Extents per logged create; larger fsynced creates are a reported limit.
pub const MAX_LOG_EXTENTS: usize = 16;
/// Operations per record; a larger unlogged group is a reported limit.
pub const MAX_LOG_OPS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogOp {
    Create {
        parent_id: u64,
        name: Vec<u8>,
        expected_object_id: u64,
        size_bytes: u64,
        content_crc: u32,
        timestamp: Timespec,
        /// (physical start, blocks); the data was written before the record.
        extents: Vec<(u64, u32)>,
    },
    Delete {
        parent_id: u64,
        name: Vec<u8>,
        timestamp: Timespec,
    },
    Rename {
        source_parent_id: u64,
        source_name: Vec<u8>,
        target_parent_id: u64,
        target_name: Vec<u8>,
        replace: bool,
        timestamp: Timespec,
    },
    /// Replaces complete logical blocks of an existing file with already
    /// durable COW data. `logical_start` is in filesystem blocks and the
    /// extents provide the corresponding physical blocks in order.
    Write {
        object_id: u64,
        logical_start: u64,
        expected_size_bytes: u64,
        new_size_bytes: u64,
        content_crc: u32,
        timestamp: Timespec,
        extents: Vec<(u64, u32)>,
    },
    /// Changes an existing file's size. A partial written tail is replaced
    /// by exactly one already durable COW block; aligned, sparse-tail and
    /// growth truncates carry no data extent.
    Truncate {
        object_id: u64,
        logical_start: u64,
        expected_size_bytes: u64,
        new_size_bytes: u64,
        content_crc: u32,
        timestamp: Timespec,
        extents: Vec<(u64, u32)>,
    },
}

impl LogOp {
    fn wire_len_with_fixed(&self, fixed: usize) -> usize {
        match self {
            LogOp::Create { name, extents, .. } => fixed + extents.len() * EXTENT_WIRE + name.len(),
            LogOp::Delete { name, .. } => fixed + name.len(),
            LogOp::Rename {
                source_name,
                target_name,
                ..
            } => fixed + source_name.len() + target_name.len(),
            LogOp::Write { extents, .. } | LogOp::Truncate { extents, .. } => {
                fixed + extents.len() * EXTENT_WIRE
            }
        }
    }

    fn wire_len(&self) -> usize {
        self.wire_len_with_fixed(OP_FIXED)
    }

    pub fn timestamp(&self) -> Timespec {
        match self {
            LogOp::Create { timestamp, .. }
            | LogOp::Delete { timestamp, .. }
            | LogOp::Rename { timestamp, .. }
            | LogOp::Write { timestamp, .. }
            | LogOp::Truncate { timestamp, .. } => *timestamp,
        }
    }

    /// Physical COW data referenced by this operation, if any.
    pub fn data_extents(&self) -> &[(u64, u32)] {
        match self {
            LogOp::Create { extents, .. }
            | LogOp::Write { extents, .. }
            | LogOp::Truncate { extents, .. } => extents,
            LogOp::Delete { .. } | LogOp::Rename { .. } => &[],
        }
    }

    pub fn is_existing_file_update(&self) -> bool {
        matches!(self, LogOp::Write { .. } | LogOp::Truncate { .. })
    }

    fn validate(&self) -> Result<(), FormatError> {
        self.timestamp().validate()?;
        match self {
            LogOp::Create {
                name,
                extents,
                expected_object_id,
                size_bytes,
                ..
            } => {
                validate_name(name)?;
                if *expected_object_id == 0 {
                    return Err(FormatError::Invalid("logged create without object ID"));
                }
                let total = validate_data_extents(extents)?;
                if (extents.is_empty() && *size_bytes != 0)
                    || (!extents.is_empty()
                        && (*size_bytes == 0
                            || size_bytes.div_ceil(crate::DEFAULT_BLOCK_SIZE as u64) > total))
                {
                    return Err(FormatError::Invalid(
                        "logged size inconsistent with extents",
                    ));
                }
                Ok(())
            }
            LogOp::Delete { name, .. } => validate_name(name),
            LogOp::Rename {
                source_name,
                target_name,
                ..
            } => {
                validate_name(source_name)?;
                validate_name(target_name)
            }
            LogOp::Write {
                object_id,
                logical_start,
                expected_size_bytes,
                new_size_bytes,
                extents,
                ..
            } => {
                if *object_id == 0 {
                    return Err(FormatError::Invalid("logged write without object ID"));
                }
                let blocks = validate_data_extents(extents)?;
                if blocks == 0 {
                    return Err(FormatError::Invalid(
                        "logged write without replacement data",
                    ));
                }
                let logical_end = logical_start
                    .checked_add(blocks)
                    .ok_or(FormatError::Invalid("logged write range overflows"))?;
                if new_size_bytes < expected_size_bytes || *new_size_bytes == 0 {
                    return Err(FormatError::Invalid(
                        "logged write has invalid size transition",
                    ));
                }
                let file_blocks = new_size_bytes.div_ceil(crate::DEFAULT_BLOCK_SIZE as u64);
                if logical_end > file_blocks
                    || (*new_size_bytes > *expected_size_bytes && logical_end != file_blocks)
                {
                    return Err(FormatError::Invalid(
                        "logged write range is inconsistent with file size",
                    ));
                }
                Ok(())
            }
            LogOp::Truncate {
                object_id,
                logical_start,
                expected_size_bytes,
                new_size_bytes,
                content_crc,
                extents,
                ..
            } => {
                if *object_id == 0 {
                    return Err(FormatError::Invalid("logged truncate without object ID"));
                }
                if expected_size_bytes == new_size_bytes {
                    return Err(FormatError::Invalid("logged truncate does not change size"));
                }
                let blocks = validate_data_extents(extents)?;
                if blocks > 1 {
                    return Err(FormatError::Invalid(
                        "logged truncate has more than one tail block",
                    ));
                }
                if *new_size_bytes == 0 && blocks != 0 {
                    return Err(FormatError::Invalid("empty truncate carries tail data"));
                }
                if blocks == 0 {
                    if *logical_start != 0 || *content_crc != 0 {
                        return Err(FormatError::Invalid(
                            "data-free truncate carries tail fields",
                        ));
                    }
                } else if *new_size_bytes >= *expected_size_bytes
                    || new_size_bytes.is_multiple_of(crate::DEFAULT_BLOCK_SIZE as u64)
                    || *logical_start != *new_size_bytes / crate::DEFAULT_BLOCK_SIZE as u64
                {
                    return Err(FormatError::Invalid(
                        "logged truncate tail is inconsistent with file size",
                    ));
                }
                Ok(())
            }
        }
    }
}

fn validate_data_extents(extents: &[(u64, u32)]) -> Result<u64, FormatError> {
    if extents.len() > MAX_LOG_EXTENTS {
        return Err(FormatError::Invalid(
            "logged operation has too many extents",
        ));
    }
    let mut total = 0u64;
    for (index, (start, blocks)) in extents.iter().enumerate() {
        if *blocks == 0 {
            return Err(FormatError::Invalid("logged extent has zero blocks"));
        }
        let end = start
            .checked_add(*blocks as u64)
            .ok_or(FormatError::Invalid("logged extent end overflows"))?;
        for (other_start, other_blocks) in &extents[..index] {
            let other_end = *other_start + u64::from(*other_blocks);
            if *other_start < end && *start < other_end {
                return Err(FormatError::Invalid("logged data extents overlap"));
            }
        }
        total = total
            .checked_add(*blocks as u64)
            .ok_or(FormatError::Invalid("logged extent total overflows"))?;
    }
    Ok(total)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub uuid: [u8; 16],
    pub base_generation: u64,
    pub sequence: u32,
    pub ops: Vec<LogOp>,
}

impl LogRecord {
    pub fn encode(&self, block_size: usize) -> Result<Vec<u8>, FormatError> {
        if self.ops.is_empty() || self.ops.len() > MAX_LOG_OPS {
            return Err(FormatError::Invalid(
                "log record operation count out of range",
            ));
        }
        if self.sequence == 0 || self.base_generation == 0 {
            return Err(FormatError::Invalid("log record binding out of range"));
        }
        let payload_len = FIXED_PAYLOAD + self.ops.iter().map(LogOp::wire_len).sum::<usize>();
        if block_size < HEADER_SIZE || payload_len > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow("fsync group exceeds one log record"));
        }
        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        p[0..16].copy_from_slice(&self.uuid);
        le::put_u64(&mut p[16..24], self.base_generation);
        le::put_u32(&mut p[24..28], self.sequence);
        le::put_u16(&mut p[28..30], self.ops.len() as u16);
        let record_version = if self.ops.iter().any(LogOp::is_existing_file_update) {
            LOG_RECORD_VERSION
        } else {
            PREVIOUS_LOG_RECORD_VERSION
        };
        le::put_u16(&mut p[30..32], record_version);
        let mut offset = FIXED_PAYLOAD;
        for op in &self.ops {
            op.validate()?;
            let entry = &mut p[offset..];
            match op {
                LogOp::Create {
                    parent_id,
                    name,
                    expected_object_id,
                    size_bytes,
                    content_crc,
                    extents,
                    timestamp,
                } => {
                    entry[0] = 1;
                    le::put_u16(&mut entry[2..4], name.len() as u16);
                    le::put_u16(&mut entry[6..8], extents.len() as u16);
                    le::put_u64(&mut entry[8..16], *parent_id);
                    le::put_u64(&mut entry[24..32], *expected_object_id);
                    le::put_u64(&mut entry[32..40], *size_bytes);
                    le::put_u32(&mut entry[40..44], *content_crc);
                    timestamp.write(&mut entry[48..60]);
                    let mut at = OP_FIXED;
                    for (start, blocks) in extents {
                        le::put_u64(&mut entry[at..at + 8], *start);
                        le::put_u32(&mut entry[at + 8..at + 12], *blocks);
                        at += EXTENT_WIRE;
                    }
                    entry[at..at + name.len()].copy_from_slice(name);
                }
                LogOp::Delete {
                    parent_id,
                    name,
                    timestamp,
                } => {
                    entry[0] = 2;
                    le::put_u16(&mut entry[2..4], name.len() as u16);
                    le::put_u64(&mut entry[8..16], *parent_id);
                    timestamp.write(&mut entry[48..60]);
                    entry[OP_FIXED..OP_FIXED + name.len()].copy_from_slice(name);
                }
                LogOp::Rename {
                    source_parent_id,
                    source_name,
                    target_parent_id,
                    target_name,
                    replace,
                    timestamp,
                } => {
                    entry[0] = 3;
                    entry[1] = u8::from(*replace);
                    le::put_u16(&mut entry[2..4], source_name.len() as u16);
                    le::put_u16(&mut entry[4..6], target_name.len() as u16);
                    le::put_u64(&mut entry[8..16], *source_parent_id);
                    le::put_u64(&mut entry[16..24], *target_parent_id);
                    timestamp.write(&mut entry[48..60]);
                    let mut at = OP_FIXED;
                    entry[at..at + source_name.len()].copy_from_slice(source_name);
                    at += source_name.len();
                    entry[at..at + target_name.len()].copy_from_slice(target_name);
                }
                LogOp::Write {
                    object_id,
                    logical_start,
                    expected_size_bytes,
                    new_size_bytes,
                    content_crc,
                    extents,
                    timestamp,
                }
                | LogOp::Truncate {
                    object_id,
                    logical_start,
                    expected_size_bytes,
                    new_size_bytes,
                    content_crc,
                    extents,
                    timestamp,
                } => {
                    entry[0] = if matches!(op, LogOp::Write { .. }) {
                        4
                    } else {
                        5
                    };
                    le::put_u16(&mut entry[6..8], extents.len() as u16);
                    le::put_u64(&mut entry[8..16], *object_id);
                    le::put_u64(&mut entry[16..24], *logical_start);
                    le::put_u64(&mut entry[24..32], *expected_size_bytes);
                    le::put_u64(&mut entry[32..40], *new_size_bytes);
                    le::put_u32(&mut entry[40..44], *content_crc);
                    timestamp.write(&mut entry[48..60]);
                    let mut at = OP_FIXED;
                    for (start, blocks) in extents {
                        le::put_u64(&mut entry[at..at + 8], *start);
                        le::put_u32(&mut entry[at + 8..at + 12], *blocks);
                        at += EXTENT_WIRE;
                    }
                }
            }
            offset += op.wire_len();
        }

        BlockHeader {
            block_type: block_type::INTENT_LOG,
            flags: 0,
            owner: 0,
            generation: self.base_generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<LogRecord, FormatError> {
        let header = BlockHeader::verify(block, block_type::INTENT_LOG)?;
        let p = header.payload(block);
        // The record belongs to the volume and has no flag namespace
        // (ADR-114).
        if header.flags != 0 || header.owner != 0 {
            return Err(FormatError::Invalid(
                "log record header flags or owner are nonzero",
            ));
        }
        if p.len() < FIXED_PAYLOAD {
            return Err(FormatError::Invalid("log record payload too short"));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&p[0..16]);
        let base_generation = le::get_u64(&p[16..24]);
        if base_generation == 0 || base_generation != header.generation {
            return Err(FormatError::Invalid("log record generation inconsistent"));
        }
        let sequence = le::get_u32(&p[24..28]);
        if sequence == 0 {
            return Err(FormatError::Invalid("log record sequence is zero"));
        }
        let op_count = le::get_u16(&p[28..30]) as usize;
        if op_count == 0 || op_count > MAX_LOG_OPS {
            return Err(FormatError::Invalid(
                "log record operation count out of range",
            ));
        }
        // Version 0, the prototype record without an operation timestamp,
        // is refused rather than read (ADR-115); the writer emits version 2
        // for a namespace record and version 3 for a data update.
        let record_version = le::get_u16(&p[30..32]);
        if !matches!(
            record_version,
            PREVIOUS_LOG_RECORD_VERSION | LOG_RECORD_VERSION
        ) {
            return Err(FormatError::Invalid(
                "unsupported intent-log record version",
            ));
        }
        let op_fixed = OP_FIXED;
        let mut ops = Vec::with_capacity(op_count);
        let mut offset = FIXED_PAYLOAD;
        for _ in 0..op_count {
            if p.len() - offset < op_fixed {
                return Err(FormatError::Invalid("truncated log operation"));
            }
            let entry = &p[offset..];
            let op_type = entry[0];
            let source_len = le::get_u16(&entry[2..4]) as usize;
            let target_len = le::get_u16(&entry[4..6]) as usize;
            let extent_count = le::get_u16(&entry[6..8]) as usize;
            if extent_count > MAX_LOG_EXTENTS {
                return Err(FormatError::Invalid("logged create has too many extents"));
            }
            let variable = match op_type {
                1 if target_len == 0 => extent_count * EXTENT_WIRE + source_len,
                2 if target_len == 0 && extent_count == 0 => source_len,
                3 if extent_count == 0 => source_len + target_len,
                4 | 5
                    if record_version == LOG_RECORD_VERSION
                        && source_len == 0
                        && target_len == 0 =>
                {
                    extent_count * EXTENT_WIRE
                }
                _ => return Err(FormatError::Invalid("unknown log operation type")),
            };
            if p.len() - offset - op_fixed < variable {
                return Err(FormatError::Invalid("log operation exceeds payload"));
            }
            let timestamp = Timespec::read(&entry[48..60])?;
            let op = match op_type {
                1 => {
                    let mut extents = Vec::with_capacity(extent_count);
                    let mut at = op_fixed;
                    for _ in 0..extent_count {
                        extents.push((
                            le::get_u64(&entry[at..at + 8]),
                            le::get_u32(&entry[at + 8..at + 12]),
                        ));
                        at += EXTENT_WIRE;
                    }
                    LogOp::Create {
                        parent_id: le::get_u64(&entry[8..16]),
                        name: entry[at..at + source_len].to_vec(),
                        expected_object_id: le::get_u64(&entry[24..32]),
                        size_bytes: le::get_u64(&entry[32..40]),
                        content_crc: le::get_u32(&entry[40..44]),
                        extents,
                        timestamp,
                    }
                }
                2 => LogOp::Delete {
                    parent_id: le::get_u64(&entry[8..16]),
                    name: entry[op_fixed..op_fixed + source_len].to_vec(),
                    timestamp,
                },
                3 => {
                    let at = op_fixed;
                    LogOp::Rename {
                        source_parent_id: le::get_u64(&entry[8..16]),
                        source_name: entry[at..at + source_len].to_vec(),
                        target_parent_id: le::get_u64(&entry[16..24]),
                        target_name: entry[at + source_len..at + source_len + target_len].to_vec(),
                        replace: entry[1] != 0,
                        timestamp,
                    }
                }
                4 | 5 => {
                    let mut extents = Vec::with_capacity(extent_count);
                    let mut at = op_fixed;
                    for _ in 0..extent_count {
                        extents.push((
                            le::get_u64(&entry[at..at + 8]),
                            le::get_u32(&entry[at + 8..at + 12]),
                        ));
                        at += EXTENT_WIRE;
                    }
                    let fields = (
                        le::get_u64(&entry[8..16]),
                        le::get_u64(&entry[16..24]),
                        le::get_u64(&entry[24..32]),
                        le::get_u64(&entry[32..40]),
                        le::get_u32(&entry[40..44]),
                        timestamp,
                        extents,
                    );
                    if op_type == 4 {
                        LogOp::Write {
                            object_id: fields.0,
                            logical_start: fields.1,
                            expected_size_bytes: fields.2,
                            new_size_bytes: fields.3,
                            content_crc: fields.4,
                            timestamp: fields.5,
                            extents: fields.6,
                        }
                    } else {
                        LogOp::Truncate {
                            object_id: fields.0,
                            logical_start: fields.1,
                            expected_size_bytes: fields.2,
                            new_size_bytes: fields.3,
                            content_crc: fields.4,
                            timestamp: fields.5,
                            extents: fields.6,
                        }
                    }
                }
                _ => unreachable!("validated above"),
            };
            op.validate()?;
            offset += op.wire_len_with_fixed(op_fixed);
            ops.push(op);
        }
        if offset != header.payload_len as usize {
            return Err(FormatError::Invalid("log record payload length mismatch"));
        }
        Ok(LogRecord {
            uuid,
            base_generation,
            sequence,
            ops,
        })
    }
}
