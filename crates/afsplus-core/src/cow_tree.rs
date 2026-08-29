//! Copy-on-write insertion and splitting for the shared bounded tree.

use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::tree::{
    child_value, ChildRef, TreeItem, TreeNode, MAX_TREE_KEY_BYTES, MAX_TREE_LEVEL,
};

use crate::alloc::TxAllocator;
use crate::tree::{check_tree_lba, validate_node_identity, validate_node_range, TreeSpec};
use crate::CoreError;

/// Block lifecycle required by the shared COW engine. Ordinary trees use the
/// region transaction allocator; the allocation-root tree uses a permanently
/// reserved triple-version pool to avoid describing its own allocations.
pub trait TreeAllocator<D: BlockDevice> {
    fn allocate_tree_block(&mut self, dev: &mut D) -> Result<u64, CoreError>;
    fn retire_tree_block(&mut self, dev: &mut D, lba: u64) -> Result<(), CoreError>;
}

impl<D: BlockDevice> TreeAllocator<D> for TxAllocator {
    fn allocate_tree_block(&mut self, dev: &mut D) -> Result<u64, CoreError> {
        self.allocate(dev)
    }

    fn retire_tree_block(&mut self, dev: &mut D, lba: u64) -> Result<(), CoreError> {
        self.retire(dev, lba)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeMutationStats {
    pub node_reads: u64,
    pub device_reads: u64,
    pub nodes_allocated: u64,
    pub committed_nodes_retired: u64,
    pub final_nodes_written: u64,
    pub splits: u64,
    pub root_splits: u64,
    pub deletes: u64,
    pub merges: u64,
    pub redistributions: u64,
    pub root_collapses: u64,
    pub staged_nodes_discarded: u64,
    pub max_depth: u8,
}

#[derive(Debug)]
pub struct TreeMutation {
    pub root_lba: u64,
    pub writes: Vec<(u64, Vec<u8>)>,
    pub stats: TreeMutationStats,
}

/// One ordered mutation in a transaction-local tree overlay.
pub enum TreeOperation<'a> {
    Upsert { key: &'a [u8], value: &'a [u8] },
    Delete { key: &'a [u8] },
}

/// Applies multiple upserts under one COW overlay. Repeated changes to a node
/// allocated by this transaction update its staged image in place; committed
/// nodes are never overwritten. This correctness prototype retains the whole
/// dirty overlay in memory; the tiny-cache tranche must add spill/reload of
/// unreachable staged blocks without changing the mutation semantics. After
/// any error, the caller must abort and discard the surrounding allocator
/// transaction rather than reuse its partially prepared state.
#[allow(clippy::too_many_arguments)]
pub fn upsert_many<D, A>(
    dev: &mut D,
    geo: &Geometry,
    tx: &mut A,
    root_lba: u64,
    spec: TreeSpec,
    new_generation: u64,
    entries: &[(Vec<u8>, Vec<u8>)],
) -> Result<TreeMutation, CoreError>
where
    D: BlockDevice,
    A: TreeAllocator<D>,
{
    let operations: Vec<_> = entries
        .iter()
        .map(|(key, value)| TreeOperation::Upsert { key, value })
        .collect();
    mutate_many(dev, geo, tx, root_lba, spec, new_generation, &operations)
}

/// Deletes several keys under one COW overlay. Empty children are removed,
/// underfull siblings are merged or redistributed, and a one-child root is
/// collapsed. As with [`upsert_many`], dirty-node spill is a later tiny-cache
/// layer; committed blocks are never overwritten. An error requires aborting
/// and discarding the surrounding allocator transaction.
#[allow(clippy::too_many_arguments)]
pub fn delete_many<D, A>(
    dev: &mut D,
    geo: &Geometry,
    tx: &mut A,
    root_lba: u64,
    spec: TreeSpec,
    new_generation: u64,
    keys: &[Vec<u8>],
) -> Result<TreeMutation, CoreError>
where
    D: BlockDevice,
    A: TreeAllocator<D>,
{
    let operations: Vec<_> = keys
        .iter()
        .map(|key| TreeOperation::Delete { key })
        .collect();
    mutate_many(dev, geo, tx, root_lba, spec, new_generation, &operations)
}

/// Applies an ordered mix of upserts and deletes under one COW overlay.
#[allow(clippy::too_many_arguments)]
pub fn mutate_many<D, A>(
    dev: &mut D,
    geo: &Geometry,
    tx: &mut A,
    root_lba: u64,
    spec: TreeSpec,
    new_generation: u64,
    operations: &[TreeOperation<'_>],
) -> Result<TreeMutation, CoreError>
where
    D: BlockDevice,
    A: TreeAllocator<D>,
{
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
    for operation in operations {
        match operation {
            TreeOperation::Upsert { key, value } => {
                validate_upsert(key, value)?;
                let replacement =
                    context.upsert_node(root, None, None, None, None, true, 0, key, value)?;
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
                    let lba = context.tx.allocate_tree_block(context.dev)?;
                    context.stats.nodes_allocated += 1;
                    context.stage_node(lba, &root_node)?;
                    context.stats.root_splits += 1;
                    lba
                };
            }
            TreeOperation::Delete { key } => {
                validate_key(key)?;
                let pending = context.delete_node(root, None, None, None, None, true, 0, key)?;
                if pending.node.level > 0 && pending.node.items.is_empty() {
                    let child = pending.node.leftmost_child;
                    context.discard_pending(pending)?;
                    root = child;
                    context.stats.root_collapses += 1;
                } else {
                    let source = (pending.old_lba, pending.old_staged);
                    let output = (pending.node, pending.min_key);
                    let replacement = context.persist_sources(vec![source], vec![output])?;
                    root = replacement[0].reference.lba;
                }
                context.stats.deletes += 1;
            }
        }
    }
    context.stats.final_nodes_written = context.writes.len() as u64;
    Ok(TreeMutation {
        root_lba: root,
        writes: context.writes.into_iter().collect(),
        stats: context.stats,
    })
}

struct MutationContext<'a, D: BlockDevice, A: TreeAllocator<D>> {
    dev: &'a mut D,
    geo: Geometry,
    tx: &'a mut A,
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

struct PendingNode {
    old_lba: u64,
    old_staged: bool,
    node: TreeNode,
    /// Exact subtree minimum. It is absent only for an empty tree or where a
    /// root boundary does not need to expose the value to a parent.
    min_key: Option<Vec<u8>>,
}

type NodeImage = (TreeNode, Option<Vec<u8>>);

impl<D: BlockDevice, A: TreeAllocator<D>> MutationContext<'_, D, A> {
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
    fn delete_node(
        &mut self,
        lba: u64,
        expected_level: Option<u8>,
        known_min: Option<Vec<u8>>,
        lower: Option<Vec<u8>>,
        upper: Option<Vec<u8>>,
        is_root: bool,
        depth: u8,
        key: &[u8],
    ) -> Result<PendingNode, CoreError> {
        if depth > MAX_TREE_LEVEL {
            return Err(CoreError::Corrupt(
                "tree mutation exceeded maximum depth".into(),
            ));
        }
        let (mut node, staged, _) = self.read_node(
            lba,
            expected_level,
            lower.as_deref(),
            upper.as_deref(),
            is_root,
            depth,
        )?;
        if node.is_leaf() {
            let index = node
                .items
                .binary_search_by(|item| item.key.as_slice().cmp(key))
                .map_err(|_| CoreError::NotFound)?;
            node.items.remove(index);
            node.subtree_items = node.items.len() as u64;
            let min_key = node.items.first().map(|item| item.key.clone());
            return Ok(PendingNode {
                old_lba: lba,
                old_staged: staged,
                node,
                min_key,
            });
        }

        let child_index = node
            .items
            .partition_point(|item| item.key.as_slice() <= key);
        let mut children = children_from_node(&node)?;
        children[0].min_key = known_min.clone();
        let (child_lower, child_upper) = child_range(&node, child_index, &lower, &upper);
        let child = children[child_index].clone();
        let edited = self.delete_node(
            child.reference.lba,
            Some(node.level - 1),
            child.min_key,
            child_lower,
            child_upper,
            false,
            depth + 1,
            key,
        )?;

        if needs_rebalance(&edited.node, self.geo.block_size)? {
            let sibling_index = if child_index + 1 < children.len() {
                child_index + 1
            } else {
                child_index - 1
            };
            let (sibling_lower, sibling_upper) = child_range(&node, sibling_index, &lower, &upper);
            let sibling_desc = children[sibling_index].clone();
            let (sibling_node, sibling_staged, _) = self.read_node(
                sibling_desc.reference.lba,
                Some(node.level - 1),
                sibling_lower.as_deref(),
                sibling_upper.as_deref(),
                false,
                depth + 1,
            )?;
            let sibling = PendingNode {
                old_lba: sibling_desc.reference.lba,
                old_staged: sibling_staged,
                node: sibling_node,
                min_key: sibling_desc.min_key,
            };
            let (left_index, left, right) = if child_index < sibling_index {
                (child_index, edited, sibling)
            } else {
                (sibling_index, sibling, edited)
            };
            let outputs = rebalance_pair(&left, &right, self.geo.block_size)?;
            let output_count = outputs.len();
            let sources = vec![
                (left.old_lba, left.old_staged),
                (right.old_lba, right.old_staged),
            ];
            let replacement = self.persist_sources(sources, outputs)?;
            if output_count == 1 {
                self.stats.merges += 1;
            } else {
                self.stats.redistributions += 1;
            }
            children.splice(left_index..=left_index + 1, replacement);
        } else {
            let source = (edited.old_lba, edited.old_staged);
            let output = (edited.node, edited.min_key);
            let replacement = self.persist_sources(vec![source], vec![output])?;
            children.splice(child_index..=child_index, replacement);
        }

        let min_key = children[0].min_key.clone().or(known_min);
        node = internal_allow_one(node, &children)?;
        Ok(PendingNode {
            old_lba: lba,
            old_staged: staged,
            node,
            min_key,
        })
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
        nodes: Vec<NodeImage>,
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
            self.tx.retire_tree_block(self.dev, old_lba)?;
            self.stats.committed_nodes_retired += 1;
        }
        while lbas.len() < nodes.len() {
            lbas.push(self.tx.allocate_tree_block(self.dev)?);
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

    fn persist_sources(
        &mut self,
        sources: Vec<(u64, bool)>,
        outputs: Vec<NodeImage>,
    ) -> Result<Vec<ChildDesc>, CoreError> {
        if outputs.is_empty() {
            return Err(CoreError::Corrupt("empty tree replacement".into()));
        }
        let level = outputs[0].0.level;
        if outputs.iter().any(|(node, _)| node.level != level) {
            return Err(CoreError::Corrupt(
                "tree replacement mixes node levels".into(),
            ));
        }
        let mut reusable = Vec::new();
        for (lba, staged) in sources {
            if staged {
                if !self.writes.contains_key(&lba) {
                    return Err(CoreError::Corrupt(
                        "staged tree source has no write image".into(),
                    ));
                }
                reusable.push(lba);
            } else {
                self.tx.retire_tree_block(self.dev, lba)?;
                self.stats.committed_nodes_retired += 1;
            }
        }
        while reusable.len() < outputs.len() {
            reusable.push(self.tx.allocate_tree_block(self.dev)?);
            self.stats.nodes_allocated += 1;
        }
        while reusable.len() > outputs.len() {
            let lba = reusable.pop().expect("length checked above");
            self.writes.remove(&lba);
            self.tx.retire_tree_block(self.dev, lba)?;
            self.stats.staged_nodes_discarded += 1;
        }
        let mut replacement = Vec::with_capacity(outputs.len());
        for ((node, min_key), lba) in outputs.into_iter().zip(reusable) {
            let reference = ChildRef {
                lba,
                subtree_items: node.subtree_items,
            };
            self.stage_node(lba, &node)?;
            replacement.push(ChildDesc { min_key, reference });
        }
        Ok(replacement)
    }

    fn discard_pending(&mut self, pending: PendingNode) -> Result<(), CoreError> {
        if pending.old_staged {
            if self.writes.remove(&pending.old_lba).is_none() {
                return Err(CoreError::Corrupt(
                    "staged tree source has no write image".into(),
                ));
            }
            self.tx.retire_tree_block(self.dev, pending.old_lba)?;
            self.stats.staged_nodes_discarded += 1;
        } else {
            self.tx.retire_tree_block(self.dev, pending.old_lba)?;
            self.stats.committed_nodes_retired += 1;
        }
        Ok(())
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
    let mut seen = BTreeSet::new();
    if children
        .iter()
        .any(|child| !seen.insert(child.reference.lba))
    {
        return Err(CoreError::Corrupt(
            "internal tree node references a child more than once".into(),
        ));
    }
    Ok(children)
}

fn child_range(
    node: &TreeNode,
    child_index: usize,
    lower: &Option<Vec<u8>>,
    upper: &Option<Vec<u8>>,
) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
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
    (child_lower, child_upper)
}

fn needs_rebalance(node: &TreeNode, block_size: usize) -> Result<bool, CoreError> {
    let structurally_underfull = node.items.is_empty();
    let twice_payload = node
        .encoded_len()?
        .checked_mul(2)
        .ok_or_else(|| CoreError::Corrupt("tree node size overflow".into()))?;
    Ok(structurally_underfull || twice_payload <= block_size)
}

fn internal_allow_one(
    mut template: TreeNode,
    children: &[ChildDesc],
) -> Result<TreeNode, CoreError> {
    if children.is_empty() {
        return Err(CoreError::Corrupt(
            "internal tree node lost every child".into(),
        ));
    }
    if children.len() >= 2 {
        return internal_from_children(template, children);
    }
    template.leftmost_child = children[0].reference.lba;
    template.leftmost_items = children[0].reference.subtree_items;
    template.subtree_items = template.leftmost_items;
    template.items.clear();
    Ok(template)
}

fn rebalance_pair(
    left: &PendingNode,
    right: &PendingNode,
    block_size: usize,
) -> Result<Vec<NodeImage>, CoreError> {
    if left.node.level != right.node.level
        || left.node.kind != right.node.kind
        || left.node.owner != right.node.owner
    {
        return Err(CoreError::Corrupt(
            "tree rebalance siblings are incompatible".into(),
        ));
    }
    if left.node.is_leaf() {
        if left
            .node
            .items
            .last()
            .zip(right.node.items.first())
            .is_some_and(|(left_item, right_item)| left_item.key >= right_item.key)
        {
            return Err(CoreError::Corrupt("tree rebalance siblings overlap".into()));
        }
        let mut combined = left.node.clone();
        combined.items.extend(right.node.items.iter().cloned());
        combined.subtree_items = combined.items.len() as u64;
        let combined_min = combined.items.first().map(|item| item.key.clone());
        if combined.fits(block_size) {
            return Ok(vec![(combined, combined_min)]);
        }
        let (left_node, right_node) = split_leaf(combined, block_size)?;
        let left_min = left_node.items.first().map(|item| item.key.clone());
        let right_min = right_node.items.first().map(|item| item.key.clone());
        return Ok(vec![(left_node, left_min), (right_node, right_min)]);
    }

    let mut children = children_from_node(&left.node)?;
    children[0].min_key = left.min_key.clone();
    let mut right_children = children_from_node(&right.node)?;
    right_children[0].min_key = right.min_key.clone();
    if right_children[0].min_key.is_none() {
        return Err(CoreError::Corrupt(
            "right rebalance sibling has no minimum".into(),
        ));
    }
    children.extend(right_children);
    let combined_min = children[0].min_key.clone();
    let combined = internal_from_children(left.node.clone(), &children)?;
    if combined.fits(block_size) {
        return Ok(vec![(combined, combined_min)]);
    }
    let (left_node, right_node, right_min) = split_internal(combined, &children, block_size)?;
    Ok(vec![
        (left_node, combined_min),
        (right_node, Some(right_min)),
    ])
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
    validate_key(key)?;
    if value.is_empty() {
        return Err(CoreError::Corrupt("tree upsert value is empty".into()));
    }
    Ok(())
}

fn validate_key(key: &[u8]) -> Result<(), CoreError> {
    if key.is_empty() || key.len() > MAX_TREE_KEY_BYTES {
        return Err(CoreError::Corrupt(
            "tree mutation key length out of range".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::tree::{child_value, key_u64, ChildRef, TreeItem, TreeKind, TreeNode};
    use afsplus_format::Timespec;

    use super::{delete_many, mutate_many, upsert_many, TreeOperation};
    use crate::alloc::TxAllocator;
    use crate::tree::{lookup, validate_tree, TreeSpec};
    use crate::{mkfs, mount, MkfsParams};

    fn wide_key(value: u64) -> Vec<u8> {
        let mut key = key_u64(value).to_vec();
        key.resize(200, 0x5a);
        key
    }

    #[test]
    fn split_merge_and_height_changes_share_one_cow_engine() {
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
        for (lba, block) in &finished.bitmap_writes {
            dev.write_block(*lba, block).unwrap();
        }
        for (lba, block) in &finished.descriptor_writes {
            dev.write_block(*lba, block).unwrap();
        }
        let mut checkpoint2 = checkpoint.clone();
        checkpoint2.generation = 2;
        checkpoint2.object_map_block = mutation.root_lba;
        checkpoint2.regions = finished.records.clone();
        let mut tx2 = TxAllocator::begin(
            &mut dev,
            &geo,
            &checkpoint2,
            Some(&checkpoint),
            &finished.retired,
            3,
        )
        .unwrap();
        assert!(matches!(
            delete_many(
                &mut dev,
                &geo,
                &mut tx2,
                mutation.root_lba,
                new_spec,
                3,
                &[wide_key(999)],
            ),
            Err(crate::CoreError::NotFound)
        ));
        let delete_keys: Vec<_> = (0..299u64)
            .map(|ordinal| wide_key((ordinal * 137) % 299))
            .collect();
        let surviving_key = wide_key(299);
        let surviving_value = vec![0xab; 80];
        let mut operations = vec![TreeOperation::Upsert {
            key: &surviving_key,
            value: &surviving_value,
        }];
        operations.extend(delete_keys.iter().map(|key| TreeOperation::Delete { key }));
        let deletion = mutate_many(
            &mut dev,
            &geo,
            &mut tx2,
            mutation.root_lba,
            new_spec,
            3,
            &operations,
        )
        .unwrap();
        assert_eq!(deletion.stats.deletes, 299);
        assert!(deletion.stats.merges > 0);
        assert!(deletion.stats.root_collapses >= 2);
        for (lba, block) in &deletion.writes {
            dev.write_block(*lba, block).unwrap();
        }
        let final_spec = TreeSpec {
            max_generation: 3,
            ..spec
        };
        let summary = validate_tree(&mut dev, &geo, deletion.root_lba, final_spec).unwrap();
        assert_eq!(summary.items, 1);
        assert_eq!(summary.height, 1);
        assert_eq!(
            lookup(
                &mut dev,
                &geo,
                deletion.root_lba,
                final_spec,
                &surviving_key,
            )
            .unwrap()
            .0,
            Some(surviving_value)
        );
        tx2.finish(&checkpoint2, Some(&checkpoint)).unwrap();
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
                    lba: root + 1,
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
