//! Region allocator prototype (architecture blocker 3, step 6 of the
//! first-contributor plan).
//!
//! Free-space state lives in per-region bitmap pages written to *reserved*
//! generational slot blocks (three per region, `geometry`), chosen so the
//! states referenced by both retained checkpoints are never overwritten.
//! Because the pages are never allocated through the allocator they
//! describe, the "allocate bitmap COW → bitmap changes → allocate again"
//! recursion does not exist.
//!
//! Retired/quarantine semantics: a block that becomes unreachable in
//! transaction N keeps its allocated bit and is listed in the retired list
//! committed with generation N. The next transaction (N + 1) *promotes* all
//! committed retired entries — clears their bits — because the only
//! checkpoint still selectable after N is durable is N itself, which does
//! not reference them. A block is therefore never reusable in the same
//! transaction that freed it, and always survives a crash back to the
//! newest committed state.

use std::collections::BTreeSet;

use afsplus_block::BlockDevice;
use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::{Checkpoint, RegionRecord};
use afsplus_format::geometry::{Geometry, BITMAP_SLOTS};
use afsplus_format::retired::{RetiredEntry, RetiredList};

use crate::CoreError;

/// Committed in-memory allocation state: one decoded page per region.
///
/// The prototype loads every region at mount; a bounded-memory
/// implementation loads regions on demand (`docs/02-architecture.md` §2) —
/// `ram_bytes` exists so that cost is measured, not guessed.
#[derive(Debug, Clone)]
pub struct Bitmaps {
    pub geo: Geometry,
    pub pages: Vec<BitmapPage>,
}

impl Bitmaps {
    /// Loads and validates the allocation state referenced by a checkpoint.
    /// Every page must decode, belong to the right region, and carry exactly
    /// the generation and free count the checkpoint recorded.
    pub fn load<D: BlockDevice>(
        dev: &mut D,
        geo: &Geometry,
        checkpoint: &Checkpoint,
    ) -> Result<Bitmaps, CoreError> {
        let mut buf = vec![0u8; geo.block_size];
        let mut pages = Vec::with_capacity(checkpoint.regions.len());
        for (r, record) in checkpoint.regions.iter().enumerate() {
            let region = r as u32;
            let lba = geo.bitmap_slot_lba(region, record.slot);
            dev.read_block(lba, &mut buf)?;
            let (page, generation) = BitmapPage::decode(&buf)
                .map_err(|e| CoreError::Corrupt(format!("region {region} bitmap: {e}")))?;
            if page.region != region || page.valid_blocks != geo.region_valid_blocks(region) {
                return Err(CoreError::Corrupt(format!("region {region} bitmap geometry mismatch")));
            }
            if generation != record.bitmap_generation {
                return Err(CoreError::Corrupt(format!(
                    "region {region} bitmap generation {generation} does not match checkpoint record {}",
                    record.bitmap_generation
                )));
            }
            if page.free_blocks() != record.free_blocks {
                return Err(CoreError::Corrupt(format!(
                    "region {region} free count mismatch: bitmap {}, checkpoint {}",
                    page.free_blocks(),
                    record.free_blocks
                )));
            }
            pages.push(page);
        }
        Ok(Bitmaps { geo: *geo, pages })
    }

    pub fn is_allocated(&self, lba: u64) -> bool {
        let region = self.geo.region_of(lba);
        let index = (lba - self.geo.region_base(region)) as u32;
        self.pages[region as usize].is_allocated(index)
    }

    fn set_allocated(&mut self, lba: u64, allocated: bool) -> bool {
        let region = self.geo.region_of(lba);
        let index = (lba - self.geo.region_base(region)) as u32;
        self.pages[region as usize].set_allocated(index, allocated)
    }

    pub fn free_blocks_total(&self) -> u64 {
        self.pages.iter().map(|p| p.free_blocks() as u64).sum()
    }

    /// Bytes of allocator state held in memory (the loaded bitmap pages).
    pub fn ram_bytes(&self) -> usize {
        self.pages.iter().map(|p| p.bits.len()).sum()
    }
}

/// Measured allocator behavior for one transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AllocStats {
    pub blocks_allocated: u64,
    pub blocks_retired: u64,
    pub blocks_promoted: u64,
    /// Generations spent in quarantine by each promoted block, summed
    /// (divide by `blocks_promoted` for the mean; 1 by construction today).
    pub reclaim_latency_generations: u64,
    pub bitmap_pages_dirty: u64,
    pub allocator_ram_bytes: u64,
}

/// Working allocation state of one in-flight transaction. Dropped on any
/// error, leaving the committed state untouched; adopted on durable commit.
pub struct TxAllocator {
    working: Bitmaps,
    new_generation: u64,
    dirty_regions: BTreeSet<u32>,
    new_retired: RetiredList,
    stats: AllocStats,
}

impl TxAllocator {
    /// Starts a transaction: clones the committed bitmaps and promotes every
    /// committed retired entry (they are unreachable from the only still
    /// selectable checkpoint, so their quarantine ends now).
    pub fn begin(
        committed: &Bitmaps,
        committed_retired: &RetiredList,
        new_generation: u64,
    ) -> Result<TxAllocator, CoreError> {
        let mut tx = TxAllocator {
            working: committed.clone(),
            new_generation,
            dirty_regions: BTreeSet::new(),
            new_retired: RetiredList::default(),
            stats: AllocStats::default(),
        };
        for entry in &committed_retired.entries {
            if !tx.working.set_allocated(entry.lba, false) {
                return Err(CoreError::Corrupt(format!(
                    "retired block {} was not marked allocated",
                    entry.lba
                )));
            }
            tx.dirty_regions.insert(tx.working.geo.region_of(entry.lba));
            tx.stats.blocks_promoted += 1;
            tx.stats.reclaim_latency_generations +=
                new_generation.saturating_sub(entry.retire_generation);
        }
        Ok(tx)
    }

    /// Allocates one block, lowest-address first fit.
    pub fn allocate(&mut self) -> Result<u64, CoreError> {
        self.allocate_run(1)
    }

    /// Allocates `len` physically contiguous blocks, lowest first fit.
    pub fn allocate_run(&mut self, len: u64) -> Result<u64, CoreError> {
        assert!(len > 0);
        let geo = self.working.geo;
        let mut run_start = None;
        let mut run_len = 0u64;
        for lba in 0..geo.total_blocks {
            let usable = geo.is_allocatable(lba) && !self.working.is_allocated(lba);
            if usable {
                if run_len == 0 {
                    run_start = Some(lba);
                }
                run_len += 1;
                if run_len == len {
                    let start = run_start.unwrap();
                    for b in start..start + len {
                        self.working.set_allocated(b, true);
                        self.dirty_regions.insert(geo.region_of(b));
                    }
                    self.stats.blocks_allocated += len;
                    return Ok(start);
                }
            } else {
                run_start = None;
                run_len = 0;
            }
        }
        Err(CoreError::NoSpace)
    }

    /// Retires a block that the new state no longer reaches. Its bit stays
    /// allocated; it enters quarantine via the new retired list.
    pub fn retire(&mut self, lba: u64) -> Result<(), CoreError> {
        if !self.working.geo.is_allocatable(lba) || !self.working.is_allocated(lba) {
            return Err(CoreError::Corrupt(format!("retiring block {lba} that is not live")));
        }
        self.new_retired
            .insert(lba, self.new_generation)
            .map_err(|_| CoreError::Corrupt(format!("block {lba} retired twice")))?;
        self.stats.blocks_retired += 1;
        Ok(())
    }

    pub fn new_retired(&self) -> &RetiredList {
        &self.new_retired
    }

    /// Finalizes the transaction's allocation state: encoded bitmap pages
    /// for every dirty region (each written to a slot that neither retained
    /// checkpoint references) and the region records for the new checkpoint.
    pub fn finish(
        mut self,
        current: &Checkpoint,
        other: Option<&Checkpoint>,
    ) -> Result<FinishedAlloc, CoreError> {
        let geo = self.working.geo;
        let mut page_writes = Vec::new();
        let mut records = Vec::with_capacity(current.regions.len());
        for (r, current_record) in current.regions.iter().enumerate() {
            let region = r as u32;
            if self.dirty_regions.contains(&region) {
                let slot = choose_slot(
                    current_record.slot,
                    other.and_then(|c| c.regions.get(r)).map(|rec| rec.slot),
                );
                let page = &self.working.pages[r];
                let encoded = page
                    .encode(geo.block_size, self.new_generation)
                    .map_err(CoreError::Format)?;
                page_writes.push((geo.bitmap_slot_lba(region, slot), encoded));
                records.push(RegionRecord {
                    slot,
                    free_blocks: page.free_blocks(),
                    bitmap_generation: self.new_generation,
                });
            } else {
                records.push(*current_record);
            }
        }
        self.stats.bitmap_pages_dirty = page_writes.len() as u64;
        self.stats.allocator_ram_bytes = self.working.ram_bytes() as u64;
        Ok(FinishedAlloc {
            bitmaps: self.working,
            page_writes,
            records,
            retired: self.new_retired,
            stats: self.stats,
        })
    }
}

pub struct FinishedAlloc {
    /// The working bitmaps, to adopt as committed state after a durable commit.
    pub bitmaps: Bitmaps,
    /// (lba, encoded page) for every dirty region.
    pub page_writes: Vec<(u64, Vec<u8>)>,
    /// Region records for the new checkpoint, in region order.
    pub records: Vec<RegionRecord>,
    pub retired: RetiredList,
    pub stats: AllocStats,
}

/// Picks the smallest bitmap slot referenced by neither retained checkpoint.
/// With three slots and at most two retained references, one always remains.
fn choose_slot(current: u8, other: Option<u8>) -> u8 {
    (0..BITMAP_SLOTS)
        .find(|s| *s != current && Some(*s) != other)
        .expect("three slots minus at most two references")
}

/// Convenience for tests and the checker.
pub fn retired_entries(list: &RetiredList) -> &[RetiredEntry] {
    &list.entries
}

#[cfg(test)]
mod tests {
    use super::choose_slot;

    #[test]
    fn slot_choice_avoids_both_retained_references() {
        assert_eq!(choose_slot(0, Some(1)), 2);
        assert_eq!(choose_slot(1, Some(0)), 2);
        assert_eq!(choose_slot(2, Some(0)), 1);
        assert_eq!(choose_slot(2, Some(1)), 0);
        assert_eq!(choose_slot(0, None), 1);
        assert_eq!(choose_slot(0, Some(0)), 1);
    }
}
