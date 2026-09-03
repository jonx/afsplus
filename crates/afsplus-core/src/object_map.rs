//! Typed adapter for the shared COW object-map tree.

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::tree::{key_u64, TreeItem, TreeKind, TreeNode};

use crate::tree::{lookup, visit_tree_nodes, TreeSpec, TreeSummary};
use crate::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectMapEntry {
    pub object_id: u64,
    pub block: u64,
}

pub struct LoadedObjectMap {
    pub entries: Vec<ObjectMapEntry>,
    pub tree_blocks: Vec<u64>,
    pub summary: TreeSummary,
}

impl LoadedObjectMap {
    pub fn lookup(&self, object_id: u64) -> Option<u64> {
        self.entries
            .binary_search_by_key(&object_id, |entry| entry.object_id)
            .ok()
            .map(|index| self.entries[index].block)
    }
}

pub fn spec(max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::ObjectMap,
        owner: 0,
        max_generation,
    }
}

pub fn key(object_id: u64) -> [u8; 8] {
    key_u64(object_id)
}

pub fn value(block: u64) -> Result<[u8; 8], CoreError> {
    if block == 0 {
        return Err(CoreError::Corrupt(
            "object map references block zero".into(),
        ));
    }
    Ok(block.to_le_bytes())
}

pub fn initial_leaf(object_id: u64, block: u64) -> Result<TreeNode, CoreError> {
    Ok(TreeNode {
        kind: TreeKind::ObjectMap,
        owner: 0,
        level: 0,
        subtree_items: 1,
        leftmost_child: 0,
        leftmost_items: 0,
        items: vec![TreeItem {
            key: key(object_id).to_vec(),
            value: value(block)?.to_vec(),
        }],
    })
}

pub fn lookup_lba<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    max_generation: u64,
    object_id: u64,
) -> Result<Option<u64>, CoreError> {
    let (encoded, _) = lookup(dev, geo, root_lba, spec(max_generation), &key(object_id))?;
    encoded
        .map(|encoded| decode_value(&encoded, geo, object_id))
        .transpose()
}

/// Checker-only exhaustive load. Normal mount and object access use bounded
/// point lookups and never materialize the complete map.
pub fn load_all<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    max_generation: u64,
) -> Result<LoadedObjectMap, CoreError> {
    let mut entries = Vec::new();
    let mut tree_blocks = Vec::new();
    let summary = visit_tree_nodes(dev, geo, root_lba, spec(max_generation), |lba, node| {
        tree_blocks.push(lba);
        if node.is_leaf() {
            for item in &node.items {
                let object_id = decode_key(&item.key)?;
                entries.push(ObjectMapEntry {
                    object_id,
                    block: decode_value(&item.value, geo, object_id)?,
                });
            }
        }
        Ok(())
    })?;
    if entries.len() as u64 != summary.items {
        return Err(CoreError::Corrupt(
            "object map leaf count does not match tree summary".into(),
        ));
    }
    Ok(LoadedObjectMap {
        entries,
        tree_blocks,
        summary,
    })
}

fn decode_key(encoded: &[u8]) -> Result<u64, CoreError> {
    let bytes: [u8; 8] = encoded
        .try_into()
        .map_err(|_| CoreError::Corrupt("object map key is not eight bytes".into()))?;
    Ok(u64::from_be_bytes(bytes))
}

fn decode_value(encoded: &[u8], geo: &Geometry, object_id: u64) -> Result<u64, CoreError> {
    let bytes: [u8; 8] = encoded.try_into().map_err(|_| {
        CoreError::Corrupt(format!(
            "object map value for object {object_id} is not eight bytes"
        ))
    })?;
    let block = afsplus_format::le::get_u64(&bytes);
    if !geo.is_allocatable(block) {
        return Err(CoreError::Corrupt(format!(
            "object {object_id} record block {block} outside allocatable bounds"
        )));
    }
    Ok(block)
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::{Timespec, OBJECT_FIRST_DYNAMIC};

    use super::{key, load_all, lookup_lba, spec, value};
    use crate::alloc::TxAllocator;
    use crate::cow_tree::{mutate_many, TreeOperation};
    use crate::{mkfs, mount, MkfsParams};

    #[test]
    fn typed_object_map_grows_beyond_one_block_and_loads_exhaustively() {
        let mut dev = MemoryBackend::new(4096, 4096);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [93u8; 16],
                label: "ObjectMapTree".into(),
                region_size: 4096,
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

        let mut encoded = Vec::new();
        for offset in 0..1000u64 {
            let record_lba = tx.allocate(&mut dev).unwrap();
            encoded.push((
                key(OBJECT_FIRST_DYNAMIC + offset),
                value(record_lba).unwrap(),
            ));
        }
        let operations: Vec<_> = encoded
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let mutation = mutate_many(
            &mut dev,
            &geo,
            &mut tx,
            checkpoint.object_map_block,
            spec(1),
            2,
            &operations,
        )
        .unwrap();
        assert!(mutation.writes.len() > 1);
        for (lba, block) in &mutation.writes {
            dev.write_block(*lba, block).unwrap();
        }

        let loaded = load_all(&mut dev, &geo, mutation.root_lba, 2).unwrap();
        assert_eq!(loaded.entries.len(), 1001);
        assert!(loaded.summary.nodes > 1);
        let last_id = OBJECT_FIRST_DYNAMIC + 999;
        assert_eq!(
            lookup_lba(&mut dev, &geo, mutation.root_lba, 2, last_id).unwrap(),
            Some(u64::from_le_bytes(encoded[999].1))
        );
    }
}
