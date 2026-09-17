//! Reclaim-queue wire structures (ADR-036).
//!
//! A FIFO of retired runs with a persistent consumption cursor:
//!
//! - the root block (`AFSH`) is COW'd every transaction and carries totals,
//!   the cursor, per-volume area capacities, and three areas: refs to sealed
//!   table blocks (oldest first), refs to sealed segment blocks not yet
//!   grouped into a table, and the newest inline entries;
//! - sealed segment blocks (`AFSS`) hold entries and are immutable;
//! - sealed table blocks (`AFSL`) hold segment refs and are immutable.
//!
//! Entries are runs: `(start LBA, block count, retire generation)`.
//! Consumption progress lives only in the root: `head_segment_offset`
//! (consumed segment refs inside the first table), `head_entry_offset`
//! (consumed entries inside the head sealed segment) and `head_block_offset`
//! (consumed blocks inside the head entry of that segment). Root-owned
//! entries and refs are simply dropped or rewritten in place when the root
//! is republished, so the cursor never refers to them.

use alloc::vec;
use alloc::vec::Vec;

use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError};

pub const ENTRY_WIRE_SIZE: usize = 20;
pub const REF_WIRE_SIZE: usize = 12;
const ROOT_FIXED: usize = 64;

/// Entries per sealed segment block: (4096 − 32 − 8) / 20.
pub const SEGMENT_ENTRY_CAP: usize = 202;
/// Segment refs per sealed table block: (4096 − 32 − 8) / 12.
pub const TABLE_REF_CAP: usize = 338;

/// One quarantined physical run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimEntry {
    pub start: u64,
    pub blocks: u32,
    pub retire_generation: u64,
}

impl ReclaimEntry {
    pub fn end(&self) -> Result<u64, FormatError> {
        self.start
            .checked_add(self.blocks as u64)
            .ok_or(FormatError::Invalid("reclaim run end overflows"))
    }

    fn validate(&self) -> Result<(), FormatError> {
        if self.blocks == 0 {
            return Err(FormatError::Invalid("reclaim run has zero blocks"));
        }
        if self.retire_generation == 0 {
            return Err(FormatError::Invalid("reclaim run has zero generation"));
        }
        self.end()?;
        Ok(())
    }

    fn write(&self, buf: &mut [u8]) {
        le::put_u64(&mut buf[0..8], self.start);
        le::put_u32(&mut buf[8..12], self.blocks);
        le::put_u64(&mut buf[12..20], self.retire_generation);
    }

    fn read(buf: &[u8]) -> Result<Self, FormatError> {
        let entry = ReclaimEntry {
            start: le::get_u64(&buf[0..8]),
            blocks: le::get_u32(&buf[8..12]),
            retire_generation: le::get_u64(&buf[12..20]),
        };
        entry.validate()?;
        Ok(entry)
    }
}

/// Reference to a sealed segment block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentRef {
    pub lba: u64,
    pub entry_count: u32,
}

/// Reference to a sealed table block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableRef {
    pub lba: u64,
    pub ref_count: u32,
}

/// Exact admission of the block around a payload (ADR-110): the queue
/// belongs to the volume, so the common header carries no flags and no
/// owner, and nothing follows the payload.
fn admit_envelope(block: &[u8], header: &BlockHeader) -> Result<(), FormatError> {
    if header.flags != 0 || header.owner != 0 {
        return Err(FormatError::Invalid(
            "reclaim block header flags or owner are nonzero",
        ));
    }
    if block[HEADER_SIZE + header.payload_len as usize..]
        .iter()
        .any(|byte| *byte != 0)
    {
        return Err(FormatError::Invalid("reclaim block unused tail is nonzero"));
    }
    Ok(())
}

fn write_ref(buf: &mut [u8], lba: u64, count: u32) {
    le::put_u64(&mut buf[0..8], lba);
    le::put_u32(&mut buf[8..12], count);
}

/// Per-volume root-area capacities, recorded in the root so crash tests can
/// force sealing/consumption with tiny areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimCaps {
    pub inline_entries: u16,
    pub segment_refs: u16,
    pub table_refs: u16,
}

impl Default for ReclaimCaps {
    fn default() -> Self {
        // 64 + 64×12 + 64×12 + 96×20 = 3520 payload bytes; capacity with
        // full sealed blocks ≈ 64 × 338 × 202 ≈ 4.4M entries (runs).
        ReclaimCaps {
            inline_entries: 96,
            segment_refs: 64,
            table_refs: 64,
        }
    }
}

impl ReclaimCaps {
    fn validate(&self, block_size: usize) -> Result<(), FormatError> {
        if self.inline_entries == 0 || self.segment_refs == 0 || self.table_refs == 0 {
            return Err(FormatError::Invalid("reclaim capacity is zero"));
        }
        if block_size < HEADER_SIZE || root_payload_len(*self)? > block_size - HEADER_SIZE {
            return Err(FormatError::Overflow("reclaim root areas exceed one block"));
        }
        Ok(())
    }
}

fn root_payload_len(caps: ReclaimCaps) -> Result<usize, FormatError> {
    (caps.table_refs as usize)
        .checked_mul(REF_WIRE_SIZE)
        .and_then(|t| {
            (caps.segment_refs as usize)
                .checked_mul(REF_WIRE_SIZE)
                .map(|s| (t, s))
        })
        .and_then(|(t, s)| {
            (caps.inline_entries as usize)
                .checked_mul(ENTRY_WIRE_SIZE)
                .and_then(|i| ROOT_FIXED.checked_add(t)?.checked_add(s)?.checked_add(i))
        })
        .ok_or(FormatError::Invalid("reclaim capacity overflow"))
}

/// Reclaim-queue root block.
///
/// Payload layout after the common header:
///
/// ```text
/// offset size field
/// 0      4    version (1)
/// 4      4    reserved (zero)
/// 8      8    pending blocks (appended − reclaimed)
/// 16     8    appended blocks, monotonic total
/// 24     8    reclaimed blocks, monotonic total
/// 32     4    head segment offset (consumed refs in the first table)
/// 36     4    head entry offset (consumed entries in the head segment)
/// 40     4    head block offset (consumed blocks in the head entry)
/// 44     2    inline entry capacity
/// 46     2    segment ref capacity
/// 48     2    table ref capacity
/// 50     2    reserved (zero)
/// 52     4    table ref count
/// 56     4    segment ref count
/// 60     4    inline entry count
/// 64     ...  table refs, then segment refs, then inline entries
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimRoot {
    pub pending_blocks: u64,
    pub appended_blocks_total: u64,
    pub reclaimed_blocks_total: u64,
    pub head_segment_offset: u32,
    pub head_entry_offset: u32,
    pub head_block_offset: u32,
    pub caps: ReclaimCaps,
    /// Oldest first.
    pub table_refs: Vec<TableRef>,
    /// Oldest first; newer than every table.
    pub segment_refs: Vec<SegmentRef>,
    /// Newest appended runs, oldest first.
    pub inline_entries: Vec<ReclaimEntry>,
}

impl ReclaimRoot {
    pub fn empty(caps: ReclaimCaps) -> Self {
        ReclaimRoot {
            pending_blocks: 0,
            appended_blocks_total: 0,
            reclaimed_blocks_total: 0,
            head_segment_offset: 0,
            head_entry_offset: 0,
            head_block_offset: 0,
            caps,
            table_refs: Vec::new(),
            segment_refs: Vec::new(),
            inline_entries: Vec::new(),
        }
    }

    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        self.validate(block_size)?;
        let payload_len = root_payload_len(self.caps)?;
        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], 1);
        le::put_u64(&mut p[8..16], self.pending_blocks);
        le::put_u64(&mut p[16..24], self.appended_blocks_total);
        le::put_u64(&mut p[24..32], self.reclaimed_blocks_total);
        le::put_u32(&mut p[32..36], self.head_segment_offset);
        le::put_u32(&mut p[36..40], self.head_entry_offset);
        le::put_u32(&mut p[40..44], self.head_block_offset);
        le::put_u16(&mut p[44..46], self.caps.inline_entries);
        le::put_u16(&mut p[46..48], self.caps.segment_refs);
        le::put_u16(&mut p[48..50], self.caps.table_refs);
        le::put_u32(&mut p[52..56], self.table_refs.len() as u32);
        le::put_u32(&mut p[56..60], self.segment_refs.len() as u32);
        le::put_u32(&mut p[60..64], self.inline_entries.len() as u32);
        let mut offset = ROOT_FIXED;
        for table in &self.table_refs {
            write_ref(
                &mut p[offset..offset + REF_WIRE_SIZE],
                table.lba,
                table.ref_count,
            );
            offset += REF_WIRE_SIZE;
        }
        offset = ROOT_FIXED + self.caps.table_refs as usize * REF_WIRE_SIZE;
        for segment in &self.segment_refs {
            write_ref(
                &mut p[offset..offset + REF_WIRE_SIZE],
                segment.lba,
                segment.entry_count,
            );
            offset += REF_WIRE_SIZE;
        }
        offset = ROOT_FIXED
            + (self.caps.table_refs as usize + self.caps.segment_refs as usize) * REF_WIRE_SIZE;
        for entry in &self.inline_entries {
            entry.write(&mut p[offset..offset + ENTRY_WIRE_SIZE]);
            offset += ENTRY_WIRE_SIZE;
        }

        BlockHeader {
            block_type: block_type::RECLAIM_ROOT,
            flags: 0,
            owner: 0,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<(ReclaimRoot, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::RECLAIM_ROOT)?;
        admit_envelope(block, &header)?;
        let p = header.payload(block);
        if p.len() < ROOT_FIXED {
            return Err(FormatError::Invalid("reclaim root payload too short"));
        }
        if le::get_u32(&p[0..4]) != 1 {
            return Err(FormatError::Invalid("unsupported reclaim root version"));
        }
        if p[4..8].iter().chain(&p[50..52]).any(|&byte| byte != 0) {
            return Err(FormatError::Invalid("reclaim root reserved bytes"));
        }
        let caps = ReclaimCaps {
            inline_entries: le::get_u16(&p[44..46]),
            segment_refs: le::get_u16(&p[46..48]),
            table_refs: le::get_u16(&p[48..50]),
        };
        // Bounds-first: capacities bound every later area read.
        caps.validate(block.len())?;
        if p.len() != root_payload_len(caps)? {
            return Err(FormatError::Invalid(
                "reclaim root payload length is not exact",
            ));
        }
        let table_count = le::get_u32(&p[52..56]) as usize;
        let segment_count = le::get_u32(&p[56..60]) as usize;
        let inline_count = le::get_u32(&p[60..64]) as usize;
        if table_count > caps.table_refs as usize
            || segment_count > caps.segment_refs as usize
            || inline_count > caps.inline_entries as usize
        {
            return Err(FormatError::Invalid(
                "reclaim root area count exceeds capacity",
            ));
        }
        let mut table_refs = Vec::with_capacity(table_count);
        let mut offset = ROOT_FIXED;
        for _ in 0..table_count {
            table_refs.push(TableRef {
                lba: le::get_u64(&p[offset..offset + 8]),
                ref_count: le::get_u32(&p[offset + 8..offset + 12]),
            });
            offset += REF_WIRE_SIZE;
        }
        let mut segment_refs = Vec::with_capacity(segment_count);
        offset = ROOT_FIXED + caps.table_refs as usize * REF_WIRE_SIZE;
        for _ in 0..segment_count {
            segment_refs.push(SegmentRef {
                lba: le::get_u64(&p[offset..offset + 8]),
                entry_count: le::get_u32(&p[offset + 8..offset + 12]),
            });
            offset += REF_WIRE_SIZE;
        }
        let mut inline_entries = Vec::with_capacity(inline_count);
        offset =
            ROOT_FIXED + (caps.table_refs as usize + caps.segment_refs as usize) * REF_WIRE_SIZE;
        for _ in 0..inline_count {
            inline_entries.push(ReclaimEntry::read(&p[offset..offset + ENTRY_WIRE_SIZE])?);
            offset += ENTRY_WIRE_SIZE;
        }
        // The unused slots of each area are zero: a rewrite encodes the
        // decoded items into a zeroed block and would drop anything else.
        let table_end = ROOT_FIXED + caps.table_refs as usize * REF_WIRE_SIZE;
        let segment_end = table_end + caps.segment_refs as usize * REF_WIRE_SIZE;
        let unused = [
            ROOT_FIXED + table_count * REF_WIRE_SIZE..table_end,
            table_end + segment_count * REF_WIRE_SIZE..segment_end,
            segment_end + inline_count * ENTRY_WIRE_SIZE..p.len(),
        ];
        if unused
            .into_iter()
            .any(|range| p[range].iter().any(|byte| *byte != 0))
        {
            return Err(FormatError::Invalid(
                "reclaim root unused area slots are nonzero",
            ));
        }
        let root = ReclaimRoot {
            pending_blocks: le::get_u64(&p[8..16]),
            appended_blocks_total: le::get_u64(&p[16..24]),
            reclaimed_blocks_total: le::get_u64(&p[24..32]),
            head_segment_offset: le::get_u32(&p[32..36]),
            head_entry_offset: le::get_u32(&p[36..40]),
            head_block_offset: le::get_u32(&p[40..44]),
            caps,
            table_refs,
            segment_refs,
            inline_entries,
        };
        root.validate(block.len())?;
        Ok((root, header.generation))
    }

    fn validate(&self, block_size: usize) -> Result<(), FormatError> {
        self.caps.validate(block_size)?;
        if self.table_refs.len() > self.caps.table_refs as usize
            || self.segment_refs.len() > self.caps.segment_refs as usize
            || self.inline_entries.len() > self.caps.inline_entries as usize
        {
            return Err(FormatError::Invalid(
                "reclaim root area count exceeds capacity",
            ));
        }
        for table in &self.table_refs {
            if table.ref_count == 0 || table.ref_count as usize > TABLE_REF_CAP {
                return Err(FormatError::Invalid("reclaim table ref count out of range"));
            }
        }
        for segment in &self.segment_refs {
            if segment.entry_count == 0 || segment.entry_count as usize > SEGMENT_ENTRY_CAP {
                return Err(FormatError::Invalid(
                    "reclaim segment ref count out of range",
                ));
            }
        }
        // The cursor refers exclusively to sealed blocks, in FIFO order.
        if self.table_refs.is_empty() {
            if self.head_segment_offset != 0 {
                return Err(FormatError::Invalid("reclaim cursor names a missing table"));
            }
        } else if self.head_segment_offset >= self.table_refs[0].ref_count {
            return Err(FormatError::Invalid(
                "reclaim cursor beyond the first table",
            ));
        }
        if self.table_refs.is_empty() && self.segment_refs.is_empty() {
            if self.head_entry_offset != 0 || self.head_block_offset != 0 {
                return Err(FormatError::Invalid(
                    "reclaim cursor names a missing segment",
                ));
            }
        } else if self.table_refs.is_empty()
            && self.head_entry_offset >= self.segment_refs[0].entry_count
        {
            return Err(FormatError::Invalid(
                "reclaim cursor beyond the head segment",
            ));
        }
        let pending = self
            .appended_blocks_total
            .checked_sub(self.reclaimed_blocks_total)
            .ok_or(FormatError::Invalid("reclaimed more blocks than appended"))?;
        if pending != self.pending_blocks {
            return Err(FormatError::Invalid(
                "reclaim totals do not match pending count",
            ));
        }
        Ok(())
    }
}

/// Sealed segment block: an immutable batch of entries, oldest first.
///
/// Payload: entry count (4), reserved (4), entries × 20.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReclaimSegment {
    pub entries: Vec<ReclaimEntry>,
}

impl ReclaimSegment {
    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        if self.entries.is_empty() || self.entries.len() > SEGMENT_ENTRY_CAP {
            return Err(FormatError::Invalid(
                "reclaim segment entry count out of range",
            ));
        }
        let payload_len = 8 + self.entries.len() * ENTRY_WIRE_SIZE;
        if payload_len > block_size.saturating_sub(HEADER_SIZE) {
            return Err(FormatError::Overflow("reclaim segment exceeds one block"));
        }
        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.entries.len() as u32);
        for (i, entry) in self.entries.iter().enumerate() {
            entry.validate()?;
            entry.write(&mut p[8 + i * ENTRY_WIRE_SIZE..8 + (i + 1) * ENTRY_WIRE_SIZE]);
        }
        BlockHeader {
            block_type: block_type::RECLAIM_SEGMENT,
            flags: 0,
            owner: 0,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<(ReclaimSegment, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::RECLAIM_SEGMENT)?;
        admit_envelope(block, &header)?;
        let p = header.payload(block);
        if p.len() < 8 {
            return Err(FormatError::Invalid("reclaim segment payload too short"));
        }
        if p[4..8].iter().any(|&byte| byte != 0) {
            return Err(FormatError::Invalid("reclaim sealed block reserved bytes"));
        }
        let count = le::get_u32(&p[0..4]) as usize;
        if count == 0 || count > SEGMENT_ENTRY_CAP || count > (p.len() - 8) / ENTRY_WIRE_SIZE {
            return Err(FormatError::Invalid(
                "reclaim segment entry count out of range",
            ));
        }
        if header.payload_len as usize != 8 + count * ENTRY_WIRE_SIZE {
            return Err(FormatError::Invalid(
                "reclaim segment payload length mismatch",
            ));
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            entries.push(ReclaimEntry::read(
                &p[8 + i * ENTRY_WIRE_SIZE..8 + (i + 1) * ENTRY_WIRE_SIZE],
            )?);
        }
        Ok((ReclaimSegment { entries }, header.generation))
    }
}

/// Sealed table block: an immutable batch of segment refs, oldest first.
///
/// Payload: ref count (4), reserved (4), refs × 12.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReclaimTable {
    pub refs: Vec<SegmentRef>,
}

impl ReclaimTable {
    pub fn encode(&self, block_size: usize, generation: u64) -> Result<Vec<u8>, FormatError> {
        if self.refs.is_empty() || self.refs.len() > TABLE_REF_CAP {
            return Err(FormatError::Invalid("reclaim table ref count out of range"));
        }
        let payload_len = 8 + self.refs.len() * REF_WIRE_SIZE;
        if payload_len > block_size.saturating_sub(HEADER_SIZE) {
            return Err(FormatError::Overflow("reclaim table exceeds one block"));
        }
        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u32(&mut p[0..4], self.refs.len() as u32);
        for (i, segment) in self.refs.iter().enumerate() {
            if segment.entry_count == 0 || segment.entry_count as usize > SEGMENT_ENTRY_CAP {
                return Err(FormatError::Invalid(
                    "reclaim table segment count out of range",
                ));
            }
            write_ref(
                &mut p[8 + i * REF_WIRE_SIZE..8 + (i + 1) * REF_WIRE_SIZE],
                segment.lba,
                segment.entry_count,
            );
        }
        BlockHeader {
            block_type: block_type::RECLAIM_TABLE,
            flags: 0,
            owner: 0,
            generation,
            payload_len: payload_len as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<(ReclaimTable, u64), FormatError> {
        let header = BlockHeader::verify(block, block_type::RECLAIM_TABLE)?;
        admit_envelope(block, &header)?;
        let p = header.payload(block);
        if p.len() < 8 {
            return Err(FormatError::Invalid("reclaim table payload too short"));
        }
        if p[4..8].iter().any(|&byte| byte != 0) {
            return Err(FormatError::Invalid("reclaim sealed block reserved bytes"));
        }
        let count = le::get_u32(&p[0..4]) as usize;
        if count == 0 || count > TABLE_REF_CAP || count > (p.len() - 8) / REF_WIRE_SIZE {
            return Err(FormatError::Invalid("reclaim table ref count out of range"));
        }
        if header.payload_len as usize != 8 + count * REF_WIRE_SIZE {
            return Err(FormatError::Invalid(
                "reclaim table payload length mismatch",
            ));
        }
        let mut refs = Vec::with_capacity(count);
        for i in 0..count {
            let offset = 8 + i * REF_WIRE_SIZE;
            let segment = SegmentRef {
                lba: le::get_u64(&p[offset..offset + 8]),
                entry_count: le::get_u32(&p[offset + 8..offset + 12]),
            };
            if segment.entry_count == 0 || segment.entry_count as usize > SEGMENT_ENTRY_CAP {
                return Err(FormatError::Invalid(
                    "reclaim table segment count out of range",
                ));
            }
            refs.push(segment);
        }
        Ok((ReclaimTable { refs }, header.generation))
    }
}
