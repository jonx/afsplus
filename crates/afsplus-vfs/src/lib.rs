//! Filesystem-neutral, handle-based API shared by host and MacAROS adapters.
//!
//! This layer owns no disk semantics. It translates stable API-v2 concepts
//! into [`afsplus_core::Volume`] operations and keeps OS-specific paths,
//! errno values, FUSE request types, and DOS packets outside the core.

use std::collections::BTreeMap;
use std::fmt;

use afsplus_block::BlockDevice;
use afsplus_core::volume::{DataUpdatePolicy, DirectoryCursor, ObjectMetadata, Volume};
use afsplus_core::{mount_with_options, CoreError, MountMode, MountOptions};
use afsplus_format::ident::{
    NameKeyAlgorithm, COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG_DATA_UPDATES,
    RO_COMPAT_ORPHAN_DIRECTORY, RO_COMPAT_SHARED_EXTENTS,
};
use afsplus_format::object::ObjectType;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    pub object_id: ObjectId,
    pub kind: NodeKind,
    pub size: u64,
    pub allocated_size: u64,
    pub links: u32,
    pub protection: u64,
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
            created: record.created,
            modified: record.modified,
            changed: record.changed,
            content_generation: record.content_generation,
        }
    }
}

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
            CoreError::InvalidMove(_) | CoreError::InvalidName(_) => VfsError::Invalid,
            CoreError::NoSpace => VfsError::NoSpace,
            CoreError::ReadOnly => VfsError::ReadOnly,
            CoreError::Stale => VfsError::Stale,
            CoreError::WindowOpen | CoreError::WindowPoisoned | CoreError::Busy => VfsError::Busy,
            CoreError::UnsupportedGeometry(_)
            | CoreError::UnsupportedIncompatFeatures(_)
            | CoreError::ReadOnlyRequiredFeatures(_)
            | CoreError::FeatureDisabled(_) => VfsError::NotSupported,
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

pub struct Vfs<D: BlockDevice> {
    volume: Volume<D>,
    handles: BTreeMap<Handle, OpenHandle>,
    next_handle: Handle,
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
            next_handle: 1,
        }
    }

    pub fn root_object(&self) -> ObjectId {
        OBJECT_ROOT
    }

    pub fn mount_mode(&self) -> MountMode {
        self.volume.mount_mode()
    }

    pub fn capabilities(&self) -> Capabilities {
        let mut bits = Capabilities::BASELINE.bits();
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
        Capabilities(bits)
    }

    fn logged_data_fsync_enabled(&self) -> bool {
        self.volume.ident().features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0
    }

    /// Immediate namespace and reflink transactions cannot run beside the
    /// global data-update window. Publish that window first; this is also the
    /// bounded fallback used when an fsync group cannot fit in the log.
    fn checkpoint_data_window(&mut self, now: Timespec) -> Result<(), VfsError> {
        if self.volume.mount_mode() == MountMode::ReadWrite && self.logged_data_fsync_enabled() {
            self.volume.window_commit(now)?;
        }
        Ok(())
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
    pub fn resume_one_orphan(&mut self, now: Timespec) -> Result<(), VfsError> {
        let Some(object_id) = self.volume.first_orphan()? else {
            return Ok(());
        };
        self.volume.cleanup_orphan(object_id, now)?;
        Ok(())
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

    pub fn close(&mut self, handle: Handle) -> Result<(), VfsError> {
        let state = self.handles.remove(&handle).ok_or(VfsError::Stale)?;
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
        if self.logged_data_fsync_enabled() {
            self.volume
                .window_write_file_at(object_id, offset, source, now)?;
        } else {
            self.volume.write_file_at(object_id, offset, source, now)?;
        }
        Ok(source.len())
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
            Ok(self.volume.window_truncate_file(object_id, size, now)?)
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
        let page = self.volume.read_directory_page(
            object_id,
            Some(DirectoryCursor {
                generation,
                ordinal: cookie,
            }),
            max_entries,
        )?;
        Ok(DirectoryPage {
            entries: page
                .entries
                .into_iter()
                .map(|entry| DirectoryEntry {
                    name: entry.name,
                    object_id: entry.child_id,
                    kind: if entry.child_type_hint == 1 {
                        NodeKind::File
                    } else {
                        NodeKind::Directory
                    },
                })
                .collect(),
            next_cookie: page.next.ordinal,
            eof: page.eof,
        })
    }

    pub fn create_file(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<ObjectId, VfsError> {
        self.checkpoint_data_window(now)?;
        Ok(self
            .volume
            .create_file_in_directory(parent, name, b"", now)?)
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
        self.checkpoint_data_window(now)?;
        let object_id = self.lookup(parent, name)?;
        let metadata = self
            .volume
            .visible_metadata(object_id)?
            .ok_or(VfsError::NotFound)?;
        if metadata.object_type == ObjectType::File
            && metadata.link_count == 1
            && self.volume.ident().features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0
        {
            // Always make the visible namespace transition bounded. With no
            // live handle the orphan is eligible for idle/sync cleanup
            // immediately; with a live handle last-close starts cleanup.
            self.volume.orphan_file(parent, name, now)?;
            return Ok(());
        }
        Ok(self.volume.delete_file(parent, name, now)?)
    }

    pub fn remove_directory(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.checkpoint_data_window(now)?;
        Ok(self.volume.remove_directory(parent, name, now)?)
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
            }
            Ok(self.volume.rename_replace(
                source_parent,
                source_name,
                target_parent,
                target_name,
                now,
            )?)
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
        if self.volume.mount_mode() == MountMode::ReadWrite {
            self.resume_one_orphan(Timespec::default())?;
        }
        Ok(())
    }

    pub fn into_volume(self) -> Volume<D> {
        self.volume
    }
}
