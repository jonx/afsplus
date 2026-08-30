//! Reclaim-queue engine (ADR-036): bounded batch consumption, run appends,
//! append-only sealing, and the exhaustive walk used by the checker.
//!
//! Lifecycle inside one transaction, driven by `TxAllocator`:
//!
//! 1. [`ReclaimTx::begin`] reads the committed root and consumes up to the
//!    block budget from the FIFO head, returning the promoted runs (whose
//!    bitmap bits the allocator clears) and recording fully consumed
//!    segment/table blocks.
//! 2. Operations append retired runs ([`ReclaimTx::append_run`]).
//! 3. [`ReclaimTx::plan`] folds the consumed structure blocks and the old
//!    root into the append list and returns exactly how many fresh blocks
//!    the rebuild needs — computable before any allocation, so there is no
//!    append/allocate fixpoint.
//! 4. [`ReclaimTx::build`] seals overflowing areas into immutable segment
//!    and table blocks and encodes the new root, consuming the allocated
//!    LBAs in order.

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::reclaim::{
    ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable, SegmentRef, TableRef,
    SEGMENT_ENTRY_CAP, TABLE_REF_CAP,
};

use crate::CoreError;

/// Default per-transaction reclamation budget, in blocks.
pub const DEFAULT_RECLAIM_BATCH_BLOCKS: u64 = 4096;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReclaimStats {
    pub entries_appended: u64,
    pub segments_sealed: u64,
    pub tables_sealed: u64,
    /// Segment/table/root blocks written by this transaction.
    pub structure_blocks_written: u64,
    /// Queue blocks (segments/tables/old root) retired by this transaction.
    pub structure_blocks_retired: u64,
    pub pending_blocks_after: u64,
}

/// A run whose blocks leave quarantine in this transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromotedRun {
    pub start: u64,
    pub blocks: u32,
    pub retire_generation: u64,
}

pub struct ReclaimBuild {
    pub root_lba: u64,
    pub writes: Vec<(u64, Vec<u8>)>,
    pub pending_blocks: u64,
    pub stats: ReclaimStats,
}

pub struct ReclaimTx {
    committed_root_lba: u64,
    new_generation: u64,
    /// Working root: consumption already applied to its arrays and cursor.
    root: ReclaimRoot,
    /// Runs appended by this transaction, FIFO.
    appends: Vec<ReclaimEntry>,
    /// Runs promoted by this transaction's batch.
    promoted: Vec<PromotedRun>,
    /// Segment/table blocks fully consumed by the batch; they leave the root
    /// in this rebuild and re-enter the queue as retired runs.
    consumed_structure: Vec<u64>,
    reclaimed_blocks: u64,
    planned_allocations: Option<usize>,
    stats: ReclaimStats,
}

/// Validates that a run stays within one region's allocatable span; runs
/// produced by the allocator never cross a region head.
pub fn validate_run(geo: &Geometry, entry: &ReclaimEntry) -> Result<(), CoreError> {
    let end = entry.end().map_err(CoreError::Format)?;
    if end > geo.total_blocks || !geo.is_allocatable(entry.start) {
        return Err(CoreError::Corrupt(format!(
            "reclaim run {}+{} outside allocatable bounds",
            entry.start, entry.blocks
        )));
    }
    if geo.region_of(entry.start) != geo.region_of(end - 1) {
        return Err(CoreError::Corrupt(format!(
            "reclaim run {}+{} crosses a region boundary",
            entry.start, entry.blocks
        )));
    }
    Ok(())
}

fn read_root<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    committed_generation: u64,
) -> Result<ReclaimRoot, CoreError> {
    if !geo.is_allocatable(root_lba) {
        return Err(CoreError::Corrupt(format!(
            "reclaim root block {root_lba} outside allocatable bounds"
        )));
    }
    let mut buf = vec![0u8; geo.block_size];
    dev.read_block(root_lba, &mut buf)?;
    let (root, generation) =
        ReclaimRoot::decode(&buf).map_err(|e| CoreError::Corrupt(format!("reclaim root: {e}")))?;
    if generation > committed_generation {
        return Err(CoreError::Corrupt(
            "reclaim root generation is from the future".into(),
        ));
    }
    for entry in &root.inline_entries {
        validate_run(geo, entry)?;
        if entry.retire_generation > committed_generation {
            return Err(CoreError::Corrupt(
                "reclaim entry generation is from the future".into(),
            ));
        }
    }
    for reference in root
        .table_refs
        .iter()
        .map(|t| t.lba)
        .chain(root.segment_refs.iter().map(|s| s.lba))
    {
        if !geo.is_allocatable(reference) {
            return Err(CoreError::Corrupt(format!(
                "reclaim structure block {reference} outside allocatable bounds"
            )));
        }
    }
    Ok(root)
}

fn read_segment<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    reference: SegmentRef,
    committed_generation: u64,
) -> Result<ReclaimSegment, CoreError> {
    let mut buf = vec![0u8; geo.block_size];
    dev.read_block(reference.lba, &mut buf)?;
    let (segment, generation) = ReclaimSegment::decode(&buf)
        .map_err(|e| CoreError::Corrupt(format!("reclaim segment {}: {e}", reference.lba)))?;
    if generation > committed_generation || segment.entries.len() != reference.entry_count as usize
    {
        return Err(CoreError::Corrupt(format!(
            "reclaim segment {} does not match its reference",
            reference.lba
        )));
    }
    for entry in &segment.entries {
        validate_run(geo, entry)?;
        if entry.retire_generation > committed_generation {
            return Err(CoreError::Corrupt(
                "reclaim entry generation is from the future".into(),
            ));
        }
    }
    Ok(segment)
}

fn read_table<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    reference: TableRef,
    committed_generation: u64,
) -> Result<ReclaimTable, CoreError> {
    let mut buf = vec![0u8; geo.block_size];
    dev.read_block(reference.lba, &mut buf)?;
    let (table, generation) = ReclaimTable::decode(&buf)
        .map_err(|e| CoreError::Corrupt(format!("reclaim table {}: {e}", reference.lba)))?;
    if generation > committed_generation || table.refs.len() != reference.ref_count as usize {
        return Err(CoreError::Corrupt(format!(
            "reclaim table {} does not match its reference",
            reference.lba
        )));
    }
    for segment in &table.refs {
        if !geo.is_allocatable(segment.lba) {
            return Err(CoreError::Corrupt(format!(
                "reclaim segment block {} outside allocatable bounds",
                segment.lba
            )));
        }
    }
    Ok(table)
}

impl ReclaimTx {
    /// Reads the committed queue and consumes up to `batch_blocks` from its
    /// head. The caller clears the returned runs' bitmap bits.
    pub fn begin<D: BlockDevice>(
        dev: &mut D,
        geo: &Geometry,
        committed_root_lba: u64,
        committed_generation: u64,
        new_generation: u64,
        batch_blocks: u64,
    ) -> Result<ReclaimTx, CoreError> {
        let root = read_root(dev, geo, committed_root_lba, committed_generation)?;
        let mut tx = ReclaimTx {
            committed_root_lba,
            new_generation,
            root,
            appends: Vec::new(),
            promoted: Vec::new(),
            consumed_structure: Vec::new(),
            reclaimed_blocks: 0,
            planned_allocations: None,
            stats: ReclaimStats::default(),
        };
        tx.consume(dev, geo, committed_generation, batch_blocks)?;
        Ok(tx)
    }

    pub fn promoted_runs(&self) -> &[PromotedRun] {
        &self.promoted
    }

    /// Consumes up to `budget` blocks from the FIFO head, updating the
    /// working root's arrays and cursor in place.
    fn consume<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        geo: &Geometry,
        committed_generation: u64,
        mut budget: u64,
    ) -> Result<(), CoreError> {
        while budget > 0 {
            if let Some(table_ref) = self.root.table_refs.first().copied() {
                let table = read_table(dev, geo, table_ref, committed_generation)?;
                let segment_ref = table.refs[self.root.head_segment_offset as usize];
                let finished_segment =
                    self.consume_segment(dev, geo, segment_ref, committed_generation, &mut budget)?;
                if finished_segment {
                    self.consumed_structure.push(segment_ref.lba);
                    self.root.head_segment_offset += 1;
                    self.root.head_entry_offset = 0;
                    self.root.head_block_offset = 0;
                    if self.root.head_segment_offset as usize == table.refs.len() {
                        self.consumed_structure.push(table_ref.lba);
                        self.root.table_refs.remove(0);
                        self.root.head_segment_offset = 0;
                    }
                }
            } else if let Some(segment_ref) = self.root.segment_refs.first().copied() {
                let finished_segment =
                    self.consume_segment(dev, geo, segment_ref, committed_generation, &mut budget)?;
                if finished_segment {
                    self.consumed_structure.push(segment_ref.lba);
                    self.root.segment_refs.remove(0);
                    self.root.head_entry_offset = 0;
                    self.root.head_block_offset = 0;
                }
            } else if let Some(entry) = self.root.inline_entries.first_mut() {
                let take = (entry.blocks as u64).min(budget) as u32;
                self.promoted.push(PromotedRun {
                    start: entry.start,
                    blocks: take,
                    retire_generation: entry.retire_generation,
                });
                entry.start += take as u64;
                entry.blocks -= take;
                budget -= take as u64;
                self.reclaimed_blocks += take as u64;
                if entry.blocks == 0 {
                    self.root.inline_entries.remove(0);
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Consumes entries from one sealed segment; returns whether the segment
    /// is now fully consumed. Progress is held in the root's cursor.
    fn consume_segment<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        geo: &Geometry,
        segment_ref: SegmentRef,
        committed_generation: u64,
        budget: &mut u64,
    ) -> Result<bool, CoreError> {
        let segment = read_segment(dev, geo, segment_ref, committed_generation)?;
        if self.root.head_entry_offset as usize >= segment.entries.len() {
            return Err(CoreError::Corrupt(
                "reclaim cursor beyond the head segment".into(),
            ));
        }
        while *budget > 0 && (self.root.head_entry_offset as usize) < segment.entries.len() {
            let entry = segment.entries[self.root.head_entry_offset as usize];
            if self.root.head_block_offset >= entry.blocks {
                return Err(CoreError::Corrupt(
                    "reclaim cursor beyond the head run".into(),
                ));
            }
            let remaining = entry.blocks - self.root.head_block_offset;
            let take = (remaining as u64).min(*budget) as u32;
            self.promoted.push(PromotedRun {
                start: entry.start + self.root.head_block_offset as u64,
                blocks: take,
                retire_generation: entry.retire_generation,
            });
            *budget -= take as u64;
            self.reclaimed_blocks += take as u64;
            self.root.head_block_offset += take;
            if self.root.head_block_offset == entry.blocks {
                self.root.head_entry_offset += 1;
                self.root.head_block_offset = 0;
            }
        }
        Ok(self.root.head_entry_offset as usize == segment.entries.len())
    }

    /// Appends a run retired by this transaction.
    pub fn append_run(&mut self, geo: &Geometry, start: u64, blocks: u32) -> Result<(), CoreError> {
        let entry = ReclaimEntry {
            start,
            blocks,
            retire_generation: self.new_generation,
        };
        validate_run(geo, &entry)?;
        self.appends.push(entry);
        Ok(())
    }

    /// Folds the consumed structure blocks and the previous root into the
    /// append list and returns the number of fresh blocks the rebuild needs
    /// (sealed segments + sealed tables + the new root).
    pub fn plan(&mut self) -> Result<usize, CoreError> {
        if self.planned_allocations.is_some() {
            return Err(CoreError::Corrupt("reclaim rebuild planned twice".into()));
        }
        let mut structure_retired = std::mem::take(&mut self.consumed_structure);
        structure_retired.push(self.committed_root_lba);
        self.stats.structure_blocks_retired = structure_retired.len() as u64;
        for lba in structure_retired {
            self.appends.push(ReclaimEntry {
                start: lba,
                blocks: 1,
                retire_generation: self.new_generation,
            });
        }
        self.stats.entries_appended = self.appends.len() as u64;

        let caps = self.root.caps;
        let mut inline_len = self.root.inline_entries.len() + self.appends.len();
        let mut segments_sealed = 0usize;
        while inline_len > caps.inline_entries as usize {
            inline_len -= SEGMENT_ENTRY_CAP.min(inline_len);
            segments_sealed += 1;
        }
        let mut refs_len = self.root.segment_refs.len() + segments_sealed;
        let mut tables_sealed = 0usize;
        while refs_len > caps.segment_refs as usize {
            refs_len -= TABLE_REF_CAP.min(refs_len);
            tables_sealed += 1;
        }
        if self.root.table_refs.len() + tables_sealed > caps.table_refs as usize {
            return Err(CoreError::PrototypeLimit(
                "reclaim backlog exceeds prototype capacity",
            ));
        }
        let allocations = segments_sealed + tables_sealed + 1;
        self.planned_allocations = Some(allocations);
        Ok(allocations)
    }

    /// Seals and encodes the rebuilt queue, consuming exactly the planned
    /// LBAs in order (segments, then tables, then the new root).
    pub fn build(mut self, geo: &Geometry, lbas: Vec<u64>) -> Result<ReclaimBuild, CoreError> {
        let planned = self
            .planned_allocations
            .ok_or_else(|| CoreError::Corrupt("reclaim rebuild was not planned".into()))?;
        if lbas.len() != planned {
            return Err(CoreError::Corrupt(
                "reclaim rebuild allocation count mismatch".into(),
            ));
        }
        let mut lbas = lbas.into_iter();
        let caps = self.root.caps;
        let block_size = geo.block_size;
        let mut writes = Vec::new();

        let appended_blocks: u64 = self
            .appends
            .iter()
            .try_fold(0u64, |sum, entry| sum.checked_add(entry.blocks as u64))
            .ok_or_else(|| CoreError::Corrupt("reclaim append total overflows".into()))?;
        let mut combined = std::mem::take(&mut self.root.inline_entries);
        combined.append(&mut self.appends);

        while combined.len() > caps.inline_entries as usize {
            let take = SEGMENT_ENTRY_CAP.min(combined.len());
            let entries: Vec<_> = combined.drain(..take).collect();
            let lba = lbas.next().expect("planned segment allocation");
            let segment = ReclaimSegment { entries };
            writes.push((lba, segment.encode(block_size, self.new_generation)?));
            self.root.segment_refs.push(SegmentRef {
                lba,
                entry_count: segment.entries.len() as u32,
            });
            self.stats.segments_sealed += 1;
        }
        self.root.inline_entries = combined;

        while self.root.segment_refs.len() > caps.segment_refs as usize {
            let take = TABLE_REF_CAP.min(self.root.segment_refs.len());
            let refs: Vec<_> = self.root.segment_refs.drain(..take).collect();
            let lba = lbas.next().expect("planned table allocation");
            let ref_count = refs.len() as u32;
            let table = ReclaimTable { refs };
            writes.push((lba, table.encode(block_size, self.new_generation)?));
            self.root.table_refs.push(TableRef { lba, ref_count });
            self.stats.tables_sealed += 1;
        }

        self.root.appended_blocks_total = self
            .root
            .appended_blocks_total
            .checked_add(appended_blocks)
            .ok_or_else(|| CoreError::Corrupt("reclaim appended total overflows".into()))?;
        self.root.reclaimed_blocks_total = self
            .root
            .reclaimed_blocks_total
            .checked_add(self.reclaimed_blocks)
            .ok_or_else(|| CoreError::Corrupt("reclaim reclaimed total overflows".into()))?;
        self.root.pending_blocks = self
            .root
            .appended_blocks_total
            .checked_sub(self.root.reclaimed_blocks_total)
            .ok_or_else(|| CoreError::Corrupt("reclaimed more blocks than appended".into()))?;

        let root_lba = lbas.next().expect("planned root allocation");
        debug_assert!(lbas.next().is_none());
        writes.push((root_lba, self.root.encode(block_size, self.new_generation)?));
        self.stats.structure_blocks_written = writes.len() as u64;
        self.stats.pending_blocks_after = self.root.pending_blocks;
        Ok(ReclaimBuild {
            root_lba,
            writes,
            pending_blocks: self.root.pending_blocks,
            stats: self.stats,
        })
    }
}

/// Exhaustively loaded queue state for the checker.
pub struct LoadedReclaim {
    /// Every unconsumed run, cursor-adjusted, FIFO order.
    pub runs: Vec<ReclaimEntry>,
    /// Root, table, and segment blocks still referenced by the queue.
    pub structure_blocks: Vec<u64>,
    pub pending_blocks: u64,
}

/// Walks the whole queue (checker/shadow verification; not bounded work).
pub fn load_all<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    committed_generation: u64,
) -> Result<LoadedReclaim, CoreError> {
    let root = read_root(dev, geo, root_lba, committed_generation)?;
    let mut runs = Vec::new();
    let mut structure_blocks = vec![root_lba];

    let push_segment = |dev: &mut D,
                        structure_blocks: &mut Vec<u64>,
                        runs: &mut Vec<ReclaimEntry>,
                        reference: SegmentRef,
                        entry_offset: u32,
                        block_offset: u32|
     -> Result<(), CoreError> {
        structure_blocks.push(reference.lba);
        let segment = read_segment(dev, geo, reference, committed_generation)?;
        if entry_offset as usize > segment.entries.len()
            || (entry_offset as usize == segment.entries.len() && block_offset != 0)
        {
            return Err(CoreError::Corrupt(
                "reclaim cursor beyond the head segment".into(),
            ));
        }
        for (index, entry) in segment
            .entries
            .iter()
            .enumerate()
            .skip(entry_offset as usize)
        {
            let mut entry = *entry;
            if index == entry_offset as usize && block_offset != 0 {
                if block_offset >= entry.blocks {
                    return Err(CoreError::Corrupt(
                        "reclaim cursor beyond the head run".into(),
                    ));
                }
                entry.start += block_offset as u64;
                entry.blocks -= block_offset;
            }
            runs.push(entry);
        }
        Ok(())
    };

    for (table_index, table_ref) in root.table_refs.iter().enumerate() {
        structure_blocks.push(table_ref.lba);
        let table = read_table(dev, geo, *table_ref, committed_generation)?;
        let start_ref = if table_index == 0 {
            root.head_segment_offset as usize
        } else {
            0
        };
        for (ref_index, segment_ref) in table.refs.iter().enumerate().skip(start_ref) {
            let (entry_offset, block_offset) = if table_index == 0 && ref_index == start_ref {
                (root.head_entry_offset, root.head_block_offset)
            } else {
                (0, 0)
            };
            push_segment(
                dev,
                &mut structure_blocks,
                &mut runs,
                *segment_ref,
                entry_offset,
                block_offset,
            )?;
        }
    }
    for (index, segment_ref) in root.segment_refs.iter().enumerate() {
        let (entry_offset, block_offset) = if root.table_refs.is_empty() && index == 0 {
            (root.head_entry_offset, root.head_block_offset)
        } else {
            (0, 0)
        };
        push_segment(
            dev,
            &mut structure_blocks,
            &mut runs,
            *segment_ref,
            entry_offset,
            block_offset,
        )?;
    }
    runs.extend(root.inline_entries.iter().copied());

    let counted: u64 = runs
        .iter()
        .try_fold(0u64, |sum, run| sum.checked_add(run.blocks as u64))
        .ok_or_else(|| CoreError::Corrupt("reclaim pending total overflows".into()))?;
    if counted != root.pending_blocks {
        return Err(CoreError::Corrupt(format!(
            "reclaim root claims {} pending blocks, queue holds {counted}",
            root.pending_blocks
        )));
    }
    Ok(LoadedReclaim {
        runs,
        structure_blocks,
        pending_blocks: root.pending_blocks,
    })
}

/// Diagnostic: whether `lba` is currently quarantined. Walks the queue.
pub fn contains<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    committed_generation: u64,
    lba: u64,
) -> Result<bool, CoreError> {
    let loaded = load_all(dev, geo, root_lba, committed_generation)?;
    for run in loaded.runs {
        if lba >= run.start && lba < run.end().map_err(CoreError::Format)? {
            return Ok(true);
        }
    }
    Ok(false)
}
