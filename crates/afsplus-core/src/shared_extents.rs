//! Volume-wide shared-extent reference tree (ADR-061).
//!
//! One record per shared physical run, keyed by the run's first physical
//! block. A record exists only while its run has two or more references:
//! the extent flag `EXTENT_SHARED` is a conservative marker obliging an
//! overlap lookup, and this tree is the authority. Records are canonical —
//! ordered, non-overlapping, maximal (adjacent contiguous records with equal
//! counts must have been merged) — so a given sharing state has exactly one
//! representation and the checker compares without guessing.
//!
//! Loading and overlap resolution are deliberately autonomous: the checker
//! validates sharing through these functions without depending on the write
//! path's own readers.

use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::le;
use afsplus_format::tree::{child_value, key_u64, ChildRef, TreeItem, TreeKind, TreeNode};

use crate::tree::{visit_tree_nodes, TreeSpec, TreeSummary};
use crate::CoreError;

pub const VALUE_SIZE: usize = 16;

/// A record never stores fewer references than this: a run with a single
/// reference is an ordinary private run and has no record (ADR-061).
pub const MIN_REFERENCE_COUNT: u32 = 2;

/// One shared physical run and its authoritative reference count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedRun {
    pub physical_start: u64,
    pub block_count: u64,
    pub reference_count: u32,
    /// Reserved; must be zero.
    pub flags: u32,
}

impl SharedRun {
    pub fn physical_end(self) -> Result<u64, CoreError> {
        self.physical_start
            .checked_add(self.block_count)
            .ok_or_else(|| CoreError::Corrupt("shared run end overflows".into()))
    }
}

/// One segment of an extent partitioned against the reference tree.
/// `reference_count` is `None` for a private gap (no overlapping record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRun {
    pub physical_start: u64,
    pub block_count: u64,
    pub reference_count: Option<u32>,
}

pub struct LoadedSharedExtents {
    pub records: Vec<SharedRun>,
    pub tree_blocks: Vec<u64>,
    pub summary: TreeSummary,
}

pub fn spec(max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::SharedExtents,
        owner: 0,
        max_generation,
    }
}

pub fn empty_leaf() -> TreeNode {
    TreeNode::leaf(TreeKind::SharedExtents, 0)
}

pub fn encode_run(run: SharedRun) -> Result<([u8; 8], [u8; VALUE_SIZE]), CoreError> {
    validate_run_shape(run)?;
    let mut value = [0u8; VALUE_SIZE];
    le::put_u64(&mut value[0..8], run.block_count);
    le::put_u32(&mut value[8..12], run.reference_count);
    le::put_u32(&mut value[12..16], run.flags);
    Ok((key_u64(run.physical_start), value))
}

pub fn item(run: SharedRun) -> Result<TreeItem, CoreError> {
    let (key, value) = encode_run(run)?;
    Ok(TreeItem {
        key: key.into(),
        value: value.into(),
    })
}

pub fn decode_run(key: &[u8], value: &[u8], geo: &Geometry) -> Result<SharedRun, CoreError> {
    let key: [u8; 8] = key
        .try_into()
        .map_err(|_| CoreError::Corrupt("shared-run key is not eight bytes".into()))?;
    if value.len() != VALUE_SIZE {
        return Err(CoreError::Corrupt(
            "shared-run value is not sixteen bytes".into(),
        ));
    }
    let run = SharedRun {
        physical_start: u64::from_be_bytes(key),
        block_count: le::get_u64(&value[0..8]),
        reference_count: le::get_u32(&value[8..12]),
        flags: le::get_u32(&value[12..16]),
    };
    validate_run_shape(run)?;
    validate_physical_range(run, geo)?;
    Ok(run)
}

fn validate_run_shape(run: SharedRun) -> Result<(), CoreError> {
    if run.block_count == 0 {
        return Err(CoreError::Corrupt("zero-length shared run".into()));
    }
    run.physical_end()?;
    if run.reference_count < MIN_REFERENCE_COUNT {
        return Err(CoreError::Corrupt(
            "shared run with fewer than two references".into(),
        ));
    }
    if run.flags != 0 {
        return Err(CoreError::Corrupt(
            "shared-run reserved flags are nonzero".into(),
        ));
    }
    Ok(())
}

/// Shared runs describe the same physical data runs as extents, so the same
/// bounds rule applies: allocatable, and within one allocation region.
fn validate_physical_range(run: SharedRun, geo: &Geometry) -> Result<(), CoreError> {
    let end = run.physical_end()?;
    if end > geo.total_blocks
        || !geo.is_allocatable(run.physical_start)
        || !geo.is_allocatable(end - 1)
        || geo.region_of(run.physical_start) != geo.region_of(end - 1)
    {
        return Err(CoreError::Corrupt(format!(
            "shared run {}..{end} is not allocatable",
            run.physical_start
        )));
    }
    Ok(())
}

/// Rejects any record sequence that is not the canonical form: strictly
/// ordered, non-overlapping, and maximal (an adjacent contiguous pair with
/// equal counts should have been merged into one record).
pub fn validate_canonical(records: &[SharedRun]) -> Result<(), CoreError> {
    for pair in records.windows(2) {
        let end = pair[0].physical_end()?;
        if end > pair[1].physical_start {
            return Err(CoreError::Corrupt("shared runs overlap".into()));
        }
        if end == pair[1].physical_start && pair[0].reference_count == pair[1].reference_count {
            return Err(CoreError::Corrupt(
                "adjacent shared runs with equal counts are not merged".into(),
            ));
        }
    }
    Ok(())
}

/// Exhaustively loads and validates the reference tree. `root_lba` must be
/// nonzero; a volume that has never cloned has no tree at all.
pub fn load_all<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    max_generation: u64,
) -> Result<LoadedSharedExtents, CoreError> {
    let mut records = Vec::new();
    let mut tree_blocks = Vec::new();
    let summary = visit_tree_nodes(dev, geo, root_lba, spec(max_generation), |lba, node| {
        tree_blocks.push(lba);
        if node.is_leaf() {
            for item in &node.items {
                records.push(decode_run(&item.key, &item.value, geo)?);
            }
        }
        Ok(())
    })?;
    if records.len() as u64 != summary.items {
        return Err(CoreError::Corrupt(
            "shared-run leaf count does not match tree summary".into(),
        ));
    }
    validate_canonical(&records)?;
    Ok(LoadedSharedExtents {
        records,
        tree_blocks,
        summary,
    })
}

/// Partitions one physical run against the canonical record sequence.
///
/// This is the mandatory resolution for any operation on an extent flagged
/// `EXTENT_SHARED`: peers split the underlying runs independently, so one
/// flagged extent may cover shared sub-runs and private gaps. An exact
/// lookup on the extent's own start would find the head and miss the tail
/// (ADR-061). Only the returned private gaps are sole-owned.
pub fn resolve_overlaps(
    physical_start: u64,
    block_count: u64,
    records: &[SharedRun],
) -> Result<Vec<SubRun>, CoreError> {
    if block_count == 0 {
        return Err(CoreError::Corrupt("zero-length overlap query".into()));
    }
    let end = physical_start
        .checked_add(block_count)
        .ok_or_else(|| CoreError::Corrupt("overlap query end overflows".into()))?;

    let mut segments = Vec::new();
    let mut cursor = physical_start;
    // First record whose end lies past the query start; canonical ordering
    // makes every earlier record irrelevant.
    let mut index =
        records.partition_point(|run| run.physical_start.saturating_add(run.block_count) <= cursor);
    while cursor < end && index < records.len() {
        let run = records[index];
        if run.physical_start >= end {
            break;
        }
        if run.physical_start > cursor {
            segments.push(SubRun {
                physical_start: cursor,
                block_count: run.physical_start - cursor,
                reference_count: None,
            });
            cursor = run.physical_start;
        }
        let segment_end = run.physical_end()?.min(end);
        segments.push(SubRun {
            physical_start: cursor,
            block_count: segment_end - cursor,
            reference_count: Some(run.reference_count),
        });
        cursor = segment_end;
        index += 1;
    }
    if cursor < end {
        segments.push(SubRun {
            physical_start: cursor,
            block_count: end - cursor,
            reference_count: None,
        });
    }
    Ok(segments)
}

/// Per-block accounting of one transaction's reference edits, for
/// `CommitStats` (ADR-061 review request: promotion, new sharing, decrements
/// and privatisation transitions are separately observable).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefEditStats {
    /// Blocks that went from one implicit reference to a first record (1→2).
    pub blocks_newly_shared: u64,
    /// Blocks whose existing record count was incremented (≥2 → +1).
    pub blocks_reference_incremented: u64,
    /// Blocks whose record count was decremented and kept a record (>2 → −1).
    pub blocks_reference_decremented: u64,
    /// Blocks whose record was removed because the count reached one (2→1).
    /// Their storage is NOT retired: one live mapping remains.
    pub blocks_privatized: u64,
}

impl RefEditStats {
    pub fn changed(&self) -> bool {
        *self != RefEditStats::default()
    }
}

/// The publication of one transaction's reference edits: the tree operations
/// that turn the committed canonical form into the new one, plus the new
/// canonical record list itself.
pub struct RefPublication {
    pub deletes: Vec<[u8; 8]>,
    pub upserts: Vec<([u8; 8], [u8; VALUE_SIZE])>,
    /// The canonical records of the overlay's fetched window. Complete for
    /// the whole volume only when the tree did not exist yet (first clone),
    /// which is exactly when the initial bulk build consumes it.
    pub records: Vec<SharedRun>,
    pub stats: RefEditStats,
    root_required: bool,
}

impl RefPublication {
    pub fn changed(&self) -> bool {
        !self.deletes.is_empty() || !self.upserts.is_empty()
    }

    /// True when a root must exist after this transaction even without a
    /// record change (the empty first clone).
    pub fn root_required(&self) -> bool {
        self.root_required
    }
}

/// Transactional edit over a bounded window of the committed canonical
/// records. The caller prefetches, through the bounded tree range walk, only
/// the records overlapping the runs an operation touches (plus their
/// immediate neighbours, so canonical merging stays local); the overlay
/// never materialises the volume-wide tree. `acquire` and `release`
/// implement the ADR-061 tables; `finish` re-canonicalises and diffs.
pub struct RefEdit {
    original: BTreeMap<u64, SharedRun>,
    current: BTreeMap<u64, SharedRun>,
    /// Physical intervals whose committed records are loaded (merged,
    /// sorted). An edit outside them is refused fail-closed: it would
    /// silently treat unknown shared state as private.
    fetched: Vec<(u64, u64)>,
    stats: RefEditStats,
    root_required: bool,
}

impl Default for RefEdit {
    fn default() -> Self {
        Self::new()
    }
}

impl RefEdit {
    pub fn new() -> Self {
        RefEdit {
            original: BTreeMap::new(),
            current: BTreeMap::new(),
            fetched: Vec::new(),
            stats: RefEditStats::default(),
            root_required: false,
        }
    }

    /// The volume has no reference tree: every range is known empty.
    pub fn mark_all_fetched(&mut self) {
        self.fetched = vec![(0, u64::MAX)];
    }

    /// ADR-061: the first `CloneFile` allocates the root even when it shares
    /// nothing (an empty or fully sparse source), so root zero keeps meaning
    /// "this volume has never cloned".
    pub fn require_root(&mut self) {
        self.root_required = true;
    }

    pub fn covers(&self, start: u64, end: u64) -> bool {
        self.fetched
            .iter()
            .any(|(low, high)| *low <= start && end <= *high)
    }

    /// Registers the committed records fetched for `[start, end)`. A record
    /// already present in the overlay keeps its edited state.
    pub fn note_fetched(&mut self, start: u64, end: u64, records: Vec<SharedRun>) {
        for run in records {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                self.original.entry(run.physical_start)
            {
                slot.insert(run);
                self.current.insert(run.physical_start, run);
            }
        }
        self.fetched.push((start, end));
        self.fetched.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(self.fetched.len());
        for (low, high) in self.fetched.drain(..) {
            match merged.last_mut() {
                Some((_, previous_high)) if low <= *previous_high => {
                    *previous_high = (*previous_high).max(high);
                }
                _ => merged.push((low, high)),
            }
        }
        self.fetched = merged;
    }

    fn ensure_covered(&self, start: u64, end: u64) -> Result<(), CoreError> {
        if self.covers(start, end) {
            Ok(())
        } else {
            Err(CoreError::Corrupt(format!(
                "shared-reference edit touches {start}..{end} without prefetching it"
            )))
        }
    }

    fn sorted_current(&self) -> Vec<SharedRun> {
        self.current.values().copied().collect()
    }

    /// Partitions `[start, start+count)` against the current records.
    pub fn resolve(&self, start: u64, count: u64) -> Result<Vec<SubRun>, CoreError> {
        let end = start
            .checked_add(count)
            .ok_or_else(|| CoreError::Corrupt("overlap query end overflows".into()))?;
        self.ensure_covered(start, end)?;
        resolve_overlaps(start, count, &self.sorted_current())
    }

    /// Replaces the covered segment of the record containing it. The segment
    /// lies within exactly one record by construction of `resolve_overlaps`.
    fn rewrite_segment(
        &mut self,
        segment_start: u64,
        segment_end: u64,
        new_count: Option<u32>,
    ) -> Result<(), CoreError> {
        let (&record_start, &record) = self
            .current
            .range(..=segment_start)
            .next_back()
            .ok_or_else(|| CoreError::Corrupt("shared segment without a record".into()))?;
        let record_end = record.physical_end()?;
        if segment_start < record_start || segment_end > record_end {
            return Err(CoreError::Corrupt(
                "shared segment escapes its record".into(),
            ));
        }
        self.current.remove(&record_start);
        if record_start < segment_start {
            self.current.insert(
                record_start,
                SharedRun {
                    physical_start: record_start,
                    block_count: segment_start - record_start,
                    ..record
                },
            );
        }
        if let Some(count) = new_count {
            self.current.insert(
                segment_start,
                SharedRun {
                    physical_start: segment_start,
                    block_count: segment_end - segment_start,
                    reference_count: count,
                    flags: 0,
                },
            );
        }
        if segment_end < record_end {
            self.current.insert(
                segment_end,
                SharedRun {
                    physical_start: segment_end,
                    block_count: record_end - segment_end,
                    ..record
                },
            );
        }
        Ok(())
    }

    /// Adds one reference over the run (clone side). A previously private
    /// segment gains a record at two references; an existing record is
    /// incremented, refusing overflow rather than wrapping.
    pub fn acquire(&mut self, start: u64, count: u64) -> Result<(), CoreError> {
        for segment in self.resolve(start, count)? {
            let end = segment.physical_start + segment.block_count;
            match segment.reference_count {
                Some(references) => {
                    let raised = references.checked_add(1).ok_or(CoreError::PrototypeLimit(
                        "shared-extent reference count limit reached",
                    ))?;
                    self.rewrite_segment(segment.physical_start, end, Some(raised))?;
                    self.stats.blocks_reference_incremented += segment.block_count;
                }
                None => {
                    self.current.insert(
                        segment.physical_start,
                        SharedRun {
                            physical_start: segment.physical_start,
                            block_count: segment.block_count,
                            reference_count: MIN_REFERENCE_COUNT,
                            flags: 0,
                        },
                    );
                    self.stats.blocks_newly_shared += segment.block_count;
                }
            }
        }
        Ok(())
    }

    /// Drops one reference over the run and returns the sub-runs that were
    /// sole-owned (private gaps): only those may be retired. A record at two
    /// references is removed WITHOUT returning its blocks — one live mapping
    /// remains, and retiring here is the data-destroying bug the ADR-061
    /// review caught (rc 2→1 privatises, never frees).
    pub fn release(&mut self, start: u64, count: u64) -> Result<Vec<(u64, u64)>, CoreError> {
        let mut retire = Vec::new();
        for segment in self.resolve(start, count)? {
            let end = segment.physical_start + segment.block_count;
            match segment.reference_count {
                Some(references) if references > MIN_REFERENCE_COUNT => {
                    self.rewrite_segment(segment.physical_start, end, Some(references - 1))?;
                    self.stats.blocks_reference_decremented += segment.block_count;
                }
                Some(_) => {
                    self.rewrite_segment(segment.physical_start, end, None)?;
                    self.stats.blocks_privatized += segment.block_count;
                }
                None => retire.push((segment.physical_start, segment.block_count)),
            }
        }
        Ok(retire)
    }

    /// Canonicalises (merges adjacent contiguous records with equal counts)
    /// and diffs against the committed form. Keys are physical starts, so a
    /// split or merge shows up as delete-plus-upsert pairs.
    pub fn finish(mut self) -> Result<RefPublication, CoreError> {
        let mut merged: Vec<SharedRun> = Vec::with_capacity(self.current.len());
        for run in self.current.values().copied() {
            if let Some(previous) = merged.last_mut() {
                if previous.physical_end()? == run.physical_start
                    && previous.reference_count == run.reference_count
                {
                    previous.block_count = previous
                        .block_count
                        .checked_add(run.block_count)
                        .ok_or_else(|| CoreError::Corrupt("merged shared run overflows".into()))?;
                    continue;
                }
            }
            merged.push(run);
        }
        self.current = merged
            .iter()
            .map(|run| (run.physical_start, *run))
            .collect();
        validate_canonical(&merged)?;

        let mut deletes = Vec::new();
        let mut upserts = Vec::new();
        for start in self.original.keys() {
            if !self.current.contains_key(start) {
                deletes.push(key_u64(*start));
            }
        }
        for (start, run) in &self.current {
            if self.original.get(start) != Some(run) {
                let (key, value) = encode_run(*run)?;
                upserts.push((key, value));
            }
        }
        Ok(RefPublication {
            deletes,
            upserts,
            records: merged,
            stats: self.stats,
            root_required: self.root_required,
        })
    }
}

pub struct BuiltSharedTree {
    pub root_lba: u64,
    pub nodes: Vec<(u64, TreeNode)>,
}

fn leaf_capacity(block_size: usize) -> Result<usize, CoreError> {
    let mut node = empty_leaf();
    let mut capacity = 0usize;
    loop {
        node.items.push(item(SharedRun {
            physical_start: capacity as u64 * 2 + 8,
            block_count: 1,
            reference_count: MIN_REFERENCE_COUNT,
            flags: 0,
        })?);
        node.subtree_items = node.items.len() as u64;
        if !node.fits(block_size) {
            break;
        }
        capacity += 1;
    }
    if capacity == 0 {
        return Err(CoreError::UnsupportedGeometry(
            "block cannot hold one shared-run record",
        ));
    }
    Ok(capacity)
}

fn internal_fanout(block_size: usize) -> Result<usize, CoreError> {
    let mut node = TreeNode {
        kind: TreeKind::SharedExtents,
        owner: 0,
        level: 1,
        subtree_items: 1,
        leftmost_child: 1,
        leftmost_items: 1,
        items: Vec::new(),
    };
    let mut children = 1usize;
    loop {
        node.items.push(TreeItem {
            key: key_u64(children as u64).into(),
            value: child_value(ChildRef {
                lba: children as u64 + 1,
                subtree_items: 1,
            })
            .map_err(CoreError::Format)?
            .into(),
        });
        node.subtree_items = children as u64 + 1;
        if !node.fits(block_size) {
            break;
        }
        children += 1;
    }
    if children < 2 {
        return Err(CoreError::UnsupportedGeometry(
            "block cannot hold two shared-tree children",
        ));
    }
    Ok(children)
}

/// Blocks needed for a balanced initial tree, so the first clone can reserve
/// every LBA transactionally before building (like the extent maps).
pub fn bulk_node_count(block_size: usize, record_count: usize) -> Result<usize, CoreError> {
    if record_count == 0 {
        return Ok(1);
    }
    let leaf_capacity = leaf_capacity(block_size)?;
    let fanout = internal_fanout(block_size)?;
    let mut level_nodes = record_count.div_ceil(leaf_capacity);
    let mut nodes = level_nodes;
    while level_nodes > 1 {
        level_nodes = level_nodes.div_ceil(fanout);
        nodes = nodes
            .checked_add(level_nodes)
            .ok_or_else(|| CoreError::Corrupt("shared tree node count overflows".into()))?;
    }
    Ok(nodes)
}

/// Builds the volume's first reference tree from the canonical records,
/// balanced across exactly the supplied transactionally allocated LBAs.
/// Later growth goes through the generic COW engine.
pub fn bulk_build(
    block_size: usize,
    records: &[SharedRun],
    lbas: &[u64],
) -> Result<BuiltSharedTree, CoreError> {
    validate_canonical(records)?;
    let expected = bulk_node_count(block_size, records.len())?;
    if lbas.len() != expected {
        return Err(CoreError::Corrupt(
            "shared tree bulk-build LBA count mismatch".into(),
        ));
    }
    if records.is_empty() {
        return Ok(BuiltSharedTree {
            root_lba: lbas[0],
            nodes: vec![(lbas[0], empty_leaf())],
        });
    }
    let leaf_capacity = leaf_capacity(block_size)?;
    let fanout = internal_fanout(block_size)?;
    let mut next_lba = lbas.iter().copied();
    let mut nodes = Vec::with_capacity(expected);
    let mut level_nodes = Vec::new();
    let mut offset = 0usize;
    for group_len in crate::extent_map::balanced_groups(records.len(), leaf_capacity)? {
        let lba = next_lba
            .next()
            .ok_or_else(|| CoreError::Corrupt("shared bulk-build pool exhausted".into()))?;
        let mut node = empty_leaf();
        for run in &records[offset..offset + group_len] {
            node.items.push(item(*run)?);
        }
        node.subtree_items = node.items.len() as u64;
        level_nodes.push(BulkChild {
            lba,
            min_key: node.items[0].key.clone().to_vec(),
            items: node.subtree_items,
        });
        nodes.push((lba, node));
        offset += group_len;
    }

    let mut level = 0u8;
    while level_nodes.len() > 1 {
        level = level
            .checked_add(1)
            .filter(|level| *level <= afsplus_format::tree::MAX_TREE_LEVEL)
            .ok_or(CoreError::PrototypeLimit(
                "shared tree height limit reached",
            ))?;
        let mut next_level = Vec::new();
        let mut child_offset = 0usize;
        for group_len in crate::extent_map::balanced_groups(level_nodes.len(), fanout)? {
            let children = &level_nodes[child_offset..child_offset + group_len];
            let lba = next_lba
                .next()
                .ok_or_else(|| CoreError::Corrupt("shared bulk-build pool exhausted".into()))?;
            let mut total = children[0].items;
            let mut items = Vec::with_capacity(children.len() - 1);
            for child in &children[1..] {
                total = total
                    .checked_add(child.items)
                    .ok_or_else(|| CoreError::Corrupt("shared record count overflows".into()))?;
                items.push(TreeItem {
                    key: child.min_key.clone().into(),
                    value: child_value(ChildRef {
                        lba: child.lba,
                        subtree_items: child.items,
                    })
                    .map_err(CoreError::Format)?
                    .into(),
                });
            }
            let node = TreeNode {
                kind: TreeKind::SharedExtents,
                owner: 0,
                level,
                subtree_items: total,
                leftmost_child: children[0].lba,
                leftmost_items: children[0].items,
                items,
            };
            next_level.push(BulkChild {
                lba,
                min_key: children[0].min_key.clone(),
                items: total,
            });
            nodes.push((lba, node));
            child_offset += group_len;
        }
        level_nodes = next_level;
    }
    if next_lba.next().is_some() || nodes.len() != expected {
        return Err(CoreError::Corrupt(
            "shared bulk-build node count mismatch".into(),
        ));
    }
    Ok(BuiltSharedTree {
        root_lba: level_nodes[0].lba,
        nodes,
    })
}

#[derive(Clone)]
struct BulkChild {
    lba: u64,
    min_key: Vec<u8>,
    items: u64,
}

#[cfg(test)]
mod tests {
    use super::{resolve_overlaps, validate_canonical, SharedRun, SubRun};

    fn run(start: u64, blocks: u64, count: u32) -> SharedRun {
        SharedRun {
            physical_start: start,
            block_count: blocks,
            reference_count: count,
            flags: 0,
        }
    }

    /// The ADR-061 example: A and B share [0,100), B rewrites [40,60).
    /// A's flagged extent partitions into shared head, private gap, shared
    /// tail — an exact lookup on the start would miss the tail.
    #[test]
    fn overlap_partition_finds_the_tail() {
        let records = [run(0, 40, 2), run(60, 40, 2)];
        let segments = resolve_overlaps(0, 100, &records).unwrap();
        assert_eq!(
            segments,
            vec![
                SubRun {
                    physical_start: 0,
                    block_count: 40,
                    reference_count: Some(2),
                },
                SubRun {
                    physical_start: 40,
                    block_count: 20,
                    reference_count: None,
                },
                SubRun {
                    physical_start: 60,
                    block_count: 40,
                    reference_count: Some(2),
                },
            ]
        );
    }

    #[test]
    fn overlap_partition_clips_to_the_query() {
        let records = [run(10, 80, 3)];
        let segments = resolve_overlaps(30, 20, &records).unwrap();
        assert_eq!(
            segments,
            vec![SubRun {
                physical_start: 30,
                block_count: 20,
                reference_count: Some(3),
            }]
        );
        // A query entirely inside a gap is one private segment.
        let segments = resolve_overlaps(100, 10, &records).unwrap();
        assert_eq!(
            segments,
            vec![SubRun {
                physical_start: 100,
                block_count: 10,
                reference_count: None,
            }]
        );
    }

    #[test]
    fn bulk_build_balances_past_one_leaf_and_reloads() {
        use afsplus_block::{BlockDevice, MemoryBackend};
        use afsplus_format::geometry::Geometry;

        // Enough canonical records to force several leaves and one internal
        // level; alternating counts keep the sequence maximal.
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 1 << 20,
            region_size: 1 << 20,
        };
        let records: Vec<_> = (0..500u64)
            .map(|index| run(8192 + index * 4, 2, 2 + (index % 2) as u32))
            .collect();
        let node_count = super::bulk_node_count(4096, records.len()).unwrap();
        assert!(
            node_count > 1,
            "expected a multi-node tree, got {node_count}"
        );
        let lbas: Vec<u64> = (0..node_count as u64).map(|index| 512 + index).collect();
        let built = super::bulk_build(4096, &records, &lbas).unwrap();
        assert_eq!(built.nodes.len(), node_count);

        // Only the tree nodes are written; the loader never reads the runs.
        let mut dev = MemoryBackend::new(4096, 1024);
        for (lba, node) in &built.nodes {
            dev.write_block(*lba, &node.encode(4096, 1).unwrap())
                .unwrap();
        }
        let loaded = super::load_all(&mut dev, &geo, built.root_lba, 1).unwrap();
        assert_eq!(loaded.records, records);
        assert_eq!(loaded.tree_blocks.len(), node_count);
    }

    #[test]
    fn canonical_form_is_enforced() {
        validate_canonical(&[run(0, 10, 2), run(10, 5, 3), run(20, 5, 3)]).unwrap();
        // Overlap.
        assert!(validate_canonical(&[run(0, 10, 2), run(9, 5, 3)]).is_err());
        // Adjacent contiguous equal counts must have merged.
        assert!(validate_canonical(&[run(0, 10, 2), run(10, 5, 2)]).is_err());
    }
}
