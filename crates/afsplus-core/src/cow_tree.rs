//! Copy-on-write insertion and splitting for the shared bounded tree.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::small_bytes::SmallBytes;
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
    /// Optional synchronous observation, with the same no-allocation/no-I/O contract as the recorder.
    fn observe_tree(
        &mut self,
        _generation: u64,
        _kind: crate::flight::EventKind,
        _context: crate::flight::TreeContext,
    ) {
    }

    /// Runtime staged-image budget for each tree mutation in this transaction.
    fn tree_cache_pages(&self) -> usize {
        usize::MAX
    }
    /// Successful mutation accounting, including provisional spill I/O.
    fn record_tree_mutation(&mut self, _stats: TreeMutationStats) {}
    fn allocate_tree_block(&mut self, dev: &mut D) -> Result<u64, CoreError>;
    /// Quarantines a node the committed state reaches.
    fn retire_tree_block(&mut self, dev: &mut D, lba: u64) -> Result<(), CoreError>;
    /// Frees a node this transaction allocated and then discarded — no
    /// committed state can reference it, so it skips quarantine entirely
    /// and may be reused immediately.
    fn release_tree_block(&mut self, dev: &mut D, lba: u64) -> Result<(), CoreError>;
}

impl<D: BlockDevice> TreeAllocator<D> for TxAllocator {
    fn observe_tree(
        &mut self,
        generation: u64,
        kind: crate::flight::EventKind,
        context: crate::flight::TreeContext,
    ) {
        TxAllocator::observe_tree(self, generation, kind, context);
    }

    fn tree_cache_pages(&self) -> usize {
        self.tree_cache_pages.get()
    }
    fn record_tree_mutation(&mut self, stats: TreeMutationStats) {
        self.tree_mutations.include(stats);
    }
    fn allocate_tree_block(&mut self, dev: &mut D) -> Result<u64, CoreError> {
        self.allocate(dev)
    }

    fn retire_tree_block(&mut self, dev: &mut D, lba: u64) -> Result<(), CoreError> {
        self.retire(dev, lba)
    }

    fn release_tree_block(&mut self, dev: &mut D, lba: u64) -> Result<(), CoreError> {
        self.release_uncommitted(dev, lba)
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
    /// Provisional, unreachable node images written early to enforce a
    /// constrained mutation-cache budget.
    pub staged_spill_writes: u64,
    /// Provisional images reloaded while the same transaction continues.
    pub staged_spill_reloads: u64,
    /// Maximum resident staged entries after enforcing the cache limit.
    pub max_resident_staged_nodes: u64,
    /// Images encoded because their bytes were wanted, rather than as they
    /// were staged. Zero where the decoded cache is switched off.
    pub deferred_encodes: u64,
    /// Resident staged entries before eviction, including the admitted image.
    /// Decoding/encoding temporaries and caller-held mutation results are separate.
    pub max_staged_nodes_before_eviction: u64,
    /// Maximum decoded or derived full nodes alive at once. Compact descent
    /// frames and child descriptors are not full-page equivalents.
    pub max_live_decoded_nodes: u64,
    pub max_depth: u8,
}

impl TreeMutationStats {
    /// Sum work across mutations; residency/depth fields retain their maximum.
    pub fn include(&mut self, other: Self) {
        self.node_reads += other.node_reads;
        self.device_reads += other.device_reads;
        self.nodes_allocated += other.nodes_allocated;
        self.committed_nodes_retired += other.committed_nodes_retired;
        self.final_nodes_written += other.final_nodes_written;
        self.splits += other.splits;
        self.root_splits += other.root_splits;
        self.deletes += other.deletes;
        self.merges += other.merges;
        self.redistributions += other.redistributions;
        self.root_collapses += other.root_collapses;
        self.staged_nodes_discarded += other.staged_nodes_discarded;
        self.staged_spill_writes += other.staged_spill_writes;
        self.staged_spill_reloads += other.staged_spill_reloads;
        self.max_resident_staged_nodes = self
            .max_resident_staged_nodes
            .max(other.max_resident_staged_nodes);
        self.deferred_encodes += other.deferred_encodes;
        self.max_staged_nodes_before_eviction = self
            .max_staged_nodes_before_eviction
            .max(other.max_staged_nodes_before_eviction);
        self.max_live_decoded_nodes = self
            .max_live_decoded_nodes
            .max(other.max_live_decoded_nodes);
        self.max_depth = self.max_depth.max(other.max_depth);
    }
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
/// nodes are never overwritten. This default entry point keeps the complete
/// dirty overlay in memory when the allocator uses its default budget;
/// constrained allocator profiles use provisional spill/reload. After any
/// error, the caller must abort and discard the surrounding allocator
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
/// collapsed. As with [`upsert_many`], the allocator's cache policy controls
/// dirty-node spill; committed blocks are never overwritten. An error requires aborting
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
    let cache_pages = tx.tree_cache_pages();
    mutate_many_with_cache_limit(
        dev,
        geo,
        tx,
        root_lba,
        spec,
        new_generation,
        operations,
        cache_pages,
    )
}

/// Applies a mixed mutation while retaining at most `cache_pages` final
/// staged node images in RAM. Evicted images are written only to freshly
/// allocated, still-unreachable blocks and reloaded on demand. The caller's
/// later metadata barrier makes those provisional writes durable before
/// checkpoint publication; aborting leaves only harmless garbage in blocks
/// that the committed allocator still considers free.
#[allow(clippy::too_many_arguments)]
pub fn mutate_many_with_cache_limit<D, A>(
    dev: &mut D,
    geo: &Geometry,
    tx: &mut A,
    root_lba: u64,
    spec: TreeSpec,
    new_generation: u64,
    operations: &[TreeOperation<'_>],
    cache_pages: usize,
) -> Result<TreeMutation, CoreError>
where
    D: BlockDevice,
    A: TreeAllocator<D>,
{
    mutate_many_inner(
        dev,
        geo,
        tx,
        root_lba,
        spec,
        new_generation,
        operations,
        cache_pages,
        false,
    )
}

/// Applies operations to a transactionally allocated empty root. Unlike a
/// committed root, this block is staged in the overlay immediately, so the
/// first edit may reuse or split it without either reading it from the device
/// or attempting to quarantine an allocation from the current transaction.
/// This is used when a metadata tree and its first entries must become visible
/// in the same checkpoint.
#[allow(clippy::too_many_arguments)]
pub fn mutate_new_empty_tree<D, A>(
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
    let cache_pages = tx.tree_cache_pages();
    mutate_many_inner(
        dev,
        geo,
        tx,
        root_lba,
        spec,
        new_generation,
        operations,
        cache_pages,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn mutate_many_inner<D, A>(
    dev: &mut D,
    geo: &Geometry,
    tx: &mut A,
    root_lba: u64,
    spec: TreeSpec,
    new_generation: u64,
    operations: &[TreeOperation<'_>],
    cache_pages: usize,
    new_empty_root: bool,
) -> Result<TreeMutation, CoreError>
where
    D: BlockDevice,
    A: TreeAllocator<D>,
{
    let _allocation_scope = crate::allocation_trace::enter(crate::allocation_trace::Domain::Tree);

    if cache_pages == 0 {
        return Err(CoreError::PrototypeLimit(
            "tree mutation cache must retain at least one page",
        ));
    }
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
        cache_pages,
        resident_staged_nodes: 0,
        access_clock: 0,
        resident_lru: BTreeSet::new(),
        decoded_residency: Rc::new(NodeResidency::default()),
        decoded: BTreeMap::new(),
        decoded_lru: BTreeSet::new(),
        decoded_limit: (cache_pages / 16).min(DECODED_CACHE_NODES),
        stats: TreeMutationStats::default(),
    };
    if new_empty_root {
        let node = context.track_node(TreeNode::leaf(spec.kind, spec.owner));
        context.stage_node(root_lba, node)?;
        context.stats.nodes_allocated = 1;
    }
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
                    let root_node = context.track_node(TreeNode {
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
                            value: child_value(right.reference)
                                .map_err(CoreError::Format)?
                                .into(),
                        }],
                    });
                    let lba = context.allocate_block()?;
                    context.stats.nodes_allocated += 1;
                    context.stage_node(lba, root_node)?;
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
    // Everything still deferred is encoded now, while its decoded node is
    // still here: the caller is handed bytes, as it always was.
    let deferred: Vec<u64> = context
        .writes
        .iter()
        .filter(|(_, image)| matches!(image.resident, Resident::Deferred))
        .map(|(lba, _)| *lba)
        .collect();
    for lba in deferred {
        context.materialize(lba)?;
    }
    context.stats.max_live_decoded_nodes = context.decoded_residency.peak.get();
    context.decoded.clear();
    context.decoded_lru.clear();
    debug_assert_eq!(context.decoded_residency.live.get(), 0);
    context.tx.record_tree_mutation(context.stats);
    let writes = context
        .writes
        .into_iter()
        .filter_map(|(lba, image)| match image.resident {
            Resident::Bytes(block) => Some((lba, block)),
            Resident::Deferred | Resident::Spilled => None,
        })
        .collect();
    Ok(TreeMutation {
        root_lba: root,
        writes,
        stats: context.stats,
    })
}

struct MutationContext<'a, D: BlockDevice, A: TreeAllocator<D>> {
    dev: &'a mut D,
    geo: Geometry,
    tx: &'a mut A,
    spec: TreeSpec,
    new_generation: u64,
    writes: BTreeMap<u64, StagedImage>,
    cache_pages: usize,
    resident_staged_nodes: usize,
    access_clock: u64,
    resident_lru: BTreeSet<(u64, u64)>,
    decoded_residency: Rc<NodeResidency>,
    /// Nodes this batch has already decoded, by block. A visit that finds
    /// its block here skips the image copy, the block checksum and the
    /// decode, and re-runs only the checks that depend on the descent.
    decoded: BTreeMap<u64, DecodedNode>,
    /// Access order of `decoded`, oldest first.
    decoded_lru: BTreeSet<(u64, u64)>,
    decoded_limit: usize,
    stats: TreeMutationStats,
}

/// Decoded nodes a batch may hold beside its staged images. A decoded node
/// costs roughly twice its block image, so this is about 64 KiB at the
/// prototype's 4 KiB block size, held only while one mutation runs. A
/// profile that asked for a staged page budget keeps proportionally fewer
/// and, below sixteen pages, none at all: a constrained machine pays the
/// decode again rather than double what a mutation holds.
const DECODED_CACHE_NODES: usize = 8;

struct DecodedNode {
    node: TrackedNode,
    /// Header generation the image carried, for the identity check that
    /// every visit repeats.
    generation: u64,
    last_used: u64,
}

#[derive(Default)]
struct NodeResidency {
    live: Cell<u64>,
    peak: Cell<u64>,
}

impl NodeResidency {
    fn track(self: &Rc<Self>, node: TreeNode) -> TrackedNode {
        let live = self.live.get().saturating_add(1);
        self.live.set(live);
        self.peak.set(self.peak.get().max(live));
        TrackedNode {
            node: Some(node),
            residency: Rc::clone(self),
        }
    }
}

/// RAII accounting for decoded and derived full tree nodes. Keeping this guard
/// attached to the node makes the tiny-cache measurement follow actual Rust
/// lifetimes rather than hand-maintained recursion counters.
struct TrackedNode {
    node: Option<TreeNode>,
    residency: Rc<NodeResidency>,
}

impl TrackedNode {
    fn fork(&self, node: TreeNode) -> Self {
        self.residency.track(node)
    }
}

impl Deref for TrackedNode {
    type Target = TreeNode;

    fn deref(&self) -> &Self::Target {
        self.node.as_ref().expect("tracked node is always present")
    }
}

impl DerefMut for TrackedNode {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.node.as_mut().expect("tracked node is always present")
    }
}

impl Drop for TrackedNode {
    fn drop(&mut self) {
        if self.node.take().is_some() {
            self.residency.live.set(
                self.residency
                    .live
                    .get()
                    .checked_sub(1)
                    .expect("tracked node residency underflow"),
            );
        }
    }
}

struct StagedImage {
    resident: Resident,
    last_used: u64,
}

/// What a staged block's image is at this moment.
///
/// A node is encoded once the bytes are wanted -- to spill it, to read it
/// back through the image rather than the decoded cache, or to hand the
/// writes to the caller -- and not once per operation of the batch that
/// passes through it. A batch of 512 creates passes through the same leaf,
/// its parent and the root some three times for every image it finally
/// writes.
///
/// `Deferred` means the bytes are not made yet and the node is the batch's
/// decoded node for this block. That cache is then load-bearing: nothing may
/// drop a decoded node whose block is deferred without encoding it first,
/// which [`MutationContext::materialize`] does. Where the decoded cache is
/// switched off -- below sixteen staged pages, the constrained profiles --
/// nothing is ever deferred and every image is encoded as it is staged.
enum Resident {
    /// The encoded block, in memory.
    Bytes(Vec<u8>),
    /// Not encoded yet; the block's decoded node holds it.
    Deferred,
    /// Nothing in memory: the image was spilled to its block.
    Spilled,
}

impl Resident {
    fn in_memory(&self) -> bool {
        !matches!(self, Resident::Spilled)
    }
}

#[derive(Clone)]
struct ChildDesc {
    /// None only for the leftmost child at the current node/root boundary.
    min_key: Option<SmallBytes>,
    reference: ChildRef,
}

struct Replacement {
    children: Vec<ChildDesc>,
    level: u8,
}

struct PendingNode {
    old_lba: u64,
    old_staged: bool,
    node: TrackedNode,
    /// Exact subtree minimum. It is absent only for an empty tree or where a
    /// root boundary does not need to expose the value to a parent.
    min_key: Option<SmallBytes>,
}

type NodeImage = (TrackedNode, Option<SmallBytes>);

impl<D: BlockDevice, A: TreeAllocator<D>> MutationContext<'_, D, A> {
    #[allow(clippy::too_many_arguments)]
    fn upsert_node(
        &mut self,
        lba: u64,
        expected_level: Option<u8>,
        known_min: Option<SmallBytes>,
        lower: Option<SmallBytes>,
        upper: Option<SmallBytes>,
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
        let (mut node, staged, generation) = self.read_node(
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
                Ok(index) => node.items[index].value = value.into(),
                Err(index) => node.items.insert(
                    index,
                    TreeItem {
                        key: key.into(),
                        value: value.into(),
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
        let child = child_at(&node, child_index)?;
        let child_level = node.level - 1;
        // Keep only the compact descent frame across recursion. The parent
        // goes back to the decoded cache, whose bound -- not the tree height
        // -- decides how many pages a descent holds; where that cache is
        // switched off the page is dropped here and re-read on unwind.
        self.cache_decoded(lba, node, generation)?;
        let replacement = self.upsert_node(
            child.reference.lba,
            Some(child_level),
            child.min_key.clone(),
            child_lower,
            child_upper,
            false,
            depth + 1,
            key,
            value,
        )?;
        let (mut node, staged, _) = self.read_node(
            lba,
            expected_level,
            lower.as_deref(),
            upper.as_deref(),
            is_root,
            depth,
        )?;
        if let [single] = replacement.children.as_slice() {
            // The child below did not split, so this node keeps its shape:
            // one slot changes. Writing that slot in place spares two clones
            // of every key the node holds, one to build a child vector and
            // one to put the keys back.
            replace_child(
                &mut node,
                child_index,
                single,
                child.reference.subtree_items,
            )?;
            if node.fits(self.geo.block_size) {
                return self.persist(lba, staged, vec![(node, known_min)]);
            }
            // A longer separator key can still overflow the block; that is
            // the ordinary split below, over the node as just updated.
            let children = children_from_node(&node)?;
            let (left, right, right_min) = split_internal(node, &children, self.geo.block_size)?;
            self.stats.splits += 1;
            return self.persist(
                lba,
                staged,
                vec![(left, known_min), (right, Some(right_min))],
            );
        }
        let mut children = children_from_node(&node)?;
        children.splice(child_index..=child_index, replacement.children);
        if child_index == 0 {
            children[0].min_key = None;
        }
        let node = internal_from_children(node, &children)?;
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
        known_min: Option<SmallBytes>,
        lower: Option<SmallBytes>,
        upper: Option<SmallBytes>,
        is_root: bool,
        depth: u8,
        key: &[u8],
    ) -> Result<PendingNode, CoreError> {
        if depth > MAX_TREE_LEVEL {
            return Err(CoreError::Corrupt(
                "tree mutation exceeded maximum depth".into(),
            ));
        }
        let (mut node, staged, generation) = self.read_node(
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
        let (child_lower, child_upper) = child_range(&node, child_index, &lower, &upper);
        let mut child = child_at(&node, child_index)?;
        if child_index == 0 {
            // The leftmost child's exact minimum is the one this node was
            // entered with; the node itself does not store it.
            child.min_key = known_min.clone();
        }
        let child_level = node.level - 1;
        self.cache_decoded(lba, node, generation)?;
        let edited = self.delete_node(
            child.reference.lba,
            Some(child_level),
            child.min_key,
            child_lower,
            child_upper,
            false,
            depth + 1,
            key,
        )?;
        let (replace_start, replace_end, replacement) =
            if needs_rebalance(&edited.node, self.geo.block_size)? {
                let (parent, _, parent_generation) = self.read_node(
                    lba,
                    expected_level,
                    lower.as_deref(),
                    upper.as_deref(),
                    is_root,
                    depth,
                )?;
                let mut parent_children = children_from_node(&parent)?;
                parent_children[0].min_key = known_min.clone();
                let sibling_index = if child_index + 1 < parent_children.len() {
                    child_index + 1
                } else {
                    child_index - 1
                };
                let (sibling_lower, sibling_upper) =
                    child_range(&parent, sibling_index, &lower, &upper);
                let sibling_desc = parent_children[sibling_index].clone();
                drop(parent_children);
                self.cache_decoded(lba, parent, parent_generation)?;
                let (sibling_node, sibling_staged, _) = self.read_node(
                    sibling_desc.reference.lba,
                    Some(child_level),
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
                let sources = vec![
                    (left.old_lba, left.old_staged),
                    (right.old_lba, right.old_staged),
                ];
                let outputs = rebalance_pair(left, right, self.geo.block_size)?;
                let output_count = outputs.len();
                let replacement = self.persist_sources(sources, outputs)?;
                if output_count == 1 {
                    self.stats.merges += 1;
                } else {
                    self.stats.redistributions += 1;
                }
                (left_index, left_index + 1, replacement)
            } else {
                let source = (edited.old_lba, edited.old_staged);
                let output = (edited.node, edited.min_key);
                let replacement = self.persist_sources(vec![source], vec![output])?;
                (child_index, child_index, replacement)
            };

        let (node, staged, _) = self.read_node(
            lba,
            expected_level,
            lower.as_deref(),
            upper.as_deref(),
            is_root,
            depth,
        )?;
        let mut children = children_from_node(&node)?;
        children[0].min_key = known_min.clone();
        children.splice(replace_start..=replace_end, replacement);
        let min_key = children[0].min_key.clone().or(known_min);
        let node = internal_allow_one(node, &children)?;
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
    ) -> Result<(TrackedNode, bool, u64), CoreError> {
        if depth > MAX_TREE_LEVEL {
            return Err(CoreError::Corrupt(
                "tree mutation exceeded maximum depth".into(),
            ));
        }
        check_tree_lba(&self.geo, lba)?;
        let is_staged = self.writes.contains_key(&lba);
        if let Some((node, generation)) = self.take_decoded(lba) {
            // This batch decoded the block and has staged no newer image for
            // it since, so its checksum and its child set were checked when
            // it was decoded and nothing outside this batch can have touched
            // it. What depends on this descent -- the node's identity, its
            // generation and its key range against the parent's bounds -- is
            // checked again here.
            self.stats.node_reads += 1;
            self.stats.max_depth = self.stats.max_depth.max(depth + 1);
            self.validate_read(&node, generation, is_staged, expected_level, lba)?;
            validate_node_range(&node, lower, upper, is_root)?;
            return Ok((node, is_staged, generation));
        }
        let block = if is_staged {
            let access = self.next_access();
            // A read through the image needs the bytes, so an image that is
            // still deferred is encoded here.
            self.materialize(lba)?;
            let resident = {
                let image = self.writes.get_mut(&lba).ok_or_else(|| {
                    CoreError::Corrupt("staged tree membership changed during read".into())
                })?;
                if image.resident.in_memory() {
                    self.resident_lru.remove(&(image.last_used, lba));
                }
                image.last_used = access;
                match &image.resident {
                    Resident::Bytes(block) => Some(block.clone()),
                    Resident::Deferred | Resident::Spilled => None,
                }
            };
            if let Some(block) = resident {
                self.resident_lru.insert((access, lba));
                block
            } else {
                let mut block = vec![0u8; self.geo.block_size];
                self.observe(crate::flight::EventKind::TreeReadBegin, lba);
                if let Err(error) = self.dev.read_block(lba, &mut block) {
                    self.observe(crate::flight::EventKind::TreeIoFailed, lba);
                    return Err(error.into());
                }
                self.observe(crate::flight::EventKind::TreeReadComplete, lba);
                self.stats.device_reads += 1;
                self.stats.staged_spill_reloads += 1;
                self.writes
                    .get_mut(&lba)
                    .ok_or_else(|| CoreError::Corrupt("staged tree image disappeared".into()))?
                    .resident = Resident::Bytes(block.clone());
                self.resident_staged_nodes += 1;
                self.resident_lru.insert((access, lba));
                self.enforce_cache_limit()?;
                block
            }
        } else {
            let mut block = vec![0u8; self.geo.block_size];
            self.observe(crate::flight::EventKind::TreeReadBegin, lba);
            if let Err(error) = self.dev.read_block(lba, &mut block) {
                self.observe(crate::flight::EventKind::TreeIoFailed, lba);
                return Err(error.into());
            }
            self.observe(crate::flight::EventKind::TreeReadComplete, lba);
            self.stats.device_reads += 1;
            block
        };
        self.stats.node_reads += 1;
        self.stats.max_depth = self.stats.max_depth.max(depth + 1);
        let (node, generation) = TreeNode::decode(&block)
            .map_err(|error| CoreError::Corrupt(format!("tree node {lba}: {error}")))?;
        self.validate_read(&node, generation, is_staged, expected_level, lba)?;
        validate_node_range(&node, lower, upper, is_root)?;
        if !node.is_leaf() {
            check_children_unique(&node)?;
        }
        Ok((self.track_node(node), is_staged, generation))
    }

    /// Identity and generation of a node this mutation is about to use,
    /// whether it was just decoded or came back from the decoded cache.
    fn validate_read(
        &self,
        node: &TreeNode,
        generation: u64,
        is_staged: bool,
        expected_level: Option<u8>,
        lba: u64,
    ) -> Result<(), CoreError> {
        let validation_spec = TreeSpec {
            max_generation: if is_staged {
                self.new_generation
            } else {
                self.spec.max_generation
            },
            ..self.spec
        };
        validate_node_identity(node, generation, validation_spec, expected_level, lba)?;
        if is_staged && generation != self.new_generation {
            return Err(CoreError::Corrupt(
                "staged tree node generation mismatch".into(),
            ));
        }
        Ok(())
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
            self.retire_block(old_lba)?;
            self.stats.committed_nodes_retired += 1;
        }
        while lbas.len() < nodes.len() {
            lbas.push(self.allocate_block()?);
            self.stats.nodes_allocated += 1;
        }
        let mut children = Vec::with_capacity(nodes.len());
        for ((node, min_key), lba) in nodes.into_iter().zip(lbas) {
            let reference = ChildRef {
                lba,
                subtree_items: node.subtree_items,
            };
            self.stage_node(lba, node)?;
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
                self.retire_block(lba)?;
                self.stats.committed_nodes_retired += 1;
            }
        }
        while reusable.len() < outputs.len() {
            reusable.push(self.allocate_block()?);
            self.stats.nodes_allocated += 1;
        }
        while reusable.len() > outputs.len() {
            let lba = reusable.pop().expect("length checked above");
            self.remove_staged(lba);
            self.release_block(lba)?;
            self.stats.staged_nodes_discarded += 1;
        }
        let mut replacement = Vec::with_capacity(outputs.len());
        for ((node, min_key), lba) in outputs.into_iter().zip(reusable) {
            let reference = ChildRef {
                lba,
                subtree_items: node.subtree_items,
            };
            self.stage_node(lba, node)?;
            replacement.push(ChildDesc { min_key, reference });
        }
        Ok(replacement)
    }

    fn discard_pending(&mut self, pending: PendingNode) -> Result<(), CoreError> {
        if pending.old_staged {
            if self.remove_staged(pending.old_lba).is_none() {
                return Err(CoreError::Corrupt(
                    "staged tree source has no write image".into(),
                ));
            }
            self.release_block(pending.old_lba)?;
            self.stats.staged_nodes_discarded += 1;
        } else {
            self.retire_block(pending.old_lba)?;
            self.stats.committed_nodes_retired += 1;
        }
        Ok(())
    }

    /// Stages `node` as the image of block `lba` and gives it to the
    /// decoded cache. The image is deferred where that cache keeps the node,
    /// and encoded here where it does not, so a block is never staged
    /// without a way to produce its bytes.
    fn stage_node(&mut self, lba: u64, node: TrackedNode) -> Result<(), CoreError> {
        // Whatever the batch had decoded for this block is now an old image,
        // and so is whatever it staged: neither has to be encoded.
        self.decoded_drop(lba);
        let resident = if self.decoded_limit == 0 {
            Resident::Bytes(
                node.encode(self.geo.block_size, self.new_generation)
                    .map_err(CoreError::Format)?,
            )
        } else {
            Resident::Deferred
        };
        let access = self.next_access();
        let previous = self.writes.insert(
            lba,
            StagedImage {
                resident,
                last_used: access,
            },
        );
        if let Some(previous) = &previous {
            if previous.resident.in_memory() {
                self.resident_lru.remove(&(previous.last_used, lba));
            }
        }
        if previous.is_none_or(|image| !image.resident.in_memory()) {
            self.resident_staged_nodes += 1;
        }
        self.resident_lru.insert((access, lba));
        let generation = self.new_generation;
        self.cache_decoded(lba, node, generation)?;
        self.enforce_cache_limit()?;
        Ok(())
    }

    /// Encodes block `lba`'s image if it is still deferred, so that the
    /// bytes exist. The node comes from the decoded cache, which holds it
    /// for exactly as long as the image is deferred.
    fn materialize(&mut self, lba: u64) -> Result<(), CoreError> {
        if !matches!(
            self.writes.get(&lba).map(|image| &image.resident),
            Some(Resident::Deferred)
        ) {
            return Ok(());
        }
        let node = self
            .decoded
            .get(&lba)
            .ok_or_else(|| CoreError::Corrupt("deferred tree image has no decoded node".into()))?;
        let encoded = node
            .node
            .encode(self.geo.block_size, self.new_generation)
            .map_err(CoreError::Format)?;
        self.stats.deferred_encodes += 1;
        if let Some(image) = self.writes.get_mut(&lba) {
            image.resident = Resident::Bytes(encoded);
        }
        Ok(())
    }

    fn observe(&mut self, kind: crate::flight::EventKind, lba: u64) {
        self.tx.observe_tree(
            self.new_generation,
            kind,
            crate::flight::TreeContext {
                owner: self.spec.owner,
                block: lba,
                resident: self.resident_staged_nodes as u64,
            },
        );
    }

    fn enforce_cache_limit(&mut self) -> Result<(), CoreError> {
        self.stats.max_staged_nodes_before_eviction = self
            .stats
            .max_staged_nodes_before_eviction
            .max(self.resident_staged_nodes as u64);
        while self.resident_staged_nodes > self.cache_pages {
            let (_, lba) = self
                .resident_lru
                .pop_first()
                .ok_or_else(|| CoreError::Corrupt("staged cache accounting mismatch".into()))?;
            self.materialize(lba)?;
            let block = match self
                .writes
                .get_mut(&lba)
                .map(|image| std::mem::replace(&mut image.resident, Resident::Spilled))
            {
                Some(Resident::Bytes(block)) => block,
                _ => {
                    return Err(CoreError::Corrupt(
                        "staged cache victim has no image".into(),
                    ))
                }
            };
            self.observe(crate::flight::EventKind::TreeSpillBegin, lba);
            if let Err(error) = self.dev.write_block(lba, &block) {
                self.observe(crate::flight::EventKind::TreeIoFailed, lba);
                return Err(error.into());
            }
            self.observe(crate::flight::EventKind::TreeSpillComplete, lba);
            // The image left memory; let its decoded node go with it rather
            // than keep a page the budget has just refused. It is already
            // encoded, so nothing here needs it.
            self.decoded_drop(lba);
            self.resident_staged_nodes -= 1;
            self.stats.staged_spill_writes += 1;
        }
        self.stats.max_resident_staged_nodes = self
            .stats
            .max_resident_staged_nodes
            .max(self.resident_staged_nodes as u64);
        Ok(())
    }

    fn remove_staged(&mut self, lba: u64) -> Option<StagedImage> {
        self.decoded_drop(lba);
        let image = self.writes.remove(&lba)?;
        if image.resident.in_memory() {
            self.resident_lru.remove(&(image.last_used, lba));
            self.resident_staged_nodes -= 1;
        }
        Some(image)
    }

    fn next_access(&mut self) -> u64 {
        self.access_clock = self.access_clock.saturating_add(1);
        self.access_clock
    }

    fn track_node(&self, node: TreeNode) -> TrackedNode {
        self.decoded_residency.track(node)
    }

    /// Takes a block's decoded node out of the cache. The caller owns it and
    /// either consumes it or gives it back with [`Self::cache_decoded`].
    fn take_decoded(&mut self, lba: u64) -> Option<(TrackedNode, u64)> {
        let entry = self.decoded.remove(&lba)?;
        self.decoded_lru.remove(&(entry.last_used, lba));
        Some((entry.node, entry.generation))
    }

    /// Gives a decoded node to the batch, dropping the least recently used
    /// one when the cache is full. A zero bound drops the node here.
    fn cache_decoded(
        &mut self,
        lba: u64,
        node: TrackedNode,
        generation: u64,
    ) -> Result<(), CoreError> {
        if self.decoded_limit == 0 {
            return Ok(());
        }
        self.decoded_drop(lba);
        let last_used = self.next_access();
        self.decoded.insert(
            lba,
            DecodedNode {
                node,
                generation,
                last_used,
            },
        );
        self.decoded_lru.insert((last_used, lba));
        while self.decoded.len() > self.decoded_limit {
            let Some((_, victim)) = self.decoded_lru.pop_first() else {
                break;
            };
            // A victim whose image is still deferred is encoded before its
            // node goes: the image must not lose its only copy.
            self.materialize(victim)?;
            self.decoded.remove(&victim);
        }
        Ok(())
    }

    /// Forgets a block's decoded node, encoding its image first where that
    /// image is still deferred.
    fn decoded_forget(&mut self, lba: u64) -> Result<(), CoreError> {
        self.materialize(lba)?;
        self.decoded_drop(lba);
        Ok(())
    }

    /// Forgets a block's decoded node without encoding anything, for the
    /// callers that are discarding the staged image with it.
    fn decoded_drop(&mut self, lba: u64) {
        if let Some(previous) = self.decoded.remove(&lba) {
            self.decoded_lru.remove(&(previous.last_used, lba));
        }
    }

    /// Allocates a tree block and forgets anything the batch still had
    /// decoded for it: a block this transaction released can come back.
    fn allocate_block(&mut self) -> Result<u64, CoreError> {
        let lba = self.tx.allocate_tree_block(self.dev)?;
        self.decoded_forget(lba)?;
        Ok(lba)
    }

    fn retire_block(&mut self, lba: u64) -> Result<(), CoreError> {
        self.decoded_forget(lba)?;
        self.tx.retire_tree_block(self.dev, lba)
    }

    fn release_block(&mut self, lba: u64) -> Result<(), CoreError> {
        self.decoded_forget(lba)?;
        self.tx.release_tree_block(self.dev, lba)
    }
}

/// The full child vector of an internal node. Only the paths that rebuild or
/// split a node need it; a descent asks [`child_at`] for the one child it
/// follows and leaves the node's keys where they are.
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

/// The one child a descent follows. The items are ordered, so the caller's
/// partition point over the keys already names the slot: child 0 is the
/// leftmost pointer, child `i` is item `i - 1`. Reading it in place keeps a
/// descent independent of the node's fanout.
fn child_at(node: &TreeNode, index: usize) -> Result<ChildDesc, CoreError> {
    if index == 0 {
        return Ok(ChildDesc {
            min_key: None,
            reference: ChildRef {
                lba: node.leftmost_child,
                subtree_items: node.leftmost_items,
            },
        });
    }
    let item = node
        .items
        .get(index - 1)
        .ok_or_else(|| CoreError::Corrupt("tree child index out of range".into()))?;
    Ok(ChildDesc {
        min_key: Some(item.key.clone()),
        reference: TreeNode::child_ref(item).map_err(CoreError::Format)?,
    })
}

/// Every child of an internal node names a distinct block: a node that named
/// one twice would have the batch stage and retire the same block along two
/// paths. The answer cannot change while the batch holds the node, so this
/// runs once per node event -- a decode, or an image staged by this batch --
/// and not once per visit. Sorting the child blocks costs one small vector
/// where a set cost an insertion per child.
fn check_children_unique(node: &TreeNode) -> Result<(), CoreError> {
    let mut blocks = Vec::with_capacity(node.items.len() + 1);
    blocks.push(node.leftmost_child);
    for item in &node.items {
        blocks.push(TreeNode::child_ref(item).map_err(CoreError::Format)?.lba);
    }
    blocks.sort_unstable();
    if blocks.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(CoreError::Corrupt(
            "internal tree node references a child more than once".into(),
        ));
    }
    Ok(())
}

/// Points one slot of an internal node at a new child, keeping the node's
/// item count and therefore its shape. `previous_items` is what the slot's
/// subtree held before, so the node's total follows without walking it.
fn replace_child(
    node: &mut TreeNode,
    index: usize,
    child: &ChildDesc,
    previous_items: u64,
) -> Result<(), CoreError> {
    if index == 0 {
        node.leftmost_child = child.reference.lba;
        node.leftmost_items = child.reference.subtree_items;
    } else {
        let key = child
            .min_key
            .clone()
            .ok_or_else(|| CoreError::Corrupt("non-leftmost child has no minimum".into()))?;
        let item = node
            .items
            .get_mut(index - 1)
            .ok_or_else(|| CoreError::Corrupt("tree child index out of range".into()))?;
        item.key = key;
        item.value = child_value(child.reference)
            .map_err(CoreError::Format)?
            .into();
    }
    node.subtree_items = node
        .subtree_items
        .checked_sub(previous_items)
        .and_then(|total| total.checked_add(child.reference.subtree_items))
        .ok_or_else(|| CoreError::Corrupt("tree item count overflow".into()))?;
    Ok(())
}

fn child_range(
    node: &TreeNode,
    child_index: usize,
    lower: &Option<SmallBytes>,
    upper: &Option<SmallBytes>,
) -> (Option<SmallBytes>, Option<SmallBytes>) {
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
    mut template: TrackedNode,
    children: &[ChildDesc],
) -> Result<TrackedNode, CoreError> {
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
    left: PendingNode,
    mut right: PendingNode,
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
        let mut combined = left.node;
        combined.items.append(&mut right.node.items);
        combined.subtree_items = combined.items.len() as u64;
        drop(right.node);
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
    drop(right.node);
    let combined_min = children[0].min_key.clone();
    let combined = internal_from_children(left.node, &children)?;
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
    mut template: TrackedNode,
    children: &[ChildDesc],
) -> Result<TrackedNode, CoreError> {
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
            value: child_value(child.reference)
                .map_err(CoreError::Format)?
                .into(),
        });
    }
    template.subtree_items = total;
    Ok(template)
}

fn split_leaf(
    mut node: TrackedNode,
    block_size: usize,
) -> Result<(TrackedNode, TrackedNode), CoreError> {
    let capacity = block_size.saturating_sub(afsplus_format::header::HEADER_SIZE);
    // The encoded length of a prefix of the items is a prefix sum, so one
    // pass answers every candidate split. Measuring both halves for each
    // candidate made splitting a leaf cost the square of its item count.
    let base = encoded_items_len(&[])?;
    let mut prefix = Vec::with_capacity(node.items.len() + 1);
    prefix.push(base);
    for index in 0..node.items.len() {
        let length = encoded_items_len(&node.items[index..=index])?
            .checked_sub(base)
            .and_then(|item| prefix[index].checked_add(item))
            .ok_or_else(|| CoreError::Corrupt("tree node size overflow".into()))?;
        prefix.push(length);
    }
    let total = prefix[node.items.len()];
    let mut best: Option<(usize, usize)> = None;
    for (split, left_len) in prefix
        .iter()
        .copied()
        .enumerate()
        .take(node.items.len())
        .skip(1)
    {
        let right_len = base + (total - left_len);
        if left_len <= capacity && right_len <= capacity {
            let difference = left_len.abs_diff(right_len);
            if best
                .as_ref()
                .is_none_or(|(best_difference, _)| difference < *best_difference)
            {
                best = Some((difference, split));
            }
        }
    }
    let split = best
        .map(|(_, split)| split)
        .ok_or(CoreError::PrototypeLimit(
            "tree leaf item cannot be split to fit",
        ))?;
    let right_items = node.items.split_off(split);
    node.subtree_items = node.items.len() as u64;
    let right = node.fork(TreeNode {
        kind: node.kind,
        owner: node.owner,
        level: node.level,
        subtree_items: right_items.len() as u64,
        leftmost_child: 0,
        leftmost_items: 0,
        items: right_items,
    });
    Ok((node, right))
}

fn split_internal(
    node: TrackedNode,
    children: &[ChildDesc],
    block_size: usize,
) -> Result<(TrackedNode, TrackedNode, SmallBytes), CoreError> {
    let capacity = block_size.saturating_sub(afsplus_format::header::HEADER_SIZE);
    let mut best: Option<(usize, usize)> = None;
    for split in 2..children.len().saturating_sub(1) {
        let left_len = encoded_children_len(&children[..split])?;
        let right_len = encoded_children_len(&children[split..])?;
        if left_len <= capacity && right_len <= capacity {
            let difference = left_len.abs_diff(right_len);
            if best
                .as_ref()
                .is_none_or(|(best_difference, _)| difference < *best_difference)
            {
                best = Some((difference, split));
            }
        }
    }
    let split = best
        .map(|(_, split)| split)
        .ok_or(CoreError::PrototypeLimit(
            "internal tree node cannot be split to fit",
        ))?;
    let promoted = children[split]
        .min_key
        .clone()
        .ok_or_else(|| CoreError::Corrupt("internal split has no promoted key".into()))?;
    let right_template = node.fork(TreeNode {
        kind: node.kind,
        owner: node.owner,
        level: node.level,
        subtree_items: 0,
        leftmost_child: 0,
        leftmost_items: 0,
        items: Vec::new(),
    });
    let left = internal_from_children(node, &children[..split])?;
    let mut right_children = children[split..].to_vec();
    right_children[0].min_key = None;
    let right = internal_from_children(right_template, &right_children)?;
    Ok((left, right, promoted))
}

fn encoded_items_len(items: &[TreeItem]) -> Result<usize, CoreError> {
    items.iter().try_fold(32usize, |length, item| {
        length
            .checked_add(8)
            .and_then(|value| value.checked_add(item.key.len()))
            .and_then(|value| value.checked_add(item.value.len()))
            .ok_or_else(|| CoreError::Corrupt("tree node size overflow".into()))
    })
}

fn encoded_children_len(children: &[ChildDesc]) -> Result<usize, CoreError> {
    if children.len() < 2 {
        return Err(CoreError::Corrupt(
            "internal split side has fewer than two children".into(),
        ));
    }
    children[1..].iter().try_fold(32usize, |length, child| {
        let key = child
            .min_key
            .as_ref()
            .ok_or_else(|| CoreError::Corrupt("internal child has no minimum".into()))?;
        length
            .checked_add(8 + 16)
            .and_then(|value| value.checked_add(key.len()))
            .ok_or_else(|| CoreError::Corrupt("tree node size overflow".into()))
    })
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

    use super::{
        delete_many, mutate_many, mutate_many_with_cache_limit, upsert_many, TreeOperation,
    };
    use crate::alloc::TxAllocator;
    use crate::allocation_root::{self, ReservedTreePool};
    use crate::tree::{lookup, validate_tree, visit_tree_nodes, TreeSpec};
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
        let old_root = checkpoint.object_map_block;
        let mut dev = vol.into_device();
        let empty = TreeNode::leaf(TreeKind::ObjectMap, 0)
            .encode(4096, 1)
            .unwrap();
        dev.write_block(old_root, &empty).unwrap();
        let mut tx = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 4096, 0).unwrap();
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
        let finished = tx.finish(&mut dev).unwrap();
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
        let current_allocation = allocation_root::load_all(
            &mut dev,
            &geo,
            checkpoint.allocation_root_block,
            checkpoint.generation,
        )
        .unwrap();
        let mut allocation_pool = ReservedTreePool::new(
            allocation_root::reserved_pool_lbas(&geo).unwrap(),
            &current_allocation.tree_blocks,
            &[],
        )
        .unwrap();
        let allocation_values: Vec<_> = finished
            .dirty_records
            .iter()
            .map(|(region, record)| {
                (
                    allocation_root::key(*region),
                    allocation_root::value(*record).unwrap(),
                )
            })
            .collect();
        let allocation_operations: Vec<_> = allocation_values
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let allocation_mutation = mutate_many(
            &mut dev,
            &geo,
            &mut allocation_pool,
            checkpoint.allocation_root_block,
            allocation_root::spec(1),
            2,
            &allocation_operations,
        )
        .unwrap();
        for (lba, block) in &allocation_mutation.writes {
            dev.write_block(*lba, block).unwrap();
        }
        checkpoint2.allocation_root_block = allocation_mutation.root_lba;
        checkpoint2.free_blocks_total = finished.free_blocks_total;
        for (lba, block) in &finished.reclaim_writes {
            dev.write_block(*lba, block).unwrap();
        }
        checkpoint2.reclaim_root_block = finished.reclaim_root_lba;
        let mut tx2 =
            TxAllocator::begin(&mut dev, &geo, &checkpoint2, Some(&checkpoint), 3, 4096, 0)
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
        tx2.finish(&mut dev).unwrap();
    }

    /// A batch encodes an image once, not once per operation that passes
    /// through it, and a profile too small for a decoded cache encodes every
    /// image as it is staged, as it always did.
    #[test]
    fn a_batch_encodes_an_image_once_and_a_tiny_profile_encodes_eagerly() {
        let geo = afsplus_format::geometry::Geometry {
            block_size: 4096,
            total_blocks: 8192,
            region_size: 8192,
        };
        let spec = TreeSpec {
            kind: TreeKind::ObjectMap,
            owner: 0,
            max_generation: 1,
        };
        let entries: Vec<_> = (0..512u64)
            .map(|ordinal| (wide_key(ordinal), vec![ordinal as u8; 24]))
            .collect();
        let operations: Vec<_> = entries
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();

        // 256 staged pages give the batch a decoded cache; 8 give it none.
        for (cache_pages, deferred) in [(256usize, true), (8, false)] {
            let mut dev = MemoryBackend::new(4096, 8192);
            dev.write_block(
                100,
                &TreeNode::leaf(TreeKind::ObjectMap, 0)
                    .encode(4096, 1)
                    .unwrap(),
            )
            .unwrap();
            let mut pool = ReservedTreePool::new(100..4000, &[100], &[]).unwrap();
            let mutation = mutate_many_with_cache_limit(
                &mut dev,
                &geo,
                &mut pool,
                100,
                spec,
                2,
                &operations,
                cache_pages,
            )
            .unwrap();
            if deferred {
                // One encode per image the batch ends with, give or take the
                // images it read back or spilled, against one per operation.
                assert!(
                    mutation.stats.deferred_encodes
                        <= mutation.stats.final_nodes_written + mutation.stats.staged_spill_writes,
                    "{} encodes for {} images",
                    mutation.stats.deferred_encodes,
                    mutation.stats.final_nodes_written
                );
                assert!(
                    mutation.stats.deferred_encodes < operations.len() as u64,
                    "the batch encoded once per operation"
                );
            } else {
                assert_eq!(mutation.stats.deferred_encodes, 0);
            }
            for (lba, block) in &mutation.writes {
                dev.write_block(*lba, block).unwrap();
            }
            let summary = validate_tree(
                &mut dev,
                &geo,
                mutation.root_lba,
                TreeSpec {
                    max_generation: 2,
                    ..spec
                },
            )
            .unwrap();
            assert_eq!(summary.items, 512);
            for (key, value) in &entries {
                assert_eq!(
                    lookup(
                        &mut dev,
                        &geo,
                        mutation.root_lba,
                        TreeSpec {
                            max_generation: 2,
                            ..spec
                        },
                        key,
                    )
                    .unwrap()
                    .0,
                    Some(value.clone())
                );
            }
        }
    }

    #[test]
    fn constrained_mutation_cache_spills_and_reloads_at_two_four_and_eight_pages() {
        let geo = afsplus_format::geometry::Geometry {
            block_size: 4096,
            total_blocks: 8192,
            region_size: 8192,
        };
        let spec = TreeSpec {
            kind: TreeKind::ObjectMap,
            owner: 0,
            max_generation: 1,
        };
        let entries: Vec<_> = (0..1_000u64)
            .map(|ordinal| {
                let key = (ordinal * 137) % 1_000;
                (wide_key(key), vec![key as u8; 80])
            })
            .collect();
        let operations: Vec<_> = entries
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();

        for cache_pages in [2, 4, 8] {
            let mut dev = MemoryBackend::new(4096, 8192);
            dev.write_block(
                100,
                &TreeNode::leaf(TreeKind::ObjectMap, 0)
                    .encode(4096, 1)
                    .unwrap(),
            )
            .unwrap();
            let mut pool = ReservedTreePool::new(100..4000, &[100], &[]).unwrap();
            let mutation = mutate_many_with_cache_limit(
                &mut dev,
                &geo,
                &mut pool,
                100,
                spec,
                2,
                &operations,
                cache_pages,
            )
            .unwrap();
            assert!(mutation.stats.staged_spill_writes > 0);
            assert!(mutation.stats.staged_spill_reloads > 0);
            assert!(mutation.stats.max_resident_staged_nodes <= cache_pages as u64);
            assert_eq!(
                mutation.stats.max_staged_nodes_before_eviction,
                cache_pages as u64 + 1
            );
            assert!(
                mutation.stats.max_live_decoded_nodes <= 2,
                "insertion retained {} decoded nodes with a {cache_pages}-page staged cache",
                mutation.stats.max_live_decoded_nodes
            );
            assert!(mutation.writes.len() < mutation.stats.final_nodes_written as usize);
            for (lba, block) in &mutation.writes {
                dev.write_block(*lba, block).unwrap();
            }
            let summary = validate_tree(
                &mut dev,
                &geo,
                mutation.root_lba,
                TreeSpec {
                    max_generation: 2,
                    ..spec
                },
            )
            .unwrap();
            assert_eq!(summary.items, 1_000);
            assert!(summary.height >= 3);

            let generation_two_spec = TreeSpec {
                max_generation: 2,
                ..spec
            };
            let mut current_blocks = Vec::new();
            visit_tree_nodes(
                &mut dev,
                &geo,
                mutation.root_lba,
                generation_two_spec,
                |lba, _| {
                    current_blocks.push(lba);
                    Ok(())
                },
            )
            .unwrap();
            // The reserved-pool exclusion set must include spilled images.
            // Filtering only mutation.writes would omit live committed nodes.
            let mut tracked: Vec<_> = pool.allocated_nodes().collect();
            tracked.extend(
                [100]
                    .into_iter()
                    .filter(|lba| !pool.retired_nodes().any(|r| r == *lba)),
            );
            tracked.sort_unstable();
            current_blocks.sort_unstable();
            assert_eq!(tracked, current_blocks);
            let delete_keys: Vec<_> = (0..999u64)
                .map(|ordinal| wide_key((ordinal * 137) % 999))
                .collect();
            let delete_operations: Vec<_> = delete_keys
                .iter()
                .map(|key| TreeOperation::Delete { key })
                .collect();
            let mut delete_pool = ReservedTreePool::new(100..4000, &current_blocks, &[]).unwrap();
            let deletion = mutate_many_with_cache_limit(
                &mut dev,
                &geo,
                &mut delete_pool,
                mutation.root_lba,
                generation_two_spec,
                3,
                &delete_operations,
                cache_pages,
            )
            .unwrap();
            assert_eq!(deletion.stats.deletes, 999);
            assert!(deletion.stats.merges > 0);
            assert!(deletion.stats.root_collapses > 0);
            assert!(deletion.stats.staged_spill_writes > 0);
            assert!(deletion.stats.staged_spill_reloads > 0);
            assert!(deletion.stats.max_resident_staged_nodes <= cache_pages as u64);
            assert!(
                deletion.stats.max_live_decoded_nodes <= 2,
                "deletion retained {} decoded nodes with a {cache_pages}-page staged cache",
                deletion.stats.max_live_decoded_nodes
            );
            for (lba, block) in &deletion.writes {
                dev.write_block(*lba, block).unwrap();
            }
            let final_summary = validate_tree(
                &mut dev,
                &geo,
                deletion.root_lba,
                TreeSpec {
                    max_generation: 3,
                    ..spec
                },
            )
            .unwrap();
            assert_eq!(final_summary.items, 1);
            assert_eq!(final_summary.height, 1);
        }
    }

    #[test]
    #[ignore = "explicit 100k-key scale qualification"]
    fn bounded_overlay_qualifies_one_hundred_thousand_compact_keys() {
        let geo = afsplus_format::geometry::Geometry {
            block_size: 4096,
            total_blocks: 8192,
            region_size: 8192,
        };
        let spec = TreeSpec {
            kind: TreeKind::ObjectMap,
            owner: 0,
            max_generation: 1,
        };
        let entries: Vec<_> = (0..100_000u64)
            .map(|ordinal| {
                let key = (ordinal * 7_919) % 100_000;
                (key_u64(key), key.to_le_bytes())
            })
            .collect();
        let operations: Vec<_> = entries
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let mut dev = MemoryBackend::new(4096, 8192);
        dev.write_block(
            100,
            &TreeNode::leaf(TreeKind::ObjectMap, 0)
                .encode(4096, 1)
                .unwrap(),
        )
        .unwrap();
        let mut pool = ReservedTreePool::new(100..8000, &[100], &[]).unwrap();
        let mutation =
            mutate_many_with_cache_limit(&mut dev, &geo, &mut pool, 100, spec, 2, &operations, 8)
                .unwrap();
        assert!(mutation.stats.staged_spill_writes > 0);
        assert!(mutation.stats.staged_spill_reloads > 0);
        assert!(mutation.stats.max_resident_staged_nodes <= 8);
        for (lba, block) in &mutation.writes {
            dev.write_block(*lba, block).unwrap();
        }
        let summary = validate_tree(
            &mut dev,
            &geo,
            mutation.root_lba,
            TreeSpec {
                max_generation: 2,
                ..spec
            },
        )
        .unwrap();
        assert_eq!(summary.items, 100_000);
        assert!(summary.height >= 3);
        for key in [0u64, 49_999, 99_999] {
            assert_eq!(
                lookup(
                    &mut dev,
                    &geo,
                    mutation.root_lba,
                    TreeSpec {
                        max_generation: 2,
                        ..spec
                    },
                    &key_u64(key),
                )
                .unwrap()
                .0,
                Some(key.to_le_bytes().to_vec())
            );
        }
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
        let root = checkpoint.object_map_block;
        let mut dev = vol.into_device();
        let corrupt_root = TreeNode {
            kind: TreeKind::ObjectMap,
            owner: 0,
            level: 1,
            subtree_items: 2,
            leftmost_child: root,
            leftmost_items: 1,
            items: vec![TreeItem {
                key: key_u64(100).into(),
                value: child_value(ChildRef {
                    lba: root + 1,
                    subtree_items: 1,
                })
                .unwrap()
                .into(),
            }],
        }
        .encode(4096, 1)
        .unwrap();
        dev.write_block(root, &corrupt_root).unwrap();
        let mut tx = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 4096, 0).unwrap();
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

    /// Negative control for the duplicate-child check now that it runs once
    /// per node event instead of once per visit: an internal node whose
    /// leftmost pointer and whose only item name the same block must still
    /// stop the mutation, and it must do so before the descent follows
    /// either of them.
    #[test]
    fn mutation_rejects_a_node_that_names_one_child_twice() {
        let mut dev = MemoryBackend::new(4096, 2048);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [93u8; 16],
                label: "CowTreeTwice".into(),
                region_size: 2048,
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
        let root = checkpoint.object_map_block;
        let mut dev = vol.into_device();
        // A block past every reserved head, so the leaf this node names twice
        // is a real leaf and not a piece of volume metadata.
        let leaf = geo.region0_reserved_blocks() + 200;
        dev.write_block(
            leaf,
            &TreeNode {
                kind: TreeKind::ObjectMap,
                owner: 0,
                level: 0,
                subtree_items: 1,
                leftmost_child: 0,
                leftmost_items: 0,
                items: vec![TreeItem {
                    key: key_u64(100).into(),
                    value: vec![7].into(),
                }],
            }
            .encode(4096, 1)
            .unwrap(),
        )
        .unwrap();
        let corrupt_root = TreeNode {
            kind: TreeKind::ObjectMap,
            owner: 0,
            level: 1,
            subtree_items: 2,
            leftmost_child: leaf,
            leftmost_items: 1,
            items: vec![TreeItem {
                key: key_u64(100).into(),
                value: child_value(ChildRef {
                    lba: leaf,
                    subtree_items: 1,
                })
                .unwrap()
                .into(),
            }],
        }
        .encode(4096, 1)
        .unwrap();
        dev.write_block(root, &corrupt_root).unwrap();
        let mut tx = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 4096, 0).unwrap();
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
            crate::CoreError::Corrupt(message)
                if message.contains("references a child more than once")
        ));
    }
}
