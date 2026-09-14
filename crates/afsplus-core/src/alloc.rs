//! Region allocator with COW region descriptors and multi-page bitmaps.
//!
//! Every region reserves three descriptor slots and three slots for each
//! logical bitmap page. A checkpoint selects one descriptor; the descriptor
//! selects the pages. Dirty pages and the replacement descriptor are written
//! to slots referenced by neither retained checkpoint before publication.
//!
//! A block run removed in transaction N remains allocated and enters the
//! reclaim queue (ADR-036). A later transaction clears those bits only in
//! bounded batches, after N is the newest durable checkpoint, so no physical
//! block is reused while a selectable checkpoint can still reach its previous
//! contents. Blocks allocated by an in-flight transaction and discarded
//! before publication are released back to free immediately: no committed
//! state can reference them. With persistent snapshots, namespace retirement
//! first enters the lifetime ledger; only an eligible, atomically published
//! transfer enters ordinary quarantine. Housekeeping uses quarantine directly.

use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::BlockDevice;
use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::{Checkpoint, RegionRecord, SnapshotRoots};
use afsplus_format::geometry::{Geometry, BITMAP_SLOTS, DESCRIPTOR_SLOTS};
use afsplus_format::region::RegionDescriptor;

use crate::allocation_root;
use crate::reclaim::{ReclaimStats, ReclaimTx};
use crate::snapshot::edit::{mutate_lifetimes, LifetimeChanges, LifetimeMutation};
use crate::snapshot::LifetimeRun;
use crate::CoreError;

/// Compact interval set used to track this transaction's own allocations
/// and retirements without one map entry per block.
#[derive(Debug, Default)]
struct RunSet {
    /// start -> end (exclusive), non-overlapping runs.
    runs: std::collections::BTreeMap<u64, u64>,
}

impl RunSet {
    fn overlaps(&self, start: u64, end: u64) -> bool {
        if let Some((_, prev_end)) = self.runs.range(..=start).next_back() {
            if *prev_end > start {
                return true;
            }
        }
        self.runs.range(start..end).next().is_some()
    }

    /// Inserts a run; fails when it overlaps an existing one.
    fn insert(&mut self, start: u64, end: u64) -> bool {
        debug_assert!(start < end);
        if self.overlaps(start, end) {
            return false;
        }
        self.runs.insert(start, end);
        true
    }

    /// Removes one block, splitting its run if needed.
    fn remove_block(&mut self, lba: u64) -> bool {
        let Some((start, end)) = self.runs.range(..=lba).next_back().map(|(s, e)| (*s, *e)) else {
            return false;
        };
        if lba >= end {
            return false;
        }
        self.runs.remove(&start);
        if start < lba {
            self.runs.insert(start, lba);
        }
        if lba + 1 < end {
            self.runs.insert(lba + 1, end);
        }
        true
    }
}

fn checkpoint_records<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    checkpoint: &Checkpoint,
) -> Result<Vec<RegionRecord>, CoreError> {
    Ok(allocation_root::load_all(
        dev,
        geo,
        checkpoint.allocation_root_block,
        checkpoint.generation,
    )?
    .records)
}

fn checkpoint_record<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    checkpoint: &Checkpoint,
    region: u32,
) -> Result<RegionRecord, CoreError> {
    allocation_root::lookup_record(
        dev,
        geo,
        checkpoint.allocation_root_block,
        checkpoint.generation,
        region,
    )?
    .ok_or_else(|| CoreError::Corrupt(format!("allocation root missing region {region}")))
}

#[derive(Debug, Clone)]
pub struct Bitmaps {
    pub geo: Geometry,
    /// Logical bitmap pages, grouped by region.
    pub pages: Vec<Vec<BitmapPage>>,
}

impl Bitmaps {
    /// Exhaustively loads every descriptor and bitmap page. Normal mount does
    /// not use this path; it is the checker's authoritative allocation view.
    pub fn load<D: BlockDevice>(
        dev: &mut D,
        geo: &Geometry,
        checkpoint: &Checkpoint,
    ) -> Result<Bitmaps, CoreError> {
        let mut buf = vec![0u8; geo.block_size];
        let records = checkpoint_records(dev, geo, checkpoint)?;
        let mut regions = Vec::with_capacity(records.len());
        for (region_index, record) in records.iter().enumerate() {
            let region = region_index as u32;
            let descriptor = load_region_descriptor(dev, geo, region, record, &mut buf)?;
            let mut pages = Vec::with_capacity(descriptor.pages.len());
            for page_index in 0..descriptor.pages.len() as u32 {
                pages.push(load_bitmap_page(
                    dev,
                    geo,
                    region,
                    page_index,
                    &descriptor,
                    &mut buf,
                )?);
            }
            regions.push(pages);
        }
        Ok(Bitmaps {
            geo: *geo,
            pages: regions,
        })
    }

    pub fn is_allocated(&self, lba: u64) -> bool {
        let region = self.geo.region_of(lba);
        let region_index = (lba - self.geo.region_base(region)) as u32;
        let (page_index, local_index) = self.geo.bitmap_page_for_index(region_index);
        self.pages[region as usize][page_index as usize].is_allocated(local_index)
    }

    pub fn free_blocks_total(&self) -> u64 {
        self.pages
            .iter()
            .flatten()
            .map(|page| page.free_blocks() as u64)
            .sum()
    }

    pub fn ram_bytes(&self) -> usize {
        self.pages
            .iter()
            .flatten()
            .map(|page| page.bits.len())
            .sum()
    }
}

fn load_region_descriptor<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    region: u32,
    record: &RegionRecord,
    buf: &mut [u8],
) -> Result<RegionDescriptor, CoreError> {
    let lba = geo.descriptor_slot_lba(region, record.descriptor_slot);
    dev.read_block(lba, buf)?;
    let (descriptor, generation) = RegionDescriptor::decode(buf)
        .map_err(|e| CoreError::Corrupt(format!("region {region} descriptor: {e}")))?;
    if generation != record.descriptor_generation {
        return Err(CoreError::Corrupt(format!(
            "region {region} descriptor generation {generation} does not match checkpoint record {}",
            record.descriptor_generation
        )));
    }
    descriptor
        .validate(geo, region, generation)
        .map_err(|e| CoreError::Corrupt(format!("region {region} descriptor: {e}")))?;
    if descriptor.free_blocks != record.free_blocks {
        return Err(CoreError::Corrupt(format!(
            "region {region} free count mismatch: descriptor {}, checkpoint {}",
            descriptor.free_blocks, record.free_blocks
        )));
    }
    Ok(descriptor)
}

fn load_bitmap_page<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    region: u32,
    page_index: u32,
    descriptor: &RegionDescriptor,
    buf: &mut [u8],
) -> Result<BitmapPage, CoreError> {
    let binding = descriptor.pages.get(page_index as usize).ok_or_else(|| {
        CoreError::Corrupt(format!("region {region} missing bitmap page {page_index}"))
    })?;
    dev.read_block(geo.bitmap_slot_lba(region, page_index, binding.slot), buf)?;
    let (page, generation) = BitmapPage::decode(buf).map_err(|e| {
        CoreError::Corrupt(format!("region {region} bitmap page {page_index}: {e}"))
    })?;
    if page.region != region
        || page.page_index != page_index
        || page.valid_blocks != geo.bitmap_page_valid_blocks(region, page_index)
    {
        return Err(CoreError::Corrupt(format!(
            "region {region} bitmap page {page_index} geometry mismatch"
        )));
    }
    if generation != binding.generation {
        return Err(CoreError::Corrupt(format!(
            "region {region} bitmap page {page_index} generation {generation} does not match descriptor binding {}",
            binding.generation
        )));
    }
    if page.free_blocks() != binding.free_blocks {
        return Err(CoreError::Corrupt(format!(
            "region {region} bitmap page {page_index} free count mismatch: bitmap {}, descriptor {}",
            page.free_blocks(), binding.free_blocks
        )));
    }
    Ok(page)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AllocStats {
    /// Extent searches, including requests rejected before scanning.
    pub allocation_searches: u64,
    /// Bitmap positions examined while searching for free runs.
    pub bitmap_bits_examined: u64,
    pub blocks_allocated: u64,
    pub blocks_retired: u64,
    pub blocks_promoted: u64,
    /// Blocks allocated by this transaction and released before publication
    /// (never quarantined: no committed state can reference them).
    pub blocks_released: u64,
    pub reclaim_latency_generations: u64,
    pub bitmap_pages_dirty: u64,
    pub region_descriptors_dirty: u64,
    /// Current/older allocation-root region records fetched on demand.
    pub allocation_records_loaded: u64,
    /// Peak resident bitmap payload bytes (descriptor objects excluded).
    pub allocator_ram_bytes: u64,
    pub reclaim: ReclaimStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotPhase {
    Namespace,
    Housekeeping,
    Preparing,
    Sealed,
    Finalizing,
    Failed,
}

struct SnapshotAccounting {
    phase: SnapshotPhase,
    registry_root: u64,
    allocations: RunSet,
    retirements: RunSet,
    mutation: Option<LifetimeMutation>,
}

pub struct TxAllocator {
    geo: Geometry,
    current_checkpoint: Checkpoint,
    other_checkpoint: Option<Checkpoint>,
    current_records: BTreeMap<u32, RegionRecord>,
    other_records: BTreeMap<u32, RegionRecord>,
    current_free_blocks_total: u64,
    descriptors: BTreeMap<u32, RegionDescriptor>,
    other_descriptors: BTreeMap<u32, RegionDescriptor>,
    /// Transaction working set. Clean scan pages are evicted immediately;
    /// dirty pages remain through commit.
    pages: BTreeMap<(u32, u32), BitmapPage>,
    new_generation: u64,
    dirty_pages: BTreeSet<(u32, u32)>,
    dirty_regions: BTreeSet<u32>,
    reclaim: Option<ReclaimTx>,
    allocated_this_tx: RunSet,
    retired_this_tx: RunSet,
    // Feature-absent transactions carry only the optional pointer.
    snapshot: Option<Box<SnapshotAccounting>>,
    /// Region where allocation last succeeded; searches start here.
    rover_region: u32,
    /// Raw free blocks that must remain after every allocation made by this
    /// transaction. Normal growth transactions use the volume's emergency
    /// metadata headroom; destructive/recovery transactions leave this at
    /// zero so they can make forward progress at ENOSPC.
    free_block_floor: u64,
    stats: AllocStats,
}

impl TxAllocator {
    /// Starts a transaction: reads the committed reclaim queue and consumes
    /// up to `batch_blocks` from its head, clearing the promoted runs' bits.
    pub fn begin<D: BlockDevice>(
        dev: &mut D,
        geo: &Geometry,
        current: &Checkpoint,
        other: Option<&Checkpoint>,
        new_generation: u64,
        batch_blocks: u64,
        rover_region: u32,
    ) -> Result<TxAllocator, CoreError> {
        let reclaim = ReclaimTx::begin(
            dev,
            geo,
            current.reclaim_root_block,
            current.generation,
            new_generation,
            other.map_or(current.generation, |older| {
                older.generation.min(current.generation)
            }),
            batch_blocks,
        )?;
        let mut tx = TxAllocator {
            geo: *geo,
            current_checkpoint: current.clone(),
            other_checkpoint: other.cloned(),
            current_records: BTreeMap::new(),
            other_records: BTreeMap::new(),
            current_free_blocks_total: current.free_blocks_total,
            descriptors: BTreeMap::new(),
            other_descriptors: BTreeMap::new(),
            pages: BTreeMap::new(),
            new_generation,
            dirty_pages: BTreeSet::new(),
            dirty_regions: BTreeSet::new(),
            reclaim: Some(reclaim),
            allocated_this_tx: RunSet::default(),
            retired_this_tx: RunSet::default(),
            snapshot: current.snapshot_roots.map(|roots| {
                Box::new(SnapshotAccounting {
                    phase: SnapshotPhase::Namespace,
                    registry_root: roots.registry,
                    allocations: RunSet::default(),
                    retirements: RunSet::default(),
                    mutation: None,
                })
            }),
            rover_region: rover_region % geo.region_count().max(1),
            free_block_floor: 0,
            stats: AllocStats::default(),
        };
        let promoted: Vec<_> = tx
            .reclaim
            .as_ref()
            .expect("reclaim initialized above")
            .promoted_runs()
            .to_vec();
        for run in promoted {
            for lba in run.start..run.start + run.blocks as u64 {
                let region = tx.geo.region_of(lba);
                let region_index = (lba - tx.geo.region_base(region)) as u32;
                let (page_index, local_index) = tx.geo.bitmap_page_for_index(region_index);
                let page = tx.page_mut(dev, region, page_index)?;
                if !page.set_allocated(local_index, false) {
                    return Err(CoreError::Corrupt(format!(
                        "quarantined block {lba} was not marked allocated"
                    )));
                }
                tx.mark_dirty(region, page_index);
            }
            tx.stats.blocks_promoted += run.blocks as u64;
            tx.stats.reclaim_latency_generations += (run.blocks as u64)
                .saturating_mul(new_generation.saturating_sub(run.retire_generation));
        }
        Ok(tx)
    }

    fn check_snapshot_health(&self) -> Result<(), CoreError> {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.phase == SnapshotPhase::Failed)
        {
            return Err(CoreError::Corrupt(
                "snapshot accounting failed; discard transaction".into(),
            ));
        }
        Ok(())
    }

    fn check_snapshot_writable(&self) -> Result<(), CoreError> {
        self.check_snapshot_health()?;
        if self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.phase == SnapshotPhase::Sealed)
        {
            return Err(CoreError::Corrupt(
                "allocator mutation after lifetime seal".into(),
            ));
        }
        Ok(())
    }

    fn track_snapshot_allocation(&mut self, start: u64, end: u64) -> Result<(), CoreError> {
        if let Some(snapshot) = &mut self.snapshot {
            if snapshot.phase == SnapshotPhase::Namespace
                && !snapshot.allocations.insert(start, end)
            {
                return Err(CoreError::Corrupt("snapshot allocations overlap".into()));
            }
        }
        Ok(())
    }

    fn check_snapshot_release(&self, start: u64, end: u64) -> Result<(), CoreError> {
        if self.snapshot.as_ref().is_some_and(|s| {
            matches!(s.phase, SnapshotPhase::Preparing | SnapshotPhase::Sealed)
                && s.allocations.overlaps(start, end)
        }) {
            return Err(CoreError::Corrupt(
                "namespace release after lifetime preparation".into(),
            ));
        }
        Ok(())
    }

    /// Close namespace allocation capture before shared-reference, registry,
    /// lifetime and other housekeeping tree maintenance. Existing non-snapshot
    /// transactions need no phase transition.
    pub fn begin_snapshot_housekeeping(&mut self) -> Result<(), CoreError> {
        self.check_snapshot_health()?;
        if let Some(snapshot) = &mut self.snapshot {
            if snapshot.phase != SnapshotPhase::Namespace {
                return Err(CoreError::Corrupt(
                    "snapshot housekeeping entered twice".into(),
                ));
            }
            snapshot.phase = SnapshotPhase::Housekeeping;
        }
        Ok(())
    }

    /// Install the result of a registry COW mutation before sealing lifetimes.
    pub(crate) fn set_snapshot_registry_root(&mut self, root: u64) -> Result<(), CoreError> {
        self.check_snapshot_health()?;
        let snapshot = self
            .snapshot
            .as_mut()
            .ok_or(CoreError::FeatureDisabled("persistent snapshots"))?;
        if snapshot.phase != SnapshotPhase::Housekeeping
            || !self.geo.is_allocatable(root)
            || self
                .current_checkpoint
                .snapshot_roots
                .is_some_and(|roots| roots.lifetimes == root)
        {
            return Err(CoreError::Corrupt(
                "invalid snapshot registry publication".into(),
            ));
        }
        snapshot.registry_root = root;
        Ok(())
    }

    /// Prepare the ledger and queue eligible transfers in this allocator
    /// transaction. New lifetime nodes are housekeeping, preventing recursive
    /// lifetime records. A failure poisons this transaction until discarded.
    /// `finish` returns the new root and writes with the bitmap/quarantine state.
    pub fn seal_snapshot_lifetimes<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        transfers: Vec<LifetimeRun>,
        next_scan_position: Option<u64>,
        max_records: usize,
        max_views: usize,
    ) -> Result<(), CoreError> {
        self.check_snapshot_health()?;
        let Some(snapshot) = &mut self.snapshot else {
            if !transfers.is_empty() || next_scan_position.is_some() {
                return Err(CoreError::FeatureDisabled("persistent snapshots"));
            }
            return Ok(());
        };
        if snapshot.phase != SnapshotPhase::Housekeeping {
            return Err(CoreError::Corrupt(
                "lifetime preparation requires housekeeping phase".into(),
            ));
        }
        let changes = LifetimeChanges {
            allocations: snapshot
                .allocations
                .runs
                .iter()
                .map(|(&start, &end)| (start, end - start))
                .collect(),
            retirements: snapshot
                .retirements
                .runs
                .iter()
                .map(|(&start, &end)| (start, end - start))
                .collect(),
            transfers,
            next_scan_position,
        };
        snapshot.phase = SnapshotPhase::Preparing;
        let geo = self.geo;
        let roots = self
            .current_checkpoint
            .snapshot_roots
            .expect("snapshot accounting has roots");
        let result = (|| {
            let mutation = mutate_lifetimes(
                dev,
                &geo,
                self,
                roots,
                self.current_checkpoint.generation,
                self.new_generation,
                &changes,
                max_records,
                max_views,
            )?;
            for &(start, blocks) in &mutation.quarantine {
                self.retire_run(dev, start, blocks)?;
            }
            Ok(mutation)
        })();
        let snapshot = self
            .snapshot
            .as_mut()
            .expect("snapshot accounting persists");
        match result {
            Ok(mutation) => {
                snapshot.phase = SnapshotPhase::Sealed;
                snapshot.mutation = Some(mutation);
                Ok(())
            }
            Err(error) => {
                snapshot.phase = SnapshotPhase::Failed;
                Err(error)
            }
        }
    }

    /// Prevents subsequent allocations, including metadata allocations made
    /// while sealing the transaction, from consuming the final `blocks` of
    /// raw free capacity. The floor is runtime policy, not on-disk state.
    pub fn set_free_block_floor(&mut self, blocks: u64) {
        self.free_block_floor = blocks;
    }

    /// Resource counters for the current transaction, without sealing it.
    pub fn stats(&self) -> AllocStats {
        self.stats
    }

    pub fn free_block_floor(&self) -> u64 {
        self.free_block_floor
    }

    fn free_blocks_remaining(&self) -> u64 {
        let made_available = self
            .current_free_blocks_total
            .saturating_add(self.stats.blocks_promoted);
        let live_allocations = self
            .stats
            .blocks_allocated
            .saturating_sub(self.stats.blocks_released);
        made_available.saturating_sub(live_allocations)
    }

    fn ensure_above_floor(&self, blocks: u64) -> Result<(), CoreError> {
        if self.free_blocks_remaining().saturating_sub(blocks) < self.free_block_floor {
            return Err(CoreError::NoSpace);
        }
        Ok(())
    }

    pub fn allocate<D: BlockDevice>(&mut self, dev: &mut D) -> Result<u64, CoreError> {
        self.allocate_run(dev, 1)
    }

    pub fn allocate_run<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        len: u64,
    ) -> Result<u64, CoreError> {
        self.check_snapshot_writable()?;
        self.stats.allocation_searches += 1;
        assert!(len > 0);
        if len > self.geo.region_size as u64 {
            return Err(CoreError::NoSpace);
        }
        self.ensure_above_floor(len)?;
        let geo = self.geo;
        let region_count = geo.region_count();
        for step in 0..region_count {
            let region = (self.rover_region + step) % region_count;
            // Skip regions whose committed free count cannot satisfy the
            // request without loading any bitmap page. Regions this
            // transaction already dirtied (allocations or promotions) have
            // diverged from the committed count and are never skipped.
            if !self.dirty_regions.contains(&region) {
                let record = self.current_record(dev, region)?;
                if (record.free_blocks as u64) < len {
                    continue;
                }
            }
            let base = geo.region_base(region);
            let mut run_start = 0u32;
            let mut run_len = 0u64;
            let mut found = None;
            let mut examined = 0u64;
            for page_index in 0..geo.bitmap_page_count(region) {
                let first_block = page_index * afsplus_format::bitmap::BITMAP_PAGE_BLOCKS;
                {
                    let page = self.page_mut(dev, region, page_index)?;
                    for local_index in 0..page.valid_blocks {
                        examined += 1;
                        let region_index = first_block + local_index;
                        let lba = base + region_index as u64;
                        if geo.is_allocatable(lba) && !page.is_allocated(local_index) {
                            if run_len == 0 {
                                run_start = region_index;
                            }
                            run_len += 1;
                            if run_len == len {
                                found = Some(run_start);
                                break;
                            }
                        } else {
                            run_len = 0;
                        }
                    }
                }
                if !self.dirty_pages.contains(&(region, page_index)) {
                    self.pages.remove(&(region, page_index));
                }
                if found.is_some() {
                    break;
                }
            }
            self.stats.bitmap_bits_examined += examined;
            if let Some(start) = found {
                for region_index in start..start + len as u32 {
                    let (page_index, local_index) = geo.bitmap_page_for_index(region_index);
                    self.page_mut(dev, region, page_index)?
                        .set_allocated(local_index, true);
                    self.mark_dirty(region, page_index);
                }
                let lba = base + start as u64;
                if !self.allocated_this_tx.insert(lba, lba + len) {
                    return Err(CoreError::Corrupt(format!(
                        "allocator returned overlapping run at {lba}"
                    )));
                }
                self.track_snapshot_allocation(lba, lba + len)?;
                self.rover_region = region;
                self.stats.blocks_allocated += len;
                return Ok(lba);
            }
        }
        Err(CoreError::NoSpace)
    }

    pub fn retire<D: BlockDevice>(&mut self, dev: &mut D, lba: u64) -> Result<(), CoreError> {
        self.retire_run(dev, lba, 1)
    }

    /// Claims a specific free run (intent-log replay: the record names the
    /// exact extents whose data survived the crash). Every block must be
    /// FREE in the working state; the run then behaves like any allocation
    /// of this transaction.
    pub fn allocate_exact_run<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        start: u64,
        blocks: u64,
    ) -> Result<(), CoreError> {
        self.check_snapshot_writable()?;
        if blocks == 0 {
            return Err(CoreError::Corrupt("claiming an empty run".into()));
        }
        self.ensure_above_floor(blocks)?;
        let end = start
            .checked_add(blocks)
            .ok_or_else(|| CoreError::Corrupt("claimed run end overflows".into()))?;
        for lba in start..end {
            if !self.geo.is_allocatable(lba) {
                return Err(CoreError::Corrupt(format!(
                    "claimed block {lba} is not allocatable"
                )));
            }
            let region = self.geo.region_of(lba);
            let region_index = (lba - self.geo.region_base(region)) as u32;
            let (page_index, local_index) = self.geo.bitmap_page_for_index(region_index);
            if self
                .page_mut(dev, region, page_index)?
                .is_allocated(local_index)
            {
                return Err(CoreError::Corrupt(format!(
                    "claimed block {lba} is already allocated"
                )));
            }
            self.page_mut(dev, region, page_index)?
                .set_allocated(local_index, true);
            self.mark_dirty(region, page_index);
        }
        if !self.allocated_this_tx.insert(start, end) {
            return Err(CoreError::Corrupt(format!(
                "claimed run {start}+{blocks} overlaps this transaction"
            )));
        }
        self.track_snapshot_allocation(start, end)?;
        self.stats.blocks_allocated += blocks;
        Ok(())
    }

    /// Retires a committed run while keeping its bits allocated. Namespace
    /// capture sends it to the snapshot ledger; housekeeping and feature-absent
    /// transactions send it to quarantine. Allocations from this transaction use
    /// [`TxAllocator::release_uncommitted`] instead.
    pub fn retire_run<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        start: u64,
        blocks: u64,
    ) -> Result<(), CoreError> {
        self.check_snapshot_writable()?;
        if blocks == 0 || blocks > u32::MAX as u64 {
            return Err(CoreError::Corrupt(format!(
                "retiring invalid run length {blocks}"
            )));
        }
        let end = start
            .checked_add(blocks)
            .ok_or_else(|| CoreError::Corrupt("retired run end overflows".into()))?;
        if self.allocated_this_tx.overlaps(start, end) {
            return Err(CoreError::Corrupt(format!(
                "retiring run {start}+{blocks} allocated by this transaction; release it instead"
            )));
        }
        if !self.retired_this_tx.insert(start, end) {
            return Err(CoreError::Corrupt(format!(
                "run {start}+{blocks} retired twice"
            )));
        }
        let mut touched_pages = BTreeSet::new();
        for lba in start..end {
            if !self.geo.is_allocatable(lba) {
                return Err(CoreError::Corrupt(format!(
                    "retiring block {lba} that is not live"
                )));
            }
            let region = self.geo.region_of(lba);
            let region_index = (lba - self.geo.region_base(region)) as u32;
            let (page_index, local_index) = self.geo.bitmap_page_for_index(region_index);
            touched_pages.insert((region, page_index));
            if !self
                .page_mut(dev, region, page_index)?
                .is_allocated(local_index)
            {
                return Err(CoreError::Corrupt(format!(
                    "retiring block {lba} that is not live"
                )));
            }
        }
        // Retiring only reads bits; drop pages this check loaded so a large
        // quarantine does not inflate resident allocator state.
        for key in touched_pages {
            if !self.dirty_pages.contains(&key) {
                self.pages.remove(&key);
            }
        }
        if let Some(snapshot) = self
            .snapshot
            .as_mut()
            .filter(|s| s.phase == SnapshotPhase::Namespace)
        {
            if !snapshot.retirements.insert(start, end) {
                return Err(CoreError::Corrupt("snapshot run retired twice".into()));
            }
        } else {
            self.reclaim
                .as_mut()
                .expect("reclaim lives until finish")
                .append_run(&self.geo, start, blocks as u32)?;
        }
        self.stats.blocks_retired += blocks;
        Ok(())
    }

    /// Quarantines a run this transaction allocated whose content a durable
    /// log record still covers (ADR-037 sacrifice): the bits stay set and
    /// the run enters the reclaim queue instead of returning to FREE.
    pub fn abandon_uncommitted_run<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        start: u64,
        blocks: u64,
    ) -> Result<(), CoreError> {
        self.check_snapshot_writable()?;
        let _ = dev;
        if blocks == 0 || blocks > u32::MAX as u64 {
            return Err(CoreError::Corrupt("abandoning invalid run".into()));
        }
        let end = start
            .checked_add(blocks)
            .ok_or_else(|| CoreError::Corrupt("abandoned run end overflows".into()))?;
        self.check_snapshot_release(start, end)?;
        for lba in start..end {
            if !self.allocated_this_tx.remove_block(lba) {
                return Err(CoreError::Corrupt(format!(
                    "abandoning block {lba} that this transaction did not allocate"
                )));
            }
        }
        if let Some(snapshot) = &mut self.snapshot {
            for lba in start..end {
                snapshot.allocations.remove_block(lba);
            }
        }
        if !self.retired_this_tx.insert(start, end) {
            return Err(CoreError::Corrupt(format!(
                "run {start}+{blocks} abandoned twice"
            )));
        }
        self.reclaim
            .as_mut()
            .expect("reclaim lives until finish")
            .append_run(&self.geo, start, blocks as u32)?;
        self.stats.blocks_retired += blocks;
        Ok(())
    }

    /// Releases a block this transaction allocated and no longer needs. It
    /// returns to FREE immediately — no committed state can reference it, so
    /// quarantine would only delay reuse for no protection.
    pub fn release_uncommitted<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        lba: u64,
    ) -> Result<(), CoreError> {
        self.check_snapshot_writable()?;
        let end = lba
            .checked_add(1)
            .ok_or_else(|| CoreError::Corrupt("release block overflows".into()))?;
        self.check_snapshot_release(lba, end)?;
        if !self.allocated_this_tx.remove_block(lba) {
            return Err(CoreError::Corrupt(format!(
                "releasing block {lba} that this transaction did not allocate"
            )));
        }
        let region = self.geo.region_of(lba);
        let region_index = (lba - self.geo.region_base(region)) as u32;
        let (page_index, local_index) = self.geo.bitmap_page_for_index(region_index);
        if !self
            .page_mut(dev, region, page_index)?
            .set_allocated(local_index, false)
        {
            return Err(CoreError::Corrupt(format!(
                "released block {lba} was not marked allocated"
            )));
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.allocations.remove_block(lba);
        }
        self.mark_dirty(region, page_index);
        self.stats.blocks_released += 1;
        Ok(())
    }

    fn mark_dirty(&mut self, region: u32, page_index: u32) {
        self.dirty_pages.insert((region, page_index));
        self.dirty_regions.insert(region);
    }

    /// The committed allocation-root record for a region, loaded on demand
    /// and cached for the transaction.
    fn current_record<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        region: u32,
    ) -> Result<RegionRecord, CoreError> {
        if let Some(record) = self.current_records.get(&region) {
            return Ok(*record);
        }
        let record = checkpoint_record(dev, &self.geo, &self.current_checkpoint, region)?;
        self.current_records.insert(region, record);
        self.stats.allocation_records_loaded += 1;
        Ok(record)
    }

    fn ensure_descriptors<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        region: u32,
    ) -> Result<(), CoreError> {
        if !self.descriptors.contains_key(&region) {
            let record = self.current_record(dev, region)?;
            let mut buf = vec![0u8; self.geo.block_size];
            let descriptor = load_region_descriptor(dev, &self.geo, region, &record, &mut buf)?;
            self.descriptors.insert(region, descriptor);

            if let Some(other_checkpoint) = &self.other_checkpoint {
                let other_record = checkpoint_record(dev, &self.geo, other_checkpoint, region)?;
                self.other_records.insert(region, other_record);
                self.stats.allocation_records_loaded += 1;
                let other =
                    load_region_descriptor(dev, &self.geo, region, &other_record, &mut buf)?;
                self.other_descriptors.insert(region, other);
            }
        }
        Ok(())
    }

    fn page_mut<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        region: u32,
        page_index: u32,
    ) -> Result<&mut BitmapPage, CoreError> {
        let key = (region, page_index);
        if !self.pages.contains_key(&key) {
            self.ensure_descriptors(dev, region)?;
            let descriptor = self.descriptors.get(&region).expect("loaded above");
            let mut buf = vec![0u8; self.geo.block_size];
            let page = load_bitmap_page(dev, &self.geo, region, page_index, descriptor, &mut buf)?;
            self.pages.insert(key, page);
            let resident = self.pages.values().map(|page| page.bits.len() as u64).sum();
            self.stats.allocator_ram_bytes = self.stats.allocator_ram_bytes.max(resident);
        }
        Ok(self.pages.get_mut(&key).expect("inserted above"))
    }

    pub fn finish<D: BlockDevice>(mut self, dev: &mut D) -> Result<FinishedAlloc, CoreError> {
        self.check_snapshot_health()?;
        if self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.phase != SnapshotPhase::Sealed)
        {
            return Err(CoreError::Corrupt(
                "snapshot lifetime accounting must be sealed before allocator finish".into(),
            ));
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.phase = SnapshotPhase::Finalizing;
        }
        // Rebuild the reclaim queue first: sealing and the new root allocate
        // ordinary blocks, which must land in the dirty bitmap state emitted
        // below. The allocation count is known before allocating (ADR-036),
        // so recording an allocation never allocates in turn.
        let mut reclaim = self.reclaim.take().expect("reclaim lives until finish");
        let needed = reclaim.plan()?;
        let mut structure_lbas = Vec::with_capacity(needed);
        for _ in 0..needed {
            structure_lbas.push(self.allocate(dev)?);
        }
        let geo = self.geo;
        let build = reclaim.build(&geo, structure_lbas)?;
        self.stats.reclaim = build.stats;

        let mut bitmap_writes = Vec::new();
        let mut descriptor_writes = Vec::new();
        let mut dirty_records = Vec::with_capacity(self.dirty_regions.len());
        let mut free_blocks_total = self.current_free_blocks_total;
        for region in self.dirty_regions.iter().copied() {
            let current_record = self.current_records.get(&region).ok_or_else(|| {
                CoreError::Corrupt(format!("dirty region {region} has no root record"))
            })?;
            let mut descriptor = self.descriptors.get(&region).cloned().ok_or_else(|| {
                CoreError::Corrupt(format!("dirty region {region} has no descriptor"))
            })?;
            for page_index in 0..descriptor.pages.len() as u32 {
                if !self.dirty_pages.contains(&(region, page_index)) {
                    continue;
                }
                let current_binding = descriptor.pages[page_index as usize];
                let older_slot = self
                    .other_descriptors
                    .get(&region)
                    .and_then(|older| older.pages.get(page_index as usize))
                    .map(|binding| binding.slot);
                let slot = choose_slot(BITMAP_SLOTS, current_binding.slot, older_slot);
                let page = self.pages.get(&(region, page_index)).ok_or_else(|| {
                    CoreError::Corrupt(format!(
                        "dirty region {region} bitmap page {page_index} is not resident"
                    ))
                })?;
                bitmap_writes.push((
                    self.geo.bitmap_slot_lba(region, page_index, slot),
                    page.encode(self.geo.block_size, self.new_generation)
                        .map_err(CoreError::Format)?,
                ));
                descriptor.pages[page_index as usize] = afsplus_format::region::BitmapBinding {
                    slot,
                    free_blocks: page.free_blocks(),
                    generation: self.new_generation,
                };
            }
            descriptor.free_blocks = descriptor
                .pages
                .iter()
                .map(|binding| binding.free_blocks)
                .sum();
            let older_descriptor_slot = self
                .other_records
                .get(&region)
                .map(|record| record.descriptor_slot);
            let descriptor_slot = choose_slot(
                DESCRIPTOR_SLOTS,
                current_record.descriptor_slot,
                older_descriptor_slot,
            );
            descriptor_writes.push((
                self.geo.descriptor_slot_lba(region, descriptor_slot),
                descriptor
                    .encode(self.geo.block_size, self.new_generation)
                    .map_err(CoreError::Format)?,
            ));
            let new_record = RegionRecord {
                descriptor_slot,
                free_blocks: descriptor.free_blocks,
                descriptor_generation: self.new_generation,
            };
            if new_record.free_blocks >= current_record.free_blocks {
                free_blocks_total = free_blocks_total
                    .checked_add((new_record.free_blocks - current_record.free_blocks) as u64)
                    .ok_or_else(|| CoreError::Corrupt("free block total overflows".into()))?;
            } else {
                free_blocks_total = free_blocks_total
                    .checked_sub((current_record.free_blocks - new_record.free_blocks) as u64)
                    .ok_or_else(|| CoreError::Corrupt("free block total underflows".into()))?;
            }
            dirty_records.push((region, new_record));
        }
        self.stats.bitmap_pages_dirty = bitmap_writes.len() as u64;
        self.stats.region_descriptors_dirty = descriptor_writes.len() as u64;
        let resident = self.pages.values().map(|page| page.bits.len() as u64).sum();
        self.stats.allocator_ram_bytes = self.stats.allocator_ram_bytes.max(resident);
        let registry_root = self.snapshot.as_ref().map(|s| s.registry_root);
        let snapshot_lifetimes = self.snapshot.and_then(|s| s.mutation);
        let snapshot_roots = self
            .current_checkpoint
            .snapshot_roots
            .map(|_| SnapshotRoots {
                registry: registry_root.expect("snapshot registry root"),
                lifetimes: snapshot_lifetimes
                    .as_ref()
                    .expect("sealed lifetime mutation")
                    .tree
                    .root_lba,
            });
        Ok(FinishedAlloc {
            snapshot_roots,
            snapshot_lifetimes,
            bitmap_writes,
            descriptor_writes,
            dirty_records,
            free_blocks_total,
            reclaim_root_lba: build.root_lba,
            reclaim_writes: build.writes,
            reclaim_pending_blocks: build.pending_blocks,
            rover_region: self.rover_region,
            stats: self.stats,
        })
    }
}

pub struct FinishedAlloc {
    /// Publish these roots with the lifetime writes in this result, never
    /// the old checkpoint's roots. None for feature-absent transactions.
    pub snapshot_roots: Option<SnapshotRoots>,
    pub snapshot_lifetimes: Option<LifetimeMutation>,
    pub bitmap_writes: Vec<(u64, Vec<u8>)>,
    pub descriptor_writes: Vec<(u64, Vec<u8>)>,
    /// Only records whose descriptor/bitmap state changed in this
    /// transaction, for dirty-path allocation-root COW.
    pub dirty_records: Vec<(u32, RegionRecord)>,
    pub free_blocks_total: u64,
    /// New reclaim-queue root and the sealed blocks written with it; part of
    /// the metadata barrier group.
    pub reclaim_root_lba: u64,
    pub reclaim_writes: Vec<(u64, Vec<u8>)>,
    pub reclaim_pending_blocks: u64,
    /// Region of the last successful allocation, for the next transaction's
    /// search start.
    pub rover_region: u32,
    pub stats: AllocStats,
}

fn choose_slot(slot_count: u8, current: u8, other: Option<u8>) -> u8 {
    (0..slot_count)
        .find(|slot| *slot != current && Some(*slot) != other)
        .expect("three slots minus at most two references")
}

#[cfg(test)]
mod tests {
    use super::choose_slot;
    use super::TxAllocator;
    use afsplus_block::MemoryBackend;
    use afsplus_format::bitmap::BITMAP_PAGE_BLOCKS;
    use afsplus_format::Timespec;

    use crate::{mkfs, mount, CoreError, MkfsParams};

    #[test]
    fn slot_choice_avoids_both_retained_references() {
        assert_eq!(choose_slot(3, 0, Some(1)), 2);
        assert_eq!(choose_slot(3, 1, Some(0)), 2);
        assert_eq!(choose_slot(3, 2, Some(0)), 1);
        assert_eq!(choose_slot(3, 2, Some(1)), 0);
        assert_eq!(choose_slot(3, 0, None), 1);
        assert_eq!(choose_slot(3, 0, Some(0)), 1);
    }

    #[test]
    fn one_run_can_cross_bitmap_pages_with_page_level_writes() {
        let mut dev = MemoryBackend::new(4096, 262_144);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [17u8; 16],
                label: "MultiPage".into(),
                region_size: 262_144,
                reclaim_caps: Default::default(),
                log_slots: 8,
                shared_extents: true,
                data_policy: false,
                name_policy: crate::NamePolicy::Sensitive,
                timestamp: Timespec::default(),
            },
        )
        .unwrap();
        let vol = mount(dev).unwrap();
        let geo = vol.ident().geometry();
        let checkpoint = vol.checkpoint().clone();
        let mut dev = vol.into_device();

        let mut tx = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 4096, 0).unwrap();
        let start = tx
            .allocate_run(&mut dev, BITMAP_PAGE_BLOCKS as u64 + 1)
            .unwrap();
        // Four bootstrap metadata blocks (root record, root directory,
        // object map, reclaim root), the three-image allocation-root pool,
        // and the eight-slot intent-log area precede ordinary free space.
        assert_eq!(start, geo.region0_reserved_blocks() + 15);
        let finished = tx.finish(&mut dev).unwrap();
        assert_eq!(finished.bitmap_writes.len(), 2);
        assert_eq!(finished.descriptor_writes.len(), 1);
        assert_eq!(finished.stats.bitmap_pages_dirty, 2);
        assert_eq!(finished.stats.region_descriptors_dirty, 1);
        assert!(finished.stats.allocator_ram_bytes <= 2 * 4096);
    }

    #[test]
    fn soft_floor_covers_data_and_transaction_sealing_allocations() {
        let mut dev = MemoryBackend::new(4096, 64);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [18u8; 16],
                label: "Headroom".into(),
                region_size: 64,
                reclaim_caps: Default::default(),
                log_slots: 0,
                shared_extents: false,
                data_policy: false,
                name_policy: crate::NamePolicy::Sensitive,
                timestamp: Timespec::default(),
            },
        )
        .unwrap();
        let vol = mount(dev).unwrap();
        let geo = vol.ident().geometry();
        let checkpoint = vol.checkpoint().clone();
        let free = checkpoint.free_blocks_total;
        let mut dev = vol.into_device();

        let mut too_large = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 0, 0).unwrap();
        too_large.set_free_block_floor(8);
        assert!(matches!(
            too_large.allocate_run(&mut dev, free - 7),
            Err(CoreError::NoSpace)
        ));

        let mut sealing = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 0, 0).unwrap();
        sealing.set_free_block_floor(8);
        sealing.allocate_run(&mut dev, free - 8).unwrap();
        assert!(matches!(sealing.finish(&mut dev), Err(CoreError::NoSpace)));

        let mut succeeds = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 0, 0).unwrap();
        succeeds.set_free_block_floor(8);
        succeeds.allocate_run(&mut dev, free - 9).unwrap();
        let finished = succeeds.finish(&mut dev).unwrap();
        assert_eq!(finished.free_blocks_total, 8);
    }
}
