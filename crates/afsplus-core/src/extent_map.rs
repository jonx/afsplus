//! Typed regular-file extent adapter for the shared COW tree.

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::le;
use afsplus_format::tree::{
    child_value, key_u64, ChildRef, TreeItem, TreeKind, TreeNode, MAX_TREE_LEVEL,
};

use crate::tree::{lookup_floor, visit_tree_nodes, TreeSpec, TreeSummary};
use crate::CoreError;

const VALUE_SIZE: usize = 24;

/// Allocated blocks whose contents are not yet part of the logical file read
/// as zeros until a later write replaces the extent or clears this flag.
pub const EXTENT_UNWRITTEN: u32 = 1 << 0;

/// Conservative marker: this extent's physical run *may* overlap shared
/// records, and every operation on it must resolve by overlap against the
/// reference tree (ADR-061). Set with no overlapping record is legal (the
/// run is private); clear over an existing record is corruption.
pub const EXTENT_SHARED: u32 = 1 << 1;
const KNOWN_FLAGS: u32 = EXTENT_UNWRITTEN | EXTENT_SHARED;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub logical_start: u64,
    pub physical_start: u64,
    pub block_count: u64,
    pub flags: u32,
}

impl Extent {
    pub fn logical_end(self) -> Result<u64, CoreError> {
        self.logical_start
            .checked_add(self.block_count)
            .ok_or_else(|| CoreError::Corrupt("extent logical end overflows".into()))
    }

    pub fn physical_end(self) -> Result<u64, CoreError> {
        self.physical_start
            .checked_add(self.block_count)
            .ok_or_else(|| CoreError::Corrupt("extent physical end overflows".into()))
    }
}

pub struct LoadedExtentMap {
    pub owner: u64,
    pub extents: Vec<Extent>,
    pub tree_blocks: Vec<u64>,
    pub summary: TreeSummary,
    pub allocated_blocks: u64,
}

pub struct BuiltExtentMap {
    pub root_lba: u64,
    pub nodes: Vec<(u64, TreeNode)>,
}

pub fn spec(owner: u64, max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::ExtentMap,
        owner,
        max_generation,
    }
}

pub fn empty_leaf(owner: u64) -> TreeNode {
    TreeNode::leaf(TreeKind::ExtentMap, owner)
}

/// Builds the initial single-node extent tree used when a direct file first
/// becomes sparse or fragmented. Later growth goes through the shared COW
/// tree engine and can split this leaf normally.
pub fn leaf_from_extents(owner: u64, extents: &[Extent]) -> Result<TreeNode, CoreError> {
    for pair in extents.windows(2) {
        if pair[0].logical_end()? > pair[1].logical_start {
            return Err(CoreError::Corrupt("logical extents overlap".into()));
        }
    }
    let items = extents
        .iter()
        .copied()
        .map(|extent| {
            let (key, value) = encode_extent(extent)?;
            Ok(TreeItem {
                key: key.to_vec(),
                value: value.to_vec(),
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    Ok(TreeNode {
        kind: TreeKind::ExtentMap,
        owner,
        level: 0,
        subtree_items: items.len() as u64,
        leftmost_child: 0,
        leftmost_items: 0,
        items,
    })
}

/// Returns the number of blocks needed for a balanced initial tree. This lets
/// the caller reserve every LBA transactionally before constructing child
/// references, without writing a temporary root to the device.
pub fn bulk_node_count(block_size: usize, item_count: usize) -> Result<usize, CoreError> {
    if item_count == 0 {
        return Ok(1);
    }
    let leaf_capacity = leaf_capacity(block_size)?;
    let fanout = internal_fanout(block_size)?;
    let mut level_nodes = item_count.div_ceil(leaf_capacity);
    let mut nodes = level_nodes;
    while level_nodes > 1 {
        level_nodes = level_nodes.div_ceil(fanout);
        nodes = nodes
            .checked_add(level_nodes)
            .ok_or_else(|| CoreError::Corrupt("extent tree node count overflows".into()))?;
    }
    Ok(nodes)
}

/// Builds a balanced initial tree using exactly the supplied transactionally
/// allocated LBAs. Subsequent edits use the generic shared COW engine.
pub fn bulk_build(
    owner: u64,
    block_size: usize,
    extents: &[Extent],
    lbas: &[u64],
) -> Result<BuiltExtentMap, CoreError> {
    let expected = bulk_node_count(block_size, extents.len())?;
    if lbas.len() != expected {
        return Err(CoreError::Corrupt(
            "extent bulk-build LBA count mismatch".into(),
        ));
    }
    if extents.is_empty() {
        return Ok(BuiltExtentMap {
            root_lba: lbas[0],
            nodes: vec![(lbas[0], empty_leaf(owner))],
        });
    }
    for pair in extents.windows(2) {
        if pair[0].logical_end()? > pair[1].logical_start {
            return Err(CoreError::Corrupt("logical extents overlap".into()));
        }
    }
    let leaf_capacity = leaf_capacity(block_size)?;
    let fanout = internal_fanout(block_size)?;
    let mut next_lba = lbas.iter().copied();
    let mut nodes = Vec::with_capacity(expected);
    let mut level_nodes = Vec::new();
    let mut offset = 0usize;
    for group_len in balanced_groups(extents.len(), leaf_capacity)? {
        let lba = next_lba
            .next()
            .ok_or_else(|| CoreError::Corrupt("extent bulk-build pool exhausted".into()))?;
        let node = leaf_from_extents(owner, &extents[offset..offset + group_len])?;
        level_nodes.push(BulkChild {
            lba,
            min_key: node.items[0].key.clone(),
            items: node.subtree_items,
        });
        nodes.push((lba, node));
        offset += group_len;
    }

    let mut level = 0u8;
    while level_nodes.len() > 1 {
        level = level
            .checked_add(1)
            .filter(|level| *level <= MAX_TREE_LEVEL)
            .ok_or(CoreError::PrototypeLimit(
                "extent tree height limit reached",
            ))?;
        let mut next_level = Vec::new();
        let mut child_offset = 0usize;
        for group_len in balanced_groups(level_nodes.len(), fanout)? {
            let children = &level_nodes[child_offset..child_offset + group_len];
            let lba = next_lba
                .next()
                .ok_or_else(|| CoreError::Corrupt("extent bulk-build pool exhausted".into()))?;
            let mut total = children[0].items;
            let mut items = Vec::with_capacity(children.len() - 1);
            for child in &children[1..] {
                total = total
                    .checked_add(child.items)
                    .ok_or_else(|| CoreError::Corrupt("extent item count overflows".into()))?;
                items.push(TreeItem {
                    key: child.min_key.clone(),
                    value: child_value(ChildRef {
                        lba: child.lba,
                        subtree_items: child.items,
                    })
                    .map_err(CoreError::Format)?,
                });
            }
            let node = TreeNode {
                kind: TreeKind::ExtentMap,
                owner,
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
            "extent bulk-build node count mismatch".into(),
        ));
    }
    Ok(BuiltExtentMap {
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

fn leaf_capacity(block_size: usize) -> Result<usize, CoreError> {
    let mut node = empty_leaf(1);
    let mut capacity = 0usize;
    loop {
        let (key, value) = encode_extent(Extent {
            logical_start: capacity as u64 * 2,
            physical_start: 1,
            block_count: 1,
            flags: 0,
        })?;
        node.items.push(TreeItem {
            key: key.to_vec(),
            value: value.to_vec(),
        });
        node.subtree_items = node.items.len() as u64;
        if !node.fits(block_size) {
            break;
        }
        capacity += 1;
    }
    if capacity == 0 {
        return Err(CoreError::UnsupportedGeometry(
            "block cannot hold one extent record",
        ));
    }
    Ok(capacity)
}

fn internal_fanout(block_size: usize) -> Result<usize, CoreError> {
    let mut node = TreeNode {
        kind: TreeKind::ExtentMap,
        owner: 1,
        level: 1,
        subtree_items: 1,
        leftmost_child: 1,
        leftmost_items: 1,
        items: Vec::new(),
    };
    let mut children = 1usize;
    loop {
        node.items.push(TreeItem {
            key: key_u64(children as u64).to_vec(),
            value: child_value(ChildRef {
                lba: children as u64 + 1,
                subtree_items: 1,
            })
            .map_err(CoreError::Format)?,
        });
        node.subtree_items += 1;
        if !node.fits(block_size) {
            break;
        }
        children += 1;
    }
    if children < 2 {
        return Err(CoreError::UnsupportedGeometry(
            "block cannot hold two extent-tree children",
        ));
    }
    Ok(children)
}

fn balanced_groups(count: usize, capacity: usize) -> Result<Vec<usize>, CoreError> {
    if count == 0 || capacity == 0 {
        return Err(CoreError::Corrupt("invalid extent bulk-build group".into()));
    }
    let group_count = count.div_ceil(capacity);
    let base = count / group_count;
    let remainder = count % group_count;
    Ok((0..group_count)
        .map(|index| base + usize::from(index < remainder))
        .collect())
}

/// Validates only the extent-tree root for bounded object access. Individual
/// mappings are checked on lookup; the checker uses [`load_all`].
pub fn validate_root<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    owner: u64,
    max_generation: u64,
) -> Result<(), CoreError> {
    crate::tree::check_tree_lba(geo, root_lba)?;
    let mut block = vec![0u8; geo.block_size];
    dev.read_block(root_lba, &mut block)?;
    let (node, generation) = TreeNode::decode(&block)
        .map_err(|error| CoreError::Corrupt(format!("extent root {root_lba}: {error}")))?;
    crate::tree::validate_node_identity(
        &node,
        generation,
        spec(owner, max_generation),
        None,
        root_lba,
    )?;
    crate::tree::validate_node_range(&node, None, None, true)
}

pub fn encode_extent(extent: Extent) -> Result<([u8; 8], [u8; VALUE_SIZE]), CoreError> {
    validate_extent_shape(extent)?;
    let mut value = [0u8; VALUE_SIZE];
    le::put_u64(&mut value[0..8], extent.physical_start);
    le::put_u64(&mut value[8..16], extent.block_count);
    le::put_u32(&mut value[16..20], extent.flags);
    Ok((key_u64(extent.logical_start), value))
}

/// Finds the extent containing `logical_block`, or `None` for a hole.
pub fn lookup_extent<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    owner: u64,
    max_generation: u64,
    logical_block: u64,
) -> Result<Option<Extent>, CoreError> {
    let lookup_key = key_u64(logical_block);
    let (item, _) = lookup_floor(dev, geo, root_lba, spec(owner, max_generation), &lookup_key)?;
    let Some((key, value)) = item else {
        return Ok(None);
    };
    let extent = decode_extent(&key, &value, geo)?;
    Ok((logical_block < extent.logical_end()?).then_some(extent))
}

pub fn load_all<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    owner: u64,
    max_generation: u64,
) -> Result<LoadedExtentMap, CoreError> {
    let mut extents = Vec::new();
    let mut tree_blocks = Vec::new();
    let summary = visit_tree_nodes(
        dev,
        geo,
        root_lba,
        spec(owner, max_generation),
        |lba, node| {
            tree_blocks.push(lba);
            if node.is_leaf() {
                for item in &node.items {
                    extents.push(decode_extent(&item.key, &item.value, geo)?);
                }
            }
            Ok(())
        },
    )?;
    if extents.len() as u64 != summary.items {
        return Err(CoreError::Corrupt(
            "extent leaf count does not match tree summary".into(),
        ));
    }
    for pair in extents.windows(2) {
        if pair[0].logical_end()? > pair[1].logical_start {
            return Err(CoreError::Corrupt("logical extents overlap".into()));
        }
    }
    let allocated_blocks = extents.iter().try_fold(0u64, |total, extent| {
        total
            .checked_add(extent.block_count)
            .ok_or_else(|| CoreError::Corrupt("extent allocated-block count overflows".into()))
    })?;
    Ok(LoadedExtentMap {
        owner,
        extents,
        tree_blocks,
        summary,
        allocated_blocks,
    })
}

fn decode_extent(key: &[u8], value: &[u8], geo: &Geometry) -> Result<Extent, CoreError> {
    let key: [u8; 8] = key
        .try_into()
        .map_err(|_| CoreError::Corrupt("extent key is not eight bytes".into()))?;
    if value.len() != VALUE_SIZE {
        return Err(CoreError::Corrupt(
            "extent value is not twenty-four bytes".into(),
        ));
    }
    if value[20..24] != [0; 4] {
        return Err(CoreError::Corrupt(
            "extent reserved bytes are nonzero".into(),
        ));
    }
    let extent = Extent {
        logical_start: u64::from_be_bytes(key),
        physical_start: le::get_u64(&value[0..8]),
        block_count: le::get_u64(&value[8..16]),
        flags: le::get_u32(&value[16..20]),
    };
    validate_extent_shape(extent)?;
    validate_physical_range(extent, geo)?;
    Ok(extent)
}

fn validate_extent_shape(extent: Extent) -> Result<(), CoreError> {
    if extent.block_count == 0 {
        return Err(CoreError::Corrupt("zero-length extent".into()));
    }
    if extent.flags & !KNOWN_FLAGS != 0 {
        return Err(CoreError::Corrupt("extent has unsupported flags".into()));
    }
    extent.logical_end()?;
    extent.physical_end()?;
    Ok(())
}

fn validate_physical_range(extent: Extent, geo: &Geometry) -> Result<(), CoreError> {
    let end = extent.physical_end()?;
    if end > geo.total_blocks
        || !geo.is_allocatable(extent.physical_start)
        || !geo.is_allocatable(end - 1)
        || geo.region_of(extent.physical_start) != geo.region_of(end - 1)
    {
        return Err(CoreError::Corrupt(format!(
            "extent physical range {}..{end} is not allocatable",
            extent.physical_start
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::geometry::Geometry;

    use super::{
        bulk_build, bulk_node_count, empty_leaf, encode_extent, load_all, lookup_extent, spec,
        Extent,
    };
    use crate::allocation_root::ReservedTreePool;
    use crate::cow_tree::{mutate_many, TreeOperation};

    #[test]
    fn extent_adapter_crosses_pages_and_finds_containing_ranges() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 8192,
            region_size: 8192,
        };
        let mut dev = MemoryBackend::new(4096, 8192);
        dev.write_block(100, &empty_leaf(17).encode(4096, 1).unwrap())
            .unwrap();
        let extents: Vec<_> = (0..600u64)
            .map(|index| Extent {
                logical_start: index * 3,
                physical_start: 1000 + index * 2,
                block_count: 2,
                flags: 0,
            })
            .collect();
        let encoded: Vec<_> = extents
            .iter()
            .copied()
            .map(encode_extent)
            .collect::<Result<_, _>>()
            .unwrap();
        let operations: Vec<_> = encoded
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let mut pool = ReservedTreePool::new(100..500, &[100], &[]).unwrap();
        let mutation =
            mutate_many(&mut dev, &geo, &mut pool, 100, spec(17, 1), 2, &operations).unwrap();
        for (lba, block) in &mutation.writes {
            dev.write_block(*lba, block).unwrap();
        }

        let loaded = load_all(&mut dev, &geo, mutation.root_lba, 17, 2).unwrap();
        assert_eq!(loaded.extents, extents);
        assert_eq!(loaded.allocated_blocks, 1200);
        assert!(loaded.summary.nodes > 1);
        assert_eq!(
            lookup_extent(&mut dev, &geo, mutation.root_lba, 17, 2, 3 * 599 + 1)
                .unwrap()
                .unwrap(),
            extents[599]
        );
        assert!(
            lookup_extent(&mut dev, &geo, mutation.root_lba, 17, 2, 3 * 599 + 2)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn initial_bulk_build_is_not_limited_to_one_leaf() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 8192,
            region_size: 8192,
        };
        let mut dev = MemoryBackend::new(4096, 8192);
        let extents: Vec<_> = (0..600u64)
            .map(|index| Extent {
                logical_start: index * 2,
                physical_start: 2000 + index,
                block_count: 1,
                flags: 0,
            })
            .collect();
        let count = bulk_node_count(4096, extents.len()).unwrap();
        assert!(count > 1);
        let lbas: Vec<_> = (100..100 + count as u64).collect();
        let built = bulk_build(23, 4096, &extents, &lbas).unwrap();
        for (lba, node) in built.nodes {
            dev.write_block(lba, &node.encode(4096, 2).unwrap())
                .unwrap();
        }
        let loaded = load_all(&mut dev, &geo, built.root_lba, 23, 2).unwrap();
        assert_eq!(loaded.extents, extents);
        assert_eq!(loaded.summary.nodes as usize, count);
    }
}
