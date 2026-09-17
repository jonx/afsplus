//! Explain API, first slice: what the committed state believes about one
//! block (`docs/26-debug-observability.md` section 5).
//!
//! The walk is its own reader. It selects the checkpoint, descends every
//! tree and follows every reference with the block codecs of
//! `afsplus-format` only, and shares no traversal, claim set or loader with
//! the checker. The two therefore answer the ownership question
//! independently, and a disagreement between them is a finding about one of
//! them.
//!
//! The walk never writes and never repairs. A structure it cannot decode
//! ends that branch and is reported in [`Explainer::problems`]; blocks behind
//! it stay unattributed, which is what the committed state can prove.

use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::extent::{ExtentItem, EXTENT_SHARED, EXTENT_UNWRITTEN};
use afsplus_format::header::{block_type, BlockHeader};
use afsplus_format::ident::Identification;
use afsplus_format::object::{
    ObjectRecord, ObjectType, SymlinkRecord, OBJECT_FLAG_DATA_IN_PLACE, OBJECT_FLAG_EXTENT_TREE,
    SECURITY_REF_PROJECTION_DIVERGED,
};
use afsplus_format::reclaim::{ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable};
use afsplus_format::region::RegionDescriptor;
use afsplus_format::security::SecuritySegment;
use afsplus_format::tree::{TreeKind, TreeNode};
use afsplus_format::Timespec;

/// Versioned structured-output schema of the explain records (ADR-025).
pub const EXPLAIN_SCHEMA_VERSION: u32 = 1;

/// One thing the committed state says a block is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockRole {
    Identification,
    /// Checkpoint slot 0 (A) or 1 (B); `selected` names the committed one.
    CheckpointSlot {
        slot: u8,
        selected: bool,
    },
    /// Reserved region-descriptor slot; `live` when the allocation root
    /// selects it.
    RegionDescriptorSlot {
        region: u32,
        slot: u8,
        live: bool,
    },
    /// Reserved bitmap-page slot; `live` when the live descriptor binds it.
    BitmapSlot {
        region: u32,
        page: u32,
        slot: u8,
        live: bool,
    },
    /// Block of the permanent allocation-root pool (ADR-035). The live tree
    /// nodes inside it carry a `VolumeTreeNode` role as well.
    AllocationRootPool,
    /// Slot `index` of the permanent intent-log area (ADR-037).
    IntentLogSlot {
        index: u16,
    },
    /// Any other block outside the allocatable range.
    Reserved,
    /// Reachable node of a volume-wide tree with owner zero.
    VolumeTreeNode {
        kind: VolumeTree,
        level: u8,
    },
    ObjectRecord {
        object_id: u64,
    },
    DirectoryNode {
        object_id: u64,
        level: u8,
    },
    ExtentNode {
        object_id: u64,
        level: u8,
    },
    /// File data: `logical_block` of `object_id`.
    Data {
        object_id: u64,
        logical_block: u64,
        shared: bool,
        unwritten: bool,
    },
    /// Segment `index` of the security descriptor chain of `object_id`.
    SecuritySegment {
        object_id: u64,
        index: u16,
    },
    ReclaimRoot,
    ReclaimTable,
    ReclaimSegment,
    /// Retired and waiting in the reclaim queue.
    Quarantined {
        retire_generation: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VolumeTree {
    ObjectMap,
    AllocationRoot,
    SharedExtents,
    SnapshotRegistry,
    SnapshotLifetimes,
}

/// The allocation bit of a block, where a bitmap covers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allocation {
    /// Reserved area: no bitmap bit exists.
    Reserved,
    Allocated,
    Free,
}

/// Self-description read from the block itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockIdentity {
    pub magic: [u8; 4],
    pub owner: u64,
    pub generation: u64,
    pub checksum_valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockExplanation {
    pub lba: u64,
    pub allocation: Allocation,
    /// Every role the committed state gives the block. More than one only
    /// for shared file data. Empty with `Allocation::Allocated` means the
    /// block is allocated and owned by nothing the live state reaches.
    pub roles: Vec<BlockRole>,
    /// Present when the block starts with a known metadata magic.
    pub identity: Option<BlockIdentity>,
}

impl BlockExplanation {
    /// Allocated, and unreachable from the live committed state.
    pub fn is_unowned(&self) -> bool {
        self.allocation == Allocation::Allocated && self.roles.is_empty()
    }
}

/// What the committed state says about the data of one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataSummary {
    pub extents: u64,
    pub mapped_blocks: u64,
    pub shared_blocks: u64,
    pub unwritten_blocks: u64,
}

/// What the committed state says about one object's security descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuritySummary {
    pub total_len: u32,
    pub segments_expected: u16,
    /// Segments consistent with the reference, from the first one.
    pub segments_found: u16,
    /// Format identity and version of the first consistent segment.
    pub format: Option<(u32, u16)>,
    pub projection_diverged: bool,
}

/// What the committed state believes about one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectExplanation {
    pub object_id: u64,
    pub object_type: ObjectType,
    pub record_block: u64,
    pub link_count: u32,
    pub size_bytes: u64,
    pub allocated_bytes: u64,
    pub protection: u32,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
    pub content_generation: u64,
    pub extent_tree: bool,
    pub data_in_place: bool,
    /// Empty when the object has none.
    pub comment: String,
    /// Target of a symlink.
    pub symlink_target: Option<String>,
    /// Nodes of the object's own tree: directory tree or extent tree.
    pub tree_nodes: u64,
    /// Entries of a directory.
    pub entries: u64,
    pub data: DataSummary,
    pub security: Option<SecuritySummary>,
    /// Every directory entry that names the object: parent object and name.
    /// Empty for the root, and for an object no directory reaches.
    pub names: Vec<(u64, String)>,
}

/// One resolved path, component by component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathExplanation {
    /// Each component with the object it names, from the root downwards.
    pub components: Vec<(String, u64)>,
    pub object: ObjectExplanation,
}

/// The ownership picture of one committed state.
pub struct Explainer {
    pub generation: u64,
    pub total_blocks: u64,
    /// The volume keeps snapshot trees; blocks retained only by a historical
    /// view appear as allocated without a live role.
    pub has_snapshots: bool,
    /// Branches the walk could not decode.
    pub problems: Vec<String>,
    pub root_object_id: u64,
    roles: BTreeMap<u64, Vec<BlockRole>>,
    bitmaps: BTreeMap<(u32, u32), BitmapPage>,
    ident: Identification,
    objects: BTreeMap<u64, ObjectExplanation>,
    /// Directory object -> original name -> child object.
    children: BTreeMap<u64, BTreeMap<Vec<u8>, u64>>,
}

struct Walk<'a, D: BlockDevice> {
    dev: &'a mut D,
    buf: Vec<u8>,
    max_generation: u64,
    roles: BTreeMap<u64, Vec<BlockRole>>,
    problems: Vec<String>,
    objects: BTreeMap<u64, ObjectExplanation>,
    children: BTreeMap<u64, BTreeMap<Vec<u8>, u64>>,
    /// Tree nodes accepted so far, over every tree.
    tree_nodes: u64,
}

impl<D: BlockDevice> Walk<'_, D> {
    fn read(&mut self, lba: u64, what: &str) -> bool {
        if lba >= self.dev.total_blocks() {
            self.problems
                .push(format!("{what}: block {lba} outside the device"));
            return false;
        }
        match self.dev.read_block(lba, &mut self.buf) {
            Ok(()) => true,
            Err(error) => {
                self.problems.push(format!("{what}: block {lba}: {error}"));
                false
            }
        }
    }

    fn add(&mut self, lba: u64, role: BlockRole) {
        self.roles.entry(lba).or_default().push(role);
    }

    /// Descend one tree, tagging each node through `node_role`, and return
    /// its leaf items in key order.
    fn tree(
        &mut self,
        root: u64,
        kind: TreeKind,
        owner: u64,
        what: &str,
        node_role: &dyn Fn(u8) -> BlockRole,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut leaves = Vec::new();
        let mut stack = vec![root];
        let mut visited = 0u64;
        while let Some(lba) = stack.pop() {
            visited += 1;
            if visited > self.dev.total_blocks() {
                self.problems
                    .push(format!("{what}: tree walk does not end"));
                break;
            }
            if !self.read(lba, what) {
                continue;
            }
            let node = match TreeNode::decode(&self.buf) {
                Ok((node, generation))
                    if node.kind == kind
                        && node.owner == owner
                        && generation != 0
                        && generation <= self.max_generation =>
                {
                    node
                }
                Ok(_) => {
                    self.problems
                        .push(format!("{what}: block {lba} is a foreign tree node"));
                    continue;
                }
                Err(error) => {
                    self.problems.push(format!("{what}: block {lba}: {error}"));
                    continue;
                }
            };
            self.add(lba, node_role(node.level));
            self.tree_nodes += 1;
            if node.is_leaf() {
                leaves.extend(node.items.into_iter().map(|item| (item.key, item.value)));
                continue;
            }
            // Children are pushed in reverse so leaves come out in key order.
            let mut children = Vec::with_capacity(node.items.len() + 1);
            children.push(node.leftmost_child);
            for item in &node.items {
                match TreeNode::child_ref(item) {
                    Ok(child) => children.push(child.lba),
                    Err(error) => self.problems.push(format!("{what}: block {lba}: {error}")),
                }
            }
            stack.extend(children.into_iter().rev());
        }
        leaves
    }
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

impl Explainer {
    /// Walk the committed state of `dev`. Fails only when no committed state
    /// can be selected at all.
    pub fn load<D: BlockDevice>(dev: &mut D) -> Result<Explainer, String> {
        let block_size = dev.block_size();
        let mut buf = vec![0u8; block_size];
        dev.read_block(0, &mut buf).map_err(|e| e.to_string())?;
        let ident = Identification::decode(&buf).map_err(|e| e.to_string())?;
        let geo = ident.geometry();

        // Checkpoint selection: the structurally valid slot with the highest
        // generation; equal generations are ambiguous.
        let mut slots: [Option<Checkpoint>; 2] = [None, None];
        for (index, slot) in slots.iter_mut().enumerate() {
            if dev.read_block(1 + index as u64, &mut buf).is_ok() {
                *slot = Checkpoint::decode(&buf, &ident.uuid)
                    .ok()
                    .filter(|checkpoint| checkpoint.validate_structural(&geo).is_ok());
            }
        }
        let selected = match (&slots[0], &slots[1]) {
            (Some(a), Some(b)) if a.generation == b.generation => {
                return Err(format!(
                    "both checkpoint slots carry generation {}",
                    a.generation
                ))
            }
            (Some(a), Some(b)) => usize::from(b.generation > a.generation),
            (Some(_), None) => 0,
            (None, Some(_)) => 1,
            (None, None) => return Err("no structurally valid checkpoint".into()),
        };
        let checkpoint = slots[selected].clone().expect("selected slot is present");

        let mut walk = Walk {
            dev,
            buf,
            max_generation: checkpoint.generation,
            roles: BTreeMap::new(),
            problems: Vec::new(),
            objects: BTreeMap::new(),
            children: BTreeMap::new(),
            tree_nodes: 0,
        };
        walk.add(0, BlockRole::Identification);
        for slot in 0..2u8 {
            walk.add(
                1 + u64::from(slot),
                BlockRole::CheckpointSlot {
                    slot,
                    selected: usize::from(slot) == selected,
                },
            );
        }

        // Allocation state: root tree -> live descriptor slot -> live bitmap
        // slots. Every other reserved slot is named with `live: false`.
        let mut bitmaps = BTreeMap::new();
        let mut live_descriptor = BTreeMap::new();
        let mut live_bitmap = BTreeMap::new();
        let records = walk.tree(
            checkpoint.allocation_root_block,
            TreeKind::AllocationRoot,
            0,
            "allocation root",
            &|level| BlockRole::VolumeTreeNode {
                kind: VolumeTree::AllocationRoot,
                level,
            },
        );
        for (key, value) in records {
            let (Ok(region), Some(&slot)) = (
                <[u8; 4]>::try_from(key.as_slice()).map(u32::from_be_bytes),
                value.first(),
            ) else {
                walk.problems
                    .push("allocation root: malformed record".into());
                continue;
            };
            if region >= geo.region_count() || slot > 2 {
                walk.problems
                    .push(format!("allocation root: region {region} slot {slot}"));
                continue;
            }
            live_descriptor.insert(region, slot);
            let lba = geo.descriptor_slot_lba(region, slot);
            if !walk.read(lba, "region descriptor") {
                continue;
            }
            match RegionDescriptor::decode(&walk.buf) {
                Ok((descriptor, _)) if descriptor.region == region => {
                    for (page, binding) in descriptor.pages.iter().enumerate() {
                        let page = page as u32;
                        if binding.slot > 2 {
                            continue;
                        }
                        live_bitmap.insert((region, page), binding.slot);
                        let lba = geo.bitmap_slot_lba(region, page, binding.slot);
                        if walk.read(lba, "bitmap page") {
                            match BitmapPage::decode(&walk.buf) {
                                Ok((bitmap, _)) => {
                                    bitmaps.insert((region, page), bitmap);
                                }
                                Err(error) => walk
                                    .problems
                                    .push(format!("bitmap page {region}/{page}: {error}")),
                            }
                        }
                    }
                }
                Ok(_) => walk
                    .problems
                    .push(format!("region descriptor {region}: wrong region")),
                Err(error) => walk
                    .problems
                    .push(format!("region descriptor {region}: {error}")),
            }
        }
        for region in 0..geo.region_count() {
            for slot in 0..3u8 {
                walk.add(
                    geo.descriptor_slot_lba(region, slot),
                    BlockRole::RegionDescriptorSlot {
                        region,
                        slot,
                        live: live_descriptor.get(&region) == Some(&slot),
                    },
                );
                for page in 0..geo.bitmap_page_count(region) {
                    walk.add(
                        geo.bitmap_slot_lba(region, page, slot),
                        BlockRole::BitmapSlot {
                            region,
                            page,
                            slot,
                            live: live_bitmap.get(&(region, page)) == Some(&slot),
                        },
                    );
                }
            }
        }

        // The two permanently allocated areas.
        match geo.allocation_root_pool_lbas() {
            Ok(pool) => {
                for lba in pool {
                    walk.add(lba, BlockRole::AllocationRootPool);
                }
            }
            Err(error) => walk.problems.push(format!("allocation-root pool: {error}")),
        }
        match geo.intent_log_slot_lbas(ident.log_slots) {
            Ok(slots) => {
                for (index, lba) in slots.into_iter().enumerate() {
                    walk.add(
                        lba,
                        BlockRole::IntentLogSlot {
                            index: index as u16,
                        },
                    );
                }
            }
            Err(error) => walk.problems.push(format!("intent-log area: {error}")),
        }

        // Object map, then every object it names.
        let objects = walk.tree(
            checkpoint.object_map_block,
            TreeKind::ObjectMap,
            0,
            "object map",
            &|level| BlockRole::VolumeTreeNode {
                kind: VolumeTree::ObjectMap,
                level,
            },
        );
        for (key, value) in objects {
            let (Ok(object_id), Some(record_lba)) = (
                <[u8; 8]>::try_from(key.as_slice()).map(u64::from_be_bytes),
                u64_at(&value, 0),
            ) else {
                walk.problems.push("object map: malformed entry".into());
                continue;
            };
            Self::object(&mut walk, object_id, record_lba);
        }

        if checkpoint.shared_extent_root_block != 0 {
            walk.tree(
                checkpoint.shared_extent_root_block,
                TreeKind::SharedExtents,
                0,
                "shared-extent tree",
                &|level| BlockRole::VolumeTreeNode {
                    kind: VolumeTree::SharedExtents,
                    level,
                },
            );
        }
        if let Some(roots) = checkpoint.snapshot_roots {
            for (root, kind, tree) in [
                (
                    roots.registry,
                    TreeKind::SnapshotRegistry,
                    VolumeTree::SnapshotRegistry,
                ),
                (
                    roots.lifetimes,
                    TreeKind::SnapshotLifetimes,
                    VolumeTree::SnapshotLifetimes,
                ),
            ] {
                walk.tree(root, kind, 0, "snapshot tree", &|level| {
                    BlockRole::VolumeTreeNode { kind: tree, level }
                });
            }
        }

        Self::reclaim_queue(&mut walk, checkpoint.reclaim_root_block);

        let Walk {
            roles,
            problems,
            mut objects,
            children,
            ..
        } = walk;
        for (parent, entries) in &children {
            for (name, child) in entries {
                if let Some(object) = objects.get_mut(child) {
                    object
                        .names
                        .push((*parent, String::from_utf8_lossy(name).into_owned()));
                }
            }
        }
        Ok(Explainer {
            generation: checkpoint.generation,
            total_blocks: ident.total_blocks,
            has_snapshots: checkpoint.snapshot_roots.is_some(),
            problems,
            root_object_id: checkpoint.root_object_id,
            roles,
            bitmaps,
            ident,
            objects,
            children,
        })
    }

    fn object<D: BlockDevice>(walk: &mut Walk<'_, D>, object_id: u64, record_lba: u64) {
        let what = format!("object {object_id}");
        if !walk.read(record_lba, &what) {
            return;
        }
        let record = match ObjectRecord::decode_metadata_with_generation(&walk.buf) {
            Ok((record, _)) if record.object_id == object_id => record,
            Ok(_) => {
                walk.problems.push(format!(
                    "{what}: record block {record_lba} names another object"
                ));
                return;
            }
            Err(error) => {
                walk.problems.push(format!("{what}: {error}"));
                return;
            }
        };
        walk.add(record_lba, BlockRole::ObjectRecord { object_id });
        let symlink_target = (record.object_type == ObjectType::Symlink)
            .then(|| SymlinkRecord::decode(&walk.buf).ok())
            .flatten()
            .map(|(link, _)| link.target.to_owned());
        let nodes_before = walk.tree_nodes;
        let mut entries = 0u64;
        let mut data = DataSummary::default();

        match record.object_type {
            ObjectType::Directory => {
                let leaves = walk.tree(
                    record.data_root,
                    TreeKind::Directory,
                    object_id,
                    &what,
                    &|level| BlockRole::DirectoryNode { object_id, level },
                );
                for (key, value) in leaves {
                    match afsplus_format::dir::decode_tree_entry_value(&key, &value) {
                        Ok(entry) => {
                            entries += 1;
                            walk.children
                                .entry(object_id)
                                .or_default()
                                .insert(entry.name, entry.child_id);
                        }
                        Err(error) => walk.problems.push(format!("{what}: {error}")),
                    }
                }
            }
            ObjectType::File if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 => {
                let extents = walk.tree(
                    record.data_root,
                    TreeKind::ExtentMap,
                    object_id,
                    &what,
                    &|level| BlockRole::ExtentNode { object_id, level },
                );
                for (key, value) in extents {
                    let item = match ExtentItem::decode(&key, &value) {
                        Ok(item) if item.block_count <= walk.dev.total_blocks() => item,
                        Ok(_) => {
                            walk.problems.push(format!("{what}: extent too long"));
                            continue;
                        }
                        Err(error) => {
                            walk.problems.push(format!("{what}: {error}"));
                            continue;
                        }
                    };
                    data.extents += 1;
                    data.mapped_blocks += item.block_count;
                    if item.flags & EXTENT_SHARED != 0 {
                        data.shared_blocks += item.block_count;
                    }
                    if item.flags & EXTENT_UNWRITTEN != 0 {
                        data.unwritten_blocks += item.block_count;
                    }
                    for offset in 0..item.block_count {
                        walk.add(
                            item.physical_start + offset,
                            BlockRole::Data {
                                object_id,
                                logical_block: item.logical_start + offset,
                                shared: item.flags & EXTENT_SHARED != 0,
                                unwritten: item.flags & EXTENT_UNWRITTEN != 0,
                            },
                        );
                    }
                }
            }
            ObjectType::File => {
                data.extents = u64::from(record.data_blocks > 0);
                data.mapped_blocks = record.data_blocks;
                for offset in 0..record.data_blocks {
                    walk.add(
                        record.data_root + offset,
                        BlockRole::Data {
                            object_id,
                            logical_block: offset,
                            shared: false,
                            unwritten: false,
                        },
                    );
                }
            }
            ObjectType::Symlink | ObjectType::Internal => {}
        }
        let tree_nodes = walk.tree_nodes - nodes_before;

        // Descriptor chain: one segment at a time, up to the first segment
        // that does not prove it belongs to this object.
        let mut security = record.security.map(|reference| SecuritySummary {
            total_len: reference.total_len,
            segments_expected: reference.segment_count,
            segments_found: 0,
            format: None,
            projection_diverged: reference.flags & SECURITY_REF_PROJECTION_DIVERGED != 0,
        });
        if let Some(reference) = record.security {
            let mut lba = reference.first_block;
            // One commit writes a whole chain (ADR-105).
            let mut chain_generation = None;
            for index in 0..reference.segment_count {
                if !walk.read(lba, &what) {
                    break;
                }
                let next = match SecuritySegment::decode(&walk.buf) {
                    Ok((segment, generation))
                        if segment.object_id == object_id
                            && segment.index == index
                            && segment.count == reference.segment_count
                            && segment.total_len == reference.total_len
                            && generation != 0
                            && generation <= walk.max_generation
                            && *chain_generation.get_or_insert(generation) == generation =>
                    {
                        if let Some(summary) = security.as_mut() {
                            summary.segments_found += 1;
                            summary
                                .format
                                .get_or_insert((segment.format, segment.version));
                        }
                        Some(segment.next)
                    }
                    _ => None,
                };
                match next {
                    Some(next) => {
                        walk.add(lba, BlockRole::SecuritySegment { object_id, index });
                        lba = next;
                    }
                    None => {
                        walk.problems.push(format!(
                            "{what}: security segment {index} at block {lba} is not provable"
                        ));
                        break;
                    }
                }
            }
        }
        walk.objects.insert(
            object_id,
            ObjectExplanation {
                object_id,
                object_type: record.object_type,
                record_block: record_lba,
                link_count: record.link_count,
                size_bytes: record.size_bytes,
                allocated_bytes: record.allocated_bytes,
                protection: record.protection,
                created: record.created,
                modified: record.modified,
                changed: record.changed,
                content_generation: record.content_generation,
                extent_tree: record.flags & OBJECT_FLAG_EXTENT_TREE != 0,
                data_in_place: record.flags & OBJECT_FLAG_DATA_IN_PLACE != 0,
                comment: record.comment.as_str().to_owned(),
                symlink_target,
                tree_nodes,
                entries,
                data,
                security,
                names: Vec::new(),
            },
        );
    }

    /// What the committed state believes about `object_id`.
    pub fn explain_object(&self, object_id: u64) -> Result<&ObjectExplanation, String> {
        self.objects
            .get(&object_id)
            .ok_or_else(|| format!("object {object_id} is not in the committed object map"))
    }

    /// Resolve `path` from the root directory, one `/`-separated component
    /// at a time, by the original spelling of each name. The walk reads the
    /// format alone, so it does not apply a volume's case folding: a path is
    /// found under the spelling the directory stores.
    pub fn explain_path(&self, path: &str) -> Result<PathExplanation, String> {
        let mut current = self.root_object_id;
        let mut components = Vec::new();
        for component in path.split('/').filter(|part| !part.is_empty()) {
            let directory = self
                .children
                .get(&current)
                .ok_or_else(|| format!("{component:?}: object {current} has no entries"))?;
            current = *directory
                .get(component.as_bytes())
                .ok_or_else(|| format!("{component:?} is not an entry of object {current}"))?;
            components.push((component.to_owned(), current));
        }
        Ok(PathExplanation {
            components,
            object: self.explain_object(current)?.clone(),
        })
    }

    fn reclaim_queue<D: BlockDevice>(walk: &mut Walk<'_, D>, root_lba: u64) {
        if !walk.read(root_lba, "reclaim root") {
            return;
        }
        let root = match ReclaimRoot::decode(&walk.buf) {
            Ok((root, _)) => root,
            Err(error) => {
                walk.problems.push(format!("reclaim root: {error}"));
                return;
            }
        };
        walk.add(root_lba, BlockRole::ReclaimRoot);

        // FIFO order: tables (oldest), then sealed segments, then the inline
        // entries. The cursor consumes the head segment of the head table,
        // or of the segment list when no table exists.
        let mut segments = Vec::new();
        for (table_index, table_ref) in root.table_refs.iter().enumerate() {
            if !walk.read(table_ref.lba, "reclaim table") {
                continue;
            }
            match ReclaimTable::decode(&walk.buf) {
                Ok((table, _)) => {
                    walk.add(table_ref.lba, BlockRole::ReclaimTable);
                    let skip = if table_index == 0 {
                        root.head_segment_offset as usize
                    } else {
                        0
                    };
                    segments.extend(table.refs.into_iter().skip(skip));
                }
                Err(error) => walk.problems.push(format!("reclaim table: {error}")),
            }
        }
        segments.extend(root.segment_refs.iter().copied());

        let mut entries: Vec<ReclaimEntry> = Vec::new();
        for (segment_index, segment_ref) in segments.iter().enumerate() {
            if !walk.read(segment_ref.lba, "reclaim segment") {
                continue;
            }
            match ReclaimSegment::decode(&walk.buf) {
                Ok((segment, _)) => {
                    walk.add(segment_ref.lba, BlockRole::ReclaimSegment);
                    let skip = if segment_index == 0 {
                        root.head_entry_offset as usize
                    } else {
                        0
                    };
                    let mut pending = segment.entries.into_iter().skip(skip);
                    if segment_index == 0 {
                        if let Some(mut head) = pending.next() {
                            let consumed = root.head_block_offset.min(head.blocks);
                            head.start += u64::from(consumed);
                            head.blocks -= consumed;
                            entries.push(head);
                        }
                    }
                    entries.extend(pending);
                }
                Err(error) => walk.problems.push(format!("reclaim segment: {error}")),
            }
        }
        entries.extend(root.inline_entries.iter().copied());
        for entry in entries {
            for offset in 0..u64::from(entry.blocks) {
                walk.add(
                    entry.start + offset,
                    BlockRole::Quarantined {
                        retire_generation: entry.retire_generation,
                    },
                );
            }
        }
    }

    /// The allocation bit of `lba` in the live bitmaps.
    pub fn allocation(&self, lba: u64) -> Allocation {
        let geo = self.ident.geometry();
        if !geo.is_allocatable(lba) {
            return Allocation::Reserved;
        }
        let region = geo.region_of(lba);
        let local = (lba - geo.region_base(region)) as u32;
        let (page, local_index) = geo.bitmap_page_for_index(local);
        match self.bitmaps.get(&(region, page)) {
            Some(bitmap) => {
                if bitmap.is_allocated(local_index) {
                    Allocation::Allocated
                } else {
                    Allocation::Free
                }
            }
            // A bitmap the walk could not read proves nothing; report the
            // block as allocated so nothing is ever called free on a guess.
            _ => Allocation::Allocated,
        }
    }

    /// What the committed state believes about `lba`, plus what the block
    /// says about itself.
    pub fn explain_block<D: BlockDevice>(
        &self,
        dev: &mut D,
        lba: u64,
    ) -> Result<BlockExplanation, String> {
        if lba >= self.total_blocks {
            return Err(format!(
                "block {lba} is outside the volume of {} blocks",
                self.total_blocks
            ));
        }
        let mut roles = self.roles.get(&lba).cloned().unwrap_or_default();
        let allocation = self.allocation(lba);
        if roles.is_empty() && allocation == Allocation::Reserved {
            roles.push(BlockRole::Reserved);
        }
        roles.sort();
        let mut buf = vec![0u8; dev.block_size()];
        dev.read_block(lba, &mut buf).map_err(|e| e.to_string())?;
        let magic: [u8; 4] = buf[0..4].try_into().expect("four bytes");
        let known = [
            block_type::IDENTIFICATION,
            block_type::CHECKPOINT,
            block_type::OBJECT,
            block_type::DIRECTORY,
            block_type::OBJECT_MAP,
            block_type::BITMAP,
            block_type::REGION_DESCRIPTOR,
            block_type::TREE_NODE,
            block_type::RETIRED,
            block_type::RECLAIM_ROOT,
            block_type::RECLAIM_SEGMENT,
            block_type::RECLAIM_TABLE,
            block_type::INTENT_LOG,
            block_type::SECURITY_DESCRIPTOR,
        ];
        let identity = known
            .contains(&u32::from_le_bytes(magic))
            .then(|| BlockIdentity {
                magic,
                owner: u64::from_le_bytes(buf[8..16].try_into().expect("eight bytes")),
                generation: u64::from_le_bytes(buf[16..24].try_into().expect("eight bytes")),
                checksum_valid: BlockHeader::verify(&buf, u32::from_le_bytes(magic)).is_ok(),
            });
        Ok(BlockExplanation {
            lba,
            allocation,
            roles,
            identity,
        })
    }
}
