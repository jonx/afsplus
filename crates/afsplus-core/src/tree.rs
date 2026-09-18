//! Bounded reader and exhaustive verifier for shared COW tree nodes.

use std::collections::BTreeSet;
use std::sync::Arc;

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::header::{block_type, BlockHeader};
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
    /// Peak raw/decoded page equivalents; point lookups use two.
    pub peak_page_buffers: u8,
}

pub type TreeFloorItem = (Vec<u8>, Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRangePage {
    pub items: Vec<(Vec<u8>, Vec<u8>)>,
    pub total_items: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeSummary {
    pub items: u64,
    pub nodes: u64,
    pub height: u8,
}

/// A tree node decoded from the bytes of block `lba`, which the device kept
/// beside them. A write of the block drops it.
struct KeptNode {
    node: Arc<TreeNode>,
    generation: u64,
}

/// Decodes tree node `lba` from `buf`, the bytes just read from it. A block
/// decoded here was checked when it was decoded; when the device kept that
/// decoded node, it is used as it is. With the `verify-cached-metadata`
/// feature the bytes are checked again first, at the cost of a CRC32C of
/// the block on every read.
pub(crate) fn decode_node<D: BlockDevice>(
    dev: &mut D,
    lba: u64,
    buf: &[u8],
) -> Result<(Arc<TreeNode>, u64), CoreError> {
    if let Some(kept) = dev
        .attached(lba)
        .and_then(|value| value.downcast::<KeptNode>().ok())
    {
        if cfg!(feature = "verify-cached-metadata") {
            BlockHeader::verify(buf, block_type::TREE_NODE)
                .map_err(|error| CoreError::Corrupt(format!("tree node {lba}: {error}")))?;
        }
        return Ok((Arc::clone(&kept.node), kept.generation));
    }
    let (node, generation) = TreeNode::decode(buf)
        .map_err(|error| CoreError::Corrupt(format!("tree node {lba}: {error}")))?;
    let node = Arc::new(node);
    dev.attach(
        lba,
        Arc::new(KeptNode {
            node: Arc::clone(&node),
            generation,
        }),
    );
    Ok((node, generation))
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
        let (node, generation) = decode_node(dev, lba, &buf)?;
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
        let (node, generation) = decode_node(dev, lba, &buf)?;
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

/// Reads a bounded ordinal range of leaf items. Subtree counters let the
/// traversal skip whole branches, so work is proportional to tree height
/// plus the pages containing returned items rather than directory size.
pub fn read_range<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    spec: TreeSpec,
    start: u64,
    limit: usize,
) -> Result<TreeRangePage, CoreError> {
    check_tree_lba(geo, root_lba)?;
    let mut items = Vec::with_capacity(limit);
    let mut path = BTreeSet::new();
    let total_items = read_range_node(
        dev, geo, root_lba, spec, None, None, None, true, start, limit, &mut path, &mut items,
    )?;
    Ok(TreeRangePage { items, total_items })
}

/// Reads at most `limit` entries starting at an inclusive binary key.
/// Persistent maintenance cursors use keys so deleting earlier entries does
/// not shift their position. Resume after the final returned key; explicitly
/// wrap to the first key to revisit records inserted behind the cursor.
/// Working memory is bounded by the tree height plus returned entries.
pub fn read_key_page<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    spec: TreeSpec,
    low: &[u8],
    limit: usize,
) -> Result<(TreeRangePage, TreeLookupStats), CoreError> {
    if low.is_empty() || low.len() > MAX_TREE_KEY_BYTES {
        return Err(CoreError::Corrupt(
            "tree page key length out of range".into(),
        ));
    }
    check_tree_lba(geo, root_lba)?;
    let mut items = Vec::new();
    let mut path = BTreeSet::new();
    let mut stats = TreeLookupStats::default();
    let total_items = read_key_page_node(
        dev, geo, root_lba, spec, None, None, None, low, limit, &mut path, &mut items, &mut stats,
    )?;
    Ok((TreeRangePage { items, total_items }, stats))
}

#[allow(clippy::too_many_arguments)]
fn read_key_page_node<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    lba: u64,
    spec: TreeSpec,
    expected_level: Option<u8>,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    low: &[u8],
    limit: usize,
    path: &mut BTreeSet<u64>,
    out: &mut Vec<TreeFloorItem>,
    stats: &mut TreeLookupStats,
) -> Result<u64, CoreError> {
    if !path.insert(lba) {
        return Err(CoreError::Corrupt(format!("tree cycle at block {lba}")));
    }
    let result = (|| {
        let mut buf = vec![0; geo.block_size];
        dev.read_block(lba, &mut buf)?;
        stats.pages_read += 1;
        stats.peak_page_buffers = stats.peak_page_buffers.max((path.len() * 2) as u8);
        let (node, generation) = decode_node(dev, lba, &buf)?;
        validate_node_identity(&node, generation, spec, expected_level, lba)?;
        validate_node_range(&node, lower, upper, expected_level.is_none())?;
        if out.len() >= limit {
            return Ok(node.subtree_items);
        }
        if node.is_leaf() {
            let start = node.items.partition_point(|item| item.key.as_slice() < low);
            out.extend(
                node.items[start..]
                    .iter()
                    .take(limit - out.len())
                    .map(|item| (item.key.clone(), item.value.clone())),
            );
            return Ok(node.subtree_items);
        }
        for index in 0..=node.items.len() {
            if out.len() >= limit {
                break;
            }
            let child_upper = node
                .items
                .get(index)
                .map(|item| item.key.as_slice())
                .or(upper);
            if child_upper.is_some_and(|bound| bound <= low) {
                continue;
            }
            let (child, child_lower) = if index == 0 {
                (
                    afsplus_format::tree::ChildRef {
                        lba: node.leftmost_child,
                        subtree_items: node.leftmost_items,
                    },
                    lower,
                )
            } else {
                (
                    TreeNode::child_ref(&node.items[index - 1]).map_err(CoreError::Format)?,
                    Some(node.items[index - 1].key.as_slice()),
                )
            };
            check_tree_lba(geo, child.lba)?;
            let actual = read_key_page_node(
                dev,
                geo,
                child.lba,
                spec,
                Some(node.level - 1),
                child_lower,
                child_upper,
                low,
                limit,
                path,
                out,
                stats,
            )?;
            if actual != child.subtree_items {
                return Err(CoreError::Corrupt(format!(
                    "tree child {} count {actual}, parent records {}",
                    child.lba, child.subtree_items
                )));
            }
        }
        Ok(node.subtree_items)
    })();
    path.remove(&lba);
    result
}

#[allow(clippy::too_many_arguments)]
fn read_range_node<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    lba: u64,
    spec: TreeSpec,
    expected_level: Option<u8>,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    is_root: bool,
    mut start: u64,
    limit: usize,
    path: &mut BTreeSet<u64>,
    out: &mut Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<u64, CoreError> {
    if !path.insert(lba) {
        return Err(CoreError::Corrupt(format!("tree cycle at block {lba}")));
    }
    let result = (|| {
        let mut buf = vec![0u8; geo.block_size];
        dev.read_block(lba, &mut buf)?;
        let (node, generation) = decode_node(dev, lba, &buf)?;
        validate_node_identity(&node, generation, spec, expected_level, lba)?;
        validate_node_range(&node, lower, upper, is_root)?;
        let total = node.subtree_items;
        if start >= total || out.len() >= limit {
            return Ok(total);
        }

        if node.is_leaf() {
            let first = usize::try_from(start)
                .map_err(|_| CoreError::Corrupt("leaf ordinal does not fit memory".into()))?;
            let take = limit.saturating_sub(out.len());
            out.extend(
                node.items[first..]
                    .iter()
                    .take(take)
                    .map(|item| (item.key.clone(), item.value.clone())),
            );
            return Ok(total);
        }

        for child_index in 0..=node.items.len() {
            if out.len() >= limit {
                break;
            }
            let child = if child_index == 0 {
                afsplus_format::tree::ChildRef {
                    lba: node.leftmost_child,
                    subtree_items: node.leftmost_items,
                }
            } else {
                TreeNode::child_ref(&node.items[child_index - 1]).map_err(CoreError::Format)?
            };
            if start >= child.subtree_items {
                start -= child.subtree_items;
                continue;
            }
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
            let actual = read_range_node(
                dev,
                geo,
                child.lba,
                spec,
                Some(node.level - 1),
                child_lower,
                child_upper,
                false,
                start,
                limit,
                path,
                out,
            )?;
            if actual != child.subtree_items {
                return Err(CoreError::Corrupt(format!(
                    "tree child {} count {actual}, parent records {}",
                    child.lba, child.subtree_items
                )));
            }
            start = 0;
        }
        Ok(total)
    })();
    path.remove(&lba);
    result
}

/// Visits every leaf item whose key lies in `[low, high]` (inclusive),
/// pruning whole subtrees by their separator windows: work is proportional
/// to tree height plus the leaves actually overlapping the range, with a
/// recursion stack bounded by tree height. Callers that need the record
/// *straddling* `low` pass the floor key from [`lookup_floor`] as `low`.
pub fn visit_key_range<D, F>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    spec: TreeSpec,
    low: &[u8],
    high: &[u8],
    mut visitor: F,
) -> Result<(), CoreError>
where
    D: BlockDevice,
    F: FnMut(&[u8], &[u8]) -> Result<(), CoreError>,
{
    if low.is_empty() || low.len() > MAX_TREE_KEY_BYTES || high.len() > MAX_TREE_KEY_BYTES {
        return Err(CoreError::Corrupt(
            "tree range key length out of range".into(),
        ));
    }
    check_tree_lba(geo, root_lba)?;
    let mut path = BTreeSet::new();
    visit_key_range_node(
        dev,
        geo,
        root_lba,
        spec,
        None,
        None,
        None,
        true,
        low,
        high,
        &mut path,
        &mut visitor,
    )
}

#[allow(clippy::too_many_arguments)]
fn visit_key_range_node<D, F>(
    dev: &mut D,
    geo: &Geometry,
    lba: u64,
    spec: TreeSpec,
    expected_level: Option<u8>,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    is_root: bool,
    low: &[u8],
    high: &[u8],
    path: &mut BTreeSet<u64>,
    visitor: &mut F,
) -> Result<(), CoreError>
where
    D: BlockDevice,
    F: FnMut(&[u8], &[u8]) -> Result<(), CoreError>,
{
    if !path.insert(lba) {
        return Err(CoreError::Corrupt(format!("tree cycle at block {lba}")));
    }
    let result = (|| {
        let mut buf = vec![0u8; geo.block_size];
        dev.read_block(lba, &mut buf)?;
        let (node, generation) = decode_node(dev, lba, &buf)?;
        validate_node_identity(&node, generation, spec, expected_level, lba)?;
        validate_node_range(&node, lower, upper, is_root)?;

        if node.is_leaf() {
            for item in &node.items {
                if item.key.as_slice() > high {
                    break;
                }
                if item.key.as_slice() >= low {
                    visitor(&item.key, &item.value)?;
                }
            }
            return Ok(());
        }

        for child_index in 0..=node.items.len() {
            // Child i holds keys in [separator(i-1), separator(i)); skip
            // subtrees entirely outside the requested window.
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
            if child_upper.is_some_and(|bound| bound <= low)
                || child_lower.is_some_and(|bound| bound > high)
            {
                continue;
            }
            let child = if child_index == 0 {
                afsplus_format::tree::ChildRef {
                    lba: node.leftmost_child,
                    subtree_items: node.leftmost_items,
                }
            } else {
                TreeNode::child_ref(&node.items[child_index - 1]).map_err(CoreError::Format)?
            };
            check_tree_lba(geo, child.lba)?;
            visit_key_range_node(
                dev,
                geo,
                child.lba,
                spec,
                Some(node.level - 1),
                child_lower,
                child_upper,
                false,
                low,
                high,
                path,
                visitor,
            )?;
        }
        Ok(())
    })();
    path.remove(&lba);
    result
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
        true,
    )?;
    Ok(TreeSummary {
        items: checked.items,
        nodes: checked.nodes,
        height: checked.level + 1,
    })
}

/// Validates and visits a tree while retaining only the current root-to-node
/// path in the cycle set. This gives streaming adapters O(height) traversal
/// bookkeeping. The exhaustive checker should use [`visit_tree_nodes`], whose
/// volume-wide set additionally diagnoses duplicate ownership directly.
pub fn visit_tree_nodes_bounded<D, F>(
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
    let mut path = BTreeSet::new();
    let checked = validate_subtree(
        dev,
        geo,
        root_lba,
        spec,
        None,
        None,
        None,
        true,
        &mut path,
        &mut visitor,
        false,
    )?;
    debug_assert!(path.is_empty());
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
    retain_visited: bool,
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
        let checked = CheckedSubtree {
            items: node.subtree_items,
            nodes: 1,
            level: 0,
            min_key: node.items.first().map(|item| item.key.clone()),
            max_key: node.items.last().map(|item| item.key.clone()),
        };
        if !retain_visited {
            visited.remove(&lba);
        }
        return Ok(checked);
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
            retain_visited,
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
    let checked = CheckedSubtree {
        items: total_items,
        nodes: total_nodes,
        level: node.level,
        min_key: tree_min,
        max_key: tree_max,
    };
    if !retain_visited {
        visited.remove(&lba);
    }
    Ok(checked)
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

    /// A device that keeps decoded forms and never drops them, and whose
    /// bytes a test may damage behind them.
    struct Keeps {
        inner: MemoryBackend,
        kept: std::collections::HashMap<u64, afsplus_block::Decoded>,
    }

    impl BlockDevice for Keeps {
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }
        fn total_blocks(&self) -> u64 {
            self.inner.total_blocks()
        }
        fn read_block(
            &mut self,
            lba: u64,
            buf: &mut [u8],
        ) -> Result<(), afsplus_block::BlockError> {
            self.inner.read_block(lba, buf)
        }
        fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), afsplus_block::BlockError> {
            self.inner.write_block(lba, data)
        }
        fn flush(&mut self) -> Result<(), afsplus_block::BlockError> {
            Ok(())
        }
        fn attached(&self, lba: u64) -> Option<afsplus_block::Decoded> {
            self.kept.get(&lba).cloned()
        }
        fn attach(&mut self, lba: u64, value: afsplus_block::Decoded) {
            self.kept.insert(lba, value);
        }
    }

    #[test]
    fn a_kept_node_answers_and_is_rechecked_only_when_asked() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 128,
            region_size: 128,
        };
        let mut dev = Keeps {
            inner: MemoryBackend::new(4096, 128),
            kept: Default::default(),
        };
        dev.write_block(20, &leaf(&[1, 2, 3]).encode(4096, 1).unwrap())
            .unwrap();
        let spec = TreeSpec {
            kind: TreeKind::ObjectMap,
            owner: 0,
            max_generation: 1,
        };
        let (value, _) = lookup(&mut dev, &geo, 20, spec, &key_u64(2)).unwrap();
        assert_eq!(u64::from_le_bytes(value.unwrap().try_into().unwrap()), 1002);
        assert!(
            dev.attached(20).is_some(),
            "the lookup kept the decoded node"
        );
        // The kept node answers again for the same bytes.
        let (value, _) = lookup(&mut dev, &geo, 20, spec, &key_u64(3)).unwrap();
        assert_eq!(u64::from_le_bytes(value.unwrap().try_into().unwrap()), 1003);
        // Damage the bytes behind the kept node. By default the kept node,
        // checked when it was decoded, still answers; with
        // verify-cached-metadata the read fails instead.
        let mut block = vec![0u8; 4096];
        dev.inner.read_block(20, &mut block).unwrap();
        block[100] ^= 0x40;
        dev.inner.write_block(20, &block).unwrap();
        let answer = lookup(&mut dev, &geo, 20, spec, &key_u64(2));
        if cfg!(feature = "verify-cached-metadata") {
            assert!(answer.is_err());
        } else {
            let (value, _) = answer.unwrap();
            assert_eq!(u64::from_le_bytes(value.unwrap().try_into().unwrap()), 1002);
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

    fn key_page_fixture() -> (MemoryBackend, Geometry, TreeSpec, Vec<u64>) {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 512,
            region_size: 512,
        };
        let mut dev = MemoryBackend::new(4096, 512);
        let keys: Vec<u64> = (1..=1024).map(|i| i * 3).collect();
        for (index, group) in keys.chunks(8).enumerate() {
            dev.write_block(20 + index as u64, &leaf(group).encode(4096, 1).unwrap())
                .unwrap();
        }
        for branch in 0..16 {
            let node = TreeNode {
                kind: TreeKind::ObjectMap,
                owner: 0,
                level: 1,
                subtree_items: 64,
                leftmost_child: 20 + branch * 8,
                leftmost_items: 8,
                items: (1..8)
                    .map(|index| TreeItem {
                        key: key_u64((branch * 64 + index * 8 + 1) * 3).to_vec(),
                        value: child_value(ChildRef {
                            lba: 20 + branch * 8 + index,
                            subtree_items: 8,
                        })
                        .unwrap(),
                    })
                    .collect(),
            };
            dev.write_block(200 + branch, &node.encode(4096, 1).unwrap())
                .unwrap();
        }
        let root = TreeNode {
            kind: TreeKind::ObjectMap,
            owner: 0,
            level: 2,
            subtree_items: 1024,
            leftmost_child: 200,
            leftmost_items: 64,
            items: (1..16)
                .map(|branch| TreeItem {
                    key: key_u64((branch * 64 + 1) * 3).to_vec(),
                    value: child_value(ChildRef {
                        lba: 200 + branch,
                        subtree_items: 64,
                    })
                    .unwrap(),
                })
                .collect(),
        };
        dev.write_block(216, &root.encode(4096, 1).unwrap())
            .unwrap();
        (
            dev,
            geo,
            TreeSpec {
                kind: TreeKind::ObjectMap,
                owner: 0,
                max_generation: 1,
            },
            keys,
        )
    }

    #[test]
    fn key_pages_match_scan_oracle_with_bounded_branch_reads() {
        use afsplus_block::TraceBackend;
        let (dev, geo, spec, keys) = key_page_fixture();
        let mut dev = TraceBackend::new(dev);
        for low in [0, 1, 3, 5, 24, 25, 192, 193, 2998, 3072, 3073] {
            for limit in [0, 1, 5, 20] {
                dev.reset();
                let (page, stats) =
                    super::read_key_page(&mut dev, &geo, 216, spec, &key_u64(low), limit).unwrap();
                let expected: Vec<_> = keys
                    .iter()
                    .copied()
                    .filter(|&key| key >= low)
                    .take(limit)
                    .collect();
                let actual: Vec<_> = page
                    .items
                    .iter()
                    .map(|(key, value)| {
                        let key = u64::from_be_bytes(key.as_slice().try_into().unwrap());
                        assert_eq!(
                            u64::from_le_bytes(value.as_slice().try_into().unwrap()),
                            key + 1000
                        );
                        key
                    })
                    .collect();
                assert_eq!(actual, expected, "low={low} limit={limit}");
                assert_eq!(page.total_items, 1024);
                assert_eq!(dev.stats().reads, stats.pages_read);
                assert!(stats.pages_read <= 6 + (limit as u64).div_ceil(8));
                assert!(stats.peak_page_buffers <= 6);
                assert_eq!(dev.stats().writes, 0);
                assert_eq!(dev.stats().flushes, 0);
            }
        }
        assert!(super::read_key_page(&mut dev, &geo, 216, spec, &[], 1).is_err());
    }

    #[test]
    fn key_cursor_survives_earlier_deletion_and_wrap_revisits_insertions() {
        let (mut dev, geo, mut spec, _) = key_page_fixture();
        let (first, _) = super::read_key_page(&mut dev, &geo, 216, spec, &key_u64(0), 5).unwrap();
        let last = u64::from_be_bytes(first.items.last().unwrap().0.as_slice().try_into().unwrap());
        assert_eq!(last, 15);
        // Delete five earlier keys and insert one behind the persisted cursor.
        dev.write_block(20, &leaf(&[2, 18, 21, 24]).encode(4096, 2).unwrap())
            .unwrap();
        let (mut branch, _) = TreeNode::decode(&dev.peek(200)).unwrap();
        branch.leftmost_items -= 4;
        branch.subtree_items -= 4;
        dev.write_block(200, &branch.encode(4096, 2).unwrap())
            .unwrap();
        let (mut root, _) = TreeNode::decode(&dev.peek(216)).unwrap();
        root.leftmost_items -= 4;
        root.subtree_items -= 4;
        dev.write_block(216, &root.encode(4096, 2).unwrap())
            .unwrap();
        spec.max_generation = 2;
        validate_tree(&mut dev, &geo, 216, spec).unwrap();
        let (next, _) =
            super::read_key_page(&mut dev, &geo, 216, spec, &key_u64(last + 1), 3).unwrap();
        assert_eq!(
            next.items
                .iter()
                .map(|item| u64::from_be_bytes(item.0.as_slice().try_into().unwrap()))
                .collect::<Vec<_>>(),
            [18, 21, 24]
        );
        let (wrapped, _) = super::read_key_page(&mut dev, &geo, 216, spec, &key_u64(0), 1).unwrap();
        assert_eq!(wrapped.items[0].0, key_u64(2));
    }

    #[test]
    fn key_pages_reject_corrupt_visited_counts_generations_and_cycles() {
        let (pristine, geo, spec, _) = key_page_fixture();
        for mode in 0..3 {
            let mut dev = pristine.clone();
            let (mut root, _) = TreeNode::decode(&dev.peek(216)).unwrap();
            if mode == 0 {
                root.leftmost_items += 1;
                root.subtree_items += 1;
            }
            if mode == 1 {
                root.leftmost_child = 216;
            }
            dev.write_block(
                216,
                &root.encode(4096, if mode == 2 { 2 } else { 1 }).unwrap(),
            )
            .unwrap();
            assert!(super::read_key_page(&mut dev, &geo, 216, spec, &key_u64(0), 1).is_err());
        }
    }
}
