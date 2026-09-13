//! Transaction preparation for ADR-071 lifetime ownership. The allocator
//! passed to `mutate_lifetimes` must classify these tree nodes as housekeeping.
use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::SnapshotRoots;
use afsplus_format::geometry::Geometry;
use afsplus_format::snapshot::{LedgerState, LifetimeRecord};
use afsplus_format::tree::key_u64;

use super::{
    add_stats, adjacent, lifetime, lifetime_spec, namespace_range, read_ledger_state,
    read_registry_page, LifetimeRun,
};
use crate::cow_tree::{mutate_many, TreeAllocator, TreeMutation, TreeOperation};
use crate::tree::{self, TreeLookupStats};
use crate::CoreError;

const READ_PAGE: usize = 64;

#[derive(Debug, Default)]
pub struct LifetimeChanges {
    /// Newly committed namespace allocations. Reflinks never enter this list.
    pub allocations: Vec<(u64, u64)>,
    /// Last-live-reference removals, with allocation birth already committed.
    pub retirements: Vec<(u64, u64)>,
    /// Exact committed retired records requested for ordinary quarantine.
    pub transfers: Vec<LifetimeRun>,
    pub next_scan_position: Option<u64>,
}

#[derive(Debug)]
pub struct LifetimeMutation {
    pub tree: TreeMutation,
    pub state: LedgerState,
    /// Queue these at the publication generation, atomically with the new
    /// ledger root. Their allocation bits must remain set until quarantine ends.
    pub quarantine: Vec<(u64, u64)>,
    pub reads: TreeLookupStats,
    pub loaded_records: usize,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Corrupt(message.into())
}
fn budget() -> CoreError {
    CoreError::PrototypeLimit("lifetime edit record budget exhausted")
}

fn view_budget() -> CoreError {
    CoreError::PrototypeLimit("snapshot registry scan budget exhausted")
}

struct Overlay {
    original: BTreeMap<u64, LifetimeRecord>,
    current: BTreeMap<u64, LifetimeRecord>,
    max_records: usize,
}

impl Overlay {
    fn remember(&mut self, run: LifetimeRun) -> Result<(), CoreError> {
        if let Some(previous) = self.original.get(&run.start) {
            if *previous != run.record {
                return Err(invalid("lifetime changed during preparation"));
            }
        } else {
            if self.original.len() >= self.max_records {
                return Err(budget());
            }
            self.original.insert(run.start, run.record);
        }
        Ok(())
    }

    fn allocate(&mut self, start: u64, blocks: u64, generation: u64) -> Result<(), CoreError> {
        let end = start + blocks; // all requested ranges validated before edits
        if self
            .current
            .range(..end)
            .next_back()
            .is_some_and(|(&low, run)| low + run.blocks > start)
        {
            return Err(invalid("new allocation overlaps a tracked lifetime"));
        }
        if self.current.len() >= self.max_records {
            return Err(budget());
        }
        self.current.insert(
            start,
            LifetimeRecord {
                blocks,
                birth: generation,
                retirement: 0,
            },
        );
        Ok(())
    }

    fn retire(&mut self, start: u64, blocks: u64, generation: u64) -> Result<(), CoreError> {
        let end = start + blocks;
        let mut position = start;
        while position < end {
            let (&low, &record) = self
                .current
                .range(..=position)
                .next_back()
                .ok_or_else(|| invalid("retirement has no allocation lifetime"))?;
            let old_end = low + record.blocks;
            if old_end <= position || record.retirement != 0 || record.birth >= generation {
                return Err(invalid(
                    "retirement crosses a gap, retired run or uncommitted birth",
                ));
            }
            let high = end.min(old_end);
            // At most two additional records are needed for this split.
            let needed = usize::from(low < position) + usize::from(high < old_end);
            if self
                .current
                .len()
                .checked_add(needed)
                .is_none_or(|n| n > self.max_records)
            {
                return Err(budget());
            }
            self.current.remove(&low);
            if low < position {
                self.current.insert(
                    low,
                    LifetimeRecord {
                        blocks: position - low,
                        ..record
                    },
                );
            }
            self.current.insert(
                position,
                LifetimeRecord {
                    blocks: high - position,
                    retirement: generation,
                    ..record
                },
            );
            if high < old_end {
                self.current.insert(
                    high,
                    LifetimeRecord {
                        blocks: old_end - high,
                        ..record
                    },
                );
            }
            position = high;
        }
        Ok(())
    }

    fn canonicalize(&mut self) -> Result<(), CoreError> {
        let mut merged: BTreeMap<u64, LifetimeRecord> = BTreeMap::new();
        for (&start, &record) in &self.current {
            if let Some((&low, previous)) = merged.last_key_value() {
                let end = low + previous.blocks;
                if end > start {
                    return Err(invalid("edited lifetimes overlap"));
                }
                if end == start
                    && previous.birth == record.birth
                    && previous.retirement == record.retirement
                {
                    merged.get_mut(&low).expect("existing predecessor").blocks += record.blocks;
                    continue;
                }
            }
            merged.insert(start, record);
        }
        self.current = merged;
        Ok(())
    }
}

fn retired_sum(records: &BTreeMap<u64, LifetimeRecord>) -> Result<u64, CoreError> {
    records
        .values()
        .filter(|run| run.retirement != 0)
        .try_fold(0u64, |sum, run| {
            sum.checked_add(run.blocks)
                .ok_or_else(|| invalid("retained block sum overflow"))
        })
}

/// Prepares one COW ledger mutation from committed roots. Every requested
/// range is fetched with its immediate neighbors before editing; memory is
/// proportional to `max_records`, input changes and a fixed 64-entry read page.
/// Registry validation streams all views when transfers are requested, with
/// `max_views` limiting work rather than silently ignoring excess views.
///
/// This does not publish a checkpoint, change allocation bits, or grant mount
/// support. The caller must atomically queue `quarantine`, publish the returned
/// writes/root and keep registry creation/deletion serialized. On any error
/// discard the surrounding allocator transaction, as for the common COW engine.
#[allow(clippy::too_many_arguments)]
pub fn mutate_lifetimes<D: BlockDevice, A: TreeAllocator<D>>(
    dev: &mut D,
    geo: &Geometry,
    allocator: &mut A,
    roots: SnapshotRoots,
    generation: u64,
    new_generation: u64,
    changes: &LifetimeChanges,
    max_records: usize,
    max_views: usize,
) -> Result<LifetimeMutation, CoreError> {
    if new_generation <= generation {
        return Err(invalid("lifetime edit generation does not advance"));
    }
    let (old_state, _, mut reads) = read_ledger_state(dev, geo, roots.lifetimes, generation)?;
    let mut state = old_state;
    if let Some(position) = changes.next_scan_position {
        state.scan_position = position;
    }
    state.encode(geo.total_blocks)?;
    let mut ranges = Vec::new();
    for &(start, blocks) in changes.allocations.iter().chain(&changes.retirements) {
        namespace_range(geo, start, blocks)?;
        ranges.push((start, start + blocks));
    }
    for run in &changes.transfers {
        run.record
            .validate(run.start, generation, geo.total_blocks)?;
        namespace_range(geo, run.start, run.record.blocks)?;
        if run.record.retirement == 0 {
            return Err(invalid("cannot transfer a live lifetime"));
        }
        ranges.push((run.start, run.start + run.record.blocks));
    }
    ranges.sort_unstable();
    let mut intervals: Vec<(u64, u64)> = Vec::new();
    for (start, end) in ranges {
        match intervals.last_mut() {
            Some((_, high)) if start <= *high => *high = (*high).max(end),
            _ => intervals.push((start, end)),
        }
    }
    let mut overlay = Overlay {
        original: BTreeMap::new(),
        current: BTreeMap::new(),
        max_records,
    };
    for (start, end) in intervals {
        let (before, stats) = tree::lookup_floor(
            dev,
            geo,
            roots.lifetimes,
            lifetime_spec(generation),
            &key_u64(start - 1),
        )?;
        add_stats(&mut reads, stats);
        if let Some((key, value)) = before {
            if key != key_u64(0) {
                overlay.remember(lifetime(&key, &value, geo, generation)?)?;
            }
        }
        let mut cursor = start;
        'pages: loop {
            let (page, stats) = tree::read_key_page(
                dev,
                geo,
                roots.lifetimes,
                lifetime_spec(generation),
                &key_u64(cursor),
                READ_PAGE,
            )?;
            add_stats(&mut reads, stats);
            if page.items.is_empty() {
                break;
            }
            for (key, value) in page.items {
                let run = lifetime(&key, &value, geo, generation)?;
                overlay.remember(run)?;
                if run.start >= end {
                    break 'pages;
                }
                cursor = run.start + 1;
            }
        }
    }
    let mut previous = None;
    for (&start, &record) in &overlay.original {
        let run = LifetimeRun { start, record };
        if let Some(left) = previous {
            adjacent(left, run)?;
        }
        previous = Some(run);
    }
    // Streaming every registered view avoids a caller-supplied incomplete
    // generation set accidentally granting an unsafe release.
    if !changes.transfers.is_empty() {
        let mut next = Some(0);
        let mut seen = 0usize;
        while let Some(low) = next {
            let page = read_registry_page(dev, geo, roots.registry, generation, low, READ_PAGE)?;
            add_stats(&mut reads, page.stats);
            if page.advertised_count > max_views as u64 {
                return Err(view_budget());
            }
            seen = seen
                .checked_add(page.records.len())
                .ok_or_else(view_budget)?;
            if seen > max_views {
                return Err(view_budget());
            }
            if page.next_id.is_none() && seen as u64 != page.advertised_count {
                return Err(invalid("snapshot registry traversal count mismatch"));
            }
            for (_, view) in page.records {
                if changes
                    .transfers
                    .iter()
                    .any(|run| run.record.contains(view.generation))
                {
                    return Err(invalid("registered snapshot still owns requested transfer"));
                }
            }
            next = page.next_id;
        }
    }
    overlay.current = overlay.original.clone();
    for &(start, blocks) in &changes.allocations {
        overlay.allocate(start, blocks, new_generation)?;
    }
    for &(start, blocks) in &changes.retirements {
        overlay.retire(start, blocks, new_generation)?;
    }
    let mut quarantine = Vec::new();
    for run in &changes.transfers {
        if overlay.original.get(&run.start) != Some(&run.record)
            || overlay.current.remove(&run.start) != Some(run.record)
        {
            return Err(invalid(
                "transfer differs from committed lifetime or occurs twice",
            ));
        }
        quarantine.push((run.start, run.record.blocks));
    }
    overlay.canonicalize()?;
    let old_retired = retired_sum(&overlay.original)?;
    let new_retired = retired_sum(&overlay.current)?;
    state.retained_blocks = old_state
        .retained_blocks
        .checked_sub(old_retired)
        .and_then(|remaining| remaining.checked_add(new_retired))
        .ok_or_else(|| invalid("retained block accounting mismatch"))?;
    let control_value = state.encode(geo.total_blocks)?;
    let deletes: Vec<_> = overlay
        .original
        .keys()
        .filter(|key| !overlay.current.contains_key(key))
        .map(|key| key_u64(*key))
        .collect();
    let mut upserts = Vec::new();
    for (&start, &record) in &overlay.current {
        if overlay.original.get(&start) != Some(&record) {
            upserts.push((
                key_u64(start),
                record.encode(start, new_generation, geo.total_blocks)?,
            ));
        }
    }
    if state != old_state {
        upserts.push((key_u64(0), control_value));
    }
    let operations: Vec<_> = deletes
        .iter()
        .map(|key| TreeOperation::Delete { key })
        .chain(
            upserts
                .iter()
                .map(|(key, value)| TreeOperation::Upsert { key, value }),
        )
        .collect();
    let tree = mutate_many(
        dev,
        geo,
        allocator,
        roots.lifetimes,
        lifetime_spec(generation),
        new_generation,
        &operations,
    )?;
    Ok(LifetimeMutation {
        tree,
        state,
        quarantine,
        reads,
        loaded_records: overlay.original.len(),
    })
}
