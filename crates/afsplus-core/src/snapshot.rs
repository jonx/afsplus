//! Typed, bounded readers for the experimental snapshot trees (ADR-071–073).
//! These readers do not enable mounts or establish global bitmap ownership.
pub mod edit;
pub(crate) mod view;

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::snapshot::{
    decode_key, LedgerState, LifetimeRecord, RegistryState, SnapshotRecord,
};
use afsplus_format::tree::{key_u64, TreeKind};

use crate::allocation_root::reserved_pool_bounds;
use crate::tree::{self, TreeLookupStats, TreeSpec};
use crate::CoreError;

pub fn registry_spec(max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::SnapshotRegistry,
        owner: 0,
        max_generation,
    }
}

pub fn lifetime_spec(max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::SnapshotLifetimes,
        owner: 0,
        max_generation,
    }
}

fn add_stats(total: &mut TreeLookupStats, next: TreeLookupStats) {
    total.pages_read += next.pages_read;
    total.peak_page_buffers = total.peak_page_buffers.max(next.peak_page_buffers);
}

fn control<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root: u64,
    spec: TreeSpec,
) -> Result<(Vec<u8>, u64, TreeLookupStats), CoreError> {
    geo.validate()?;
    let (page, stats) = tree::read_key_page(dev, geo, root, spec, &key_u64(0), 1)?;
    let Some((key, value)) = page.items.into_iter().next() else {
        return Err(CoreError::Corrupt(
            "snapshot tree lacks control record".into(),
        ));
    };
    if decode_key(&key)? != 0 || page.total_items == 0 {
        return Err(CoreError::Corrupt(
            "snapshot tree lacks key-zero control".into(),
        ));
    }
    Ok((value, page.total_items - 1, stats))
}

/// The count is the checked root's advertised count; an exhaustive checker
/// must still validate unvisited subtrees and record ownership.
pub fn read_registry_state<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root: u64,
    generation: u64,
) -> Result<(RegistryState, u64, TreeLookupStats), CoreError> {
    let (value, count, stats) = control(dev, geo, root, registry_spec(generation))?;
    let state = RegistryState::decode(&value)?;
    if count >= state.next_id {
        return Err(CoreError::Corrupt(
            "snapshot count exceeds allocated IDs".into(),
        ));
    }
    Ok((state, count, stats))
}

pub fn read_ledger_state<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root: u64,
    generation: u64,
) -> Result<(LedgerState, u64, TreeLookupStats), CoreError> {
    let (value, count, stats) = control(dev, geo, root, lifetime_spec(generation))?;
    Ok((LedgerState::decode(&value, geo.total_blocks)?, count, stats))
}

/// Validates geometry and permanent allocator-pool exclusion. Cross-tree
/// aliasing, bitmap ownership and intent-log exclusion need enclosing-volume
/// validation; this helper has no identification or allocation state.
fn namespace_range(geo: &Geometry, start: u64, blocks: u64) -> Result<(), CoreError> {
    let end = start
        .checked_add(blocks)
        .ok_or_else(|| CoreError::Corrupt("snapshot range overflow".into()))?;
    if blocks == 0
        || !geo.is_allocatable(start)
        || !geo.is_allocatable(end - 1)
        || geo.region_of(start) != geo.region_of(end - 1)
    {
        return Err(CoreError::Corrupt(
            "snapshot range intersects reserved geometry".into(),
        ));
    }
    let (pool_start, pool_end) = reserved_pool_bounds(geo)?;
    if start < pool_end && pool_start < end {
        return Err(CoreError::Corrupt(
            "snapshot range intersects allocation-root pool".into(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub struct RegistryPage {
    pub state: RegistryState,
    pub advertised_count: u64,
    pub records: Vec<(u64, SnapshotRecord)>,
    /// Inclusive key for another page. None means this traversal reached EOF.
    pub next_id: Option<u64>,
    pub stats: TreeLookupStats,
}

/// Reads up to `limit` views starting at `low_id` (zero starts at ID one).
/// A full page may require a subsequent empty page to establish EOF.
pub fn read_registry_page<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root: u64,
    generation: u64,
    low_id: u64,
    limit: usize,
) -> Result<RegistryPage, CoreError> {
    if limit == 0 {
        return Err(CoreError::PrototypeLimit(
            "snapshot page limit must be positive",
        ));
    }
    let (state, count, mut stats) = read_registry_state(dev, geo, root, generation)?;
    let (page, reads) = tree::read_key_page(
        dev,
        geo,
        root,
        registry_spec(generation),
        &key_u64(low_id.max(1)),
        limit,
    )?;
    add_stats(&mut stats, reads);
    let mut records = Vec::with_capacity(page.items.len());
    for (key, value) in page.items {
        let id = decode_key(&key)?;
        if id == 0 || id >= state.next_id {
            return Err(CoreError::Corrupt(
                "snapshot ID outside registry control".into(),
            ));
        }
        let record = SnapshotRecord::decode(&value, generation, geo.total_blocks)?;
        namespace_range(geo, record.object_map_root, 1)?;
        records.push((id, record));
    }
    let next_id = if records.len() == limit {
        records.last().and_then(|(id, _)| id.checked_add(1))
    } else {
        None
    };
    Ok(RegistryPage {
        state,
        advertised_count: count,
        records,
        next_id,
        stats,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifetimeRun {
    pub start: u64,
    pub record: LifetimeRecord,
}

fn lifetime(
    key: &[u8],
    value: &[u8],
    geo: &Geometry,
    generation: u64,
) -> Result<LifetimeRun, CoreError> {
    let start = decode_key(key)?;
    let record = LifetimeRecord::decode(value, start, generation, geo.total_blocks)?;
    namespace_range(geo, start, record.blocks)?;
    Ok(LifetimeRun { start, record })
}

fn adjacent(left: LifetimeRun, right: LifetimeRun) -> Result<(), CoreError> {
    let end = left.start + left.record.blocks; // decoded ranges cannot overflow
    if end > right.start {
        return Err(CoreError::Corrupt("snapshot lifetimes overlap".into()));
    }
    if end == right.start
        && left.record.birth == right.record.birth
        && left.record.retirement == right.record.retirement
    {
        return Err(CoreError::Corrupt(
            "adjacent equal snapshot lifetimes are not merged".into(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub struct LifetimePage {
    pub state: LedgerState,
    pub advertised_count: u64,
    pub records: Vec<LifetimeRun>,
    /// Persist with edits in the same checkpoint. Zero explicitly wraps;
    /// wrapping alone does not prove no protected or reclaimable records exist.
    pub next_position: u64,
    pub stats: TreeLookupStats,
}

/// Reads from the persistent physical-key cursor, validating both page edges
/// against their neighbors. Keeps at most `limit + 2` decoded lifetimes plus
/// tree traversal buffers; never sums the whole ledger during ordinary access.
/// Eligibility and retained-count updates belong to the transaction layer.
pub fn read_lifetime_page<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root: u64,
    generation: u64,
    limit: usize,
) -> Result<LifetimePage, CoreError> {
    let fetch = limit
        .checked_add(1)
        .filter(|_| limit != 0)
        .ok_or(CoreError::PrototypeLimit(
            "snapshot page limit must be positive and leave lookahead room",
        ))?;
    let (state, count, mut stats) = read_ledger_state(dev, geo, root, generation)?;
    let low = state.scan_position.max(1);
    let (previous, reads) =
        tree::lookup_floor(dev, geo, root, lifetime_spec(generation), &key_u64(low - 1))?;
    add_stats(&mut stats, reads);
    let mut previous = match previous {
        Some((key, value)) if decode_key(&key)? != 0 => {
            Some(lifetime(&key, &value, geo, generation)?)
        }
        _ => None,
    };
    let (page, reads) = tree::read_key_page(
        dev,
        geo,
        root,
        lifetime_spec(generation),
        &key_u64(low),
        fetch,
    )?;
    add_stats(&mut stats, reads);
    let mut records = Vec::with_capacity(page.items.len().min(limit));
    let has_more = page.items.len() > limit;
    for (index, (key, value)) in page.items.into_iter().enumerate() {
        let run = lifetime(&key, &value, geo, generation)?;
        if let Some(left) = previous {
            adjacent(left, run)?;
        }
        previous = Some(run);
        if index < limit {
            records.push(run);
        }
    }
    let next_position = if has_more {
        records.last().expect("positive limit with lookahead").start + 1
    } else {
        0
    };
    Ok(LifetimePage {
        state,
        advertised_count: count,
        records,
        next_position,
        stats,
    })
}
