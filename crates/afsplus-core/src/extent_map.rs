//! Typed regular-file extent adapter for the shared COW tree.

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::le;
use afsplus_format::tree::{key_u64, TreeKind, TreeNode};

use crate::tree::{lookup_floor, visit_tree_nodes, TreeSpec, TreeSummary};
use crate::CoreError;

const VALUE_SIZE: usize = 24;

/// Allocated blocks whose contents are not yet part of the logical file read
/// as zeros until a later write replaces the extent or clears this flag.
pub const EXTENT_UNWRITTEN: u32 = 1 << 0;
const KNOWN_FLAGS: u32 = EXTENT_UNWRITTEN;

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

    use super::{empty_leaf, encode_extent, load_all, lookup_extent, spec, Extent};
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
}
