//! Optional bounded diagnostics for core API calls, object maps and checkpoint publication.
//!
//! These are runtime observations, not on-disk records or a durability oracle.
//! Ring construction allocates once; ring emission neither allocates nor reads
//! a clock. An optional trusted live adapter must obey the same bounded contract.

use std::collections::{TryReserveError, VecDeque};
use std::num::NonZeroUsize;

pub(crate) type SharedRecorder = std::sync::Arc<RecorderCell>;

/// A transferable recorder with nonblocking runtime borrows.
/// Volume operations already require exclusive access. Contention therefore
/// means a caller retained a diagnostic guard or a sink reentered diagnostics.
/// Never wait for such a borrow inside a filesystem operation.
#[derive(Debug)]
pub(crate) struct RecorderCell(std::sync::RwLock<FlightRecorder>);
impl RecorderCell {
    pub(crate) fn new(recorder: FlightRecorder) -> Self {
        Self(std::sync::RwLock::new(recorder))
    }
    pub(crate) fn borrow(&self) -> std::sync::RwLockReadGuard<'_, FlightRecorder> {
        match self.0.try_read() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => panic!("flight recorder already borrowed"),
        }
    }
    pub(crate) fn borrow_mut(&self) -> std::sync::RwLockWriteGuard<'_, FlightRecorder> {
        match self.0.try_write() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => panic!("flight recorder already borrowed"),
        }
    }
    pub(crate) fn into_inner(self) -> FlightRecorder {
        self.0
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Categories emitted by API observation and the commit tail. Other coverage has
/// separate integration gates; a category name alone is not that evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Transaction,
    Checkpoint,
    Io,
    Error,
    Api,
    Window,
    Object,
    Allocator,
    Tree,
    Reclaim,
    /// Observed mount: selection, intent-log inspection/replay and outcome.
    Mount,
    /// Observed formatter barriers and publication.
    Format,
    /// Observed standalone verification phases, findings and outcome.
    Verify,
    /// Observed file data staged before the common commit tail.
    Data,
    /// Observed read-only tree descents over a live or captured view.
    View,
}

/// Runtime selection, independent of ring capacity and event identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Categories(u16);

impl Categories {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(32767);

    pub const fn with(self, category: Category) -> Self {
        Self(self.0 | (1 << category as u8))
    }

    pub const fn contains(self, category: Category) -> bool {
        self.0 & (1 << category as u8) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Begin,
    /// The tail's queued data writes and their conditional barrier completed.
    /// Earlier writes and intent-log durability have separate owners.
    DataWritesComplete,
    MetadataDurable,
    PublicationBegin,
    CheckpointDurable,
    Adopted,
    Failed,
    ApiBegin,
    ApiSucceeded,
    ApiFailed,
    ApiUnwound,
    WindowOpened,
    WindowAttached,
    WindowLogBegin,
    WindowLogDurable,
    WindowLogFailed,
    WindowFailed,
    WindowClosed,
    WindowDetached,
    ObjectLookup,
    ObjectMapped,
    ObjectMissing,
    AllocationBegin,
    AllocationGranted,
    AllocationRetired,
    AllocationFailed,
    TreeReadBegin,
    TreeReadComplete,
    TreeSpillBegin,
    TreeSpillComplete,
    TreeIoFailed,
    ReclaimBegin,
    ReclaimPromoted,
    ReclaimBlocked,
    ReclaimAppended,
    ReclaimPlanned,
    ReclaimBuilt,
    ReclaimFailed,
    /// Observed mount entry, before identification I/O.
    MountBegin,
    /// Structural checkpoint selection chose the event generation.
    MountSelected,
    /// Intent-log replay (writable modes) or inspection (read-only modes) begins.
    MountIntentBegin,
    /// The valid intent-log prefix was scanned; count is its record total.
    MountIntentScanned,
    /// One intent group's operations were applied to the recovery batch.
    MountIntentReplayed,
    /// The mounted volume is returned at the event generation.
    MountComplete,
    /// Mount refused or failed at the context stage; no volume is returned.
    MountFailed,
    /// Observed formatter entry, before validation and device I/O.
    FormatBegin,
    /// Metadata, identification and slot-B zeroing completed their barrier.
    FormatMetadataDurable,
    /// The slot-A checkpoint write begins.
    FormatPublicationBegin,
    /// The slot-A checkpoint barrier completed.
    FormatCheckpointDurable,
    /// Formatting failed at the context stage.
    FormatFailed,
    /// Observed verification of one checkpoint begins.
    VerifyBegin,
    /// A verification phase begins at its root block, when it has one.
    VerifyPhase,
    /// One full-sweep finding at its ordinal in the returned findings.
    VerifyFinding,
    /// Verification returned its result; full sweeps report their finding count.
    VerifyComplete,
    /// Verification returned an error at the context phase and location.
    VerifyFailed,
    /// One object's file data is about to be written before the commit tail.
    DataWriteBegin,
    /// Every block of that logical range reached the device.
    DataWriteComplete,
    /// A write in that logical range failed; the range is partly written.
    DataWriteFailed,
    /// The barrier covering existing-file data written before a log record.
    IntentDataDurable,
    /// The barrier of an fsync whose group carries no record.
    IntentEmptyFlush,
    /// A read-only tree descent over a live or captured view begins.
    ViewReadBegin,
    /// That descent returned its result.
    ViewReadComplete,
    /// That descent failed; the view is unchanged.
    ViewReadFailed,
    /// Snapshot creation, deletion or ledger maintenance failed, naming the
    /// view it was applied to where the operation names one.
    ViewMaintenanceFailed,
}

impl EventKind {
    pub const fn category(self) -> Category {
        match self {
            Self::DataWriteBegin | Self::DataWriteComplete | Self::DataWriteFailed => {
                Category::Data
            }
            Self::ViewReadBegin
            | Self::ViewReadComplete
            | Self::ViewReadFailed
            | Self::ViewMaintenanceFailed => Category::View,
            Self::MountBegin
            | Self::MountSelected
            | Self::MountIntentBegin
            | Self::MountIntentScanned
            | Self::MountIntentReplayed
            | Self::MountComplete
            | Self::MountFailed => Category::Mount,
            Self::FormatBegin
            | Self::FormatMetadataDurable
            | Self::FormatPublicationBegin
            | Self::FormatCheckpointDurable
            | Self::FormatFailed => Category::Format,
            Self::VerifyBegin
            | Self::VerifyPhase
            | Self::VerifyFinding
            | Self::VerifyComplete
            | Self::VerifyFailed => Category::Verify,
            Self::ReclaimBegin
            | Self::ReclaimPromoted
            | Self::ReclaimBlocked
            | Self::ReclaimAppended
            | Self::ReclaimPlanned
            | Self::ReclaimBuilt
            | Self::ReclaimFailed => Category::Reclaim,
            Self::TreeReadBegin
            | Self::TreeReadComplete
            | Self::TreeSpillBegin
            | Self::TreeSpillComplete
            | Self::TreeIoFailed => Category::Tree,
            Self::AllocationBegin
            | Self::AllocationGranted
            | Self::AllocationRetired
            | Self::AllocationFailed => Category::Allocator,
            Self::ObjectLookup | Self::ObjectMapped | Self::ObjectMissing => Category::Object,
            Self::Begin | Self::Adopted => Category::Transaction,
            Self::DataWritesComplete | Self::IntentDataDurable | Self::IntentEmptyFlush => {
                Category::Io
            }
            Self::MetadataDurable | Self::PublicationBegin | Self::CheckpointDurable => {
                Category::Checkpoint
            }
            Self::Failed => Category::Error,
            Self::ApiBegin | Self::ApiSucceeded | Self::ApiFailed | Self::ApiUnwound => {
                Category::Api
            }
            Self::WindowOpened
            | Self::WindowAttached
            | Self::WindowLogBegin
            | Self::WindowLogDurable
            | Self::WindowLogFailed
            | Self::WindowFailed
            | Self::WindowClosed
            | Self::WindowDetached => Category::Window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emission_keeps_allocation_and_stops_before_identity_reuse() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        let allocated = ring.events.capacity();
        for _ in 0..1000 {
            ring.emit(9, EventKind::Begin, false);
            ring.emit(9, EventKind::Failed, false);
        }
        assert_eq!(ring.events.capacity(), allocated);
        assert_eq!(ring.events.len(), 1);
        assert_eq!(ring.dropped(), 1999);
        ring.attempt = u64::MAX;
        let last = *ring.events.back().unwrap();
        ring.emit(9, EventKind::Begin, false);
        ring.emit(9, EventKind::Adopted, false);
        assert_eq!(*ring.events.back().unwrap(), last);
        assert_eq!(ring.dropped(), 2001);

        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.sequence = u64::MAX - 1;
        ring.emit(1, EventKind::Begin, false);
        ring.emit(1, EventKind::Adopted, false);
        assert_eq!(ring.events.back().unwrap().sequence, u64::MAX);
        assert_eq!(ring.events.back().unwrap().kind, EventKind::Begin);
        assert_eq!(ring.dropped(), 1);
    }
}

/// Core API method identities. Append new IDs; never renumber or reuse them.
/// These are diagnostic identifiers, not filesystem API v2 ABI ordinals.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiMethod {
    CleanupOrphan = 1,
    CloneFile = 2,
    CloneRange = 3,
    CreateDirectory = 4,
    CreateDirectoryInRoot = 5,
    CreateFileInDirectory = 6,
    CreateFileInRoot = 7,
    CreateSymlink = 8,
    DeleteFile = 9,
    DeleteFileInRoot = 10,
    FileAllocationPage = 11,
    FileDataPolicy = 12,
    FirstOrphan = 13,
    LinkFile = 14,
    ListDirectory = 15,
    ListRoot = 16,
    LookupInDirectory = 17,
    LookupRoot = 18,
    OrphanCount = 19,
    OrphanFile = 20,
    OrphanObject = 21,
    PreallocateFile = 22,
    PreallocateFileBounded = 23,
    QuarantineContains = 24,
    ReadDirectoryPage = 25,
    ReadFile = 26,
    ReadFileAt = 27,
    ReadLink = 28,
    ReclaimStep = 29,
    RemoveDirectory = 30,
    Rename = 31,
    RenameReplace = 32,
    RenameReplaceOrphanTarget = 33,
    RestoreObjectMetadata = 34,
    RunBatch = 35,
    SetDataUpdatePolicy = 36,
    SetFileDataPolicy = 37,
    SetObjectProtection = 38,
    SetOrphanCleanupExtentBudget = 39,
    SetReclaimBatchBlocks = 40,
    SetSnapshotWorkLimits = 41,
    SetTreeCachePages = 42,
    SnapshotAllocationPage = 43,
    SnapshotCreate = 44,
    SnapshotDelete = 45,
    SnapshotList = 46,
    SnapshotLookup = 47,
    SnapshotMaintenanceStep = 48,
    SnapshotOpen = 49,
    SnapshotReadDirectoryPage = 50,
    SnapshotReadFileAt = 51,
    SnapshotReadLink = 52,
    SnapshotStat = 53,
    Stat = 54,
    Sync = 55,
    TruncateFile = 56,
    TruncateFileBounded = 57,
    UnlinkSymlink = 58,
    VisibleMetadata = 59,
    WindowCommit = 60,
    WindowFsync = 61,
    WindowOp = 62,
    WindowTruncateFile = 63,
    WindowWriteFileAt = 64,
    WriteFileAt = 65,
    WriteFileAtBounded = 66,
    ClearSecurityDescriptor = 67,
    SecurityDescriptor = 68,
    SetSecurityDescriptor = 69,
    SetSecurityProjectionPolicy = 70,
    SetVolumeLabel = 71,
    FileAllocationFrom = 72,
    ObjectComment = 73,
    SetObjectComment = 74,
    SnapshotObjectComment = 75,
    Attribute = 76,
    AttributeNames = 77,
    SetAttributes = 78,
    SnapshotAttribute = 79,
    SnapshotAttributeNames = 80,
    SnapshotSecurityDescriptor = 81,
    SetObjectOwner = 82,
    SetObjectTimes = 83,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApiContext {
    pub operation: u64,
    pub span: u64,
    pub parent_span: u64,
    pub method: Option<ApiMethod>,
}

pub(crate) struct ApiToken {
    previous: ApiContext,
    active: ApiContext,
}

#[derive(Clone, Copy)]
pub(crate) enum ApiOutcome {
    Succeeded,
    Failed,
    Unwound,
}

/// Object-map resolution, not proof of a successful read or validated metadata.
/// View zero identifies the live committed map; nonzero is a persistent snapshot ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectContext {
    pub object_id: u64,
    /// Zero for a lookup attempt or a missing object.
    pub record_block: u64,
    pub view_id: u64,
}

/// Allocation decision context; zero start denotes a request without a chosen run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationContext {
    pub start: u64,
    pub blocks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeContext {
    pub owner: u64,
    pub block: u64,
    pub resident: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimContext {
    pub root: u64,
    pub start: u64,
    pub blocks: u64,
}

/// Mount stage owning an observation. Append values; never renumber them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountStage {
    /// Device block size, identification read and decode.
    Identification = 1,
    /// Feature negotiation and the declared block count.
    Negotiation = 2,
    /// Checkpoint slot reads, structural decode and choice.
    Selection = 3,
    /// Feature/root congruence and bounded root loading.
    RootState = 4,
    /// Cache and snapshot work budgets.
    Configuration = 5,
    /// Intent-log scan, replay publication or inspection.
    IntentLog = 6,
}

/// Values the mount path already holds. Selection fields are zero before
/// `MountSelected`; later events describe the volume's selected slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountContext {
    pub mode: crate::mount::MountMode,
    pub stage: MountStage,
    /// Slot holding the event generation's checkpoint (0 = A, 1 = B).
    pub slot: u8,
    /// Generation of the other structurally valid checkpoint, or zero.
    pub other_generation: u64,
    /// Log slots at `MountIntentBegin`; valid prefix records at
    /// `MountIntentScanned`; operations in the group at `MountIntentReplayed`;
    /// replayed (writable modes) or pending (read-only modes) records at
    /// `MountComplete`; zero elsewhere.
    pub count: u32,
    /// `MountIntentScanned` only: the prefix ended at a nonzero invalid record,
    /// a sequence gap or unverifiable referenced data.
    pub damaged_tail: bool,
}

/// Formatter stage owning an observation. Append values; never renumber them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatStage {
    /// Device geometry and parameter checks before the first write.
    Validation = 1,
    /// Metadata construction and writes through identification and slot-B zeroing.
    Metadata = 2,
    /// The barrier after metadata and identification writes.
    MetadataBarrier = 3,
    /// Checkpoint encoding and the slot-A write.
    Publication = 4,
    /// The final barrier after the slot-A write.
    PublicationBarrier = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatContext {
    pub stage: FormatStage,
    /// Device block count read at formatter entry.
    pub total_blocks: u64,
    /// Slot-A address on publication events, zero on the others.
    pub block: u64,
}

/// Verification entry owning an observation. Append values; never renumber them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyScope {
    /// Bounded roots loaded by `verify::load_mount_state_observed`.
    MountState = 1,
    /// Exhaustive decode by `verify::load_committed_state_observed`.
    CommittedState = 2,
    /// Invariant sweep by `verify::full_sweep_observed`.
    FullSweep = 3,
}

/// Verification phase. Append values; never renumber them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyPhase {
    AllocationRoot = 1,
    SharedExtents = 2,
    ObjectMap = 3,
    ObjectRecords = 4,
    SharedMappings = 5,
    Namespace = 6,
    ReclaimQueue = 7,
    AllocationBitmaps = 8,
    IntentLogArea = 9,
    Snapshots = 10,
    RootObject = 11,
    RootDirectory = 12,
    LinkCounts = 13,
    QuarantinedRuns = 14,
    BitmapAccounting = 15,
    FreeCounts = 16,
}

/// Full-sweep finding class. Append values; never renumber them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// Object link count differs from reachable references (object).
    LinkCount = 1,
    /// Mapped object without a directory reference (object).
    UnreferencedObject = 2,
    /// Quarantined block marked free (block).
    QuarantinedFree = 3,
    /// Quarantined block reachable from the checkpoint (block).
    QuarantinedReachable = 4,
    /// Quarantined block inside the allocation-root pool (block).
    QuarantinedPool = 5,
    /// Quarantined block inside the intent-log area (block).
    QuarantinedLogArea = 6,
    /// Reachable, reserved or retained block marked free (block).
    ReachableFree = 7,
    /// Allocated block owned by nothing (block).
    Leak = 8,
    /// Region free count differs from its bitmap pages (region).
    RegionFreeCount = 9,
    /// Region record names an invalid descriptor slot (region).
    DescriptorSlot = 10,
    /// Checkpoint free total differs from the region records (no location).
    FreeTotal = 11,
}

/// Values verification already holds. Location fields are zero when the
/// phase or finding class has no such location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyContext {
    pub scope: VerifyScope,
    /// Absent on `VerifyBegin` and `VerifyComplete`.
    pub phase: Option<VerifyPhase>,
    /// Present only on `VerifyFinding`.
    pub finding: Option<FindingKind>,
    pub region: u32,
    /// Finding index on `VerifyFinding`, finding count on a full sweep's
    /// `VerifyComplete`, zero elsewhere.
    pub ordinal: u64,
    pub object_id: u64,
    pub block: u64,
}

/// File data staged before the common commit tail. Append values; never
/// renumber them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataScope {
    /// A window create writing its content through to claimed blocks.
    CreateWriteThrough = 1,
    /// An existing file's data written in place or to fresh blocks.
    ExistingFileWrite = 2,
    /// The partial tail block a truncate zeroes.
    TruncateTailZero = 3,
}

/// One object's staged write: the logical range it covers and the physical
/// run holding it. Values the filesystem already holds; observation adds no
/// device I/O and no allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataContext {
    pub scope: DataScope,
    pub object_id: u64,
    /// First logical byte the write covers.
    pub offset: u64,
    /// Bytes the write covers.
    pub length: u64,
    /// First block of the physical run, zero when the range holds no block.
    pub start: u64,
    /// Blocks in that run, saturating at `u32::MAX`.
    pub blocks: u32,
}

/// Read-only descent path. Append values; never renumber them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPath {
    /// One name resolved in one directory.
    Lookup = 1,
    /// A whole directory or one bounded directory page.
    Enumeration = 2,
    /// File data through its extent mapping or direct layout.
    FileData = 3,
    /// Snapshot registry or ledger maintenance over the captured views.
    Maintenance = 4,
}

/// A read-only tree descent. View zero identifies the live committed view;
/// nonzero is a persistent snapshot ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewReadContext {
    pub path: ReadPath,
    pub view_id: u64,
    /// Object owning the tree being descended.
    pub owner: u64,
    /// Root block of that tree.
    pub block: u64,
}

/// Mutually exclusive payload of the mount, format, verify, data and view
/// categories, stored in one field to bound the event layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleContext {
    Mount(MountContext),
    Format(FormatContext),
    Verify(VerifyContext),
    Data(DataContext),
    View(ViewReadContext),
}

/// A transaction may outlive recorder replacement in a deferred window.
/// Weak ownership prevents that transaction retaining a detached recorder.
#[derive(Debug, Clone, Default)]
pub(crate) struct AllocationObserver(std::sync::Weak<RecorderCell>);
impl AllocationObserver {
    pub(crate) fn new(recorder: Option<&SharedRecorder>) -> Self {
        Self(recorder.map_or_else(std::sync::Weak::new, std::sync::Arc::downgrade))
    }
    pub(crate) fn reclaim(&self, generation: u64, kind: EventKind, context: ReclaimContext) {
        if let Some(recorder) = self.0.upgrade() {
            let mut recorder = recorder.borrow_mut();
            if recorder.subsystem_enabled {
                recorder.emit_context(
                    generation,
                    kind,
                    false,
                    None,
                    None,
                    None,
                    Some(context),
                    None,
                );
            }
        }
    }
    pub(crate) fn tree(&self, generation: u64, kind: EventKind, context: TreeContext) {
        if let Some(recorder) = self.0.upgrade() {
            let mut recorder = recorder.borrow_mut();
            if recorder.subsystem_enabled {
                recorder.emit_context(
                    generation,
                    kind,
                    false,
                    None,
                    None,
                    Some(context),
                    None,
                    None,
                );
            }
        }
    }
    pub(crate) fn emit(&self, generation: u64, kind: EventKind, start: u64, blocks: u64) {
        if let Some(recorder) = self.0.upgrade() {
            let mut recorder = recorder.borrow_mut();
            if recorder.subsystem_enabled {
                recorder.emit_context(
                    generation,
                    kind,
                    false,
                    None,
                    Some(AllocationContext { start, blocks }),
                    None,
                    None,
                    None,
                );
            }
        }
    }
}

/// An attempt is unique within this recorder, including retries that reuse a
/// checkpoint generation. It starts at the common commit tail, not API entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub attempt: u64,
    pub generation: u64,
    pub kind: EventKind,
    /// The volume requires remount after unsafe window mutation or publication.
    /// API events sample this flag at entry/exit; it is not a durability verdict
    /// or a claim that unwinding restored all in-memory filesystem state.
    pub requires_remount: bool,
    /// Zero context denotes no observed API (including legacy commit-only scope).
    pub api: ApiContext,
    /// Recorder-local deferred-window identity, zero outside observed windows.
    pub window: u64,
    /// Last observed durable group, except WindowLogBegin/WindowLogFailed
    /// identify the attempted group. Zero means no group is represented.
    pub log_sequence: u32,
    /// Present only on explicit object resolution events; never inherited by siblings.
    pub object: Option<ObjectContext>,
    pub allocation: Option<AllocationContext>,
    pub tree: Option<TreeContext>,
    pub reclaim: Option<ReclaimContext>,
    /// Present exactly on mount, format and verify events.
    pub lifecycle: Option<LifecycleContext>,
}

/// Outcome of a nonblocking live-adapter delivery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkResult {
    Accepted,
    /// This event was not accepted; future events may be attempted.
    Busy,
    /// Stop callbacks until the adapter is explicitly replaced.
    Closed,
}

/// Trusted synchronous adapter, called inside filesystem operations. It must
/// not block, allocate, unwind, reenter the filesystem or change its state.
/// Use a preallocated queue to dispatch arbitrary consumer code elsewhere.
/// Return Busy/Closed for ordinary transport failure; these are observations,
/// never filesystem errors. The core cannot enforce arbitrary host code's
/// execution-time or panic behaviour, just as for a BlockDevice implementation.
pub trait LiveSink: Send + Sync {
    fn try_event(&mut self, event: Event) -> SinkResult;
}

impl<F: FnMut(Event) -> SinkResult + Send + Sync> LiveSink for F {
    fn try_event(&mut self, event: Event) -> SinkResult {
        self(event)
    }
}

pub struct FlightRecorder {
    events: VecDeque<Event>,
    limit: usize,
    sequence: u64,
    attempt: u64,
    dropped: u64,
    categories: Categories,
    filtered: u64,
    sink: Option<Box<dyn LiveSink>>,
    sink_closed: bool,
    delivered: u64,
    missed: u64,
    api_enabled: bool,
    object_enabled: bool,
    subsystem_enabled: bool,
    data_enabled: bool,
    view_enabled: bool,
    api_next: u64,
    api_context: ApiContext,
    api_exhausted: bool,
    window_next: u64,
    window: u64,
    window_log_sequence: u32,
    window_exhausted: bool,
}

impl std::fmt::Debug for FlightRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlightRecorder")
            .field("events", &self.events)
            .field("limit", &self.limit)
            .field("sequence", &self.sequence)
            .field("attempt", &self.attempt)
            .field("dropped", &self.dropped)
            .field("categories", &self.categories)
            .field("filtered", &self.filtered)
            .field("sink_attached", &self.sink.is_some())
            .field("sink_closed", &self.sink_closed)
            .field("delivered", &self.delivered)
            .field("missed", &self.missed)
            .field("api_enabled", &self.api_enabled)
            .field("object_enabled", &self.object_enabled)
            .field("api_context", &self.api_context)
            .field("api_exhausted", &self.api_exhausted)
            .field("window", &self.window)
            .field("window_log_sequence", &self.window_log_sequence)
            .field("window_exhausted", &self.window_exhausted)
            .finish()
    }
}

impl FlightRecorder {
    pub fn new(capacity: NonZeroUsize) -> Result<Self, TryReserveError> {
        let mut events = VecDeque::new();
        events.try_reserve_exact(capacity.get())?;
        Ok(Self {
            events,
            limit: capacity.get(),
            sequence: 0,
            attempt: 0,
            dropped: 0,
            categories: Categories::ALL,
            filtered: 0,
            sink: None,
            sink_closed: false,
            delivered: 0,
            missed: 0,
            api_enabled: false,
            object_enabled: false,
            subsystem_enabled: false,
            data_enabled: false,
            view_enabled: false,
            api_next: 0,
            api_context: ApiContext::default(),
            api_exhausted: false,
            window_next: 0,
            window: 0,
            window_log_sequence: 0,
            window_exhausted: false,
        })
    }

    pub fn events(&self) -> impl ExactSizeIterator<Item = &Event> {
        self.events.iter()
    }

    /// Remove retained events without releasing storage or resetting identities
    /// and the cumulative dropped count. Consumed events cannot later count as
    /// overwritten records when a consumer captures the next operation.
    pub fn drain(&mut self) -> impl ExactSizeIterator<Item = Event> + '_ {
        self.events.drain(..)
    }

    pub fn capacity(&self) -> usize {
        self.limit
    }

    /// Overwritten records, or records refused after identifier exhaustion.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Change future admission only. Retained records and cumulative counters
    /// are preserved; filtered Begin events still advance attempt identities.
    pub fn set_categories(&mut self, categories: Categories) -> Categories {
        std::mem::replace(&mut self.categories, categories)
    }

    /// Last assigned identity, including events deliberately filtered out.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn attempt(&self) -> u64 {
        self.attempt
    }

    pub fn categories(&self) -> Categories {
        self.categories
    }

    /// Deliberately unrecorded events, separate from overwrites/exhaustion.
    pub fn filtered(&self) -> u64 {
        self.filtered
    }

    /// Replace the adapter without dropping it during emission. Counts are
    /// cumulative across replacement; only Closed state is cleared. None
    /// detaches live observation without changing local ring collection.
    pub fn replace_sink(&mut self, sink: Option<Box<dyn LiveSink>>) -> Option<Box<dyn LiveSink>> {
        self.sink_closed = false;
        std::mem::replace(&mut self.sink, sink)
    }

    pub fn sink_closed(&self) -> bool {
        self.sink_closed
    }

    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    /// Selected events not accepted while a live adapter is attached, including
    /// events following Closed. Detached and category-filtered events do not
    /// count. Local ring overwrites have their independent dropped counter.
    pub fn missed(&self) -> u64 {
        self.missed
    }

    /// Opt in to core API and deferred-window spans. Commit-only profiles do not enable
    /// this scope, preserving their event sequences and historical wire bytes.
    pub fn enable_api_observation(&mut self) {
        self.api_enabled = true;
    }

    pub fn api_observation_enabled(&self) -> bool {
        self.api_enabled
    }

    pub(crate) fn observe_window(
        &mut self,
        generation: u64,
        log_sequence: u32,
        attached: bool,
        requires_remount: bool,
    ) {
        if !self.api_enabled {
            return;
        }
        if self.window != 0 {
            if attached {
                return;
            }
            // A fresh engine window cannot inherit a prior observation whose
            // final event was interrupted (for example by provider unwinding).
            self.window_event(
                generation,
                EventKind::WindowDetached,
                None,
                requires_remount,
            );
        }
        if self.window_next == u64::MAX || self.window_exhausted {
            self.window_exhausted = true;
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.window_next += 1;
        self.window = self.window_next;
        self.window_log_sequence = log_sequence;
        self.emit(
            generation,
            if attached {
                EventKind::WindowAttached
            } else {
                EventKind::WindowOpened
            },
            requires_remount,
        );
    }

    pub(crate) fn window_event(
        &mut self,
        generation: u64,
        kind: EventKind,
        attempted_sequence: Option<u32>,
        requires_remount: bool,
    ) {
        if !self.api_enabled || self.window == 0 {
            return;
        }
        let previous_sequence = self.window_log_sequence;
        if let Some(sequence) = attempted_sequence {
            self.window_log_sequence = sequence;
        }
        self.emit(generation, kind, requires_remount);
        if kind != EventKind::WindowLogDurable {
            self.window_log_sequence = previous_sequence;
        }
        if matches!(kind, EventKind::WindowClosed | EventKind::WindowDetached) {
            self.window = 0;
            self.window_log_sequence = 0;
        }
    }

    pub(crate) fn begin_api(
        &mut self,
        method: ApiMethod,
        generation: u64,
        requires_remount: bool,
    ) -> Option<ApiToken> {
        if !self.api_enabled {
            return None;
        }
        if self.api_next == u64::MAX || self.api_exhausted {
            self.api_exhausted = true;
            self.dropped = self.dropped.saturating_add(1);
            return None;
        }
        self.api_next += 1;
        let previous = self.api_context;
        self.api_context = ApiContext {
            operation: if previous.span == 0 {
                self.api_next
            } else {
                previous.operation
            },
            span: self.api_next,
            parent_span: previous.span,
            method: Some(method),
        };
        let token = ApiToken {
            previous,
            active: self.api_context,
        };
        self.emit(generation, EventKind::ApiBegin, requires_remount);
        Some(token)
    }

    pub(crate) fn end_api(
        &mut self,
        token: ApiToken,
        generation: u64,
        outcome: ApiOutcome,
        requires_remount: bool,
    ) {
        if self.api_context != token.active {
            // Never attribute a stale guard to a different active scope.
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.emit(
            generation,
            match outcome {
                ApiOutcome::Succeeded => EventKind::ApiSucceeded,
                ApiOutcome::Failed => EventKind::ApiFailed,
                ApiOutcome::Unwound => EventKind::ApiUnwound,
            },
            requires_remount,
        );
        self.api_context = token.previous;
    }

    /// Observe allocation, mutable-tree I/O and reclaim transitions, including
    /// their enclosing API identities. Observation adds no disk I/O.
    /// Category selection independently filters each subsystem's records.
    pub fn enable_subsystem_observation(&mut self) {
        self.api_enabled = true;
        self.subsystem_enabled = true;
    }

    /// Opt in to object resolution and its API identities. No new disk I/O.
    pub fn enable_object_observation(&mut self) {
        self.api_enabled = true;
        self.object_enabled = true;
    }

    /// Observe file data staged before the common commit tail, including the
    /// enclosing API identities. Observation adds no disk I/O.
    pub fn enable_data_observation(&mut self) {
        self.api_enabled = true;
        self.data_enabled = true;
    }

    /// Observe read-only tree descents over live and captured views,
    /// including the enclosing API identities. Observation adds no disk I/O.
    pub fn enable_view_observation(&mut self) {
        self.api_enabled = true;
        self.view_enabled = true;
    }

    /// Staged-write events keep the ambient window identity and intent-group
    /// sequence, because a window create writes inside its own group.
    pub(crate) fn data_event(
        &mut self,
        generation: u64,
        kind: EventKind,
        requires_remount: bool,
        context: DataContext,
    ) {
        if self.data_enabled {
            self.emit_context(
                generation,
                kind,
                requires_remount,
                None,
                None,
                None,
                None,
                Some(LifecycleContext::Data(context)),
            );
        }
    }

    pub(crate) fn view_event(
        &mut self,
        generation: u64,
        kind: EventKind,
        requires_remount: bool,
        context: ViewReadContext,
    ) {
        if self.view_enabled {
            self.emit_context(
                generation,
                kind,
                requires_remount,
                None,
                None,
                None,
                None,
                Some(LifecycleContext::View(context)),
            );
        }
    }

    /// The two intent-group barriers owned by an fsync: the data barrier a
    /// record's existing-file updates require, and an empty group's flush.
    pub(crate) fn intent_io_event(
        &mut self,
        generation: u64,
        kind: EventKind,
        requires_remount: bool,
    ) {
        if self.data_enabled {
            self.emit(generation, kind, requires_remount);
        }
    }

    pub(crate) fn object_event(
        &mut self,
        generation: u64,
        kind: EventKind,
        context: ObjectContext,
        requires_remount: bool,
    ) {
        if self.object_enabled {
            self.emit_context(
                generation,
                kind,
                requires_remount,
                Some(context),
                None,
                None,
                None,
                None,
            );
        }
    }

    pub(crate) fn emit(&mut self, generation: u64, kind: EventKind, requires_remount: bool) {
        self.emit_context(
            generation,
            kind,
            requires_remount,
            None,
            None,
            None,
            None,
            None,
        );
    }

    /// Emit a mount, format or verify event with its explicit intent-group
    /// sequence (zero when no group is represented). Entry points supplying a
    /// recorder are the opt-in; identities, filtering and loss accounting follow
    /// the common path. Emission performs no I/O and no allocation.
    pub(crate) fn lifecycle_event(
        &mut self,
        generation: u64,
        kind: EventKind,
        requires_remount: bool,
        log_sequence: u32,
        context: LifecycleContext,
    ) {
        let previous = std::mem::replace(&mut self.window_log_sequence, log_sequence);
        self.emit_context(
            generation,
            kind,
            requires_remount,
            None,
            None,
            None,
            None,
            Some(context),
        );
        self.window_log_sequence = previous;
    }

    /// Leave `remaining` assignable sequence identities for exhaustion tests.
    #[cfg(test)]
    pub(crate) fn leave_sequence_identities(&mut self, remaining: u64) {
        self.sequence = u64::MAX - remaining;
    }

    // Keep the typed optional payloads explicit at each emission site.
    #[allow(clippy::too_many_arguments)]
    fn emit_context(
        &mut self,
        generation: u64,
        kind: EventKind,
        requires_remount: bool,
        object: Option<ObjectContext>,
        allocation: Option<AllocationContext>,
        tree: Option<TreeContext>,
        reclaim: Option<ReclaimContext>,
        lifecycle: Option<LifecycleContext>,
    ) {
        if self.api_exhausted
            || self.window_exhausted
            || self.sequence == u64::MAX
            || (kind == EventKind::Begin && self.attempt == u64::MAX)
        {
            self.dropped = self.dropped.saturating_add(1);
            // Refuse all following events rather than reuse an identity.
            self.sequence = u64::MAX;
            return;
        }
        if kind == EventKind::Begin {
            self.attempt += 1;
        }
        self.sequence += 1;
        if !self.categories.contains(kind.category()) {
            self.filtered = self.filtered.saturating_add(1);
            return;
        }
        if self.events.len() == self.limit {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        let event = Event {
            sequence: self.sequence,
            attempt: if matches!(
                kind.category(),
                Category::Api
                    | Category::Window
                    | Category::Object
                    | Category::Allocator
                    | Category::Tree
                    | Category::Reclaim
                    | Category::Mount
                    | Category::Format
                    | Category::Verify
                    | Category::Data
                    | Category::View
            ) {
                0
            } else {
                self.attempt
            },
            generation,
            kind,
            requires_remount,
            api: self.api_context,
            window: self.window,
            log_sequence: self.window_log_sequence,
            object,
            allocation,
            tree,
            reclaim,
            lifecycle,
        };
        self.events.push_back(event);
        if let Some(sink) = &mut self.sink {
            let result = if self.sink_closed {
                SinkResult::Closed
            } else {
                sink.try_event(event)
            };
            match result {
                SinkResult::Accepted => self.delivered = self.delivered.saturating_add(1),
                SinkResult::Busy => self.missed = self.missed.saturating_add(1),
                SinkResult::Closed => {
                    self.missed = self.missed.saturating_add(1);
                    self.sink_closed = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod category_tests {
    use super::*;

    #[test]
    fn filtering_preserves_attempts_sequences_and_loss_accounting() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        let allocated = ring.events.capacity();
        assert_eq!(ring.set_categories(Categories::NONE), Categories::ALL);
        ring.emit(9, EventKind::Begin, false);
        ring.emit(9, EventKind::Failed, false);
        ring.set_categories(Categories::NONE.with(Category::Error));
        ring.emit(9, EventKind::Begin, false);
        ring.emit(9, EventKind::Failed, true);
        let last = *ring.events().last().unwrap();
        assert_eq!((last.sequence, last.attempt), (4, 2));
        assert!(last.requires_remount);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 0));
        ring.set_categories(Categories::ALL);
        ring.emit(9, EventKind::Begin, false);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 1));
        assert_eq!(ring.drain().count(), 1);
        ring.emit(9, EventKind::Adopted, false);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 1));
        assert_eq!(ring.events.capacity(), allocated);
        ring.set_categories(Categories::NONE);
        ring.sequence = u64::MAX;
        ring.emit(9, EventKind::Begin, false);
        assert_eq!((ring.filtered(), ring.dropped()), (3, 2));
        assert_eq!(ring.events().last().unwrap().sequence, 6);
    }

    #[test]
    fn category_mapping_distinguishes_transaction_io_checkpoint_and_error() {
        for (category, kinds) in [
            (
                Category::Transaction,
                vec![EventKind::Begin, EventKind::Adopted],
            ),
            (Category::Io, vec![EventKind::DataWritesComplete]),
            (
                Category::Checkpoint,
                vec![
                    EventKind::MetadataDurable,
                    EventKind::PublicationBegin,
                    EventKind::CheckpointDurable,
                ],
            ),
            (Category::Error, vec![EventKind::Failed]),
        ] {
            let selected = Categories::NONE.with(category);
            for kind in kinds {
                assert_eq!(kind.category(), category);
                assert!(selected.contains(kind.category()));
                assert!(Categories::ALL.contains(kind.category()));
                assert!(!Categories::NONE.contains(kind.category()));
            }
        }
    }
}

#[cfg(test)]
mod sink_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn live_failures_are_counted_without_dropping_or_recalling_closed_adapter() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<FlightRecorder>();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.replace_sink(Some(Box::new(move |_event: Event| {
            match observed.fetch_add(1, Ordering::SeqCst) {
                0 => SinkResult::Accepted,
                1 => SinkResult::Busy,
                2 => SinkResult::Closed,
                _ => panic!("closed adapter called"),
            }
        })));
        for _ in 0..4 {
            ring.emit(1, EventKind::Begin, false);
            ring.emit(1, EventKind::Adopted, false);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!((ring.delivered(), ring.missed(), ring.dropped()), (1, 7, 7));
        assert!(ring.sink_closed());
        ring.set_categories(Categories::NONE);
        ring.emit(2, EventKind::Begin, false);
        assert_eq!((ring.filtered(), ring.missed()), (1, 7));
        let old = ring.replace_sink(None);
        assert!(old.is_some());
        assert!(!ring.sink_closed());
        ring.set_categories(Categories::ALL);
        ring.emit(2, EventKind::Adopted, false);
        assert_eq!((ring.delivered(), ring.missed()), (1, 7));
        ring.replace_sink(Some(Box::new(|_: Event| SinkResult::Accepted)));
        ring.emit(3, EventKind::Begin, false);
        assert_eq!((ring.delivered(), ring.missed()), (2, 7));
        ring.sequence = u64::MAX;
        ring.emit(3, EventKind::Adopted, false);
        assert_eq!((ring.delivered(), ring.missed()), (2, 7));
    }
}

#[cfg(test)]
mod api_tests {
    use super::*;

    #[test]
    fn nested_api_spans_share_a_request_and_bind_only_commits_to_attempts() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(32).unwrap()).unwrap();
        assert!(ring.begin_api(ApiMethod::Sync, 1, false).is_none());
        assert_eq!(ring.sequence(), 0);
        ring.enable_api_observation();
        let outer = ring
            .begin_api(ApiMethod::CreateFileInRoot, 1, false)
            .unwrap();
        let inner = ring
            .begin_api(ApiMethod::CreateFileInDirectory, 1, false)
            .unwrap();
        ring.emit(2, EventKind::Begin, false);
        ring.emit(2, EventKind::Adopted, false);
        ring.end_api(inner, 2, ApiOutcome::Succeeded, false);
        ring.end_api(outer, 2, ApiOutcome::Failed, false);
        let events: Vec<_> = ring.events().copied().collect();
        assert!(events.iter().all(|e| e.api.operation == 1));
        assert_eq!((events[1].api.span, events[1].api.parent_span), (2, 1));
        assert_eq!(events[2].api, events[1].api);
        assert_eq!((events[2].attempt, events[3].attempt), (1, 1));
        assert_eq!(events[5].api.span, 1);
        assert_eq!(events[5].kind, EventKind::ApiFailed);
        assert_eq!(events[5].attempt, 0);
        assert_eq!(ring.api_context, ApiContext::default());
        let next = ring.begin_api(ApiMethod::Stat, 2, false).unwrap();
        assert_eq!(
            (ring.api_context.operation, ring.api_context.parent_span),
            (3, 0)
        );
        ring.end_api(next, 2, ApiOutcome::Succeeded, false);
    }

    #[test]
    fn filtered_api_spans_keep_context_and_exhaustion_never_reuses_it() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.enable_api_observation();
        ring.set_categories(Categories::NONE.with(Category::Checkpoint));
        let token = ring.begin_api(ApiMethod::Sync, 1, false).unwrap();
        ring.emit(2, EventKind::Begin, false);
        ring.emit(2, EventKind::CheckpointDurable, false);
        ring.end_api(token, 2, ApiOutcome::Succeeded, false);
        let event = *ring.events().last().unwrap();
        assert_eq!(
            (event.api.operation, event.api.span, event.sequence),
            (1, 1, 3)
        );
        assert_eq!(ring.filtered(), 3);
        assert_eq!(ring.dropped(), 0);
        ring.api_next = u64::MAX;
        assert!(ring.begin_api(ApiMethod::Sync, 2, false).is_none());
        ring.emit(3, EventKind::Begin, false);
        assert_eq!(*ring.events().last().unwrap(), event);
        assert_eq!(ring.dropped(), 2);
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn window_context_survives_filtering_and_stops_before_identity_reuse() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(16).unwrap()).unwrap();
        ring.observe_window(2, 0, false, false);
        assert_eq!((ring.window, ring.sequence()), (0, 0));
        ring.enable_api_observation();
        ring.set_categories(Categories::NONE.with(Category::Checkpoint));
        ring.observe_window(2, 0, false, false);
        ring.window_event(2, EventKind::WindowLogBegin, Some(1), false);
        assert_eq!(ring.window_log_sequence, 0);
        ring.window_event(2, EventKind::WindowLogDurable, Some(1), false);
        ring.emit(2, EventKind::Begin, false);
        ring.emit(2, EventKind::CheckpointDurable, false);
        ring.window_event(2, EventKind::WindowClosed, None, false);
        assert_eq!((ring.window, ring.window_log_sequence), (0, 0));
        let event = *ring.events().next().unwrap();
        assert_eq!((event.window, event.log_sequence, event.attempt), (1, 1, 1));
        assert_eq!((ring.filtered(), ring.dropped()), (5, 0));
        ring.window_next = u64::MAX;
        ring.observe_window(3, 0, false, false);
        ring.emit(3, EventKind::Begin, false);
        assert_eq!(ring.dropped(), 2);
        assert_eq!(ring.events().len(), 1);
        assert_eq!(ring.window, 0);
    }

    #[test]
    fn fresh_window_detaches_an_interrupted_context_and_failed_group_is_not_acknowledged() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(16).unwrap()).unwrap();
        ring.enable_api_observation();
        ring.observe_window(2, 3, true, false);
        ring.window_event(2, EventKind::WindowLogFailed, Some(4), true);
        assert_eq!(ring.window_log_sequence, 3);
        assert_eq!(ring.events().last().unwrap().log_sequence, 4);
        ring.observe_window(2, 0, false, false);
        let events: Vec<_> = ring.events().copied().collect();
        assert_eq!(events[2].kind, EventKind::WindowDetached);
        assert_eq!((events[2].window, events[2].log_sequence), (1, 3));
        assert_eq!(events[3].kind, EventKind::WindowOpened);
        assert_eq!((events[3].window, events[3].log_sequence), (2, 0));
    }
}

#[cfg(test)]
mod object_tests {
    use super::*;

    #[test]
    fn object_payload_reaches_bounded_live_delivery_with_explicit_loss() {
        use std::sync::mpsc::{sync_channel, TrySendError};
        let (sender, receiver) = sync_channel(1);
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.enable_object_observation();
        ring.replace_sink(Some(Box::new(move |event| match sender.try_send(event) {
            Ok(()) => SinkResult::Accepted,
            Err(TrySendError::Full(_)) => SinkResult::Busy,
            Err(TrySendError::Disconnected(_)) => SinkResult::Closed,
        })));
        let context = ObjectContext {
            object_id: 17,
            record_block: 42,
            view_id: 3,
        };
        ring.object_event(5, EventKind::ObjectMapped, context, false);
        ring.object_event(5, EventKind::ObjectMapped, context, false);
        assert_eq!(receiver.try_recv().unwrap().object, Some(context));
        assert_eq!((ring.delivered(), ring.missed(), ring.dropped()), (1, 1, 1));
        drop(receiver);
        ring.object_event(5, EventKind::ObjectMapped, context, false);
        assert_eq!((ring.delivered(), ring.missed()), (1, 2));
        assert_eq!(ring.events().next().unwrap().object, Some(context));
    }

    #[test]
    fn object_filtering_loss_and_scope_do_not_reuse_commit_identity() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        let context = ObjectContext {
            object_id: 17,
            record_block: 0,
            view_id: 3,
        };
        ring.object_event(5, EventKind::ObjectLookup, context, false);
        assert_eq!(
            ring.sequence(),
            0,
            "disabled observation must preserve legacy sequences"
        );
        ring.enable_object_observation();
        ring.set_categories(Categories::NONE.with(Category::Object));
        ring.emit(6, EventKind::Begin, false);
        ring.object_event(5, EventKind::ObjectLookup, context, false);
        ring.object_event(
            5,
            EventKind::ObjectMapped,
            ObjectContext {
                record_block: 42,
                ..context
            },
            false,
        );
        let event = *ring.events().next().unwrap();
        assert_eq!(event.sequence, 3);
        assert_eq!(event.attempt, 0);
        assert_eq!(event.object.unwrap().record_block, 42);
        assert_eq!((ring.filtered(), ring.dropped()), (1, 1));
        ring.set_categories(Categories::ALL);
        ring.emit(6, EventKind::Adopted, false);
        let event = ring.events().next().unwrap();
        assert_eq!(event.attempt, 1);
        assert!(
            event.object.is_none(),
            "object context must not leak into later events"
        );
    }
}

#[cfg(test)]
mod recorder_borrow_tests {
    use super::*;
    #[test]
    fn shared_readers_conflicts_and_unwind_leave_recorder_usable() {
        let cell = RecorderCell::new(FlightRecorder::new(NonZeroUsize::new(8).unwrap()).unwrap());
        let a = cell.borrow();
        let b = cell.borrow();
        assert_eq!(a.events().count(), b.events().count());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(cell.borrow_mut());
        }))
        .is_err());
        drop((a, b));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut writer = cell.borrow_mut();
            writer.emit(1, EventKind::Begin, false);
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(cell.borrow());
            }))
            .is_err());
            panic!("provider unwind while observing");
        }))
        .is_err());
        assert_eq!(cell.borrow().events().count(), 1);
        cell.borrow_mut().emit(1, EventKind::Adopted, false);
        assert_eq!(cell.into_inner().events().count(), 2);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    fn mount_context() -> LifecycleContext {
        LifecycleContext::Mount(MountContext {
            mode: crate::mount::MountMode::Recovery,
            stage: MountStage::IntentLog,
            slot: 1,
            other_generation: 4,
            count: 2,
            damaged_tail: true,
        })
    }

    fn format_context() -> LifecycleContext {
        LifecycleContext::Format(FormatContext {
            stage: FormatStage::PublicationBarrier,
            total_blocks: 256,
            block: 1,
        })
    }

    fn verify_context() -> LifecycleContext {
        LifecycleContext::Verify(VerifyContext {
            scope: VerifyScope::FullSweep,
            phase: Some(VerifyPhase::LinkCounts),
            finding: Some(FindingKind::LinkCount),
            region: 3,
            ordinal: 9,
            object_id: 17,
            block: 42,
        })
    }

    #[test]
    fn every_lifecycle_kind_is_admitted_by_exactly_its_category() {
        for (category, kinds) in [
            (
                Category::Mount,
                vec![
                    EventKind::MountBegin,
                    EventKind::MountSelected,
                    EventKind::MountIntentBegin,
                    EventKind::MountIntentScanned,
                    EventKind::MountIntentReplayed,
                    EventKind::MountComplete,
                    EventKind::MountFailed,
                ],
            ),
            (
                Category::Format,
                vec![
                    EventKind::FormatBegin,
                    EventKind::FormatMetadataDurable,
                    EventKind::FormatPublicationBegin,
                    EventKind::FormatCheckpointDurable,
                    EventKind::FormatFailed,
                ],
            ),
            (
                Category::Verify,
                vec![
                    EventKind::VerifyBegin,
                    EventKind::VerifyPhase,
                    EventKind::VerifyFinding,
                    EventKind::VerifyComplete,
                    EventKind::VerifyFailed,
                ],
            ),
            (
                Category::Data,
                vec![
                    EventKind::DataWriteBegin,
                    EventKind::DataWriteComplete,
                    EventKind::DataWriteFailed,
                ],
            ),
            (
                Category::View,
                vec![
                    EventKind::ViewReadBegin,
                    EventKind::ViewReadComplete,
                    EventKind::ViewReadFailed,
                ],
            ),
            (
                Category::Io,
                vec![
                    EventKind::DataWritesComplete,
                    EventKind::IntentDataDurable,
                    EventKind::IntentEmptyFlush,
                ],
            ),
        ] {
            let selected = Categories::NONE.with(category);
            for kind in kinds {
                assert_eq!(kind.category(), category);
                assert!(selected.contains(kind.category()));
                assert!(Categories::ALL.contains(kind.category()));
                assert!(!Categories::NONE.contains(kind.category()));
                for other in [
                    Category::Mount,
                    Category::Format,
                    Category::Verify,
                    Category::Data,
                    Category::View,
                    Category::Io,
                ] {
                    assert_eq!(
                        Categories::NONE.with(other).contains(kind.category()),
                        other == category
                    );
                }
            }
        }
    }

    #[test]
    fn lifecycle_filtering_loss_and_delivery_keep_identities_and_group_sequences() {
        use std::sync::mpsc::{sync_channel, TrySendError};
        let (sender, receiver) = sync_channel(1);
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        let allocated = ring.events.capacity();
        ring.replace_sink(Some(Box::new(move |event| match sender.try_send(event) {
            Ok(()) => SinkResult::Accepted,
            Err(TrySendError::Full(_)) => SinkResult::Busy,
            Err(TrySendError::Disconnected(_)) => SinkResult::Closed,
        })));
        ring.set_categories(Categories::NONE.with(Category::Format));
        ring.lifecycle_event(7, EventKind::MountIntentReplayed, false, 3, mount_context());
        assert_eq!((ring.sequence(), ring.filtered()), (1, 1));
        assert_eq!(ring.events().len(), 0);
        ring.lifecycle_event(
            1,
            EventKind::FormatCheckpointDurable,
            false,
            0,
            format_context(),
        );
        ring.lifecycle_event(1, EventKind::FormatFailed, true, 0, format_context());
        assert_eq!(
            (ring.sequence(), ring.filtered(), ring.dropped()),
            (3, 1, 1)
        );
        assert_eq!((ring.delivered(), ring.missed()), (1, 1));
        let delivered = receiver.try_recv().unwrap();
        assert_eq!(delivered.kind, EventKind::FormatCheckpointDurable);
        assert_eq!(delivered.lifecycle, Some(format_context()));
        assert_eq!((delivered.attempt, delivered.log_sequence), (0, 0));
        let retained = *ring.events().next().unwrap();
        assert!(retained.requires_remount);
        assert_eq!(retained.sequence, 3);

        // The intent-group sequence belongs to its event alone.
        ring.set_categories(Categories::ALL);
        ring.lifecycle_event(7, EventKind::MountIntentReplayed, false, 5, mount_context());
        assert_eq!(ring.events().next().unwrap().log_sequence, 5);
        assert_eq!(ring.window_log_sequence, 0);
        ring.lifecycle_event(2, EventKind::VerifyFinding, false, 0, verify_context());
        let event = *ring.events().next().unwrap();
        assert_eq!((event.log_sequence, event.attempt), (0, 0));
        assert_eq!(event.lifecycle, Some(verify_context()));
        assert_eq!(ring.events.capacity(), allocated);
    }

    fn data_context() -> DataContext {
        DataContext {
            scope: DataScope::ExistingFileWrite,
            object_id: 21,
            offset: 4096,
            length: 8192,
            start: 900,
            blocks: 2,
        }
    }

    fn view_read_context() -> ViewReadContext {
        ViewReadContext {
            path: ReadPath::Enumeration,
            view_id: 6,
            owner: 2,
            block: 77,
        }
    }

    #[test]
    fn staged_write_and_read_path_scopes_are_opt_in_and_lose_records_explicitly() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(1).unwrap()).unwrap();
        ring.data_event(3, EventKind::DataWriteBegin, false, data_context());
        ring.view_event(3, EventKind::ViewReadBegin, false, view_read_context());
        ring.intent_io_event(3, EventKind::IntentEmptyFlush, false);
        assert_eq!(
            (ring.sequence(), ring.events().len()),
            (0, 0),
            "disabled scopes preserve legacy sequences"
        );
        assert!(!ring.api_observation_enabled());

        ring.enable_data_observation();
        ring.enable_view_observation();
        assert!(ring.api_observation_enabled());
        ring.set_categories(Categories::NONE.with(Category::View));
        ring.data_event(3, EventKind::DataWriteBegin, false, data_context());
        ring.intent_io_event(3, EventKind::IntentDataDurable, false);
        assert_eq!((ring.sequence(), ring.filtered()), (2, 2));
        ring.view_event(3, EventKind::ViewReadBegin, false, view_read_context());
        ring.view_event(3, EventKind::ViewReadComplete, true, view_read_context());
        assert_eq!((ring.sequence(), ring.dropped()), (4, 1));
        let event = *ring.events().next().unwrap();
        assert_eq!(event.kind, EventKind::ViewReadComplete);
        assert_eq!(
            event.lifecycle,
            Some(LifecycleContext::View(view_read_context()))
        );
        assert_eq!(event.attempt, 0);
        assert!(event.requires_remount);

        ring.set_categories(Categories::ALL);
        ring.data_event(3, EventKind::DataWriteComplete, false, data_context());
        let event = *ring.events().next().unwrap();
        assert_eq!(
            event.lifecycle,
            Some(LifecycleContext::Data(data_context()))
        );
        ring.leave_sequence_identities(0);
        ring.data_event(3, EventKind::DataWriteFailed, false, data_context());
        ring.view_event(3, EventKind::ViewReadFailed, false, view_read_context());
        assert_eq!(*ring.events().next().unwrap(), event);
        assert_eq!(ring.dropped(), 4);
    }

    #[test]
    fn lifecycle_emission_stops_before_reusing_an_identity() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(8).unwrap()).unwrap();
        ring.leave_sequence_identities(1);
        ring.lifecycle_event(1, EventKind::VerifyBegin, false, 0, verify_context());
        assert_eq!(ring.sequence(), u64::MAX);
        assert_eq!((ring.events().len(), ring.dropped()), (1, 0));
        let last = *ring.events().last().unwrap();
        for kind in [
            EventKind::VerifyFinding,
            EventKind::VerifyComplete,
            EventKind::MountBegin,
            EventKind::FormatBegin,
        ] {
            ring.lifecycle_event(1, kind, false, 0, verify_context());
        }
        assert_eq!(*ring.events().last().unwrap(), last);
        assert_eq!((ring.events().len(), ring.dropped()), (1, 4));
        assert_eq!(ring.filtered(), 0);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// Host layout of the observation types. The ring is the only sized
    /// allocation; a shared owner and a weak observer are pointer pairs.
    #[test]
    fn observation_layout_stays_bounded_and_reports_requested_ring_bytes() {
        let event = std::mem::size_of::<Event>();
        let recorder = std::mem::size_of::<FlightRecorder>();
        println!("size_of Event = {event}");
        println!("size_of FlightRecorder = {recorder}");
        println!(
            "size_of RecorderCell = {}",
            std::mem::size_of::<RecorderCell>()
        );
        println!(
            "size_of SharedRecorder = {}, AllocationObserver = {}",
            std::mem::size_of::<SharedRecorder>(),
            std::mem::size_of::<AllocationObserver>()
        );
        println!(
            "size_of Option<LifecycleContext> = {}, Mount = {}, Format = {}, Verify = {}",
            std::mem::size_of::<Option<LifecycleContext>>(),
            std::mem::size_of::<MountContext>(),
            std::mem::size_of::<FormatContext>(),
            std::mem::size_of::<VerifyContext>()
        );
        for events in [1usize, 256, 4096] {
            let ring = FlightRecorder::new(NonZeroUsize::new(events).unwrap()).unwrap();
            assert!(ring.events.capacity() >= events);
            println!(
                "ring of {events} events requests {} bytes, capacity {}",
                events * event,
                ring.events.capacity()
            );
        }
        assert!(event <= 256, "event layout grew to {event} bytes");
        assert!(recorder <= 256, "recorder layout grew to {recorder} bytes");
        assert_eq!(
            std::mem::size_of::<SharedRecorder>(),
            std::mem::size_of::<usize>()
        );
    }
}

/// Cost qualification for the emission path itself. Heap qualification uses
/// the counting allocator of the measurement crate, because the filesystem
/// crates forbid unsafe code.
#[cfg(test)]
mod mechanism_tests {
    use super::*;

    /// Every payload an emission can carry, exercised in one pass.
    fn emit_every_kind(ring: &mut FlightRecorder, generation: u64) {
        ring.emit(generation, EventKind::Begin, false);
        ring.emit(generation, EventKind::DataWritesComplete, false);
        ring.emit(generation, EventKind::CheckpointDurable, false);
        ring.emit(generation, EventKind::Adopted, false);
        ring.object_event(
            generation,
            EventKind::ObjectMapped,
            ObjectContext {
                object_id: 3,
                record_block: 9,
                view_id: 0,
            },
            false,
        );
        ring.data_event(
            generation,
            EventKind::DataWriteComplete,
            false,
            DataContext {
                scope: DataScope::ExistingFileWrite,
                object_id: 3,
                offset: 0,
                length: 4096,
                start: 40,
                blocks: 1,
            },
        );
        ring.view_event(
            generation,
            EventKind::ViewReadComplete,
            false,
            ViewReadContext {
                path: ReadPath::Lookup,
                view_id: 0,
                owner: 2,
                block: 11,
            },
        );
        ring.lifecycle_event(
            generation,
            EventKind::VerifyFinding,
            false,
            0,
            LifecycleContext::Verify(VerifyContext {
                scope: VerifyScope::FullSweep,
                phase: Some(VerifyPhase::LinkCounts),
                finding: Some(FindingKind::LinkCount),
                region: 0,
                ordinal: 1,
                object_id: 3,
                block: 0,
            }),
        );
        ring.intent_io_event(generation, EventKind::IntentEmptyFlush, false);
    }

    #[test]
    fn emission_keeps_its_reserved_storage_with_or_without_a_live_adapter() {
        let mut ring = FlightRecorder::new(NonZeroUsize::new(256).unwrap()).unwrap();
        ring.enable_subsystem_observation();
        ring.enable_object_observation();
        ring.enable_data_observation();
        ring.enable_view_observation();
        let delivered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = delivered.clone();
        ring.replace_sink(Some(Box::new(move |_event: Event| {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            SinkResult::Accepted
        })));
        // Warm the ring to its full capacity before measuring.
        for generation in 0..64 {
            emit_every_kind(&mut ring, generation);
        }
        let capacity = ring.events.capacity();
        for generation in 0..1024 {
            emit_every_kind(&mut ring, generation);
        }
        assert_eq!(ring.events.capacity(), capacity);
        assert_eq!(ring.events().len(), 256);
        assert!(ring.dropped() > 0 && ring.delivered() > 0);
        assert_eq!(
            delivered.load(std::sync::atomic::Ordering::Relaxed) as u64,
            ring.delivered()
        );

        // Filtering and identity exhaustion keep the same storage.
        ring.set_categories(Categories::NONE);
        emit_every_kind(&mut ring, 1);
        ring.set_categories(Categories::ALL);
        ring.leave_sequence_identities(0);
        emit_every_kind(&mut ring, 1);
        assert_eq!(ring.events.capacity(), capacity);
    }

    /// Uncontended emission cost on this host. Run with
    /// `cargo test --release -- --ignored --nocapture flight::mechanism`.
    #[test]
    #[ignore = "host cost measurement"]
    fn measure_uncontended_emission_cost() {
        const ROUNDS: usize = 200_000;
        let mut ring = FlightRecorder::new(NonZeroUsize::new(4096).unwrap()).unwrap();
        ring.enable_subsystem_observation();
        ring.enable_object_observation();
        ring.enable_data_observation();
        ring.enable_view_observation();
        for generation in 0..64 {
            emit_every_kind(&mut ring, generation);
        }
        let kinds = 9u64;
        let start = std::time::Instant::now();
        for generation in 0..ROUNDS as u64 {
            emit_every_kind(&mut ring, generation);
        }
        let elapsed = start.elapsed();
        let events = ROUNDS as u64 * kinds;
        println!(
            "ring only: {events} events in {elapsed:?} ({:.0} events/s, {:.1} ns/event)",
            events as f64 / elapsed.as_secs_f64(),
            elapsed.as_nanos() as f64 / events as f64
        );

        ring.replace_sink(Some(Box::new(|_event: Event| SinkResult::Accepted)));
        let start = std::time::Instant::now();
        for generation in 0..ROUNDS as u64 {
            emit_every_kind(&mut ring, generation);
        }
        let elapsed = start.elapsed();
        println!(
            "ring and adapter: {events} events in {elapsed:?} ({:.0} events/s, {:.1} ns/event)",
            events as f64 / elapsed.as_secs_f64(),
            elapsed.as_nanos() as f64 / events as f64
        );
        assert!(ring.delivered() > 0);
    }
}
