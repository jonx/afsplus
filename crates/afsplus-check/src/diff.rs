//! Semantic image diff: what two committed states differ by in filesystem
//! terms (`docs/28-virtual-images-and-viewports.md` section 11).
//!
//! The diff answers "what did this operation really do to the volume": the
//! objects that appeared, disappeared, were renamed or moved, the metadata
//! fields that changed, including the stored comment (ADR-106), the logical byte ranges whose content changed, the
//! allocation changes that changed no content, and the volume-wide facts
//! (label, generation, free space, quarantine, orphans, snapshots).
//!
//! Like [`crate::explain`], the diff is its own reader. It selects a
//! checkpoint and descends every tree with the block codecs of
//! `afsplus-format` alone, sharing no traversal, claim set or loader with
//! the checker or with the core, so a disagreement with either is a finding
//! about one of them. It never writes and never repairs: a structure it
//! cannot decode ends that branch and is reported in [`ImageDiff::problems`],
//! and what is behind it stays uncompared.
//!
//! Memory bound: every tree is read through a cursor that holds one leaf
//! node and the pending child LBAs of the path above it, so a directory, an
//! object map or an extent map of any size costs one block of items and
//! `O(fan-out * depth)` block numbers per image, never the whole tree. File
//! content is compared one logical block per side at a time. What the diff
//! accumulates is the report itself, which is proportional to the number of
//! changes, not to the size of the volume.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use afsplus_block::BlockDevice;
use afsplus_format::attrs::{
    decode_attribute_set, ATTRIBUTE_CHAIN, ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION,
};
use afsplus_format::chain::{ChainKind, ChainSegment};
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::extent::{ExtentItem, EXTENT_SHARED, EXTENT_UNWRITTEN};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType, SymlinkRecord, OBJECT_FLAG_EXTENT_TREE};
use afsplus_format::reclaim::{ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable};
use afsplus_format::security::SECURITY_CHAIN;
use afsplus_format::snapshot::{decode_key, RegistryState, SnapshotRecord};
use afsplus_format::tree::{TreeKind, TreeNode};
use afsplus_format::{Timespec, OBJECT_ORPHAN_DIRECTORY};

/// Versioned structured-output schema of the diff (ADR-025).
pub const DIFF_SCHEMA_VERSION: u32 = 3;

/// What the diff compares.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    /// Skip file content comparison. Metadata, names, allocation summaries
    /// and volume facts are still compared.
    pub metadata_only: bool,
}

/// A half-open range of logical bytes of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteRange {
    pub start: u64,
    /// Exclusive.
    pub end: u64,
}

/// What an object's extent map allocates, independent of its content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AllocationSummary {
    /// Logical blocks the map resolves to a physical block.
    pub mapped_blocks: u64,
    /// Of those, blocks reached through an extent marked shared.
    pub shared_blocks: u64,
    /// Of those, blocks reached through an extent marked unwritten.
    pub unwritten_blocks: u64,
}

/// The security descriptor reference of one object record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityState {
    pub format: u32,
    pub version: u16,
    pub total_len: u32,
    /// The classic projection was edited without evaluating the descriptor.
    pub diverged: bool,
}

/// Longest attribute value reported byte for byte. A longer value is
/// reported by its length and the SHA-256 of its bytes, so a change is always
/// provable without the report carrying 64 KiB of payload.
pub const ATTRIBUTE_VALUE_INLINE_BYTES: usize = 256;

/// One value of one attribute, on one side of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeValue {
    pub len: usize,
    /// The bytes, when the value is at most
    /// [`ATTRIBUTE_VALUE_INLINE_BYTES`] long.
    pub bytes: Option<Vec<u8>>,
    /// Lowercase hexadecimal SHA-256 of the bytes, when they are longer.
    pub digest: Option<String>,
}

impl AttributeValue {
    fn of(bytes: &[u8]) -> AttributeValue {
        if bytes.len() <= ATTRIBUTE_VALUE_INLINE_BYTES {
            AttributeValue {
                len: bytes.len(),
                bytes: Some(bytes.to_vec()),
                digest: None,
            }
        } else {
            AttributeValue {
                len: bytes.len(),
                bytes: None,
                digest: Some(hex(&Sha256::digest(bytes))),
            }
        }
    }
}

/// One extended attribute that the two states disagree about. `from` absent
/// is an attribute added, `to` absent one removed, both present a value
/// changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeChange {
    pub name: String,
    pub from: Option<AttributeValue>,
    pub to: Option<AttributeValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampField {
    Created,
    Modified,
    Changed,
}

impl TimestampField {
    fn name(self) -> &'static str {
        match self {
            TimestampField::Created => "created",
            TimestampField::Modified => "modified",
            TimestampField::Changed => "changed",
        }
    }
}

/// One field of an object that the two states disagree about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldChange {
    Type {
        from: &'static str,
        to: &'static str,
    },
    Size {
        from: u64,
        to: u64,
    },
    AllocatedBytes {
        from: u64,
        to: u64,
    },
    LinkCount {
        from: u32,
        to: u32,
    },
    Protection {
        from: u32,
        to: u32,
    },
    Timestamp {
        field: TimestampField,
        from: Timespec,
        to: Timespec,
    },
    ContentGeneration {
        from: u64,
        to: u64,
    },
    SymlinkTarget {
        from: String,
        to: String,
    },
    /// The stored comment of the object (ADR-106). The empty string is the
    /// absent comment, so adding, changing and clearing one are the same
    /// change with different ends.
    Comment {
        from: String,
        to: String,
    },
    /// Presence, format identity, length or divergence mark of the security
    /// descriptor, or descriptor bytes of the same length and format.
    Security {
        from: Option<SecurityState>,
        to: Option<SecurityState>,
        bytes_changed: bool,
    },
    /// Extended attributes added, removed or given another value (ADR-108),
    /// in name order.
    Attributes {
        changes: Vec<AttributeChange>,
    },
    /// Logical byte ranges whose content differs. A byte past a file's
    /// logical size is absent; absent differs from any present byte and
    /// equals another absent byte, so truncation and extension appear here.
    Content {
        ranges: Vec<ByteRange>,
    },
    /// The extent map changed without changing any content byte: a clone, a
    /// preallocation, a copy-on-write rewrite of identical data.
    Allocation {
        from: AllocationSummary,
        to: AllocationSummary,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectSummary {
    pub object_type: &'static str,
    pub size_bytes: u64,
    pub link_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectChange {
    Created(ObjectSummary),
    Removed(ObjectSummary),
    Modified(Vec<FieldChange>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectDiff {
    pub object_id: u64,
    pub change: ObjectChange,
}

/// One directory entry that exists on one side only, or names another
/// object on the two sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkChange {
    Added {
        parent: u64,
        name: String,
        child: u64,
    },
    Removed {
        parent: u64,
        name: String,
        child: u64,
    },
    /// Same parent and name, different object.
    Retargeted {
        parent: u64,
        name: String,
        from: u64,
        to: u64,
    },
}

impl LinkChange {
    fn sort_key(&self) -> (u64, &str, u8) {
        match self {
            LinkChange::Added { parent, name, .. } => (*parent, name.as_str(), 0),
            LinkChange::Removed { parent, name, .. } => (*parent, name.as_str(), 1),
            LinkChange::Retargeted { parent, name, .. } => (*parent, name.as_str(), 2),
        }
    }
}

/// An object whose single name moved: the same object ID under another
/// parent, another name, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub object_id: u64,
    pub from_parent: u64,
    pub from_name: String,
    pub to_parent: u64,
    pub to_name: String,
}

/// An object that entered or left the orphan directory (ADR-041).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrphanChange {
    pub object_id: u64,
    /// True when the second state holds the orphan.
    pub added: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotChange {
    Created {
        id: u64,
        generation: u64,
        object_map_root: u64,
    },
    Removed {
        id: u64,
        generation: u64,
    },
    Modified {
        id: u64,
        from_object_map_root: u64,
        to_object_map_root: u64,
    },
}

impl SnapshotChange {
    fn id(&self) -> u64 {
        match self {
            SnapshotChange::Created { id, .. }
            | SnapshotChange::Removed { id, .. }
            | SnapshotChange::Modified { id, .. } => *id,
        }
    }
}

/// Volume-wide facts of the two committed states. Every field is
/// `(first, second)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeDiff {
    pub generation: (u64, u64),
    /// The committed label lives in the checkpoint (ADR-104).
    pub label: (String, String),
    pub free_blocks: (u64, u64),
    pub root_object_id: (u64, u64),
    pub next_object_id: (u64, u64),
    /// Blocks and runs waiting in the reclaim queue (ADR-036).
    pub quarantine_blocks: (u64, u64),
    pub quarantine_runs: (u64, u64),
    /// Next snapshot ID of the registry control record; zero without a
    /// registry.
    pub snapshot_next_id: (u64, u64),
    /// The two images are different volumes, so object IDs and block
    /// numbers do not necessarily mean the same thing on both sides.
    pub different_volume: bool,
}

impl VolumeDiff {
    pub fn free_blocks_delta(&self) -> i128 {
        i128::from(self.free_blocks.1) - i128::from(self.free_blocks.0)
    }

    pub fn is_empty(&self) -> bool {
        !self.different_volume
            && self.generation.0 == self.generation.1
            && self.label.0 == self.label.1
            && self.free_blocks.0 == self.free_blocks.1
            && self.root_object_id.0 == self.root_object_id.1
            && self.next_object_id.0 == self.next_object_id.1
            && self.quarantine_blocks.0 == self.quarantine_blocks.1
            && self.quarantine_runs.0 == self.quarantine_runs.1
            && self.snapshot_next_id.0 == self.snapshot_next_id.1
    }
}

/// The semantic difference between two committed states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDiff {
    pub schema_version: u32,
    pub metadata_only: bool,
    pub volume: VolumeDiff,
    /// By object ID.
    pub objects: Vec<ObjectDiff>,
    /// By parent, then name.
    pub links: Vec<LinkChange>,
    /// By object ID.
    pub renames: Vec<Rename>,
    /// By object ID.
    pub orphans: Vec<OrphanChange>,
    /// By snapshot ID.
    pub snapshots: Vec<SnapshotChange>,
    /// Branches neither state could be read far enough to compare. A
    /// non-empty list means the diff below it is partial, and says so.
    pub problems: Vec<String>,
}

impl ImageDiff {
    /// No difference and nothing left uncompared.
    pub fn is_empty(&self) -> bool {
        self.volume.is_empty()
            && self.objects.is_empty()
            && self.links.is_empty()
            && self.renames.is_empty()
            && self.orphans.is_empty()
            && self.snapshots.is_empty()
            && self.problems.is_empty()
    }

    /// The reported difference is partial.
    pub fn has_problems(&self) -> bool {
        !self.problems.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Reading one image
// ---------------------------------------------------------------------------

/// A tree read one leaf at a time, with one item of lookahead.
struct Stream {
    kind: TreeKind,
    owner: u64,
    what: String,
    stack: Vec<u64>,
    items: std::vec::IntoIter<(Vec<u8>, Vec<u8>)>,
    peeked: Option<(Vec<u8>, Vec<u8>)>,
    visited: u64,
    /// A node of this tree could not be read or decoded, so keys it held are
    /// missing from the stream and nothing one-sided may be concluded from
    /// it any more.
    broken: bool,
}

impl Stream {
    fn new(root: u64, kind: TreeKind, owner: u64, what: String) -> Stream {
        Stream {
            kind,
            owner,
            what,
            stack: vec![root],
            items: Vec::new().into_iter(),
            peeked: None,
            visited: 0,
            broken: false,
        }
    }

    fn empty(kind: TreeKind, owner: u64) -> Stream {
        Stream {
            kind,
            owner,
            what: String::new(),
            stack: Vec::new(),
            items: Vec::new().into_iter(),
            peeked: None,
            visited: 0,
            broken: false,
        }
    }
}

/// The content of one owned chain, or the fact that it could not be proven.
enum ChainRead {
    Content {
        format: u32,
        version: u16,
        bytes: Vec<u8>,
    },
    Unreadable,
}

/// What one state holds for a field read through a chain: nothing, a value,
/// or damage. Damage is never read as absence.
enum Read<T> {
    Absent,
    Present(T),
    Unreadable,
}

type AttributeSet = Vec<(String, Vec<u8>)>;

struct Image<'a, D: BlockDevice> {
    dev: &'a mut D,
    side: &'static str,
    ident: Identification,
    checkpoint: Checkpoint,
    buf: Vec<u8>,
    problems: Vec<String>,
}

impl<'a, D: BlockDevice> Image<'a, D> {
    /// Select the committed state exactly as mount does: the structurally
    /// valid slot with the highest generation; equal generations are
    /// ambiguous.
    fn open(dev: &'a mut D, side: &'static str) -> Result<Image<'a, D>, String> {
        let mut buf = vec![0u8; dev.block_size()];
        dev.read_block(0, &mut buf)
            .map_err(|error| format!("{side}: identification block: {error}"))?;
        let ident = Identification::decode(&buf)
            .map_err(|error| format!("{side}: identification block: {error}"))?;
        let geo = ident.geometry();
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
                    "{side}: both checkpoint slots carry generation {}",
                    a.generation
                ))
            }
            (Some(a), Some(b)) => usize::from(b.generation > a.generation),
            (Some(_), None) => 0,
            (None, Some(_)) => 1,
            (None, None) => return Err(format!("{side}: no structurally valid checkpoint")),
        };
        let checkpoint = slots[selected].clone().expect("selected slot is present");
        Ok(Image {
            dev,
            side,
            ident,
            checkpoint,
            buf,
            problems: Vec::new(),
        })
    }

    fn problem(&mut self, what: &str) {
        let side = self.side;
        self.problems.push(format!("{side}: {what}"));
    }

    fn read(&mut self, lba: u64, what: &str) -> bool {
        if lba >= self.dev.total_blocks() {
            self.problem(&format!("{what}: block {lba} outside the device"));
            return false;
        }
        let mut buf = std::mem::take(&mut self.buf);
        let ok = match self.dev.read_block(lba, &mut buf) {
            Ok(()) => true,
            Err(error) => {
                self.problem(&format!("{what}: block {lba}: {error}"));
                false
            }
        };
        self.buf = buf;
        ok
    }

    /// The next leaf item of `stream` in key order, or `None` at the end of
    /// the tree or of a branch that could not be decoded.
    fn advance(&mut self, stream: &mut Stream) -> Option<(Vec<u8>, Vec<u8>)> {
        loop {
            if let Some(item) = stream.items.next() {
                return Some(item);
            }
            let lba = stream.stack.pop()?;
            stream.visited += 1;
            if stream.visited > self.dev.total_blocks() {
                let what = stream.what.clone();
                self.problem(&format!("{what}: tree walk does not end"));
                stream.broken = true;
                stream.stack.clear();
                return None;
            }
            if !self.read(lba, &stream.what.clone()) {
                stream.broken = true;
                continue;
            }
            let max_generation = self.checkpoint.generation;
            let node = match TreeNode::decode(&self.buf) {
                Ok((node, generation))
                    if node.kind == stream.kind
                        && node.owner == stream.owner
                        && generation != 0
                        && generation <= max_generation =>
                {
                    node
                }
                Ok(_) => {
                    let what = stream.what.clone();
                    self.problem(&format!("{what}: block {lba} is a foreign tree node"));
                    stream.broken = true;
                    continue;
                }
                Err(error) => {
                    let what = stream.what.clone();
                    self.problem(&format!("{what}: block {lba}: {error}"));
                    stream.broken = true;
                    continue;
                }
            };
            if node.is_leaf() {
                stream.items = node
                    .items
                    .into_iter()
                    .map(|item| (item.key.into_vec(), item.value.into_vec()))
                    .collect::<Vec<_>>()
                    .into_iter();
                continue;
            }
            // Children are pushed in reverse so leaves come out in key order.
            let mut children = Vec::with_capacity(node.items.len() + 1);
            children.push(node.leftmost_child);
            for item in &node.items {
                match TreeNode::child_ref(item) {
                    Ok(child) => children.push(child.lba),
                    Err(error) => {
                        let what = stream.what.clone();
                        self.problem(&format!("{what}: block {lba}: {error}"));
                        stream.broken = true;
                    }
                }
            }
            stream.stack.extend(children.into_iter().rev());
        }
    }

    fn peek(&mut self, stream: &mut Stream) -> Option<(Vec<u8>, Vec<u8>)> {
        if stream.peeked.is_none() {
            stream.peeked = self.advance(stream);
        }
        stream.peeked.clone()
    }

    fn take(&mut self, stream: &mut Stream) -> Option<(Vec<u8>, Vec<u8>)> {
        self.peek(stream);
        stream.peeked.take()
    }

    fn object_map(&mut self) -> Stream {
        Stream::new(
            self.checkpoint.object_map_block,
            TreeKind::ObjectMap,
            0,
            "object map".into(),
        )
    }

    fn directory(&mut self, object_id: u64, root: u64) -> Stream {
        Stream::new(
            root,
            TreeKind::Directory,
            object_id,
            format!("directory {object_id}"),
        )
    }

    fn snapshot_registry(&mut self) -> Stream {
        match self.checkpoint.snapshot_roots {
            Some(roots) => Stream::new(
                roots.registry,
                TreeKind::SnapshotRegistry,
                0,
                "snapshot registry".into(),
            ),
            None => Stream::empty(TreeKind::SnapshotRegistry, 0),
        }
    }

    /// The object record of `object_id`, or `None` when the record cannot be
    /// proven to belong to it.
    fn record(
        &mut self,
        object_id: u64,
        record_lba: u64,
    ) -> Option<(ObjectRecord, Option<String>)> {
        let what = format!("object {object_id}");
        if !self.read(record_lba, &what) {
            return None;
        }
        let (record, _) = match ObjectRecord::decode_metadata_with_generation(&self.buf) {
            Ok(decoded) if decoded.0.object_id == object_id => decoded,
            Ok(_) => {
                self.problem(&format!(
                    "{what}: record block {record_lba} names another object"
                ));
                return None;
            }
            Err(error) => {
                self.problem(&format!("{what}: {error}"));
                return None;
            }
        };
        let target = if record.object_type == ObjectType::Symlink {
            match SymlinkRecord::decode(&self.buf) {
                Ok((link, _)) => Some(link.target.to_string()),
                Err(error) => {
                    self.problem(&format!("{what}: symlink target: {error}"));
                    None
                }
            }
        } else {
            None
        };
        Some((record, target))
    }

    /// Read one owned chain of `object_id` (`afsplus_format::chain`): the
    /// same walk for the security descriptor and for the attribute set, and
    /// for every kind added later. Bounded by the kind's content bound.
    fn chain(
        &mut self,
        object_id: u64,
        kind: &ChainKind,
        first_block: u64,
        total_len: u32,
        segment_count: u16,
    ) -> ChainRead {
        let what = format!("object {object_id}");
        let label = kind.label;
        let mut lba = first_block;
        let mut bytes = Vec::new();
        let mut head: Option<(u32, u16)> = None;
        let mut chain_generation: Option<u64> = None;
        for index in 0..segment_count {
            if !self.read(lba, &what) {
                return ChainRead::Unreadable;
            }
            match ChainSegment::decode(kind, &self.buf) {
                Ok((segment, generation))
                    if segment.object_id == object_id
                        && segment.index == index
                        && segment.count == segment_count
                        && segment.total_len == total_len
                        && generation != 0
                        && generation <= self.checkpoint.generation
                        && *chain_generation.get_or_insert(generation) == generation =>
                {
                    if index == 0 {
                        head = Some((segment.format, segment.version));
                    }
                    bytes.extend_from_slice(segment.bytes);
                    lba = segment.next;
                }
                _ => {
                    self.problem(&format!(
                        "{what}: {label} segment {index} at block {lba} is not provable"
                    ));
                    return ChainRead::Unreadable;
                }
            }
        }
        match head {
            Some((format, version)) => ChainRead::Content {
                format,
                version,
                bytes,
            },
            None => ChainRead::Unreadable,
        }
    }

    /// The security descriptor of `object_id`, with its bytes.
    fn security(
        &mut self,
        object_id: u64,
        record: &ObjectRecord,
    ) -> Read<(SecurityState, Vec<u8>)> {
        let Some(reference) = record.security else {
            return Read::Absent;
        };
        match self.chain(
            object_id,
            &SECURITY_CHAIN,
            reference.first_block,
            reference.total_len,
            reference.segment_count,
        ) {
            ChainRead::Unreadable => Read::Unreadable,
            ChainRead::Content {
                format,
                version,
                bytes,
            } => Read::Present((
                SecurityState {
                    format,
                    version,
                    total_len: reference.total_len,
                    diverged: reference.flags
                        & afsplus_format::object::SECURITY_REF_PROJECTION_DIVERGED
                        != 0,
                },
                bytes,
            )),
        }
    }

    /// The whole attribute set of `object_id`, in stored order, which is
    /// ascending by name bytes. Metadata: read in every mode, bounded by
    /// `MAX_ATTRIBUTE_SET_BYTES` per object.
    fn attributes(&mut self, object_id: u64, record: &ObjectRecord) -> Read<AttributeSet> {
        let Some(reference) = record.attributes else {
            return Read::Absent;
        };
        let bytes = match self.chain(
            object_id,
            &ATTRIBUTE_CHAIN,
            reference.first_block,
            reference.total_len,
            reference.segment_count,
        ) {
            ChainRead::Unreadable => return Read::Unreadable,
            ChainRead::Content {
                format,
                version,
                bytes,
            } => {
                if (format, version) != (ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION) {
                    self.problem(&format!(
                        "object {object_id}: attribute set format {format} version {version}"
                    ));
                    return Read::Unreadable;
                }
                bytes
            }
        };
        match decode_attribute_set(&bytes) {
            Ok(entries) => Read::Present(
                entries
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), value.to_vec()))
                    .collect(),
            ),
            Err(error) => {
                self.problem(&format!("object {object_id}: attribute set: {error}"));
                Read::Unreadable
            }
        }
    }

    /// Blocks and runs the reclaim queue holds, reconstructed in FIFO order
    /// from the queue structures alone.
    fn quarantine(&mut self) -> (u64, u64) {
        let root_lba = self.checkpoint.reclaim_root_block;
        if !self.read(root_lba, "reclaim root") {
            return (0, 0);
        }
        let root = match ReclaimRoot::decode(&self.buf) {
            Ok((root, _)) => root,
            Err(error) => {
                self.problem(&format!("reclaim root: {error}"));
                return (0, 0);
            }
        };
        let mut segments = Vec::new();
        for (table_index, table_ref) in root.table_refs.iter().enumerate() {
            if !self.read(table_ref.lba, "reclaim table") {
                continue;
            }
            match ReclaimTable::decode(&self.buf) {
                Ok((table, _)) => {
                    let skip = if table_index == 0 {
                        root.head_segment_offset as usize
                    } else {
                        0
                    };
                    segments.extend(table.refs.into_iter().skip(skip));
                }
                Err(error) => self.problem(&format!("reclaim table: {error}")),
            }
        }
        segments.extend(root.segment_refs.iter().copied());

        let mut entries: Vec<ReclaimEntry> = Vec::new();
        for (segment_index, segment_ref) in segments.iter().enumerate() {
            if !self.read(segment_ref.lba, "reclaim segment") {
                continue;
            }
            match ReclaimSegment::decode(&self.buf) {
                Ok((segment, _)) => {
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
                Err(error) => self.problem(&format!("reclaim segment: {error}")),
            }
        }
        entries.extend(root.inline_entries.iter().copied());
        let runs = entries.iter().filter(|entry| entry.blocks > 0).count() as u64;
        let blocks = entries
            .iter()
            .map(|entry| u64::from(entry.blocks))
            .sum::<u64>();
        (blocks, runs)
    }
}

// ---------------------------------------------------------------------------
// Directory entries and extent maps
// ---------------------------------------------------------------------------

/// Directory leaf value: name length, child type hint, child object ID, then
/// the original UTF-8 name (`docs/05-directories-and-names.md`).
fn decode_entry(value: &[u8]) -> Option<(u64, String)> {
    // The key is not needed to read the child and the name.
    let entry = afsplus_format::dir::decode_tree_entry_value(&[], value).ok()?;
    Some((
        entry.child_id,
        String::from_utf8_lossy(&entry.name).into_owned(),
    ))
}

/// The logical-to-physical map of one file, read forward only.
struct DataMap {
    /// `Some((start, blocks))` for a file that carries one direct extent.
    direct: Option<(u64, u64)>,
    stream: Stream,
    current: Option<ExtentItem>,
    summary: AllocationSummary,
    drained: bool,
}

impl DataMap {
    fn new(object_id: u64, record: &ObjectRecord) -> DataMap {
        if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            DataMap {
                direct: None,
                stream: Stream::new(
                    record.data_root,
                    TreeKind::ExtentMap,
                    object_id,
                    format!("object {object_id} extent map"),
                ),
                current: None,
                summary: AllocationSummary::default(),
                drained: false,
            }
        } else {
            let blocks = record.data_blocks;
            DataMap {
                direct: Some((record.data_root, blocks)),
                stream: Stream::empty(TreeKind::ExtentMap, object_id),
                current: None,
                summary: AllocationSummary {
                    mapped_blocks: blocks,
                    shared_blocks: 0,
                    unwritten_blocks: 0,
                },
                drained: true,
            }
        }
    }

    fn account(&mut self, item: &ExtentItem) {
        self.summary.mapped_blocks += item.block_count;
        if item.flags & EXTENT_SHARED != 0 {
            self.summary.shared_blocks += item.block_count;
        }
        if item.flags & EXTENT_UNWRITTEN != 0 {
            self.summary.unwritten_blocks += item.block_count;
        }
    }

    /// Where logical block `logical` lives, with the flags of its extent.
    /// `logical` must not go backwards.
    fn mapping<D: BlockDevice>(
        &mut self,
        image: &mut Image<'_, D>,
        logical: u64,
    ) -> Option<(u64, u32)> {
        if let Some((start, blocks)) = self.direct {
            return (logical < blocks).then_some((start + logical, 0));
        }
        while let Some((key, value)) = image.peek(&mut self.stream) {
            match ExtentItem::decode(&key, &value) {
                Ok(item) => {
                    if item.logical_start > logical {
                        break;
                    }
                    image.take(&mut self.stream);
                    self.account(&item);
                    self.current = Some(item);
                }
                Err(error) => {
                    image.take(&mut self.stream);
                    image.problem(&format!("extent map: {error}"));
                }
            }
        }
        match self.current {
            Some(item)
                if logical >= item.logical_start
                    && logical - item.logical_start < item.block_count =>
            {
                Some((
                    item.physical_start + (logical - item.logical_start),
                    item.flags,
                ))
            }
            _ => None,
        }
    }

    /// Account for the extents past the last block compared, so that a
    /// preallocation beyond the end of the file is still reported.
    fn drain<D: BlockDevice>(&mut self, image: &mut Image<'_, D>) -> AllocationSummary {
        if !self.drained {
            while let Some((key, value)) = image.take(&mut self.stream) {
                match ExtentItem::decode(&key, &value) {
                    Ok(item) => self.account(&item),
                    Err(error) => image.problem(&format!("extent map: {error}")),
                }
            }
            self.drained = true;
        }
        self.summary
    }
}

/// Reads one logical block of a file. A hole and an unwritten extent both
/// read as zeros, which is what the file returns.
fn read_logical<D: BlockDevice>(
    image: &mut Image<'_, D>,
    mapping: Option<(u64, u32)>,
    into: &mut [u8],
) {
    into.fill(0);
    let Some((physical, flags)) = mapping else {
        return;
    };
    if flags & EXTENT_UNWRITTEN != 0 {
        return;
    }
    if physical >= image.dev.total_blocks() {
        image.problem(&format!("data block {physical} outside the device"));
        return;
    }
    if let Err(error) = image.dev.read_block(physical, into) {
        image.problem(&format!("data block {physical}: {error}"));
        into.fill(0);
    }
}

// ---------------------------------------------------------------------------
// The diff itself
// ---------------------------------------------------------------------------

/// Compares the committed states of two images.
///
/// Fails only when one of the two has no committed state to compare at all.
/// Anything that cannot be read below that is reported in
/// [`ImageDiff::problems`] and leaves the branch behind it uncompared.
pub fn diff_devices<A: BlockDevice, B: BlockDevice>(
    first: &mut A,
    second: &mut B,
    options: DiffOptions,
) -> Result<ImageDiff, String> {
    let mut a = Image::open(first, "first")?;
    let mut b = Image::open(second, "second")?;
    if a.ident.block_size() != b.ident.block_size() {
        return Err(format!(
            "block sizes differ: {} and {}",
            a.ident.block_size(),
            b.ident.block_size()
        ));
    }
    let block_size = a.ident.block_size();

    let mut objects = Vec::new();
    let mut links = Vec::new();
    let mut orphans = Vec::new();

    // Objects, in object-ID order, and with them the directory entries of
    // every directory either state holds.
    let mut sa = a.object_map();
    let mut sb = b.object_map();
    loop {
        let left = a.peek(&mut sa);
        let right = b.peek(&mut sb);
        let (key, take_left, take_right) = match (&left, &right) {
            (None, None) => break,
            (Some((key, _)), None) => (key.clone(), true, false),
            (None, Some((key, _))) => (key.clone(), false, true),
            (Some((ka, _)), Some((kb, _))) => match ka.cmp(kb) {
                std::cmp::Ordering::Less => (ka.clone(), true, false),
                std::cmp::Ordering::Greater => (kb.clone(), false, true),
                std::cmp::Ordering::Equal => (ka.clone(), true, true),
            },
        };
        let entry_a = take_left.then(|| a.take(&mut sa)).flatten();
        let entry_b = take_right.then(|| b.take(&mut sb)).flatten();
        let Ok(object_id) = <[u8; 8]>::try_from(key.as_slice()).map(u64::from_be_bytes) else {
            a.problem("object map: malformed key");
            continue;
        };
        let present_a = entry_a.is_some();
        let present_b = entry_b.is_some();
        let record_a = match entry_a.and_then(|(_, value)| lba_of(&value)) {
            Some(lba) => a.record(object_id, lba),
            None => {
                if present_a {
                    a.problem(&format!("object map: entry {object_id} is malformed"));
                }
                None
            }
        };
        let record_b = match entry_b.and_then(|(_, value)| lba_of(&value)) {
            Some(lba) => b.record(object_id, lba),
            None => {
                if present_b {
                    b.problem(&format!("object map: entry {object_id} is malformed"));
                }
                None
            }
        };
        // An object whose record one state names but cannot prove is
        // uncompared, never reported as created or removed: the problem
        // above says the diff is partial there.
        if (present_a && record_a.is_none()) || (present_b && record_b.is_none()) {
            continue;
        }
        // A damaged object map hides objects. What is missing from a broken
        // stream is uncompared, never reported as created or removed.
        if (!present_a && sa.broken) || (!present_b && sb.broken) {
            let side = if present_a { "second" } else { "first" };
            a.problems.push(format!(
                "object {object_id}: not compared, the object map of the {side} image is damaged"
            ));
            continue;
        }
        compare_object(
            &mut a,
            &mut b,
            object_id,
            record_a,
            record_b,
            block_size,
            options,
            &mut objects,
            &mut links,
            &mut orphans,
        );
    }

    // Snapshot registry.
    let mut snapshots = Vec::new();
    let mut next_id = (0u64, 0u64);
    let mut ra = a.snapshot_registry();
    let mut rb = b.snapshot_registry();
    loop {
        let left = a.peek(&mut ra);
        let right = b.peek(&mut rb);
        let (key, take_left, take_right) = match (&left, &right) {
            (None, None) => break,
            (Some((key, _)), None) => (key.clone(), true, false),
            (None, Some((key, _))) => (key.clone(), false, true),
            (Some((ka, _)), Some((kb, _))) => match ka.cmp(kb) {
                std::cmp::Ordering::Less => (ka.clone(), true, false),
                std::cmp::Ordering::Greater => (kb.clone(), false, true),
                std::cmp::Ordering::Equal => (ka.clone(), true, true),
            },
        };
        let value_a = take_left
            .then(|| a.take(&mut ra))
            .flatten()
            .map(|(_, value)| value);
        let value_b = take_right
            .then(|| b.take(&mut rb))
            .flatten()
            .map(|(_, value)| value);
        let Ok(id) = decode_key(&key) else {
            a.problem("snapshot registry: malformed key");
            continue;
        };
        if id == 0 {
            if let Some(value) = &value_a {
                next_id.0 = RegistryState::decode(value).map(|s| s.next_id).unwrap_or(0);
            }
            if let Some(value) = &value_b {
                next_id.1 = RegistryState::decode(value).map(|s| s.next_id).unwrap_or(0);
            }
            continue;
        }
        let (max_a, blocks_a) = (a.checkpoint.generation, a.ident.total_blocks);
        let (max_b, blocks_b) = (b.checkpoint.generation, b.ident.total_blocks);
        let snapshot_a =
            value_a.and_then(|value| decode_snapshot(&mut a, id, &value, max_a, blocks_a));
        let snapshot_b =
            value_b.and_then(|value| decode_snapshot(&mut b, id, &value, max_b, blocks_b));
        match (snapshot_a, snapshot_b) {
            (None, Some(record)) => snapshots.push(SnapshotChange::Created {
                id,
                generation: record.generation,
                object_map_root: record.object_map_root,
            }),
            (Some(record), None) => snapshots.push(SnapshotChange::Removed {
                id,
                generation: record.generation,
            }),
            (Some(x), Some(y)) if x != y => snapshots.push(SnapshotChange::Modified {
                id,
                from_object_map_root: x.object_map_root,
                to_object_map_root: y.object_map_root,
            }),
            _ => {}
        }
    }

    let quarantine_a = a.quarantine();
    let quarantine_b = b.quarantine();
    let volume = VolumeDiff {
        generation: (a.checkpoint.generation, b.checkpoint.generation),
        label: (a.checkpoint.label.clone(), b.checkpoint.label.clone()),
        free_blocks: (
            a.checkpoint.free_blocks_total,
            b.checkpoint.free_blocks_total,
        ),
        root_object_id: (a.checkpoint.root_object_id, b.checkpoint.root_object_id),
        next_object_id: (a.checkpoint.next_object_id, b.checkpoint.next_object_id),
        quarantine_blocks: (quarantine_a.0, quarantine_b.0),
        quarantine_runs: (quarantine_a.1, quarantine_b.1),
        snapshot_next_id: next_id,
        different_volume: a.ident.uuid != b.ident.uuid,
    };

    let mut problems = a.problems.clone();
    problems.extend(b.problems.clone());

    objects.sort_by_key(|object| object.object_id);
    orphans.sort_by_key(|orphan| (orphan.object_id, orphan.added));
    snapshots.sort_by_key(|change| change.id());
    let renames = derive_renames(&mut links);
    links.sort_by(|x, y| x.sort_key().cmp(&y.sort_key()));

    Ok(ImageDiff {
        schema_version: DIFF_SCHEMA_VERSION,
        metadata_only: options.metadata_only,
        volume,
        objects,
        links,
        renames,
        orphans,
        snapshots,
        problems,
    })
}

fn lba_of(value: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(value.get(0..8)?.try_into().ok()?))
}

fn decode_snapshot<D: BlockDevice>(
    image: &mut Image<'_, D>,
    id: u64,
    value: &[u8],
    max_generation: u64,
    total_blocks: u64,
) -> Option<SnapshotRecord> {
    match SnapshotRecord::decode(value, max_generation, total_blocks) {
        Ok(record) => Some(record),
        Err(error) => {
            image.problem(&format!("snapshot {id}: {error}"));
            None
        }
    }
}

fn type_name(object_type: ObjectType) -> &'static str {
    match object_type {
        ObjectType::File => "file",
        ObjectType::Directory => "directory",
        ObjectType::Symlink => "symlink",
        ObjectType::Internal => "internal",
    }
}

fn summary(record: &ObjectRecord) -> ObjectSummary {
    ObjectSummary {
        object_type: type_name(record.object_type),
        size_bytes: record.size_bytes,
        link_count: record.link_count,
    }
}

type Loaded = (ObjectRecord, Option<String>);

#[allow(clippy::too_many_arguments)]
fn compare_object<A: BlockDevice, B: BlockDevice>(
    a: &mut Image<'_, A>,
    b: &mut Image<'_, B>,
    object_id: u64,
    record_a: Option<Loaded>,
    record_b: Option<Loaded>,
    block_size: usize,
    options: DiffOptions,
    objects: &mut Vec<ObjectDiff>,
    links: &mut Vec<LinkChange>,
    orphans: &mut Vec<OrphanChange>,
) {
    match (&record_a, &record_b) {
        (None, None) => return,
        (Some((record, _)), None) => objects.push(ObjectDiff {
            object_id,
            change: ObjectChange::Removed(summary(record)),
        }),
        (None, Some((record, _))) => objects.push(ObjectDiff {
            object_id,
            change: ObjectChange::Created(summary(record)),
        }),
        (Some((x, target_a)), Some((y, target_b))) => {
            let mut fields = Vec::new();
            if x.object_type != y.object_type {
                fields.push(FieldChange::Type {
                    from: type_name(x.object_type),
                    to: type_name(y.object_type),
                });
            }
            if x.size_bytes != y.size_bytes {
                fields.push(FieldChange::Size {
                    from: x.size_bytes,
                    to: y.size_bytes,
                });
            }
            if x.allocated_bytes != y.allocated_bytes {
                fields.push(FieldChange::AllocatedBytes {
                    from: x.allocated_bytes,
                    to: y.allocated_bytes,
                });
            }
            if x.link_count != y.link_count {
                fields.push(FieldChange::LinkCount {
                    from: x.link_count,
                    to: y.link_count,
                });
            }
            if x.protection != y.protection {
                fields.push(FieldChange::Protection {
                    from: x.protection,
                    to: y.protection,
                });
            }
            for (field, from, to) in [
                (TimestampField::Created, x.created, y.created),
                (TimestampField::Modified, x.modified, y.modified),
                (TimestampField::Changed, x.changed, y.changed),
            ] {
                if from != to {
                    fields.push(FieldChange::Timestamp { field, from, to });
                }
            }
            if x.content_generation != y.content_generation {
                fields.push(FieldChange::ContentGeneration {
                    from: x.content_generation,
                    to: y.content_generation,
                });
            }
            if target_a != target_b {
                fields.push(FieldChange::SymlinkTarget {
                    from: target_a.clone().unwrap_or_default(),
                    to: target_b.clone().unwrap_or_default(),
                });
            }
            if x.comment != y.comment {
                fields.push(FieldChange::Comment {
                    from: x.comment.as_str().to_owned(),
                    to: y.comment.as_str().to_owned(),
                });
            }
            let security_a = a.security(object_id, x);
            let security_b = b.security(object_id, y);
            // A chain that could not be proven is not an absent one: the
            // problem stands and the field stays uncompared.
            if !matches!(security_a, Read::Unreadable) && !matches!(security_b, Read::Unreadable) {
                let state_a = match &security_a {
                    Read::Present((state, _)) => Some(*state),
                    _ => None,
                };
                let state_b = match &security_b {
                    Read::Present((state, _)) => Some(*state),
                    _ => None,
                };
                let bytes_changed = match (&security_a, &security_b) {
                    (Read::Present((_, bytes_a)), Read::Present((_, bytes_b))) => {
                        bytes_a != bytes_b
                    }
                    _ => false,
                };
                if state_a != state_b || bytes_changed {
                    fields.push(FieldChange::Security {
                        from: state_a,
                        to: state_b,
                        bytes_changed,
                    });
                }
            }
            let attributes_a = a.attributes(object_id, x);
            let attributes_b = b.attributes(object_id, y);
            if !matches!(attributes_a, Read::Unreadable)
                && !matches!(attributes_b, Read::Unreadable)
            {
                let empty = Vec::new();
                let left = match &attributes_a {
                    Read::Present(set) => set,
                    _ => &empty,
                };
                let right = match &attributes_b {
                    Read::Present(set) => set,
                    _ => &empty,
                };
                let changes = attribute_changes(left, right);
                if !changes.is_empty() {
                    fields.push(FieldChange::Attributes { changes });
                }
            }
            if x.object_type == ObjectType::File && y.object_type == ObjectType::File {
                compare_data(a, b, object_id, x, y, block_size, options, &mut fields);
            }
            if !fields.is_empty() {
                objects.push(ObjectDiff {
                    object_id,
                    change: ObjectChange::Modified(fields),
                });
            }
        }
    }

    // Directory entries of this object, on whichever side holds them.
    let directory_a = record_a
        .as_ref()
        .filter(|(record, _)| record.object_type == ObjectType::Directory)
        .map(|(record, _)| a.directory(object_id, record.data_root));
    let directory_b = record_b
        .as_ref()
        .filter(|(record, _)| record.object_type == ObjectType::Directory)
        .map(|(record, _)| b.directory(object_id, record.data_root));
    if directory_a.is_none() && directory_b.is_none() {
        return;
    }
    let mut da = directory_a.unwrap_or_else(|| Stream::empty(TreeKind::Directory, object_id));
    let mut db = directory_b.unwrap_or_else(|| Stream::empty(TreeKind::Directory, object_id));
    loop {
        let left = a.peek(&mut da);
        let right = b.peek(&mut db);
        let (take_left, take_right) = match (&left, &right) {
            (None, None) => break,
            (Some(_), None) => (true, false),
            (None, Some(_)) => (false, true),
            (Some((ka, _)), Some((kb, _))) => match ka.cmp(kb) {
                std::cmp::Ordering::Less => (true, false),
                std::cmp::Ordering::Greater => (false, true),
                std::cmp::Ordering::Equal => (true, true),
            },
        };
        let entry_a = take_left
            .then(|| a.take(&mut da))
            .flatten()
            .and_then(|(_, value)| decode_entry(&value));
        let entry_b = take_right
            .then(|| b.take(&mut db))
            .flatten()
            .and_then(|(_, value)| decode_entry(&value));
        if (entry_a.is_none() && da.broken) || (entry_b.is_none() && db.broken) {
            a.problems.push(format!(
                "directory {object_id}: an entry is not compared, the directory is damaged"
            ));
            continue;
        }
        match (entry_a, entry_b) {
            (Some((child, _)), None) if object_id == OBJECT_ORPHAN_DIRECTORY => {
                orphans.push(OrphanChange {
                    object_id: child,
                    added: false,
                })
            }
            (None, Some((child, _))) if object_id == OBJECT_ORPHAN_DIRECTORY => {
                orphans.push(OrphanChange {
                    object_id: child,
                    added: true,
                })
            }
            _ if object_id == OBJECT_ORPHAN_DIRECTORY => {}
            (Some((child, name)), None) => links.push(LinkChange::Removed {
                parent: object_id,
                name,
                child,
            }),
            (None, Some((child, name))) => links.push(LinkChange::Added {
                parent: object_id,
                name,
                child,
            }),
            (Some((child_a, name)), Some((child_b, _))) if child_a != child_b => {
                links.push(LinkChange::Retargeted {
                    parent: object_id,
                    name,
                    from: child_a,
                    to: child_b,
                })
            }
            _ => {}
        }
    }
}

/// Merge two stored sets, both ascending by name bytes, into the changes
/// between them, in name order.
fn attribute_changes(
    left: &[(String, Vec<u8>)],
    right: &[(String, Vec<u8>)],
) -> Vec<AttributeChange> {
    let mut changes = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < left.len() || j < right.len() {
        let order = match (left.get(i), right.get(j)) {
            (Some((name_a, _)), Some((name_b, _))) => name_a.as_bytes().cmp(name_b.as_bytes()),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => break,
        };
        match order {
            std::cmp::Ordering::Less => {
                let (name, value) = &left[i];
                changes.push(AttributeChange {
                    name: name.clone(),
                    from: Some(AttributeValue::of(value)),
                    to: None,
                });
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                let (name, value) = &right[j];
                changes.push(AttributeChange {
                    name: name.clone(),
                    from: None,
                    to: Some(AttributeValue::of(value)),
                });
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                let (name, value_a) = &left[i];
                let (_, value_b) = &right[j];
                if value_a != value_b {
                    changes.push(AttributeChange {
                        name: name.clone(),
                        from: Some(AttributeValue::of(value_a)),
                        to: Some(AttributeValue::of(value_b)),
                    });
                }
                i += 1;
                j += 1;
            }
        }
    }
    changes
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn compare_data<A: BlockDevice, B: BlockDevice>(
    a: &mut Image<'_, A>,
    b: &mut Image<'_, B>,
    object_id: u64,
    x: &ObjectRecord,
    y: &ObjectRecord,
    block_size: usize,
    options: DiffOptions,
    fields: &mut Vec<FieldChange>,
) {
    let mut map_a = DataMap::new(object_id, x);
    let mut map_b = DataMap::new(object_id, y);
    let mut ranges: Vec<ByteRange> = Vec::new();

    if !options.metadata_only {
        let size = x.size_bytes.max(y.size_bytes);
        let limit = a
            .ident
            .total_blocks
            .max(b.ident.total_blocks)
            .saturating_mul(block_size as u64);
        if size > limit {
            a.problem(&format!(
                "object {object_id}: logical size {size} exceeds the volume; content not compared"
            ));
        } else {
            let bs = block_size as u64;
            let mut buf_a = vec![0u8; block_size];
            let mut buf_b = vec![0u8; block_size];
            let mut open: Option<ByteRange> = None;
            for logical in 0..size.div_ceil(bs) {
                let base = logical * bs;
                let mapping_a = map_a.mapping(a, logical);
                let mapping_b = map_b.mapping(b, logical);
                read_logical(a, mapping_a, &mut buf_a);
                read_logical(b, mapping_b, &mut buf_b);
                for offset in 0..block_size {
                    let at = base + offset as u64;
                    if at >= size {
                        break;
                    }
                    let byte_a = (at < x.size_bytes).then(|| buf_a[offset]);
                    let byte_b = (at < y.size_bytes).then(|| buf_b[offset]);
                    if byte_a == byte_b {
                        if let Some(range) = open.take() {
                            ranges.push(range);
                        }
                    } else {
                        match &mut open {
                            Some(range) => range.end = at + 1,
                            None => {
                                open = Some(ByteRange {
                                    start: at,
                                    end: at + 1,
                                })
                            }
                        }
                    }
                }
            }
            if let Some(range) = open.take() {
                ranges.push(range);
            }
        }
    }

    let summary_a = map_a.drain(a);
    let summary_b = map_b.drain(b);
    if !ranges.is_empty() {
        fields.push(FieldChange::Content { ranges });
    } else if summary_a != summary_b {
        fields.push(FieldChange::Allocation {
            from: summary_a,
            to: summary_b,
        });
    }
}

/// An object whose only name disappeared on one side and appeared on the
/// other has been renamed or moved, not created and removed.
fn derive_renames(links: &mut Vec<LinkChange>) -> Vec<Rename> {
    let mut removed: BTreeMap<u64, Vec<(u64, String)>> = BTreeMap::new();
    let mut added: BTreeMap<u64, Vec<(u64, String)>> = BTreeMap::new();
    for link in links.iter() {
        match link {
            LinkChange::Removed {
                parent,
                name,
                child,
            } => removed
                .entry(*child)
                .or_default()
                .push((*parent, name.clone())),
            LinkChange::Added {
                parent,
                name,
                child,
            } => added
                .entry(*child)
                .or_default()
                .push((*parent, name.clone())),
            LinkChange::Retargeted { .. } => {}
        }
    }
    let mut renames = Vec::new();
    for (object_id, from) in &removed {
        let Some(to) = added.get(object_id) else {
            continue;
        };
        if from.len() != 1 || to.len() != 1 {
            continue;
        }
        renames.push(Rename {
            object_id: *object_id,
            from_parent: from[0].0,
            from_name: from[0].1.clone(),
            to_parent: to[0].0,
            to_name: to[0].1.clone(),
        });
    }
    let moved: Vec<u64> = renames.iter().map(|rename| rename.object_id).collect();
    links.retain(|link| match link {
        LinkChange::Added { child, .. } | LinkChange::Removed { child, .. } => {
            !moved.contains(child)
        }
        LinkChange::Retargeted { .. } => true,
    });
    renames.sort_by_key(|rename| rename.object_id);
    renames
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_pair_u64(name: &str, pair: (u64, u64)) -> String {
    format!("\"{name}\":{{\"from\":{},\"to\":{}}}", pair.0, pair.1)
}

fn json_time(time: Timespec) -> String {
    format!(
        "{{\"seconds\":{},\"nanoseconds\":{}}}",
        time.seconds, time.nanoseconds
    )
}

fn json_allocation(value: &AllocationSummary) -> String {
    format!(
        "{{\"mapped_blocks\":{},\"shared_blocks\":{},\"unwritten_blocks\":{}}}",
        value.mapped_blocks, value.shared_blocks, value.unwritten_blocks
    )
}

fn json_security(state: &Option<SecurityState>) -> String {
    match state {
        None => "null".into(),
        Some(state) => format!(
            "{{\"format\":{},\"version\":{},\"length\":{},\"diverged\":{}}}",
            state.format, state.version, state.total_len, state.diverged
        ),
    }
}

fn json_attribute_value(value: &Option<AttributeValue>) -> String {
    match value {
        None => "null".into(),
        Some(value) => match (&value.bytes, &value.digest) {
            (Some(bytes), _) => format!(
                "{{\"length\":{},\"hex\":{}}}",
                value.len,
                json_string(&hex(bytes))
            ),
            (None, Some(digest)) => format!(
                "{{\"length\":{},\"sha256\":{}}}",
                value.len,
                json_string(digest)
            ),
            (None, None) => format!("{{\"length\":{}}}", value.len),
        },
    }
}

fn json_field(field: &FieldChange) -> String {
    match field {
        FieldChange::Type { from, to } => format!(
            "{{\"field\":\"type\",\"from\":{},\"to\":{}}}",
            json_string(from),
            json_string(to)
        ),
        FieldChange::Size { from, to } => {
            format!("{{\"field\":\"size\",\"from\":{from},\"to\":{to}}}")
        }
        FieldChange::AllocatedBytes { from, to } => {
            format!("{{\"field\":\"allocated_bytes\",\"from\":{from},\"to\":{to}}}")
        }
        FieldChange::LinkCount { from, to } => {
            format!("{{\"field\":\"link_count\",\"from\":{from},\"to\":{to}}}")
        }
        FieldChange::Protection { from, to } => {
            format!("{{\"field\":\"protection\",\"from\":{from},\"to\":{to}}}")
        }
        FieldChange::Timestamp { field, from, to } => format!(
            "{{\"field\":\"timestamp\",\"which\":{},\"from\":{},\"to\":{}}}",
            json_string(field.name()),
            json_time(*from),
            json_time(*to)
        ),
        FieldChange::ContentGeneration { from, to } => {
            format!("{{\"field\":\"content_generation\",\"from\":{from},\"to\":{to}}}")
        }
        FieldChange::SymlinkTarget { from, to } => format!(
            "{{\"field\":\"symlink_target\",\"from\":{},\"to\":{}}}",
            json_string(from),
            json_string(to)
        ),
        FieldChange::Comment { from, to } => format!(
            "{{\"field\":\"comment\",\"from\":{},\"to\":{}}}",
            json_string(from),
            json_string(to)
        ),
        FieldChange::Security {
            from,
            to,
            bytes_changed,
        } => format!(
            "{{\"field\":\"security\",\"from\":{},\"to\":{},\"bytes_changed\":{}}}",
            json_security(from),
            json_security(to),
            bytes_changed
        ),
        FieldChange::Attributes { changes } => {
            let mut out = String::from("{\"field\":\"attributes\",\"changes\":[");
            for (index, change) in changes.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(
                    out,
                    "{{\"name\":{},\"from\":{},\"to\":{}}}",
                    json_string(&change.name),
                    json_attribute_value(&change.from),
                    json_attribute_value(&change.to)
                );
            }
            out.push_str("]}");
            out
        }
        FieldChange::Content { ranges } => {
            let mut out = String::from("{\"field\":\"content\",\"ranges\":[");
            for (index, range) in ranges.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{{\"start\":{},\"end\":{}}}", range.start, range.end);
            }
            out.push_str("]}");
            out
        }
        FieldChange::Allocation { from, to } => format!(
            "{{\"field\":\"allocation\",\"from\":{},\"to\":{}}}",
            json_allocation(from),
            json_allocation(to)
        ),
    }
}

impl ImageDiff {
    /// The versioned machine-readable form (ADR-025).
    pub fn render_json(&self) -> String {
        let mut out = String::from("{");
        let _ = write!(
            out,
            "\"schema_version\":{},\"metadata_only\":{},\"empty\":{},\"partial\":{},",
            self.schema_version,
            self.metadata_only,
            self.is_empty(),
            self.has_problems()
        );
        let volume = &self.volume;
        let _ = write!(
            out,
            "\"volume\":{{{},\"label\":{{\"from\":{},\"to\":{}}},{},{},{},{},{},{},\
             \"free_blocks_delta\":{},\"different_volume\":{}}},",
            json_pair_u64("generation", volume.generation),
            json_string(&volume.label.0),
            json_string(&volume.label.1),
            json_pair_u64("free_blocks", volume.free_blocks),
            json_pair_u64("root_object_id", volume.root_object_id),
            json_pair_u64("next_object_id", volume.next_object_id),
            json_pair_u64("quarantine_blocks", volume.quarantine_blocks),
            json_pair_u64("quarantine_runs", volume.quarantine_runs),
            json_pair_u64("snapshot_next_id", volume.snapshot_next_id),
            volume.free_blocks_delta(),
            volume.different_volume,
        );

        out.push_str("\"objects\":[");
        for (index, object) in self.objects.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(out, "{{\"object_id\":{},", object.object_id);
            match &object.change {
                ObjectChange::Created(value) | ObjectChange::Removed(value) => {
                    let kind = if matches!(object.change, ObjectChange::Created(_)) {
                        "created"
                    } else {
                        "removed"
                    };
                    let _ = write!(
                        out,
                        "\"change\":\"{kind}\",\"type\":{},\"size\":{},\"link_count\":{}}}",
                        json_string(value.object_type),
                        value.size_bytes,
                        value.link_count
                    );
                }
                ObjectChange::Modified(fields) => {
                    out.push_str("\"change\":\"modified\",\"fields\":[");
                    for (at, field) in fields.iter().enumerate() {
                        if at > 0 {
                            out.push(',');
                        }
                        out.push_str(&json_field(field));
                    }
                    out.push_str("]}");
                }
            }
        }
        out.push_str("],\"links\":[");
        for (index, link) in self.links.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            match link {
                LinkChange::Added {
                    parent,
                    name,
                    child,
                } => {
                    let _ = write!(
                        out,
                        "{{\"change\":\"added\",\"parent\":{parent},\"name\":{},\"child\":{child}}}",
                        json_string(name)
                    );
                }
                LinkChange::Removed {
                    parent,
                    name,
                    child,
                } => {
                    let _ = write!(
                        out,
                        "{{\"change\":\"removed\",\"parent\":{parent},\"name\":{},\"child\":{child}}}",
                        json_string(name)
                    );
                }
                LinkChange::Retargeted {
                    parent,
                    name,
                    from,
                    to,
                } => {
                    let _ = write!(
                        out,
                        "{{\"change\":\"retargeted\",\"parent\":{parent},\"name\":{},\"from\":{from},\"to\":{to}}}",
                        json_string(name)
                    );
                }
            }
        }
        out.push_str("],\"renames\":[");
        for (index, rename) in self.renames.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"object_id\":{},\"from\":{{\"parent\":{},\"name\":{}}},\
                 \"to\":{{\"parent\":{},\"name\":{}}}}}",
                rename.object_id,
                rename.from_parent,
                json_string(&rename.from_name),
                rename.to_parent,
                json_string(&rename.to_name)
            );
        }
        out.push_str("],\"orphans\":[");
        for (index, orphan) in self.orphans.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"object_id\":{},\"change\":\"{}\"}}",
                orphan.object_id,
                if orphan.added { "added" } else { "removed" }
            );
        }
        out.push_str("],\"snapshots\":[");
        for (index, snapshot) in self.snapshots.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            match snapshot {
                SnapshotChange::Created {
                    id,
                    generation,
                    object_map_root,
                } => {
                    let _ = write!(
                        out,
                        "{{\"id\":{id},\"change\":\"created\",\"generation\":{generation},\
                         \"object_map_root\":{object_map_root}}}"
                    );
                }
                SnapshotChange::Removed { id, generation } => {
                    let _ = write!(
                        out,
                        "{{\"id\":{id},\"change\":\"removed\",\"generation\":{generation}}}"
                    );
                }
                SnapshotChange::Modified {
                    id,
                    from_object_map_root,
                    to_object_map_root,
                } => {
                    let _ = write!(
                        out,
                        "{{\"id\":{id},\"change\":\"modified\",\"from_object_map_root\":{from_object_map_root},\
                         \"to_object_map_root\":{to_object_map_root}}}"
                    );
                }
            }
        }
        out.push_str("],\"problems\":[");
        for (index, problem) in self.problems.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&json_string(problem));
        }
        out.push_str("]}");
        out
    }

    /// The short human form: one line per change.
    pub fn render_human(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        let volume = &self.volume;
        if volume.different_volume {
            lines.push("volume: the two images are different volumes".into());
        }
        if volume.generation.0 != volume.generation.1 {
            lines.push(format!(
                "checkpoint {} -> {}",
                volume.generation.0, volume.generation.1
            ));
        }
        if volume.label.0 != volume.label.1 {
            lines.push(format!(
                "label {:?} -> {:?}",
                volume.label.0, volume.label.1
            ));
        }
        if volume.free_blocks.0 != volume.free_blocks.1 {
            lines.push(format!(
                "free blocks {} -> {} ({:+})",
                volume.free_blocks.0,
                volume.free_blocks.1,
                volume.free_blocks_delta()
            ));
        }
        if volume.quarantine_blocks.0 != volume.quarantine_blocks.1
            || volume.quarantine_runs.0 != volume.quarantine_runs.1
        {
            lines.push(format!(
                "reclaim quarantine {} blocks in {} runs -> {} blocks in {} runs",
                volume.quarantine_blocks.0,
                volume.quarantine_runs.0,
                volume.quarantine_blocks.1,
                volume.quarantine_runs.1
            ));
        }
        if volume.next_object_id.0 != volume.next_object_id.1 {
            lines.push(format!(
                "next object ID {} -> {}",
                volume.next_object_id.0, volume.next_object_id.1
            ));
        }
        for rename in &self.renames {
            lines.push(format!(
                "object {} moved {}/{:?} -> {}/{:?}",
                rename.object_id,
                rename.from_parent,
                rename.from_name,
                rename.to_parent,
                rename.to_name
            ));
        }
        for link in &self.links {
            match link {
                LinkChange::Added {
                    parent,
                    name,
                    child,
                } => lines.push(format!("link added {parent}/{name:?} -> object {child}")),
                LinkChange::Removed {
                    parent,
                    name,
                    child,
                } => lines.push(format!("link removed {parent}/{name:?} -> object {child}")),
                LinkChange::Retargeted {
                    parent,
                    name,
                    from,
                    to,
                } => lines.push(format!(
                    "link {parent}/{name:?} object {from} -> object {to}"
                )),
            }
        }
        for object in &self.objects {
            let id = object.object_id;
            match &object.change {
                ObjectChange::Created(value) => lines.push(format!(
                    "object {id} created ({}, {} bytes, {} links)",
                    value.object_type, value.size_bytes, value.link_count
                )),
                ObjectChange::Removed(value) => lines.push(format!(
                    "object {id} removed ({}, {} bytes, {} links)",
                    value.object_type, value.size_bytes, value.link_count
                )),
                ObjectChange::Modified(fields) => {
                    for field in fields {
                        lines.push(format!("object {id} {}", human_field(field)));
                    }
                }
            }
        }
        for orphan in &self.orphans {
            lines.push(format!(
                "orphan {} {}",
                orphan.object_id,
                if orphan.added { "added" } else { "cleared" }
            ));
        }
        for snapshot in &self.snapshots {
            match snapshot {
                SnapshotChange::Created { id, generation, .. } => {
                    lines.push(format!("snapshot {id} created at generation {generation}"))
                }
                SnapshotChange::Removed { id, generation } => {
                    lines.push(format!("snapshot {id} removed (generation {generation})"))
                }
                SnapshotChange::Modified { id, .. } => {
                    lines.push(format!("snapshot {id} object map changed"))
                }
            }
        }
        for problem in &self.problems {
            lines.push(format!("problem: {problem}"));
        }
        if lines.is_empty() {
            lines.push("no difference".into());
        }
        lines.join("\n")
    }
}

fn human_field(field: &FieldChange) -> String {
    match field {
        FieldChange::Type { from, to } => format!("type {from} -> {to}"),
        FieldChange::Size { from, to } => format!("size {from} -> {to}"),
        FieldChange::AllocatedBytes { from, to } => format!("allocated {from} -> {to} bytes"),
        FieldChange::LinkCount { from, to } => format!("link count {from} -> {to}"),
        FieldChange::Protection { from, to } => format!("protection {from:#010x} -> {to:#010x}"),
        FieldChange::Timestamp { field, from, to } => format!(
            "{} time {}.{:09} -> {}.{:09}",
            field.name(),
            from.seconds,
            from.nanoseconds,
            to.seconds,
            to.nanoseconds
        ),
        FieldChange::ContentGeneration { from, to } => {
            format!("content generation {from} -> {to}")
        }
        FieldChange::SymlinkTarget { from, to } => format!("symlink target {from:?} -> {to:?}"),
        FieldChange::Comment { from, to } => match (from.is_empty(), to.is_empty()) {
            (true, false) => format!("comment added {to:?}"),
            (false, true) => format!("comment removed {from:?}"),
            _ => format!("comment {from:?} -> {to:?}"),
        },
        FieldChange::Security {
            from,
            to,
            bytes_changed,
        } => match (from, to) {
            (None, Some(state)) => format!(
                "security descriptor added (format {}, version {}, {} bytes)",
                state.format, state.version, state.total_len
            ),
            (Some(state), None) => format!("security descriptor removed (format {})", state.format),
            (Some(x), Some(y)) => format!(
                "security descriptor format {} -> {}, {} -> {} bytes, diverged {} -> {}{}",
                x.format,
                y.format,
                x.total_len,
                y.total_len,
                x.diverged,
                y.diverged,
                if *bytes_changed { ", bytes differ" } else { "" }
            ),
            (None, None) => "security descriptor unreadable".into(),
        },
        FieldChange::Attributes { changes } => {
            let mut parts = Vec::new();
            for change in changes {
                parts.push(match (&change.from, &change.to) {
                    (None, Some(to)) => {
                        format!("{:?} added ({} bytes)", change.name, to.len)
                    }
                    (Some(from), None) => {
                        format!("{:?} removed ({} bytes)", change.name, from.len)
                    }
                    (Some(from), Some(to)) => format!(
                        "{:?} value {} -> {} bytes",
                        change.name, from.len, to.len
                    ),
                    (None, None) => format!("{:?} unchanged", change.name),
                });
            }
            format!("attribute {}", parts.join("; attribute "))
        }
        FieldChange::Content { ranges } => {
            let shown: Vec<String> = ranges
                .iter()
                .take(8)
                .map(|range| format!("{}..{}", range.start, range.end))
                .collect();
            let more = ranges.len().saturating_sub(shown.len());
            if more > 0 {
                format!(
                    "content changed in {} ranges: {} and {more} more",
                    ranges.len(),
                    shown.join(", ")
                )
            } else {
                format!("content changed in bytes {}", shown.join(", "))
            }
        }
        FieldChange::Allocation { from, to } => format!(
            "allocation {} -> {} blocks mapped, {} -> {} shared, {} -> {} unwritten, content unchanged",
            from.mapped_blocks,
            to.mapped_blocks,
            from.shared_blocks,
            to.shared_blocks,
            from.unwritten_blocks,
            to.unwritten_blocks
        ),
    }
}
