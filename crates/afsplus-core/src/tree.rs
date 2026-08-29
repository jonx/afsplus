//! Bounded reader and exhaustive verifier for shared COW tree nodes.

use std::collections::BTreeSet;

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::tree::{TreeKind, TreeNode, MAX_TREE_KEY_BYTES, MAX_TREE_LEVEL};

use crate::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeSpec {
    pub kind: TreeKind,
    pub owner: u64,
    pub max_generation: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeLookupStats {
    pub pages_read: u64,
    /// One raw block plus one decoded current node, regardless of tree height.
    pub peak_page_buffers: u8,
}

pub type TreeFloorItem = (Vec<u8>, Vec<u8>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeSummary {
    pub items: u64,
    pub nodes: u64,
    pub height: u8,
}

/// Looks up one binary key with a two-page-equivalent working set.
pub fn lookup<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    spec: TreeSpec,
    key: &[u8],
) -> Result<(Option<Vec<u8>>, TreeLookupStats), CoreError> {
    if key.is_empty() || key.len() > MAX_TREE_KEY_BYTES {
        return Err(CoreError::Corrupt(
            "tree lookup key length out of range".into(),
        ));
    }
    check_tree_lba(geo, root_lba)?;
    let mut lba = root_lba;
    let mut expected_level = None;
    let mut lower: Option<Vec<u8>> = None;
    let mut upper: Option<Vec<u8>> = None;
    let mut visited = BTreeSet::new();
    let mut buf = vec![0u8; geo.block_size];
    let mut stats = TreeLookupStats {
        pages_read: 0,
        peak_page_buffers: 2,
    };

    for depth in 0..=MAX_TREE_LEVEL {
        if !visited.insert(lba) {
            return Err(CoreError::Corrupt(format!("tree cycle at block {lba}")));
        }
        dev.read_block(lba, &mut buf)?;
        stats.pages_read += 1;
        let (node, generation) = TreeNode::decode(&buf)
            .map_err(|error| CoreError::Corrupt(format!("tree node {lba}: {error}")))?;
        validate_node_identity(&node, generation, spec, expected_level, lba)?;
        validate_node_range(&node, lower.as_deref(), upper.as_deref(), depth == 0)?;

        if node.is_leaf() {
            let value = node
                .items
                .binary_search_by(|item| item.key.as_slice().cmp(key))
                .ok()
                .map(|index| node.items[index].value.clone());
            return Ok((value, stats));
        }

        let separator_index = node
            .items
            .partition_point(|item| item.key.as_slice() <= key);
        let child = if separator_index == 0 {
            upper = node.items.first().map(|item| item.key.clone());
            afsplus_format::tree::ChildRef {
                lba: node.leftmost_child,
                subtree_items: node.leftmost_items,
            }
        } else {
            lower = Some(node.items[separator_index - 1].key.clone());
            if let Some(next_separator) = node.items.get(separator_index) {
                upper = Some(next_separator.key.clone());
            }
            TreeNode::child_ref(&node.items[separator_index - 1]).map_err(CoreError::Format)?
        };
        check_tree_lba(geo, child.lba)?;
        lba = child.lba;
        expected_level = Some(node.level - 1);
    }
    Err(CoreError::Corrupt(
        "tree traversal exceeded maximum depth".into(),
    ))
}

/// Finds the greatest stored key less than or equal to `key` with the same
/// bounded working set as [`lookup`]. Extent maps use this to find a mapping
/// that begins before the requested logical block.
pub fn lookup_floor<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    spec: TreeSpec,
    key: &[u8],
) -> Result<(Option<TreeFloorItem>, TreeLookupStats), CoreError> {
    if key.is_empty() || key.len() > MAX_TREE_KEY_BYTES {
        return Err(CoreError::Corrupt(
            "tree lookup key length out of range".into(),
        ));
    }
    check_tree_lba(geo, root_lba)?;
    let mut lba = root_lba;
    let mut expected_level = None;
    let mut lower: Option<Vec<u8>> = None;
    let mut upper: Option<Vec<u8>> = None;
    let mut visited = BTreeSet::new();
    let mut buf = vec![0u8; geo.block_size];
    let mut stats = TreeLookupStats {
        pages_read: 0,
        peak_page_buffers: 2,
    };

    for depth in 0..=MAX_TREE_LEVEL {
        if !visited.insert(lba) {
            return Err(CoreError::Corrupt(format!("tree cycle at block {lba}")));
        }
        dev.read_block(lba, &mut buf)?;
        stats.pages_read += 1;
        let (node, generation) = TreeNode::decode(&buf)
            .map_err(|error| CoreError::Corrupt(format!("tree node {lba}: {error}")))?;
        validate_node_identity(&node, generation, spec, expected_level, lba)?;
        validate_node_range(&node, lower.as_deref(), upper.as_deref(), depth == 0)?;

        if node.is_leaf() {
            let position = node
                .items
                .partition_point(|item| item.key.as_slice() <= key);
            let value = position.checked_sub(1).map(|index| {
                (
                    node.items[index].key.clone(),
                    node.items[index].value.clone(),
                )
            });
            return Ok((value, stats));
        }

        let separator_index = node
            .items
            .partition_point(|item| item.key.as_slice() <= key);
        let child = if separator_index == 0 {
            upper = node.items.first().map(|item| item.key.clone());
            afsplus_format::tree::ChildRef {
                lba: node.leftmost_child,
                subtree_items: node.leftmost_items,
            }
        } else {
            lower = Some(node.items[separator_index - 1].key.clone());
            if let Some(next_separator) = node.items.get(separator_index) {
                upper = Some(next_separator.key.clone());
            }
            TreeNode::child_ref(&node.items[separator_index - 1]).map_err(CoreError::Format)?
        };
        check_tree_lba(geo, child.lba)?;
        lba = child.lba;
        expected_level = Some(node.level - 1);
    }
    Err(CoreError::Corrupt(
        "tree traversal exceeded maximum depth".into(),
    ))
}

/// Exhaustively checks level, range, separator, count, ownership, and cycle
/// invariants. This belongs to the checker/shadow-verification path.
pub fn validate_tree<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    spec: TreeSpec,
) -> Result<TreeSummary, CoreError> {
    visit_tree_nodes(dev, geo, root_lba, spec, |_, _| Ok(()))
}

/// Exhaustively validates a tree and visits every valid node exactly once.
/// The checker uses this to decode typed leaf records and claim all tree
/// blocks without duplicating the structural verifier.
pub fn visit_tree_nodes<D, F>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    spec: TreeSpec,
    mut visitor: F,
) -> Result<TreeSummary, CoreError>
where
    D: BlockDevice,
    F: FnMut(u64, &TreeNode) -> Result<(), CoreError>,
{
    check_tree_lba(geo, root_lba)?;
    let mut visited = BTreeSet::new();
    let checked = validate_subtree(
        dev,
        geo,
        root_lba,
        spec,
        None,
        None,
        None,
        true,
        &mut visited,
        &mut visitor,
    )?;
    Ok(TreeSummary {
        items: checked.items,
        nodes: checked.nodes,
        height: checked.level + 1,
    })
}

struct CheckedSubtree {
    items: u64,
    nodes: u64,
    level: u8,
    min_key: Option<Vec<u8>>,
    max_key: Option<Vec<u8>>,
}

#[allow(clippy::too_many_arguments)]
fn validate_subtree<D, F>(
    dev: &mut D,
    geo: &Geometry,
    lba: u64,
    spec: TreeSpec,
    expected_level: Option<u8>,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    is_root: bool,
    visited: &mut BTreeSet<u64>,
    visitor: &mut F,
) -> Result<CheckedSubtree, CoreError>
where
    D: BlockDevice,
    F: FnMut(u64, &TreeNode) -> Result<(), CoreError>,
{
    if !visited.insert(lba) {
        return Err(CoreError::Corrupt(format!(
            "tree cycle or duplicate child at block {lba}"
        )));
    }
    let mut buf = vec![0u8; geo.block_size];
    dev.read_block(lba, &mut buf)?;
    let (node, generation) = TreeNode::decode(&buf)
        .map_err(|error| CoreError::Corrupt(format!("tree node {lba}: {error}")))?;
    validate_node_identity(&node, generation, spec, expected_level, lba)?;
    validate_node_range(&node, lower, upper, is_root)?;
    visitor(lba, &node)?;

    if node.is_leaf() {
        return Ok(CheckedSubtree {
            items: node.subtree_items,
            nodes: 1,
            level: 0,
            min_key: node.items.first().map(|item| item.key.clone()),
            max_key: node.items.last().map(|item| item.key.clone()),
        });
    }

    let mut total_items = 0u64;
    let mut total_nodes = 1u64;
    let mut tree_min = None;
    let mut tree_max = None;
    for child_index in 0..=node.items.len() {
        let child = if child_index == 0 {
            afsplus_format::tree::ChildRef {
                lba: node.leftmost_child,
                subtree_items: node.leftmost_items,
            }
        } else {
            TreeNode::child_ref(&node.items[child_index - 1]).map_err(CoreError::Format)?
        };
        check_tree_lba(geo, child.lba)?;
        let child_lower = if child_index == 0 {
            lower
        } else {
            Some(node.items[child_index - 1].key.as_slice())
        };
        let child_upper = node
            .items
            .get(child_index)
            .map(|item| item.key.as_slice())
            .or(upper);
        let checked = validate_subtree(
            dev,
            geo,
            child.lba,
            spec,
            Some(node.level - 1),
            child_lower,
            child_upper,
            false,
            visited,
            visitor,
        )?;
        if checked.items != child.subtree_items {
            return Err(CoreError::Corrupt(format!(
                "tree child {} count {}, parent records {}",
                child.lba, checked.items, child.subtree_items
            )));
        }
        if child_index > 0
            && checked.min_key.as_deref() != Some(node.items[child_index - 1].key.as_slice())
        {
            return Err(CoreError::Corrupt(format!(
                "tree separator does not equal minimum key of child {}",
                child.lba
            )));
        }
        if tree_min.is_none() {
            tree_min = checked.min_key.clone();
        }
        if checked.max_key.is_some() {
            tree_max = checked.max_key;
        }
        total_items = total_items
            .checked_add(checked.items)
            .ok_or_else(|| CoreError::Corrupt("tree item count overflow".into()))?;
        total_nodes = total_nodes
            .checked_add(checked.nodes)
            .ok_or_else(|| CoreError::Corrupt("tree node count overflow".into()))?;
    }
    if total_items != node.subtree_items {
        return Err(CoreError::Corrupt(format!(
            "tree node {lba} subtree count {}, children total {total_items}",
            node.subtree_items
        )));
    }
    Ok(CheckedSubtree {
        items: total_items,
        nodes: total_nodes,
        level: node.level,
        min_key: tree_min,
        max_key: tree_max,
    })
}

pub(crate) fn validate_node_identity(
    node: &TreeNode,
    generation: u64,
    spec: TreeSpec,
    expected_level: Option<u8>,
    lba: u64,
) -> Result<(), CoreError> {
    if node.kind != spec.kind || node.owner != spec.owner {
        return Err(CoreError::Corrupt(format!(
            "tree node {lba} kind/owner mismatch"
        )));
    }
    if generation == 0 || generation > spec.max_generation {
        return Err(CoreError::Corrupt(format!(
            "tree node {lba} generation {generation} outside committed range"
        )));
    }
    if expected_level.is_some_and(|level| level != node.level) {
        return Err(CoreError::Corrupt(format!(
            "tree node {lba} level mismatch"
        )));
    }
    Ok(())
}

pub(crate) fn validate_node_range(
    node: &TreeNode,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    is_root: bool,
) -> Result<(), CoreError> {
    if !is_root && node.items.is_empty() {
        return Err(CoreError::Corrupt("non-root tree node is empty".into()));
    }
    if let (Some(lower), Some(first)) = (lower, node.items.first()) {
        if first.key.as_slice() < lower {
            return Err(CoreError::Corrupt(
                "tree node key below parent range".into(),
            ));
        }
        if node.is_leaf() && first.key.as_slice() != lower {
            return Err(CoreError::Corrupt(
                "tree separator does not equal leaf minimum".into(),
            ));
        }
    }
    if let (Some(upper), Some(last)) = (upper, node.items.last()) {
        if last.key.as_slice() >= upper {
            return Err(CoreError::Corrupt(
                "tree node key above parent range".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn check_tree_lba(geo: &Geometry, lba: u64) -> Result<(), CoreError> {
    if !geo.is_allocatable(lba) {
        return Err(CoreError::Corrupt(format!(
            "tree block {lba} outside allocatable bounds"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::tree::{child_value, key_u64, ChildRef, TreeItem, TreeKind, TreeNode};

    use super::{lookup, lookup_floor, validate_tree, TreeSpec};
    use afsplus_format::geometry::Geometry;

    fn leaf(keys: &[u64]) -> TreeNode {
        TreeNode {
            kind: TreeKind::ObjectMap,
            owner: 0,
            level: 0,
            subtree_items: keys.len() as u64,
            leftmost_child: 0,
            leftmost_items: 0,
            items: keys
                .iter()
                .map(|key| TreeItem {
                    key: key_u64(*key).to_vec(),
                    value: (1000 + *key).to_le_bytes().to_vec(),
                })
                .collect(),
        }
    }

    #[test]
    fn lookup_and_full_validation_cross_an_internal_root() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 128,
            region_size: 128,
        };
        let mut dev = MemoryBackend::new(4096, 128);
        let left = leaf(&[1, 10, 49]);
        let right = leaf(&[50, 60]);
        dev.write_block(20, &left.encode(4096, 1).unwrap()).unwrap();
        dev.write_block(21, &right.encode(4096, 1).unwrap())
            .unwrap();
        let root = TreeNode {
            kind: TreeKind::ObjectMap,
            owner: 0,
            level: 1,
            subtree_items: 5,
            leftmost_child: 20,
            leftmost_items: 3,
            items: vec![TreeItem {
                key: key_u64(50).to_vec(),
                value: child_value(ChildRef {
                    lba: 21,
                    subtree_items: 2,
                })
                .unwrap(),
            }],
        };
        dev.write_block(22, &root.encode(4096, 1).unwrap()).unwrap();
        let spec = TreeSpec {
            kind: TreeKind::ObjectMap,
            owner: 0,
            max_generation: 1,
        };

        let (value, stats) = lookup(&mut dev, &geo, 22, spec, &key_u64(60)).unwrap();
        assert_eq!(u64::from_le_bytes(value.unwrap().try_into().unwrap()), 1060);
        assert_eq!(stats.pages_read, 2);
        assert_eq!(stats.peak_page_buffers, 2);
        assert!(lookup(&mut dev, &geo, 22, spec, &key_u64(40))
            .unwrap()
            .0
            .is_none());
        let (floor, _) = lookup_floor(&mut dev, &geo, 22, spec, &key_u64(55)).unwrap();
        let (floor_key, floor_value) = floor.unwrap();
        assert_eq!(u64::from_be_bytes(floor_key.try_into().unwrap()), 50);
        assert_eq!(u64::from_le_bytes(floor_value.try_into().unwrap()), 1050);
        assert!(lookup_floor(&mut dev, &geo, 22, spec, &key_u64(0))
            .unwrap()
            .0
            .is_none());
        assert_eq!(
            validate_tree(&mut dev, &geo, 22, spec).unwrap(),
            super::TreeSummary {
                items: 5,
                nodes: 3,
                height: 2
            }
        );

        let mut bad_root = root;
        bad_root.items[0].key = key_u64(49).to_vec();
        dev.write_block(22, &bad_root.encode(4096, 1).unwrap())
            .unwrap();
        assert!(validate_tree(&mut dev, &geo, 22, spec).is_err());
        assert!(lookup(&mut dev, &geo, 22, spec, &key_u64(60)).is_err());
    }
}
