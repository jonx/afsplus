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
use afsplus_format::tree::{TreeItem, TreeKind, TreeNode};

use crate::cow_tree::TreeAllocator;
use crate::tree::{lookup, visit_tree_nodes, TreeSpec, TreeSummary};
use crate::CoreError;

const VALUE_BYTES: usize = 16;

pub struct LoadedAllocationRoot {
    pub records: Vec<RegionRecord>,
    pub tree_blocks: Vec<u64>,
    pub summary: TreeSummary,
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
    validate_record(record)?;
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
            key: key(region).to_vec(),
            value: value(record)?.to_vec(),
        }],
    })
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
    validate_record(record)?;
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

fn validate_record(record: RegionRecord) -> Result<(), CoreError> {
    if record.descriptor_slot >= DESCRIPTOR_SLOTS {
        return Err(CoreError::Corrupt(
            "allocation-root descriptor slot out of range".into(),
        ));
    }
    if record.descriptor_generation == 0 {
        return Err(CoreError::Corrupt(
            "allocation-root descriptor generation is zero".into(),
        ));
    }
    Ok(())
}

/// Allocator for a permanently allocated tree-node pool. Blocks reachable
/// from either retained checkpoint are excluded. A block allocated and then
/// discarded within the same uncommitted mutation may be reused immediately.
pub struct ReservedTreePool {
    pool: BTreeSet<u64>,
    available: BTreeSet<u64>,
    allocated_here: BTreeSet<u64>,
    retired_here: BTreeSet<u64>,
}

impl ReservedTreePool {
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
            pool,
            available,
            allocated_here: BTreeSet::new(),
            retired_here: BTreeSet::new(),
        })
    }
}

impl<D: BlockDevice> TreeAllocator<D> for ReservedTreePool {
    fn allocate_tree_block(&mut self, _dev: &mut D) -> Result<u64, CoreError> {
        let lba = self.available.pop_first().ok_or(CoreError::NoSpace)?;
        self.retired_here.remove(&lba);
        self.allocated_here.insert(lba);
        Ok(lba)
    }

    fn retire_tree_block(&mut self, _dev: &mut D, lba: u64) -> Result<(), CoreError> {
        if !self.pool.contains(&lba) || !self.retired_here.insert(lba) {
            return Err(CoreError::Corrupt(format!(
                "allocation-root pool block {lba} retired invalidly"
            )));
        }
        if self.allocated_here.remove(&lba) {
            self.available.insert(lba);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::checkpoint::RegionRecord;
    use afsplus_format::geometry::Geometry;

    use super::{initial_leaf, key, load_all, lookup_record, spec, value, ReservedTreePool};
    use crate::cow_tree::{mutate_many, TreeOperation};

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
}
