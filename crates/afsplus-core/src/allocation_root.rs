//! Typed allocation-root adapter and reserved COW-node pool.
//!
//! Region records have a fixed key set for the lifetime of a volume. Their
//! shared-tree nodes live in a permanently allocated pool rather than the
//! free space described by the tree itself, breaking the allocation
//! self-reference. Three physical generations per logical-node upper bound
//! leave one writable image while two checkpoints remain selectable.

use std::collections::BTreeSet;

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::RegionRecord;
use afsplus_format::geometry::{Geometry, DESCRIPTOR_SLOTS};
use afsplus_format::le;
use afsplus_format::small_bytes::SmallBytes;
use afsplus_format::tree::{child_value, ChildRef, TreeItem, TreeKind, TreeNode};

use crate::cow_tree::TreeAllocator;
use crate::tree::{lookup, visit_tree_nodes, TreeSpec, TreeSummary};
use crate::CoreError;

const VALUE_BYTES: usize = 16;
/// The placement rule of the permanent areas is format knowledge and lives
/// in `afsplus_format::geometry`; this module applies it.
use afsplus_format::geometry::BOOTSTRAP_METADATA_BLOCKS;

pub struct LoadedAllocationRoot {
    pub records: Vec<RegionRecord>,
    pub tree_blocks: Vec<u64>,
    pub summary: TreeSummary,
}

pub struct BuiltAllocationRoot {
    pub root_lba: u64,
    pub pool_lbas: Vec<u64>,
    pub nodes: Vec<(u64, TreeNode)>,
}

pub fn spec(max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::AllocationRoot,
        owner: 0,
        max_generation,
    }
}

pub fn key(region: u32) -> [u8; 4] {
    region.to_be_bytes()
}

pub fn value(record: RegionRecord) -> Result<[u8; VALUE_BYTES], CoreError> {
    match validate_record(record) {
        RecordValidation::Valid => {}
        RecordValidation::DescriptorSlot => {
            return Err(CoreError::Corrupt(
                "allocation-root descriptor slot out of range".into(),
            ));
        }
        RecordValidation::DescriptorGeneration => {
            return Err(CoreError::Corrupt(
                "allocation-root descriptor generation is zero".into(),
            ));
        }
    }
    let mut encoded = [0u8; VALUE_BYTES];
    encoded[0] = record.descriptor_slot;
    encoded[4..8].copy_from_slice(&record.free_blocks.to_le_bytes());
    encoded[8..16].copy_from_slice(&record.descriptor_generation.to_le_bytes());
    Ok(encoded)
}

pub fn initial_leaf(region: u32, record: RegionRecord) -> Result<TreeNode, CoreError> {
    Ok(TreeNode {
        kind: TreeKind::AllocationRoot,
        owner: 0,
        level: 0,
        subtree_items: 1,
        leftmost_child: 0,
        leftmost_items: 0,
        items: vec![TreeItem {
            key: key(region).into(),
            value: value(record)?.into(),
        }],
    })
}

/// Deterministically bulk-builds the fixed region-key tree and reserves three
/// physical images per logical node. `records` must describe every region in
/// numeric order.
pub fn bulk_build(
    geo: &Geometry,
    records: &[RegionRecord],
) -> Result<BuiltAllocationRoot, CoreError> {
    if records.len() != geo.region_count() as usize || records.is_empty() {
        return Err(CoreError::Corrupt(
            "allocation-root bulk build record count mismatch".into(),
        ));
    }
    let leaf_capacity = leaf_capacity(geo.block_size)?;
    let internal_fanout = internal_fanout(geo.block_size)?;
    let leaf_groups = balanced_groups(records.len(), leaf_capacity)?;
    let pool_lbas = reserved_pool_lbas(geo)?;
    let logical_nodes = pool_lbas.len() / 3;
    let mut next_lba = pool_lbas[..logical_nodes].iter().copied();
    let mut nodes = Vec::with_capacity(logical_nodes);
    let mut level_nodes = Vec::with_capacity(leaf_groups.len());
    let mut record_offset = 0usize;
    for group_len in leaf_groups {
        let lba = next_lba
            .next()
            .ok_or_else(|| CoreError::Corrupt("allocation-root active pool exhausted".into()))?;
        let mut node = TreeNode::leaf(TreeKind::AllocationRoot, 0);
        for (region, record) in records
            .iter()
            .enumerate()
            .skip(record_offset)
            .take(group_len)
        {
            node.items.push(TreeItem {
                key: key(region as u32).into(),
                value: value(*record)?.into(),
            });
        }
        node.subtree_items = node.items.len() as u64;
        let min_key = node.items[0].key.clone();
        level_nodes.push(BulkChild {
            lba,
            min_key,
            items: node.subtree_items,
        });
        nodes.push((lba, node));
        record_offset += group_len;
    }

    let mut level = 0u8;
    while level_nodes.len() > 1 {
        level = level
            .checked_add(1)
            .ok_or_else(|| CoreError::Corrupt("allocation-root level overflow".into()))?;
        let groups = balanced_groups(level_nodes.len(), internal_fanout)?;
        let mut next_level = Vec::with_capacity(groups.len());
        let mut child_offset = 0usize;
        for group_len in groups {
            let children = &level_nodes[child_offset..child_offset + group_len];
            let lba = next_lba.next().ok_or_else(|| {
                CoreError::Corrupt("allocation-root active pool exhausted".into())
            })?;
            let mut total = children[0].items;
            let mut items = Vec::with_capacity(children.len() - 1);
            for child in &children[1..] {
                total = total.checked_add(child.items).ok_or_else(|| {
                    CoreError::Corrupt("allocation-root item count overflow".into())
                })?;
                items.push(TreeItem {
                    key: child.min_key.clone(),
                    value: child_value(ChildRef {
                        lba: child.lba,
                        subtree_items: child.items,
                    })
                    .map_err(CoreError::Format)?
                    .into(),
                });
            }
            let node = TreeNode {
                kind: TreeKind::AllocationRoot,
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
    if next_lba.next().is_some() || nodes.len() != logical_nodes {
        return Err(CoreError::Corrupt(
            "allocation-root logical node count mismatch".into(),
        ));
    }
    Ok(BuiltAllocationRoot {
        root_lba: level_nodes[0].lba,
        pool_lbas,
        nodes,
    })
}

/// Deterministic permanent `3N` node pool for the fixed region-key topology,
/// independent of the current record values.
pub fn reserved_pool_lbas(geo: &Geometry) -> Result<Vec<u64>, CoreError> {
    geo.allocation_root_pool_lbas().map_err(placement_error)
}

fn reserved_pool_block_count(geo: &Geometry) -> Result<usize, CoreError> {
    // The pool always carries exactly three physical generations.
    geo.allocation_root_logical_nodes()
        .map_err(placement_error)?
        .checked_mul(3)
        .ok_or_else(|| CoreError::Corrupt("allocation-root pool size overflow".into()))
}

fn placement_error(error: afsplus_format::FormatError) -> CoreError {
    match error {
        afsplus_format::FormatError::Invalid(message) => CoreError::UnsupportedGeometry(message),
        other => CoreError::Format(other),
    }
}

/// Half-open physical envelope of the permanent pool, without enumerating
/// regions or pool blocks. Reserved region heads inside the envelope are
/// already excluded by geometry; bootstrap namespace blocks precede it.
pub fn reserved_pool_bounds(geo: &Geometry) -> Result<(u64, u64), CoreError> {
    geo.validate()?;
    let count = reserved_pool_block_count(geo)? as u64;
    let first = allocatable_at(geo, BOOTSTRAP_METADATA_BLOCKS as u64)?;
    let last = allocatable_at(geo, BOOTSTRAP_METADATA_BLOCKS as u64 + count - 1)?;
    Ok((first, last + 1))
}

fn allocatable_at(geo: &Geometry, ordinal: u64) -> Result<u64, CoreError> {
    use afsplus_format::bitmap::BITMAP_PAGE_BLOCKS;
    use afsplus_format::geometry::BITMAP_SLOTS;
    let first_capacity = geo.region_valid_blocks(0) as u64 - geo.region0_reserved_blocks();
    let lba = if ordinal < first_capacity {
        geo.region0_reserved_blocks() + ordinal
    } else {
        let full_reserved = DESCRIPTOR_SLOTS as u64
            + geo.region_size.div_ceil(BITMAP_PAGE_BLOCKS) as u64 * BITMAP_SLOTS as u64;
        let capacity = geo.region_size as u64 - full_reserved;
        let remainder = ordinal - first_capacity;
        let region = 1 + remainder / capacity;
        if region >= geo.region_count() as u64 {
            return Err(CoreError::UnsupportedGeometry(
                "volume cannot hold allocation-root pool",
            ));
        }
        geo.region_base(region as u32)
            + geo.region_reserved_blocks(region as u32)
            + remainder % capacity
    };
    if !geo.is_allocatable(lba) {
        return Err(CoreError::UnsupportedGeometry(
            "volume cannot hold allocation-root pool",
        ));
    }
    Ok(lba)
}

#[derive(Clone)]
struct BulkChild {
    lba: u64,
    min_key: SmallBytes,
    items: u64,
}

fn leaf_capacity(block_size: usize) -> Result<usize, CoreError> {
    let capacity =
        TreeNode::fixed_item_capacity(block_size, 4, VALUE_BYTES).map_err(CoreError::Format)?;
    if capacity == 0 {
        return Err(CoreError::UnsupportedGeometry(
            "block cannot hold one allocation-root record",
        ));
    }
    Ok(capacity)
}

fn internal_fanout(block_size: usize) -> Result<usize, CoreError> {
    let separators = TreeNode::fixed_item_capacity(block_size, 4, 16).map_err(CoreError::Format)?;
    let fanout = separators + 1;
    if fanout < 2 {
        return Err(CoreError::UnsupportedGeometry(
            "block cannot hold an internal allocation-root node",
        ));
    }
    Ok(fanout)
}

fn balanced_groups(total: usize, maximum: usize) -> Result<Vec<usize>, CoreError> {
    if total == 0 || maximum == 0 {
        return Err(CoreError::Corrupt(
            "cannot partition empty allocation-root level".into(),
        ));
    }
    let groups = total.div_ceil(maximum);
    let base = total / groups;
    let remainder = total % groups;
    if groups > 1 && base < 2 {
        return Err(CoreError::UnsupportedGeometry(
            "allocation-root fanout cannot form valid internal nodes",
        ));
    }
    Ok((0..groups)
        .map(|index| base + usize::from(index < remainder))
        .collect())
}

/// Allocatable blocks occupied by the bootstrap metadata plus the pool;
/// the intent-log area starts after this span.
pub fn reserved_span_before_log(geo: &Geometry) -> Result<usize, CoreError> {
    Ok(BOOTSTRAP_METADATA_BLOCKS + reserved_pool_lbas(geo)?.len())
}

pub fn lookup_record<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    max_generation: u64,
    region: u32,
) -> Result<Option<RegionRecord>, CoreError> {
    if region >= geo.region_count() {
        return Err(CoreError::Corrupt(format!(
            "allocation-root region {region} outside geometry"
        )));
    }
    let (encoded, _) = lookup(dev, geo, root_lba, spec(max_generation), &key(region))?;
    encoded
        .map(|encoded| decode_value(&encoded, geo, region, max_generation))
        .transpose()
}

pub fn load_all<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    max_generation: u64,
) -> Result<LoadedAllocationRoot, CoreError> {
    let mut keyed_records = Vec::new();
    let mut tree_blocks = Vec::new();
    let summary = visit_tree_nodes(dev, geo, root_lba, spec(max_generation), |lba, node| {
        tree_blocks.push(lba);
        if node.is_leaf() {
            for item in &node.items {
                let region = decode_key(&item.key)?;
                keyed_records.push((
                    region,
                    decode_value(&item.value, geo, region, max_generation)?,
                ));
            }
        }
        Ok(())
    })?;
    if keyed_records.len() != geo.region_count() as usize
        || keyed_records
            .iter()
            .enumerate()
            .any(|(index, (region, _))| *region as usize != index)
    {
        return Err(CoreError::Corrupt(
            "allocation root does not contain exactly one record per region".into(),
        ));
    }
    Ok(LoadedAllocationRoot {
        records: keyed_records
            .into_iter()
            .map(|(_, record)| record)
            .collect(),
        tree_blocks,
        summary,
    })
}

/// Traverses only generic tree structure to discover the retained physical
/// node set. Ordinary allocation transactions use this to protect the fixed
/// `3N` pool without materializing every typed region record.
pub fn load_tree_blocks<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    max_generation: u64,
) -> Result<Vec<u64>, CoreError> {
    let mut tree_blocks = Vec::new();
    visit_tree_nodes(dev, geo, root_lba, spec(max_generation), |lba, _| {
        tree_blocks.push(lba);
        Ok(())
    })?;
    Ok(tree_blocks)
}

fn decode_key(encoded: &[u8]) -> Result<u32, CoreError> {
    let bytes: [u8; 4] = encoded
        .try_into()
        .map_err(|_| CoreError::Corrupt("allocation-root key is not four bytes".into()))?;
    Ok(u32::from_be_bytes(bytes))
}

fn decode_value(
    encoded: &[u8],
    geo: &Geometry,
    region: u32,
    max_generation: u64,
) -> Result<RegionRecord, CoreError> {
    let bytes: [u8; VALUE_BYTES] = encoded.try_into().map_err(|_| {
        CoreError::Corrupt(format!(
            "allocation-root value for region {region} has wrong size"
        ))
    })?;
    if bytes[1..4] != [0; 3] {
        return Err(CoreError::Corrupt(format!(
            "allocation-root value for region {region} has nonzero reserved bytes"
        )));
    }
    let record = RegionRecord {
        descriptor_slot: bytes[0],
        free_blocks: le::get_u32(&bytes[4..8]),
        descriptor_generation: le::get_u64(&bytes[8..16]),
    };
    match validate_record(record) {
        RecordValidation::Valid => {}
        RecordValidation::DescriptorSlot => {
            return Err(CoreError::Corrupt(
                "allocation-root descriptor slot out of range".into(),
            ));
        }
        RecordValidation::DescriptorGeneration => {
            return Err(CoreError::Corrupt(
                "allocation-root descriptor generation is zero".into(),
            ));
        }
    }
    if region >= geo.region_count() || record.free_blocks > geo.region_valid_blocks(region) {
        return Err(CoreError::Corrupt(format!(
            "allocation-root record for region {region} exceeds geometry"
        )));
    }
    if record.descriptor_generation > max_generation {
        return Err(CoreError::Corrupt(format!(
            "allocation-root record for region {region} is from the future"
        )));
    }
    Ok(record)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordValidation {
    Valid,
    DescriptorSlot,
    DescriptorGeneration,
}

/// Keep the hot validation result scalar. In particular, this avoids forcing
/// every successful record encoding through an indirect `Result<(),
/// CoreError>` return merely to construct an owned diagnostic on the two cold
/// error paths.
fn validate_record(record: RegionRecord) -> RecordValidation {
    if record.descriptor_slot >= DESCRIPTOR_SLOTS {
        return RecordValidation::DescriptorSlot;
    }
    if record.descriptor_generation == 0 {
        return RecordValidation::DescriptorGeneration;
    }
    RecordValidation::Valid
}

/// Allocator for a permanently allocated tree-node pool. Blocks reachable
/// from either retained checkpoint are excluded. A block allocated and then
/// discarded within the same uncommitted mutation may be reused immediately.
pub struct ReservedTreePool {
    tree_cache_pages: std::num::NonZeroUsize,
    pool: BTreeSet<u64>,
    available: BTreeSet<u64>,
    allocated_here: BTreeSet<u64>,
    retired_here: BTreeSet<u64>,
}

impl ReservedTreePool {
    pub(crate) fn with_tree_cache_pages(mut self, pages: std::num::NonZeroUsize) -> Self {
        self.tree_cache_pages = pages;
        self
    }

    /// Final live allocations, including images already spilled to the device.
    /// Discarded transaction-local nodes are removed by release_tree_block.
    pub fn allocated_nodes(&self) -> impl Iterator<Item = u64> + '_ {
        self.allocated_here.iter().copied()
    }
    /// Committed pool nodes this mutation retired (replaced COW paths).
    pub fn retired_nodes(&self) -> impl Iterator<Item = u64> + '_ {
        self.retired_here.iter().copied()
    }

    pub fn new(
        pool_lbas: impl IntoIterator<Item = u64>,
        current_tree: &[u64],
        older_tree: &[u64],
    ) -> Result<Self, CoreError> {
        let pool: BTreeSet<_> = pool_lbas.into_iter().collect();
        if pool.is_empty() {
            return Err(CoreError::Corrupt(
                "allocation-root node pool is empty".into(),
            ));
        }
        let mut available = pool.clone();
        for lba in current_tree.iter().chain(older_tree) {
            if !pool.contains(lba) {
                return Err(CoreError::Corrupt(format!(
                    "allocation-root node {lba} lies outside its reserved pool"
                )));
            }
            available.remove(lba);
        }
        Ok(ReservedTreePool {
            tree_cache_pages: std::num::NonZeroUsize::MAX,
            pool,
            available,
            allocated_here: BTreeSet::new(),
            retired_here: BTreeSet::new(),
        })
    }
}

impl<D: BlockDevice> TreeAllocator<D> for ReservedTreePool {
    fn tree_cache_pages(&self) -> usize {
        self.tree_cache_pages.get()
    }
    fn allocate_tree_block(&mut self, _dev: &mut D) -> Result<u64, CoreError> {
        let lba = self.available.pop_first().ok_or(CoreError::NoSpace)?;
        self.retired_here.remove(&lba);
        self.allocated_here.insert(lba);
        Ok(lba)
    }

    fn retire_tree_block(&mut self, _dev: &mut D, lba: u64) -> Result<(), CoreError> {
        if !self.pool.contains(&lba)
            || self.allocated_here.contains(&lba)
            || !self.retired_here.insert(lba)
        {
            return Err(CoreError::Corrupt(format!(
                "allocation-root pool block {lba} retired invalidly"
            )));
        }
        Ok(())
    }

    fn release_tree_block(&mut self, _dev: &mut D, lba: u64) -> Result<(), CoreError> {
        if !self.pool.contains(&lba) || !self.allocated_here.remove(&lba) {
            return Err(CoreError::Corrupt(format!(
                "allocation-root pool block {lba} released invalidly"
            )));
        }
        self.available.insert(lba);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::checkpoint::RegionRecord;
    use afsplus_format::geometry::Geometry;

    use super::{
        bulk_build, initial_leaf, key, load_all, lookup_record, spec, value, ReservedTreePool,
    };
    use crate::cow_tree::{mutate_many, TreeOperation};
    use crate::CoreError;

    #[test]
    fn record_validation_preserves_owned_error_diagnostics() {
        let invalid_slot = value(RegionRecord {
            descriptor_slot: 3,
            free_blocks: 1,
            descriptor_generation: 1,
        });
        assert!(matches!(
            invalid_slot,
            Err(CoreError::Corrupt(message))
                if message == "allocation-root descriptor slot out of range"
        ));

        let zero_generation = value(RegionRecord {
            descriptor_slot: 0,
            free_blocks: 1,
            descriptor_generation: 0,
        });
        assert!(matches!(
            zero_generation,
            Err(CoreError::Corrupt(message))
                if message == "allocation-root descriptor generation is zero"
        ));
    }

    #[test]
    fn reserved_pool_keeps_three_checkpoint_generations_disjoint() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 64,
            region_size: 64,
        };
        let mut dev = MemoryBackend::new(4096, 64);
        let record1 = RegionRecord {
            descriptor_slot: 0,
            free_blocks: 40,
            descriptor_generation: 1,
        };
        dev.write_block(
            10,
            &initial_leaf(0, record1).unwrap().encode(4096, 1).unwrap(),
        )
        .unwrap();

        let record2 = RegionRecord {
            descriptor_slot: 1,
            free_blocks: 39,
            descriptor_generation: 2,
        };
        let key0 = key(0);
        let value2 = value(record2).unwrap();
        let mut pool = ReservedTreePool::new(10..=12, &[10], &[]).unwrap();
        let mutation2 = mutate_many(
            &mut dev,
            &geo,
            &mut pool,
            10,
            spec(1),
            2,
            &[TreeOperation::Upsert {
                key: &key0,
                value: &value2,
            }],
        )
        .unwrap();
        assert_eq!(mutation2.root_lba, 11);
        for (lba, block) in &mutation2.writes {
            dev.write_block(*lba, block).unwrap();
        }

        let record3 = RegionRecord {
            descriptor_slot: 2,
            free_blocks: 38,
            descriptor_generation: 3,
        };
        let value3 = value(record3).unwrap();
        let mut pool = ReservedTreePool::new(10..=12, &[11], &[10]).unwrap();
        let mutation3 = mutate_many(
            &mut dev,
            &geo,
            &mut pool,
            11,
            spec(2),
            3,
            &[TreeOperation::Upsert {
                key: &key0,
                value: &value3,
            }],
        )
        .unwrap();
        assert_eq!(mutation3.root_lba, 12);
        for (lba, block) in &mutation3.writes {
            dev.write_block(*lba, block).unwrap();
        }
        assert_eq!(
            lookup_record(&mut dev, &geo, 12, 3, 0).unwrap(),
            Some(record3)
        );
        let loaded = load_all(&mut dev, &geo, 12, 3).unwrap();
        assert_eq!(loaded.records, vec![record3]);
        assert_eq!(loaded.tree_blocks, vec![12]);
    }

    #[test]
    fn one_tib_geometry_bulk_builds_a_multi_node_allocation_root() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 1u64 << 28,
            region_size: 262_144,
        };
        geo.validate().unwrap();
        assert_eq!(geo.total_blocks * geo.block_size as u64, 1u64 << 40);
        assert_eq!(geo.region_count(), 1024);
        let records: Vec<_> = (0..geo.region_count())
            .map(|region| RegionRecord {
                descriptor_slot: 0,
                free_blocks: geo.region_valid_blocks(region),
                descriptor_generation: 1,
            })
            .collect();
        let built = bulk_build(&geo, &records).unwrap();
        assert!(built.nodes.len() > 1);
        assert_eq!(built.pool_lbas.len(), built.nodes.len() * 3);
        assert!(built
            .nodes
            .iter()
            .all(|(lba, _)| built.pool_lbas.contains(lba)));

        let mut dev = MemoryBackend::new(geo.block_size, geo.total_blocks);
        for (lba, node) in &built.nodes {
            dev.write_block(*lba, &node.encode(geo.block_size, 1).unwrap())
                .unwrap();
        }
        let loaded = load_all(&mut dev, &geo, built.root_lba, 1).unwrap();
        assert_eq!(loaded.records, records);
        assert_eq!(loaded.summary.items, 1024);
        assert!(loaded.summary.height >= 2);
    }
}
