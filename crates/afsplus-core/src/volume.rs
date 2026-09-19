//! Mounted volume: COW transactions over the region allocator.
//!
//! Commit ordering (`docs/08-transactions-and-journal.md` §3):
//!
//! 1. write new user data, barrier (skipped when the transaction has none)
//! 2. write COW metadata and the dirty region bitmap pages, barrier
//! 3. write the alternate checkpoint slot with generation + 1, barrier
//!
//! Every transaction retires the blocks it makes unreachable (replaced
//! object-map paths, old records, old directory paths, old retired list,
//! deleted data) and promotes the previous transaction's retirees; see
//! `alloc`.
//!
//! Per-transaction resource accounting is collected from the start
//! ([`CommitStats`]) — metadata bytes, bitmap pages, region descriptors,
//! flushes, retired and promoted blocks, reclaim latency, allocator RAM.

pub(crate) mod attributes;
pub(crate) use attributes::load_attribute_chain;
pub use attributes::AttributeWriteMode;
mod chain;
mod metadata;
pub use metadata::PreservedMetadata;
mod security;
pub(crate) use security::load_descriptor_chain;
pub use security::{SecurityDescriptor, SecurityProjectionPolicy};
mod snapshots;
use snapshots::SnapshotRegistryChange;
pub use snapshots::{
    SnapshotAllocationPage, SnapshotAllocationRange, SnapshotCommitStats, SnapshotDirectoryCursor,
    SnapshotDirectoryPage, SnapshotHandle, SnapshotInfo, SnapshotListPage, SnapshotMaintenance,
    SnapshotWorkLimits,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::crc32c::{crc32c, Hasher};
use afsplus_format::dir::DirEntry;
use afsplus_format::ident::{
    Identification, COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG_DATA_UPDATES,
    RO_COMPAT_ORPHAN_DIRECTORY, RO_COMPAT_SHARED_EXTENTS,
};
use afsplus_format::intent_log::{LogOp, LogRecord, MAX_LOG_OPS};
use afsplus_format::object::{
    ObjectRecord, ObjectType, MAX_EXTENT_BLOCKS, OBJECT_FLAG_COMMENT, OBJECT_FLAG_DATA_IN_PLACE,
    OBJECT_FLAG_EXTENT_TREE, OBJECT_FLAG_SECURITY_REF,
};
use afsplus_format::{validate_name, FormatError, Timespec, OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};

use crate::alloc::{AllocStats, TxAllocator};
use crate::allocation_root::{self, ReservedTreePool};
use crate::cow_tree::{mutate_many, mutate_new_empty_tree, TreeMutation, TreeOperation};
use crate::directory;
use crate::extent_map::{self, Extent, EXTENT_SHARED, EXTENT_UNWRITTEN};
use crate::intent_log;
use crate::mount::{MountMode, Selection};
use crate::name_key;
use crate::object_map;
use crate::shared_extents::{self, RefEdit, RefEditStats};
use crate::verify::{load_mount_state, MountState};
use crate::CoreError;

/// Semantic byte allocation. Rounded tails and reservations can exceed EOF.
/// Physical addresses and sharing representation are deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileAllocationRange {
    pub offset: u64,
    pub length: u64,
    pub unwritten: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub struct FileAllocationPage {
    pub ranges: Vec<FileAllocationRange>,
    pub next: u64,
    pub eof: bool,
}

/// Measured cost of the last committed transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitStats {
    /// Tree work across the transaction, including early spill writes.
    pub tree_mutations: crate::cow_tree::TreeMutationStats,
    pub data_blocks_written: u64,
    /// Previously written data overwritten at its committed physical address. This is
    /// a subset of `data_blocks_written` and is zero under the default full
    /// COW policy.
    pub data_blocks_overwritten_in_place: u64,
    /// Previously unwritten private blocks initialized before COW publication.
    pub data_blocks_initialized_from_reservation: u64,
    /// Successful metadata write requests, including repeated provisional spills.
    pub metadata_blocks_written: u64,
    pub bitmap_pages_written: u64,
    pub region_descriptors_written: u64,
    /// Allocation-region records changed in the authoritative AFST root.
    pub allocation_records_updated: u64,
    /// COW allocation-root nodes written for those record changes.
    pub allocation_tree_nodes_written: u64,
    pub checkpoint_blocks_written: u64,
    /// Reference-tree nodes written for this transaction (ADR-061).
    pub shared_tree_nodes_written: u64,
    /// Direct-layout files promoted to an extent tree by a clone (ADR-061).
    pub layout_promotions: u64,
    /// Per-block shared-reference accounting (ADR-061).
    pub shared_refs: RefEditStats,
    pub flushes: u64,
    /// Total bytes issued to the device by this transaction.
    pub bytes_written: u64,
    pub alloc: AllocStats,
    pub snapshots: SnapshotCommitStats,
}

pub const DEFAULT_ORPHAN_CLEANUP_EXTENTS: usize = 16;
pub const MIN_ORPHAN_CLEANUP_RECLAIM_BLOCKS: u64 = 16;

/// Runtime-only soft reserve used by normal growth transactions. It scales
/// from 8 blocks (32 KiB with the current format) to 64 blocks and therefore
/// stays useful on classic media without becoming a large-volume partition.
/// Images below 64 blocks are test/minimal geometries and retain the legacy
/// zero-floor behavior because they cannot support the complete lifecycle.
pub const MIN_EMERGENCY_HEADROOM_BLOCKS: u64 = 8;
pub const MAX_EMERGENCY_HEADROOM_BLOCKS: u64 = 64;
pub const EMERGENCY_HEADROOM_SCALE_BLOCKS: u64 = 32;
pub const MIN_HEADROOM_VOLUME_BLOCKS: u64 = 64;

pub fn emergency_headroom_for_volume(total_blocks: u64) -> u64 {
    if total_blocks < MIN_HEADROOM_VOLUME_BLOCKS {
        return 0;
    }
    total_blocks
        .div_ceil(EMERGENCY_HEADROOM_SCALE_BLOCKS)
        .clamp(MIN_EMERGENCY_HEADROOM_BLOCKS, MAX_EMERGENCY_HEADROOM_BLOCKS)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OrphanCleanupProgress {
    pub extents_removed: usize,
    pub object_removed: bool,
    pub still_pending: bool,
}

/// Runtime data-update policy used by the Q1 architecture qualification.
///
/// This does not alter the on-disk format. `InPlacePrivate` is deliberately
/// conservative: only already-written, unshared blocks of a non-extending
/// write are eligible. A hole, unwritten extent, shared marker, or any other
/// uncertainty makes the complete operation fall back to full data COW.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DataUpdatePolicy {
    #[default]
    FullCow,
    InPlacePrivate,
}

pub const MAX_DIRECTORY_PAGE_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryCursor {
    pub generation: u64,
    pub ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPage {
    pub entries: Vec<DirEntry>,
    pub next: DirectoryCursor,
    pub eof: bool,
}

/// Filesystem-facing object metadata. Unlike [`ObjectRecord`], this is a
/// logical view and never exposes an in-flight extent root as if it were an
/// encoded on-disk record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub object_id: u64,
    pub object_type: ObjectType,
    pub size_bytes: u64,
    pub allocated_bytes: u64,
    pub link_count: u32,
    pub protection: u32,
    /// POSIX owner. Zero is root, a real identity, not "unset".
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
    pub content_generation: u64,
}

impl From<ObjectRecord> for ObjectMetadata {
    fn from(record: ObjectRecord) -> Self {
        ObjectMetadata {
            object_id: record.object_id,
            object_type: record.object_type,
            size_bytes: record.size_bytes,
            allocated_bytes: record.allocated_bytes,
            link_count: record.link_count,
            protection: record.protection,
            owner_uid: record.owner_uid,
            owner_gid: record.owner_gid,
            created: record.created,
            modified: record.modified,
            changed: record.changed,
            content_generation: record.content_generation,
        }
    }
}

struct StagedFileLayout {
    record_lba: u64,
    metadata_writes: Vec<(u64, Vec<u8>)>,
}

struct OrphanDataStep {
    extents_removed: usize,
    data_empty: bool,
}

/// One committed file whose logical layout is being edited inside an open
/// intent-log window. The original layout is retained so materialization can
/// publish one COW extent-map mutation no matter how many fsync groups edited
/// the file first.
struct PendingFileLayout {
    record_lba: u64,
    old_extents: Vec<Extent>,
    old_tree_blocks: Vec<u64>,
    extents: Vec<Extent>,
    size_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct WindowAllocation {
    start: u64,
    blocks: u64,
}

/// One operation inside a [`Volume::run_batch`] group commit (ADR-026).
#[derive(Debug, Clone)]
pub enum BatchOp<'a> {
    CreateFile {
        parent_id: u64,
        name: &'a str,
        content: &'a [u8],
    },
    /// An empty directory. The intent log has no record kind for one, so a
    /// window that stages this can only become durable as a checkpoint
    /// (ADR-121 decisions 5 and 8).
    CreateDirectory {
        parent_id: u64,
        name: &'a str,
    },
    DeleteFile {
        parent_id: u64,
        name: &'a str,
    },
    Rename {
        source_parent_id: u64,
        source_name: &'a str,
        target_parent_id: u64,
        target_name: &'a str,
        /// Atomically replace an existing file target in the same batch.
        replace: bool,
    },
}

/// User data passed to the common commit tail. Plain batches borrow surviving
/// caller payloads; other mutation paths own their prepared block images.
/// Neither variant changes the data-before-metadata durability barriers.
enum CommitData<'a> {
    Prepared(Vec<(u64, Vec<u8>)>),
    Borrowed(Vec<(u64, &'a [u8])>),
}

impl CommitData<'_> {
    fn blocks(&self, block_size: usize) -> u64 {
        match self {
            Self::Prepared(writes) => writes.len() as u64,
            Self::Borrowed(runs) => runs
                .iter()
                .map(|(_, bytes)| (bytes.len() as u64).div_ceil(block_size as u64))
                .sum(),
        }
    }

    fn write_to<D: BlockDevice>(self, dev: &mut D) -> Result<(), CoreError> {
        match self {
            Self::Prepared(writes) => {
                for (lba, block) in writes {
                    dev.write_block(lba, &block)?;
                }
            }
            Self::Borrowed(runs) => {
                let block_size = dev.block_size();
                // Full blocks use the caller's bytes directly. Allocate at most
                // one block for zero-padded tails and reuse it across files.
                let mut tail = Vec::new();
                for (start, bytes) in runs {
                    let mut chunks = bytes.chunks_exact(block_size);
                    for (offset, chunk) in chunks.by_ref().enumerate() {
                        dev.write_block(start + offset as u64, chunk)?;
                    }
                    let remainder = chunks.remainder();
                    if !remainder.is_empty() {
                        tail.resize(block_size, 0);
                        tail.fill(0);
                        tail[..remainder.len()].copy_from_slice(remainder);
                        dev.write_block(start + (bytes.len() / block_size) as u64, &tail)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Logical read-your-writes overlay of one in-flight batch.
struct PendingBatch {
    /// Per-directory entry changes: Some = upsert, None = delete.
    dir_changes: BTreeMap<u64, BTreeMap<Vec<u8>, Option<DirEntry>>>,
    /// Timestamp of the last namespace operation touching each directory.
    dir_timestamps: BTreeMap<u64, Timespec>,
    /// Object-record changes: Some = rewrite, None = delete from the map.
    records: BTreeMap<u64, Option<ObjectRecord>>,
    /// Committed record blocks to retire when their object is rewritten.
    committed_record_lbas: BTreeMap<u64, u64>,
    /// Data runs of objects created by this batch (cancellable).
    created_data: BTreeMap<u64, (u64, u64)>,
    /// Directory roots allocated by this transaction and not present in the
    /// committed object map. Their first tree mutation must start from a
    /// staged empty root rather than attempting to read/retire it.
    created_directories: BTreeSet<u64>,
    /// Final logical layouts for committed files edited in this window.
    file_layouts: BTreeMap<u64, PendingFileLayout>,
    /// Still-live data runs allocated by existing-file log operations. A run
    /// removed by a later operation is quarantined because an earlier
    /// record in the window may continue to reference its bytes.
    window_allocations: Vec<WindowAllocation>,
    /// User-data blocks already issued by this write-through window. They
    /// are included in the eventual checkpoint's accounting even though the
    /// commit tail must not write them a second time.
    prewritten_data_blocks: u64,
    /// Windowed batches write data blocks at operation time (covered by the
    /// fsync or metadata barrier); plain batches borrow caller bytes for commit.
    write_through: bool,
    /// Window creates already covered by a durable log record: cancelling
    /// one must not release its blocks (an earlier record's content CRC
    /// still covers them); they are sacrificed to quarantine instead.
    logged_created: BTreeSet<u64>,
    /// Data runs of cancelled logged creates, quarantined at materialize.
    sacrificed: Vec<(u64, u64)>,
    /// This batch contains a final-link orphan transition. Its publication is
    /// destructive/recovery work and may consume the runtime emergency floor;
    /// ordinary window growth remains floor-limited until materialization.
    uses_emergency_headroom: bool,
    next_object_id: u64,
}

/// An open operation window: a live ADR-026 batch whose fsynced prefix is
/// persisted in the intent log (ADR-037).
struct OpenWindow {
    tx: TxAllocator,
    pending: PendingBatch,
    generation: u64,
    unlogged: Vec<LogOp>,
    logged_records: u32,
    /// The window holds a change no log record describes, such as a write
    /// to a file it created: an fsync must commit it, never log it.
    unloggable: bool,
}

/// Largest file created in a window that later writes may still rewrite in
/// place of a commit; beyond it the caller commits the window first.
pub const WINDOW_CREATED_REWRITE_MAX: u64 = 256 * 1024;

/// Explicit reservation transaction admission; independent of disk encoding.
#[derive(Debug, Clone, Copy)]
pub struct FileEditLimits {
    /// Touched logical blocks for writes/reservations; retired allocated blocks
    /// (including a replaced partial tail) for shrinking. Growth retires none.
    pub max_blocks: u64,
    /// Local mapping records, including boundary neighbors and result records.
    pub max_records: usize,
}

pub struct Volume<D: BlockDevice> {
    flight: Option<crate::flight::SharedRecorder>,
    tree_cache_pages: std::num::NonZeroUsize,
    dev: D,
    ident: Identification,
    checkpoint: Checkpoint,
    /// Which checkpoint slot holds the committed state (0 = A, 1 = B).
    current_slot: usize,
    /// The other slot's structurally valid checkpoint, if any: its bitmap
    /// slots and quarantined blocks must stay untouched.
    other_checkpoint: Option<Checkpoint>,
    /// Selection had to skip a newer but unusable checkpoint (see
    /// [`Selection::fell_back_to_older`]). A property of this mount, kept for
    /// the health report; a commit supersedes it.
    fell_back_to_older: bool,
    state: MountState,
    mount_mode: MountMode,
    pending_intent_records: u32,
    /// Per-transaction reclamation budget in blocks (runtime policy).
    reclaim_batch_blocks: u64,
    /// Maximum logical extent records removed from one orphan before a VFS
    /// maintenance call returns. The wire format is independent of this
    /// runtime budget (ADR-066).
    orphan_cleanup_extent_budget: usize,
    /// Allocation-root node sets for (current, other) checkpoints, cached
    /// across commits so the reserved-pool exclusion set needs no tree walk
    /// per transaction. Populated lazily on the first commit (mount stays
    /// bounded) and maintained incrementally afterwards.
    allocation_tree_cache: Option<(Vec<u64>, Vec<u64>)>,
    /// Region where the last allocation succeeded; the next transaction
    /// starts its search there instead of rescanning from region zero.
    alloc_rover_region: u32,
    /// Host policy for protection edits of descriptor-bearing objects.
    security_projection: SecurityProjectionPolicy,
    window: Option<OpenWindow>,
    window_poisoned: bool,
    last_commit: Option<CommitStats>,
    /// The transaction-scoped shared-reference edit (ADR-061), keyed by the
    /// generation it was opened for. `next_generation` clears it so an
    /// aborted transaction can never leak half-applied edits into the next
    /// one, and `commit_transaction` publishes and consumes it.
    shared_refs: Option<(u64, RefEdit)>,
    /// Direct-layout promotions performed by the transaction being built.
    pending_layout_promotions: u64,
    data_update_policy: DataUpdatePolicy,
    /// Transaction-scoped count consumed by `commit_transaction`.
    pending_in_place_data_blocks: u64,
    pending_reservation_initializations: u64,
    pending_prewritten_data_blocks: u64,
    snapshot_limits: Option<SnapshotWorkLimits>,
    snapshot_handles: BTreeMap<u64, Weak<()>>,
    snapshot_mount: Arc<()>,
}

struct ApiTraceGuard<'a, D: BlockDevice> {
    volume: &'a mut Volume<D>,
    token: Option<crate::flight::ApiToken>,
    outcome: crate::flight::ApiOutcome,
}

impl<D: BlockDevice> Drop for ApiTraceGuard<'_, D> {
    fn drop(&mut self) {
        let generation = self.volume.checkpoint.generation;
        if let (Some(recorder), Some(token)) = (&mut self.volume.flight, self.token.take()) {
            recorder.borrow_mut().end_api(
                token,
                generation,
                self.outcome,
                self.volume.window_poisoned,
            );
        }
    }
}

impl<D: BlockDevice> Volume<D> {
    fn trace_api<T, E>(
        &mut self,
        method: crate::flight::ApiMethod,
        body: impl FnOnce(&mut Self) -> Result<T, E>,
    ) -> Result<T, E> {
        // Enabling observation through the control accessor also covers the
        // next read-only call against an already open (or poisoned) window.
        if let (Some(window), Some(recorder)) = (&self.window, &mut self.flight) {
            recorder.borrow_mut().observe_window(
                window.generation,
                window.logged_records,
                true,
                self.window_poisoned,
            );
        }
        let generation = self.checkpoint.generation;
        let token = self.flight.as_mut().and_then(|recorder| {
            recorder
                .borrow_mut()
                .begin_api(method, generation, self.window_poisoned)
        });
        let Some(token) = token else {
            return body(self);
        };
        let mut guard = ApiTraceGuard {
            volume: self,
            token: Some(token),
            outcome: crate::flight::ApiOutcome::Unwound,
        };
        let result = body(&mut *guard.volume);
        guard.outcome = if result.is_ok() {
            crate::flight::ApiOutcome::Succeeded
        } else {
            crate::flight::ApiOutcome::Failed
        };
        result
    }

    fn trace_api_infallible(
        &mut self,
        method: crate::flight::ApiMethod,
        body: impl FnOnce(&mut Self),
    ) {
        let result: Result<(), std::convert::Infallible> = self.trace_api(method, |volume| {
            body(volume);
            Ok(())
        });
        match result {
            Ok(()) => (),
            Err(never) => match never {},
        }
    }

    pub(crate) fn new(
        dev: D,
        ident: Identification,
        selection: Selection,
        state: MountState,
        mount_mode: MountMode,
    ) -> Self {
        Volume {
            flight: None,
            tree_cache_pages: std::num::NonZeroUsize::MAX,
            dev,
            ident,
            checkpoint: selection.chosen,
            current_slot: selection.chosen_slot,
            other_checkpoint: selection.other,
            fell_back_to_older: selection.fell_back_to_older,
            state,
            mount_mode,
            pending_intent_records: 0,
            reclaim_batch_blocks: crate::reclaim::DEFAULT_RECLAIM_BATCH_BLOCKS,
            orphan_cleanup_extent_budget: DEFAULT_ORPHAN_CLEANUP_EXTENTS,
            allocation_tree_cache: None,
            alloc_rover_region: 0,
            security_projection: SecurityProjectionPolicy::default(),
            window: None,
            window_poisoned: false,
            last_commit: None,
            shared_refs: None,
            pending_layout_promotions: 0,
            data_update_policy: DataUpdatePolicy::FullCow,
            pending_in_place_data_blocks: 0,
            pending_reservation_initializations: 0,
            pending_prewritten_data_blocks: 0,
            snapshot_limits: None,
            snapshot_handles: BTreeMap::new(),
            snapshot_mount: Arc::new(()),
        }
    }

    /// Replace optional diagnostics, recording a window detachment/attachment.
    /// Existing records are returned; ordinary draining uses flight_recorder_mut.
    /// Installation does not perform I/O or change filesystem policy.
    pub fn replace_flight_recorder(
        &mut self,
        recorder: Option<crate::flight::FlightRecorder>,
    ) -> Option<crate::flight::FlightRecorder> {
        let generation = self
            .window
            .as_ref()
            .map_or(self.checkpoint.generation, |w| w.generation);
        self.flight_window_event(generation, crate::flight::EventKind::WindowDetached, None);
        let shared = recorder.map(|recorder| Arc::new(crate::flight::RecorderCell::new(recorder)));
        let previous = std::mem::replace(&mut self.flight, shared);
        if let Some(window) = &mut self.window {
            window
                .tx
                .replace_observer(crate::flight::AllocationObserver::new(self.flight.as_ref()));
        }
        if let (Some(window), Some(recorder)) = (&self.window, &mut self.flight) {
            recorder.borrow_mut().observe_window(
                window.generation,
                window.logged_records,
                true,
                self.window_poisoned,
            );
        }
        previous.map(|recorder| {
            Arc::try_unwrap(recorder)
                .expect("only weak transaction observers may escape")
                .into_inner()
        })
    }

    pub fn flight_recorder(
        &self,
    ) -> Option<std::sync::RwLockReadGuard<'_, crate::flight::FlightRecorder>> {
        self.flight.as_ref().map(|recorder| recorder.borrow())
    }

    /// Drain or configure diagnostics without detaching their window context.
    pub fn flight_recorder_mut(
        &mut self,
    ) -> Option<std::sync::RwLockWriteGuard<'_, crate::flight::FlightRecorder>> {
        self.flight.as_ref().map(|recorder| recorder.borrow_mut())
    }

    fn flight_window_event(
        &mut self,
        generation: u64,
        kind: crate::flight::EventKind,
        sequence: Option<u32>,
    ) {
        if let Some(recorder) = &mut self.flight {
            recorder
                .borrow_mut()
                .window_event(generation, kind, sequence, self.window_poisoned);
        }
    }

    fn flight_event(&mut self, generation: u64, kind: crate::flight::EventKind) {
        if let Some(recorder) = &mut self.flight {
            recorder
                .borrow_mut()
                .emit(generation, kind, self.window_poisoned);
        }
    }

    /// Observe an intent-log mount step with values the volume already holds.
    /// Only observed mounts install a recorder before these steps. No I/O.
    pub(crate) fn flight_mount_event(
        &self,
        kind: crate::flight::EventKind,
        count: u32,
        damaged_tail: bool,
        log_sequence: u32,
        generation: u64,
    ) {
        if let Some(recorder) = &self.flight {
            let context = crate::flight::MountContext {
                mode: self.mount_mode,
                stage: crate::flight::MountStage::IntentLog,
                slot: self.current_slot as u8,
                other_generation: self
                    .other_checkpoint
                    .as_ref()
                    .map_or(0, |checkpoint| checkpoint.generation),
                count,
                damaged_tail,
            };
            recorder.borrow_mut().lifecycle_event(
                generation,
                kind,
                self.window_poisoned,
                log_sequence,
                crate::flight::LifecycleContext::Mount(context),
            );
        }
    }

    /// Observe one staged data write with values the caller already holds.
    /// Only a recorder with data observation enabled records anything. No I/O.
    pub(crate) fn flight_data_event(
        &self,
        kind: crate::flight::EventKind,
        context: crate::flight::DataContext,
        generation: u64,
    ) {
        if let Some(recorder) = &self.flight {
            recorder
                .borrow_mut()
                .data_event(generation, kind, self.window_poisoned, context);
        }
    }

    fn flight_intent_io_event(&self, kind: crate::flight::EventKind, generation: u64) {
        if let Some(recorder) = &self.flight {
            recorder
                .borrow_mut()
                .intent_io_event(generation, kind, self.window_poisoned);
        }
    }

    /// Observe one read-only descent over a live or captured view.
    pub(crate) fn flight_view_event(
        &self,
        kind: crate::flight::EventKind,
        context: crate::flight::ViewReadContext,
    ) {
        if let Some(recorder) = &self.flight {
            recorder.borrow_mut().view_event(
                self.checkpoint.generation,
                kind,
                self.window_poisoned,
                context,
            );
        }
    }

    /// Observe one read-only descent from its begin event to its outcome.
    fn flight_view_scope<T>(
        &mut self,
        path: crate::flight::ReadPath,
        view_id: u64,
        owner: u64,
        block: u64,
        body: impl FnOnce(&mut Self) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let context = crate::flight::ViewReadContext {
            path,
            view_id,
            owner,
            block,
        };
        self.flight_view_event(crate::flight::EventKind::ViewReadBegin, context);
        let result = body(self);
        self.flight_view_event(
            if result.is_ok() {
                crate::flight::EventKind::ViewReadComplete
            } else {
                crate::flight::EventKind::ViewReadFailed
            },
            context,
        );
        result
    }

    pub fn generation(&self) -> u64 {
        self.checkpoint.generation
    }

    pub fn ident(&self) -> &Identification {
        &self.ident
    }

    fn comparison_key(&self, name: &[u8]) -> Result<Vec<u8>, CoreError> {
        name_key::comparison_key(&self.ident, name)
    }

    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    pub fn mount_mode(&self) -> MountMode {
        self.mount_mode
    }

    pub fn pending_intent_records(&self) -> u32 {
        self.pending_intent_records
    }

    /// Whether this mount had to skip a newer but unusable checkpoint and
    /// read the older slot of the pair. False after the first commit of this
    /// mount, which publishes a checkpoint of its own.
    pub fn mounted_from_older_checkpoint(&self) -> bool {
        self.fell_back_to_older
    }

    /// Free blocks the allocation root accounts for, summed over its region
    /// records, against the total the mounted checkpoint states. `None` when
    /// they agree.
    ///
    /// One bounded read of the allocation root, not of the bitmap pages: a
    /// normal mount never reads those, and the region records are exactly
    /// what the descriptors and the pages are checked against on the full
    /// load path and by `afsplus-check`. A checkpoint whose total disagrees
    /// with its own regions is corrupt accounting either way.
    pub fn checkpoint_free_count_mismatch(&mut self) -> Result<Option<(u64, u64)>, CoreError> {
        let geo = self.ident.geometry();
        let records = crate::allocation_root::load_all(
            &mut self.dev,
            &geo,
            self.checkpoint.allocation_root_block,
            self.checkpoint.generation,
        )?
        .records;
        let counted: u64 = records.iter().map(|record| record.free_blocks as u64).sum();
        Ok((counted != self.checkpoint.free_blocks_total)
            .then_some((self.checkpoint.free_blocks_total, counted)))
    }

    /// Blocks currently quarantined in the reclaim queue.
    pub fn reclaim_pending_blocks(&self) -> u64 {
        self.state.reclaim_root.pending_blocks
    }

    /// Sets the per-transaction reclamation budget (blocks promoted from the
    /// queue head before each transaction allocates).
    pub fn set_reclaim_batch_blocks(&mut self, blocks: u64) {
        self.trace_api_infallible(crate::flight::ApiMethod::SetReclaimBatchBlocks, |volume| {
            volume.set_reclaim_batch_blocks_untraced(blocks)
        })
    }

    fn set_reclaim_batch_blocks_untraced(&mut self, blocks: u64) {
        self.reclaim_batch_blocks = blocks.max(1);
    }

    pub fn set_orphan_cleanup_extent_budget(&mut self, extents: usize) {
        self.trace_api_infallible(
            crate::flight::ApiMethod::SetOrphanCleanupExtentBudget,
            |volume| volume.set_orphan_cleanup_extent_budget_untraced(extents),
        )
    }

    fn set_orphan_cleanup_extent_budget_untraced(&mut self, extents: usize) {
        self.orphan_cleanup_extent_budget = extents.max(1);
    }

    /// Bound resident staged images in each COW tree mutation. Other metadata,
    /// decoded nodes, accumulated publication buffers and data have separate
    /// memory costs. Runtime-only; reapply through MountOptions at remount.
    /// An open intent window must be closed before changing its resource policy.
    pub fn set_tree_cache_pages(&mut self, pages: usize) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetTreeCachePages, |volume| {
            volume.set_tree_cache_pages_untraced(pages)
        })
    }

    fn set_tree_cache_pages_untraced(&mut self, pages: usize) -> Result<(), CoreError> {
        let pages = std::num::NonZeroUsize::new(pages).ok_or(CoreError::PrototypeLimit(
            "tree cache requires at least one page",
        ))?;
        if self.window.is_some() {
            return Err(CoreError::Busy);
        }
        self.tree_cache_pages = pages;
        Ok(())
    }

    pub fn tree_cache_pages(&self) -> usize {
        self.tree_cache_pages.get()
    }

    pub fn orphan_cleanup_extent_budget(&self) -> usize {
        self.orphan_cleanup_extent_budget
    }

    /// Diagnostic: whether `lba` is currently quarantined. Walks the queue;
    /// intended for tests and tooling, not the I/O path.
    pub fn quarantine_contains(&mut self, lba: u64) -> Result<bool, CoreError> {
        self.trace_api(crate::flight::ApiMethod::QuarantineContains, |volume| {
            volume.quarantine_contains_untraced(lba)
        })
    }

    fn quarantine_contains_untraced(&mut self, lba: u64) -> Result<bool, CoreError> {
        crate::reclaim::contains(
            &mut self.dev,
            &self.ident.geometry(),
            self.checkpoint.reclaim_root_block,
            self.checkpoint.generation,
            lba,
        )
    }

    /// Runs one maintenance transaction that only advances reclamation:
    /// promotes up to the configured budget from the queue head and commits.
    /// Returns the net reduction in ordinary queued blocks. Zero can accompany
    /// committed checkpoint-retention or snapshot-scan progress; inspect
    /// `last_commit_stats().alloc.reclaim.blocked_by_checkpoint` or use
    /// [`Self::snapshot_maintenance_step`] for separate lifetime progress.
    pub fn reclaim_step(&mut self, now: Timespec) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ReclaimStep, |volume| {
            volume.reclaim_step_untraced(now)
        })
    }

    fn reclaim_step_untraced(&mut self, now: Timespec) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        if self.state.reclaim_root.pending_blocks == 0
            && self
                .state
                .snapshots
                .is_none_or(|s| s.ledger.retained_blocks == 0)
        {
            return Ok(0);
        }
        let before = self.state.reclaim_root.pending_blocks;
        let generation = self.next_generation()?;
        let tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            Vec::new(),
            self.checkpoint.object_map_block,
        )?;
        Ok(before.saturating_sub(self.state.reclaim_root.pending_blocks))
    }

    pub fn free_blocks(&self) -> u64 {
        self.checkpoint.free_blocks_total
    }

    /// Raw blocks kept available to bounded destructive and recovery work.
    /// This is implementation policy only: no bitmap bits or disk fields are
    /// dedicated to the reserve, and emergency transactions may consume it.
    pub fn emergency_headroom_blocks(&self) -> u64 {
        emergency_headroom_for_volume(self.ident.total_blocks)
    }

    /// Capacity available to ordinary growth after preserving emergency
    /// metadata headroom. Raw free space remains available through
    /// [`Self::free_blocks`] for diagnostics and privileged maintenance.
    pub fn available_blocks(&self) -> u64 {
        self.free_blocks()
            .saturating_sub(self.emergency_headroom_blocks())
    }

    /// [`Self::available_blocks`] less what the open window has already
    /// allocated: the checkpoint's count does not see the window, and a
    /// window of large writes can hold most of what it reports.
    pub fn window_available_blocks(&self) -> u64 {
        match &self.window {
            Some(window) => window
                .tx
                .free_blocks_remaining()
                .saturating_sub(self.emergency_headroom_blocks()),
            None => self.available_blocks(),
        }
    }

    fn protect_emergency_headroom(&self, tx: &mut TxAllocator) {
        tx.set_free_block_floor(self.emergency_headroom_blocks());
    }

    /// Peak bitmap bytes resident during the most recent transaction (zero
    /// before any mutation on this mount).
    pub fn allocator_ram_bytes(&self) -> usize {
        self.last_commit
            .map(|stats| stats.alloc.allocator_ram_bytes as usize)
            .unwrap_or(0)
    }

    pub fn last_commit_stats(&self) -> Option<CommitStats> {
        self.last_commit
    }

    pub fn data_update_policy(&self) -> DataUpdatePolicy {
        self.data_update_policy
    }

    /// Selects the runtime-only data update policy. Newly mounted volumes
    /// always start in [`DataUpdatePolicy::FullCow`].
    pub fn set_data_update_policy(&mut self, policy: DataUpdatePolicy) {
        self.trace_api_infallible(crate::flight::ApiMethod::SetDataUpdatePolicy, |volume| {
            volume.set_data_update_policy_untraced(policy)
        })
    }

    fn set_data_update_policy_untraced(&mut self, policy: DataUpdatePolicy) {
        self.data_update_policy = policy;
    }

    /// The persistent per-file policy (ADR-065): `InPlacePrivate` when the
    /// file's record carries `OBJECT_FLAG_DATA_IN_PLACE`.
    pub fn file_data_policy(&mut self, object_id: u64) -> Result<DataUpdatePolicy, CoreError> {
        self.trace_api(crate::flight::ApiMethod::FileDataPolicy, |volume| {
            volume.file_data_policy_untraced(object_id)
        })
    }

    fn file_data_policy_untraced(&mut self, object_id: u64) -> Result<DataUpdatePolicy, CoreError> {
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        Ok(if record.flags & OBJECT_FLAG_DATA_IN_PLACE != 0 {
            DataUpdatePolicy::InPlacePrivate
        } else {
            DataUpdatePolicy::FullCow
        })
    }

    /// Persistently opts a file into (or back out of) ADR-062 private
    /// in-place updates by setting `OBJECT_FLAG_DATA_IN_PLACE` on its record
    /// (ADR-065). A metadata-COW transaction: the change timestamp advances
    /// and the choice survives remounts. Requires the volume `COMPAT`
    /// data-policy feature; files only. Eligibility per write is unchanged —
    /// shared, unwritten, unmapped or extending writes still take full COW.
    pub fn set_file_data_policy(
        &mut self,
        object_id: u64,
        policy: DataUpdatePolicy,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetFileDataPolicy, |volume| {
            volume.set_file_data_policy_untraced(object_id, policy, now)
        })
    }

    fn set_file_data_policy_untraced(
        &mut self,
        object_id: u64,
        policy: DataUpdatePolicy,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        if !self.data_policy_enabled() {
            return Err(CoreError::FeatureDisabled(
                "data-policy feature is not enabled on this volume",
            ));
        }
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let new_flags = match policy {
            DataUpdatePolicy::InPlacePrivate => record.flags | OBJECT_FLAG_DATA_IN_PLACE,
            DataUpdatePolicy::FullCow => record.flags & !OBJECT_FLAG_DATA_IN_PLACE,
        };
        if new_flags == record.flags {
            return Ok(());
        }
        self.commit_object_metadata(ObjectRecord {
            flags: new_flags,
            changed: now,
            ..record
        })
    }

    /// Filesystem-wide durability barrier. Successful immediate mutations
    /// are already durable; this also gives adapters an explicit sync hook.
    /// Read-only modes return success without touching the device.
    pub fn sync(&mut self) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::Sync, |volume| {
            volume.sync_untraced()
        })
    }

    fn sync_untraced(&mut self) -> Result<(), CoreError> {
        if !self.mount_mode.allows_user_writes() {
            return Ok(());
        }
        self.ensure_window_closed()?;
        self.dev.flush()?;
        Ok(())
    }

    /// Looks a name up in the root directory.
    pub fn lookup_root(&mut self, name: &str) -> Result<Option<u64>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::LookupRoot, |volume| {
            volume.lookup_root_untraced(name)
        })
    }

    fn lookup_root_untraced(&mut self, name: &str) -> Result<Option<u64>, CoreError> {
        self.lookup_in_directory(OBJECT_ROOT, name)
    }

    /// Looks a name up in a directory identified by its stable object ID.
    pub fn lookup_in_directory(
        &mut self,
        directory_id: u64,
        name: &str,
    ) -> Result<Option<u64>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::LookupInDirectory, |volume| {
            volume.lookup_in_directory_untraced(directory_id, name)
        })
    }

    fn lookup_in_directory_untraced(
        &mut self,
        directory_id: u64,
        name: &str,
    ) -> Result<Option<u64>, CoreError> {
        self.ensure_public_object_id(directory_id)?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        // Changes waiting in the open window are part of the namespace every
        // caller sees.
        if let Some(window) = self.window.take() {
            let result = self
                .comparison_key(name.as_bytes())
                .and_then(|key| self.batch_lookup(&window.pending, directory_id, &key));
            self.window = Some(window);
            return result.map(|entry| entry.map(|entry| entry.child_id));
        }
        let directory_record = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        if directory_record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let key = self.comparison_key(name.as_bytes())?;
        let root = directory_record.data_root;
        let entry = self.flight_view_scope(
            crate::flight::ReadPath::Lookup,
            0,
            directory_id,
            root,
            |volume| {
                directory::lookup_entry(
                    &mut volume.dev,
                    &volume.ident.geometry(),
                    root,
                    directory_id,
                    volume.checkpoint.generation,
                    &volume.ident,
                    &key,
                )
            },
        )?;
        Ok(entry.map(|entry| entry.child_id))
    }

    /// Lists the root directory as (original name, object ID) pairs.
    pub fn list_root(&mut self) -> Result<Vec<(String, u64)>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ListRoot, |volume| {
            volume.list_root_untraced()
        })
    }

    fn list_root_untraced(&mut self) -> Result<Vec<(String, u64)>, CoreError> {
        self.list_directory(OBJECT_ROOT)
    }

    /// Lists a directory as `(original name, object ID)` pairs.
    pub fn list_directory(&mut self, directory_id: u64) -> Result<Vec<(String, u64)>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ListDirectory, |volume| {
            volume.list_directory_untraced(directory_id)
        })
    }

    fn list_directory_untraced(
        &mut self,
        directory_id: u64,
    ) -> Result<Vec<(String, u64)>, CoreError> {
        self.ensure_public_object_id(directory_id)?;
        let directory_record = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        if directory_record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let root = directory_record.data_root;
        Ok(self
            .flight_view_scope(
                crate::flight::ReadPath::Enumeration,
                0,
                directory_id,
                root,
                |volume| {
                    directory::load_all(
                        &mut volume.dev,
                        &volume.ident.geometry(),
                        root,
                        directory_id,
                        volume.checkpoint.generation,
                        &volume.ident,
                    )
                },
            )?
            .entries
            .iter()
            .map(|e| (String::from_utf8_lossy(&e.name).into_owned(), e.child_id))
            .collect())
    }

    /// Reads a bounded directory page. Cursors bind to the mounted checkpoint
    /// generation; callers must restart after any commit that makes one stale.
    pub fn read_directory_page(
        &mut self,
        directory_id: u64,
        cursor: Option<DirectoryCursor>,
        max_entries: usize,
    ) -> Result<DirectoryPage, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ReadDirectoryPage, |volume| {
            volume.read_directory_page_untraced(directory_id, cursor, max_entries)
        })
    }

    fn read_directory_page_untraced(
        &mut self,
        directory_id: u64,
        cursor: Option<DirectoryCursor>,
        max_entries: usize,
    ) -> Result<DirectoryPage, CoreError> {
        self.ensure_public_object_id(directory_id)?;
        if max_entries > MAX_DIRECTORY_PAGE_ENTRIES {
            return Err(CoreError::PrototypeLimit(
                "directory page exceeds entry cap",
            ));
        }
        let cursor = cursor.unwrap_or(DirectoryCursor {
            generation: self.checkpoint.generation,
            ordinal: 0,
        });
        if cursor.generation != self.checkpoint.generation {
            return Err(CoreError::Stale);
        }
        let record = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let root = record.data_root;
        let (entries, total) = self.flight_view_scope(
            crate::flight::ReadPath::Enumeration,
            0,
            directory_id,
            root,
            |volume| {
                directory::read_page(
                    &mut volume.dev,
                    &volume.ident.geometry(),
                    root,
                    directory::spec(directory_id, volume.checkpoint.generation),
                    &volume.ident,
                    cursor.ordinal,
                    max_entries,
                )
            },
        )?;
        let next_ordinal = cursor
            .ordinal
            .checked_add(entries.len() as u64)
            .ok_or_else(|| CoreError::Corrupt("directory cursor overflow".into()))?;
        Ok(DirectoryPage {
            entries,
            next: DirectoryCursor {
                generation: cursor.generation,
                ordinal: next_ordinal,
            },
            eof: next_ordinal >= total,
        })
    }

    /// Reads and validates one committed object record on demand. Returning
    /// the record by value keeps the low-memory path independent of a
    /// mandatory object cache; modern implementations may add a bounded or
    /// aggressive cache above this API.
    pub fn stat(&mut self, object_id: u64) -> Result<Option<ObjectRecord>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::Stat, |volume| {
            volume.stat_untraced(object_id)
        })
    }

    fn stat_untraced(&mut self, object_id: u64) -> Result<Option<ObjectRecord>, CoreError> {
        self.ensure_public_object_id(object_id)?;
        self.read_object(object_id)
    }

    /// Read committed semantic allocation without flushing or materializing
    /// an open mutation window. Cursors are entry ordinals, not byte offsets.
    pub fn file_allocation_page(
        &mut self,
        object_id: u64,
        start: u64,
        limit: usize,
    ) -> Result<FileAllocationPage, CoreError> {
        self.trace_api(crate::flight::ApiMethod::FileAllocationPage, |volume| {
            volume.file_allocation_page_untraced(object_id, start, limit)
        })
    }

    /// Committed semantic allocation from a byte offset: the range covering
    /// `offset`, or the first one after it, and its successors, at most
    /// `limit` (1 to 64). One tree descent wherever the offset lies, unlike
    /// the ordinal cursor of [`Self::file_allocation_page`], whose contract it
    /// shares. `next` is the ordinal-free resume point:
    /// the byte offset after the last returned range. Refuses an open
    /// mutation window with `Busy`.
    pub fn file_allocation_from(
        &mut self,
        object_id: u64,
        offset: u64,
        limit: usize,
    ) -> Result<FileAllocationPage, CoreError> {
        self.trace_api(crate::flight::ApiMethod::FileAllocationFrom, |volume| {
            volume.file_allocation_from_untraced(object_id, offset, limit)
        })
    }

    fn file_allocation_from_untraced(
        &mut self,
        object_id: u64,
        offset: u64,
        limit: usize,
    ) -> Result<FileAllocationPage, CoreError> {
        if limit == 0 || limit > 64 {
            return Err(CoreError::PrototypeLimit(
                "allocation page limit out of range",
            ));
        }
        if self.window.is_some() {
            return Err(CoreError::Busy);
        }
        let record = self.stat(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let geo = self.ident.geometry();
        let block_size = geo.block_size as u64;
        let start_block = offset / block_size;
        let (extents, cut) = if record.flags & afsplus_format::object::OBJECT_FLAG_EXTENT_TREE != 0
        {
            extent_map::read_from(
                &mut self.dev,
                &geo,
                record.data_root,
                object_id,
                self.checkpoint.generation,
                start_block,
                limit,
            )?
        } else if record.data_blocks > start_block {
            (
                vec![extent_map::Extent {
                    logical_start: 0,
                    physical_start: record.data_root,
                    block_count: record.data_blocks,
                    flags: 0,
                }],
                false,
            )
        } else {
            (vec![], false)
        };
        let mut ranges = Vec::with_capacity(extents.len());
        let mut next = offset;
        for extent in extents {
            let start = extent
                .logical_start
                .checked_mul(block_size)
                .ok_or_else(|| CoreError::Corrupt("allocation byte offset overflows".into()))?;
            let length = extent
                .block_count
                .checked_mul(block_size)
                .ok_or_else(|| CoreError::Corrupt("allocation byte length overflows".into()))?;
            next = start.saturating_add(length);
            ranges.push(FileAllocationRange {
                offset: start,
                length,
                unwritten: extent.flags & extent_map::EXTENT_UNWRITTEN != 0,
            });
        }
        Ok(FileAllocationPage {
            ranges,
            next,
            eof: !cut,
        })
    }

    fn file_allocation_page_untraced(
        &mut self,
        object_id: u64,
        start: u64,
        limit: usize,
    ) -> Result<FileAllocationPage, CoreError> {
        if limit == 0 || limit > 64 {
            return Err(CoreError::PrototypeLimit(
                "allocation page limit out of range",
            ));
        }
        if self.window.is_some() {
            return Err(CoreError::Busy);
        }
        let record = self.stat(object_id)?.ok_or(CoreError::NotFound)?;
        self.read_allocation_page(&record, object_id, self.checkpoint.generation, start, limit)
    }

    fn read_allocation_page(
        &mut self,
        record: &ObjectRecord,
        object_id: u64,
        generation: u64,
        start: u64,
        limit: usize,
    ) -> Result<FileAllocationPage, CoreError> {
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let geo = self.ident.geometry();
        let (extents, total) =
            if record.flags & afsplus_format::object::OBJECT_FLAG_EXTENT_TREE != 0 {
                let page = extent_map::read_page(
                    &mut self.dev,
                    &geo,
                    record.data_root,
                    object_id,
                    generation,
                    start,
                    limit,
                )?;
                (page.extents, page.total_extents)
            } else if record.data_blocks != 0 {
                let records = if start == 0 {
                    vec![extent_map::Extent {
                        logical_start: 0,
                        physical_start: record.data_root,
                        block_count: record.data_blocks,
                        flags: 0,
                    }]
                } else {
                    vec![]
                };
                (records, 1)
            } else {
                (vec![], 0)
            };
        if start > total {
            return Err(CoreError::PrototypeLimit("allocation cursor beyond end"));
        }
        let mut ranges = Vec::with_capacity(extents.len());
        for extent in extents {
            let offset = extent
                .logical_start
                .checked_mul(geo.block_size as u64)
                .ok_or_else(|| CoreError::Corrupt("allocation byte offset overflows".into()))?;
            let length = extent
                .block_count
                .checked_mul(geo.block_size as u64)
                .ok_or_else(|| CoreError::Corrupt("allocation byte length overflows".into()))?;
            ranges.push(FileAllocationRange {
                offset,
                length,
                unwritten: extent.flags & extent_map::EXTENT_UNWRITTEN != 0,
            });
        }
        let next = start + ranges.len() as u64;
        Ok(FileAllocationPage {
            ranges,
            next,
            eof: next == total,
        })
    }

    /// Returns filesystem-facing metadata through the open intent-log
    /// overlay. This keeps adapters away from physical roots that do not exist
    /// until the final checkpoint materializes the pending logical layout.
    pub fn visible_metadata(
        &mut self,
        object_id: u64,
    ) -> Result<Option<ObjectMetadata>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::VisibleMetadata, |volume| {
            volume.visible_metadata_untraced(object_id)
        })
    }

    fn visible_metadata_untraced(
        &mut self,
        object_id: u64,
    ) -> Result<Option<ObjectMetadata>, CoreError> {
        if let Some(window) = self.window.as_ref() {
            if let Some(record) = window.pending.records.get(&object_id) {
                let Some(record) = *record else {
                    return Ok(None);
                };
                let mut metadata = ObjectMetadata::from(record);
                if let Some(layout) = window.pending.file_layouts.get(&object_id) {
                    let allocated_blocks =
                        layout.extents.iter().try_fold(0u64, |total, extent| {
                            total
                                .checked_add(extent.block_count)
                                .ok_or(CoreError::PrototypeLimit(
                                    "visible allocated block count overflow",
                                ))
                        })?;
                    metadata.size_bytes = layout.size_bytes;
                    metadata.allocated_bytes = allocated_blocks
                        .checked_mul(self.dev.block_size() as u64)
                        .ok_or(CoreError::PrototypeLimit(
                            "visible allocated byte count overflow",
                        ))?;
                }
                return Ok(Some(metadata));
            }
        }
        Ok(self.read_object(object_id)?.map(ObjectMetadata::from))
    }

    /// Reads a file's visible content, including existing-file edits in the
    /// open intent-log window.
    pub fn read_file(&mut self, object_id: u64) -> Result<Vec<u8>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ReadFile, |volume| {
            volume.read_file_untraced(object_id)
        })
    }

    fn read_file_untraced(&mut self, object_id: u64) -> Result<Vec<u8>, CoreError> {
        let record = self
            .visible_metadata(object_id)?
            .ok_or_else(|| CoreError::Corrupt(format!("no object {object_id}")))?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::Corrupt(format!(
                "object {object_id} is not a file"
            )));
        }
        let content_len = usize::try_from(record.size_bytes)
            .map_err(|_| CoreError::PrototypeLimit("file is too large to read into one buffer"))?;
        let mut content = vec![0u8; content_len];
        self.read_file_at(object_id, 0, &mut content)?;
        Ok(content)
    }

    /// Reads visible file bytes at `offset` into a caller-owned buffer. Sparse
    /// holes and unwritten extents are returned as zeros. Existing-file edits
    /// in the open intent-log window take precedence over the committed map.
    pub fn read_file_at(
        &mut self,
        object_id: u64,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ReadFileAt, |volume| {
            volume.read_file_at_untraced(object_id, offset, destination)
        })
    }

    fn read_file_at_untraced(
        &mut self,
        object_id: u64,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, CoreError> {
        // The mapping descent resolves its own root, so the read-path event
        // names the object and leaves the tree root block zero.
        let context = crate::flight::ViewReadContext {
            path: crate::flight::ReadPath::FileData,
            view_id: 0,
            owner: object_id,
            block: 0,
        };
        self.flight_view_event(crate::flight::EventKind::ViewReadBegin, context);
        let result = self.read_file_at_mapped(object_id, offset, destination);
        self.flight_view_event(
            if result.is_ok() {
                crate::flight::EventKind::ViewReadComplete
            } else {
                crate::flight::EventKind::ViewReadFailed
            },
            context,
        );
        result
    }

    fn read_file_at_mapped(
        &mut self,
        object_id: u64,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, CoreError> {
        let record = self
            .visible_metadata(object_id)?
            .ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        if destination.is_empty() || offset >= record.size_bytes {
            return Ok(0);
        }
        let count = (record.size_bytes - offset).min(destination.len() as u64);
        let end = offset + count;
        let block_size = self.dev.block_size() as u64;
        let mut block = vec![0u8; block_size as usize];
        let pending_layout = self.window.as_ref().and_then(|window| {
            window
                .pending
                .file_layouts
                .get(&object_id)
                .map(|layout| layout.extents.clone())
        });
        // A file the window created lives in one run until the commit.
        let created_run = self
            .window
            .as_ref()
            .and_then(|window| window.pending.created_data.get(&object_id).copied());
        let committed_record = if pending_layout.is_none() && created_run.is_none() {
            Some(self.read_object(object_id)?.ok_or(CoreError::NotFound)?)
        } else {
            None
        };
        for logical_block in offset / block_size..end.div_ceil(block_size) {
            block.fill(0);
            let mapped = if let Some(extents) = pending_layout.as_deref() {
                extent_at(extents, logical_block)
                    .filter(|extent| extent.flags & EXTENT_UNWRITTEN == 0)
                    .map(|extent| extent.physical_start + logical_block - extent.logical_start)
            } else if let Some((start, blocks)) = created_run {
                (logical_block < blocks).then_some(start + logical_block)
            } else {
                let record = committed_record
                    .as_ref()
                    .expect("committed record loaded without a pending layout");
                if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
                    extent_map::lookup_extent(
                        &mut self.dev,
                        &self.ident.geometry(),
                        record.data_root,
                        object_id,
                        self.checkpoint.generation,
                        logical_block,
                    )?
                    .filter(|extent| extent.flags & EXTENT_UNWRITTEN == 0)
                    .map(|extent| extent.physical_start + logical_block - extent.logical_start)
                } else if logical_block < record.data_blocks {
                    Some(record.data_root.checked_add(logical_block).ok_or_else(|| {
                        CoreError::Corrupt(format!("object {object_id} extent overflow"))
                    })?)
                } else {
                    None
                }
            };
            if let Some(lba) = mapped {
                self.dev.read_block(lba, &mut block)?;
            }
            let block_start = logical_block * block_size;
            let copy_start = offset.max(block_start);
            let copy_end = end.min(block_start.saturating_add(block_size));
            let source = (copy_start - block_start) as usize..(copy_end - block_start) as usize;
            let target = (copy_start - offset) as usize..(copy_end - offset) as usize;
            destination[target].copy_from_slice(&block[source]);
        }
        Ok(count as usize)
    }

    /// Replaces `content.len()` bytes at `offset` and publishes the metadata
    /// atomically. Written data uses fresh blocks by default; eligible private
    /// unwritten reservations are initialized before COW publication (ADR-079).
    /// The experimental
    /// private-in-place policy may reuse proven-private physical blocks for a
    /// non-extending write; see [`DataUpdatePolicy`]. Writing beyond EOF
    /// creates a hole; the file is converted from its cheap direct extent to
    /// an AFST extent map only when the resulting layout is sparse or
    /// fragmented.
    pub fn write_file_at(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WriteFileAt, |volume| {
            volume.write_file_at_untraced(object_id, offset, content, now)
        })
    }

    fn write_file_at_untraced(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.write_file_at_with_limits(object_id, offset, content, now, None)
    }

    /// Atomic write with explicit touched-block and local extent-record budgets.
    pub fn write_file_at_bounded(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
        limits: FileEditLimits,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WriteFileAtBounded, |volume| {
            volume.write_file_at_bounded_untraced(object_id, offset, content, now, limits)
        })
    }

    fn write_file_at_bounded_untraced(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
        limits: FileEditLimits,
    ) -> Result<(), CoreError> {
        if limits.max_blocks == 0 || limits.max_records == 0 || limits.max_records == usize::MAX {
            return Err(CoreError::PrototypeLimit("file edit limits invalid"));
        }
        self.write_file_at_with_limits(object_id, offset, content, now, Some(limits))
    }

    fn write_file_at_with_limits(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
        limits: Option<FileEditLimits>,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        if content.is_empty() {
            return Ok(());
        }
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {object_id} missing from object map"))
        })?;
        let content_len = u64::try_from(content.len())
            .map_err(|_| CoreError::PrototypeLimit("write buffer is too large"))?;
        let end_offset = offset
            .checked_add(content_len)
            .ok_or(CoreError::PrototypeLimit("file size limit reached"))?;
        let block_size = self.dev.block_size() as u64;
        let first_block = offset / block_size;
        let end_block = end_offset.div_ceil(block_size);
        let write_block_count = end_block - first_block;
        if limits.is_some_and(|limit| write_block_count > limit.max_blocks) {
            return Err(CoreError::PrototypeLimit(
                "file edit block budget exhausted",
            ));
        }
        let local_tree = limits.is_some() && record.flags & OBJECT_FLAG_EXTENT_TREE != 0;
        let (old_extents, old_tree_blocks) = if let Some(limit) = limits.filter(|_| local_tree) {
            (
                extent_map::read_window(
                    &mut self.dev,
                    &self.ident.geometry(),
                    record.data_root,
                    object_id,
                    self.checkpoint.generation,
                    first_block,
                    end_block,
                    limit.max_records,
                )?,
                Vec::new(),
            )
        } else {
            self.load_file_layout(&record)?
        };
        let overwrite_in_place = (self.data_update_policy == DataUpdatePolicy::InPlacePrivate
            || record.flags & OBJECT_FLAG_DATA_IN_PLACE != 0)
            && !self.snapshots_require_cow()
            && end_offset <= record.size_bytes
            && (first_block..end_block).all(|logical_block| {
                extent_at(&old_extents, logical_block).is_some_and(|extent| extent.flags == 0)
            });

        // Partial first/last blocks inherit their committed bytes. Holes and
        // unwritten preallocation read as zeros, so they need no special case.
        let mut blocks = Vec::with_capacity(write_block_count as usize);
        for logical_block in first_block..end_block {
            let mut block = vec![0u8; block_size as usize];
            self.read_layout_block(&old_extents, logical_block, &mut block)?;
            let logical_byte = logical_block.saturating_mul(block_size);
            let copy_start = offset.max(logical_byte);
            let copy_end = end_offset.min(logical_byte.saturating_add(block_size));
            let source_start = usize::try_from(copy_start - offset)
                .map_err(|_| CoreError::PrototypeLimit("write offset is too large"))?;
            let source_end = usize::try_from(copy_end - offset)
                .map_err(|_| CoreError::PrototypeLimit("write offset is too large"))?;
            let target_start = usize::try_from(copy_start - logical_byte)
                .map_err(|_| CoreError::PrototypeLimit("block offset is too large"))?;
            block[target_start..target_start + source_end - source_start]
                .copy_from_slice(&content[source_start..source_end]);
            blocks.push(block);
        }

        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);
        if overwrite_in_place {
            let mut data_writes = Vec::with_capacity(write_block_count as usize);
            for (logical_block, block) in (first_block..end_block).zip(blocks) {
                let extent = extent_at(&old_extents, logical_block).ok_or_else(|| {
                    CoreError::Corrupt("in-place write lost its validated extent".into())
                })?;
                let physical = extent
                    .physical_start
                    .checked_add(logical_block - extent.logical_start)
                    .ok_or_else(|| {
                        CoreError::Corrupt("in-place physical block overflows".into())
                    })?;
                data_writes.push((physical, block));
            }
            self.pending_in_place_data_blocks = write_block_count;
            if local_tree {
                let staged = self.stage_extent_delta(
                    &mut tx,
                    record,
                    record_lba,
                    &old_extents,
                    &old_extents,
                    record.size_bytes,
                    true,
                    now,
                    generation,
                )?;
                return self.commit_staged_file_layout(
                    record,
                    staged,
                    Vec::new(),
                    generation,
                    tx,
                    data_writes,
                );
            }
            return self.commit_file_layout(
                record,
                record_lba,
                old_extents.clone(),
                old_tree_blocks,
                old_extents,
                Vec::new(),
                record.size_bytes,
                true,
                now,
                generation,
                tx,
                data_writes,
            );
        }
        let mut additions = Vec::new();
        let mut logical = first_block;
        while logical < end_block {
            if let Some(extent) =
                extent_at(&old_extents, logical).filter(|extent| extent.flags == EXTENT_UNWRITTEN)
            {
                let count = extent.logical_end()?.min(end_block) - logical;
                let physical = extent.physical_start + logical - extent.logical_start;
                // A false-private marker cannot authorize initialization of a
                // block another live object maps. Keep the lookup local.
                self.shared_prefetch(generation, physical, count)?;
                if self
                    .shared_refs_edit(generation)?
                    .resolve(physical, count)?
                    .iter()
                    .any(|run| run.reference_count.is_some())
                {
                    return Err(CoreError::Corrupt(
                        "private unwritten extent overlaps shared references".into(),
                    ));
                }
                additions.push(Extent {
                    logical_start: logical,
                    physical_start: physical,
                    block_count: count,
                    flags: 0,
                });
                if limits.is_some_and(|limit| additions.len() > limit.max_records) {
                    return Err(CoreError::PrototypeLimit(
                        "file edit allocation record budget exhausted",
                    ));
                }
                self.pending_reservation_initializations += count;
                logical += count;
            } else {
                let after = old_extents.partition_point(|extent| extent.logical_start <= logical);
                let stop = old_extents[after..]
                    .iter()
                    .find(|extent| extent.flags == EXTENT_UNWRITTEN)
                    .map_or(end_block, |extent| extent.logical_start.min(end_block));
                additions.extend(allocate_extent_runs_bounded(
                    &mut tx,
                    &mut self.dev,
                    &self.ident.geometry(),
                    logical,
                    stop - logical,
                    0,
                    limits.map_or(usize::MAX, |limit| {
                        limit.max_records.saturating_sub(additions.len())
                    }),
                )?);
                logical = stop;
            }
        }
        let mut block_iter = blocks.into_iter();
        let mut data_writes = Vec::with_capacity(write_block_count as usize);
        for extent in &additions {
            for offset in 0..extent.block_count {
                let block = block_iter.next().ok_or_else(|| {
                    CoreError::Corrupt("write extent/data block count mismatch".into())
                })?;
                data_writes.push((extent.physical_start + offset, block));
            }
        }
        if block_iter.next().is_some() {
            return Err(CoreError::Corrupt(
                "write extent/data block count mismatch".into(),
            ));
        }
        let (mut new_extents, mut removed_extents) =
            replace_logical_range(&old_extents, first_block, end_block, None)?;
        // Initialized reservations keep their allocation and lifetime birth.
        removed_extents.retain(|extent| extent.flags != EXTENT_UNWRITTEN);
        new_extents.extend(additions);
        let new_extents = coalesce_extents(new_extents)?;

        if limits.is_some_and(|limit| new_extents.len() > limit.max_records) {
            return Err(CoreError::PrototypeLimit(
                "file edit result record budget exhausted",
            ));
        }
        if local_tree {
            let staged = self.stage_extent_delta(
                &mut tx,
                record,
                record_lba,
                &old_extents,
                &new_extents,
                record.size_bytes.max(end_offset),
                true,
                now,
                generation,
            )?;
            return self.commit_staged_file_layout(
                record,
                staged,
                removed_extents,
                generation,
                tx,
                data_writes,
            );
        }
        self.commit_file_layout(
            record,
            record_lba,
            old_extents,
            old_tree_blocks,
            new_extents,
            removed_extents,
            record.size_bytes.max(end_offset),
            true,
            now,
            generation,
            tx,
            data_writes,
        )
    }

    /// Changes a file's logical size atomically. Growth creates zero-reading
    /// holes. Shrinking a written partial block COW-rewrites that block with a
    /// zeroed tail so a later extension cannot reveal bytes past the old EOF.
    pub fn truncate_file(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::TruncateFile, |volume| {
            volume.truncate_file_untraced(object_id, new_size, now)
        })
    }

    fn truncate_file_untraced(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.truncate_file_with_limits(object_id, new_size, now, None)
    }

    /// Resize atomically with bounded affected extent records and retired blocks.
    pub fn truncate_file_bounded(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
        limits: FileEditLimits,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::TruncateFileBounded, |volume| {
            volume.truncate_file_bounded_untraced(object_id, new_size, now, limits)
        })
    }

    fn truncate_file_bounded_untraced(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
        limits: FileEditLimits,
    ) -> Result<(), CoreError> {
        if limits.max_blocks == 0 || limits.max_records == 0 || limits.max_records == usize::MAX {
            return Err(CoreError::PrototypeLimit("file edit limits invalid"));
        }
        self.truncate_file_with_limits(object_id, new_size, now, Some(limits))
    }

    fn truncate_file_with_limits(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
        limits: Option<FileEditLimits>,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        if new_size == record.size_bytes {
            return Ok(());
        }
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {object_id} missing from object map"))
        })?;
        if new_size > record.size_bytes && record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            let generation = self.next_generation()?;
            let mut tx = TxAllocator::begin_observed(
                crate::flight::AllocationObserver::new(self.flight.as_ref()),
                &mut self.dev,
                &self.ident.geometry(),
                &self.checkpoint,
                self.other_checkpoint.as_ref(),
                generation,
                self.reclaim_batch_blocks,
                self.alloc_rover_region,
            )?
            .with_tree_cache_pages(self.tree_cache_pages);
            self.protect_emergency_headroom(&mut tx);
            let staged = self.stage_extent_delta(
                &mut tx,
                record,
                record_lba,
                &[],
                &[],
                new_size,
                true,
                now,
                generation,
            )?;
            return self.commit_staged_file_layout(
                record,
                staged,
                Vec::new(),
                generation,
                tx,
                Vec::new(),
            );
        }
        let block_size = self.dev.block_size() as u64;
        let local_tree = limits.is_some() && record.flags & OBJECT_FLAG_EXTENT_TREE != 0;
        let (old_extents, old_tree_blocks) = if let Some(limit) = limits.filter(|_| local_tree) {
            (
                extent_map::read_window(
                    &mut self.dev,
                    &self.ident.geometry(),
                    record.data_root,
                    object_id,
                    self.checkpoint.generation,
                    new_size / block_size,
                    u64::MAX,
                    limit.max_records,
                )?,
                Vec::new(),
            )
        } else {
            self.load_file_layout(&record)?
        };
        let mut new_extents = old_extents.clone();
        let mut removed_extents = Vec::new();
        let mut tail_rewrite = None;

        if new_size < record.size_bytes {
            let retained_blocks = new_size.div_ceil(block_size);
            (new_extents, removed_extents) =
                replace_logical_range(&old_extents, retained_blocks, u64::MAX, None)?;
            if !new_size.is_multiple_of(block_size) {
                let logical_block = retained_blocks - 1;
                if extent_at(&new_extents, logical_block)
                    .is_some_and(|extent| extent.flags & EXTENT_UNWRITTEN == 0)
                {
                    let mut block = vec![0u8; block_size as usize];
                    self.read_layout_block(&new_extents, logical_block, &mut block)?;
                    block[(new_size % block_size) as usize..].fill(0);
                    tail_rewrite = Some((logical_block, block));
                }
            }
        }

        if let Some(limit) = limits {
            let retired = removed_extents.iter().try_fold(
                u64::from(tail_rewrite.is_some()),
                |sum, extent| {
                    sum.checked_add(extent.block_count)
                        .ok_or(CoreError::PrototypeLimit("retired block count overflow"))
                },
            )?;
            if retired > limit.max_blocks {
                return Err(CoreError::PrototypeLimit(
                    "file edit retirement block budget exhausted",
                ));
            }
        }
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        if new_size > record.size_bytes {
            self.protect_emergency_headroom(&mut tx);
        }
        let mut data_writes = Vec::new();
        if let Some((logical_block, block)) = tail_rewrite {
            let physical_start = tx.allocate(&mut self.dev)?;
            let replacement = Extent {
                logical_start: logical_block,
                physical_start,
                block_count: 1,
                flags: 0,
            };
            let (rewritten, mut removed) = replace_logical_range(
                &new_extents,
                logical_block,
                logical_block + 1,
                Some(replacement),
            )?;
            new_extents = rewritten;
            removed_extents.append(&mut removed);
            data_writes.push((physical_start, block));
        }

        if limits.is_some_and(|limit| new_extents.len() > limit.max_records) {
            return Err(CoreError::PrototypeLimit(
                "file edit result record budget exhausted",
            ));
        }
        if local_tree {
            let staged = self.stage_extent_delta(
                &mut tx,
                record,
                record_lba,
                &old_extents,
                &new_extents,
                new_size,
                true,
                now,
                generation,
            )?;
            return self.commit_staged_file_layout(
                record,
                staged,
                removed_extents,
                generation,
                tx,
                data_writes,
            );
        }
        self.commit_file_layout(
            record,
            record_lba,
            old_extents,
            old_tree_blocks,
            new_extents,
            removed_extents,
            new_size,
            true,
            now,
            generation,
            tx,
            data_writes,
        )
    }

    /// Reserves physical blocks for a byte range without changing the file's
    /// logical size. New mappings carry the unwritten flag and therefore read
    /// as zeros until replaced by [`Self::write_file_at`].
    pub fn preallocate_file(
        &mut self,
        object_id: u64,
        offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::PreallocateFile, |volume| {
            volume.preallocate_file_untraced(object_id, offset, length, now)
        })
    }

    fn preallocate_file_untraced(
        &mut self,
        object_id: u64,
        offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.preallocate_file_bounded(
            object_id,
            offset,
            length,
            now,
            FileEditLimits {
                max_blocks: u64::MAX,
                max_records: usize::MAX - 1,
            },
        )
    }

    /// Reserve using local extent edits with explicit block and record budgets.
    pub fn preallocate_file_bounded(
        &mut self,
        object_id: u64,
        offset: u64,
        length: u64,
        now: Timespec,
        limits: FileEditLimits,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::PreallocateFileBounded, |volume| {
            volume.preallocate_file_bounded_untraced(object_id, offset, length, now, limits)
        })
    }

    fn preallocate_file_bounded_untraced(
        &mut self,
        object_id: u64,
        offset: u64,
        length: u64,
        now: Timespec,
        limits: FileEditLimits,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        if limits.max_blocks == 0 || limits.max_records == 0 || limits.max_records == usize::MAX {
            return Err(CoreError::PrototypeLimit("file edit limits invalid"));
        }
        self.ensure_window_closed()?;
        if length == 0 {
            return Ok(());
        }
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let end_offset = offset
            .checked_add(length)
            .ok_or(CoreError::PrototypeLimit("preallocation range overflows"))?;
        let block_size = self.dev.block_size() as u64;
        let start_block = offset / block_size;
        let end_block = end_offset.div_ceil(block_size);
        if end_block - start_block > limits.max_blocks {
            return Err(CoreError::PrototypeLimit(
                "file edit block budget exhausted",
            ));
        }
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {object_id} missing from object map"))
        })?;
        let old_extents = if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            extent_map::read_window(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                object_id,
                self.checkpoint.generation,
                start_block,
                end_block,
                limits.max_records,
            )?
        } else {
            self.load_file_layout(&record)?.0
        };
        let holes = logical_holes(&old_extents, start_block, end_block)?;
        if holes.is_empty() {
            return Ok(());
        }

        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);
        let mut additions = Vec::new();
        for (logical_start, logical_end) in holes {
            additions.extend(allocate_extent_runs_bounded(
                &mut tx,
                &mut self.dev,
                &self.ident.geometry(),
                logical_start,
                logical_end - logical_start,
                EXTENT_UNWRITTEN,
                limits.max_records.saturating_sub(additions.len()),
            )?);
        }
        let mut new_extents = old_extents.clone();
        new_extents.extend(additions);
        let new_extents = coalesce_extents(new_extents)?;
        if new_extents.len() > limits.max_records {
            return Err(CoreError::PrototypeLimit(
                "file edit result record budget exhausted",
            ));
        }
        let staged = if record.flags & OBJECT_FLAG_EXTENT_TREE == 0 {
            self.stage_file_layout(
                &mut tx,
                record,
                record_lba,
                &old_extents,
                &[],
                &new_extents,
                record.size_bytes,
                false,
                now,
                now,
                generation,
            )?
        } else {
            self.stage_extent_delta(
                &mut tx,
                record,
                record_lba,
                &old_extents,
                &new_extents,
                record.size_bytes,
                false,
                now,
                generation,
            )?
        };
        self.commit_staged_file_layout(record, staged, Vec::new(), generation, tx, Vec::new())
    }

    /// Clones a committed file into a new directory entry that shares every
    /// data run with the source (ADR-027 `CloneFile`, mechanism ADR-061).
    /// Clone is a checkpoint transaction, never an intent-log operation: an
    /// open window is committed first so the clone cannot straddle the two
    /// durability mechanisms. Both sides' maps carry `EXTENT_SHARED`
    /// afterwards, and a direct-layout source is promoted to an extent tree
    /// because the direct representation has no flag word.
    pub fn clone_file(
        &mut self,
        source_id: u64,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::CloneFile, |volume| {
            volume.clone_file_untraced(source_id, parent_id, name, now)
        })
    }

    fn clone_file_untraced(
        &mut self,
        source_id: u64,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        // The feature gate comes before every side effect: refusing a clone
        // must not have committed an open window first (F12).
        if !self.shared_extents_enabled() {
            return Err(CoreError::FeatureDisabled(
                "shared-extents feature is not enabled on this volume",
            ));
        }
        if self.window.is_some() {
            self.window_commit(now)?;
        }
        self.ensure_window_closed()?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = self.comparison_key(name.as_bytes())?;
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }
        let source = self.read_object(source_id)?.ok_or(CoreError::NotFound)?;
        if source.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let source_record_lba = self.object_record_lba(source_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {source_id} missing from object map"))
        })?;
        let (source_extents, source_tree_blocks) = self.load_file_layout(&source)?;
        let source_was_tree = source.flags & OBJECT_FLAG_EXTENT_TREE != 0;

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let object_id = self.checkpoint.next_object_id;
        let next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;

        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);

        // One more reference per source run. A run shared for the first time
        // gains a record at two references (source + clone); an already
        // shared run is incremented, refusing overflow.
        self.shared_refs_edit(generation)?.require_root();
        for extent in &source_extents {
            self.shared_prefetch(generation, extent.physical_start, extent.block_count)?;
        }
        {
            let edit = self.shared_refs_edit(generation)?;
            for extent in &source_extents {
                edit.acquire(extent.physical_start, extent.block_count)?;
            }
        }
        let shared: Vec<Extent> = source_extents
            .iter()
            .map(|extent| Extent {
                flags: extent.flags | EXTENT_SHARED,
                ..*extent
            })
            .collect();

        let mut meta_writes = Vec::new();

        // Source side: same mapping, now flagged. The direct layout carries
        // no flag word, so a direct source is promoted to an extent tree in
        // this same transaction (and never collapses back, because
        // direct_layout refuses flagged extents).
        let source_new_record_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, source_record_lba)?;
        let (source_flags, source_data_root) = if shared.is_empty() {
            (source.flags, source.data_root)
        } else if source_was_tree {
            let encoded = shared
                .iter()
                .copied()
                .map(extent_map::encode_extent)
                .collect::<Result<Vec<_>, _>>()?;
            let operations: Vec<TreeOperation<'_>> = encoded
                .iter()
                .map(|(key, value)| TreeOperation::Upsert { key, value })
                .collect();
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                source.data_root,
                extent_map::spec(source_id, self.checkpoint.generation),
                generation,
                &operations,
            )?;
            meta_writes.extend(mutation.writes);
            (source.flags, mutation.root_lba)
        } else {
            self.pending_layout_promotions += 1;
            let node_count = extent_map::bulk_node_count(block_size, shared.len())?;
            let mut lbas = Vec::with_capacity(node_count);
            for _ in 0..node_count {
                lbas.push(tx.allocate(&mut self.dev)?);
            }
            let built = extent_map::bulk_build(source_id, block_size, &shared, &lbas)?;
            for (lba, node) in built.nodes {
                meta_writes.push((lba, node.encode(block_size, generation)?));
            }
            (source.flags | OBJECT_FLAG_EXTENT_TREE, built.root_lba)
        };
        let _ = source_tree_blocks; // COW originals are retired by mutate_many
                                    // Sharing is a layout fact: no user-visible field of the source
                                    // changes, so its change time stays.
        let source_new_record = ObjectRecord {
            flags: source_flags,
            data_root: source_data_root,
            ..source
        };

        // Destination: a distinct object with an independent map over the
        // same physical runs. An empty source clones to an empty file; any
        // mapped or tree source clones to a tree destination.
        let dest_record_lba = tx.allocate(&mut self.dev)?;
        let (dest_flags, dest_data_root, dest_data_blocks) =
            if shared.is_empty() && !source_was_tree {
                (0u16, 0u64, 0u64)
            } else {
                let node_count = extent_map::bulk_node_count(block_size, shared.len())?;
                let mut lbas = Vec::with_capacity(node_count);
                for _ in 0..node_count {
                    lbas.push(tx.allocate(&mut self.dev)?);
                }
                let built = extent_map::bulk_build(object_id, block_size, &shared, &lbas)?;
                for (lba, node) in built.nodes {
                    meta_writes.push((lba, node.encode(block_size, generation)?));
                }
                (OBJECT_FLAG_EXTENT_TREE, built.root_lba, source.data_blocks)
            };
        // The destination receives its own copy of the source's descriptor,
        // in new blocks allocated and published by this same transaction: a
        // chain has exactly one owner, so the two files never share segments.
        // A failed allocation fails the clone as a whole.
        let dest_security = self.copy_security_descriptor(
            &mut tx,
            &source,
            object_id,
            generation,
            &mut meta_writes,
        )?;
        // The attribute set travels the same way, in its own fresh chain.
        let dest_attributes =
            self.copy_attributes(&mut tx, &source, object_id, generation, &mut meta_writes)?;
        let dest_record = ObjectRecord {
            object_id,
            object_type: ObjectType::File,
            flags: dest_flags,
            link_count: 1,
            // A clone inherits its source's owner, as it inherits the
            // attribute set and the descriptor chain.
            owner_uid: source.owner_uid,
            owner_gid: source.owner_gid,
            size_bytes: source.size_bytes,
            allocated_bytes: source.allocated_bytes,
            created: now,
            modified: source.modified,
            changed: now,
            protection: source.protection,
            content_generation: generation,
            data_root: dest_data_root,
            data_blocks: dest_data_blocks,
            security: None,
            attributes: None,
            comment: afsplus_format::object::Comment::EMPTY,
        }
        .with_security(dest_security)
        .with_attributes(dest_attributes)
        // The comment describes the content and travels with a clone, as
        // the protection word does (ADR-106).
        .with_comment(source.comment);

        // Namespace: one new directory entry, parent record COW'd.
        let parent_record_new_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, parent_record_lba)?;
        let directory_entry = DirEntry {
            key,
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        };
        let (directory_key, directory_value) =
            directory::encode_entry(&self.ident, &directory_entry)?;
        let directory_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &directory_key,
                value: &directory_value,
            }],
        )?;
        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: directory_mutation.root_lba,
            ..parent
        };

        let source_key = object_map::key(source_id);
        let source_value = object_map::value(source_new_record_lba)?;
        let dest_key = object_map::key(object_id);
        let dest_value = object_map::value(dest_record_lba)?;
        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let operations = [
            TreeOperation::Upsert {
                key: &source_key,
                value: &source_value,
            },
            TreeOperation::Upsert {
                key: &dest_key,
                value: &dest_value,
            },
            TreeOperation::Upsert {
                key: &parent_key,
                value: &parent_value,
            },
        ];
        let omap_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        let omap_lba = omap_mutation.root_lba;

        meta_writes.push((
            source_new_record_lba,
            source_new_record.encode(block_size, generation)?,
        ));
        meta_writes.push((dest_record_lba, dest_record.encode(block_size, generation)?));
        meta_writes.push((
            parent_record_new_lba,
            new_parent.encode(block_size, generation)?,
        ));
        meta_writes.extend(directory_mutation.writes);
        meta_writes.extend(omap_mutation.writes);

        self.commit_transaction(
            generation,
            next_object_id,
            tx,
            Vec::new(),
            meta_writes,
            omap_lba,
        )?;
        Ok(object_id)
    }

    /// Replaces a byte range in an existing destination file with a reflink
    /// to the corresponding source range (ADR-027/ADR-061).
    ///
    /// Offsets must have the same position within a filesystem block.  This
    /// makes every complete interior block physically shareable; at most the
    /// first and last partial blocks are copied privately so bytes outside the
    /// requested range retain their old destination values.  Source holes
    /// remain destination holes for complete blocks.  Self-cloning is kept as
    /// an explicit prototype limit because overlapping edits need a separate
    /// reference-delta plan, not operation ordering by accident.
    #[allow(clippy::too_many_arguments)]
    pub fn clone_range(
        &mut self,
        source_id: u64,
        source_offset: u64,
        destination_id: u64,
        destination_offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::CloneRange, |volume| {
            volume.clone_range_untraced(
                source_id,
                source_offset,
                destination_id,
                destination_offset,
                length,
                now,
            )
        })
    }

    fn clone_range_untraced(
        &mut self,
        source_id: u64,
        source_offset: u64,
        destination_id: u64,
        destination_offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        if !self.shared_extents_enabled() {
            return Err(CoreError::FeatureDisabled(
                "shared-extents feature is not enabled on this volume",
            ));
        }
        if self.window.is_some() {
            self.window_commit(now)?;
        }
        self.ensure_window_closed()?;

        let source = self.read_object(source_id)?.ok_or(CoreError::NotFound)?;
        let destination = self
            .read_object(destination_id)?
            .ok_or(CoreError::NotFound)?;
        if source.object_type != ObjectType::File || destination.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        if source_id == destination_id {
            return Err(CoreError::PrototypeLimit(
                "same-file range cloning is not implemented",
            ));
        }
        let source_end = source_offset
            .checked_add(length)
            .ok_or(CoreError::PrototypeLimit("clone source range overflows"))?;
        let destination_end =
            destination_offset
                .checked_add(length)
                .ok_or(CoreError::PrototypeLimit(
                    "clone destination range overflows",
                ))?;
        if source_end > source.size_bytes {
            return Err(CoreError::PrototypeLimit(
                "clone source range exceeds the file size",
            ));
        }
        if length == 0 {
            return Ok(());
        }

        let block_size = self.dev.block_size() as u64;
        if source_offset % block_size != destination_offset % block_size {
            return Err(CoreError::PrototypeLimit(
                "clone range offsets must have matching block alignment",
            ));
        }
        let source_record_lba = self.object_record_lba(source_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {source_id} missing from object map"))
        })?;
        let destination_record_lba = self.object_record_lba(destination_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {destination_id} missing from object map"))
        })?;
        let (source_extents, source_tree_blocks) = self.load_file_layout(&source)?;
        let (destination_extents, destination_tree_blocks) = self.load_file_layout(&destination)?;

        let destination_first_block = destination_offset / block_size;
        let destination_end_block = destination_end.div_ceil(block_size);
        let leading = if destination_offset.is_multiple_of(block_size) {
            0
        } else {
            block_size - destination_offset % block_size
        };
        let full_destination_start = destination_offset
            .checked_add(leading)
            .ok_or(CoreError::PrototypeLimit(
                "clone destination range overflows",
            ))?
            .min(destination_end);
        let full_destination_end = destination_end - destination_end % block_size;
        let has_full_blocks = full_destination_start < full_destination_end;
        let (source_full_start_block, source_full_end_block) = if has_full_blocks {
            let source_full_start = source_offset
                .checked_add(full_destination_start - destination_offset)
                .ok_or(CoreError::PrototypeLimit("clone source range overflows"))?;
            let source_full_end = source_full_start
                .checked_add(full_destination_end - full_destination_start)
                .ok_or(CoreError::PrototypeLimit("clone source range overflows"))?;
            (source_full_start / block_size, source_full_end / block_size)
        } else {
            (0, 0)
        };
        let full_destination_start_block = full_destination_start / block_size;

        let source_new_extents = mark_logical_range_shared(
            &source_extents,
            source_full_start_block,
            source_full_end_block,
        )?;
        let shared_destination_extents = remap_extent_range(
            &source_extents,
            source_full_start_block,
            source_full_end_block,
            full_destination_start_block,
        )?;
        let (mut destination_new_extents, removed_destination_extents) = replace_logical_range(
            &destination_extents,
            destination_first_block,
            destination_end_block,
            None,
        )?;

        // Snapshot the at-most-two boundary blocks before starting the
        // transaction.  This also gives memmove-like source semantics even
        // though same-file cloning is currently rejected explicitly.
        let mut private_blocks = Vec::new();
        for logical_block in destination_first_block..destination_end_block {
            if has_full_blocks
                && logical_block >= full_destination_start_block
                && logical_block < full_destination_end / block_size
            {
                continue;
            }
            let logical_byte = logical_block
                .checked_mul(block_size)
                .ok_or(CoreError::PrototypeLimit("clone block offset overflows"))?;
            let mut block = vec![0u8; block_size as usize];
            self.read_layout_block(&destination_extents, logical_block, &mut block)?;
            let copy_start = destination_offset.max(logical_byte);
            let copy_end = destination_end.min(
                logical_byte
                    .checked_add(block_size)
                    .ok_or(CoreError::PrototypeLimit("clone block offset overflows"))?,
            );
            let mut cursor = copy_start;
            while cursor < copy_end {
                let source_byte = source_offset
                    .checked_add(cursor - destination_offset)
                    .ok_or(CoreError::PrototypeLimit("clone source range overflows"))?;
                let source_block = source_byte / block_size;
                let source_in_block = source_byte % block_size;
                let count = (copy_end - cursor).min(block_size - source_in_block);
                let mut source_bytes = vec![0u8; block_size as usize];
                self.read_layout_block(&source_extents, source_block, &mut source_bytes)?;
                let target_start = usize::try_from(cursor - logical_byte)
                    .map_err(|_| CoreError::PrototypeLimit("clone offset is too large"))?;
                let source_start = usize::try_from(source_in_block)
                    .map_err(|_| CoreError::PrototypeLimit("clone offset is too large"))?;
                let count = usize::try_from(count)
                    .map_err(|_| CoreError::PrototypeLimit("clone length is too large"))?;
                block[target_start..target_start + count]
                    .copy_from_slice(&source_bytes[source_start..source_start + count]);
                cursor += count as u64;
            }
            private_blocks.push((logical_block, block));
        }

        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);
        self.shared_refs_edit(generation)?.require_root();
        for extent in &shared_destination_extents {
            self.shared_prefetch(generation, extent.physical_start, extent.block_count)?;
        }
        for extent in &removed_destination_extents {
            if extent.flags & EXTENT_SHARED != 0 {
                self.shared_prefetch(generation, extent.physical_start, extent.block_count)?;
            }
        }
        {
            let edit = self.shared_refs_edit(generation)?;
            for extent in &shared_destination_extents {
                edit.acquire(extent.physical_start, extent.block_count)?;
            }
        }

        let mut data_writes = Vec::with_capacity(private_blocks.len());
        for (logical_block, block) in private_blocks {
            let physical_start = tx.allocate(&mut self.dev)?;
            destination_new_extents.push(Extent {
                logical_start: logical_block,
                physical_start,
                block_count: 1,
                flags: 0,
            });
            data_writes.push((physical_start, block));
        }
        destination_new_extents.extend(shared_destination_extents);
        let destination_new_extents = coalesce_extents(destination_new_extents)?;
        for extent in &removed_destination_extents {
            self.release_data_run(&mut tx, generation, extent)?;
        }

        let mut metadata_writes = Vec::new();
        let mut object_updates = Vec::new();
        if source_new_extents != source_extents {
            if source.flags & OBJECT_FLAG_EXTENT_TREE == 0 {
                self.pending_layout_promotions += 1;
            }
            let staged = self.stage_file_layout(
                &mut tx,
                source,
                source_record_lba,
                &source_extents,
                &source_tree_blocks,
                &source_new_extents,
                source.size_bytes,
                false,
                source.modified,
                // Sharing is a layout fact: the source's change time stays.
                source.changed,
                generation,
            )?;
            metadata_writes.extend(staged.metadata_writes);
            object_updates.push((
                object_map::key(source_id),
                object_map::value(staged.record_lba)?,
            ));
        }
        if destination.flags & OBJECT_FLAG_EXTENT_TREE == 0
            && direct_layout(
                &destination_new_extents,
                destination.size_bytes.max(destination_end),
                block_size,
            )
            .is_none()
        {
            self.pending_layout_promotions += 1;
        }
        let staged_destination = self.stage_file_layout(
            &mut tx,
            destination,
            destination_record_lba,
            &destination_extents,
            &destination_tree_blocks,
            &destination_new_extents,
            destination.size_bytes.max(destination_end),
            true,
            now,
            now,
            generation,
        )?;
        metadata_writes.extend(staged_destination.metadata_writes);
        object_updates.push((
            object_map::key(destination_id),
            object_map::value(staged_destination.record_lba)?,
        ));
        let object_operations: Vec<TreeOperation<'_>> = object_updates
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &object_operations,
        )?;
        metadata_writes.extend(object_map_mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            data_writes,
            metadata_writes,
            object_map_mutation.root_lba,
        )
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    pub fn into_device(self) -> D {
        self.dev
    }

    /// Creates a file with `content` in the root directory.
    pub fn create_file_in_root(
        &mut self,
        name: &str,
        content: &[u8],
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::CreateFileInRoot, |volume| {
            volume.create_file_in_root_untraced(name, content, now)
        })
    }

    fn create_file_in_root_untraced(
        &mut self,
        name: &str,
        content: &[u8],
        now: Timespec,
    ) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        self.create_file_in_directory(OBJECT_ROOT, name, content, now)
    }

    /// Creates a file in an arbitrary directory identified by object ID.
    pub fn create_file_in_directory(
        &mut self,
        parent_id: u64,
        name: &str,
        content: &[u8],
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::CreateFileInDirectory, |volume| {
            volume.create_file_in_directory_untraced(parent_id, name, content, now)
        })
    }

    fn create_file_in_directory_untraced(
        &mut self,
        parent_id: u64,
        name: &str,
        content: &[u8],
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.create_leaf_in_directory(parent_id, name, content, None, now)
    }

    /// Store an opaque UTF-8 target without resolving it.
    pub fn create_symlink(
        &mut self,
        parent_id: u64,
        name: &str,
        target: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::CreateSymlink, |volume| {
            volume.create_symlink_untraced(parent_id, name, target, now)
        })
    }

    fn create_symlink_untraced(
        &mut self,
        parent_id: u64,
        name: &str,
        target: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        if target.is_empty()
            || target.as_bytes().contains(&0)
            || target.len()
                > afsplus_format::object::SymlinkRecord::maximum_target_bytes(self.dev.block_size())
        {
            return Err(CoreError::InvalidMetadata("invalid symlink target"));
        }
        self.create_leaf_in_directory(parent_id, name, &[], Some(target), now)
    }

    fn create_leaf_in_directory(
        &mut self,
        parent_id: u64,
        name: &str,
        content: &[u8],
        target: Option<&str>,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        self.ensure_public_object_id(parent_id)?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = self.comparison_key(name.as_bytes())?;
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let object_id = self.checkpoint.next_object_id;
        let next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;

        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);

        // Data first: allocate one contiguous extent and stage its blocks.
        let data_block_count = (content.len() as u64).div_ceil(block_size as u64);
        let mut data_writes = Vec::new();
        let data_start = if data_block_count > 0 {
            let start = tx.allocate_run(&mut self.dev, data_block_count)?;
            for i in 0..data_block_count as usize {
                let mut block = vec![0u8; block_size];
                let from = i * block_size;
                let to = content.len().min(from + block_size);
                block[..to - from].copy_from_slice(&content[from..to]);
                data_writes.push((start + i as u64, block));
            }
            start
        } else {
            0
        };

        // Fresh blocks for every COW'd structure.
        let file_record_lba = tx.allocate(&mut self.dev)?;
        let parent_record_new_lba = tx.allocate(&mut self.dev)?;

        // Everything the new state no longer reaches goes into quarantine.
        tx.retire(&mut self.dev, parent_record_lba)?;

        let file_record = ObjectRecord {
            object_id,
            object_type: if target.is_some() {
                ObjectType::Symlink
            } else {
                ObjectType::File
            },
            owner_uid: 0,
            owner_gid: 0,
            flags: 0,
            link_count: 1,
            size_bytes: target.map_or(content.len(), str::len) as u64,
            allocated_bytes: data_block_count * block_size as u64,
            created: now,
            modified: now,
            changed: now,
            protection: 0,
            content_generation: generation,
            data_root: data_start,
            data_blocks: data_block_count,
            security: None,
            attributes: None,
            comment: afsplus_format::object::Comment::EMPTY,
        };

        let directory_entry = DirEntry {
            key,
            name: name.as_bytes().to_vec(),
            child_type_hint: if target.is_some() { 3 } else { 1 },
            child_id: object_id,
        };
        let (directory_key, directory_value) =
            directory::encode_entry(&self.ident, &directory_entry)?;
        let directory_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &directory_key,
                value: &directory_value,
            }],
        )?;

        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: directory_mutation.root_lba,
            ..parent
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let file_key = object_map::key(object_id);
        let file_value = object_map::value(file_record_lba)?;
        let operations = [
            TreeOperation::Upsert {
                key: &parent_key,
                value: &parent_value,
            },
            TreeOperation::Upsert {
                key: &file_key,
                value: &file_value,
            },
        ];
        let omap_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        let omap_lba = omap_mutation.root_lba;

        let mut meta_writes = vec![
            (
                file_record_lba,
                match target {
                    Some(target) => afsplus_format::object::SymlinkRecord {
                        record: file_record,
                        target,
                    }
                    .encode(block_size, generation)?,
                    None => file_record.encode(block_size, generation)?,
                },
            ),
            (
                parent_record_new_lba,
                new_parent.encode(block_size, generation)?,
            ),
        ];
        meta_writes.extend(directory_mutation.writes);
        meta_writes.extend(omap_mutation.writes);

        self.commit_transaction(
            generation,
            next_object_id,
            tx,
            data_writes,
            meta_writes,
            omap_lba,
        )?;
        Ok(object_id)
    }

    /// Creates a directory in the root directory.
    pub fn create_directory_in_root(
        &mut self,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::CreateDirectoryInRoot, |volume| {
            volume.create_directory_in_root_untraced(name, now)
        })
    }

    fn create_directory_in_root_untraced(
        &mut self,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        self.create_directory(OBJECT_ROOT, name, now)
    }

    /// The record of a directory that has just been created, empty, with its
    /// entry tree rooted at `root_lba`. Three paths make one and must agree:
    /// [`Self::create_directory`] in its own transaction, the orphan
    /// directory inside a batch, and [`BatchOp::CreateDirectory`] in the open
    /// window. The block at `root_lba` holds [`directory::empty_leaf`]; the
    /// immediate path writes it itself, a batch lets its materialization
    /// stage it.
    fn new_directory_record(
        object_id: u64,
        root_lba: u64,
        now: Timespec,
        generation: u64,
    ) -> ObjectRecord {
        ObjectRecord {
            object_id,
            object_type: ObjectType::Directory,
            flags: 0,
            link_count: 1,
            owner_uid: 0,
            owner_gid: 0,
            size_bytes: 0,
            allocated_bytes: 0,
            created: now,
            modified: now,
            changed: now,
            protection: 0,
            content_generation: generation,
            data_root: root_lba,
            data_blocks: 0,
            security: None,
            attributes: None,
            comment: afsplus_format::object::Comment::EMPTY,
        }
    }

    /// Creates an empty directory in an arbitrary parent directory.
    pub fn create_directory(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::CreateDirectory, |volume| {
            volume.create_directory_untraced(parent_id, name, now)
        })
    }

    fn create_directory_untraced(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        self.ensure_public_object_id(parent_id)?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let object_id = self.checkpoint.next_object_id;
        let next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;

        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);
        let directory_root_lba = tx.allocate(&mut self.dev)?;
        let directory_record_lba = tx.allocate(&mut self.dev)?;
        let parent_record_new_lba = tx.allocate(&mut self.dev)?;

        tx.retire(&mut self.dev, parent_record_lba)?;

        let new_directory =
            Self::new_directory_record(object_id, directory_root_lba, now, generation);

        let entry = DirEntry {
            key: self.comparison_key(name.as_bytes())?,
            name: name.as_bytes().to_vec(),
            child_type_hint: 2,
            child_id: object_id,
        };
        let (directory_key, directory_value) = directory::encode_entry(&self.ident, &entry)?;
        let parent_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &directory_key,
                value: &directory_value,
            }],
        )?;
        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: parent_mutation.root_lba,
            ..parent
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let directory_object_key = object_map::key(object_id);
        let directory_object_value = object_map::value(directory_record_lba)?;
        let operations = [
            TreeOperation::Upsert {
                key: &parent_key,
                value: &parent_value,
            },
            TreeOperation::Upsert {
                key: &directory_object_key,
                value: &directory_object_value,
            },
        ];
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;

        let mut metadata_writes = vec![
            (
                directory_root_lba,
                directory::empty_leaf(object_id).encode(block_size, generation)?,
            ),
            (
                directory_record_lba,
                new_directory.encode(block_size, generation)?,
            ),
            (
                parent_record_new_lba,
                new_parent.encode(block_size, generation)?,
            ),
        ];
        metadata_writes.extend(parent_mutation.writes);
        metadata_writes.extend(object_map_mutation.writes);

        self.commit_transaction(
            generation,
            next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            object_map_mutation.root_lba,
        )?;
        Ok(object_id)
    }

    /// Deletes a file from the root directory. Its record and data blocks are
    /// retired, not freed: they stay quarantined until no still-selectable
    /// checkpoint can reference them.
    pub fn delete_file_in_root(&mut self, name: &str, now: Timespec) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::DeleteFileInRoot, |volume| {
            volume.delete_file_in_root_untraced(name, now)
        })
    }

    fn delete_file_in_root_untraced(&mut self, name: &str, now: Timespec) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.delete_file(OBJECT_ROOT, name, now)
    }

    /// Deletes a file link from an arbitrary directory.
    pub fn delete_file(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::DeleteFile, |volume| {
            volume.delete_file_untraced(parent_id, name, now)
        })
    }

    fn delete_file_untraced(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_public_object_id(parent_id)?;
        self.remove_entry(parent_id, name, now, ObjectType::File)
    }

    /// Removes an empty child directory from an arbitrary parent.
    pub fn remove_directory(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::RemoveDirectory, |volume| {
            volume.remove_directory_untraced(parent_id, name, now)
        })
    }

    fn remove_directory_untraced(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_public_object_id(parent_id)?;
        self.remove_entry(parent_id, name, now, ObjectType::Directory)
    }

    /// Moves the final visible link of an open regular file into the reserved
    /// orphan directory (ADR-066). The lazy directory setup, if needed, is a
    /// separate preparatory checkpoint; the visible-link removal and orphan
    /// insertion themselves are one atomic rename transaction.
    pub fn orphan_file(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::OrphanFile, |volume| {
            volume.orphan_file_untraced(parent_id, name, now)
        })
    }

    fn orphan_file_untraced(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        if !self.orphan_directory_enabled() {
            return Err(CoreError::FeatureDisabled(
                "orphan-directory feature is not enabled on this volume",
            ));
        }
        self.ensure_public_object_id(parent_id)?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let key = self.comparison_key(name.as_bytes())?;
        let entry = directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            parent.data_root,
            parent_id,
            self.checkpoint.generation,
            &self.ident,
            &key,
        )?
        .ok_or(CoreError::NotFound)?;
        let victim = self
            .read_object(entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("orphan victim missing from object map".into()))?;
        if entry.child_type_hint != 1 || victim.object_type != ObjectType::File {
            return Err(if victim.object_type == ObjectType::Directory {
                CoreError::IsDirectory
            } else {
                CoreError::Corrupt("orphan victim is not a regular file".into())
            });
        }
        if victim.link_count != 1 {
            return Err(CoreError::InvalidMove(
                "only a file's final visible link can enter the orphan directory",
            ));
        }

        self.ensure_orphan_directory(now)?;
        let orphan_name = Self::orphan_name(victim.object_id);
        if self.orphan_object(victim.object_id)? {
            return Err(CoreError::Corrupt(format!(
                "object {} already has an orphan entry",
                victim.object_id
            )));
        }
        self.rename_internal(parent_id, name, OBJECT_ORPHAN_DIRECTORY, &orphan_name, now)?;
        Ok(victim.object_id)
    }

    /// Whether the reserved directory currently names `object_id`. This is
    /// a bounded point lookup, used by adapters to keep guessed object IDs
    /// from exposing open-unlinked files.
    pub fn orphan_object(&mut self, object_id: u64) -> Result<bool, CoreError> {
        self.trace_api(crate::flight::ApiMethod::OrphanObject, |volume| {
            volume.orphan_object_untraced(object_id)
        })
    }

    fn orphan_object_untraced(&mut self, object_id: u64) -> Result<bool, CoreError> {
        if !self.orphan_directory_enabled() {
            return Ok(false);
        }
        let Some(orphan_directory) = self.read_object(OBJECT_ORPHAN_DIRECTORY)? else {
            return Ok(false);
        };
        self.validate_orphan_directory(orphan_directory)?;
        let name = Self::orphan_name(object_id);
        let key = self.comparison_key(name.as_bytes())?;
        let entry = directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            orphan_directory.data_root,
            OBJECT_ORPHAN_DIRECTORY,
            self.checkpoint.generation,
            &self.ident,
            &key,
        )?;
        let Some(entry) = entry else {
            return Ok(false);
        };
        if entry.child_id != object_id
            || entry.child_type_hint != 1
            || entry.name.as_slice() != name.as_bytes()
        {
            return Err(CoreError::Corrupt(format!(
                "orphan entry for object {object_id} has invalid identity or type"
            )));
        }
        Ok(true)
    }

    /// Returns the number of persistent orphan entries using only the
    /// reserved directory root and its authoritative subtree item count.
    pub fn orphan_count(&mut self) -> Result<u64, CoreError> {
        self.trace_api(crate::flight::ApiMethod::OrphanCount, |volume| {
            volume.orphan_count_untraced()
        })
    }

    fn orphan_count_untraced(&mut self) -> Result<u64, CoreError> {
        if !self.orphan_directory_enabled() {
            return Ok(0);
        }
        let Some(record) = self.read_object(OBJECT_ORPHAN_DIRECTORY)? else {
            return Ok(0);
        };
        self.validate_orphan_directory(record)?;
        Ok(crate::tree::read_range(
            &mut self.dev,
            &self.ident.geometry(),
            record.data_root,
            directory::spec(OBJECT_ORPHAN_DIRECTORY, self.checkpoint.generation),
            0,
            0,
        )?
        .total_items)
    }

    /// Returns the first orphan ID without scanning the object map or the
    /// complete internal directory.
    pub fn first_orphan(&mut self) -> Result<Option<u64>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::FirstOrphan, |volume| {
            volume.first_orphan_untraced()
        })
    }

    fn first_orphan_untraced(&mut self) -> Result<Option<u64>, CoreError> {
        if !self.orphan_directory_enabled() {
            return Ok(None);
        }
        let Some(record) = self.read_object(OBJECT_ORPHAN_DIRECTORY)? else {
            return Ok(None);
        };
        self.validate_orphan_directory(record)?;
        let page = directory::read_page(
            &mut self.dev,
            &self.ident.geometry(),
            record.data_root,
            directory::spec(OBJECT_ORPHAN_DIRECTORY, self.checkpoint.generation),
            &self.ident,
            0,
            1,
        )?;
        let Some(entry) = page.0.into_iter().next() else {
            return Ok(None);
        };
        let expected_name = Self::orphan_name(entry.child_id);
        if entry.child_type_hint != 1 || entry.name.as_slice() != expected_name.as_bytes() {
            return Err(CoreError::Corrupt(format!(
                "orphan entry for object {} has invalid name or type",
                entry.child_id
            )));
        }
        Ok(Some(entry.child_id))
    }

    /// Advances one orphan by at most the configured number of logical
    /// extent records. Each shrinking step and the final object removal are
    /// separate valid checkpoints, so a crash merely selects an earlier or
    /// later restart point.
    pub fn cleanup_orphan(
        &mut self,
        object_id: u64,
        now: Timespec,
    ) -> Result<OrphanCleanupProgress, CoreError> {
        self.trace_api(crate::flight::ApiMethod::CleanupOrphan, |volume| {
            volume.cleanup_orphan_untraced(object_id, now)
        })
    }

    fn cleanup_orphan_untraced(
        &mut self,
        object_id: u64,
        now: Timespec,
    ) -> Result<OrphanCleanupProgress, CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        if !self.orphan_object(object_id)? {
            return Ok(OrphanCleanupProgress::default());
        }
        let step =
            self.cleanup_orphan_data_step(object_id, self.orphan_cleanup_extent_budget, now)?;
        if !step.data_empty {
            return Ok(OrphanCleanupProgress {
                extents_removed: step.extents_removed,
                object_removed: false,
                still_pending: true,
            });
        }
        let name = Self::orphan_name(object_id);
        self.remove_entry(OBJECT_ORPHAN_DIRECTORY, &name, now, ObjectType::File)?;
        Ok(OrphanCleanupProgress {
            extents_removed: step.extents_removed,
            object_removed: true,
            still_pending: false,
        })
    }

    fn cleanup_orphan_data_step(
        &mut self,
        object_id: u64,
        limit: usize,
        now: Timespec,
    ) -> Result<OrphanDataStep, CoreError> {
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File || record.link_count != 1 {
            return Err(CoreError::Corrupt(format!(
                "orphan object {object_id} is not a singly-linked regular file"
            )));
        }
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("orphan object {object_id} missing from object map"))
        })?;
        let (removed, total_extents) = if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            let tail = extent_map::read_tail(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                object_id,
                self.checkpoint.generation,
                limit,
            )?;
            (tail.extents, tail.total_extents)
        } else if record.data_blocks == 0 {
            (Vec::new(), 0)
        } else {
            (
                vec![Extent {
                    logical_start: 0,
                    physical_start: record.data_root,
                    block_count: record.data_blocks,
                    flags: 0,
                }],
                1,
            )
        };
        if removed.is_empty() {
            return Ok(OrphanDataStep {
                extents_removed: 0,
                data_empty: total_extents == 0,
            });
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks
                .max(MIN_ORPHAN_CLEANUP_RECLAIM_BLOCKS),
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        let new_record_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, record_lba)?;

        let removed_blocks = removed.iter().try_fold(0u64, |total, extent| {
            total
                .checked_add(extent.block_count)
                .ok_or(CoreError::PrototypeLimit(
                    "orphan cleanup block count overflow",
                ))
        })?;
        let remaining_blocks = record
            .data_blocks
            .checked_sub(removed_blocks)
            .ok_or_else(|| CoreError::Corrupt("orphan extent count exceeds record".into()))?;
        for extent in &removed {
            self.release_data_run(&mut tx, generation, extent)?;
        }

        let (data_root, mut metadata_writes) = if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            let keys = removed
                .iter()
                .map(|extent| extent_map::encode_extent(*extent).map(|(key, _)| key))
                .collect::<Result<Vec<_>, _>>()?;
            let operations = keys
                .iter()
                .map(|key| TreeOperation::Delete { key })
                .collect::<Vec<_>>();
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                record.data_root,
                extent_map::spec(object_id, self.checkpoint.generation),
                generation,
                &operations,
            )?;
            (mutation.root_lba, mutation.writes)
        } else {
            (0, Vec::new())
        };
        let new_size = if remaining_blocks == 0 {
            0
        } else {
            record.size_bytes.min(
                removed[0]
                    .logical_start
                    .checked_mul(block_size as u64)
                    .ok_or(CoreError::PrototypeLimit("orphan cleanup size overflow"))?,
            )
        };
        let new_record = ObjectRecord {
            size_bytes: new_size,
            allocated_bytes: remaining_blocks
                .checked_mul(block_size as u64)
                .ok_or(CoreError::PrototypeLimit("orphan allocated size overflow"))?,
            modified: now,
            changed: now,
            content_generation: generation,
            data_root,
            data_blocks: remaining_blocks,
            ..record
        };

        let map_key = object_map::key(object_id);
        let map_value = object_map::value(new_record_lba)?;
        let map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &map_key,
                value: &map_value,
            }],
        )?;
        metadata_writes.push((new_record_lba, new_record.encode(block_size, generation)?));
        metadata_writes.extend(map_mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            map_mutation.root_lba,
        )?;

        Ok(OrphanDataStep {
            extents_removed: removed.len(),
            data_empty: total_extents == removed.len() as u64,
        })
    }

    fn ensure_orphan_directory(&mut self, now: Timespec) -> Result<(), CoreError> {
        if let Some(record) = self.read_object(OBJECT_ORPHAN_DIRECTORY)? {
            return self.validate_orphan_directory(record);
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        let directory_root_lba = tx.allocate(&mut self.dev)?;
        let directory_record_lba = tx.allocate(&mut self.dev)?;
        let record = ObjectRecord {
            object_id: OBJECT_ORPHAN_DIRECTORY,
            object_type: ObjectType::Directory,
            flags: 0,
            link_count: 1,
            owner_uid: 0,
            owner_gid: 0,
            size_bytes: 0,
            allocated_bytes: 0,
            created: now,
            modified: now,
            changed: now,
            protection: 0,
            content_generation: generation,
            data_root: directory_root_lba,
            data_blocks: 0,
            security: None,
            attributes: None,
            comment: afsplus_format::object::Comment::EMPTY,
        };
        let map_key = object_map::key(OBJECT_ORPHAN_DIRECTORY);
        let map_value = object_map::value(directory_record_lba)?;
        let map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &map_key,
                value: &map_value,
            }],
        )?;
        let mut metadata_writes = vec![
            (
                directory_root_lba,
                directory::empty_leaf(OBJECT_ORPHAN_DIRECTORY).encode(block_size, generation)?,
            ),
            (directory_record_lba, record.encode(block_size, generation)?),
        ];
        metadata_writes.extend(map_mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            map_mutation.root_lba,
        )
    }

    fn validate_orphan_directory(&self, record: ObjectRecord) -> Result<(), CoreError> {
        if record.object_id != OBJECT_ORPHAN_DIRECTORY
            || record.object_type != ObjectType::Directory
            || record.flags != 0
            || record.link_count != 1
            || record.size_bytes != 0
            || record.allocated_bytes != 0
            || record.data_blocks != 0
        {
            return Err(CoreError::Corrupt(
                "reserved orphan-directory object has invalid metadata".into(),
            ));
        }
        Ok(())
    }

    fn orphan_name(object_id: u64) -> String {
        format!("{object_id:016x}")
    }

    fn remove_entry(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
        expected_type: ObjectType,
    ) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = self.comparison_key(name.as_bytes())?;
        let entry = directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            parent.data_root,
            parent_id,
            self.checkpoint.generation,
            &self.ident,
            &key,
        )?
        .ok_or(CoreError::NotFound)?;
        let victim = self
            .read_object(entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("victim missing from object map".into()))?;
        if !matches!(
            (entry.child_type_hint, victim.object_type),
            (1, ObjectType::File) | (2, ObjectType::Directory) | (3, ObjectType::Symlink)
        ) {
            return Err(CoreError::Corrupt(
                "directory entry type hint does not match victim object".into(),
            ));
        }
        if victim.object_type != expected_type {
            return Err(if victim.object_type == ObjectType::Directory {
                CoreError::IsDirectory
            } else {
                CoreError::NotDirectory
            });
        }
        let victim_directory_blocks = if victim.object_type == ObjectType::Directory {
            if victim.link_count != 1 {
                return Err(CoreError::Corrupt(
                    "directory hard links are not supported".into(),
                ));
            }
            let loaded = directory::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                victim.data_root,
                victim.object_id,
                self.checkpoint.generation,
                &self.ident,
            )?;
            if !loaded.entries.is_empty() {
                return Err(CoreError::DirectoryNotEmpty);
            }
            loaded.tree_blocks
        } else {
            Vec::new()
        };
        let keep_file_object = victim.object_type == ObjectType::File && victim.link_count > 1;
        let victim_extent_map = if victim.object_type == ObjectType::File
            && victim.flags & OBJECT_FLAG_EXTENT_TREE != 0
            && !keep_file_object
        {
            Some(extent_map::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                victim.data_root,
                victim.object_id,
                self.checkpoint.generation,
            )?)
        } else {
            None
        };

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;

        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);

        let parent_record_new_lba = tx.allocate(&mut self.dev)?;
        let victim_record_new_lba = if keep_file_object {
            Some(tx.allocate(&mut self.dev)?)
        } else {
            None
        };

        // Quarantine the object's storage and the COW'd originals.
        let victim_record_lba = self
            .object_record_lba(entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("victim missing from object map".into()))?;
        tx.retire(&mut self.dev, victim_record_lba)?;
        if !keep_file_object {
            self.retire_security_descriptor(&mut tx, &victim)?;
            self.retire_attributes(&mut tx, &victim)?;
            if let Some(map) = victim_extent_map {
                for lba in map.tree_blocks {
                    tx.retire(&mut self.dev, lba)?;
                }
                for extent in map.extents {
                    self.release_data_run(&mut tx, generation, &extent)?;
                }
            } else if victim.data_blocks > 0 {
                tx.retire_run(&mut self.dev, victim.data_root, victim.data_blocks)?;
            }
            for lba in victim_directory_blocks {
                tx.retire(&mut self.dev, lba)?;
            }
        }
        tx.retire(&mut self.dev, parent_record_lba)?;

        let directory_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Delete { key: &key }],
        )?;

        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: directory_mutation.root_lba,
            ..parent
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let victim_key = object_map::key(entry.child_id);
        let victim_value = victim_record_new_lba.map(object_map::value).transpose()?;
        let mut operations = vec![TreeOperation::Upsert {
            key: &parent_key,
            value: &parent_value,
        }];
        if let Some(value) = &victim_value {
            operations.push(TreeOperation::Upsert {
                key: &victim_key,
                value,
            });
        } else {
            operations.push(TreeOperation::Delete { key: &victim_key });
        }
        let omap_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        let omap_lba = omap_mutation.root_lba;

        let mut meta_writes = vec![(
            parent_record_new_lba,
            new_parent.encode(block_size, generation)?,
        )];
        if let Some(lba) = victim_record_new_lba {
            let new_victim = ObjectRecord {
                link_count: victim.link_count - 1,
                changed: now,
                ..victim
            };
            meta_writes.push((lba, new_victim.encode(block_size, generation)?));
        }
        meta_writes.extend(directory_mutation.writes);
        meta_writes.extend(omap_mutation.writes);

        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            meta_writes,
            omap_lba,
        )
    }

    /// Adds another directory link to an existing regular file and updates
    /// its authoritative link count in the same checkpoint transaction.
    pub fn link_file(
        &mut self,
        object_id: u64,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::LinkFile, |volume| {
            volume.link_file_untraced(object_id, parent_id, name, now)
        })
    }

    fn link_file_untraced(
        &mut self,
        object_id: u64,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        self.ensure_public_object_id(parent_id)?;
        self.ensure_public_object_id(object_id)?;
        if self.orphan_object(object_id)? {
            return Err(CoreError::NotFound);
        }
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let file = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if file.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let new_link_count = file
            .link_count
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("link count exhausted"))?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }
        let file_lba = self
            .object_record_lba(object_id)?
            .ok_or_else(|| CoreError::Corrupt("linked file missing from object map".into()))?;
        let parent_lba = self
            .object_record_lba(parent_id)?
            .ok_or_else(|| CoreError::Corrupt("link parent missing from object map".into()))?;

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);
        let file_new_lba = tx.allocate(&mut self.dev)?;
        let parent_new_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, file_lba)?;
        tx.retire(&mut self.dev, parent_lba)?;

        let entry = DirEntry {
            key: self.comparison_key(name.as_bytes())?,
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        };
        let (entry_key, entry_value) = directory::encode_entry(&self.ident, &entry)?;
        let directory_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &entry_key,
                value: &entry_value,
            }],
        )?;
        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: directory_mutation.root_lba,
            ..parent
        };
        let new_file = ObjectRecord {
            link_count: new_link_count,
            changed: now,
            ..file
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_new_lba)?;
        let file_key = object_map::key(object_id);
        let file_value = object_map::value(file_new_lba)?;
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[
                TreeOperation::Upsert {
                    key: &parent_key,
                    value: &parent_value,
                },
                TreeOperation::Upsert {
                    key: &file_key,
                    value: &file_value,
                },
            ],
        )?;

        let mut metadata_writes = vec![
            (file_new_lba, new_file.encode(block_size, generation)?),
            (parent_new_lba, new_parent.encode(block_size, generation)?),
        ];
        metadata_writes.extend(directory_mutation.writes);
        metadata_writes.extend(object_map_mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            object_map_mutation.root_lba,
        )
    }

    /// Atomically renames or moves one namespace entry without changing its
    /// stable object ID. Replacement of an existing destination is a separate
    /// future operation with an explicit contract.
    pub fn rename(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::Rename, |volume| {
            volume.rename_untraced(
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                now,
            )
        })
    }

    fn rename_untraced(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_public_object_id(source_parent_id)?;
        self.ensure_public_object_id(target_parent_id)?;
        self.rename_internal(
            source_parent_id,
            source_name,
            target_parent_id,
            target_name,
            now,
        )
    }

    fn rename_internal(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        validate_name(source_name.as_bytes()).map_err(CoreError::InvalidName)?;
        validate_name(target_name.as_bytes()).map_err(CoreError::InvalidName)?;
        let source_key = self.comparison_key(source_name.as_bytes())?;
        let target_key = self.comparison_key(target_name.as_bytes())?;

        let source_parent = self
            .read_object(source_parent_id)?
            .ok_or(CoreError::NotFound)?;
        if source_parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let target_parent = if target_parent_id == source_parent_id {
            source_parent
        } else {
            self.read_object(target_parent_id)?
                .ok_or(CoreError::NotFound)?
        };
        if target_parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }

        let source_entry = directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            source_parent.data_root,
            source_parent_id,
            self.checkpoint.generation,
            &self.ident,
            &source_key,
        )?
        .ok_or(CoreError::NotFound)?;
        let same_key = source_parent_id == target_parent_id && source_key == target_key;
        if same_key && source_entry.name == target_name.as_bytes() {
            return Ok(());
        }
        if !same_key
            && directory::lookup_entry(
                &mut self.dev,
                &self.ident.geometry(),
                target_parent.data_root,
                target_parent_id,
                self.checkpoint.generation,
                &self.ident,
                &target_key,
            )?
            .is_some()
        {
            return Err(CoreError::AlreadyExists);
        }
        let moved = self
            .read_object(source_entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("moved object missing from object map".into()))?;
        if !matches!(
            (source_entry.child_type_hint, moved.object_type),
            (1, ObjectType::File) | (2, ObjectType::Directory) | (3, ObjectType::Symlink)
        ) {
            return Err(CoreError::Corrupt(
                "source entry type hint does not match moved object".into(),
            ));
        }
        if moved.object_type == ObjectType::Directory
            && self.directory_reaches(moved.object_id, target_parent_id)?
        {
            return Err(CoreError::InvalidMove(
                "a directory cannot be moved into itself or a descendant",
            ));
        }

        let source_parent_lba = self
            .object_record_lba(source_parent_id)?
            .ok_or_else(|| CoreError::Corrupt("source parent missing from object map".into()))?;
        let target_parent_lba = if target_parent_id == source_parent_id {
            source_parent_lba
        } else {
            self.object_record_lba(target_parent_id)?
                .ok_or_else(|| CoreError::Corrupt("target parent missing from object map".into()))?
        };
        let moved_lba = self
            .object_record_lba(moved.object_id)?
            .ok_or_else(|| CoreError::Corrupt("moved object missing from object map".into()))?;

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        let source_parent_new_lba = tx.allocate(&mut self.dev)?;
        let target_parent_new_lba = if target_parent_id == source_parent_id {
            source_parent_new_lba
        } else {
            tx.allocate(&mut self.dev)?
        };
        let moved_new_lba = tx.allocate(&mut self.dev)?;

        tx.retire(&mut self.dev, source_parent_lba)?;
        if target_parent_id != source_parent_id {
            tx.retire(&mut self.dev, target_parent_lba)?;
        }
        tx.retire(&mut self.dev, moved_lba)?;

        let target_entry = DirEntry {
            key: target_key,
            name: target_name.as_bytes().to_vec(),
            child_type_hint: source_entry.child_type_hint,
            child_id: source_entry.child_id,
        };
        let (target_entry_key, target_entry_value) =
            directory::encode_entry(&self.ident, &target_entry)?;

        let (source_mutation, target_mutation) = if source_parent_id == target_parent_id {
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                source_parent.data_root,
                directory::spec(source_parent_id, self.checkpoint.generation),
                generation,
                &[
                    TreeOperation::Delete { key: &source_key },
                    TreeOperation::Upsert {
                        key: &target_entry_key,
                        value: &target_entry_value,
                    },
                ],
            )?;
            (mutation, None)
        } else {
            let source_mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                source_parent.data_root,
                directory::spec(source_parent_id, self.checkpoint.generation),
                generation,
                &[TreeOperation::Delete { key: &source_key }],
            )?;
            let target_mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                target_parent.data_root,
                directory::spec(target_parent_id, self.checkpoint.generation),
                generation,
                &[TreeOperation::Upsert {
                    key: &target_entry_key,
                    value: &target_entry_value,
                }],
            )?;
            (source_mutation, Some(target_mutation))
        };

        let new_source_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: source_mutation.root_lba,
            ..source_parent
        };
        let new_target_parent = target_mutation.as_ref().map(|mutation| ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: mutation.root_lba,
            ..target_parent
        });
        let new_moved = ObjectRecord {
            changed: now,
            ..moved
        };

        let source_parent_map_key = object_map::key(source_parent_id);
        let source_parent_map_value = object_map::value(source_parent_new_lba)?;
        let target_parent_map_key = object_map::key(target_parent_id);
        let target_parent_map_value = object_map::value(target_parent_new_lba)?;
        let moved_map_key = object_map::key(moved.object_id);
        let moved_map_value = object_map::value(moved_new_lba)?;
        let mut map_operations = vec![TreeOperation::Upsert {
            key: &source_parent_map_key,
            value: &source_parent_map_value,
        }];
        if target_parent_id != source_parent_id {
            map_operations.push(TreeOperation::Upsert {
                key: &target_parent_map_key,
                value: &target_parent_map_value,
            });
        }
        map_operations.push(TreeOperation::Upsert {
            key: &moved_map_key,
            value: &moved_map_value,
        });
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &map_operations,
        )?;

        let mut metadata_writes = vec![
            (
                source_parent_new_lba,
                new_source_parent.encode(block_size, generation)?,
            ),
            (
                moved_new_lba,
                self.encode_preserving_target(new_moved, generation)?,
            ),
        ];
        metadata_writes.extend(source_mutation.writes);
        if let (Some(record), Some(mutation)) = (new_target_parent, target_mutation) {
            metadata_writes.push((
                target_parent_new_lba,
                record.encode(block_size, generation)?,
            ));
            metadata_writes.extend(mutation.writes);
        }
        metadata_writes.extend(object_map_mutation.writes);

        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            object_map_mutation.root_lba,
        )
    }

    /// Returns whether `target_id` is `ancestor_id` or is reachable below it.
    /// This exhaustive guard is used only for directory moves; a future
    /// parent/reverse index may accelerate it without changing semantics.
    fn directory_reaches(&mut self, ancestor_id: u64, target_id: u64) -> Result<bool, CoreError> {
        let mut pending = vec![ancestor_id];
        let mut visited = BTreeSet::new();
        while let Some(directory_id) = pending.pop() {
            if directory_id == target_id {
                return Ok(true);
            }
            if !visited.insert(directory_id) {
                return Err(CoreError::Corrupt(
                    "directory graph contains a cycle or duplicate parent".into(),
                ));
            }
            let record = self.read_object(directory_id)?.ok_or_else(|| {
                CoreError::Corrupt(format!("directory {directory_id} is missing"))
            })?;
            if record.object_type != ObjectType::Directory {
                return Err(CoreError::Corrupt(format!(
                    "directory graph references non-directory object {directory_id}"
                )));
            }
            let loaded = directory::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                directory_id,
                self.checkpoint.generation,
                &self.ident,
            )?;
            for entry in loaded.entries {
                if entry.child_type_hint == 2 {
                    let child = self.read_object(entry.child_id)?.ok_or_else(|| {
                        CoreError::Corrupt(format!(
                            "directory {directory_id} references missing object {}",
                            entry.child_id
                        ))
                    })?;
                    if child.object_type != ObjectType::Directory {
                        return Err(CoreError::Corrupt(format!(
                            "directory {directory_id} has a bad type hint for object {}",
                            entry.child_id
                        )));
                    }
                    pending.push(entry.child_id);
                }
            }
        }
        Ok(false)
    }

    fn load_file_layout(
        &mut self,
        record: &ObjectRecord,
    ) -> Result<(Vec<Extent>, Vec<u64>), CoreError> {
        if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            let loaded = extent_map::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                record.object_id,
                self.checkpoint.generation,
            )?;
            Ok((loaded.extents, loaded.tree_blocks))
        } else if record.data_blocks == 0 {
            Ok((Vec::new(), Vec::new()))
        } else {
            Ok((
                vec![Extent {
                    logical_start: 0,
                    physical_start: record.data_root,
                    block_count: record.data_blocks,
                    flags: 0,
                }],
                Vec::new(),
            ))
        }
    }

    fn read_layout_block(
        &mut self,
        extents: &[Extent],
        logical_block: u64,
        block: &mut [u8],
    ) -> Result<(), CoreError> {
        let after = extents.partition_point(|extent| extent.logical_start <= logical_block);
        let Some(extent) = after.checked_sub(1).map(|index| extents[index]) else {
            return Ok(());
        };
        if logical_block >= extent.logical_end()? || extent.flags & EXTENT_UNWRITTEN != 0 {
            return Ok(());
        }
        let lba = extent
            .physical_start
            .checked_add(logical_block - extent.logical_start)
            .ok_or_else(|| CoreError::Corrupt("mapped physical block overflows".into()))?;
        self.dev.read_block(lba, block)?;
        Ok(())
    }

    /// Stages one file record and its extent-map update inside an existing
    /// transaction.  The caller remains responsible for shared-reference
    /// edits, retiring removed data mappings, updating the object map and
    /// publishing the transaction.  Keeping those steps outside is what lets
    /// CloneRange publish the source map, destination map and reference tree
    /// atomically rather than nesting two one-file commits.
    #[allow(clippy::too_many_arguments)]
    fn stage_file_layout(
        &mut self,
        tx: &mut TxAllocator,
        record: ObjectRecord,
        record_lba: u64,
        old_extents: &[Extent],
        old_tree_blocks: &[u64],
        new_extents: &[Extent],
        new_size: u64,
        content_changed: bool,
        modified: Timespec,
        changed: Timespec,
        generation: u64,
    ) -> Result<StagedFileLayout, CoreError> {
        let block_size = self.dev.block_size();
        let allocated_blocks = new_extents.iter().try_fold(0u64, |total, extent| {
            total
                .checked_add(extent.block_count)
                .ok_or(CoreError::PrototypeLimit("allocated block count overflow"))
        })?;
        let direct = direct_layout(new_extents, new_size, block_size as u64);
        let was_tree = record.flags & OBJECT_FLAG_EXTENT_TREE != 0;
        let mut metadata_writes = Vec::new();
        let (flags, data_root, data_blocks) = if let Some(extent) = direct {
            if was_tree {
                for &lba in old_tree_blocks {
                    tx.retire(&mut self.dev, lba)?;
                }
            }
            (
                0,
                extent.map_or(0, |item| item.physical_start),
                allocated_blocks,
            )
        } else if was_tree && old_extents == new_extents {
            (OBJECT_FLAG_EXTENT_TREE, record.data_root, allocated_blocks)
        } else if was_tree {
            let old_encoded = old_extents
                .iter()
                .copied()
                .map(extent_map::encode_extent)
                .collect::<Result<Vec<_>, _>>()?;
            let new_encoded = new_extents
                .iter()
                .copied()
                .map(extent_map::encode_extent)
                .collect::<Result<Vec<_>, _>>()?;
            let mut operations = Vec::with_capacity(old_encoded.len() + new_encoded.len());
            for (key, _) in &old_encoded {
                operations.push(TreeOperation::Delete { key });
            }
            for (key, value) in &new_encoded {
                operations.push(TreeOperation::Upsert { key, value });
            }
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                tx,
                record.data_root,
                extent_map::spec(record.object_id, self.checkpoint.generation),
                generation,
                &operations,
            )?;
            metadata_writes = mutation.writes;
            (OBJECT_FLAG_EXTENT_TREE, mutation.root_lba, allocated_blocks)
        } else {
            let node_count = extent_map::bulk_node_count(block_size, new_extents.len())?;
            let mut lbas = Vec::with_capacity(node_count);
            for _ in 0..node_count {
                lbas.push(tx.allocate(&mut self.dev)?);
            }
            let built = extent_map::bulk_build(record.object_id, block_size, new_extents, &lbas)?;
            for (lba, node) in built.nodes {
                metadata_writes.push((lba, node.encode(block_size, generation)?));
            }
            (OBJECT_FLAG_EXTENT_TREE, built.root_lba, allocated_blocks)
        };

        let new_record_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, record_lba)?;
        let new_record = ObjectRecord {
            // Layout staging owns the layout flag; the persistent data-update
            // policy (ADR-065) travels with the record across every rewrite.
            flags: flags
                | (record.flags
                    & (OBJECT_FLAG_DATA_IN_PLACE
                        | OBJECT_FLAG_SECURITY_REF
                        | OBJECT_FLAG_COMMENT
                        | afsplus_format::object::OBJECT_FLAG_ATTRIBUTES)),
            size_bytes: new_size,
            allocated_bytes: allocated_blocks
                .checked_mul(block_size as u64)
                .ok_or(CoreError::PrototypeLimit("allocated byte count overflow"))?,
            modified: if content_changed {
                modified
            } else {
                record.modified
            },
            changed,
            content_generation: if content_changed {
                generation
            } else {
                record.content_generation
            },
            data_root,
            data_blocks,
            ..record
        };
        metadata_writes.push((new_record_lba, new_record.encode(block_size, generation)?));
        Ok(StagedFileLayout {
            record_lba: new_record_lba,
            metadata_writes,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_extent_delta(
        &mut self,
        tx: &mut TxAllocator,
        record: ObjectRecord,
        record_lba: u64,
        old: &[Extent],
        new: &[Extent],
        new_size: u64,
        content_changed: bool,
        now: Timespec,
        generation: u64,
    ) -> Result<StagedFileLayout, CoreError> {
        let old_encoded = old
            .iter()
            .copied()
            .map(extent_map::encode_extent)
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let new_encoded = new
            .iter()
            .copied()
            .map(extent_map::encode_extent)
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let mut operations = Vec::new();
        for key in old_encoded.keys() {
            if !new_encoded.contains_key(key) {
                operations.push(TreeOperation::Delete { key });
            }
        }
        for (key, value) in &new_encoded {
            if old_encoded.get(key) != Some(value) {
                operations.push(TreeOperation::Upsert { key, value });
            }
        }
        let sum = |items: &[Extent]| {
            items.iter().try_fold(0u64, |total, e| {
                total
                    .checked_add(e.block_count)
                    .ok_or_else(|| CoreError::Corrupt("extent allocation sum overflow".into()))
            })
        };
        let blocks = record
            .data_blocks
            .checked_sub(sum(old)?)
            .ok_or_else(|| {
                CoreError::Corrupt("local extent count exceeds object allocation".into())
            })?
            .checked_add(sum(new)?)
            .ok_or(CoreError::PrototypeLimit("allocated blocks overflow"))?;
        let mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            tx,
            record.data_root,
            extent_map::spec(record.object_id, self.checkpoint.generation),
            generation,
            &operations,
        )?;
        let mut writes = mutation.writes;
        let lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, record_lba)?;
        let updated = ObjectRecord {
            data_root: mutation.root_lba,
            data_blocks: blocks,
            allocated_bytes: blocks
                .checked_mul(self.dev.block_size() as u64)
                .ok_or(CoreError::PrototypeLimit("allocated bytes overflow"))?,
            size_bytes: new_size,
            modified: if content_changed {
                now
            } else {
                record.modified
            },
            content_generation: if content_changed {
                generation
            } else {
                record.content_generation
            },
            changed: now,
            ..record
        };
        writes.push((lba, updated.encode(self.dev.block_size(), generation)?));
        Ok(StagedFileLayout {
            record_lba: lba,
            metadata_writes: writes,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_file_layout(
        &mut self,
        record: ObjectRecord,
        record_lba: u64,
        old_extents: Vec<Extent>,
        old_tree_blocks: Vec<u64>,
        new_extents: Vec<Extent>,
        removed_extents: Vec<Extent>,
        new_size: u64,
        content_changed: bool,
        now: Timespec,
        generation: u64,
        mut tx: TxAllocator,
        data_writes: Vec<(u64, Vec<u8>)>,
    ) -> Result<(), CoreError> {
        let staged = self.stage_file_layout(
            &mut tx,
            record,
            record_lba,
            &old_extents,
            &old_tree_blocks,
            &new_extents,
            new_size,
            content_changed,
            now,
            now,
            generation,
        )?;
        self.commit_staged_file_layout(record, staged, removed_extents, generation, tx, data_writes)
    }

    fn commit_staged_file_layout(
        &mut self,
        record: ObjectRecord,
        staged: StagedFileLayout,
        removed_extents: Vec<Extent>,
        generation: u64,
        mut tx: TxAllocator,
        data_writes: Vec<(u64, Vec<u8>)>,
    ) -> Result<(), CoreError> {
        let new_record_lba = staged.record_lba;
        let mut metadata_writes = staged.metadata_writes;
        for extent in removed_extents {
            self.release_data_run(&mut tx, generation, &extent)?;
        }
        let object_key = object_map::key(record.object_id);
        let object_value = object_map::value(new_record_lba)?;
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &object_key,
                value: &object_value,
            }],
        )?;
        metadata_writes.extend(object_map_mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            data_writes,
            metadata_writes,
            object_map_mutation.root_lba,
        )
    }

    /// Executes several namespace operations as ONE transaction with ONE
    /// checkpoint publication: the group-commit / bounded atomic-batch
    /// primitive (ADR-026). Either every operation commits or none does.
    ///
    /// Prototype scope: file operations only (create with content, delete,
    /// rename with optional atomic replace). Operations see the effects of
    /// earlier operations in the same batch. Returns one entry per
    /// operation: the created object ID for `CreateFile`, `None` otherwise.
    pub fn run_batch(
        &mut self,
        ops: &[BatchOp<'_>],
        now: Timespec,
    ) -> Result<Vec<Option<u64>>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::RunBatch, |volume| {
            volume.run_batch_untraced(ops, now)
        })
    }

    fn run_batch_untraced(
        &mut self,
        ops: &[BatchOp<'_>],
        now: Timespec,
    ) -> Result<Vec<Option<u64>>, CoreError> {
        let _allocation_scope =
            crate::allocation_trace::enter(crate::allocation_trace::Domain::Batch);
        metadata::validate_time(now)?;
        self.run_batch_internal(ops, now, false)
    }

    fn run_batch_internal(
        &mut self,
        ops: &[BatchOp<'_>],
        now: Timespec,
        orphan_final_unlinks: bool,
    ) -> Result<Vec<Option<u64>>, CoreError> {
        const MAX_BATCH_OPS: usize = 1024;
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        if ops.len() > MAX_BATCH_OPS {
            return Err(CoreError::PrototypeLimit(
                "batch exceeds bounded operation count",
            ));
        }
        self.ensure_window_closed()?;
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        if ops.iter().any(|op| {
            matches!(
                op,
                BatchOp::CreateFile { .. } | BatchOp::CreateDirectory { .. }
            )
        }) {
            self.protect_emergency_headroom(&mut tx);
        }
        let mut pending = PendingBatch {
            dir_changes: BTreeMap::new(),
            dir_timestamps: BTreeMap::new(),
            records: BTreeMap::new(),
            committed_record_lbas: BTreeMap::new(),
            created_data: BTreeMap::new(),
            created_directories: BTreeSet::new(),
            file_layouts: BTreeMap::new(),
            window_allocations: Vec::new(),
            prewritten_data_blocks: 0,
            write_through: false,
            logged_created: BTreeSet::new(),
            sacrificed: Vec::new(),
            uses_emergency_headroom: false,
            next_object_id: self.checkpoint.next_object_id,
        };
        let mut results = Vec::with_capacity(ops.len());
        for op in ops {
            results.push(self.apply_batch_op(
                &mut tx,
                &mut pending,
                op,
                now,
                generation,
                orphan_final_unlinks,
            )?);
        }
        // Resolve surviving creates only after every operation has validated.
        // IDs distinguish cancelled/replaced creates even when their data LBAs
        // are reused by a later create in this same batch.
        let payloads = ops
            .iter()
            .zip(&results)
            .filter_map(|(op, result)| match (op, result) {
                (BatchOp::CreateFile { content, .. }, Some(id)) => pending
                    .created_data
                    .get(id)
                    .filter(|(_, blocks)| *blocks != 0)
                    .map(|(start, _)| (*start, *content)),
                _ => None,
            })
            .collect();
        self.materialize_batch(
            tx,
            pending,
            now,
            generation,
            false,
            CommitData::Borrowed(payloads),
        )?;
        Ok(results)
    }

    /// Atomic-replace rename for files: the target, if present, is replaced
    /// in the same transaction (its storage is quarantined or its link count
    /// decremented). One-operation batch.
    pub fn rename_replace(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::RenameReplace, |volume| {
            volume.rename_replace_untraced(
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                now,
            )
        })
    }

    fn rename_replace_untraced(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_public_object_id(source_parent_id)?;
        self.ensure_public_object_id(target_parent_id)?;
        self.run_batch(
            &[BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace: true,
            }],
            now,
        )?;
        Ok(())
    }

    /// Atomic replacement variant used when the VFS proves the target's
    /// final link still has a live handle. The replaced target enters object
    /// 2 in the same checkpoint that installs the source at its name.
    pub fn rename_replace_orphan_target(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(
            crate::flight::ApiMethod::RenameReplaceOrphanTarget,
            |volume| {
                volume.rename_replace_orphan_target_untraced(
                    source_parent_id,
                    source_name,
                    target_parent_id,
                    target_name,
                    now,
                )
            },
        )
    }

    fn rename_replace_orphan_target_untraced(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        if !self.orphan_directory_enabled() {
            return Err(CoreError::FeatureDisabled(
                "orphan-directory feature is not enabled on this volume",
            ));
        }
        self.ensure_public_object_id(source_parent_id)?;
        self.ensure_public_object_id(target_parent_id)?;
        self.ensure_orphan_directory(now)?;
        self.run_batch_internal(
            &[BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace: true,
            }],
            now,
            true,
        )?;
        Ok(())
    }

    /// Directory-entry view through the batch overlay, then committed state.
    fn batch_lookup(
        &mut self,
        pending: &PendingBatch,
        directory_id: u64,
        key: &[u8],
    ) -> Result<Option<DirEntry>, CoreError> {
        if let Some(changes) = pending.dir_changes.get(&directory_id) {
            if let Some(change) = changes.get(key) {
                return Ok(change.clone());
            }
        }
        let directory = self
            .batch_record(pending, directory_id)?
            .ok_or(CoreError::NotFound)?;
        if directory.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        if pending.created_directories.contains(&directory_id) {
            return Ok(None);
        }
        // Entries are overlaid separately, so the committed tree root is the
        // right base even when the directory record has pending changes.
        let committed = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            committed.data_root,
            directory_id,
            self.checkpoint.generation,
            &self.ident,
            key,
        )
    }

    /// Object-record view through the batch overlay, then committed state.
    fn batch_record(
        &mut self,
        pending: &PendingBatch,
        object_id: u64,
    ) -> Result<Option<ObjectRecord>, CoreError> {
        if let Some(record) = pending.records.get(&object_id) {
            return Ok(*record);
        }
        self.read_object(object_id)
    }

    /// Ensures object 2 exists in this batch without publishing a preparatory
    /// checkpoint. Intent replay needs this form: a checkpoint in between
    /// scanning the durable log and applying it would make those records
    /// stale, so a crash could lose an fsynced delete or replacement.
    fn ensure_pending_orphan_directory(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        now: Timespec,
        generation: u64,
    ) -> Result<(), CoreError> {
        if !self.orphan_directory_enabled() {
            return Err(CoreError::FeatureDisabled(
                "orphan-directory feature is not enabled on this volume",
            ));
        }
        if let Some(record) = self.batch_record(pending, OBJECT_ORPHAN_DIRECTORY)? {
            return self.validate_orphan_directory(record);
        }

        let floor = tx.free_block_floor();
        tx.set_free_block_floor(0);
        let root_result = tx.allocate(&mut self.dev);
        tx.set_free_block_floor(floor);
        let root_lba = root_result?;
        pending.records.insert(
            OBJECT_ORPHAN_DIRECTORY,
            Some(Self::new_directory_record(
                OBJECT_ORPHAN_DIRECTORY,
                root_lba,
                now,
                generation,
            )),
        );
        pending.created_directories.insert(OBJECT_ORPHAN_DIRECTORY);
        Ok(())
    }

    /// Writes or truncates a file the open window created: its content is
    /// read, changed and written to a new run, and the old run released, or
    /// quarantined when a durable log record still names it. Only a file up
    /// to [`WINDOW_CREATED_REWRITE_MAX`] qualifies; a larger one is a
    /// `PrototypeLimit` the caller answers by committing the window. Returns
    /// whether anything was changed, for [`Self::finish_created_rewrite`].
    fn rewrite_created_file(
        &mut self,
        window: &mut OpenWindow,
        object_id: u64,
        now: Timespec,
        change: impl FnOnce(&mut Vec<u8>) -> Result<(), CoreError>,
    ) -> Result<bool, (bool, CoreError)> {
        let block_size = self.dev.block_size() as u64;
        let (old_start, old_blocks) = window.pending.created_data[&object_id];
        let record = match window.pending.records.get(&object_id) {
            Some(Some(record)) => *record,
            _ => {
                return Err((
                    false,
                    CoreError::Corrupt(format!("created object {object_id} has no record")),
                ))
            }
        };
        let mut bytes = vec![0u8; record.size_bytes as usize];
        let mut block = vec![0u8; block_size as usize];
        for index in 0..old_blocks {
            let at = (index * block_size) as usize;
            if at >= bytes.len() {
                break;
            }
            self.dev
                .read_block(old_start + index, &mut block)
                .map_err(|error| (false, error.into()))?;
            let end = bytes.len().min(at + block_size as usize);
            bytes[at..end].copy_from_slice(&block[..end - at]);
        }
        change(&mut bytes).map_err(|error| (false, error))?;
        let new_size = bytes.len() as u64;
        if new_size > WINDOW_CREATED_REWRITE_MAX {
            return Err((
                false,
                CoreError::PrototypeLimit("created file past the window rewrite bound"),
            ));
        }
        let new_blocks = new_size.div_ceil(block_size);
        let new_start = if new_blocks > 0 {
            window
                .tx
                .allocate_run(&mut self.dev, new_blocks)
                .map_err(|error| (false, error))?
        } else {
            0
        };
        // From here the window has changed.
        for index in 0..new_blocks {
            let at = (index * block_size) as usize;
            let end = bytes.len().min(at + block_size as usize);
            block.fill(0);
            block[..end - at].copy_from_slice(&bytes[at..end]);
            self.dev
                .write_block(new_start + index, &block)
                .map_err(|error| (true, error.into()))?;
        }
        window.pending.prewritten_data_blocks = window
            .pending
            .prewritten_data_blocks
            .checked_add(new_blocks)
            .ok_or((
                true,
                CoreError::PrototypeLimit("window data accounting overflow"),
            ))?;
        if window.pending.logged_created.remove(&object_id) {
            if old_blocks > 0 {
                window.pending.sacrificed.push((old_start, old_blocks));
            }
        } else {
            for lba in old_start..old_start + old_blocks {
                window
                    .tx
                    .release_uncommitted(&mut self.dev, lba)
                    .map_err(|error| (true, error))?;
            }
        }
        window
            .pending
            .created_data
            .insert(object_id, (new_start, new_blocks));
        window.pending.records.insert(
            object_id,
            Some(ObjectRecord {
                size_bytes: new_size,
                allocated_bytes: new_blocks * block_size,
                data_root: new_start,
                data_blocks: new_blocks,
                modified: now,
                changed: now,
                content_generation: window.generation,
                ..record
            }),
        );
        window.unloggable = true;
        Ok(true)
    }

    /// Puts the window back after [`Self::rewrite_created_file`]: a refusal
    /// before any change keeps it, a failure after one poisons it as every
    /// other window mutation does.
    fn finish_created_rewrite(
        &mut self,
        window: OpenWindow,
        result: Result<bool, (bool, CoreError)>,
    ) -> Result<(), CoreError> {
        let generation = window.generation;
        match result {
            Ok(_) => {
                self.window = Some(window);
                Ok(())
            }
            Err((false, error)) => {
                self.window = Some(window);
                Err(error)
            }
            Err((true, error)) => {
                self.window_poisoned = true;
                self.flight_window_event(generation, crate::flight::EventKind::WindowFailed, None);
                self.flight_window_event(generation, crate::flight::EventKind::WindowClosed, None);
                Err(error)
            }
        }
    }

    /// Whether the open window changes the entries of `directory_id`, so that
    /// a directory listing must commit it first.
    pub fn window_changes_directory(&self, directory_id: u64) -> bool {
        self.window.as_ref().is_some_and(|window| {
            window.pending.dir_changes.contains_key(&directory_id)
                || window.pending.created_directories.contains(&directory_id)
        })
    }

    /// The object record every caller sees: the open window's when it holds
    /// the object, the committed one otherwise. A mutation commits the
    /// window before it reads, so this matters only to readers.
    pub(crate) fn visible_record(
        &mut self,
        object_id: u64,
    ) -> Result<Option<ObjectRecord>, CoreError> {
        if let Some(window) = self.window.as_ref() {
            if let Some(record) = window.pending.records.get(&object_id) {
                return Ok(*record);
            }
        }
        self.read_object(object_id)
    }

    /// Whether a window is open.
    pub fn window_open(&self) -> bool {
        self.window.is_some()
    }

    fn ensure_pending_file_layout(
        &mut self,
        pending: &mut PendingBatch,
        object_id: u64,
    ) -> Result<(), CoreError> {
        if pending.file_layouts.contains_key(&object_id) {
            return Ok(());
        }
        if pending.created_data.contains_key(&object_id) {
            return Err(CoreError::PrototypeLimit(
                "created-file writes stay represented by the create record",
            ));
        }
        let record = self
            .batch_record(pending, object_id)?
            .ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let (old_extents, old_tree_blocks) = self.load_file_layout(&record)?;
        self.note_committed_record(pending, object_id)?;
        let record_lba = pending.committed_record_lbas[&object_id];
        pending.file_layouts.insert(
            object_id,
            PendingFileLayout {
                record_lba,
                old_extents: old_extents.clone(),
                old_tree_blocks,
                extents: old_extents,
                size_bytes: record.size_bytes,
            },
        );
        Ok(())
    }

    /// Removes one mapped extent from a pending file. Portions allocated by
    /// earlier operations in this same window are quarantined (their bytes
    /// may still be named by an earlier durable record); committed portions
    /// follow the normal private/shared release rules.
    fn release_window_extent(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        generation: u64,
        extent: Extent,
    ) -> Result<(), CoreError> {
        let start = extent.physical_start;
        let end = extent.physical_end()?;
        let mut protected = Vec::new();
        let mut remaining_allocations = Vec::new();
        for allocation in std::mem::take(&mut pending.window_allocations) {
            let allocation_end = allocation
                .start
                .checked_add(allocation.blocks)
                .ok_or_else(|| CoreError::Corrupt("window allocation overflows".into()))?;
            let overlap_start = start.max(allocation.start);
            let overlap_end = end.min(allocation_end);
            if overlap_start >= overlap_end {
                remaining_allocations.push(allocation);
                continue;
            }
            protected.push((overlap_start, overlap_end));
            if allocation.start < overlap_start {
                remaining_allocations.push(WindowAllocation {
                    start: allocation.start,
                    blocks: overlap_start - allocation.start,
                });
            }
            if overlap_end < allocation_end {
                remaining_allocations.push(WindowAllocation {
                    start: overlap_end,
                    blocks: allocation_end - overlap_end,
                });
            }
        }
        pending.window_allocations = remaining_allocations;
        protected.sort_unstable();

        let mut cursor = start;
        for (protected_start, protected_end) in protected {
            if cursor < protected_start {
                self.release_data_run(
                    tx,
                    generation,
                    &Extent {
                        logical_start: extent.logical_start + cursor - start,
                        physical_start: cursor,
                        block_count: protected_start - cursor,
                        flags: extent.flags,
                    },
                )?;
            }
            tx.abandon_uncommitted_run(
                &mut self.dev,
                protected_start,
                protected_end - protected_start,
            )?;
            cursor = protected_end;
        }
        if cursor < end {
            self.release_data_run(
                tx,
                generation,
                &Extent {
                    logical_start: extent.logical_start + cursor - start,
                    physical_start: cursor,
                    block_count: end - cursor,
                    flags: extent.flags,
                },
            )?;
        }
        Ok(())
    }

    fn logged_data_layout(
        logical_start: u64,
        extents: &[(u64, u32)],
    ) -> Result<Vec<Extent>, CoreError> {
        let mut logical = logical_start;
        let mut layout = Vec::with_capacity(extents.len());
        for (physical_start, blocks) in extents {
            let block_count = u64::from(*blocks);
            if block_count == 0 {
                return Err(CoreError::Corrupt("logged data extent is empty".into()));
            }
            layout.push(Extent {
                logical_start: logical,
                physical_start: *physical_start,
                block_count,
                flags: 0,
            });
            logical = logical
                .checked_add(block_count)
                .ok_or_else(|| CoreError::Corrupt("logged logical range overflows".into()))?;
        }
        Ok(layout)
    }

    #[allow(clippy::too_many_arguments)]
    fn install_logged_file_range(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        object_id: u64,
        logical_start: u64,
        expected_size: u64,
        new_size: u64,
        extents: &[(u64, u32)],
        now: Timespec,
        generation: u64,
    ) -> Result<(), CoreError> {
        self.ensure_pending_file_layout(pending, object_id)?;
        let replacement = Self::logged_data_layout(logical_start, extents)?;
        let block_count = replacement.iter().try_fold(0u64, |total, extent| {
            total
                .checked_add(extent.block_count)
                .ok_or_else(|| CoreError::Corrupt("logged block count overflows".into()))
        })?;
        if block_count == 0 {
            return Err(CoreError::Corrupt("logged write has no data".into()));
        }
        let current_size = pending.file_layouts[&object_id].size_bytes;
        if current_size != expected_size {
            return Err(CoreError::Corrupt(format!(
                "logged write expected size {expected_size}, found {current_size}"
            )));
        }
        let end = logical_start
            .checked_add(block_count)
            .ok_or_else(|| CoreError::Corrupt("logged write range overflows".into()))?;
        let current_extents = pending.file_layouts[&object_id].extents.clone();
        let (mut updated, removed) =
            replace_logical_range(&current_extents, logical_start, end, None)?;
        for extent in removed {
            self.release_window_extent(tx, pending, generation, extent)?;
        }
        updated.extend(replacement.iter().copied());
        let updated = coalesce_extents(updated)?;
        pending
            .window_allocations
            .extend(replacement.iter().map(|extent| WindowAllocation {
                start: extent.physical_start,
                blocks: extent.block_count,
            }));
        let layout = pending
            .file_layouts
            .get_mut(&object_id)
            .expect("layout prepared above");
        layout.extents = updated;
        layout.size_bytes = new_size;
        let record = self
            .batch_record(pending, object_id)?
            .ok_or(CoreError::NotFound)?;
        pending.records.insert(
            object_id,
            Some(ObjectRecord {
                size_bytes: new_size,
                modified: now,
                changed: now,
                content_generation: generation,
                ..record
            }),
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn install_logged_truncate(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        object_id: u64,
        logical_start: u64,
        expected_size: u64,
        new_size: u64,
        extents: &[(u64, u32)],
        now: Timespec,
        generation: u64,
    ) -> Result<(), CoreError> {
        self.ensure_pending_file_layout(pending, object_id)?;
        let current_size = pending.file_layouts[&object_id].size_bytes;
        if current_size != expected_size || current_size == new_size {
            return Err(CoreError::Corrupt(format!(
                "logged truncate expected size {expected_size}, found {current_size}, new {new_size}"
            )));
        }
        let block_size = self.dev.block_size() as u64;
        let current_extents = pending.file_layouts[&object_id].extents.clone();
        let mut updated = current_extents.clone();
        let mut removed = Vec::new();
        let mut tail_rewrite_required = false;
        let retained_blocks = new_size.div_ceil(block_size);
        if new_size < current_size {
            (updated, removed) =
                replace_logical_range(&current_extents, retained_blocks, u64::MAX, None)?;
            if !new_size.is_multiple_of(block_size) {
                let tail = retained_blocks - 1;
                tail_rewrite_required = extent_at(&updated, tail)
                    .is_some_and(|extent| extent.flags & EXTENT_UNWRITTEN == 0);
            }
        }
        for extent in removed {
            self.release_window_extent(tx, pending, generation, extent)?;
        }

        let replacement = Self::logged_data_layout(logical_start, extents)?;
        match replacement.as_slice() {
            [] if tail_rewrite_required => {
                return Err(CoreError::Corrupt(
                    "logged truncate is missing its partial tail block".into(),
                ));
            }
            [] => {
                if logical_start != 0 {
                    return Err(CoreError::Corrupt(
                        "data-free logged truncate has a logical block".into(),
                    ));
                }
            }
            [tail] if tail_rewrite_required && tail.logical_start == retained_blocks - 1 => {
                let (rewritten, replaced) = replace_logical_range(
                    &updated,
                    tail.logical_start,
                    tail.logical_start + 1,
                    None,
                )?;
                updated = rewritten;
                for extent in replaced {
                    self.release_window_extent(tx, pending, generation, extent)?;
                }
                updated.push(*tail);
                updated = coalesce_extents(updated)?;
                pending.window_allocations.push(WindowAllocation {
                    start: tail.physical_start,
                    blocks: tail.block_count,
                });
            }
            _ => {
                return Err(CoreError::Corrupt(
                    "logged truncate carries an invalid tail replacement".into(),
                ));
            }
        }

        let layout = pending
            .file_layouts
            .get_mut(&object_id)
            .expect("layout prepared above");
        layout.extents = updated;
        layout.size_bytes = new_size;
        let record = self
            .batch_record(pending, object_id)?
            .ok_or(CoreError::NotFound)?;
        pending.records.insert(
            object_id,
            Some(ObjectRecord {
                size_bytes: new_size,
                modified: now,
                changed: now,
                content_generation: generation,
                ..record
            }),
        );
        Ok(())
    }

    /// Remembers the committed record block of an object the batch rewrites
    /// or deletes, so materialization retires exactly one old block per
    /// object.
    fn note_committed_record(
        &mut self,
        pending: &mut PendingBatch,
        object_id: u64,
    ) -> Result<(), CoreError> {
        if pending.committed_record_lbas.contains_key(&object_id)
            || pending.created_data.contains_key(&object_id)
        {
            return Ok(());
        }
        let lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("object {object_id} missing from object map"))
        })?;
        pending.committed_record_lbas.insert(object_id, lba);
        Ok(())
    }

    fn apply_batch_op(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        op: &BatchOp<'_>,
        now: Timespec,
        generation: u64,
        orphan_final_unlinks: bool,
    ) -> Result<Option<u64>, CoreError> {
        let block_size = self.dev.block_size();
        match op {
            BatchOp::CreateFile {
                parent_id,
                name,
                content,
            } => {
                validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
                let parent = self
                    .batch_record(pending, *parent_id)?
                    .ok_or(CoreError::NotFound)?;
                if parent.object_type != ObjectType::Directory {
                    return Err(CoreError::NotDirectory);
                }
                let key = self.comparison_key(name.as_bytes())?;
                if self.batch_lookup(pending, *parent_id, &key)?.is_some() {
                    return Err(CoreError::AlreadyExists);
                }
                let object_id = pending.next_object_id;
                pending.next_object_id = object_id
                    .checked_add(1)
                    .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;
                let data_block_count = (content.len() as u64).div_ceil(block_size as u64);
                let data_start = if data_block_count > 0 {
                    let start = tx.allocate_run(&mut self.dev, data_block_count)?;
                    if pending.write_through {
                        let staged = crate::flight::DataContext {
                            scope: crate::flight::DataScope::CreateWriteThrough,
                            object_id,
                            offset: 0,
                            length: content.len() as u64,
                            start,
                            blocks: u32::try_from(data_block_count).unwrap_or(u32::MAX),
                        };
                        self.flight_data_event(
                            crate::flight::EventKind::DataWriteBegin,
                            staged,
                            generation,
                        );
                        for i in 0..data_block_count as usize {
                            let mut block = vec![0u8; block_size];
                            let from = i * block_size;
                            let to = content.len().min(from + block_size);
                            block[..to - from].copy_from_slice(&content[from..to]);
                            if let Err(error) = self.dev.write_block(start + i as u64, &block) {
                                self.flight_data_event(
                                    crate::flight::EventKind::DataWriteFailed,
                                    crate::flight::DataContext {
                                        start: start + i as u64,
                                        blocks: 1,
                                        ..staged
                                    },
                                    generation,
                                );
                                return Err(error.into());
                            }
                        }
                        self.flight_data_event(
                            crate::flight::EventKind::DataWriteComplete,
                            staged,
                            generation,
                        );
                        pending.prewritten_data_blocks = pending
                            .prewritten_data_blocks
                            .checked_add(data_block_count)
                            .ok_or(CoreError::PrototypeLimit("window data accounting overflow"))?;
                    }
                    start
                } else {
                    0
                };
                pending
                    .created_data
                    .insert(object_id, (data_start, data_block_count));
                pending.records.insert(
                    object_id,
                    Some(ObjectRecord {
                        object_id,
                        object_type: ObjectType::File,
                        flags: 0,
                        link_count: 1,
                        owner_uid: 0,
                        owner_gid: 0,
                        size_bytes: content.len() as u64,
                        allocated_bytes: data_block_count * block_size as u64,
                        created: now,
                        modified: now,
                        changed: now,
                        protection: 0,
                        content_generation: generation,
                        data_root: data_start,
                        data_blocks: data_block_count,
                        security: None,
                        attributes: None,
                        comment: afsplus_format::object::Comment::EMPTY,
                    }),
                );
                pending.dir_changes.entry(*parent_id).or_default().insert(
                    key,
                    Some(DirEntry {
                        key: self.comparison_key(name.as_bytes())?,
                        name: name.as_bytes().to_vec(),
                        child_type_hint: 1,
                        child_id: object_id,
                    }),
                );
                pending.dir_timestamps.insert(*parent_id, now);
                Ok(Some(object_id))
            }
            BatchOp::CreateDirectory { parent_id, name } => {
                validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
                let parent = self
                    .batch_record(pending, *parent_id)?
                    .ok_or(CoreError::NotFound)?;
                if parent.object_type != ObjectType::Directory {
                    return Err(CoreError::NotDirectory);
                }
                let key = self.comparison_key(name.as_bytes())?;
                if self.batch_lookup(pending, *parent_id, &key)?.is_some() {
                    return Err(CoreError::AlreadyExists);
                }
                let object_id = pending.next_object_id;
                pending.next_object_id = object_id
                    .checked_add(1)
                    .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;
                let root_lba = tx.allocate(&mut self.dev)?;
                pending.records.insert(
                    object_id,
                    Some(Self::new_directory_record(
                        object_id, root_lba, now, generation,
                    )),
                );
                pending.created_directories.insert(object_id);
                // Materialization walks the directories the batch changed, and
                // it is the one that stages the empty leaf at `root_lba`. A
                // directory nothing has been put into yet changes no entry, so
                // list it explicitly or its root block is never written.
                pending.dir_changes.entry(object_id).or_default();
                pending.dir_timestamps.insert(object_id, now);
                pending.dir_changes.entry(*parent_id).or_default().insert(
                    key,
                    Some(DirEntry {
                        key: self.comparison_key(name.as_bytes())?,
                        name: name.as_bytes().to_vec(),
                        child_type_hint: 2,
                        child_id: object_id,
                    }),
                );
                pending.dir_timestamps.insert(*parent_id, now);
                Ok(Some(object_id))
            }
            BatchOp::DeleteFile { parent_id, name } => {
                validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
                let key = self.comparison_key(name.as_bytes())?;
                let entry = self
                    .batch_lookup(pending, *parent_id, &key)?
                    .ok_or(CoreError::NotFound)?;
                self.unlink_in_batch(
                    tx,
                    pending,
                    generation,
                    entry.child_id,
                    now,
                    orphan_final_unlinks,
                )?;
                pending
                    .dir_changes
                    .entry(*parent_id)
                    .or_default()
                    .insert(key, None);
                pending.dir_timestamps.insert(*parent_id, now);
                Ok(None)
            }
            BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace,
            } => {
                validate_name(source_name.as_bytes()).map_err(CoreError::InvalidName)?;
                validate_name(target_name.as_bytes()).map_err(CoreError::InvalidName)?;
                let source_key = self.comparison_key(source_name.as_bytes())?;
                let target_key = self.comparison_key(target_name.as_bytes())?;
                let same_key = source_parent_id == target_parent_id && source_key == target_key;
                let entry = self
                    .batch_lookup(pending, *source_parent_id, &source_key)?
                    .ok_or(CoreError::NotFound)?;
                if same_key && entry.name == target_name.as_bytes() {
                    return Ok(None);
                }
                let moved = self
                    .batch_record(pending, entry.child_id)?
                    .ok_or_else(|| CoreError::Corrupt("moved object missing".into()))?;
                if moved.object_type != ObjectType::File {
                    return Err(CoreError::PrototypeLimit(
                        "batched rename supports files only",
                    ));
                }
                if !same_key {
                    if let Some(existing) =
                        self.batch_lookup(pending, *target_parent_id, &target_key)?
                    {
                        if existing.child_id == entry.child_id {
                            return Ok(None);
                        }
                        if !replace {
                            return Err(CoreError::AlreadyExists);
                        }
                        let target = self
                            .batch_record(pending, existing.child_id)?
                            .ok_or_else(|| CoreError::Corrupt("replace target missing".into()))?;
                        if target.object_type != ObjectType::File {
                            return Err(CoreError::IsDirectory);
                        }
                        self.unlink_in_batch(
                            tx,
                            pending,
                            generation,
                            existing.child_id,
                            now,
                            orphan_final_unlinks,
                        )?;
                    }
                }
                let target_parent = self
                    .batch_record(pending, *target_parent_id)?
                    .ok_or(CoreError::NotFound)?;
                if target_parent.object_type != ObjectType::Directory {
                    return Err(CoreError::NotDirectory);
                }
                pending
                    .dir_changes
                    .entry(*source_parent_id)
                    .or_default()
                    .insert(source_key, None);
                pending
                    .dir_changes
                    .entry(*target_parent_id)
                    .or_default()
                    .insert(
                        target_key,
                        Some(DirEntry {
                            key: self.comparison_key(target_name.as_bytes())?,
                            name: target_name.as_bytes().to_vec(),
                            child_type_hint: 1,
                            child_id: entry.child_id,
                        }),
                    );
                pending.dir_timestamps.insert(*source_parent_id, now);
                pending.dir_timestamps.insert(*target_parent_id, now);
                if !pending.created_data.contains_key(&entry.child_id) {
                    self.note_committed_record(pending, entry.child_id)?;
                }
                pending.records.insert(
                    entry.child_id,
                    Some(ObjectRecord {
                        changed: now,
                        ..moved
                    }),
                );
                Ok(None)
            }
        }
    }

    /// Drops one link from a file inside a batch: cancels a same-batch
    /// creation entirely, decrements a multiply-linked committed file, moves
    /// a final victim into bounded orphan state when requested, or performs
    /// the legacy direct retirement path.
    fn unlink_in_batch(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        generation: u64,
        object_id: u64,
        now: Timespec,
        orphan_final: bool,
    ) -> Result<(), CoreError> {
        let victim = self
            .batch_record(pending, object_id)?
            .ok_or_else(|| CoreError::Corrupt("unlink victim missing".into()))?;
        if victim.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        if let Some((data_start, data_blocks)) = pending.created_data.remove(&object_id) {
            pending.records.remove(&object_id);
            if pending.logged_created.contains(&object_id) {
                // A durable log record's content CRC still covers these
                // blocks: never reuse them in this window; quarantine them
                // when the window materializes.
                if data_blocks > 0 {
                    pending.sacrificed.push((data_start, data_blocks));
                }
                return Ok(());
            }
            // Same-batch creation, never logged: no published state or log
            // references it. Release the reserved run; ordinary batch payloads
            // are selected by surviving object identity after validation.
            for lba in data_start..data_start + data_blocks {
                tx.release_uncommitted(&mut self.dev, lba)?;
            }
            return Ok(());
        }
        if victim.link_count > 1 {
            self.note_committed_record(pending, object_id)?;
            pending.records.insert(
                object_id,
                Some(ObjectRecord {
                    link_count: victim.link_count - 1,
                    changed: now,
                    ..victim
                }),
            );
            return Ok(());
        }
        if orphan_final && self.orphan_directory_enabled() {
            pending.uses_emergency_headroom = true;
            self.ensure_pending_orphan_directory(tx, pending, now, generation)?;
            let orphan_name = Self::orphan_name(object_id);
            let orphan_key = self.comparison_key(orphan_name.as_bytes())?;
            if self
                .batch_lookup(pending, OBJECT_ORPHAN_DIRECTORY, &orphan_key)?
                .is_some()
            {
                return Err(CoreError::Corrupt(format!(
                    "object {object_id} already has an orphan entry"
                )));
            }
            pending
                .dir_changes
                .entry(OBJECT_ORPHAN_DIRECTORY)
                .or_default()
                .insert(
                    orphan_key.clone(),
                    Some(DirEntry {
                        key: orphan_key,
                        name: orphan_name.into_bytes(),
                        child_type_hint: 1,
                        child_id: object_id,
                    }),
                );
            pending.dir_timestamps.insert(OBJECT_ORPHAN_DIRECTORY, now);
            self.note_committed_record(pending, object_id)?;
            pending.records.insert(
                object_id,
                Some(ObjectRecord {
                    changed: now,
                    ..victim
                }),
            );
            return Ok(());
        }
        if pending.file_layouts.contains_key(&object_id) {
            return Err(CoreError::PrototypeLimit(
                "direct deletion of a window-modified file requires log compaction",
            ));
        }
        self.note_committed_record(pending, object_id)?;
        let committed_lba = pending.committed_record_lbas[&object_id];
        tx.retire(&mut self.dev, committed_lba)?;
        pending.committed_record_lbas.remove(&object_id);
        self.retire_security_descriptor(tx, &victim)?;
        self.retire_attributes(tx, &victim)?;
        self.retire_file_storage(tx, generation, &victim)?;
        pending.records.insert(object_id, None);
        Ok(())
    }

    /// True when this volume may hold shared extents (ADR-061).
    fn shared_extents_enabled(&self) -> bool {
        self.ident.features.ro_compat & RO_COMPAT_SHARED_EXTENTS != 0
    }

    fn data_policy_enabled(&self) -> bool {
        self.ident.features.compat & COMPAT_DATA_POLICY != 0
    }

    fn orphan_directory_enabled(&self) -> bool {
        self.ident.features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0
    }

    fn ensure_public_object_id(&self, object_id: u64) -> Result<(), CoreError> {
        if object_id == OBJECT_ORPHAN_DIRECTORY {
            Err(CoreError::NotFound)
        } else {
            Ok(())
        }
    }

    /// The transaction-scoped reference edit. Created empty; committed
    /// records enter the overlay only through [`Self::shared_prefetch`]'s
    /// bounded range reads. Keyed by generation; `next_generation` cleared
    /// any leftover from an aborted transaction.
    fn shared_refs_edit(&mut self, generation: u64) -> Result<&mut RefEdit, CoreError> {
        let stale = self
            .shared_refs
            .as_ref()
            .is_none_or(|(opened_for, _)| *opened_for != generation);
        if stale {
            let mut edit = RefEdit::new();
            if self.checkpoint.shared_extent_root_block == 0 {
                edit.mark_all_fetched();
            }
            self.shared_refs = Some((generation, edit));
        }
        Ok(&mut self
            .shared_refs
            .as_mut()
            .expect("shared_refs initialized above")
            .1)
    }

    /// Loads into the overlay, through the bounded tree walk, exactly the
    /// committed records overlapping `[start, start+blocks)` plus the floor
    /// neighbour, so canonical merging stays local. Never materialises the
    /// volume-wide tree.
    fn shared_prefetch(
        &mut self,
        generation: u64,
        start: u64,
        blocks: u64,
    ) -> Result<(), CoreError> {
        let end = start
            .checked_add(blocks)
            .ok_or_else(|| CoreError::Corrupt("shared prefetch range overflows".into()))?;
        let root = self.checkpoint.shared_extent_root_block;
        let committed = self.checkpoint.generation;
        let geo = self.ident.geometry();
        if self.shared_refs_edit(generation)?.covers(start, end) {
            return Ok(());
        }
        let low_key = {
            // Query immediately before `start`, not at `start`: when a
            // record begins exactly on the edited boundary, lookup_floor at
            // `start` returns that record and hides its immediate left
            // neighbour.  The neighbour is needed because the edit may make
            // the two records merge on the left (for example rc=3 -> rc=2 next
            // to an existing rc=2 record).  For start zero there cannot be a
            // predecessor, and the saturating key still includes a record at
            // zero in the range walk below.
            let predecessor_key = afsplus_format::tree::key_u64(start.saturating_sub(1));
            let (floor, _) = crate::tree::lookup_floor(
                &mut self.dev,
                &geo,
                root,
                shared_extents::spec(committed),
                &predecessor_key,
            )?;
            floor
                .map(|(key, _)| key)
                .unwrap_or_else(|| afsplus_format::tree::key_u64(start).to_vec())
        };
        let mut records = Vec::new();
        crate::tree::visit_key_range(
            &mut self.dev,
            &geo,
            root,
            shared_extents::spec(committed),
            &low_key,
            &afsplus_format::tree::key_u64(end),
            |key, value| {
                records.push(shared_extents::decode_run(key, value, &geo)?);
                Ok(())
            },
        )?;
        self.shared_refs_edit(generation)?
            .note_fetched(start, end, records);
        Ok(())
    }

    /// Drops this mapping's reference to a data run and retires exactly what
    /// no live mapping still covers (the ADR-061 release table). A private
    /// extent retires whole; a flagged extent is partitioned by overlap, and
    /// a record falling from two references to one is removed WITHOUT
    /// retiring its blocks — the surviving peer still maps them.
    fn release_data_run(
        &mut self,
        tx: &mut TxAllocator,
        generation: u64,
        extent: &Extent,
    ) -> Result<(), CoreError> {
        if extent.flags & EXTENT_SHARED == 0 {
            tx.retire_run(&mut self.dev, extent.physical_start, extent.block_count)?;
            return Ok(());
        }
        // Fail closed (ADR-061): a flagged extent on a volume where the
        // feature is off, or where no reference tree exists, has no legal
        // history. Freeing it as if the gaps were private would release
        // storage through the unknown state.
        if !self.shared_extents_enabled() {
            return Err(CoreError::Corrupt(
                "extent carries EXTENT_SHARED without the shared-extents feature".into(),
            ));
        }
        if self.checkpoint.shared_extent_root_block == 0 {
            return Err(CoreError::Corrupt(
                "extent carries EXTENT_SHARED but the volume has no reference tree".into(),
            ));
        }
        self.shared_prefetch(generation, extent.physical_start, extent.block_count)?;
        let gaps = self
            .shared_refs_edit(generation)?
            .release(extent.physical_start, extent.block_count)?;
        for (start, blocks) in gaps {
            tx.retire_run(&mut self.dev, start, blocks)?;
        }
        Ok(())
    }

    /// Quarantines a committed file's data extents and extent-tree nodes,
    /// honouring shared references (ADR-061).
    fn retire_file_storage(
        &mut self,
        tx: &mut TxAllocator,
        generation: u64,
        victim: &ObjectRecord,
    ) -> Result<(), CoreError> {
        if victim.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            let map = extent_map::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                victim.data_root,
                victim.object_id,
                self.checkpoint.generation,
            )?;
            for lba in map.tree_blocks {
                tx.retire(&mut self.dev, lba)?;
            }
            for extent in map.extents {
                self.release_data_run(tx, generation, &extent)?;
            }
        } else if victim.data_blocks > 0 {
            tx.retire_run(&mut self.dev, victim.data_root, victim.data_blocks)?;
        }
        Ok(())
    }

    fn ensure_window_closed(&self) -> Result<(), CoreError> {
        if !self.mount_mode.allows_user_writes() {
            Err(CoreError::ReadOnly)
        } else if self.window_poisoned {
            Err(CoreError::WindowPoisoned)
        } else if self.window.is_some() {
            Err(CoreError::WindowOpen)
        } else {
            Ok(())
        }
    }

    /// True for errors that reject one operation without having mutated the
    /// window's staged state.
    fn is_validation_error(error: &CoreError) -> bool {
        matches!(
            error,
            CoreError::AlreadyExists
                | CoreError::NotFound
                | CoreError::InvalidName(_)
                | CoreError::NotDirectory
                | CoreError::IsDirectory
                | CoreError::NoSpace
                | CoreError::PrototypeLimit(_)
        )
    }

    /// Pending window operations not yet made durable by an fsync.
    pub fn window_unlogged_ops(&self) -> usize {
        self.window.as_ref().map_or(0, |w| w.unlogged.len())
    }

    fn take_or_open_window(&mut self) -> Result<OpenWindow, CoreError> {
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        if self.window_poisoned {
            return Err(CoreError::WindowPoisoned);
        }
        if let Some(window) = self.window.take() {
            if let Some(recorder) = &mut self.flight {
                recorder.borrow_mut().observe_window(
                    window.generation,
                    window.logged_records,
                    true,
                    self.window_poisoned,
                );
            }
            return Ok(window);
        }
        let generation = self.next_generation()?;
        // Windowed transactions never promote quarantined blocks: logged
        // data extents must be FREE in the committed bitmaps so replay can
        // claim them deterministically (ADR-037/063).
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            0,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);
        if let Some(recorder) = &mut self.flight {
            recorder
                .borrow_mut()
                .observe_window(generation, 0, false, self.window_poisoned);
        }
        Ok(OpenWindow {
            tx,
            pending: PendingBatch {
                dir_changes: BTreeMap::new(),
                dir_timestamps: BTreeMap::new(),
                records: BTreeMap::new(),
                committed_record_lbas: BTreeMap::new(),
                created_data: BTreeMap::new(),
                created_directories: BTreeSet::new(),
                file_layouts: BTreeMap::new(),
                window_allocations: Vec::new(),
                prewritten_data_blocks: 0,
                write_through: true,
                logged_created: BTreeSet::new(),
                sacrificed: Vec::new(),
                uses_emergency_headroom: false,
                next_object_id: self.checkpoint.next_object_id,
            },
            generation,
            unlogged: Vec::new(),
            logged_records: 0,
            unloggable: false,
        })
    }

    /// Applies one operation to the open window (opening it if needed). The
    /// operation is visible to later window operations but not durable until
    /// [`Volume::window_fsync`] and not checkpointed until
    /// [`Volume::window_commit`].
    pub fn window_op(&mut self, op: &BatchOp<'_>, now: Timespec) -> Result<Option<u64>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowOp, |volume| {
            volume.window_op_untraced(op, now)
        })
    }

    fn window_op_untraced(
        &mut self,
        op: &BatchOp<'_>,
        now: Timespec,
    ) -> Result<Option<u64>, CoreError> {
        metadata::validate_time(now)?;
        let mut window = self.take_or_open_window()?;
        let generation = window.generation;
        // A delete (or replacing rename) whose victim is a window create not
        // yet covered by a log record cancels to nothing: the create is
        // scrubbed from the unlogged group so the record never mentions it.
        let cancels_unlogged = match self.window_cancel_target(&window.pending, op) {
            Ok(cancelled) => cancelled,
            Err(error) => {
                // This preflight only reads the overlay/committed namespace.
                // Preserve every staged operation, including fsynced groups,
                // when lookup or name validation refuses the new request.
                self.window = Some(window);
                return Err(error);
            }
        };
        let result = self.apply_batch_op(
            &mut window.tx,
            &mut window.pending,
            op,
            now,
            generation,
            true,
        );
        match result {
            Ok(created) => {
                // Build the log record before editing the unlogged sequence.
                // An internal bookkeeping error follows a successful mutation,
                // so retaining a writable window would promise unsafe retry.
                let logged =
                    if cancels_unlogged.is_some() && matches!(op, BatchOp::DeleteFile { .. }) {
                        None
                    } else {
                        match Self::log_op_for(op, created, &window.pending, now) {
                            Ok(logged) => {
                                if logged.is_none() {
                                    window.unloggable = true;
                                }
                                logged
                            }
                            Err(error) => {
                                self.window_poisoned = true;
                                self.flight_window_event(
                                    generation,
                                    crate::flight::EventKind::WindowFailed,
                                    None,
                                );
                                self.flight_window_event(
                                    generation,
                                    crate::flight::EventKind::WindowClosed,
                                    None,
                                );
                                return Err(error);
                            }
                        }
                    };
                if let Some(cancelled) = cancels_unlogged {
                    window.unlogged.retain(|logged| {
                        !matches!(
                            logged,
                            LogOp::Create { expected_object_id, .. }
                                if *expected_object_id == cancelled
                        )
                    });
                }
                if let Some(logged) = logged {
                    window.unlogged.push(logged);
                }
                self.window = Some(window);
                Ok(created)
            }
            Err(error) if Self::is_validation_error(&error) => {
                self.window = Some(window);
                Err(error)
            }
            Err(error) => {
                self.window_poisoned = true;
                self.flight_window_event(
                    window.generation,
                    crate::flight::EventKind::WindowFailed,
                    None,
                );
                self.flight_window_event(
                    window.generation,
                    crate::flight::EventKind::WindowClosed,
                    None,
                );
                Err(error)
            }
        }
    }

    /// Stages an existing-file write in the open intent-log window. The
    /// replacement blocks are always newly allocated COW data, independent
    /// of the file's ordinary ADR-062 policy.
    pub fn window_write_file_at(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowWriteFileAt, |volume| {
            volume.window_write_file_at_untraced(object_id, offset, content, now)
        })
    }

    fn window_write_file_at_untraced(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        if content.is_empty() {
            return Ok(());
        }
        if self.ident.features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES == 0 {
            return Err(CoreError::FeatureDisabled(
                "intent-log existing-file data updates",
            ));
        }
        let content_len = u64::try_from(content.len())
            .map_err(|_| CoreError::PrototypeLimit("write buffer is too large"))?;
        let end_offset = offset
            .checked_add(content_len)
            .ok_or(CoreError::PrototypeLimit("file size limit reached"))?;
        let mut window = self.take_or_open_window()?;
        if window.pending.created_data.contains_key(&object_id) {
            let result = self.rewrite_created_file(&mut window, object_id, now, |bytes| {
                let start = usize::try_from(offset)
                    .map_err(|_| CoreError::PrototypeLimit("write offset is too large"))?;
                let end = start + content.len();
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[start..end].copy_from_slice(content);
                Ok(())
            });
            return self.finish_created_rewrite(window, result);
        }
        if let Err(error) = self.ensure_pending_file_layout(&mut window.pending, object_id) {
            self.window = Some(window);
            return Err(error);
        }
        let expected_size = window.pending.file_layouts[&object_id].size_bytes;
        let current_extents = window.pending.file_layouts[&object_id].extents.clone();
        let block_size = self.dev.block_size() as u64;
        let first_block = offset / block_size;
        let end_block = end_offset.div_ceil(block_size);
        let block_count = end_block - first_block;

        // Set before the first replacement-block write: until then the call
        // has changed no staged operation and issued no write, so a refusal
        // is a preflight refusal that keeps the whole window (ADR-037 and
        // testing/intent-log-write-truncate-qualification.md).
        let mut mutated = false;
        let result = (|| {
            let mut blocks = Vec::with_capacity(block_count as usize);
            for logical_block in first_block..end_block {
                let mut block = vec![0u8; block_size as usize];
                self.read_layout_block(&current_extents, logical_block, &mut block)?;
                let logical_byte = logical_block.saturating_mul(block_size);
                let copy_start = offset.max(logical_byte);
                let copy_end = end_offset.min(logical_byte.saturating_add(block_size));
                let source_start = usize::try_from(copy_start - offset)
                    .map_err(|_| CoreError::PrototypeLimit("write offset is too large"))?;
                let source_end = usize::try_from(copy_end - offset)
                    .map_err(|_| CoreError::PrototypeLimit("write offset is too large"))?;
                let target_start = usize::try_from(copy_start - logical_byte)
                    .map_err(|_| CoreError::PrototypeLimit("block offset is too large"))?;
                block[target_start..target_start + source_end - source_start]
                    .copy_from_slice(&content[source_start..source_end]);
                blocks.push(block);
            }

            let additions = allocate_extent_runs(
                &mut window.tx,
                &mut self.dev,
                &self.ident.geometry(),
                first_block,
                block_count,
                0,
            )?;
            let staged = crate::flight::DataContext {
                scope: crate::flight::DataScope::ExistingFileWrite,
                object_id,
                offset,
                length: content_len,
                start: additions.first().map_or(0, |extent| extent.physical_start),
                blocks: u32::try_from(block_count).unwrap_or(u32::MAX),
            };
            self.flight_data_event(
                crate::flight::EventKind::DataWriteBegin,
                staged,
                window.generation,
            );
            let mut block_iter = blocks.iter();
            let mut logical_block = first_block;
            for extent in &additions {
                for physical_offset in 0..extent.block_count {
                    let block = block_iter.next().ok_or_else(|| {
                        CoreError::Corrupt("logged write block count mismatch".into())
                    })?;
                    mutated = true;
                    if let Err(error) = self
                        .dev
                        .write_block(extent.physical_start + physical_offset, block)
                    {
                        self.flight_data_event(
                            crate::flight::EventKind::DataWriteFailed,
                            crate::flight::DataContext {
                                offset: (logical_block + physical_offset) * block_size,
                                length: block_size,
                                start: extent.physical_start + physical_offset,
                                blocks: 1,
                                ..staged
                            },
                            window.generation,
                        );
                        return Err(error.into());
                    }
                }
                self.flight_data_event(
                    crate::flight::EventKind::DataWriteComplete,
                    crate::flight::DataContext {
                        offset: logical_block * block_size,
                        length: extent.block_count * block_size,
                        start: extent.physical_start,
                        blocks: u32::try_from(extent.block_count).unwrap_or(u32::MAX),
                        ..staged
                    },
                    window.generation,
                );
                logical_block += extent.block_count;
            }
            if block_iter.next().is_some() {
                return Err(CoreError::Corrupt(
                    "logged write block count mismatch".into(),
                ));
            }
            window.pending.prewritten_data_blocks = window
                .pending
                .prewritten_data_blocks
                .checked_add(block_count)
                .ok_or(CoreError::PrototypeLimit("window data accounting overflow"))?;
            let mut hasher = Hasher::new();
            for block in &blocks {
                hasher.update(block);
            }
            let extents = additions
                .iter()
                .map(|extent| {
                    let blocks = u32::try_from(extent.block_count).map_err(|_| {
                        CoreError::PrototypeLimit("logged extent exceeds u32 blocks")
                    })?;
                    Ok((extent.physical_start, blocks))
                })
                .collect::<Result<Vec<_>, CoreError>>()?;
            let new_size = expected_size.max(end_offset);
            self.install_logged_file_range(
                &mut window.tx,
                &mut window.pending,
                object_id,
                first_block,
                expected_size,
                new_size,
                &extents,
                now,
                window.generation,
            )?;
            window.unlogged.push(LogOp::Write {
                object_id,
                logical_start: first_block,
                expected_size_bytes: expected_size,
                new_size_bytes: new_size,
                content_crc: hasher.finalize(),
                timestamp: now,
                extents,
            });
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.window = Some(window);
                Ok(())
            }
            Err(error) if !mutated => {
                self.window = Some(window);
                Err(error)
            }
            Err(error) => {
                self.window_poisoned = true;
                self.flight_window_event(
                    window.generation,
                    crate::flight::EventKind::WindowFailed,
                    None,
                );
                self.flight_window_event(
                    window.generation,
                    crate::flight::EventKind::WindowClosed,
                    None,
                );
                Err(error)
            }
        }
    }

    /// Stages an existing-file truncate in the intent-log window. Shrinking
    /// a materialized partial tail uses one fresh, zero-tailed COW block.
    pub fn window_truncate_file(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowTruncateFile, |volume| {
            volume.window_truncate_file_untraced(object_id, new_size, now)
        })
    }

    fn window_truncate_file_untraced(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        if self.ident.features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES == 0 {
            return Err(CoreError::FeatureDisabled(
                "intent-log existing-file data updates",
            ));
        }
        if self
            .window
            .as_ref()
            .is_some_and(|window| window.pending.created_data.contains_key(&object_id))
        {
            let mut window = self.take_or_open_window()?;
            let result = self.rewrite_created_file(&mut window, object_id, now, |bytes| {
                let size = usize::try_from(new_size)
                    .map_err(|_| CoreError::PrototypeLimit("file size limit reached"))?;
                bytes.resize(size, 0);
                Ok(())
            });
            return self.finish_created_rewrite(window, result);
        }
        let pending_size = self.window.as_ref().and_then(|window| {
            window
                .pending
                .file_layouts
                .get(&object_id)
                .map(|layout| layout.size_bytes)
        });
        if pending_size == Some(new_size) {
            return Ok(());
        }
        if pending_size.is_none() {
            let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
            if record.object_type != ObjectType::File {
                return Err(CoreError::IsDirectory);
            }
            if record.size_bytes == new_size {
                return Ok(());
            }
        }
        let mut window = self.take_or_open_window()?;
        if let Err(error) = self.ensure_pending_file_layout(&mut window.pending, object_id) {
            self.window = Some(window);
            return Err(error);
        }
        let expected_size = window.pending.file_layouts[&object_id].size_bytes;
        if expected_size == new_size {
            self.window = Some(window);
            return Ok(());
        }
        let current_extents = window.pending.file_layouts[&object_id].extents.clone();
        let block_size = self.dev.block_size() as u64;
        // Set before the replacement tail block is written; see
        // `window_write_file_at_untraced` for the preflight-refusal rule.
        let mut mutated = false;
        let result = (|| {
            let mut logical_start = 0;
            let mut extents = Vec::new();
            let mut content_crc = 0;
            if new_size < expected_size && !new_size.is_multiple_of(block_size) {
                let logical_block = new_size / block_size;
                if extent_at(&current_extents, logical_block)
                    .is_some_and(|extent| extent.flags & EXTENT_UNWRITTEN == 0)
                {
                    let mut block = vec![0u8; block_size as usize];
                    self.read_layout_block(&current_extents, logical_block, &mut block)?;
                    block[(new_size % block_size) as usize..].fill(0);
                    let physical_start = window.tx.allocate(&mut self.dev)?;
                    let zeroed = crate::flight::DataContext {
                        scope: crate::flight::DataScope::TruncateTailZero,
                        object_id,
                        offset: logical_block * block_size,
                        length: block_size,
                        start: physical_start,
                        blocks: 1,
                    };
                    self.flight_data_event(
                        crate::flight::EventKind::DataWriteBegin,
                        zeroed,
                        window.generation,
                    );
                    mutated = true;
                    if let Err(error) = self.dev.write_block(physical_start, &block) {
                        self.flight_data_event(
                            crate::flight::EventKind::DataWriteFailed,
                            zeroed,
                            window.generation,
                        );
                        return Err(error.into());
                    }
                    self.flight_data_event(
                        crate::flight::EventKind::DataWriteComplete,
                        zeroed,
                        window.generation,
                    );
                    window.pending.prewritten_data_blocks = window
                        .pending
                        .prewritten_data_blocks
                        .checked_add(1)
                        .ok_or(CoreError::PrototypeLimit("window data accounting overflow"))?;
                    logical_start = logical_block;
                    extents.push((physical_start, 1));
                    content_crc = crc32c(&block);
                }
            }
            self.install_logged_truncate(
                &mut window.tx,
                &mut window.pending,
                object_id,
                logical_start,
                expected_size,
                new_size,
                &extents,
                now,
                window.generation,
            )?;
            window.unlogged.push(LogOp::Truncate {
                object_id,
                logical_start,
                expected_size_bytes: expected_size,
                new_size_bytes: new_size,
                content_crc,
                timestamp: now,
                extents,
            });
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.window = Some(window);
                Ok(())
            }
            Err(error) if !mutated => {
                self.window = Some(window);
                Err(error)
            }
            Err(error) => {
                self.window_poisoned = true;
                self.flight_window_event(
                    window.generation,
                    crate::flight::EventKind::WindowFailed,
                    None,
                );
                self.flight_window_event(
                    window.generation,
                    crate::flight::EventKind::WindowClosed,
                    None,
                );
                Err(error)
            }
        }
    }

    /// The window-created, not-yet-logged object a delete or replacing
    /// rename would cancel, if any.
    fn window_cancel_target(
        &mut self,
        pending: &PendingBatch,
        op: &BatchOp<'_>,
    ) -> Result<Option<u64>, CoreError> {
        let (parent_id, name) = match op {
            BatchOp::DeleteFile { parent_id, name } => (*parent_id, *name),
            BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace: true,
                ..
            } => {
                if source_parent_id == target_parent_id
                    && self.comparison_key(source_name.as_bytes())?
                        == self.comparison_key(target_name.as_bytes())?
                {
                    return Ok(None);
                }
                (*target_parent_id, *target_name)
            }
            _ => return Ok(None),
        };
        if validate_name(name.as_bytes()).is_err() {
            return Ok(None);
        }
        let key = self.comparison_key(name.as_bytes())?;
        let Some(entry) = self.batch_lookup(pending, parent_id, &key)? else {
            return Ok(None);
        };
        Ok((pending.created_data.contains_key(&entry.child_id)
            && !pending.logged_created.contains(&entry.child_id))
        .then_some(entry.child_id))
    }

    /// The log record describing `op`, or `None` when the log has no record
    /// kind for it. `None` makes the window unloggable: the next fsync
    /// commits it as a checkpoint instead of appending (ADR-121 decision 5).
    fn log_op_for(
        op: &BatchOp<'_>,
        created: Option<u64>,
        pending: &PendingBatch,
        now: Timespec,
    ) -> Result<Option<LogOp>, CoreError> {
        Ok(Some(match op {
            BatchOp::CreateFile {
                parent_id,
                name,
                content,
            } => {
                let object_id = created
                    .ok_or_else(|| CoreError::Corrupt("create produced no object ID".into()))?;
                let (start, blocks) = pending
                    .created_data
                    .get(&object_id)
                    .copied()
                    .ok_or_else(|| CoreError::Corrupt("created data run missing".into()))?;
                let extents = if blocks > 0 {
                    vec![(start, blocks as u32)]
                } else {
                    Vec::new()
                };
                LogOp::Create {
                    parent_id: *parent_id,
                    name: name.as_bytes().to_vec(),
                    expected_object_id: object_id,
                    size_bytes: content.len() as u64,
                    content_crc: crc32c(content),
                    extents,
                    timestamp: now,
                }
            }
            // `LogOp` has Create, Delete, Rename and Write, and none of them
            // makes a directory. Adding one is a format change; until then a
            // staged directory is published by a checkpoint.
            BatchOp::CreateDirectory { .. } => return Ok(None),
            BatchOp::DeleteFile { parent_id, name } => LogOp::Delete {
                parent_id: *parent_id,
                name: name.as_bytes().to_vec(),
                timestamp: now,
            },
            BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace,
                ..
            } => LogOp::Rename {
                source_parent_id: *source_parent_id,
                source_name: source_name.as_bytes().to_vec(),
                target_parent_id: *target_parent_id,
                target_name: target_name.as_bytes().to_vec(),
                replace: *replace,
                timestamp: now,
            },
        }))
    }

    /// Makes every window operation so far durable. Existing-file update
    /// data receives its own barrier before the record that references it;
    /// the record then receives the completion barrier. Namespace-only
    /// groups keep the original one-barrier path (ADR-037/063).
    pub fn window_fsync(&mut self) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowFsync, |volume| {
            volume.window_fsync_untraced()
        })
    }

    fn window_fsync_untraced(&mut self) -> Result<(), CoreError> {
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        if self.window_poisoned {
            return Err(CoreError::WindowPoisoned);
        }
        if let (Some(window), Some(recorder)) = (&self.window, &mut self.flight) {
            recorder.borrow_mut().observe_window(
                window.generation,
                window.logged_records,
                true,
                self.window_poisoned,
            );
        }
        let Some(window) = self.window.as_mut() else {
            return Ok(());
        };
        let generation = window.generation;
        if window.unloggable {
            return Err(CoreError::PrototypeLimit(
                "the window holds changes the log cannot record; commit the window",
            ));
        }
        if window.unlogged.is_empty() {
            self.dev.flush()?;
            self.flight_intent_io_event(crate::flight::EventKind::IntentEmptyFlush, generation);
            return Ok(());
        }
        if window.unlogged.len() > MAX_LOG_OPS {
            return Err(CoreError::PrototypeLimit(
                "fsync group exceeds one log record; commit the window",
            ));
        }
        let needs_data_barrier = window
            .unlogged
            .iter()
            .any(|op| op.is_existing_file_update() && !op.data_extents().is_empty());
        if needs_data_barrier {
            if let Err(error) = self.dev.flush() {
                self.window_poisoned = true;
                self.flight_window_event(generation, crate::flight::EventKind::WindowFailed, None);
                return Err(error.into());
            }
            // Field-level borrows only: the open window stays borrowed here.
            if let Some(recorder) = &self.flight {
                recorder.borrow_mut().intent_io_event(
                    generation,
                    crate::flight::EventKind::IntentDataDurable,
                    self.window_poisoned,
                );
            }
        }
        let geo = self.ident.geometry();
        let slots = intent_log::log_slot_lbas(&geo, self.ident.log_slots)?;
        let sequence = window.logged_records + 1;
        if sequence as usize > slots.len() {
            return Err(CoreError::PrototypeLimit(
                "intent log is full; commit the window",
            ));
        }
        let record = LogRecord {
            uuid: self.ident.uuid,
            base_generation: self.checkpoint.generation,
            sequence,
            ops: window.unlogged.clone(),
        };
        let encoded = match record.encode(geo.block_size) {
            Ok(encoded) => encoded,
            Err(FormatError::Overflow(_)) => {
                return Err(CoreError::PrototypeLimit(
                    "fsync group exceeds one log record; commit the window",
                ));
            }
            Err(error) => return Err(CoreError::Format(error)),
        };
        self.flight_window_event(
            generation,
            crate::flight::EventKind::WindowLogBegin,
            Some(sequence),
        );
        let write = self
            .dev
            .write_block(slots[sequence as usize - 1], &encoded)
            .and_then(|()| self.dev.flush());
        match write {
            Ok(()) => {
                let window = self.window.as_mut().expect("window checked above");
                window.logged_records = sequence;
                window.unlogged.clear();
                for op in &record.ops {
                    if let LogOp::Create {
                        expected_object_id, ..
                    } = op
                    {
                        window.pending.logged_created.insert(*expected_object_id);
                    }
                }
                self.flight_window_event(
                    generation,
                    crate::flight::EventKind::WindowLogDurable,
                    Some(sequence),
                );
                Ok(())
            }
            Err(error) => {
                self.window_poisoned = true;
                self.flight_window_event(
                    generation,
                    crate::flight::EventKind::WindowLogFailed,
                    Some(sequence),
                );
                Err(error.into())
            }
        }
    }

    /// Materializes the open window as one checkpoint transaction. Always
    /// publishes a checkpoint when any record was logged, so stale records
    /// can never be mistaken for live ones.
    /// Abandon the open data window and say how many acknowledged operations
    /// it was holding.
    ///
    /// The only way out of a poisoned window. A window is poisoned when its
    /// staged work cannot be published, and every operation that needs the
    /// window closed then refuses; the poison is cleared only by a successful
    /// commit, and a poisoned window refuses to commit. A volume that ran out
    /// of space during one write therefore refused EVERY later operation for
    /// ever, including the delete that would have freed the space, while
    /// almost all of its blocks were free. A mount could not even be unmounted
    /// afterwards.
    ///
    /// What this discards was never durable: the window is exactly the work
    /// that has not reached a checkpoint. It was acknowledged to the caller,
    /// so losing it is a real loss and the count is returned rather than
    /// swallowed; the caller says so. Losing it is what already happened. The
    /// choice this makes is only whether the volume stays usable afterwards.
    pub fn window_discard(&mut self) -> u32 {
        self.trace_api(crate::flight::ApiMethod::WindowDiscard, |volume| {
            Ok::<_, std::convert::Infallible>(volume.window_discard_untraced())
        })
        .unwrap_or_else(|never| match never {})
    }

    fn window_discard_untraced(&mut self) -> u32 {
        let Some(window) = self.window.take() else {
            self.window_poisoned = false;
            return 0;
        };
        let lost = window.logged_records + window.unlogged.len() as u32;
        let generation = window.generation;
        drop(window);
        self.window_poisoned = false;
        self.flight_window_event(generation, crate::flight::EventKind::WindowClosed, None);
        lost
    }

    pub fn window_commit(&mut self, now: Timespec) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowCommit, |volume| {
            volume.window_commit_untraced(now)
        })
    }

    fn window_commit_untraced(&mut self, now: Timespec) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        if self.window_poisoned {
            return Err(CoreError::WindowPoisoned);
        }
        let Some(window) = self.window.take() else {
            return Ok(());
        };
        let generation = window.generation;
        if let Some(recorder) = &mut self.flight {
            recorder.borrow_mut().observe_window(
                generation,
                window.logged_records,
                true,
                self.window_poisoned,
            );
        }
        let force = window.logged_records > 0;
        let result = self.materialize_batch(
            window.tx,
            window.pending,
            now,
            window.generation,
            force,
            CommitData::Prepared(Vec::new()),
        );
        if result.is_err() {
            self.window_poisoned = true;
            self.flight_window_event(generation, crate::flight::EventKind::WindowFailed, None);
        }
        self.flight_window_event(generation, crate::flight::EventKind::WindowClosed, None);
        result
    }

    /// Replays the intent log after mount (ADR-037): re-runs the valid
    /// record prefix through the batch engine, claiming each logged create's
    /// exact data extents, and publishes one checkpoint. Returns the number
    /// of records replayed.
    pub(crate) fn recover_intent_log(&mut self) -> Result<u32, CoreError> {
        if self.ident.log_slots == 0 {
            return Ok(0);
        }
        let generation = self.checkpoint.generation;
        self.flight_mount_event(
            crate::flight::EventKind::MountIntentBegin,
            u32::from(self.ident.log_slots),
            false,
            0,
            generation,
        );
        let geo = self.ident.geometry();
        let scanned = intent_log::scan(
            &mut self.dev,
            &geo,
            self.ident.log_slots,
            &self.ident.uuid,
            self.checkpoint.generation,
            self.ident.features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0,
        )?;
        self.observe_scanned_intent_log(&scanned, generation);
        if scanned.records.is_empty() {
            self.pending_intent_records = 0;
            return Ok(0);
        }
        let generation = self.next_generation()?;
        // Same zero-promotion rule as the live window: the recorded extents
        // are FREE in the committed bitmaps and must claim cleanly.
        let mut tx = TxAllocator::begin_observed(
            crate::flight::AllocationObserver::new(self.flight.as_ref()),
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            0,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        let mut pending = PendingBatch {
            dir_changes: BTreeMap::new(),
            dir_timestamps: BTreeMap::new(),
            records: BTreeMap::new(),
            committed_record_lbas: BTreeMap::new(),
            created_data: BTreeMap::new(),
            created_directories: BTreeSet::new(),
            file_layouts: BTreeMap::new(),
            window_allocations: Vec::new(),
            prewritten_data_blocks: 0,
            write_through: true,
            logged_created: BTreeSet::new(),
            sacrificed: Vec::new(),
            uses_emergency_headroom: false,
            next_object_id: self.checkpoint.next_object_id,
        };
        let records = scanned.records;
        // Claim every record-referenced data run before allocating replay
        // metadata. The committed bitmap intentionally still calls these
        // runs FREE; pre-claiming prevents a lazily created orphan tree from
        // selecting an LBA named by a later record in the same prefix.
        for record in &records {
            for op in &record.ops {
                for (start, blocks) in op.data_extents() {
                    tx.allocate_exact_run(&mut self.dev, *start, u64::from(*blocks))?;
                }
            }
        }
        let mut last_timestamp = Timespec::default();
        let replayed = records.len() as u32;
        for record in records {
            let sequence = record.sequence;
            let operations = record.ops.len() as u32;
            for op in record.ops {
                last_timestamp = op.timestamp();
                self.apply_log_op(&mut tx, &mut pending, &op, generation)?;
            }
            self.flight_mount_event(
                crate::flight::EventKind::MountIntentReplayed,
                operations,
                false,
                sequence,
                generation,
            );
        }
        self.materialize_batch(
            tx,
            pending,
            last_timestamp,
            generation,
            true,
            CommitData::Prepared(Vec::new()),
        )?;
        self.pending_intent_records = 0;
        Ok(replayed)
    }

    /// Report the scanned prefix and whether it ended at a damaged tail.
    fn observe_scanned_intent_log(&self, scanned: &intent_log::ScannedLog, generation: u64) {
        self.flight_mount_event(
            crate::flight::EventKind::MountIntentScanned,
            scanned.records.len() as u32,
            scanned.tail_note.is_some(),
            scanned.records.last().map_or(0, |record| record.sequence),
            generation,
        );
    }

    pub(crate) fn inspect_intent_log(&mut self) -> Result<u32, CoreError> {
        if self.ident.log_slots == 0 {
            self.pending_intent_records = 0;
            return Ok(0);
        }
        let generation = self.checkpoint.generation;
        self.flight_mount_event(
            crate::flight::EventKind::MountIntentBegin,
            u32::from(self.ident.log_slots),
            false,
            0,
            generation,
        );
        let scanned = intent_log::scan(
            &mut self.dev,
            &self.ident.geometry(),
            self.ident.log_slots,
            &self.ident.uuid,
            self.checkpoint.generation,
            self.ident.features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0,
        )?;
        self.observe_scanned_intent_log(&scanned, generation);
        self.pending_intent_records = scanned.records.len() as u32;
        Ok(self.pending_intent_records)
    }

    fn apply_log_op(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        op: &LogOp,
        generation: u64,
    ) -> Result<(), CoreError> {
        let utf8 = |bytes: &[u8]| -> Result<String, CoreError> {
            String::from_utf8(bytes.to_vec())
                .map_err(|_| CoreError::Corrupt("log record name is not UTF-8".into()))
        };
        match op {
            LogOp::Create {
                parent_id,
                name,
                expected_object_id,
                size_bytes,
                extents,
                timestamp,
                ..
            } => self.apply_replay_create(
                pending,
                *parent_id,
                &utf8(name)?,
                *expected_object_id,
                *size_bytes,
                extents,
                *timestamp,
                generation,
            ),
            LogOp::Delete {
                parent_id,
                name,
                timestamp,
            } => {
                let name = utf8(name)?;
                self.apply_batch_op(
                    tx,
                    pending,
                    &BatchOp::DeleteFile {
                        parent_id: *parent_id,
                        name: &name,
                    },
                    *timestamp,
                    generation,
                    true,
                )
                .map(|_| ())
            }
            LogOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace,
                timestamp,
            } => {
                let source = utf8(source_name)?;
                let target = utf8(target_name)?;
                self.apply_batch_op(
                    tx,
                    pending,
                    &BatchOp::Rename {
                        source_parent_id: *source_parent_id,
                        source_name: &source,
                        target_parent_id: *target_parent_id,
                        target_name: &target,
                        replace: *replace,
                    },
                    *timestamp,
                    generation,
                    true,
                )
                .map(|_| ())
            }
            LogOp::Write {
                object_id,
                logical_start,
                expected_size_bytes,
                new_size_bytes,
                extents,
                timestamp,
                ..
            } => self.install_logged_file_range(
                tx,
                pending,
                *object_id,
                *logical_start,
                *expected_size_bytes,
                *new_size_bytes,
                extents,
                *timestamp,
                generation,
            ),
            LogOp::Truncate {
                object_id,
                logical_start,
                expected_size_bytes,
                new_size_bytes,
                extents,
                timestamp,
                ..
            } => self.install_logged_truncate(
                tx,
                pending,
                *object_id,
                *logical_start,
                *expected_size_bytes,
                *new_size_bytes,
                extents,
                *timestamp,
                generation,
            ),
        }
    }

    /// The create branch of the batch engine with pre-placed content: the
    /// logged extents are claimed exactly, and the data — already on disk
    /// and CRC-verified by the scan — is never rewritten.
    #[allow(clippy::too_many_arguments)]
    fn apply_replay_create(
        &mut self,
        pending: &mut PendingBatch,
        parent_id: u64,
        name: &str,
        expected_object_id: u64,
        size_bytes: u64,
        extents: &[(u64, u32)],
        now: Timespec,
        generation: u64,
    ) -> Result<(), CoreError> {
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self
            .batch_record(pending, parent_id)?
            .ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let key = self.comparison_key(name.as_bytes())?;
        if self.batch_lookup(pending, parent_id, &key)?.is_some() {
            return Err(CoreError::Corrupt(
                "log replay found the name already present".into(),
            ));
        }
        // IDs are monotonic and never reused; scrubbed (cancelled-unlogged)
        // creates leave legal gaps that replay skips over.
        if expected_object_id < pending.next_object_id {
            return Err(CoreError::Corrupt(format!(
                "log replay expected object {expected_object_id} below allocator watermark {}",
                pending.next_object_id
            )));
        }
        let object_id = expected_object_id;
        pending.next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;
        let (data_start, data_blocks) = match extents {
            [] => (0u64, 0u64),
            [(start, blocks)] => (*start, *blocks as u64),
            _ => {
                return Err(CoreError::PrototypeLimit(
                    "multi-extent logged creates are not implemented",
                ))
            }
        };
        let block_size = self.dev.block_size();
        pending
            .created_data
            .insert(object_id, (data_start, data_blocks));
        // The record that named this create is durable and its content CRC
        // still covers the run. A later record in the same replayed prefix
        // that cancels the create must quarantine the run, because reusing it
        // before the replay checkpoint publishes would break the record the
        // next recovery still has to read.
        pending.logged_created.insert(object_id);
        pending.records.insert(
            object_id,
            Some(ObjectRecord {
                object_id,
                object_type: ObjectType::File,
                flags: 0,
                link_count: 1,
                owner_uid: 0,
                owner_gid: 0,
                size_bytes,
                allocated_bytes: data_blocks * block_size as u64,
                security: None,
                attributes: None,
                comment: afsplus_format::object::Comment::EMPTY,
                created: now,
                modified: now,
                changed: now,
                protection: 0,
                content_generation: generation,
                data_root: data_start,
                data_blocks,
            }),
        );
        pending.dir_changes.entry(parent_id).or_default().insert(
            key,
            Some(DirEntry {
                key: self.comparison_key(name.as_bytes())?,
                name: name.as_bytes().to_vec(),
                child_type_hint: 1,
                child_id: object_id,
            }),
        );
        pending.dir_timestamps.insert(parent_id, now);
        Ok(())
    }

    /// Publishes the batch: one directory-tree mutation per touched
    /// directory, freshly encoded object records, one object-map mutation,
    /// one checkpoint.
    fn materialize_batch(
        &mut self,
        mut tx: TxAllocator,
        mut pending: PendingBatch,
        now: Timespec,
        generation: u64,
        force_commit: bool,
        data: CommitData<'_>,
    ) -> Result<(), CoreError> {
        let block_size = self.dev.block_size();
        let mut meta_writes: Vec<(u64, Vec<u8>)> = Vec::new();

        if pending.uses_emergency_headroom {
            tx.set_free_block_floor(0);
        }

        for (start, blocks) in std::mem::take(&mut pending.sacrificed) {
            tx.abandon_uncommitted_run(&mut self.dev, start, blocks)?;
        }

        let dir_ids: Vec<u64> = pending.dir_changes.keys().copied().collect();
        for dir_id in dir_ids {
            let is_new = pending.created_directories.remove(&dir_id);
            let changes = pending
                .dir_changes
                .remove(&dir_id)
                .expect("key listed above");
            let base = self
                .batch_record(&pending, dir_id)?
                .ok_or_else(|| CoreError::Corrupt(format!("directory {dir_id} disappeared")))?;
            let committed =
                if is_new {
                    None
                } else {
                    Some(self.read_object(dir_id)?.ok_or_else(|| {
                        CoreError::Corrupt(format!("directory {dir_id} disappeared"))
                    })?)
                };
            let mut encoded: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
            for (key, change) in changes {
                match change {
                    Some(entry) => {
                        let (entry_key, entry_value) =
                            directory::encode_entry(&self.ident, &entry)?;
                        debug_assert_eq!(entry_key, key);
                        encoded.push((entry_key, Some(entry_value)));
                    }
                    None => {
                        // A delete of a key with no committed entry is a
                        // same-batch create that was cancelled: skip it.
                        let existed = if let Some(committed) = committed {
                            directory::lookup_entry(
                                &mut self.dev,
                                &self.ident.geometry(),
                                committed.data_root,
                                dir_id,
                                self.checkpoint.generation,
                                &self.ident,
                                &key,
                            )?
                            .is_some()
                        } else {
                            false
                        };
                        if existed {
                            encoded.push((key, None));
                        }
                    }
                }
            }
            if encoded.is_empty() && !is_new {
                continue;
            }
            let operations: Vec<TreeOperation<'_>> = encoded
                .iter()
                .map(|(key, value)| match value {
                    Some(value) => TreeOperation::Upsert { key, value },
                    None => TreeOperation::Delete { key },
                })
                .collect();
            let mutation = if is_new {
                mutate_new_empty_tree(
                    &mut self.dev,
                    &self.ident.geometry(),
                    &mut tx,
                    base.data_root,
                    directory::spec(dir_id, self.checkpoint.generation),
                    generation,
                    &operations,
                )?
            } else {
                mutate_many(
                    &mut self.dev,
                    &self.ident.geometry(),
                    &mut tx,
                    base.data_root,
                    directory::spec(dir_id, self.checkpoint.generation),
                    generation,
                    &operations,
                )?
            };
            meta_writes.extend(mutation.writes);
            if !is_new {
                self.note_committed_record(&mut pending, dir_id)?;
            }
            let directory_timestamp = pending.dir_timestamps.remove(&dir_id).unwrap_or(now);
            pending.records.insert(
                dir_id,
                Some(ObjectRecord {
                    modified: directory_timestamp,
                    changed: directory_timestamp,
                    content_generation: generation,
                    data_root: mutation.root_lba,
                    ..base
                }),
            );
        }
        if !pending.created_directories.is_empty() {
            return Err(CoreError::Corrupt(
                "created directory has no materialization entry".into(),
            ));
        }

        // Materialize each existing-file window layout once, no matter how
        // many logged write/truncate operations produced its final state.
        let mut omap_encoded: Vec<([u8; 8], Option<[u8; 8]>)> = Vec::new();
        for (object_id, layout) in std::mem::take(&mut pending.file_layouts) {
            let record = pending
                .records
                .remove(&object_id)
                .flatten()
                .ok_or_else(|| CoreError::Corrupt("pending file record disappeared".into()))?;
            let remembered_lba = pending
                .committed_record_lbas
                .remove(&object_id)
                .ok_or_else(|| CoreError::Corrupt("pending file record LBA disappeared".into()))?;
            if remembered_lba != layout.record_lba {
                return Err(CoreError::Corrupt(
                    "pending file record LBA changed inside window".into(),
                ));
            }
            if record.flags & OBJECT_FLAG_EXTENT_TREE == 0
                && direct_layout(&layout.extents, layout.size_bytes, block_size as u64).is_none()
            {
                self.pending_layout_promotions += 1;
            }
            let modified = record.modified;
            let changed = record.changed;
            let staged = self.stage_file_layout(
                &mut tx,
                record,
                layout.record_lba,
                &layout.old_extents,
                &layout.old_tree_blocks,
                &layout.extents,
                layout.size_bytes,
                true,
                modified,
                changed,
                generation,
            )?;
            meta_writes.extend(staged.metadata_writes);
            omap_encoded.push((
                object_map::key(object_id),
                Some(object_map::value(staged.record_lba)?),
            ));
        }

        // Encode every other surviving pending record into a fresh block and
        // build the remaining object-map operation set.
        for (object_id, record) in &pending.records {
            match record {
                Some(record) => {
                    let lba = tx.allocate(&mut self.dev)?;
                    meta_writes.push((lba, record.encode(block_size, generation)?));
                    if let Some(old) = pending.committed_record_lbas.remove(object_id) {
                        tx.retire(&mut self.dev, old)?;
                    }
                    omap_encoded.push((object_map::key(*object_id), Some(object_map::value(lba)?)));
                }
                None => {
                    omap_encoded.push((object_map::key(*object_id), None));
                }
            }
        }
        if omap_encoded.is_empty() && !force_commit {
            // Every operation cancelled out; there is no state to publish.
            return Ok(());
        }
        let omap_operations: Vec<TreeOperation<'_>> = omap_encoded
            .iter()
            .map(|(key, value)| match value {
                Some(value) => TreeOperation::Upsert { key, value },
                None => TreeOperation::Delete { key },
            })
            .collect();
        let omap_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &omap_operations,
        )?;
        let omap_root = omap_mutation.root_lba;
        meta_writes.extend(omap_mutation.writes);

        self.pending_prewritten_data_blocks = pending.prewritten_data_blocks;
        self.commit_transaction_inner(
            generation,
            pending.next_object_id,
            tx,
            data,
            meta_writes,
            omap_root,
            None,
            None,
        )
    }

    fn next_generation(&mut self) -> Result<u64, CoreError> {
        // A fresh transaction must never see an aborted one's half-applied
        // reference edits (ADR-061): the generation number alone cannot
        // distinguish them, because an aborted commit does not consume it.
        self.shared_refs = None;
        self.pending_layout_promotions = 0;
        self.pending_in_place_data_blocks = 0;
        self.pending_reservation_initializations = 0;
        self.pending_prewritten_data_blocks = 0;
        self.checkpoint
            .generation
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("generation counter exhausted"))
    }

    /// Return the required byte count. A short buffer is left unchanged.
    /// Targets are opaque and never followed by this operation.
    pub fn read_link(&mut self, object_id: u64, output: &mut [u8]) -> Result<usize, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ReadLink, |volume| {
            volume.read_link_untraced(object_id, output)
        })
    }

    fn read_link_untraced(
        &mut self,
        object_id: u64,
        output: &mut [u8],
    ) -> Result<usize, CoreError> {
        self.ensure_public_object_id(object_id)?;
        let block = self.symlink_block(object_id)?;
        let (link, _) = afsplus_format::object::SymlinkRecord::decode(&block)?;
        let required = link.target.len();
        if output.len() >= required {
            output[..required].copy_from_slice(link.target.as_bytes());
        }
        Ok(required)
    }

    fn symlink_block(&mut self, object_id: u64) -> Result<Vec<u8>, CoreError> {
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::Symlink {
            return Err(CoreError::InvalidMetadata("object is not a symlink"));
        }
        let lba = self
            .object_record_lba(object_id)?
            .ok_or(CoreError::NotFound)?;
        let mut block = vec![0; self.dev.block_size()];
        self.dev.read_block(lba, &mut block)?;
        let (link, generation) = afsplus_format::object::SymlinkRecord::decode(&block)?;
        if link.record != record || generation == 0 || generation > self.checkpoint.generation {
            return Err(CoreError::Corrupt(
                "symlink changed during target lookup".into(),
            ));
        }
        Ok(block)
    }

    fn encode_preserving_target(
        &mut self,
        record: ObjectRecord,
        generation: u64,
    ) -> Result<Vec<u8>, CoreError> {
        if record.object_type != ObjectType::Symlink {
            return Ok(record.encode(self.dev.block_size(), generation)?);
        }
        let block = self.symlink_block(record.object_id)?;
        let (old, _) = afsplus_format::object::SymlinkRecord::decode(&block)?;
        Ok(afsplus_format::object::SymlinkRecord {
            record,
            target: old.target,
        }
        .encode(self.dev.block_size(), generation)?)
    }

    pub fn unlink_symlink(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::UnlinkSymlink, |volume| {
            volume.unlink_symlink_untraced(parent_id, name, now)
        })
    }

    fn unlink_symlink_untraced(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.remove_entry(parent_id, name, now, ObjectType::Symlink)
    }

    fn read_object(&mut self, object_id: u64) -> Result<Option<ObjectRecord>, CoreError> {
        if object_id == OBJECT_ROOT {
            self.observe_object_mapping(object_id, crate::flight::EventKind::ObjectLookup, 0);
            self.observe_object_mapping(
                object_id,
                crate::flight::EventKind::ObjectMapped,
                self.state.root_record_lba,
            );
            return Ok(Some(self.state.root_object));
        }
        let Some(lba) = self.lookup_object_lba(object_id)? else {
            return Ok(None);
        };
        if !self.ident.geometry().is_allocatable(lba) {
            return Err(CoreError::Corrupt(format!(
                "object {object_id} record block {lba} outside allocatable bounds"
            )));
        }
        let mut buf = vec![0u8; self.dev.block_size()];
        self.dev.read_block(lba, &mut buf)?;
        let (record, block_generation) = decode_record(&mut self.dev, lba, &buf)
            .map_err(|e| CoreError::Corrupt(format!("object {object_id} record invalid: {e}")))?;
        if block_generation == 0 || block_generation > self.checkpoint.generation {
            return Err(CoreError::Corrupt(format!(
                "object {object_id} record block {lba} generation {block_generation} outside committed range"
            )));
        }
        if record.flags & OBJECT_FLAG_DATA_IN_PLACE != 0 && !self.data_policy_enabled() {
            return Err(CoreError::Corrupt(format!(
                "object {object_id} carries OBJECT_FLAG_DATA_IN_PLACE without the data-policy feature"
            )));
        }
        // Where a well-formed reference points is chain state, not record
        // admission (ADR-105): the object stays reachable and deletable, and
        // a first segment outside the volume is chain damage like any other.
        if record.security.is_some() && !self.security_descriptors_enabled() {
            return Err(CoreError::Corrupt(format!(
                "object {object_id} carries a security reference without the security-descriptors feature"
            )));
        }
        if record.object_id != object_id {
            return Err(CoreError::Corrupt(format!(
                "object record at block {lba} claims ID {}, map says {object_id}",
                record.object_id
            )));
        }
        if record.object_type == ObjectType::File && record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            extent_map::validate_root(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                object_id,
                self.checkpoint.generation,
            )?;
        } else if record.object_type == ObjectType::File {
            let data_end = record
                .data_root
                .checked_add(record.data_blocks)
                .ok_or_else(|| CoreError::Corrupt(format!("object {object_id} extent overflow")))?;
            for block in record.data_root..data_end {
                if !self.ident.geometry().is_allocatable(block) {
                    return Err(CoreError::Corrupt(format!(
                        "object {object_id} data block {block} outside allocatable bounds"
                    )));
                }
            }
        }
        Ok(Some(record))
    }

    fn observe_object_mapping(
        &mut self,
        object_id: u64,
        kind: crate::flight::EventKind,
        record_block: u64,
    ) {
        if let Some(recorder) = &mut self.flight {
            recorder.borrow_mut().object_event(
                self.checkpoint.generation,
                kind,
                crate::flight::ObjectContext {
                    object_id,
                    record_block,
                    view_id: 0,
                },
                self.window_poisoned,
            );
        }
    }

    fn lookup_object_lba(&mut self, object_id: u64) -> Result<Option<u64>, CoreError> {
        self.observe_object_mapping(object_id, crate::flight::EventKind::ObjectLookup, 0);
        let lba = object_map::lookup_lba(
            &mut self.dev,
            &self.ident.geometry(),
            self.checkpoint.object_map_block,
            self.checkpoint.generation,
            object_id,
        )?;
        self.observe_object_mapping(
            object_id,
            if lba.is_some() {
                crate::flight::EventKind::ObjectMapped
            } else {
                crate::flight::EventKind::ObjectMissing
            },
            lba.unwrap_or(0),
        );
        Ok(lba)
    }

    fn object_record_lba(&mut self, object_id: u64) -> Result<Option<u64>, CoreError> {
        if object_id == OBJECT_ROOT {
            self.observe_object_mapping(object_id, crate::flight::EventKind::ObjectLookup, 0);
            self.observe_object_mapping(
                object_id,
                crate::flight::EventKind::ObjectMapped,
                self.state.root_record_lba,
            );
            Ok(Some(self.state.root_record_lba))
        } else {
            self.lookup_object_lba(object_id)
        }
    }

    /// Returns the mutation plus the new checkpoint's node set for the
    /// cross-commit cache.
    fn mutate_allocation_root(
        &mut self,
        generation: u64,
        dirty_records: &[(u32, afsplus_format::checkpoint::RegionRecord)],
    ) -> Result<(TreeMutation, Vec<u64>), CoreError> {
        if self.checkpoint.allocation_root_block == 0 {
            return Err(CoreError::PrototypeLimit(
                "inline allocation checkpoint cannot be mutated",
            ));
        }
        let geo = self.ident.geometry();
        let (current_blocks, older_blocks) = match self.allocation_tree_cache.take() {
            Some(cached) => cached,
            None => {
                let current = allocation_root::load_tree_blocks(
                    &mut self.dev,
                    &geo,
                    self.checkpoint.allocation_root_block,
                    self.checkpoint.generation,
                )?;
                let older = if let Some(older) = self
                    .other_checkpoint
                    .as_ref()
                    .filter(|checkpoint| checkpoint.allocation_root_block != 0)
                {
                    allocation_root::load_tree_blocks(
                        &mut self.dev,
                        &geo,
                        older.allocation_root_block,
                        older.generation,
                    )?
                } else {
                    Vec::new()
                };
                (current, older)
            }
        };
        // The take() above cleared the cache; restore it so an aborted
        // commit leaves the committed view intact.
        self.allocation_tree_cache = Some((current_blocks.clone(), older_blocks.clone()));
        let pool_lbas = allocation_root::reserved_pool_lbas(&geo)?;
        let mut pool = ReservedTreePool::new(pool_lbas, &current_blocks, &older_blocks)?
            .with_tree_cache_pages(self.tree_cache_pages);
        let encoded: Vec<_> = dirty_records
            .iter()
            .map(|(region, record)| {
                Ok((
                    allocation_root::key(*region),
                    allocation_root::value(*record)?,
                ))
            })
            .collect::<Result<_, CoreError>>()?;
        let operations: Vec<_> = encoded
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let mutation = mutate_many(
            &mut self.dev,
            &geo,
            &mut pool,
            self.checkpoint.allocation_root_block,
            allocation_root::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        // New node set = current − retired paths ∪ freshly written images.
        let retired: std::collections::BTreeSet<u64> = pool.retired_nodes().collect();
        let mut new_blocks: Vec<u64> = current_blocks
            .iter()
            .copied()
            .filter(|lba| !retired.contains(lba))
            .collect();
        new_blocks.extend(pool.allocated_nodes());
        new_blocks.sort_unstable();
        new_blocks.dedup();
        Ok((mutation, new_blocks))
    }

    /// The common commit tail: durability ordering, checkpoint write, state
    /// adoption, accounting. Errors before publication leave the selected
    /// generation unchanged. Once checkpoint I/O starts, an error makes the
    /// durable outcome uncertain and further mutations require remount.
    #[allow(clippy::too_many_arguments)]
    fn commit_transaction(
        &mut self,
        generation: u64,
        next_object_id: u64,
        tx: TxAllocator,
        data_writes: Vec<(u64, Vec<u8>)>,
        meta_writes: Vec<(u64, Vec<u8>)>,
        new_object_map_block: u64,
    ) -> Result<(), CoreError> {
        self.commit_transaction_inner(
            generation,
            next_object_id,
            tx,
            CommitData::Prepared(data_writes),
            meta_writes,
            new_object_map_block,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_transaction_inner(
        &mut self,
        generation: u64,
        next_object_id: u64,
        tx: TxAllocator,
        data_writes: CommitData<'_>,
        meta_writes: Vec<(u64, Vec<u8>)>,
        new_object_map_block: u64,
        snapshot_change: Option<SnapshotRegistryChange>,
        // A relabel passes its label here. Intent travels as a value, never
        // as a field, so an unwind cannot leave it for a later commit.
        label: Option<&str>,
    ) -> Result<(), CoreError> {
        self.flight_event(generation, crate::flight::EventKind::Begin);
        let result = self.commit_transaction_body(
            generation,
            next_object_id,
            tx,
            data_writes,
            meta_writes,
            new_object_map_block,
            snapshot_change,
            label,
        );
        self.flight_event(
            generation,
            if result.is_ok() {
                crate::flight::EventKind::Adopted
            } else {
                crate::flight::EventKind::Failed
            },
        );
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_transaction_body(
        &mut self,
        generation: u64,
        next_object_id: u64,
        mut tx: TxAllocator,
        data_writes: CommitData<'_>,
        mut meta_writes: Vec<(u64, Vec<u8>)>,
        new_object_map_block: u64,
        snapshot_change: Option<SnapshotRegistryChange>,
        // A relabel passes its label here. Intent travels as a value, never
        // as a field, so an unwind cannot leave it for a later commit.
        label: Option<&str>,
    ) -> Result<(), CoreError> {
        let block_size = self.dev.block_size();

        tx.begin_snapshot_housekeeping()?;
        let registry_nodes =
            self.prepare_snapshot_registry(&mut tx, generation, snapshot_change, &mut meta_writes)?;

        // ADR-061: publish the shared-extent reference edit, if this
        // transaction made one, in the same checkpoint that publishes the
        // extent maps it describes. The first clone allocates the root and
        // it stays allocated afterwards, even once the tree is empty again.
        let mut shared_root = self.checkpoint.shared_extent_root_block;
        let mut shared_stats = RefEditStats::default();
        let mut shared_nodes_written = 0u64;
        if let Some((opened_for, edit)) = self.shared_refs.take() {
            if opened_for == generation {
                let publication = edit.finish()?;
                shared_stats = publication.stats;
                if shared_root == 0 && (publication.changed() || publication.root_required()) {
                    // The volume's first clone: build the initial tree from
                    // the overlay's canonical records (complete here, since
                    // no committed tree existed), on transactionally
                    // reserved blocks — even empty, so root zero keeps
                    // meaning "never cloned" (ADR-061).
                    let node_count =
                        shared_extents::bulk_node_count(block_size, publication.records.len())?;
                    let mut lbas = Vec::with_capacity(node_count);
                    for _ in 0..node_count {
                        lbas.push(tx.allocate(&mut self.dev)?);
                    }
                    let built =
                        shared_extents::bulk_build(block_size, &publication.records, &lbas)?;
                    shared_nodes_written = built.nodes.len() as u64;
                    for (lba, node) in built.nodes {
                        meta_writes.push((lba, node.encode(block_size, generation)?));
                    }
                    shared_root = built.root_lba;
                } else if publication.changed() {
                    {
                        let operations: Vec<TreeOperation<'_>> = publication
                            .deletes
                            .iter()
                            .map(|key| TreeOperation::Delete { key: &key[..] })
                            .chain(publication.upserts.iter().map(|(key, value)| {
                                TreeOperation::Upsert {
                                    key: &key[..],
                                    value: &value[..],
                                }
                            }))
                            .collect();
                        let mutation = mutate_many(
                            &mut self.dev,
                            &self.ident.geometry(),
                            &mut tx,
                            shared_root,
                            shared_extents::spec(self.checkpoint.generation),
                            generation,
                            &operations,
                        )?;
                        shared_nodes_written = mutation.stats.final_nodes_written;
                        meta_writes.extend(mutation.writes);
                        shared_root = mutation.root_lba;
                    }
                }
            }
        }
        let layout_promotions = std::mem::take(&mut self.pending_layout_promotions);
        let in_place_data_blocks = std::mem::take(&mut self.pending_in_place_data_blocks);
        let initialized_reservations =
            std::mem::take(&mut self.pending_reservation_initializations);
        let prewritten_data_blocks = std::mem::take(&mut self.pending_prewritten_data_blocks);

        let mut snapshot_stats = self.prepare_snapshot_lifetimes(&mut tx)?;
        snapshot_stats.registry_nodes_written = registry_nodes;
        let finished = tx.finish(&mut self.dev)?;
        if let Some(lifetimes) = finished.snapshot_lifetimes {
            snapshot_stats.lifetime_nodes_written = lifetimes.tree.stats.final_nodes_written;
            snapshot_stats.ledger_retired_blocks = lifetimes.state.retained_blocks;
            meta_writes.extend(lifetimes.tree.writes);
        }
        let (allocation_root, new_allocation_tree_blocks) =
            self.mutate_allocation_root(generation, &finished.dirty_records)?;
        let new_allocation_root_block = allocation_root.root_lba;
        let mut tree_mutations = finished.tree_mutations;
        tree_mutations.include(allocation_root.stats);
        meta_writes.extend(allocation_root.writes);
        meta_writes.extend(finished.reclaim_writes);

        let data_blocks = data_writes.blocks(block_size);
        let mut stats = CommitStats {
            tree_mutations,
            data_blocks_written: data_blocks + prewritten_data_blocks,
            data_blocks_overwritten_in_place: in_place_data_blocks,
            data_blocks_initialized_from_reservation: initialized_reservations,
            metadata_blocks_written: meta_writes.len() as u64 + tree_mutations.staged_spill_writes,
            bitmap_pages_written: finished.bitmap_writes.len() as u64,
            region_descriptors_written: finished.descriptor_writes.len() as u64,
            allocation_records_updated: finished.dirty_records.len() as u64,
            allocation_tree_nodes_written: allocation_root.stats.final_nodes_written,
            checkpoint_blocks_written: 1,
            shared_tree_nodes_written: shared_nodes_written,
            layout_promotions,
            shared_refs: shared_stats,
            alloc: finished.stats,
            snapshots: snapshot_stats,
            ..CommitStats::default()
        };

        // 1. User data, then barrier — only when the transaction has data.
        data_writes.write_to(&mut self.dev)?;
        if data_blocks != 0 {
            self.dev.flush()?;
            stats.flushes += 1;
        }

        self.flight_event(generation, crate::flight::EventKind::DataWritesComplete);

        // 2. COW metadata, dirty bitmap pages, then their new region
        // descriptors, all before the publication barrier.
        for (lba, block) in meta_writes
            .iter()
            .chain(finished.bitmap_writes.iter())
            .chain(finished.descriptor_writes.iter())
        {
            self.dev.write_block(*lba, block)?;
        }
        self.dev.flush()?;
        stats.flushes += 1;

        self.flight_event(generation, crate::flight::EventKind::MetadataDurable);

        // 3. Alternate checkpoint slot, then the commit barrier.
        let new_slot = 1 - self.current_slot;
        let free_blocks_total = finished.free_blocks_total;
        let new_checkpoint = Checkpoint {
            uuid: self.ident.uuid,
            generation,
            root_object_id: OBJECT_ROOT,
            object_map_block: new_object_map_block,
            allocation_root_block: new_allocation_root_block,
            reclaim_root_block: finished.reclaim_root_lba,
            next_object_id,
            committed_tx_id: generation,
            free_blocks_total,
            flags: 0,
            shared_extent_root_block: shared_root,
            // The label travels with every commit; a relabel is the commit
            // that carries a new one.
            label: label.unwrap_or(&self.checkpoint.label).to_owned(),
            snapshot_roots: finished.snapshot_roots,
        };
        let checkpoint_bytes = new_checkpoint.encode(block_size)?;
        // A failed write may have reached the device, and a failed flush may
        // leave a complete new checkpoint visible. Never retry using the old
        // allocation state after publication has begun. Also cover failures
        // while adopting the newly committed roots below.
        self.window_poisoned = true;
        self.flight_event(generation, crate::flight::EventKind::PublicationBegin);
        self.dev
            .write_block(self.ident.checkpoint_slots[new_slot], &checkpoint_bytes)?;
        self.dev.flush()?;
        stats.flushes += 1;

        self.flight_event(generation, crate::flight::EventKind::CheckpointDurable);

        stats.bytes_written = (stats.data_blocks_written
            + stats.metadata_blocks_written
            + stats.bitmap_pages_written
            + stats.region_descriptors_written
            + stats.checkpoint_blocks_written)
            * block_size as u64;

        // Adopt the committed roots through the same bounded loader as a
        // remount. The transaction already owns the exact new bitmap state,
        // so no post-commit full-volume reload is needed.
        self.state = load_mount_state(&mut self.dev, &self.ident, &new_checkpoint)?;
        // Rotate the allocation-root cache: the previous current tree is now
        // the retained older one.
        let previous_current = self
            .allocation_tree_cache
            .take()
            .map(|(current, _)| current)
            .unwrap_or_default();
        self.allocation_tree_cache = Some((new_allocation_tree_blocks, previous_current));
        self.alloc_rover_region = finished.rover_region;
        self.other_checkpoint = Some(std::mem::replace(&mut self.checkpoint, new_checkpoint));
        self.current_slot = new_slot;
        // This mount published a checkpoint of its own; whatever the slot it
        // started from, it no longer reads an older one.
        self.fell_back_to_older = false;
        self.last_commit = Some(stats);
        self.window_poisoned = false;
        Ok(())
    }
}

/// An object record decoded from the bytes of block `lba`, which the device
/// kept beside them. A write of the block drops it.
struct KeptRecord {
    record: ObjectRecord,
    generation: u64,
}

/// Decodes the object record in `buf`, the bytes just read from block `lba`.
/// A record decoded here was checked when it was decoded; when the device
/// kept it, it is used as it is, and with the `verify-cached-metadata`
/// feature the bytes are checked again first.
fn decode_record<D: BlockDevice>(
    dev: &mut D,
    lba: u64,
    buf: &[u8],
) -> Result<(ObjectRecord, u64), afsplus_format::FormatError> {
    if let Some(kept) = dev
        .attached(lba)
        .and_then(|value| value.downcast::<KeptRecord>().ok())
    {
        if cfg!(feature = "verify-cached-metadata") {
            afsplus_format::header::BlockHeader::verify(
                buf,
                afsplus_format::header::block_type::OBJECT,
            )?;
        }
        return Ok((kept.record, kept.generation));
    }
    let (record, generation) = ObjectRecord::decode_metadata_with_generation(buf)?;
    dev.attach(lba, std::sync::Arc::new(KeptRecord { record, generation }));
    Ok((record, generation))
}

fn direct_layout(extents: &[Extent], size_bytes: u64, block_size: u64) -> Option<Option<Extent>> {
    if extents.is_empty() {
        return (size_bytes == 0).then_some(None);
    }
    let [extent] = extents else {
        return None;
    };
    if extent.logical_start != 0 || extent.flags != 0 || extent.block_count > MAX_EXTENT_BLOCKS {
        return None;
    }
    let capacity = extent.block_count.checked_mul(block_size)?;
    let minimum = extent.block_count.checked_sub(1)?.checked_mul(block_size)?;
    (size_bytes > minimum && size_bytes <= capacity).then_some(Some(*extent))
}

fn extent_at(extents: &[Extent], logical_block: u64) -> Option<Extent> {
    let after = extents.partition_point(|extent| extent.logical_start <= logical_block);
    let extent = after.checked_sub(1).map(|index| extents[index])?;
    (logical_block < extent.logical_start.saturating_add(extent.block_count)).then_some(extent)
}

fn logical_holes(extents: &[Extent], start: u64, end: u64) -> Result<Vec<(u64, u64)>, CoreError> {
    let mut holes = Vec::new();
    let mut cursor = start;
    for extent in extents {
        let extent_end = extent.logical_end()?;
        if extent_end <= cursor {
            continue;
        }
        if extent.logical_start >= end {
            break;
        }
        if extent.logical_start > cursor {
            holes.push((cursor, extent.logical_start.min(end)));
        }
        cursor = cursor.max(extent_end.min(end));
        if cursor == end {
            break;
        }
    }
    if cursor < end {
        holes.push((cursor, end));
    }
    Ok(holes)
}

fn mark_logical_range_shared(
    extents: &[Extent],
    start: u64,
    end: u64,
) -> Result<Vec<Extent>, CoreError> {
    if start >= end {
        return Ok(extents.to_vec());
    }
    let mut marked = Vec::with_capacity(extents.len() + 2);
    for extent in extents {
        let extent_end = extent.logical_end()?;
        if extent_end <= start || extent.logical_start >= end {
            marked.push(*extent);
            continue;
        }
        let overlap_start = extent.logical_start.max(start);
        let overlap_end = extent_end.min(end);
        if extent.logical_start < overlap_start {
            marked.push(Extent {
                block_count: overlap_start - extent.logical_start,
                ..*extent
            });
        }
        marked.push(Extent {
            logical_start: overlap_start,
            physical_start: extent
                .physical_start
                .checked_add(overlap_start - extent.logical_start)
                .ok_or_else(|| CoreError::Corrupt("extent physical start overflows".into()))?,
            block_count: overlap_end - overlap_start,
            flags: extent.flags | EXTENT_SHARED,
        });
        if overlap_end < extent_end {
            marked.push(Extent {
                logical_start: overlap_end,
                physical_start: extent
                    .physical_start
                    .checked_add(overlap_end - extent.logical_start)
                    .ok_or_else(|| CoreError::Corrupt("extent physical start overflows".into()))?,
                block_count: extent_end - overlap_end,
                flags: extent.flags,
            });
        }
    }
    coalesce_extents(marked)
}

/// Copies the mapped portions of a source logical range into a destination
/// logical range without allocating holes.  All returned mappings carry the
/// conservative shared flag; their physical positions and source flags are
/// otherwise unchanged.
fn remap_extent_range(
    extents: &[Extent],
    source_start: u64,
    source_end: u64,
    destination_start: u64,
) -> Result<Vec<Extent>, CoreError> {
    if source_start >= source_end {
        return Ok(Vec::new());
    }
    let mut remapped = Vec::new();
    for extent in extents {
        let extent_end = extent.logical_end()?;
        if extent_end <= source_start {
            continue;
        }
        if extent.logical_start >= source_end {
            break;
        }
        let overlap_start = extent.logical_start.max(source_start);
        let overlap_end = extent_end.min(source_end);
        remapped.push(Extent {
            logical_start: destination_start
                .checked_add(overlap_start - source_start)
                .ok_or(CoreError::PrototypeLimit(
                    "clone destination block range overflows",
                ))?,
            physical_start: extent
                .physical_start
                .checked_add(overlap_start - extent.logical_start)
                .ok_or_else(|| CoreError::Corrupt("extent physical start overflows".into()))?,
            block_count: overlap_end - overlap_start,
            flags: extent.flags | EXTENT_SHARED,
        });
    }
    coalesce_extents(remapped)
}

fn coalesce_extents(mut extents: Vec<Extent>) -> Result<Vec<Extent>, CoreError> {
    extents.sort_unstable_by_key(|extent| extent.logical_start);
    let mut coalesced: Vec<Extent> = Vec::with_capacity(extents.len());
    for extent in extents {
        if let Some(previous) = coalesced.last_mut() {
            let previous_end = previous.logical_end()?;
            if previous_end > extent.logical_start {
                return Err(CoreError::Corrupt("logical extents overlap".into()));
            }
            if previous.flags == extent.flags
                && previous_end == extent.logical_start
                && previous.physical_end()? == extent.physical_start
            {
                previous.block_count = previous
                    .block_count
                    .checked_add(extent.block_count)
                    .ok_or_else(|| CoreError::Corrupt("coalesced extent overflows".into()))?;
                continue;
            }
        }
        coalesced.push(extent);
    }
    Ok(coalesced)
}

fn allocate_extent_runs<D: BlockDevice>(
    tx: &mut TxAllocator,
    dev: &mut D,
    geo: &afsplus_format::geometry::Geometry,
    logical_start: u64,
    block_count: u64,
    flags: u32,
) -> Result<Vec<Extent>, CoreError> {
    allocate_extent_runs_bounded(tx, dev, geo, logical_start, block_count, flags, usize::MAX)
}

fn allocate_extent_runs_bounded<D: BlockDevice>(
    tx: &mut TxAllocator,
    dev: &mut D,
    geo: &afsplus_format::geometry::Geometry,
    mut logical_start: u64,
    mut block_count: u64,
    flags: u32,
    max_runs: usize,
) -> Result<Vec<Extent>, CoreError> {
    let mut max_run = maximum_allocatable_run(geo)?;
    let mut extents = Vec::new();
    while block_count > 0 {
        if extents.len() == max_runs {
            return Err(CoreError::PrototypeLimit("allocation run budget exhausted"));
        }

        let mut candidate = block_count.min(max_run);
        let physical_start = loop {
            match tx.allocate_run(dev, candidate) {
                Ok(start) => break start,
                Err(CoreError::NoSpace) if candidate > 1 => {
                    candidate = candidate.div_ceil(2);
                    // This call only consumes free space: an unsuccessful
                    // large search cannot improve while allocating its tail.
                    // Keep the fallback local so a later call can try larger
                    // runs again after reclaim or transaction-local releases.
                    max_run = candidate;
                }
                Err(error) => return Err(error),
            }
        };
        extents.push(Extent {
            logical_start,
            physical_start,
            block_count: candidate,
            flags,
        });
        logical_start = logical_start
            .checked_add(candidate)
            .ok_or(CoreError::PrototypeLimit("logical extent range overflows"))?;
        block_count -= candidate;
    }
    coalesce_extents(extents)
}

fn maximum_allocatable_run(geo: &afsplus_format::geometry::Geometry) -> Result<u64, CoreError> {
    let last = geo.region_count() - 1;
    let candidates = [0, 1.min(last), last];
    let mut maximum = 0u64;
    for region in candidates {
        let bootstrap = if region == 0 {
            afsplus_format::geometry::BOOTSTRAP_BLOCKS
        } else {
            0
        };
        let available =
            geo.region_valid_blocks(region) as u64 - geo.region_reserved_blocks(region) - bootstrap;
        maximum = maximum.max(available);
    }
    if maximum == 0 {
        return Err(CoreError::Corrupt(
            "validated geometry has no allocatable run".into(),
        ));
    }
    Ok(maximum)
}

fn replace_logical_range(
    old_extents: &[Extent],
    start: u64,
    end: u64,
    replacement: Option<Extent>,
) -> Result<(Vec<Extent>, Vec<Extent>), CoreError> {
    if start >= end {
        return Err(CoreError::Corrupt("empty logical replacement range".into()));
    }
    let mut extents = Vec::with_capacity(old_extents.len() + usize::from(replacement.is_some()));
    let mut removed = Vec::new();
    for old in old_extents {
        let old_end = old.logical_end()?;
        if old_end <= start || old.logical_start >= end {
            extents.push(*old);
            continue;
        }
        let overlap_start = old.logical_start.max(start);
        let overlap_end = old_end.min(end);
        if old.logical_start < overlap_start {
            extents.push(Extent {
                block_count: overlap_start - old.logical_start,
                ..*old
            });
        }
        removed.push(Extent {
            logical_start: overlap_start,
            physical_start: old.physical_start + overlap_start - old.logical_start,
            block_count: overlap_end - overlap_start,
            flags: old.flags,
        });
        if overlap_end < old_end {
            extents.push(Extent {
                logical_start: overlap_end,
                physical_start: old.physical_start + overlap_end - old.logical_start,
                block_count: old_end - overlap_end,
                flags: old.flags,
            });
        }
    }
    if let Some(replacement) = replacement {
        if replacement.logical_start != start || replacement.logical_end()? != end {
            return Err(CoreError::Corrupt(
                "replacement extent does not cover requested range".into(),
            ));
        }
        extents.push(replacement);
    }
    Ok((coalesce_extents(extents)?, removed))
}

#[cfg(test)]
mod fragmentation_tests {
    use super::*;
    use crate::{mkfs, mount, MkfsParams};
    use afsplus_block::MemoryBackend;

    #[test]
    fn allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation() {
        type Device = afsplus_block::TraceBackend<MemoryBackend>;
        const ORIGINAL: u8 = 0x35;
        const FILE_BYTES: usize = 6000;
        // Long names spread each batch over enough directory leaves that
        // every bounded profile, including eight pages, spills staged nodes.
        const PADDING: &str = "allocation-rotation-directory-leaf-padding";

        struct Fixture {
            source: u64,
            peers: [u64; 2],
            snapshot: u64,
            shared_start: u64,
        }

        fn open(device: Device, pages: usize) -> Volume<Device> {
            let volume = crate::mount_with_snapshot_limits(
                device,
                crate::MountOptions {
                    tree_cache_pages: std::num::NonZeroUsize::new(pages),
                    ..Default::default()
                },
                SnapshotWorkLimits {
                    max_edit_records: 4096,
                    max_views: 128,
                    reclaim_records: 8,
                },
            )
            .unwrap();
            assert_eq!(volume.tree_cache_pages(), pages);
            volume
        }

        /// Reads one file through an explicit checkpoint root and generation,
        /// independent of the live caches and of the selected checkpoint.
        fn checkpoint_file(
            volume: &mut Volume<Device>,
            checkpoint: &Checkpoint,
            id: u64,
        ) -> Vec<u8> {
            let view = afsplus_format::snapshot::SnapshotRecord {
                generation: checkpoint.generation,
                committed_tx_id: checkpoint.committed_tx_id,
                object_map_root: checkpoint.object_map_block,
            };
            let mut observation = crate::snapshot::view::Observation::new(view, 0, None, false);
            let mut bytes = vec![0xa5; FILE_BYTES + 1];
            let count = crate::snapshot::view::read_at(
                &mut volume.dev,
                &volume.ident,
                &mut observation,
                id,
                0,
                &mut bytes,
            )
            .unwrap();
            bytes.truncate(count);
            bytes
        }

        fn snapshot_file(volume: &mut Volume<Device>, snapshot: u64, id: u64) -> Vec<u8> {
            let handle = volume.snapshot_open(snapshot).unwrap();
            let mut bytes = vec![0xa5; FILE_BYTES + 1];
            let count = volume
                .snapshot_read_file_at(&handle, id, 0, &mut bytes)
                .unwrap();
            bytes.truncate(count);
            bytes
        }

        /// Both selectable checkpoints: cached allocation-root block sets, the
        /// verifier sweep, the exact shared run and each file's exact bytes.
        fn verify(volume: &mut Volume<Device>, fixture: &Fixture, current: &[u8], context: &str) {
            let original = vec![ORIGINAL; FILE_BYTES];
            // Three owners share the two-block run until the first source write
            // copies both blocks; the two peers keep one run with two references.
            let expected_shared = vec![crate::shared_extents::SharedRun {
                physical_start: fixture.shared_start,
                block_count: 2,
                reference_count: 2,
                flags: 0,
            }];
            let selected = volume.checkpoint.clone();
            let older = volume.other_checkpoint.clone().unwrap();
            assert_eq!(older.generation + 1, selected.generation, "{context}");
            let cache = volume.allocation_tree_cache.clone();
            for (index, checkpoint) in [selected, older].into_iter().enumerate() {
                let generation = checkpoint.generation;
                if let Some((cached, older_cached)) = &cache {
                    let mut expected = if index == 0 {
                        cached.clone()
                    } else {
                        older_cached.clone()
                    };
                    let mut actual = allocation_root::load_tree_blocks(
                        &mut volume.dev,
                        &volume.ident.geometry(),
                        checkpoint.allocation_root_block,
                        generation,
                    )
                    .unwrap();
                    actual.sort_unstable();
                    expected.sort_unstable();
                    assert_eq!(
                        expected, actual,
                        "{context}: checkpoint {generation} cache loses spilled nodes"
                    );
                }
                let state = crate::verify::load_committed_state(
                    &mut volume.dev,
                    &volume.ident,
                    &checkpoint,
                )
                .unwrap();
                let findings =
                    crate::verify::full_sweep(&state, &volume.ident.geometry(), &checkpoint);
                assert!(
                    findings.is_empty(),
                    "{context}: checkpoint {generation}: {findings:?}"
                );
                assert_eq!(
                    state.shared_records, expected_shared,
                    "{context}: checkpoint {generation} shared run"
                );
                assert_eq!(
                    checkpoint_file(volume, &checkpoint, fixture.source),
                    current,
                    "{context}: checkpoint {generation} source"
                );
                for peer in fixture.peers {
                    assert_eq!(
                        checkpoint_file(volume, &checkpoint, peer),
                        original,
                        "{context}: checkpoint {generation} peer {peer}"
                    );
                }
            }
            assert_eq!(
                volume.read_file(fixture.source).unwrap(),
                current,
                "{context}"
            );
            for id in [fixture.source, fixture.peers[0], fixture.peers[1]] {
                if id != fixture.source {
                    assert_eq!(
                        volume.read_file(id).unwrap(),
                        original,
                        "{context}: live {id}"
                    );
                }
                let captured = snapshot_file(volume, fixture.snapshot, id);
                assert_eq!(captured, original, "{context}: captured object {id}");
            }
        }

        for pages in [2, 4, 8, usize::MAX] {
            let mut dev = MemoryBackend::new(4096, 16 * 1024);
            crate::mkfs_with_options(
                &mut dev,
                &MkfsParams {
                    uuid: [0xcc; 16],
                    label: "CacheRotation".into(),
                    region_size: 16,
                    reclaim_caps: Default::default(),
                    log_slots: 0,
                    shared_extents: true,
                    data_policy: false,
                    name_policy: crate::NamePolicy::Sensitive,
                    timestamp: Timespec::default(),
                },
                crate::MkfsOptions {
                    persistent_snapshots: true,
                },
            )
            .unwrap();
            let mut volume = open(afsplus_block::TraceBackend::new(dev), pages);
            let original = vec![ORIGINAL; FILE_BYTES];
            let source = volume
                .create_file_in_root("source", &original, Timespec::default())
                .unwrap();
            let peers = [
                volume
                    .clone_file(source, OBJECT_ROOT, "peer-a", Timespec::default())
                    .unwrap(),
                volume
                    .clone_file(source, OBJECT_ROOT, "peer-b", Timespec::default())
                    .unwrap(),
            ];
            let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
            let selected = volume.checkpoint.clone();
            let shared =
                crate::verify::load_committed_state(&mut volume.dev, &volume.ident, &selected)
                    .unwrap()
                    .shared_records;
            assert_eq!(shared.len(), 1);
            assert_eq!(
                (
                    shared[0].block_count,
                    shared[0].reference_count,
                    shared[0].flags
                ),
                (2, 3, 0),
                "pages={pages}: three owners before rotation"
            );
            let fixture = Fixture {
                source,
                peers,
                snapshot,
                shared_start: shared[0].physical_start,
            };
            let mut current = original.clone();
            let mut spills = 0;
            for round in 0..3u32 {
                let context = format!("pages={pages} round={round}");
                // Cross the first block boundary so the source copies both
                // shared blocks while peers and the snapshot keep old bytes.
                let patch = [0x70 + round as u8; 32];
                volume
                    .write_file_at(source, 4090, &patch, Timespec::default())
                    .unwrap();
                current[4090..4122].copy_from_slice(&patch);
                // Keep the original regression's 1,024 regions and forced
                // rover movement across allocation-root leaf boundaries.
                volume.alloc_rover_region = 120 + round * 120;
                let names: Vec<_> = (0..256)
                    .map(|i| format!("r{round}-{i:04}-{PADDING}"))
                    .collect();
                let operations: Vec<_> = names
                    .iter()
                    .map(|name| BatchOp::CreateFile {
                        parent_id: OBJECT_ROOT,
                        name,
                        content: b"",
                    })
                    .collect();
                let previous = volume.generation();
                volume.dev.reset();
                volume.run_batch(&operations, Timespec::default()).unwrap();
                let stats = volume.last_commit_stats().unwrap();
                assert!(
                    stats.allocation_tree_nodes_written > 2,
                    "{context}: fixture must span allocation-root nodes"
                );
                assert!(stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
                if pages == usize::MAX {
                    assert_eq!(stats.tree_mutations.staged_spill_writes, 0, "{context}");
                } else {
                    assert!(
                        stats.tree_mutations.staged_spill_writes > 0,
                        "{context}: bounded profile must spill"
                    );
                }
                spills += stats.tree_mutations.staged_spill_writes;
                assert_eq!(stats.bytes_written, volume.dev.stats().bytes_written);
                assert_eq!(
                    volume.other_checkpoint.as_ref().unwrap().generation,
                    previous,
                    "{context}: batch must rotate the write checkpoint into the older slot"
                );
                assert!(volume.allocation_tree_cache.is_some(), "{context}");
                verify(&mut volume, &fixture, &current, &context);
                let expected_entries = (round as usize + 1) * 256 + 3;
                assert_eq!(volume.list_root().unwrap().len(), expected_entries);

                volume = open(volume.into_device(), pages);
                verify(
                    &mut volume,
                    &fixture,
                    &current,
                    &format!("{context} remounted"),
                );
                for completed in 0..=round {
                    for i in 0..256 {
                        let id = volume
                            .lookup_root(&format!("r{completed}-{i:04}-{PADDING}"))
                            .unwrap()
                            .unwrap();
                        assert!(volume.read_file(id).unwrap().is_empty());
                    }
                }
                assert_eq!(volume.list_root().unwrap().len(), expected_entries);
            }
            eprintln!("allocation rotation: pages={pages}, rounds=3, batch spills={spills}");
        }
    }

    // -----------------------------------------------------------------
    // Modeled cuts of the multi-round allocation rotation batch
    //
    // `allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation`
    // above qualifies the rotation itself. This block records the third
    // round's 256-entry batch on a recording device and replays the seeded
    // cut campaign of the family-matrix driver over it: every in-order write
    // prefix, each write torn at the representative offsets after its
    // in-order prefix, every full-write subset of a flush segment of at most
    // twelve writes and `ROTATION_SAMPLE` seeded subsets of each longer
    // segment. The rover control this fixture needs lives in this crate, so
    // the oracle uses the verifier entry points directly.
    // -----------------------------------------------------------------

    /// Seeded full-write subsets drawn per oversized flush segment.
    const ROTATION_SAMPLE: usize = 32;
    /// SplitMix64 seed of the rotation cut campaign.
    const ROTATION_SEED: u64 = 0x5eed_0a11_0cae;
    /// Representative tear offsets of the power-cut model.
    const ROTATION_TEARS: [usize; 3] = [64, 2048, 4064];
    /// Flush segments up to this many writes enumerate every full-write subset.
    const ROTATION_EXHAUSTIVE_SEGMENT: usize = 12;

    const ROTATION_ORIGINAL: u8 = 0x35;
    const ROTATION_FILE_BYTES: usize = 6000;
    const ROTATION_PADDING: &str = "allocation-rotation-directory-leaf-padding";
    /// Entries each round behind the recorded one adds to the root directory.
    const ROTATION_POPULATION: usize = 64;
    /// Entries the recorded batch adds. This bounded width keeps the campaign
    /// inside the per-test time budget while the batch writes three
    /// allocation-root nodes and spills at every bounded profile.
    const ROTATION_BATCH: usize = 96;
    /// Rounds the fixture completes before the recorded batch.
    const ROTATION_ROUNDS: u32 = 1;

    /// Spill writes the recorded batch performs per profile. Eight pages hold
    /// its staged window, so two and four pages are the profiles that evict.
    fn rotation_spills(pages: usize) -> u64 {
        match pages {
            2 => 12,
            4 => 4,
            _ => 0,
        }
    }

    struct RotationFixture {
        source: u64,
        peers: [u64; 2],
        snapshot: u64,
        shared_start: u64,
    }

    fn rotation_open<D: BlockDevice>(device: D, pages: usize) -> Volume<D> {
        let volume = crate::mount_with_snapshot_limits(
            device,
            crate::MountOptions {
                tree_cache_pages: std::num::NonZeroUsize::new(pages),
                ..Default::default()
            },
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 128,
                reclaim_records: 8,
            },
        )
        .unwrap();
        assert_eq!(volume.tree_cache_pages(), pages);
        volume
    }

    /// Reads one file through an explicit checkpoint root and generation,
    /// independent of the live caches and of the selected checkpoint.
    fn rotation_checkpoint_file<D: BlockDevice>(
        volume: &mut Volume<D>,
        checkpoint: &Checkpoint,
        id: u64,
    ) -> Vec<u8> {
        let view = afsplus_format::snapshot::SnapshotRecord {
            generation: checkpoint.generation,
            committed_tx_id: checkpoint.committed_tx_id,
            object_map_root: checkpoint.object_map_block,
        };
        let mut observation = crate::snapshot::view::Observation::new(view, 0, None, false);
        let mut bytes = vec![0xa5; ROTATION_FILE_BYTES + 1];
        let count = crate::snapshot::view::read_at(
            &mut volume.dev,
            &volume.ident,
            &mut observation,
            id,
            0,
            &mut bytes,
        )
        .unwrap();
        bytes.truncate(count);
        bytes
    }

    fn rotation_snapshot_file<D: BlockDevice>(
        volume: &mut Volume<D>,
        snapshot: u64,
        id: u64,
    ) -> Vec<u8> {
        let handle = volume.snapshot_open(snapshot).unwrap();
        let mut bytes = vec![0xa5; ROTATION_FILE_BYTES + 1];
        let count = volume
            .snapshot_read_file_at(&handle, id, 0, &mut bytes)
            .unwrap();
        bytes.truncate(count);
        bytes
    }

    /// Both selectable checkpoints: the cached allocation-root block sets when
    /// a cache exists, the verifier's committed-state load and full sweep, the
    /// exact shared run, and the literal bytes of the three objects through
    /// each checkpoint's own object-map root, live and through the snapshot.
    fn rotation_verify<D: BlockDevice>(
        volume: &mut Volume<D>,
        fixture: &RotationFixture,
        current: &[u8],
        entries: usize,
        context: &str,
    ) {
        let original = vec![ROTATION_ORIGINAL; ROTATION_FILE_BYTES];
        let expected_shared = vec![crate::shared_extents::SharedRun {
            physical_start: fixture.shared_start,
            block_count: 2,
            reference_count: 2,
            flags: 0,
        }];
        let selected = volume.checkpoint.clone();
        // A cut inside the write to the older slot leaves one selectable
        // checkpoint, which the oracle checks alone.
        let older = volume.other_checkpoint.clone();
        if let Some(older) = &older {
            assert_eq!(older.generation + 1, selected.generation, "{context}");
        }
        let cache = volume.allocation_tree_cache.clone();
        for (index, checkpoint) in [Some(selected), older].into_iter().flatten().enumerate() {
            let generation = checkpoint.generation;
            if let Some((cached, older_cached)) = &cache {
                let mut expected = if index == 0 {
                    cached.clone()
                } else {
                    older_cached.clone()
                };
                let mut actual = allocation_root::load_tree_blocks(
                    &mut volume.dev,
                    &volume.ident.geometry(),
                    checkpoint.allocation_root_block,
                    generation,
                )
                .unwrap();
                actual.sort_unstable();
                expected.sort_unstable();
                assert_eq!(
                    expected, actual,
                    "{context}: checkpoint {generation} cache loses spilled nodes"
                );
            }
            let state =
                crate::verify::load_committed_state(&mut volume.dev, &volume.ident, &checkpoint)
                    .unwrap();
            let findings = crate::verify::full_sweep(&state, &volume.ident.geometry(), &checkpoint);
            assert!(
                findings.is_empty(),
                "{context}: checkpoint {generation}: {findings:?}"
            );
            assert_eq!(
                state.shared_records, expected_shared,
                "{context}: checkpoint {generation} shared run"
            );
            assert_eq!(
                rotation_checkpoint_file(volume, &checkpoint, fixture.source),
                current,
                "{context}: checkpoint {generation} source"
            );
            for peer in fixture.peers {
                assert_eq!(
                    rotation_checkpoint_file(volume, &checkpoint, peer),
                    original,
                    "{context}: checkpoint {generation} peer {peer}"
                );
            }
        }
        assert_eq!(
            volume.read_file(fixture.source).unwrap(),
            current,
            "{context}"
        );
        for peer in fixture.peers {
            assert_eq!(
                volume.read_file(peer).unwrap(),
                original,
                "{context}: live {peer}"
            );
        }
        for id in [fixture.source, fixture.peers[0], fixture.peers[1]] {
            assert_eq!(
                rotation_snapshot_file(volume, fixture.snapshot, id),
                original,
                "{context}: captured object {id}"
            );
        }
        assert_eq!(
            volume.list_root().unwrap().len(),
            entries,
            "{context}: root entries"
        );
    }

    fn rotation_names(round: u32, count: usize) -> Vec<String> {
        (0..count)
            .map(|i| format!("r{round}-{i:04}-{ROTATION_PADDING}"))
            .collect()
    }

    fn rotation_batch<D: BlockDevice>(volume: &mut Volume<D>, names: &[String]) {
        let operations: Vec<_> = names
            .iter()
            .map(|name| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content: b"",
            })
            .collect();
        volume.run_batch(&operations, Timespec::default()).unwrap();
    }

    fn rotation_splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = *state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn rotation_segment_subsets(
        durable: &MemoryBackend,
        segment: &[(u64, &Vec<u8>)],
        rng: &mut u64,
        visit: &mut dyn FnMut(MemoryBackend, String),
    ) -> (u64, u64) {
        if segment.is_empty() {
            return (0, 0);
        }
        let exhaustive = segment.len() <= ROTATION_EXHAUSTIVE_SEGMENT;
        let masks: Vec<Vec<bool>> = if exhaustive {
            (0u64..1 << segment.len())
                .map(|mask| {
                    (0..segment.len())
                        .map(|bit| mask & (1 << bit) != 0)
                        .collect()
                })
                .collect()
        } else {
            (0..ROTATION_SAMPLE)
                .map(|_| {
                    (0..segment.len())
                        .map(|_| rotation_splitmix64(rng) & 1 == 1)
                        .collect()
                })
                .collect()
        };
        for (number, mask) in masks.iter().enumerate() {
            let mut image = durable.clone();
            for ((lba, data), keep) in segment.iter().zip(mask) {
                if *keep {
                    image.apply_raw(*lba, data);
                }
            }
            let kind = if exhaustive {
                "subset"
            } else {
                "sampled subset"
            };
            visit(
                image,
                format!("{kind} {number} of a {}-write segment", segment.len()),
            );
        }
        let count = masks.len() as u64;
        if exhaustive {
            (count, 0)
        } else {
            (0, count)
        }
    }

    /// Builds the rotation fixture, records the batch that rotates the write
    /// checkpoint into the older slot, and runs the seeded cut campaign.
    fn rotation_cut_campaign(pages: usize) {
        let mut dev = MemoryBackend::new(4096, 16 * 1024);
        crate::mkfs_with_options(
            &mut dev,
            &MkfsParams {
                uuid: [0xcc; 16],
                label: "CacheRotation".into(),
                region_size: 16,
                reclaim_caps: Default::default(),
                log_slots: 0,
                shared_extents: true,
                data_policy: false,
                name_policy: crate::NamePolicy::Sensitive,
                timestamp: Timespec::default(),
            },
            crate::MkfsOptions {
                persistent_snapshots: true,
            },
        )
        .unwrap();
        let mut volume = rotation_open(dev, pages);
        let original = vec![ROTATION_ORIGINAL; ROTATION_FILE_BYTES];
        let source = volume
            .create_file_in_root("source", &original, Timespec::default())
            .unwrap();
        let peers = [
            volume
                .clone_file(source, OBJECT_ROOT, "peer-a", Timespec::default())
                .unwrap(),
            volume
                .clone_file(source, OBJECT_ROOT, "peer-b", Timespec::default())
                .unwrap(),
        ];
        let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
        let selected = volume.checkpoint.clone();
        let shared = crate::verify::load_committed_state(&mut volume.dev, &volume.ident, &selected)
            .unwrap()
            .shared_records;
        assert_eq!(shared.len(), 1);
        assert_eq!(
            (
                shared[0].block_count,
                shared[0].reference_count,
                shared[0].flags
            ),
            (2, 3, 0),
            "pages={pages}: three owners before rotation"
        );
        let fixture = RotationFixture {
            source,
            peers,
            snapshot,
            shared_start: shared[0].physical_start,
        };
        let mut current = original.clone();
        // Rounds behind the recorded one leave two populated allocation-root
        // leaf ranges and the rotated checkpoint pair the campaign starts from.
        for round in 0..ROTATION_ROUNDS {
            let patch = [0x70 + round as u8; 32];
            volume
                .write_file_at(source, 4090, &patch, Timespec::default())
                .unwrap();
            current[4090..4122].copy_from_slice(&patch);
            volume.alloc_rover_region = 120 + round * 120;
            rotation_batch(&mut volume, &rotation_names(round, ROTATION_POPULATION));
        }
        let patch = [0x70 + ROTATION_ROUNDS as u8; 32];
        volume
            .write_file_at(source, 4090, &patch, Timespec::default())
            .unwrap();
        current[4090..4122].copy_from_slice(&patch);
        // One filler commit behind the data write puts the patched bytes in
        // both selectable checkpoints, so the recorded window holds exactly
        // the rotating batch commit.
        volume
            .create_file_in_root("rotation-filler", b"", Timespec::default())
            .unwrap();
        volume.alloc_rover_region = 120 + ROTATION_ROUNDS * 120;
        let rover = volume.alloc_rover_region;
        let before = ROTATION_ROUNDS as usize * ROTATION_POPULATION + 4;
        let after = before + ROTATION_BATCH;
        let base = volume.into_device();
        let fixture_generation = {
            let mut volume = rotation_open(base.clone(), pages);
            rotation_verify(&mut volume, &fixture, &current, before, "fixture");
            volume.generation()
        };

        let names = rotation_names(ROTATION_ROUNDS, ROTATION_BATCH);
        let mut volume = rotation_open(afsplus_block::RecordingBackend::new(base.clone()), pages);
        volume.alloc_rover_region = rover;
        rotation_batch(&mut volume, &names);
        let stats = volume.last_commit_stats().unwrap();
        assert!(
            stats.allocation_tree_nodes_written > 2,
            "pages={pages}: fixture must span allocation-root nodes"
        );
        assert!(stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
        let spills = stats.tree_mutations.staged_spill_writes;
        assert_eq!(
            spills,
            rotation_spills(pages),
            "pages={pages}: spill writes of the recorded batch"
        );
        assert_eq!(
            volume.other_checkpoint.as_ref().unwrap().generation,
            fixture_generation,
            "pages={pages}: the batch must rotate the write checkpoint into the older slot"
        );
        assert!(volume.allocation_tree_cache.is_some());
        rotation_verify(&mut volume, &fixture, &current, after, "recorded");
        let (_, log) = volume.into_device().into_parts();

        let mut outcomes = [0u64; 2];
        let mut check = |image: MemoryBackend, description: String| {
            let context = format!("pages={pages}: {description}");
            let mut volume = rotation_open(image, pages);
            let generation = volume.generation();
            let delta = generation
                .checked_sub(fixture_generation)
                .filter(|delta| *delta <= 1)
                .unwrap_or_else(|| panic!("{context}: disallowed generation {generation}"));
            let entries = if delta == 0 { before } else { after };
            rotation_verify(&mut volume, &fixture, &current, entries, &context);
            outcomes[delta as usize] += 1;
        };

        let mut rng = ROTATION_SEED;
        let (mut prefixes, mut tears, mut exhaustive, mut sampled) = (1u64, 0u64, 0u64, 0u64);
        let mut prefix = base.clone();
        let mut durable = base.clone();
        let mut segment: Vec<(u64, &Vec<u8>)> = Vec::new();
        check(prefix.clone(), "in-order prefix of 0 operations".into());
        for (index, operation) in log.iter().enumerate() {
            match operation {
                afsplus_block::RecordedOp::Write { lba, data } => {
                    for tear in ROTATION_TEARS.iter().filter(|tear| **tear < data.len()) {
                        let mut image = prefix.clone();
                        let mut torn = image.peek(*lba);
                        torn[..*tear].copy_from_slice(&data[..*tear]);
                        image.apply_raw(*lba, &torn);
                        check(
                            image,
                            format!("write {index} (lba {lba}) torn at byte {tear}"),
                        );
                        tears += 1;
                    }
                    prefix.apply_raw(*lba, data);
                    segment.push((*lba, data));
                    check(
                        prefix.clone(),
                        format!("in-order prefix through operation {index}"),
                    );
                    prefixes += 1;
                }
                afsplus_block::RecordedOp::Flush => {
                    let (full, drawn) =
                        rotation_segment_subsets(&durable, &segment, &mut rng, &mut check);
                    exhaustive += full;
                    sampled += drawn;
                    durable = prefix.clone();
                    segment.clear();
                }
            }
        }
        let (full, drawn) = rotation_segment_subsets(&durable, &segment, &mut rng, &mut check);
        exhaustive += full;
        sampled += drawn;
        assert!(
            outcomes[0] > 0 && outcomes[1] > 0,
            "pages={pages}: both outcomes required {outcomes:?}"
        );

        // The fixture generation admits the batch again, and the retry
        // restores the cached allocation-root block sets of both checkpoints.
        let mut retried = rotation_open(base, pages);
        retried.alloc_rover_region = rover;
        rotation_batch(&mut retried, &names);
        assert!(retried.allocation_tree_cache.is_some());
        rotation_verify(&mut retried, &fixture, &current, after, "retry");

        eprintln!(
            "allocation rotation cuts: pages={pages}, spills={spills}, seed={ROTATION_SEED:#x}, sample={ROTATION_SAMPLE}, prefixes={prefixes}, tears={tears}, exhaustive_subsets={exhaustive}, sampled_subsets={sampled}, outcomes={outcomes:?}, images={}",
            outcomes.iter().sum::<u64>()
        );
    }

    #[test]
    fn allocation_rotation_batch_cuts_are_atomic_at_two_pages() {
        rotation_cut_campaign(2);
    }

    #[test]
    fn allocation_rotation_batch_cuts_are_atomic_at_four_pages() {
        rotation_cut_campaign(4);
    }

    #[test]
    fn allocation_rotation_batch_cuts_are_atomic_at_eight_pages() {
        rotation_cut_campaign(8);
    }

    #[test]
    fn allocation_rotation_batch_cuts_are_atomic_at_unlimited_pages() {
        rotation_cut_campaign(usize::MAX);
    }

    #[test]
    fn fragmented_allocation_does_not_repeat_oversized_searches() {
        let mut dev = MemoryBackend::new(4096, 512);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [0xBF; 16],
                label: "Fragmented".into(),
                region_size: 512,
                reclaim_caps: Default::default(),
                log_slots: 0,
                shared_extents: false,
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
        let mut tx = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 0, 0).unwrap();
        // All ordinary free space is one contiguous suffix on this new image.
        // Occupy alternate blocks to leave only isolated one-block holes.
        let first = tx.allocate(&mut dev).unwrap();
        for lba in (first + 2..geo.total_blocks).step_by(2) {
            tx.allocate_exact_run(&mut dev, lba, 1).unwrap();
        }
        let before = tx.stats();
        let extents = allocate_extent_runs(&mut tx, &mut dev, &geo, 0, 64, 0).unwrap();
        let after = tx.stats();
        let searches = after.allocation_searches - before.allocation_searches;
        let bits = after.bitmap_bits_examined - before.bitmap_bits_examined;
        eprintln!("fragmented 64-block allocation: searches={searches}, bitmap_bits={bits}");
        assert_eq!(extents.len(), 64);
        for (i, extent) in extents.iter().enumerate() {
            assert_eq!(extent.logical_start, i as u64);
            assert_eq!(extent.block_count, 1);
            assert_eq!(extent.physical_start, first + 1 + 2 * i as u64);
        }
        assert!(
            searches <= 70,
            "repeated failed large-run searches: {searches}"
        );
        // A new request must retry larger runs after transaction-local frees.
        for lba in (first + 2..first + 18).step_by(2) {
            tx.release_uncommitted(&mut dev, lba).unwrap();
        }
        // The earlier extent allocations still occupy the intervening blocks;
        // release those too, then verify no sticky global fragmentation hint.
        for lba in (first + 1..first + 18).step_by(2) {
            tx.release_uncommitted(&mut dev, lba).unwrap();
        }
        let contiguous = allocate_extent_runs(&mut tx, &mut dev, &geo, 0, 16, 0).unwrap();
        assert_eq!(contiguous.len(), 1);
        assert_eq!(contiguous[0].block_count, 16);
    }
}
