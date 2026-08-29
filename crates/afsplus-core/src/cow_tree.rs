//! Copy-on-write insertion and splitting for the shared bounded tree.

use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::tree::{
    child_value, ChildRef, TreeItem, TreeNode, MAX_TREE_KEY_BYTES, MAX_TREE_LEVEL,
};

use crate::alloc::TxAllocator;
use crate::tree::{check_tree_lba, validate_node_identity, validate_node_range, TreeSpec};
use crate::CoreError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeMutationStats {
    pub node_reads: u64,
    pub device_reads: u64,
    pub nodes_allocated: u64,
    pub committed_nodes_retired: u64,
    pub final_nodes_written: u64,
    pub splits: u64,
    pub root_splits: u64,
    pub max_depth: u8,
}

#[derive(Debug)]
pub struct TreeMutation {
    pub root_lba: u64,
    pub writes: Vec<(u64, Vec<u8>)>,
    pub stats: TreeMutationStats,
}

/// Applies multiple upserts under one COW overlay. Repeated changes to a node
/// allocated by this transaction update its staged image in place; committed
/// nodes are never overwritten. This correctness prototype retains the whole
/// dirty overlay in memory; the tiny-cache tranche must add spill/reload of
/// unreachable staged blocks without changing the mutation semantics.
#[allow(clippy::too_many_arguments)]
pub fn upsert_many<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    tx: &mut TxAllocator,
    root_lba: u64,
    spec: TreeSpec,
    new_generation: u64,
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<TreeMutation, CoreError> {
    if new_generation <= spec.max_generation {
        return Err(CoreError::Corrupt(
            "tree mutation generation is not newer than committed state".into(),
        ));
    }
    check_tree_lba(geo, root_lba)?;
    let mut context = MutationContext {
        dev,
        geo: *geo,
        tx,
        spec,
        new_generation,
        writes: BTreeMap::new(),
        stats: TreeMutationStats::default(),
    };
    let mut root = root_lba;
    for (key, value) in entries {
        validate_upsert(key, value)?;
        let replacement = context.upsert_node(root, None, None, None, None, true, 0, key, value)?;
        let old_level = replacement.level;
        root = if replacement.children.len() == 1 {
            replacement.children[0].reference.lba
        } else {
            if old_level == MAX_TREE_LEVEL {
                return Err(CoreError::PrototypeLimit("tree height limit reached"));
            }
            let left = replacement.children[0].reference;
            let right = &replacement.children[1];
            let root_node = TreeNode {
                kind: spec.kind,
                owner: spec.owner,
                level: old_level + 1,
                subtree_items: left
                    .subtree_items
                    .checked_add(right.reference.subtree_items)
                    .ok_or_else(|| CoreError::Corrupt("tree item count overflow".into()))?,
                leftmost_child: left.lba,
                leftmost_items: left.subtree_items,
                items: vec![TreeItem {
                    key: right.min_key.clone().ok_or_else(|| {
                        CoreError::Corrupt("split child has no minimum key".into())
                    })?,
                    value: child_value(right.reference).map_err(CoreError::Format)?,
                }],
            };
            let lba = context.tx.allocate(context.dev)?;
            context.stats.nodes_allocated += 1;
            context.stage_node(lba, &root_node)?;
            context.stats.root_splits += 1;
            lba
        };
    }
    context.stats.final_nodes_written = context.writes.len() as u64;
    Ok(TreeMutation {
        root_lba: root,
        writes: context.writes.into_iter().collect(),
        stats: context.stats,
    })
}

struct MutationContext<'a, D: BlockDevice> {
    dev: &'a mut D,
    geo: Geometry,
    tx: &'a mut TxAllocator,
    spec: TreeSpec,
    new_generation: u64,
    writes: BTreeMap<u64, Vec<u8>>,
    stats: TreeMutationStats,
}

#[derive(Clone)]
struct ChildDesc {
    /// None only for the leftmost child at the current node/root boundary.
    min_key: Option<Vec<u8>>,
    reference: ChildRef,
}

struct Replacement {
    children: Vec<ChildDesc>,
    level: u8,
}

impl<D: BlockDevice> MutationContext<'_, D> {
    #[allow(clippy::too_many_arguments)]
    fn upsert_node(
        &mut self,
        lba: u64,
        expected_level: Option<u8>,
        known_min: Option<Vec<u8>>,
        lower: Option<Vec<u8>>,
        upper: Option<Vec<u8>>,
        is_root: bool,
        depth: u8,
        key: &[u8],
        value: &[u8],
    ) -> Result<Replacement, CoreError> {
        if depth > MAX_TREE_LEVEL {
            return Err(CoreError::Corrupt(
                "tree mutation exceeded maximum depth".into(),
            ));
        }
        self.stats.max_depth = self.stats.max_depth.max(depth + 1);
        let (mut node, staged, _) = self.read_node(
            lba,
            expected_level,
            lower.as_deref(),
            upper.as_deref(),
            is_root,
            depth,
        )?;
        if node.is_leaf() {
            match node
                .items
                .binary_search_by(|item| item.key.as_slice().cmp(key))
            {
                Ok(index) => node.items[index].value = value.to_vec(),
                Err(index) => node.items.insert(
                    index,
                    TreeItem {
                        key: key.to_vec(),
                        value: value.to_vec(),
                    },
                ),
            }
            node.subtree_items = node.items.len() as u64;
            let min_key = node.items.first().map(|item| item.key.clone());
            if node.fits(self.geo.block_size) {
                return self.persist(lba, staged, vec![(node, min_key)]);
            }
            let (left, right) = split_leaf(node, self.geo.block_size)?;
            self.stats.splits += 1;
            let left_min = known_min.or_else(|| left.items.first().map(|item| item.key.clone()));
            let right_min = right.items.first().map(|item| item.key.clone());
            return self.persist(lba, staged, vec![(left, left_min), (right, right_min)]);
        }

        let child_index = node
            .items
            .partition_point(|item| item.key.as_slice() <= key);
        let mut children = children_from_node(&node)?;
        let child_lower = if child_index == 0 {
            lower.clone()
        } else {
            Some(node.items[child_index - 1].key.clone())
        };
        let child_upper = node
            .items
            .get(child_index)
            .map(|item| item.key.clone())
            .or_else(|| upper.clone());
        let child = children[child_index].clone();
        let replacement = self.upsert_node(
            child.reference.lba,
            Some(node.level - 1),
            child.min_key.clone(),
            child_lower,
            child_upper,
            false,
            depth + 1,
            key,
            value,
        )?;
        children.splice(child_index..=child_index, replacement.children);
        if child_index == 0 {
            children[0].min_key = None;
        }
        node = internal_from_children(node, &children)?;
        if node.fits(self.geo.block_size) {
            return self.persist(lba, staged, vec![(node, known_min)]);
        }
        let (left, right, right_min) = split_internal(node, &children, self.geo.block_size)?;
        self.stats.splits += 1;
        self.persist(
            lba,
            staged,
            vec![(left, known_min), (right, Some(right_min))],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn read_node(
        &mut self,
        lba: u64,
        expected_level: Option<u8>,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
        is_root: bool,
        depth: u8,
    ) -> Result<(TreeNode, bool, u64), CoreError> {
        if depth > MAX_TREE_LEVEL {
            return Err(CoreError::Corrupt(
                "tree mutation exceeded maximum depth".into(),
            ));
        }
        check_tree_lba(&self.geo, lba)?;
        let staged = self.writes.get(&lba).cloned();
        let (block, is_staged) = if let Some(block) = staged {
            (block, true)
        } else {
            let mut block = vec![0u8; self.geo.block_size];
            self.dev.read_block(lba, &mut block)?;
            self.stats.device_reads += 1;
            (block, false)
        };
        self.stats.node_reads += 1;
        self.stats.max_depth = self.stats.max_depth.max(depth + 1);
        let (node, generation) = TreeNode::decode(&block)
            .map_err(|error| CoreError::Corrupt(format!("tree node {lba}: {error}")))?;
        let validation_spec = TreeSpec {
            max_generation: if is_staged {
                self.new_generation
            } else {
                self.spec.max_generation
            },
            ..self.spec
        };
        validate_node_identity(&node, generation, validation_spec, expected_level, lba)?;
        if is_staged && generation != self.new_generation {
            return Err(CoreError::Corrupt(
                "staged tree node generation mismatch".into(),
            ));
        }
        validate_node_range(&node, lower, upper, is_root)?;
        Ok((node, is_staged, generation))
    }

    fn persist(
        &mut self,
        old_lba: u64,
        old_staged: bool,
        nodes: Vec<(TreeNode, Option<Vec<u8>>)>,
    ) -> Result<Replacement, CoreError> {
        let level = nodes
            .first()
            .ok_or_else(|| CoreError::Corrupt("empty tree replacement".into()))?
            .0
            .level;
        if nodes.iter().any(|(node, _)| node.level != level) {
            return Err(CoreError::Corrupt(
                "tree replacement mixes node levels".into(),
            ));
        }
        let mut lbas = Vec::with_capacity(nodes.len());
        if old_staged {
            lbas.push(old_lba);
        } else {
            self.tx.retire(self.dev, old_lba)?;
            self.stats.committed_nodes_retired += 1;
        }
        while lbas.len() < nodes.len() {
            lbas.push(self.tx.allocate(self.dev)?);
            self.stats.nodes_allocated += 1;
        }
        let mut children = Vec::with_capacity(nodes.len());
        for ((node, min_key), lba) in nodes.into_iter().zip(lbas) {
            let reference = ChildRef {
                lba,
                subtree_items: node.subtree_items,
            };
            self.stage_node(lba, &node)?;
            children.push(ChildDesc { min_key, reference });
        }
        Ok(Replacement { children, level })
    }

    fn stage_node(&mut self, lba: u64, node: &TreeNode) -> Result<(), CoreError> {
        let encoded = node
            .encode(self.geo.block_size, self.new_generation)
            .map_err(CoreError::Format)?;
        self.writes.insert(lba, encoded);
        Ok(())
    }
}

fn children_from_node(node: &TreeNode) -> Result<Vec<ChildDesc>, CoreError> {
    let mut children = Vec::with_capacity(node.items.len() + 1);
    children.push(ChildDesc {
        min_key: None,
        reference: ChildRef {
            lba: node.leftmost_child,
            subtree_items: node.leftmost_items,
        },
    });
    for item in &node.items {
        children.push(ChildDesc {
            min_key: Some(item.key.clone()),
            reference: TreeNode::child_ref(item).map_err(CoreError::Format)?,
        });
    }
    Ok(children)
}

fn internal_from_children(
    mut template: TreeNode,
    children: &[ChildDesc],
) -> Result<TreeNode, CoreError> {
    if children.len() < 2 {
        return Err(CoreError::Corrupt(
            "internal tree node has fewer than two children".into(),
        ));
    }
    template.leftmost_child = children[0].reference.lba;
    template.leftmost_items = children[0].reference.subtree_items;
    template.items.clear();
    let mut total = template.leftmost_items;
    for child in &children[1..] {
        total = total
            .checked_add(child.reference.subtree_items)
            .ok_or_else(|| CoreError::Corrupt("tree item count overflow".into()))?;
        template.items.push(TreeItem {
            key: child
                .min_key
                .clone()
                .ok_or_else(|| CoreError::Corrupt("non-leftmost child has no minimum".into()))?,
            value: child_value(child.reference).map_err(CoreError::Format)?,
        });
    }
    template.subtree_items = total;
    Ok(template)
}

fn split_leaf(node: TreeNode, block_size: usize) -> Result<(TreeNode, TreeNode), CoreError> {
    let mut best = None;
    for split in 1..node.items.len() {
        let mut left = node.clone();
        let mut right = node.clone();
        left.items = node.items[..split].to_vec();
        right.items = node.items[split..].to_vec();
        left.subtree_items = left.items.len() as u64;
        right.subtree_items = right.items.len() as u64;
        if left.fits(block_size) && right.fits(block_size) {
            let difference = left.encoded_len()?.abs_diff(right.encoded_len()?);
            if best
                .as_ref()
                .is_none_or(|(best_difference, _, _)| difference < *best_difference)
            {
                best = Some((difference, left, right));
            }
        }
    }
    best.map(|(_, left, right)| (left, right))
        .ok_or(CoreError::PrototypeLimit(
            "tree leaf item cannot be split to fit",
        ))
}

fn split_internal(
    node: TreeNode,
    children: &[ChildDesc],
    block_size: usize,
) -> Result<(TreeNode, TreeNode, Vec<u8>), CoreError> {
    let mut best = None;
    for split in 2..children.len().saturating_sub(1) {
        let left_children = &children[..split];
        let mut right_children = children[split..].to_vec();
        let promoted = right_children[0]
            .min_key
            .take()
            .ok_or_else(|| CoreError::Corrupt("internal split has no promoted key".into()))?;
        let left = internal_from_children(node.clone(), left_children)?;
        let right = internal_from_children(node.clone(), &right_children)?;
        if left.fits(block_size) && right.fits(block_size) {
            let difference = left.encoded_len()?.abs_diff(right.encoded_len()?);
            if best
                .as_ref()
                .is_none_or(|(best_difference, _, _, _)| difference < *best_difference)
            {
                best = Some((difference, left, right, promoted));
            }
        }
    }
    best.map(|(_, left, right, promoted)| (left, right, promoted))
        .ok_or(CoreError::PrototypeLimit(
            "internal tree node cannot be split to fit",
        ))
}

fn validate_upsert(key: &[u8], value: &[u8]) -> Result<(), CoreError> {
    if key.is_empty() || key.len() > MAX_TREE_KEY_BYTES {
        return Err(CoreError::Corrupt(
            "tree upsert key length out of range".into(),
        ));
    }
    if value.is_empty() {
        return Err(CoreError::Corrupt("tree upsert value is empty".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::tree::{child_value, key_u64, ChildRef, TreeItem, TreeKind, TreeNode};
    use afsplus_format::Timespec;

    use super::upsert_many;
    use crate::alloc::TxAllocator;
    use crate::tree::{lookup, validate_tree, TreeSpec};
    use crate::{mkfs, mount, MkfsParams};

    fn wide_key(value: u64) -> Vec<u8> {
        let mut key = key_u64(value).to_vec();
        key.resize(200, 0x5a);
        key
    }

    #[test]
    fn many_upserts_split_under_one_overlay_without_rewriting_committed_nodes() {
        let mut dev = MemoryBackend::new(4096, 2048);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [91u8; 16],
                label: "CowTree".into(),
                region_size: 2048,
                timestamp: Timespec::default(),
            },
        )
        .unwrap();
        let vol = mount(dev).unwrap();
        let geo = vol.ident().geometry();
        let checkpoint = vol.checkpoint().clone();
        let old_root = checkpoint.object_map_block;
        let retired = vol.retired().clone();
        let mut dev = vol.into_device();
        let empty = TreeNode::leaf(TreeKind::ObjectMap, 0)
            .encode(4096, 1)
            .unwrap();
        dev.write_block(old_root, &empty).unwrap();
        let mut tx = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, &retired, 2).unwrap();
        let mut entries: Vec<_> = (0..300u64)
            .map(|ordinal| {
                let key = (ordinal * 137) % 300;
                (wide_key(key), vec![key as u8; 80])
            })
            .collect();
        entries.push((wide_key(149), vec![0xee; 80]));
        let spec = TreeSpec {
            kind: TreeKind::ObjectMap,
            owner: 0,
            max_generation: 1,
        };
        let mutation = upsert_many(&mut dev, &geo, &mut tx, old_root, spec, 2, &entries).unwrap();
        assert!(mutation.stats.splits > 0);
        assert!(mutation.stats.root_splits >= 2);
        assert_eq!(mutation.stats.committed_nodes_retired, 1);
        assert_eq!(mutation.stats.device_reads, 1);
        for (lba, block) in &mutation.writes {
            dev.write_block(*lba, block).unwrap();
        }
        let new_spec = TreeSpec {
            max_generation: 2,
            ..spec
        };
        let summary = validate_tree(&mut dev, &geo, mutation.root_lba, new_spec).unwrap();
        assert_eq!(summary.items, 300);
        assert!(summary.height >= 3);
        for key in [0u64, 149, 299] {
            let value = lookup(&mut dev, &geo, mutation.root_lba, new_spec, &wide_key(key))
                .unwrap()
                .0
                .unwrap();
            let expected = if key == 149 {
                vec![0xee; 80]
            } else {
                vec![key as u8; 80]
            };
            assert_eq!(value, expected);
        }
        assert!(
            lookup(&mut dev, &geo, mutation.root_lba, new_spec, &wide_key(999))
                .unwrap()
                .0
                .is_none()
        );
        let finished = tx.finish(&checkpoint, None).unwrap();
        assert!(finished.stats.blocks_allocated >= mutation.stats.nodes_allocated);
    }

    #[test]
    fn mutation_rejects_a_child_at_the_parent_level() {
        let mut dev = MemoryBackend::new(4096, 2048);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [92u8; 16],
                label: "CowTreeBadLevel".into(),
                region_size: 2048,
                timestamp: Timespec::default(),
            },
        )
        .unwrap();
        let vol = mount(dev).unwrap();
        let geo = vol.ident().geometry();
        let checkpoint = vol.checkpoint().clone();
        let root = checkpoint.object_map_block;
        let retired = vol.retired().clone();
        let mut dev = vol.into_device();
        let corrupt_root = TreeNode {
            kind: TreeKind::ObjectMap,
            owner: 0,
            level: 1,
            subtree_items: 2,
            leftmost_child: root,
            leftmost_items: 1,
            items: vec![TreeItem {
                key: key_u64(100).to_vec(),
                value: child_value(ChildRef {
                    lba: root,
                    subtree_items: 1,
                })
                .unwrap(),
            }],
        }
        .encode(4096, 1)
        .unwrap();
        dev.write_block(root, &corrupt_root).unwrap();
        let mut tx = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, &retired, 2).unwrap();
        let error = upsert_many(
            &mut dev,
            &geo,
            &mut tx,
            root,
            TreeSpec {
                kind: TreeKind::ObjectMap,
                owner: 0,
                max_generation: 1,
            },
            2,
            &[(key_u64(0).to_vec(), vec![1])],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            crate::CoreError::Corrupt(message) if message.contains("level mismatch")
        ));
    }
}
