//! Filesystem-neutral, handle-based API shared by host and MacAROS adapters.
//!
//! This layer owns no disk semantics. It translates stable API-v2 concepts
//! into [`afsplus_core::Volume`] operations and keeps OS-specific paths,
//! errno values, FUSE request types, and DOS packets outside the core.

use std::collections::BTreeMap;
use std::fmt;

use afsplus_block::BlockDevice;
use afsplus_core::volume::{DirectoryCursor, Volume};
use afsplus_core::{mount_with_options, CoreError, MountMode, MountOptions};
use afsplus_format::ident::NameKeyAlgorithm;
use afsplus_format::object::{ObjectRecord, ObjectType};
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

impl From<ObjectRecord> for Stat {
    fn from(record: ObjectRecord) -> Self {
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
            CoreError::WindowOpen | CoreError::WindowPoisoned => VfsError::Busy,
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
        Ok(Self::new(volume))
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
        Capabilities::BASELINE
    }

    pub fn pending_intent_records(&self) -> u32 {
        self.volume.pending_intent_records()
    }

    pub fn statfs(&self) -> StatFs {
        let ident = self.volume.ident();
        let free = self.volume.free_blocks();
        StatFs {
            block_size: ident.block_size() as u32,
            total_blocks: ident.total_blocks,
            free_blocks: free,
            available_blocks: free,
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
        self.volume
            .stat(object_id)?
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
        self.handles
            .remove(&handle)
            .map(|_| ())
            .ok_or(VfsError::Stale)
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
        self.volume.write_file_at(object_id, offset, source, now)?;
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
        Ok(self.volume.truncate_file(object_id, size, now)?)
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
        Ok(self.volume.create_directory(parent, name, now)?)
    }

    pub fn unlink_file(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
        Ok(self.volume.delete_file(parent, name, now)?)
    }

    pub fn remove_directory(
        &mut self,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
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
        if replace {
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
        Ok(self
            .volume
            .link_file(object_id, target_parent, target_name, now)?)
    }

    pub fn fsync(&mut self, handle: Handle) -> Result<(), VfsError> {
        if !self.handles.contains_key(&handle) {
            return Err(VfsError::Stale);
        }
        Ok(self.volume.sync()?)
    }

    pub fn sync_filesystem(&mut self) -> Result<(), VfsError> {
        Ok(self.volume.sync()?)
    }

    pub fn into_volume(self) -> Volume<D> {
        self.volume
    }
}
