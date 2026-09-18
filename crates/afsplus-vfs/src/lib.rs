//! Filesystem-neutral, handle-based API shared by host and MacAROS adapters.
//!
//! This layer owns no disk semantics. It translates stable API-v2 concepts
//! into [`afsplus_core::Volume`] operations and keeps OS-specific paths,
//! errno values, FUSE request types, and DOS packets outside the core.

mod authority;
pub mod backup;
pub mod restore;

use std::collections::BTreeMap;
use std::fmt;

use afsplus_block::BlockDevice;
use afsplus_core::flight::FlightRecorder;
use afsplus_core::name_key::comparison_key;
use afsplus_core::volume::{
    BatchOp, DataUpdatePolicy, DirectoryCursor, FileEditLimits, ObjectMetadata, PreservedMetadata,
    SecurityProjectionPolicy, Volume,
};
pub use afsplus_core::AttributeWriteMode;
use afsplus_core::{mount_with_options, CoreError, MountMode, MountOptions};
use afsplus_format::ident::{
    NameKeyAlgorithm, COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG_DATA_UPDATES,
    RO_COMPAT_ORPHAN_DIRECTORY, RO_COMPAT_SHARED_EXTENTS,
};
use afsplus_format::object::ObjectType;
use afsplus_format::posix;
use afsplus_format::FormatError;
use afsplus_format::{Timespec, NAME_MAX_UTF8_BYTES, OBJECT_ROOT};

pub type ObjectId = u64;
pub type Handle = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Directory,
    Symlink,
    Internal,
}

impl From<ObjectType> for NodeKind {
    fn from(value: ObjectType) -> Self {
        match value {
            ObjectType::File => NodeKind::File,
            ObjectType::Directory => NodeKind::Directory,
            ObjectType::Symlink => NodeKind::Symlink,
            ObjectType::Internal => NodeKind::Internal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl AccessMode {
    fn can_read(self) -> bool {
        matches!(self, AccessMode::ReadOnly | AccessMode::ReadWrite)
    }

    fn can_write(self) -> bool {
        matches!(self, AccessMode::WriteOnly | AccessMode::ReadWrite)
    }
}

/// Stored attribute names no one may write. A FUSE host sees the object's
/// comment and protection word as attributes of these names (ADR-120), so an
/// attribute stored under one of them would sit hidden behind the field.
pub const FIELD_ATTRIBUTE_NAMES: [&str; 4] = [
    "aros.comment",
    "aros.protection",
    "user.afsplus.aros.comment",
    "user.afsplus.aros.protection",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    pub object_id: ObjectId,
    pub kind: NodeKind,
    pub size: u64,
    pub allocated_size: u64,
    pub links: u32,
    pub protection: u64,
    /// The POSIX mode this object's protection word projects, and its owner.
    /// The mode is derived, not stored twice: the word is the single carrier.
    pub mode: u16,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
    pub content_generation: u64,
}

impl From<ObjectMetadata> for Stat {
    fn from(record: ObjectMetadata) -> Self {
        Stat {
            object_id: record.object_id,
            kind: record.object_type.into(),
            size: record.size_bytes,
            allocated_size: record.allocated_bytes,
            links: record.link_count,
            protection: record.protection as u64,
            mode: posix::mode_of(record.protection),
            owner_uid: record.owner_uid,
            owner_gid: record.owner_gid,
            created: record.created,
            modified: record.modified,
            changed: record.changed,
            content_generation: record.content_generation,
        }
    }
}

/// One mapped piece of a file's byte space. Bytes between two ranges, and
/// after the last one, are holes and read as zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentRange {
    pub offset: u64,
    pub length: u64,
    /// Reserved storage that was never written: reads as zeros, and a write
    /// into it allocates nothing.
    pub unwritten: bool,
}

/// Answer of [`Vfs::extent_map`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtentMap {
    pub ranges: Vec<ExtentRange>,
    /// Every mapping that intersects the queried range is in `ranges`.
    pub complete: bool,
    /// Where an incomplete answer continues: query again from this offset.
    /// For a complete answer it is the end of the queried range.
    pub next_offset: u64,
}

/// Blocks an operation keeps free beyond its own data, for the metadata
/// that publishing the data window rewrites. Reclaiming deleted space
/// publishes that window first, so this is also what makes reclaiming
/// possible at all when space runs short.
const ROOM_FOR_METADATA_BLOCKS: u64 = 1024;
/// Maintenance transactions one operation runs toward the low-water mark
/// when it already fits; see `Vfs::keep_room`.
const ROOM_STEPS_PER_OPERATION: u32 = 4;

/// Volume identity and negotiated feature masks for management output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeIdentity {
    pub uuid: [u8; 16],
    pub label: String,
    pub compat: u64,
    pub ro_compat: u64,
    pub incompat: u64,
}
/// Reclaim steps an unlink runs on its own behalf. Deliberately small: an
/// unlink must not do work proportional to the file it removes, which is what
/// `near_full_fragmented_unlink_uses_bounded_orphan_progress` pins. What it
/// cannot finish stays an orphan for the mount to resume; see
/// `Vfs::run_maintenance`, and the driver that has to call it.
const RECLAIM_STEPS_PER_RELEASE: usize = 3;

/// Orphan cleanup steps run after an unlink; bounded for the same reason.
const ORPHAN_STEPS_PER_RELEASE: usize = 4;

/// Reclaim steps run on a filesystem sync. A sync is the one moment a host
/// asks for the volume to be tidy, so it drains the backlog rather than taking
/// a slice off it: both loops stop as soon as a step makes no progress, so on
/// an ordinary volume this is two steps and the budget is never reached. It
/// exists so that a busy volume, where every delete leaves more behind than one
/// delete's worth of maintenance takes away, still converges. Without it a
/// battery of ordinary use ended with a megabyte outstanding that was not lost,
/// only never drained.
const RECLAIM_STEPS_PER_SYNC: usize = 256;

/// Orphan cleanup steps run on a filesystem sync, for the same reason.
const ORPHAN_STEPS_PER_SYNC: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatFs {
    pub block_size: u32,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub emergency_headroom_blocks: u64,
    pub available_blocks: u64,
    pub max_name_bytes: u32,
    pub case_sensitive: bool,
    pub unicode_version: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: Vec<u8>,
    pub object_id: ObjectId,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPage {
    pub entries: Vec<DirectoryEntry>,
    pub next_cookie: u64,
    pub eof: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(u64);

impl Capabilities {
    pub const IO_64BIT: u64 = 1 << 0;
    pub const UTF8_NAMES: u64 = 1 << 1;
    pub const HARD_LINKS: u64 = 1 << 2;
    pub const ATOMIC_REPLACE: u64 = 1 << 3;
    pub const OBJECT_IDS: u64 = 1 << 4;
    pub const PAGED_DIRECTORIES: u64 = 1 << 5;
    pub const SPARSE_FILES: u64 = 1 << 6;
    pub const FSYNC: u64 = 1 << 7;
    pub const CLONE_FILE: u64 = 1 << 8;
    pub const CLONE_RANGE: u64 = 1 << 9;
    /// Existing-file writes/truncates can be made durable through the bounded
    /// intent log without publishing a checkpoint per fsync.
    pub const LOGGED_DATA_FSYNC: u64 = 1 << 10;
    /// Files can be persistently opted into ADR-062 private in-place data
    /// updates (ADR-065).
    pub const DATA_POLICY: u64 = 1 << 11;
    /// Open files remain usable after their final visible link is removed;
    /// persistent crash cleanup is provided by ADR-066.
    pub const OPEN_UNLINKED: u64 = 1 << 12;
    pub const SYMLINKS: u64 = 1 << 13;
    /// Additive: `preallocate` reserves unwritten storage for a byte range
    /// without changing the logical size.
    pub const PREALLOCATE: u64 = 1 << 14;
    /// Additive: named attributes can be written. Reading them needs no
    /// capability: a volume that cannot write them still reports what it has.
    pub const EXTENDED_ATTRIBUTES: u64 = 1 << 15;

    pub const BASELINE: Capabilities = Capabilities(
        Self::IO_64BIT
            | Self::UTF8_NAMES
            | Self::HARD_LINKS
            | Self::ATOMIC_REPLACE
            | Self::OBJECT_IDS
            | Self::PAGED_DIRECTORIES
            | Self::SPARSE_FILES
            | Self::FSYNC,
    );

    /// Stable lower-case names for structured output, in bit order.
    pub const NAMES: [(u64, &'static str); 16] = [
        (Self::IO_64BIT, "io_64bit"),
        (Self::UTF8_NAMES, "utf8_names"),
        (Self::HARD_LINKS, "hard_links"),
        (Self::ATOMIC_REPLACE, "atomic_replace"),
        (Self::OBJECT_IDS, "object_ids"),
        (Self::PAGED_DIRECTORIES, "paged_directories"),
        (Self::SPARSE_FILES, "sparse_files"),
        (Self::FSYNC, "fsync"),
        (Self::CLONE_FILE, "clone_file"),
        (Self::CLONE_RANGE, "clone_range"),
        (Self::LOGGED_DATA_FSYNC, "logged_data_fsync"),
        (Self::DATA_POLICY, "data_policy"),
        (Self::OPEN_UNLINKED, "open_unlinked"),
        (Self::SYMLINKS, "symlinks"),
        (Self::PREALLOCATE, "preallocate"),
        (Self::EXTENDED_ATTRIBUTES, "extended_attributes"),
    ];

    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter(move |(bit, _)| self.contains(*bit))
            .map(|(_, name)| name)
    }

    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn contains(self, capability: u64) -> bool {
        self.0 & capability == capability
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    NotFound,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    Invalid,
    ReadOnly,
    NoSpace,
    Stale,
    Busy,
    NotSupported,
    Corrupt(String),
    Io(String),
    Limit(&'static str),
    /// Staged data could not be published and was abandoned so that the volume
    /// stays usable; this many acknowledged operations were lost.
    WindowLost(u32),
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VfsError::NotFound => write!(f, "not found"),
            VfsError::AlreadyExists => write!(f, "already exists"),
            VfsError::NotDirectory => write!(f, "not a directory"),
            VfsError::IsDirectory => write!(f, "is a directory"),
            VfsError::DirectoryNotEmpty => write!(f, "directory not empty"),
            VfsError::Invalid => write!(f, "invalid argument or operation"),
            VfsError::ReadOnly => write!(f, "filesystem is read-only"),
            VfsError::NoSpace => write!(f, "no space left"),
            VfsError::Stale => write!(f, "stale handle or directory cookie"),
            VfsError::Busy => write!(f, "filesystem operation is busy"),
            VfsError::NotSupported => write!(f, "operation is not supported"),
            VfsError::Corrupt(detail) => write!(f, "corrupt filesystem: {detail}"),
            VfsError::Io(detail) => write!(f, "I/O error: {detail}"),
            VfsError::Limit(detail) => write!(f, "implementation limit: {detail}"),
            VfsError::WindowLost(lost) => write!(
                f,
                "{lost} acknowledged writes could not be published and were lost; the volume is usable again"
            ),
        }
    }
}

impl std::error::Error for VfsError {}

impl From<CoreError> for VfsError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::AlreadyExists => VfsError::AlreadyExists,
            CoreError::NotFound => VfsError::NotFound,
            CoreError::NotDirectory => VfsError::NotDirectory,
            CoreError::IsDirectory => VfsError::IsDirectory,
            CoreError::DirectoryNotEmpty => VfsError::DirectoryNotEmpty,
            CoreError::InvalidMove(_)
            | CoreError::InvalidName(_)
            | CoreError::InvalidMetadata(_) => VfsError::Invalid,
            CoreError::NoSpace => VfsError::NoSpace,
            CoreError::ReadOnly => VfsError::ReadOnly,
            CoreError::Stale => VfsError::Stale,
            CoreError::WindowOpen | CoreError::WindowPoisoned | CoreError::Busy => VfsError::Busy,
            CoreError::UnsupportedGeometry(_)
            | CoreError::UnsupportedIncompatFeatures(_)
            | CoreError::ReadOnlyRequiredFeatures(_)
            | CoreError::FeatureDisabled(_)
            | CoreError::SecurityProjectionRefused => VfsError::NotSupported,
            CoreError::PrototypeLimit(detail) => VfsError::Limit(detail),
            CoreError::Block(error) => VfsError::Io(error.to_string()),
            CoreError::Format(error) => VfsError::Corrupt(error.to_string()),
            CoreError::NoValidCheckpoint { .. }
            | CoreError::AmbiguousCheckpoints(_)
            | CoreError::Corrupt(_) => VfsError::Corrupt(error.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum OpenHandle {
    File {
        object_id: ObjectId,
        access: AccessMode,
    },
    Directory {
        object_id: ObjectId,
        generation: u64,
    },
}

/// When changes reach the disk (ADR-121).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Every namespace change is durable when it returns; file data waits
    /// for fsync, the close of its last handle, or the next namespace change.
    Sync,
    /// Changes gather in the open window and are committed together when
    /// [`Vfs::commit_if_due`] finds the volume idle for `idle_ms` or the
    /// oldest change `max_age_ms` old, at the window's bound, or when
    /// anything asks for durability.
    Delayed { idle_ms: u32, max_age_ms: u32 },
}

impl Durability {
    /// ADR-121's defaults: one idle second, five seconds at most.
    pub const DELAYED: Durability = Durability::Delayed {
        idle_ms: 1_000,
        max_age_ms: 5_000,
    };
}

/// Changes a delayed window holds before it is committed whatever the
/// clock says.
pub const DELAYED_WINDOW_OPS_MAX: u32 = 512;

pub struct Vfs<D: BlockDevice> {
    volume: Volume<D>,
    handles: BTreeMap<Handle, OpenHandle>,
    /// Stored spelling of the entry each open directory handle returned last.
    /// A directory cursor is bound to one generation, so any commit anywhere
    /// on the volume invalidates it; this is the point enumeration resumes
    /// from when that happens, and it belongs to the handle rather than to
    /// an adapter, because every adapter needs it.
    directory_resume: BTreeMap<Handle, Vec<u8>>,
    next_handle: Handle,
    idle_maintenance: bool,
    inline_maintenance: bool,
    durability: Durability,
    /// Clock of the first and the latest change the open window holds, and
    /// how many it holds, as the callers stated them.
    window_first: Option<Timespec>,
    window_last: Option<Timespec>,
    window_changes: u32,
}

impl<D: BlockDevice> Vfs<D> {
    pub fn mount(device: D, options: MountOptions) -> Result<Self, VfsError> {
        let volume = mount_with_options(device, options)?;
        let mut vfs = Self::new(volume);
        if vfs.volume.mount_mode() == MountMode::ReadWrite {
            match vfs.resume_one_orphan(Timespec::default()) {
                Ok(()) | Err(VfsError::NoSpace) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(vfs)
    }

    pub fn new(volume: Volume<D>) -> Self {
        Vfs {
            volume,
            handles: BTreeMap::new(),
            directory_resume: BTreeMap::new(),
            next_handle: 1,
            idle_maintenance: true,
            inline_maintenance: true,
            durability: Durability::Sync,
            window_first: None,
            window_last: None,
            window_changes: 0,
        }
    }

    pub fn root_object(&self) -> ObjectId {
        OBJECT_ROOT
    }

    pub fn mount_mode(&self) -> MountMode {
        self.volume.mount_mode()
    }

    pub fn capabilities(&self) -> Capabilities {
        let mut bits =
            Capabilities::BASELINE.bits() | Capabilities::SYMLINKS | Capabilities::PREALLOCATE;
        if self.volume.ident().features.ro_compat & RO_COMPAT_SHARED_EXTENTS != 0 {
            bits |= Capabilities::CLONE_FILE | Capabilities::CLONE_RANGE;
        }
        if self.logged_data_fsync_enabled() {
            bits |= Capabilities::LOGGED_DATA_FSYNC;
        }
        if self.volume.ident().features.compat & COMPAT_DATA_POLICY != 0 {
            bits |= Capabilities::DATA_POLICY;
        }
        if self.volume.ident().features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0 {
            bits |= Capabilities::OPEN_UNLINKED;
        }
        bits |= Capabilities::EXTENDED_ATTRIBUTES;
        Capabilities(bits)
    }

    fn logged_data_fsync_enabled(&self) -> bool {
        self.volume.ident().features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0
    }

    /// Immediate namespace and reflink transactions cannot run beside the
    /// global data-update window. Publish that window first; this is also the
    /// bounded fallback used when an fsync group cannot fit in the log.
    fn checkpoint_data_window(&mut self, now: Timespec) -> Result<(), VfsError> {
        if self.volume.mount_mode() != MountMode::ReadWrite || !self.logged_data_fsync_enabled() {
            return Ok(());
        }
        match self.volume.window_commit(now) {
            Ok(()) => {
                self.forget_window();
                Ok(())
            }
            // The window could not be published and is now poisoned, which
            // refuses every later commit, which is what poisons it further. A
            // volume that hit one full-disk write used to answer "busy" to
            // everything for ever, including the delete that would have freed
            // the space, and could not be unmounted. Abandon the window so the
            // volume stays usable, and say how much was lost rather than
            // letting a caller believe the writes landed. They had not landed
            // either way: the window IS the work that has not reached a
            // checkpoint.
            Err(_) => {
                let lost = self.volume.window_discard();
                self.forget_window();
                Err(VfsError::WindowLost(lost))
            }
        }
    }

    fn forget_window(&mut self) {
        self.window_first = None;
        self.window_last = None;
        self.window_changes = 0;
    }

    /// The durability of this mount; [`Durability::Sync`] until set.
    pub fn durability(&self) -> Durability {
        self.durability
    }

    /// Chooses when changes reach the disk. Leaving `Delayed` commits what
    /// waits. `Delayed` needs the intent log's data updates; without them the
    /// mount stays `Sync` and this returns [`VfsError::NotSupported`].
    pub fn set_durability(&mut self, durability: Durability) -> Result<(), VfsError> {
        if let Durability::Delayed { .. } = durability {
            if !self.logged_data_fsync_enabled() {
                return Err(VfsError::NotSupported);
            }
        } else {
            self.checkpoint_data_window(Timespec::default())?;
        }
        self.durability = durability;
        Ok(())
    }

    fn delayed(&self) -> bool {
        matches!(self.durability, Durability::Delayed { .. })
            && self.volume.mount_mode() == MountMode::ReadWrite
            && self.logged_data_fsync_enabled()
    }

    /// Whether changes wait in the window, uncommitted.
    pub fn changes_pending(&self) -> bool {
        self.volume.window_open()
    }

    /// Counts one change the window now holds, made at `now`, and commits
    /// the window at its bound.
    fn note_change(&mut self, now: Timespec) -> Result<(), VfsError> {
        if !self.volume.window_open() {
            self.forget_window();
            return Ok(());
        }
        if self.window_first.is_none() {
            self.window_first = Some(now);
        }
        self.window_last = Some(now);
        self.window_changes = self.window_changes.saturating_add(1);
        if self.delayed() && self.window_changes >= DELAYED_WINDOW_OPS_MAX {
            self.commit_window(now)?;
        }
        Ok(())
    }

    fn commit_window(&mut self, now: Timespec) -> Result<(), VfsError> {
        self.checkpoint_data_window(now)?;
        self.reclaim_after_release(now);
        Ok(())
    }

    /// Commits the delayed window when `now` finds the volume idle long
    /// enough or the oldest change old enough (ADR-121). A clock that went
    /// back counts as due. Returns whether changes still wait, so a caller
    /// knows whether to ask again.
    pub fn commit_if_due(&mut self, now: Timespec) -> Result<bool, VfsError> {
        if !self.volume.window_open() {
            self.forget_window();
            return Ok(false);
        }
        let Durability::Delayed {
            idle_ms,
            max_age_ms,
        } = self.durability
        else {
            return Ok(true);
        };
        let since = |then: Option<Timespec>| -> Option<u64> {
            let then = then?;
            let elapsed = (i128::from(now.seconds) - i128::from(then.seconds)) * 1_000
                + (i128::from(now.nanoseconds) - i128::from(then.nanoseconds)) / 1_000_000;
            Some(if elapsed < 0 {
                u64::MAX
            } else {
                elapsed as u64
            })
        };
        let idle = since(self.window_last).is_none_or(|ms| ms >= u64::from(idle_ms));
        let old = since(self.window_first).is_none_or(|ms| ms >= u64::from(max_age_ms));
        if idle || old {
            self.commit_window(now)?;
        }
        Ok(self.volume.window_open())
    }

    /// Generation of the checkpoint or open data window the mount exposes.
    pub fn generation(&self) -> u64 {
        self.volume.generation()
    }

    /// Installs or removes the core flight recorder; see
    /// [`Volume::replace_flight_recorder`].
    pub fn replace_flight_recorder(
        &mut self,
        recorder: Option<FlightRecorder>,
    ) -> Option<FlightRecorder> {
        self.volume.replace_flight_recorder(recorder)
    }

    /// Runs `inspect` on the installed recorder, for counters and draining.
    pub fn with_flight_recorder<T>(
        &mut self,
        inspect: impl FnOnce(&mut FlightRecorder) -> T,
    ) -> Option<T> {
        self.volume
            .flight_recorder_mut()
            .map(|mut recorder| inspect(&mut recorder))
    }

    pub fn pending_intent_records(&self) -> u32 {
        self.volume.pending_intent_records()
    }

    pub fn pending_orphans(&mut self) -> Result<u64, VfsError> {
        Ok(self.volume.orphan_count()?)
    }

    pub fn set_orphan_cleanup_extent_budget(&mut self, extents: usize) {
        self.volume.set_orphan_cleanup_extent_budget(extents);
    }

    /// Advances at most one orphan and at most the volume's configured
    /// logical-extent budget. Adapters may call this from idle maintenance;
    /// mount and filesystem sync already invoke it once.
    /// Return retired blocks to the free pool, and say how many came back.
    ///
    /// Retiring a file's extents only moves them to the reclaim ledger. The
    /// free pool grows again when the ledger is drained, and nothing above the
    /// core drained it: a mounted volume never gave back a deleted byte, so
    /// writing and deleting the same file filled it until `rm` itself failed
    /// for want of space.
    ///
    /// Bounded twice over. It stops as soon as a step returns nothing, which in
    /// practice is after two, and never runs more than `max_steps`. What a step
    /// leaves behind is storage the older checkpoint still protects; the next
    /// transaction releases it.
    pub fn reclaim_space(&mut self, max_steps: usize, now: Timespec) -> Result<u64, VfsError> {
        if self.volume.mount_mode() != MountMode::ReadWrite {
            return Ok(0);
        }
        let mut returned = 0;
        for _ in 0..max_steps {
            if self.volume.reclaim_pending_blocks() == 0 {
                break;
            }
            let step = self.volume.reclaim_step(now)?;
            if step == 0 {
                break;
            }
            returned += step;
        }
        Ok(returned)
    }

    /// Blocks retired but not yet returned to the free pool.
    pub fn reclaim_pending_blocks(&self) -> u64 {
        self.volume.reclaim_pending_blocks()
    }

    pub fn resume_one_orphan(&mut self, now: Timespec) -> Result<(), VfsError> {
        let Some(object_id) = self.volume.first_orphan()? else {
            return Ok(());
        };
        self.volume.cleanup_orphan(object_id, now)?;
        Ok(())
    }

    pub fn identity(&self) -> VolumeIdentity {
        let ident = self.volume.ident();
        VolumeIdentity {
            uuid: ident.uuid,
            label: self.volume.volume_label().to_owned(),
            compat: ident.features.compat,
            ro_compat: ident.features.ro_compat,
            incompat: ident.features.incompat,
        }
    }

    /// The current volume label (ADR-104), not the format-time one of the
    /// identification block.
    pub fn volume_label(&self) -> &str {
        self.volume.volume_label()
    }

    /// Relabels the volume in one commit: at most 64 bytes of UTF-8 without
    /// NUL. Host naming rules beyond that belong to the adapter.
    pub fn set_volume_label(&mut self, label: &str, now: Timespec) -> Result<(), VfsError> {
        self.checkpoint_data_window(now)?;
        Ok(self.volume.set_volume_label(label)?)
    }

    pub fn statfs(&self) -> StatFs {
        let ident = self.volume.ident();
        let free = self.volume.free_blocks();
        let emergency_headroom = self.volume.emergency_headroom_blocks();
        StatFs {
            block_size: ident.block_size() as u32,
            total_blocks: ident.total_blocks,
            free_blocks: free,
            emergency_headroom_blocks: emergency_headroom,
            available_blocks: self.volume.available_blocks(),
            max_name_bytes: NAME_MAX_UTF8_BYTES as u32,
            case_sensitive: ident.name_key_algorithm != NameKeyAlgorithm::UnicodeNfcCasefold,
            unicode_version: ident.unicode_version,
        }
    }

    pub fn lookup(&mut self, parent: ObjectId, name: &str) -> Result<ObjectId, VfsError> {
        self.volume
            .lookup_in_directory(parent, name)?
            .ok_or(VfsError::NotFound)
    }

    pub fn stat(&mut self, object_id: ObjectId) -> Result<Stat, VfsError> {
        if self.volume.orphan_object(object_id)? {
            return Err(VfsError::NotFound);
        }
        self.volume
            .visible_metadata(object_id)?
            .map(Stat::from)
            .ok_or(VfsError::NotFound)
    }

    pub fn open_file(
        &mut self,
        object_id: ObjectId,
        access: AccessMode,
    ) -> Result<Handle, VfsError> {
        let stat = self.stat(object_id)?;
        if stat.kind != NodeKind::File {
            return Err(VfsError::IsDirectory);
        }
        if access.can_write() && self.volume.mount_mode() != MountMode::ReadWrite {
            return Err(VfsError::ReadOnly);
        }
        self.insert_handle(OpenHandle::File { object_id, access })
    }

    pub fn open_directory(&mut self, object_id: ObjectId) -> Result<Handle, VfsError> {
        let stat = self.stat(object_id)?;
        if stat.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        self.insert_handle(OpenHandle::Directory {
            object_id,
            generation: self.volume.generation(),
        })
    }

    fn insert_handle(&mut self, state: OpenHandle) -> Result<Handle, VfsError> {
        let handle = self.next_handle;
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .ok_or(VfsError::Limit("handle space exhausted"))?;
        self.handles.insert(handle, state);
        Ok(handle)
    }

    fn is_open(&self, object_id: ObjectId) -> bool {
        self.handles.values().any(
            |state| matches!(state, OpenHandle::File { object_id: open_id, .. } if *open_id == object_id),
        )
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), VfsError> {
        let state = self.handles.remove(&handle).ok_or(VfsError::Stale)?;
        self.directory_resume.remove(&handle);
        let OpenHandle::File { object_id, .. } = state else {
            return Ok(());
        };
        if self.handles.values().any(
            |state| matches!(state, OpenHandle::File { object_id: open_id, .. } if *open_id == object_id),
        ) {
            return Ok(());
        }
        if self.volume.mount_mode() != MountMode::ReadWrite
            || self.volume.ident().features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY == 0
        {
            return Ok(());
        }
        // A delayed mount commits on its own clock; only the last close of a
        // file already unlinked has cleanup to start.
        if self.delayed() && !self.volume.orphan_object(object_id)? {
            return Ok(());
        }
        if !self.idle_maintenance || !self.inline_maintenance {
            // Accepted writes are still published; the orphan, if this was
            // one, stays pending for a later resume.
            return self.checkpoint_data_window(Timespec::default());
        }
        self.checkpoint_data_window(Timespec::default())?;
        self.volume.cleanup_orphan(object_id, Timespec::default())?;
        Ok(())
    }

    pub fn read(
        &mut self,
        handle: Handle,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, VfsError> {
        let (object_id, access) = match self.handles.get(&handle).copied() {
            Some(OpenHandle::File { object_id, access }) => (object_id, access),
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        if !access.can_read() {
            return Err(VfsError::Invalid);
        }
        Ok(self.volume.read_file_at(object_id, offset, destination)?)
    }

    pub fn write(
        &mut self,
        handle: Handle,
        offset: u64,
        source: &[u8],
        now: Timespec,
    ) -> Result<usize, VfsError> {
        let (object_id, access) = match self.handles.get(&handle).copied() {
            Some(OpenHandle::File { object_id, access }) => (object_id, access),
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        if !access.can_write() {
            return Err(VfsError::ReadOnly);
        }
        self.keep_room(source.len() as u64, now)?;
        match self.write_at(object_id, offset, source, now) {
            // The estimate fell short: whatever is still waiting to be
            // reclaimed is reclaimed, and the write tried once more.
            Err(VfsError::NoSpace) if self.reclaim_everything(now)? => {
                self.write_at(object_id, offset, source, now)?
            }
            result => result?,
        }
        self.note_change(now)?;
        Ok(source.len())
    }

    fn write_at(
        &mut self,
        object_id: ObjectId,
        offset: u64,
        source: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError> {
        if self.logged_data_fsync_enabled() {
            match self
                .volume
                .window_write_file_at(object_id, offset, source, now)
            {
                // A file the window created, grown past what the window
                // rewrites: commit, and write it as the committed file it is.
                Err(CoreError::PrototypeLimit(_)) if self.volume.window_open() => {
                    self.checkpoint_data_window(now)?;
                    self.volume
                        .window_write_file_at(object_id, offset, source, now)?;
                }
                result => result?,
            }
        } else {
            self.volume.write_file_at(object_id, offset, source, now)?;
        }
        Ok(())
    }

    pub fn truncate(&mut self, handle: Handle, size: u64, now: Timespec) -> Result<(), VfsError> {
        let (object_id, access) = match self.handles.get(&handle).copied() {
            Some(OpenHandle::File { object_id, access }) => (object_id, access),
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        if !access.can_write() {
            return Err(VfsError::ReadOnly);
        }
        if self.logged_data_fsync_enabled() {
            match self.volume.window_truncate_file(object_id, size, now) {
                Err(CoreError::PrototypeLimit(_)) if self.volume.window_open() => {
                    self.checkpoint_data_window(now)?;
                    self.volume.window_truncate_file(object_id, size, now)?;
                }
                result => result?,
            }
            self.note_change(now)
        } else {
            Ok(self.volume.truncate_file(object_id, size, now)?)
        }
    }

    pub fn read_directory(
        &mut self,
        handle: Handle,
        cookie: u64,
        max_entries: usize,
    ) -> Result<DirectoryPage, VfsError> {
        let (object_id, generation) = match self.handles.get(&handle).copied() {
            Some(OpenHandle::Directory {
                object_id,
                generation,
            }) => (object_id, generation),
            Some(OpenHandle::File { .. }) => return Err(VfsError::NotDirectory),
            None => return Err(VfsError::Stale),
        };
        // A directory whose entries wait in the window is listed after they
        // are committed; the cursor's recovery below absorbs the new
        // generation.
        if self.volume.window_changes_directory(object_id) {
            self.commit_window(Timespec::default())?;
        }
        // Cookie zero is a rewind: the caller is asking for the directory
        // from its start, so there is nothing to resume after and a resume
        // point left from an earlier pass would silently skip the entries
        // before it.
        if cookie == 0 {
            self.directory_resume.remove(&handle);
        }
        // A cursor carries the generation it was made in, and the core
        // refuses one the volume has moved past. That is right for the
        // cursor and wrong as an answer to the caller: a host holding a
        // long-lived handle on a directory would see every commit anywhere
        // on the volume turn its next read into an error, and on macOS that
        // is the root of the mount becoming unlistable after the first
        // write. So a stale cursor is recovered here, once, by resuming
        // after the entry this handle returned last, which is a stored name
        // and not an ordinal and therefore survives the commit.
        let page = match self.volume.read_directory_page(
            object_id,
            Some(DirectoryCursor {
                generation,
                ordinal: cookie,
            }),
            max_entries,
        ) {
            Err(CoreError::Stale) => {
                let last = self.directory_resume.get(&handle).cloned();
                let ordinal = self.resume_directory_after(handle, last.as_deref())?;
                let generation = self.volume.generation();
                self.volume.read_directory_page(
                    object_id,
                    Some(DirectoryCursor {
                        generation,
                        ordinal,
                    }),
                    max_entries,
                )?
            }
            other => other?,
        };
        // The resume point is the last entry handed out, so a caller that
        // stops mid-directory and comes back after a commit continues from
        // where it stopped. An empty page leaves it alone: there was nothing
        // new to resume after.
        if let Some(last) = page.entries.last() {
            self.directory_resume.insert(handle, last.name.clone());
        }
        Ok(DirectoryPage {
            entries: page
                .entries
                .into_iter()
                .map(|entry| DirectoryEntry {
                    name: entry.name,
                    object_id: entry.child_id,
                    // Three kinds, not two. Folding everything that was not a
                    // file into a directory reported every symlink in every
                    // listing as a directory. The format admits only 1, 2 and
                    // 3 and refuses anything else when it decodes the entry,
                    // so the last arm is the directory case rather than a
                    // guess at an unknown one.
                    kind: match entry.child_type_hint {
                        1 => NodeKind::File,
                        3 => NodeKind::Symlink,
                        _ => NodeKind::Directory,
                    },
                })
                .collect(),
            next_cookie: page.next.ordinal,
            eof: page.eof,
        })
    }

    /// Rebinds a directory handle to the current generation and returns the
    /// cookie of the first entry ordered after `last_name` in the directory's
    /// comparison-key order, or of the first entry when `last_name` is `None`.
    ///
    /// An adapter whose host contract lets enumeration continue across
    /// namespace changes calls this after `read_directory` reports `Stale`.
    /// Entries ordered after `last_name` are each returned once; an entry
    /// created before that position during the enumeration is not returned.
    /// The search reads O(log n) single-entry pages and keeps no list.
    pub fn resume_directory_after(
        &mut self,
        handle: Handle,
        last_name: Option<&[u8]>,
    ) -> Result<u64, VfsError> {
        let object_id = match self.handles.get(&handle).copied() {
            Some(OpenHandle::Directory { object_id, .. }) => object_id,
            Some(OpenHandle::File { .. }) => return Err(VfsError::NotDirectory),
            None => return Err(VfsError::Stale),
        };
        if self.stat(object_id)?.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let generation = self.volume.generation();
        self.handles.insert(
            handle,
            OpenHandle::Directory {
                object_id,
                generation,
            },
        );
        let Some(last_name) = last_name else {
            return Ok(0);
        };
        let target = comparison_key(self.volume.ident(), last_name)?;
        // `low` counts entries known to order at or before the target.
        let mut low = 0u64;
        let mut step = 1u64;
        let mut high;
        loop {
            let probe = low
                .checked_add(step - 1)
                .ok_or_else(|| VfsError::Corrupt("directory ordinal overflow".into()))?;
            if self
                .directory_key_at(object_id, generation, probe)?
                .is_some_and(|key| key <= target)
            {
                low = probe + 1;
                step = step.saturating_mul(2);
            } else {
                high = probe;
                break;
            }
        }
        while low < high {
            let middle = low + (high - low) / 2;
            if self
                .directory_key_at(object_id, generation, middle)?
                .is_some_and(|key| key <= target)
            {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Ok(low)
    }

    fn directory_key_at(
        &mut self,
        object_id: ObjectId,
        generation: u64,
        ordinal: u64,
    ) -> Result<Option<Vec<u8>>, VfsError> {
        let page = self.volume.read_directory_page(
            object_id,
            Some(DirectoryCursor {
                generation,
                ordinal,
            }),
            1,
        )?;
        Ok(page.entries.into_iter().next().map(|entry| entry.key))
    }

    /// Comparison key of a name under the mounted volume's name policy. Two
    /// names address the same directory entry exactly when their keys match,
    /// so an adapter can match names that do not exist yet.
    pub fn name_key(&self, name: &str) -> Result<Vec<u8>, VfsError> {
        Ok(comparison_key(self.volume.ident(), name.as_bytes())?)
    }

    pub fn create_file(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<ObjectId, VfsError> {
        if self.delayed() {
            match self.volume.window_op(
                &BatchOp::CreateFile {
                    parent_id: parent,
                    name,
                    content: b"",
                },
                now,
            ) {
                Ok(Some(object_id)) => {
                    self.note_change(now)?;
                    return Ok(object_id);
                }
                // What the window cannot stage goes the immediate way.
                Ok(None) | Err(CoreError::PrototypeLimit(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.checkpoint_data_window(now)?;
        Ok(self
            .volume
            .create_file_in_directory(parent, name, b"", now)?)
    }

    pub fn create_symlink(
        &mut self,
        parent: ObjectId,
        name: &str,
        target: &str,
        now: Timespec,
    ) -> Result<ObjectId, VfsError> {
        self.checkpoint_data_window(now)?;
        Ok(self.volume.create_symlink(parent, name, target, now)?)
    }

    /// Exact opaque target bytes; short buffers are unchanged and return the required count.
    pub fn read_link(&mut self, object: ObjectId, output: &mut [u8]) -> Result<usize, VfsError> {
        Ok(self.volume.read_link(object, output)?)
    }

    pub fn create_directory(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<ObjectId, VfsError> {
        self.checkpoint_data_window(now)?;
        Ok(self.volume.create_directory(parent, name, now)?)
    }

    pub fn unlink_file(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
        if self.delayed() {
            let object_id = self.lookup(parent, name)?;
            let metadata = self
                .volume
                .visible_metadata(object_id)?
                .ok_or(VfsError::NotFound)?;
            // A file still open elsewhere keeps the immediate orphan path,
            // which its last close cleans.
            if metadata.object_type == ObjectType::File && !self.is_open(object_id) {
                match self.volume.window_op(
                    &BatchOp::DeleteFile {
                        parent_id: parent,
                        name,
                    },
                    now,
                ) {
                    Ok(_) => return self.note_change(now),
                    Err(CoreError::PrototypeLimit(_)) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        self.checkpoint_data_window(now)?;
        let object_id = self.lookup(parent, name)?;
        let metadata = self
            .volume
            .visible_metadata(object_id)?
            .ok_or(VfsError::NotFound)?;
        if metadata.object_type == ObjectType::Symlink {
            return Ok(self.volume.unlink_symlink(parent, name, now)?);
        }
        if metadata.object_type == ObjectType::File
            && metadata.link_count == 1
            && self.volume.ident().features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0
        {
            // Always make the visible namespace transition bounded. With no
            // live handle the orphan is eligible for idle/sync cleanup
            // immediately; with a live handle last-close starts cleanup.
            self.volume.orphan_file(parent, name, now)?;
            self.reclaim_after_release(now);
            return Ok(());
        }
        self.volume.delete_file(parent, name, now)?;
        self.reclaim_after_release(now);
        Ok(())
    }

    /// Maintenance after an operation that released a name. A failure here must
    /// not turn the caller's successful delete into a failure, and it is not
    /// lost either: both steps are traced calls, so a refusal reaches the flight
    /// recorder and the mount's diagnostics report.
    ///
    /// The order is the chain itself. Unlinking a file parks it in the orphan
    /// directory, cleaning the orphan retires its extents into the reclaim
    /// ledger, and draining the ledger returns the blocks to the free pool.
    /// Driving only the last of the three returns nothing.
    fn reclaim_after_release(&mut self, now: Timespec) {
        if !self.idle_maintenance || !self.inline_maintenance {
            return;
        }
        let _ = self.cleanup_orphans(ORPHAN_STEPS_PER_RELEASE, now);
        let _ = self.reclaim_space(RECLAIM_STEPS_PER_RELEASE, now);
    }

    /// Reclaim deleted space while it runs short, before an operation that
    /// needs `bytes` of it.
    ///
    /// A host that runs maintenance in the background returns a deleted
    /// file's blocks some time after the delete, and under steady load that
    /// time grows: three programs on one mounted volume drove its free space
    /// from four hundred megabytes to forty, and a write in the trough failed
    /// for want of space that was only waiting to be reclaimed, which also
    /// cost the write window. While deleted space is waiting, the operation
    /// that needs space keeps an eighth of the volume available, or what it
    /// needs if that is more, reclaiming one transaction at a time. Nothing
    /// else waits for it, and a volume with nothing to reclaim pays nothing.
    fn keep_room(&mut self, bytes: u64, now: Timespec) -> Result<(), VfsError> {
        let stats = self.statfs();
        let needed = bytes.div_ceil(u64::from(stats.block_size.max(1))) + ROOM_FOR_METADATA_BLOCKS;
        let low_water = needed.max(stats.total_blocks / 32);
        // A few steps toward the low-water mark per operation, so that no
        // one operation holds up the volume for long; as many as it takes
        // when the operation itself would not fit otherwise.
        let mut steps = 0;
        loop {
            let available = self.statfs().available_blocks;
            if available >= low_water || (available >= needed && steps >= ROOM_STEPS_PER_OPERATION)
            {
                break;
            }
            if !self.maintenance_step(now)? {
                break;
            }
            steps += 1;
        }
        Ok(())
    }

    /// Reclaim everything deleted, and say whether anything was.
    fn reclaim_everything(&mut self, now: Timespec) -> Result<bool, VfsError> {
        let mut reclaimed = false;
        while self.maintenance_step(now)? {
            reclaimed = true;
        }
        Ok(reclaimed)
    }

    /// Exactly one transaction of maintenance, and whether more remains.
    ///
    /// The smallest unit a host that runs maintenance in the background can
    /// hold its lock for: one orphan cleanup step if an orphan is waiting and
    /// nobody has it open, otherwise one reclaim step. Orphans first, because
    /// cleaning them is what fills the reclaim ledger. Stopped by write
    /// protection like every other maintenance.
    pub fn maintenance_step(&mut self, now: Timespec) -> Result<bool, VfsError> {
        if self.volume.mount_mode() != MountMode::ReadWrite || !self.idle_maintenance {
            return Ok(false);
        }
        if self.volume.first_orphan()?.is_none() && self.volume.reclaim_pending_blocks() == 0 {
            return Ok(false);
        }
        // Maintenance cannot run beside an open data window, and a writer
        // that makes every write durable keeps one open almost all the time:
        // maintenance was refused for as long as anybody wrote, and deleted
        // space stopped coming back. With work waiting, the window is
        // published first.
        self.checkpoint_data_window(now)?;
        if self.cleanup_orphans(1, now)? > 0 {
            return Ok(true);
        }
        Ok(self.reclaim_space(1, now)? > 0)
    }

    /// Resume the maintenance that bounded operations leave behind, and say
    /// whether anything remains to do.
    ///
    /// An unlink deliberately does not finish cleaning a large fragmented
    /// file: the work would be proportional to the file, and a delete must
    /// stay bounded. What it leaves is resumable, and something has to resume
    /// it. Nothing did on the macOS driver, where a filesystem sync only
    /// arrives at unmount, so the space of a fragmented file stayed
    /// outstanding for as long as the volume was mounted. A driver calls this
    /// on a timer; it is bounded per call and returns true while there is more.
    pub fn run_maintenance(&mut self, budget: usize, now: Timespec) -> Result<bool, VfsError> {
        if self.volume.mount_mode() != MountMode::ReadWrite || !self.idle_maintenance {
            return Ok(false);
        }
        self.cleanup_orphans(budget, now)?;
        self.reclaim_space(budget, now)?;
        Ok(self.volume.first_orphan()?.is_some() || self.volume.reclaim_pending_blocks() > 0)
    }

    /// Retire the storage of unlinked files that nobody still has open, and say
    /// how many cleanup steps ran.
    ///
    /// An orphan with a live handle is skipped: the name is gone but the file is
    /// not, and a reader holding it must keep reading its bytes. Such an orphan
    /// is cleaned at last close, which is what [`Self::close`] already does.
    /// Bounded by `max_steps`, and it stops as soon as a step makes no progress.
    pub fn cleanup_orphans(&mut self, max_steps: usize, now: Timespec) -> Result<usize, VfsError> {
        if self.volume.mount_mode() != MountMode::ReadWrite {
            return Ok(0);
        }
        let mut steps = 0;
        for _ in 0..max_steps {
            let Some(object_id) = self.volume.first_orphan()? else {
                break;
            };
            if self.object_is_open(object_id) {
                break;
            }
            let progress = self.volume.cleanup_orphan(object_id, now)?;
            steps += 1;
            if !progress.still_pending && !progress.object_removed {
                break;
            }
        }
        Ok(steps)
    }

    fn object_is_open(&self, object_id: ObjectId) -> bool {
        self.handles.values().any(|handle| match handle {
            OpenHandle::File {
                object_id: open, ..
            } => *open == object_id,
            OpenHandle::Directory {
                object_id: open, ..
            } => *open == object_id,
        })
    }

    pub fn remove_directory(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.checkpoint_data_window(now)?;
        self.volume.remove_directory(parent, name, now)?;
        self.reclaim_after_release(now);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rename(
        &mut self,
        source_parent: ObjectId,
        source_name: &str,
        target_parent: ObjectId,
        target_name: &str,
        replace: bool,
        now: Timespec,
    ) -> Result<(), VfsError> {
        if self.delayed() {
            let source = self.lookup(source_parent, source_name)?;
            let is_file = self
                .volume
                .visible_metadata(source)?
                .is_some_and(|metadata| metadata.object_type == ObjectType::File);
            let target_open = match self.lookup(target_parent, target_name) {
                Ok(target) => target != source && self.is_open(target),
                Err(VfsError::NotFound) => false,
                Err(error) => return Err(error),
            };
            if is_file && !target_open {
                match self.volume.window_op(
                    &BatchOp::Rename {
                        source_parent_id: source_parent,
                        source_name,
                        target_parent_id: target_parent,
                        target_name,
                        replace,
                    },
                    now,
                ) {
                    Ok(_) => return self.note_change(now),
                    Err(CoreError::PrototypeLimit(_)) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        self.checkpoint_data_window(now)?;
        if replace {
            let target = match self.lookup(target_parent, target_name) {
                Ok(object_id) => Some(object_id),
                Err(VfsError::NotFound) => None,
                Err(error) => return Err(error),
            };
            if let Some(target_id) = target {
                let source_id = self.lookup(source_parent, source_name)?;
                let target_metadata = self
                    .volume
                    .visible_metadata(target_id)?
                    .ok_or(VfsError::NotFound)?;
                if target_id != source_id
                    && target_metadata.link_count == 1
                    && self.volume.ident().features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0
                {
                    return Ok(self.volume.rename_replace_orphan_target(
                        source_parent,
                        source_name,
                        target_parent,
                        target_name,
                        now,
                    )?);
                }
                Ok(self.volume.rename_replace(
                    source_parent,
                    source_name,
                    target_parent,
                    target_name,
                    now,
                )?)
            } else {
                // Nothing to replace, so the plain rename is the whole job, and
                // it is the only one of the two that moves a directory or a
                // symlink: the batched form supports files only. POSIX rename
                // replaces by default, so without this every `mv` of a directory
                // took the batched path and failed. Reading the target and then
                // renaming is not a race here: one adapter holds the volume for
                // the whole operation.
                Ok(self.volume.rename(
                    source_parent,
                    source_name,
                    target_parent,
                    target_name,
                    now,
                )?)
            }
        } else {
            Ok(self
                .volume
                .rename(source_parent, source_name, target_parent, target_name, now)?)
        }
    }

    pub fn link_file(
        &mut self,
        object_id: ObjectId,
        target_parent: ObjectId,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.checkpoint_data_window(now)?;
        Ok(self
            .volume
            .link_file(object_id, target_parent, target_name, now)?)
    }

    /// Creates a distinct file object whose initial data mapping is shared
    /// with `source`.  Capability-gated by [`Capabilities::CLONE_FILE`].
    pub fn clone_file(
        &mut self,
        source: ObjectId,
        target_parent: ObjectId,
        target_name: &str,
        now: Timespec,
    ) -> Result<ObjectId, VfsError> {
        self.checkpoint_data_window(now)?;
        Ok(self
            .volume
            .clone_file(source, target_parent, target_name, now)?)
    }

    /// Reflinks a byte range between two open file handles.  Read access is
    /// required on the source and write access on the destination; the core
    /// reports alignment or implementation limits explicitly.
    #[allow(clippy::too_many_arguments)]
    pub fn clone_range(
        &mut self,
        source: Handle,
        source_offset: u64,
        destination: Handle,
        destination_offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let (source_object, source_access) = match self.handles.get(&source).copied() {
            Some(OpenHandle::File { object_id, access }) => (object_id, access),
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        if !source_access.can_read() {
            return Err(VfsError::Invalid);
        }
        let (destination_object, destination_access) = match self.handles.get(&destination).copied()
        {
            Some(OpenHandle::File { object_id, access }) => (object_id, access),
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        if !destination_access.can_write() {
            return Err(VfsError::ReadOnly);
        }
        self.checkpoint_data_window(now)?;
        Ok(self.volume.clone_range(
            source_object,
            source_offset,
            destination_object,
            destination_offset,
            length,
            now,
        )?)
    }

    /// The persistent per-file data-update policy (ADR-065): `true` when the
    /// file is opted into private in-place updates.
    /// Selects what a protection edit does to an object that carries a
    /// security descriptor. Host runtime state: the core resets it to
    /// [`SecurityProjectionPolicy::Strict`] at every mount, and an adapter
    /// that evaluates or preserves descriptors sets its own policy after
    /// mounting.
    pub fn set_security_projection_policy(&mut self, policy: SecurityProjectionPolicy) {
        self.volume.set_security_projection_policy(policy)
    }

    /// Replaces the stored protection word. The host adapter evaluates
    /// permission and owns the meaning of the bits; the change time is `now`.
    pub fn set_protection(
        &mut self,
        object_id: ObjectId,
        protection: u32,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.stat(object_id)?;
        self.checkpoint_data_window(now)?;
        Ok(self
            .volume
            .set_object_protection(object_id, protection, now)?)
    }

    /// Writes the POSIX permission bits of `mode` into the object's
    /// protection word, keeping everything the projection does not speak
    /// for: ARCHIVE, PURE, SCRIPT and every unassigned bit survive, so a
    /// `chmod` from a POSIX host cannot erase what only AmigaDOS expresses.
    ///
    /// A mode write IS a protection write, so it goes through the same
    /// projection policy as [`Vfs::set_protection`]: on a volume carrying a
    /// security descriptor, strict refuses it and preserve marks the
    /// projection as diverged. There is no second policy for POSIX.
    ///
    /// The sticky bit has no representation in the word and is refused by
    /// name rather than dropped.
    pub fn set_posix_mode(
        &mut self,
        object_id: ObjectId,
        mode: u16,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let protection = self.stat(object_id)?.protection as u32;
        // Two different refusals, so a caller learns which it hit: a mode
        // this format cannot express (sticky) is NotSupported, and a mode
        // that is not a mode is Invalid.
        let updated = posix::with_mode(protection, mode).map_err(|error| match error {
            FormatError::Invalid(reason) if reason.contains("sticky") => VfsError::NotSupported,
            _ => VfsError::Invalid,
        })?;
        self.set_protection(object_id, updated, now)
    }

    /// Sets the POSIX owner. `None` leaves that half alone, so `chown :group`
    /// and `chown user` are each one call and neither invents the other.
    pub fn set_owner(
        &mut self,
        object_id: ObjectId,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let current = self.stat(object_id)?;
        let uid = owner_uid.unwrap_or(current.owner_uid);
        let gid = owner_gid.unwrap_or(current.owner_gid);
        self.checkpoint_data_window(now)?;
        Ok(self.volume.set_object_owner(object_id, uid, gid, now)?)
    }

    /// Sets the modification time, as `utimes` does.
    ///
    /// This format keeps no access time, so there is nothing to set for one
    /// and the adapters report the modification time in its place. That is a
    /// declared property of the volume, not a write quietly dropped: a
    /// modification time a caller names IS stored, which is what `touch -t`
    /// asks for and what used to be answered with success and no change.
    pub fn set_times(
        &mut self,
        object_id: ObjectId,
        modified: Timespec,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.stat(object_id)?;
        self.checkpoint_data_window(now)?;
        Ok(self.volume.set_object_times(object_id, modified, now)?)
    }

    /// The value of the attribute `name`, or `None` when the object has no
    /// such attribute. Names carry their namespace (`user.`, `system.`,
    /// `security.`, `aros.`); which of them a caller may touch is the host
    /// adapter's policy, not this layer's.
    pub fn attribute(
        &mut self,
        object_id: ObjectId,
        name: &str,
    ) -> Result<Option<Vec<u8>>, VfsError> {
        self.stat(object_id)?;
        Ok(self.volume.attribute(object_id, name)?)
    }

    /// Every attribute name of the object, ascending by name bytes.
    pub fn attribute_names(&mut self, object_id: ObjectId) -> Result<Vec<String>, VfsError> {
        self.stat(object_id)?;
        Ok(self.volume.attribute_names(object_id)?)
    }

    /// Applies `changes` in one commit: every change or none survives a
    /// power cut. `Some(value)` writes under `mode`, `None` removes and is
    /// [`VfsError::NotFound`] for an absent attribute. A retained snapshot
    /// keeps what it captured (ADR-109); the change time is `now`. A name of
    /// [`FIELD_ATTRIBUTE_NAMES`] is [`VfsError::Invalid`].
    pub fn set_attributes(
        &mut self,
        object_id: ObjectId,
        changes: &[(&str, Option<&[u8]>)],
        mode: AttributeWriteMode,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.stat(object_id)?;
        if changes
            .iter()
            .any(|(name, _)| FIELD_ATTRIBUTE_NAMES.contains(name))
        {
            return Err(VfsError::Invalid);
        }
        self.checkpoint_data_window(now)?;
        Ok(self.volume.set_attributes(object_id, changes, mode, now)?)
    }

    /// Largest stored comment, in UTF-8 bytes.
    pub const COMMENT_MAX_BYTES: usize = afsplus_format::object::COMMENT_MAX_BYTES;

    /// The object's stored comment; empty when it has none.
    pub fn comment(&mut self, object_id: ObjectId) -> Result<String, VfsError> {
        self.stat(object_id)?;
        Ok(self.volume.object_comment(object_id)?)
    }

    /// Replaces the stored comment; an empty string removes it. A comment
    /// longer than [`Self::COMMENT_MAX_BYTES`] is [`VfsError::Limit`]. The
    /// change time is `now`; an unchanged comment writes nothing.
    pub fn set_comment(
        &mut self,
        object_id: ObjectId,
        comment: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.stat(object_id)?;
        if comment.len() > Self::COMMENT_MAX_BYTES {
            return Err(VfsError::Limit("comment exceeds the stored bound"));
        }
        self.checkpoint_data_window(now)?;
        Ok(self.volume.set_object_comment(object_id, comment, now)?)
    }

    /// Sets the modification time chosen by the caller. Creation time,
    /// protection and owner are kept and the change time is `now`.
    pub fn set_modified(
        &mut self,
        object_id: ObjectId,
        modified: Timespec,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.stat(object_id)?;
        self.checkpoint_data_window(now)?;
        let current = self.stat(object_id)?;
        let protection = u32::try_from(current.protection)
            .map_err(|_| VfsError::Corrupt("protection exceeds 32 bits".into()))?;
        Ok(self.volume.restore_object_metadata(
            object_id,
            PreservedMetadata {
                protection,
                owner_uid: current.owner_uid,
                owner_gid: current.owner_gid,
                created: current.created,
                modified,
                changed: now,
            },
        )?)
    }

    /// Reserves storage for `offset..offset + length` without changing the
    /// logical size; reserved ranges read as zeros until written. The range
    /// may be unaligned and covers every block it touches. One call edits at
    /// most `max_blocks` blocks and 64 extent records and otherwise returns
    /// `Limit` with the volume unchanged, so an adapter bounds the time one
    /// request holds its task and splits larger reservations.
    pub fn preallocate(
        &mut self,
        handle: Handle,
        offset: u64,
        length: u64,
        max_blocks: u64,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let (object_id, access) = match self.handles.get(&handle).copied() {
            Some(OpenHandle::File { object_id, access }) => (object_id, access),
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        if !access.can_write() {
            return Err(VfsError::ReadOnly);
        }
        if length == 0 || max_blocks == 0 || offset.checked_add(length).is_none() {
            return Err(VfsError::Invalid);
        }
        self.checkpoint_data_window(now)?;
        Ok(self.volume.preallocate_file_bounded(
            object_id,
            offset,
            length,
            now,
            FileEditLimits {
                max_blocks,
                max_records: 64,
            },
        )?)
    }

    /// Committed mapping of `offset..offset + length`, clipped to that range,
    /// without physical addresses: what a pager needs to plan faults and
    /// transfers (written, reserved, or hole). At most `max_ranges` (1 to 64)
    /// ranges per call, found in one tree descent wherever `offset` lies.
    ///
    /// The map describes committed state: with unpublished writes pending it
    /// is `Busy` and performs no implicit commit; the caller syncs first.
    pub fn extent_map(
        &mut self,
        handle: Handle,
        offset: u64,
        length: u64,
        max_ranges: usize,
    ) -> Result<ExtentMap, VfsError> {
        let object_id = match self.handles.get(&handle).copied() {
            Some(OpenHandle::File { object_id, .. }) => object_id,
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        let end = offset.checked_add(length).ok_or(VfsError::Invalid)?;
        if length == 0 || max_ranges == 0 || max_ranges > 64 {
            return Err(VfsError::Invalid);
        }
        let page = self
            .volume
            .file_allocation_from(object_id, offset, max_ranges)?;
        let mut ranges = Vec::with_capacity(page.ranges.len());
        for range in &page.ranges {
            if range.offset >= end {
                return Ok(ExtentMap {
                    ranges,
                    complete: true,
                    next_offset: end,
                });
            }
            let start = range.offset.max(offset);
            let range_end = range.offset.saturating_add(range.length);
            ranges.push(ExtentRange {
                offset: start,
                length: range_end.min(end) - start,
                unwritten: range.unwritten,
            });
        }
        let complete = page.eof || page.next >= end;
        Ok(ExtentMap {
            ranges,
            complete,
            next_offset: if complete { end } else { page.next },
        })
    }

    pub fn data_policy(&mut self, handle: Handle) -> Result<bool, VfsError> {
        let object_id = match self.handles.get(&handle).copied() {
            Some(OpenHandle::File { object_id, .. }) => object_id,
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        Ok(self.volume.file_data_policy(object_id)? == DataUpdatePolicy::InPlacePrivate)
    }

    /// Persistently opts the file into (or back out of) private in-place
    /// updates. Requires write access and the volume data-policy feature;
    /// the choice survives remounts (ADR-065).
    pub fn set_data_policy(
        &mut self,
        handle: Handle,
        in_place: bool,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let (object_id, access) = match self.handles.get(&handle).copied() {
            Some(OpenHandle::File { object_id, access }) => (object_id, access),
            Some(OpenHandle::Directory { .. }) => return Err(VfsError::IsDirectory),
            None => return Err(VfsError::Stale),
        };
        if !access.can_write() {
            return Err(VfsError::ReadOnly);
        }
        // Gate on the feature BEFORE flushing the data window: a refused
        // request must have zero side effects, so an unsupported call may not
        // publish or clear pending durable intent records (review #62).
        if self.volume.ident().features.compat & COMPAT_DATA_POLICY == 0 {
            return Err(VfsError::NotSupported);
        }
        self.checkpoint_data_window(now)?;
        let policy = if in_place {
            DataUpdatePolicy::InPlacePrivate
        } else {
            DataUpdatePolicy::FullCow
        };
        Ok(self.volume.set_file_data_policy(object_id, policy, now)?)
    }

    pub fn fsync(&mut self, handle: Handle) -> Result<(), VfsError> {
        if !self.handles.contains_key(&handle) {
            return Err(VfsError::Stale);
        }
        if self.volume.mount_mode() != MountMode::ReadWrite || !self.logged_data_fsync_enabled() {
            return Ok(self.volume.sync()?);
        }
        // Publishing the window needs space too; see `keep_room`. Timeless,
        // as the commit a close performs.
        self.keep_room(0, Timespec::default())?;
        match self.volume.window_fsync() {
            Ok(()) => Ok(()),
            Err(CoreError::PrototypeLimit(_)) => {
                Ok(self.volume.window_commit(Timespec::default())?)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn sync_filesystem(&mut self) -> Result<(), VfsError> {
        if self.volume.mount_mode() == MountMode::ReadWrite && self.logged_data_fsync_enabled() {
            self.volume.window_commit(Timespec::default())?;
        } else {
            self.volume.sync()?;
        }
        if self.volume.mount_mode() == MountMode::ReadWrite && self.idle_maintenance {
            self.cleanup_orphans(ORPHAN_STEPS_PER_SYNC, Timespec::default())?;
            self.reclaim_space(RECLAIM_STEPS_PER_SYNC, Timespec::default())?;
        }
        Ok(())
    }

    /// Whether `sync_filesystem` and the last `close` of an object may start
    /// orphan cleanup. On by default. An adapter turns it off while its host
    /// contract forbids new changes to the volume: a sync then only publishes
    /// writes it already accepted, and pending orphans wait. They stay
    /// visible through `pending_orphans` and are resumed once it is on again.
    /// Whether an unlink or a last close runs the maintenance it leaves
    /// behind before it returns. On by default.
    ///
    /// A host that runs maintenance itself turns it off. The macOS driver
    /// answers one request at a time, so any work a request does beyond what
    /// its caller asked for is work every other program on the volume waits
    /// behind: a file browser closing a thumbnail paid for somebody else's
    /// delete. With this off, those operations leave the orphan and the
    /// reclaim ledger for [`Self::run_maintenance`], which the driver calls
    /// from a thread of its own, one bounded step at a time.
    pub fn set_inline_maintenance(&mut self, enabled: bool) {
        self.inline_maintenance = enabled;
    }

    pub fn set_idle_maintenance(&mut self, enabled: bool) {
        self.idle_maintenance = enabled;
    }

    pub fn into_volume(self) -> Volume<D> {
        self.volume
    }
}
