//! Safe, packet-neutral AROS DOS handler semantics for AFS+.
//!
//! The native handler glue owns `DosPacket`, BPTR/BSTR conversion and replies.
//! This crate owns lock/file-handle lifetime, 64-bit positions, AROS open and
//! directory-enumeration semantics, and `IoErr()` translation over the shared
//! [`afsplus_vfs::Vfs`] API.

use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_core::MountMode;
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{
    AccessMode, Capabilities, Handle, NodeKind, ObjectId, Stat, StatFs, Vfs, VfsError,
};

pub type LockId = u64;
pub type FileHandleId = u64;
pub type WatchId = u64;

pub const DISK_TYPE_AFS_PLUS: i32 = i32::from_be_bytes(*b"AFS+");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameEncoding {
    Utf8,
    Latin1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArosConfig {
    pub name_encoding: NameEncoding,
    pub volume_name: Vec<u8>,
    pub max_file_handles: usize,
    pub max_locks: usize,
    pub max_file_info_name_bytes: usize,
    /// Explicit request to let a classic protection write replace security
    /// metadata the classic view cannot express. Off by default: such a write
    /// is refused and the richer metadata is preserved.
    pub allow_security_downgrade: bool,
    /// Largest number of filesystem blocks one preallocation request edits.
    /// The handler is a single task; a larger request is `ObjectTooLarge`
    /// and the caller splits it.
    pub max_preallocate_blocks: u64,
    /// Size of the notification watch table.
    pub max_watches: usize,
}

impl Default for ArosConfig {
    fn default() -> Self {
        ArosConfig {
            name_encoding: NameEncoding::Utf8,
            volume_name: b"AFS+".to_vec(),
            max_file_handles: 1024,
            max_locks: 1024,
            // MAXFILENAMELENGTH includes the terminating NUL.
            max_file_info_name_bytes: 107,
            allow_security_downgrade: false,
            max_preallocate_blocks: 4096,
            max_watches: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockAccess {
    Shared,
    Exclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    OldFile,
    NewFile,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekMode {
    Beginning,
    Current,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum EntryType {
    File = -3,
    Root = 1,
    Directory = 2,
    SoftLink = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub disk_key: u64,
    pub directory_entry_type: EntryType,
    pub entry_type: EntryType,
    pub name: Vec<u8>,
    pub protection: u32,
    pub size: u64,
    pub blocks: u64,
    pub modified: Timespec,
    pub object_id: ObjectId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskInfo {
    pub write_protected: bool,
    pub total_blocks: u64,
    pub used_blocks: u64,
    pub bytes_per_block: u32,
    pub disk_type: i32,
    pub in_use: bool,
}

/// Highest `afsplus_access_hint_t` value (`IMMUTABLE_EXPECTED`).
pub const ACCESS_HINT_LAST: u32 = 11;

/// What an access-intent hint changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AdviceEffect {
    /// Admitted and recorded nowhere: behavior is unchanged.
    None = 0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumePolicy {
    pub capabilities: Capabilities,
    pub statfs: StatFs,
    pub mount_mode: MountMode,
    pub pending_intent_records: u32,
}

/// AROS DOS secondary result values used by the Alpha-0 packet bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ArosError {
    Unknown = 100,
    NoFreeStore = 103,
    BadNumber = 115,
    ObjectInUse = 202,
    ObjectExists = 203,
    DirectoryNotFound = 204,
    ObjectNotFound = 205,
    ObjectTooLarge = 207,
    ActionNotKnown = 209,
    InvalidComponentName = 210,
    InvalidLock = 211,
    ObjectWrongType = 212,
    DiskWriteProtected = 214,
    DirectoryNotEmpty = 216,
    SeekError = 219,
    DiskFull = 221,
    WriteProtected = 223,
    NotDosDisk = 225,
    NoMoreEntries = 232,
    IsSoftLink = 233,
}

impl ArosError {
    pub fn io_error(self) -> i32 {
        self as i32
    }
}

impl From<VfsError> for ArosError {
    fn from(error: VfsError) -> Self {
        match error {
            VfsError::NotFound => ArosError::ObjectNotFound,
            VfsError::AlreadyExists => ArosError::ObjectExists,
            VfsError::NotDirectory | VfsError::IsDirectory => ArosError::ObjectWrongType,
            VfsError::DirectoryNotEmpty => ArosError::DirectoryNotEmpty,
            VfsError::Invalid => ArosError::InvalidComponentName,
            VfsError::ReadOnly => ArosError::DiskWriteProtected,
            VfsError::NoSpace => ArosError::DiskFull,
            VfsError::Stale => ArosError::InvalidLock,
            VfsError::Busy => ArosError::ObjectInUse,
            VfsError::NotSupported => ArosError::ActionNotKnown,
            VfsError::Corrupt(_) => ArosError::NotDosDisk,
            VfsError::Io(_) => ArosError::Unknown,
            VfsError::Limit(_) => ArosError::ObjectTooLarge,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LockCounts {
    shared: usize,
    exclusive: bool,
}

#[derive(Debug, Clone)]
struct LockState {
    object_id: ObjectId,
    parent: Option<ObjectId>,
    name: Vec<u8>,
    access: LockAccess,
    directory_handle: Option<Handle>,
    next_cookie: u64,
    /// Stored spelling of the entry `ExNext` returned last, the resume point
    /// when a commit invalidates the generation-bound cookie.
    last_entry: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct FileState {
    vfs_handle: Handle,
    object_id: ObjectId,
    parent: ObjectId,
    name: Vec<u8>,
    position: u64,
    access: LockAccess,
    /// Written or resized through this handle; DOS notifies at close.
    dirty: bool,
}

/// One notification request. DOS watches names, including names that do not
/// exist yet, so a watch is a parent directory plus a comparison key. A watch
/// on a directory also fires when an entry inside it changes.
#[derive(Debug, Clone)]
struct Watch {
    parent: ObjectId,
    key: Vec<u8>,
    /// Object the name resolved to when last seen, for directory watches.
    object: Option<ObjectId>,
    /// Coalesced: any number of changes since the last drain is one event.
    pending: bool,
}

/// Answers whether an object carries security metadata that the classic
/// single-user projection (owner plus DOS protection bits) cannot express.
///
/// The classic adapter never interprets such metadata. It only needs this
/// one fact to keep its rule: never silently rewrite richer security into a
/// weaker representation.
pub trait RichSecurityProbe {
    fn carries_rich_security(&mut self, object_id: ObjectId) -> Result<bool, ArosError>;
}

/// The executable format stores protection bits only, so no object carries
/// richer metadata.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProtectionBitsOnly;

impl RichSecurityProbe for ProtectionBitsOnly {
    fn carries_rich_security(&mut self, _object_id: ObjectId) -> Result<bool, ArosError> {
        Ok(false)
    }
}

pub struct ArosAdapter<D: BlockDevice> {
    security: Box<dyn RichSecurityProbe>,
    vfs: Vfs<D>,
    config: ArosConfig,
    locks: BTreeMap<LockId, LockState>,
    lock_counts: BTreeMap<ObjectId, LockCounts>,
    known_parents: BTreeMap<ObjectId, (Option<ObjectId>, Vec<u8>)>,
    files: BTreeMap<FileHandleId, FileState>,
    watches: BTreeMap<WatchId, Watch>,
    next_lock: LockId,
    next_file: FileHandleId,
    next_watch: WatchId,
}

impl<D: BlockDevice> ArosAdapter<D> {
    pub fn new(vfs: Vfs<D>, config: ArosConfig) -> Self {
        let mut known_parents = BTreeMap::new();
        known_parents.insert(OBJECT_ROOT, (None, config.volume_name.clone()));
        ArosAdapter {
            security: Box::new(ProtectionBitsOnly),
            vfs,
            config,
            locks: BTreeMap::new(),
            lock_counts: BTreeMap::new(),
            known_parents,
            files: BTreeMap::new(),
            watches: BTreeMap::new(),
            next_lock: 1,
            next_file: 1,
            next_watch: 1,
        }
    }

    pub fn root_object(&self) -> ObjectId {
        OBJECT_ROOT
    }

    /// Installs the volume's answer to "does this object carry security
    /// metadata beyond the classic bits". The default answers no.
    pub fn set_security_probe(&mut self, probe: Box<dyn RichSecurityProbe>) {
        self.security = probe;
    }

    pub fn locate(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        access: LockAccess,
    ) -> Result<LockId, ArosError> {
        self.ensure_lock_capacity()?;
        let base_object = self.lock_object_or_root(base)?;
        let (object_id, parent, stored_name) = if name.is_empty() {
            let parent = base.and_then(|id| self.locks.get(&id).and_then(|lock| lock.parent));
            let stored_name = base
                .and_then(|id| self.locks.get(&id).map(|lock| lock.name.clone()))
                .unwrap_or_else(|| self.config.volume_name.clone());
            (base_object, parent, stored_name)
        } else {
            let decoded = self.decode_component(name)?;
            let object_id = self.vfs.lookup(base_object, &decoded)?;
            // dos.library resolves the link through ACTION_READ_LINK and
            // retries with the substituted path.
            if self.vfs.stat(object_id)?.kind == NodeKind::Symlink {
                return Err(ArosError::IsSoftLink);
            }
            self.known_parents
                .insert(object_id, (Some(base_object), name.to_vec()));
            (object_id, Some(base_object), name.to_vec())
        };
        self.insert_lock(object_id, parent, stored_name, access)
    }

    pub fn duplicate_lock(&mut self, lock: LockId) -> Result<LockId, ArosError> {
        self.ensure_lock_capacity()?;
        let state = self.lock_state(lock)?.clone();
        self.insert_lock(state.object_id, state.parent, state.name, state.access)
    }

    pub fn parent_lock(&mut self, lock: LockId) -> Result<Option<LockId>, ArosError> {
        self.parent_lock_with_access(lock, LockAccess::Shared)
    }

    pub fn parent_lock_with_access(
        &mut self,
        lock: LockId,
        access: LockAccess,
    ) -> Result<Option<LockId>, ArosError> {
        let parent = self.lock_state(lock)?.parent;
        let Some(parent) = parent else {
            return Ok(None);
        };
        self.ensure_lock_capacity()?;
        let (grandparent, name) = self
            .known_parents
            .get(&parent)
            .cloned()
            .ok_or(ArosError::InvalidLock)?;
        self.insert_lock(parent, grandparent, name, access)
            .map(Some)
    }

    pub fn same_lock(
        &self,
        first: Option<LockId>,
        second: Option<LockId>,
    ) -> Result<bool, ArosError> {
        Ok(self.lock_object_or_root(first)? == self.lock_object_or_root(second)?)
    }

    pub fn free_lock(&mut self, lock: LockId) -> Result<(), ArosError> {
        let mut state = self.locks.remove(&lock).ok_or(ArosError::InvalidLock)?;
        if let Some(handle) = state.directory_handle.take() {
            self.vfs.close(handle)?;
        }
        self.release_object_lock(state.object_id, state.access);
        Ok(())
    }

    pub fn open(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        mode: OpenMode,
        now: Timespec,
    ) -> Result<FileHandleId, ArosError> {
        self.ensure_file_capacity()?;
        let parent = self.lock_object_or_root(base)?;
        let decoded = self.decode_component(name)?;
        let existing = self.vfs.lookup(parent, &decoded);
        let object_id = match (mode, existing) {
            (OpenMode::OldFile, Ok(object_id))
            | (OpenMode::ReadWrite, Ok(object_id))
            | (OpenMode::NewFile, Ok(object_id)) => object_id,
            (OpenMode::OldFile, Err(error)) => return Err(error.into()),
            (OpenMode::ReadWrite | OpenMode::NewFile, Err(VfsError::NotFound)) => {
                let created = self.vfs.create_file(parent, &decoded, now)?;
                self.touch(parent, &decoded);
                created
            }
            (_, Err(error)) => return Err(error.into()),
        };
        match self.vfs.stat(object_id)?.kind {
            NodeKind::File => {}
            NodeKind::Symlink => return Err(ArosError::IsSoftLink),
            _ => return Err(ArosError::ObjectWrongType),
        }
        // MODE_NEWFILE holds the object exclusively; MODE_OLDFILE and
        // MODE_READWRITE share it, exactly like the corresponding lock.
        let lock_access = if mode == OpenMode::NewFile {
            LockAccess::Exclusive
        } else {
            LockAccess::Shared
        };
        self.acquire_object_lock(object_id, lock_access)?;
        match self.open_locked(object_id, parent, name, mode, lock_access, now) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                self.release_object_lock(object_id, lock_access);
                Err(error)
            }
        }
    }

    fn open_locked(
        &mut self,
        object_id: ObjectId,
        parent: ObjectId,
        name: &[u8],
        mode: OpenMode,
        lock_access: LockAccess,
        now: Timespec,
    ) -> Result<FileHandleId, ArosError> {
        let access = if mode == OpenMode::OldFile {
            AccessMode::ReadOnly
        } else {
            AccessMode::ReadWrite
        };
        let vfs_handle = self.vfs.open_file(object_id, access)?;
        if mode == OpenMode::NewFile {
            if let Err(error) = self.vfs.truncate(vfs_handle, 0, now) {
                let _ = self.vfs.close(vfs_handle);
                return Err(error.into());
            }
        }
        let handle = match self.allocate_file_id() {
            Ok(handle) => handle,
            Err(error) => {
                let _ = self.vfs.close(vfs_handle);
                return Err(error);
            }
        };
        self.files.insert(
            handle,
            FileState {
                vfs_handle,
                object_id,
                parent,
                name: name.to_vec(),
                position: 0,
                access: lock_access,
                dirty: mode == OpenMode::NewFile,
            },
        );
        Ok(handle)
    }

    pub fn parent_of_file(&mut self, handle: FileHandleId) -> Result<LockId, ArosError> {
        self.ensure_lock_capacity()?;
        let parent = self.file_state(handle)?.parent;
        let (grandparent, name) = self
            .known_parents
            .get(&parent)
            .cloned()
            .ok_or(ArosError::InvalidLock)?;
        self.insert_lock(parent, grandparent, name, LockAccess::Shared)
    }

    pub fn lock_from_file(&mut self, handle: FileHandleId) -> Result<LockId, ArosError> {
        self.ensure_lock_capacity()?;
        let state = self.file_state(handle)?.clone();
        self.insert_lock(
            state.object_id,
            Some(state.parent),
            state.name,
            LockAccess::Shared,
        )
    }

    pub fn close(&mut self, handle: FileHandleId) -> Result<(), ArosError> {
        let state = self.files.remove(&handle).ok_or(ArosError::InvalidLock)?;
        self.release_object_lock(state.object_id, state.access);
        if state.dirty {
            self.touch_raw(state.parent, &state.name);
        }
        Ok(self.vfs.close(state.vfs_handle)?)
    }

    pub fn read(
        &mut self,
        handle: FileHandleId,
        destination: &mut [u8],
    ) -> Result<usize, ArosError> {
        let (vfs_handle, position) = {
            let state = self.file_state(handle)?;
            (state.vfs_handle, state.position)
        };
        let count = self.vfs.read(vfs_handle, position, destination)?;
        self.files
            .get_mut(&handle)
            .expect("validated handle")
            .position = position
            .checked_add(count as u64)
            .ok_or(ArosError::SeekError)?;
        Ok(count)
    }

    pub fn write(
        &mut self,
        handle: FileHandleId,
        source: &[u8],
        now: Timespec,
    ) -> Result<usize, ArosError> {
        let (vfs_handle, position) = {
            let state = self.file_state(handle)?;
            (state.vfs_handle, state.position)
        };
        let count = self.vfs.write(vfs_handle, position, source, now)?;
        let state = self.files.get_mut(&handle).expect("validated handle");
        state.dirty = true;
        state.position = position
            .checked_add(count as u64)
            .ok_or(ArosError::SeekError)?;
        Ok(count)
    }

    pub fn seek(
        &mut self,
        handle: FileHandleId,
        offset: i64,
        mode: SeekMode,
    ) -> Result<u64, ArosError> {
        let state = self.file_state(handle)?;
        let old = state.position;
        let base = match mode {
            SeekMode::Beginning => 0,
            SeekMode::Current => old,
            SeekMode::End => self.vfs.stat(state.object_id)?.size,
        };
        let position = add_signed(base, offset)?;
        self.files
            .get_mut(&handle)
            .expect("validated handle")
            .position = position;
        Ok(old)
    }

    pub fn file_position(&self, handle: FileHandleId) -> Result<u64, ArosError> {
        Ok(self.file_state(handle)?.position)
    }

    pub fn file_size(&mut self, handle: FileHandleId) -> Result<u64, ArosError> {
        let object_id = self.file_state(handle)?.object_id;
        Ok(self.vfs.stat(object_id)?.size)
    }

    pub fn set_file_size(
        &mut self,
        handle: FileHandleId,
        offset: i64,
        mode: SeekMode,
        now: Timespec,
    ) -> Result<u64, ArosError> {
        let (vfs_handle, object_id, position) = {
            let state = self.file_state(handle)?;
            (state.vfs_handle, state.object_id, state.position)
        };
        let base = match mode {
            SeekMode::Beginning => 0,
            SeekMode::Current => position,
            SeekMode::End => self.vfs.stat(object_id)?.size,
        };
        let size = add_signed(base, offset)?;
        self.vfs.truncate(vfs_handle, size, now)?;
        self.files.get_mut(&handle).expect("validated handle").dirty = true;
        Ok(size)
    }

    /// Positioned 64-bit read. The DOS file position is neither used nor
    /// moved, so v2 callers and classic `Read` calls share one handle.
    pub fn read_at(
        &mut self,
        handle: FileHandleId,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, ArosError> {
        let vfs_handle = self.file_state(handle)?.vfs_handle;
        Ok(self.vfs.read(vfs_handle, offset, destination)?)
    }

    /// Positioned 64-bit write; see [`Self::read_at`].
    pub fn write_at(
        &mut self,
        handle: FileHandleId,
        offset: u64,
        source: &[u8],
        now: Timespec,
    ) -> Result<usize, ArosError> {
        let vfs_handle = self.file_state(handle)?.vfs_handle;
        let count = self.vfs.write(vfs_handle, offset, source, now)?;
        self.files.get_mut(&handle).expect("validated handle").dirty = true;
        Ok(count)
    }

    /// `CloneFile`: a new file `name` that initially shares the source's
    /// storage. `NotSupported` maps to `ERROR_ACTION_NOT_KNOWN`, the signal
    /// for a caller to fall back to a byte copy.
    pub fn clone_file(
        &mut self,
        source: LockId,
        base: Option<LockId>,
        name: &[u8],
        now: Timespec,
    ) -> Result<(), ArosError> {
        let source_object = self.lock_state(source)?.object_id;
        let parent = self.lock_object_or_root(base)?;
        let decoded = self.decode_component(name)?;
        self.vfs.clone_file(source_object, parent, &decoded, now)?;
        self.touch(parent, &decoded);
        Ok(())
    }

    /// `CloneRange` between two open files.
    pub fn clone_range(
        &mut self,
        source: FileHandleId,
        source_offset: u64,
        destination: FileHandleId,
        destination_offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), ArosError> {
        let source_handle = self.file_state(source)?.vfs_handle;
        let destination_handle = self.file_state(destination)?.vfs_handle;
        self.vfs.clone_range(
            source_handle,
            source_offset,
            destination_handle,
            destination_offset,
            length,
            now,
        )?;
        self.files
            .get_mut(&destination)
            .expect("validated handle")
            .dirty = true;
        Ok(())
    }

    /// Reserves storage for a byte range without changing the file size.
    pub fn preallocate(
        &mut self,
        handle: FileHandleId,
        offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), ArosError> {
        let vfs_handle = self.file_state(handle)?.vfs_handle;
        Ok(self.vfs.preallocate(
            vfs_handle,
            offset,
            length,
            self.config.max_preallocate_blocks,
            now,
        )?)
    }

    /// Access-intent hint for a byte range. Every hint of
    /// `api/performance_hints.h` is admitted and reports what it changed, so
    /// a caller never assumes an effect. No hint alters durability, contents
    /// or allocation; the single-task handler has no read-ahead to steer, so
    /// each one reports [`AdviceEffect::None`].
    pub fn advise(
        &mut self,
        handle: FileHandleId,
        offset: u64,
        length: u64,
        hint: u32,
    ) -> Result<AdviceEffect, ArosError> {
        self.file_state(handle)?;
        if hint > ACCESS_HINT_LAST || offset.checked_add(length).is_none() {
            return Err(ArosError::BadNumber);
        }
        Ok(AdviceEffect::None)
    }

    /// Atomic replace: `target` names the source object afterwards and a
    /// crash leaves either the old or the new target, never neither. A
    /// target that a lock or handle holds is not replaced.
    pub fn replace(
        &mut self,
        source_base: Option<LockId>,
        source_name: &[u8],
        target_base: Option<LockId>,
        target_name: &[u8],
        now: Timespec,
    ) -> Result<(), ArosError> {
        let source_parent = self.lock_object_or_root(source_base)?;
        let target_parent = self.lock_object_or_root(target_base)?;
        let source = self.decode_component(source_name)?;
        let target = self.decode_component(target_name)?;
        let object_id = self.vfs.lookup(source_parent, &source)?;
        match self.vfs.lookup(target_parent, &target) {
            Ok(existing) if self.lock_counts.contains_key(&existing) => {
                return Err(ArosError::ObjectInUse);
            }
            Ok(_) | Err(VfsError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        self.vfs
            .rename(source_parent, &source, target_parent, &target, true, now)?;
        self.touch(source_parent, &source);
        self.touch(target_parent, &target);
        self.known_parents
            .insert(object_id, (Some(target_parent), target_name.to_vec()));
        Ok(())
    }

    pub fn fsync(&mut self, handle: FileHandleId) -> Result<(), ArosError> {
        let vfs_handle = self.file_state(handle)?.vfs_handle;
        Ok(self.vfs.fsync(vfs_handle)?)
    }

    pub fn flush(&mut self) -> Result<(), ArosError> {
        Ok(self.vfs.sync_filesystem()?)
    }

    pub fn create_directory(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        now: Timespec,
    ) -> Result<LockId, ArosError> {
        self.ensure_lock_capacity()?;
        let parent = self.lock_object_or_root(base)?;
        let decoded = self.decode_component(name)?;
        let object_id = self.vfs.create_directory(parent, &decoded, now)?;
        self.touch(parent, &decoded);
        self.known_parents
            .insert(object_id, (Some(parent), name.to_vec()));
        self.insert_lock(object_id, Some(parent), name.to_vec(), LockAccess::Shared)
    }

    pub fn delete_object(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        now: Timespec,
    ) -> Result<(), ArosError> {
        let parent = self.lock_object_or_root(base)?;
        let decoded = self.decode_component(name)?;
        let object = self.vfs.lookup(parent, &decoded)?;
        // DOS refuses to delete an object that a lock or file handle holds.
        if self.lock_counts.contains_key(&object) {
            return Err(ArosError::ObjectInUse);
        }
        match self.vfs.stat(object)?.kind {
            // The link itself is deleted, never its target.
            NodeKind::File | NodeKind::Symlink => self.vfs.unlink_file(parent, &decoded, now)?,
            NodeKind::Directory => self.vfs.remove_directory(parent, &decoded, now)?,
            NodeKind::Internal => return Err(ArosError::ObjectWrongType),
        }
        self.touch(parent, &decoded);
        Ok(())
    }

    pub fn rename(
        &mut self,
        source_base: Option<LockId>,
        source_name: &[u8],
        target_base: Option<LockId>,
        target_name: &[u8],
        now: Timespec,
    ) -> Result<(), ArosError> {
        let source_parent = self.lock_object_or_root(source_base)?;
        let target_parent = self.lock_object_or_root(target_base)?;
        let source = self.decode_component(source_name)?;
        let target = self.decode_component(target_name)?;
        let object_id = self.vfs.lookup(source_parent, &source)?;
        self.vfs
            .rename(source_parent, &source, target_parent, &target, false, now)?;
        self.touch(source_parent, &source);
        self.touch(target_parent, &target);
        self.known_parents
            .insert(object_id, (Some(target_parent), target_name.to_vec()));
        Ok(())
    }

    pub fn make_hard_link(
        &mut self,
        target_base: Option<LockId>,
        target_name: &[u8],
        source: LockId,
        now: Timespec,
    ) -> Result<(), ArosError> {
        let target_parent = self.lock_object_or_root(target_base)?;
        let source_object = self.lock_state(source)?.object_id;
        if self.vfs.stat(source_object)?.kind != NodeKind::File {
            return Err(ArosError::ObjectWrongType);
        }
        let target = self.decode_component(target_name)?;
        self.vfs
            .link_file(source_object, target_parent, &target, now)?;
        self.touch(target_parent, &target);
        Ok(())
    }

    /// `ACTION_SET_PROTECT`. An empty name addresses the base object. The
    /// 32-bit DOS protection word is stored as given; its inverted RWED sense
    /// is a DOS convention that the format does not reinterpret.
    pub fn set_protection(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        protection: u32,
        now: Timespec,
    ) -> Result<(), ArosError> {
        let object = self.named_object(base, name)?;
        // Classic single-user profile: the session acts as the owner and the
        // DOS bits are a projection. A write through that projection must not
        // destroy what it cannot see.
        if !self.config.allow_security_downgrade && self.security.carries_rich_security(object)? {
            return Err(ArosError::WriteProtected);
        }
        self.vfs.set_protection(object, protection, now)?;
        self.touch_named(base, name);
        Ok(())
    }

    /// `ACTION_SET_DATE`.
    pub fn set_modified(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        modified: Timespec,
        now: Timespec,
    ) -> Result<(), ArosError> {
        let object = self.named_object(base, name)?;
        self.vfs.set_modified(object, modified, now)?;
        self.touch_named(base, name);
        Ok(())
    }

    /// `ACTION_MAKE_LINK` with `LINK_SOFT`. The target is an opaque DOS path
    /// in the mount's name encoding and is stored without resolution.
    pub fn make_soft_link(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        target: &[u8],
        now: Timespec,
    ) -> Result<(), ArosError> {
        let parent = self.lock_object_or_root(base)?;
        let decoded = self.decode_component(name)?;
        let target = self.decode_text(target)?;
        if target.is_empty() {
            return Err(ArosError::InvalidComponentName);
        }
        self.vfs.create_symlink(parent, &decoded, &target, now)?;
        self.touch(parent, &decoded);
        Ok(())
    }

    /// Target bytes of the soft link `name` in the mount's name encoding.
    /// Returns the required byte count; a short buffer is left unchanged.
    pub fn read_soft_link(
        &mut self,
        base: Option<LockId>,
        name: &[u8],
        output: &mut [u8],
    ) -> Result<usize, ArosError> {
        let parent = self.lock_object_or_root(base)?;
        let decoded = self.decode_component(name)?;
        let object = self.vfs.lookup(parent, &decoded)?;
        if self.vfs.stat(object)?.kind != NodeKind::Symlink {
            return Err(ArosError::ObjectWrongType);
        }
        let mut stored = vec![0u8; self.vfs.read_link(object, &mut [])?];
        let length = self.vfs.read_link(object, &mut stored)?;
        let encoded = self.encode_text(&stored[..length])?;
        if encoded.len() <= output.len() {
            output[..encoded.len()].copy_from_slice(&encoded);
        }
        Ok(encoded.len())
    }

    fn named_object(&mut self, base: Option<LockId>, name: &[u8]) -> Result<ObjectId, ArosError> {
        let base_object = self.lock_object_or_root(base)?;
        if name.is_empty() {
            return Ok(base_object);
        }
        let decoded = self.decode_component(name)?;
        Ok(self.vfs.lookup(base_object, &decoded)?)
    }

    /// `ACTION_ADD_NOTIFY`: watches the name `name` under `base`, which need
    /// not exist. The table is bounded by `max_watches`.
    pub fn add_watch(&mut self, base: Option<LockId>, name: &[u8]) -> Result<WatchId, ArosError> {
        if self.watches.len() >= self.config.max_watches {
            return Err(ArosError::NoFreeStore);
        }
        let (parent, key, object) = if name.is_empty() {
            // The base object itself, watched under its own name.
            let object = self.lock_object_or_root(base)?;
            let (parent, stored) = self
                .known_parents
                .get(&object)
                .cloned()
                .ok_or(ArosError::InvalidLock)?;
            match parent {
                Some(parent) => {
                    let decoded = self.decode_component(&stored)?;
                    (parent, self.vfs.name_key(&decoded)?, Some(object))
                }
                // The root has no parent entry: only changes inside fire.
                None => (OBJECT_ROOT, Vec::new(), Some(OBJECT_ROOT)),
            }
        } else {
            let parent = self.lock_object_or_root(base)?;
            let decoded = self.decode_component(name)?;
            let object = match self.vfs.lookup(parent, &decoded) {
                Ok(object) => Some(object),
                Err(VfsError::NotFound) => None,
                Err(error) => return Err(error.into()),
            };
            (parent, self.vfs.name_key(&decoded)?, object)
        };
        let watch = self.next_watch;
        self.next_watch = self
            .next_watch
            .checked_add(1)
            .ok_or(ArosError::NoFreeStore)?;
        self.watches.insert(
            watch,
            Watch {
                parent,
                key,
                object,
                pending: false,
            },
        );
        Ok(watch)
    }

    /// `ACTION_REMOVE_NOTIFY`. A pending event of the watch is discarded.
    pub fn remove_watch(&mut self, watch: WatchId) -> Result<(), ArosError> {
        self.watches
            .remove(&watch)
            .map(|_| ())
            .ok_or(ArosError::ObjectNotFound)
    }

    /// Moves pending watch identifiers into `output`, lowest first, and
    /// returns how many were written. Watches that did not fit stay pending,
    /// so no event is lost and the table never grows with the change rate.
    pub fn drain_watches(&mut self, output: &mut [WatchId]) -> usize {
        let mut count = 0;
        for (id, watch) in &mut self.watches {
            if count == output.len() {
                break;
            }
            if watch.pending {
                watch.pending = false;
                output[count] = *id;
                count += 1;
            }
        }
        count
    }

    fn touch(&mut self, parent: ObjectId, decoded_name: &str) {
        let Ok(key) = self.vfs.name_key(decoded_name) else {
            return;
        };
        let object = self.vfs.lookup(parent, decoded_name).ok();
        for watch in self.watches.values_mut() {
            let named = watch.parent == parent && watch.key == key && !watch.key.is_empty();
            if named {
                watch.object = object;
            }
            if named || watch.object == Some(parent) {
                watch.pending = true;
            }
        }
    }

    fn touch_raw(&mut self, parent: ObjectId, raw_name: &[u8]) {
        if let Ok(decoded) = self.decode_component(raw_name) {
            self.touch(parent, &decoded);
        }
    }

    fn touch_named(&mut self, base: Option<LockId>, name: &[u8]) {
        if !name.is_empty() {
            if let Ok(parent) = self.lock_object_or_root(base) {
                self.touch_raw(parent, name);
            }
            return;
        }
        let Some(lock) = base.and_then(|id| self.locks.get(&id)) else {
            return;
        };
        if let Some(parent) = lock.parent {
            let stored = lock.name.clone();
            self.touch_raw(parent, &stored);
        }
    }

    pub fn examine_lock(&mut self, lock: LockId) -> Result<FileInfo, ArosError> {
        let state = self.lock_state(lock)?.clone();
        let stat = self.vfs.stat(state.object_id)?;
        self.file_info(stat, state.name, 0)
    }

    pub fn examine_file(&mut self, handle: FileHandleId) -> Result<FileInfo, ArosError> {
        let state = self.file_state(handle)?.clone();
        let stat = self.vfs.stat(state.object_id)?;
        self.file_info(stat, state.name, 0)
    }

    pub fn examine_next(&mut self, lock: LockId) -> Result<FileInfo, ArosError> {
        let (object_id, directory_handle, cookie) = {
            let state = self.lock_state(lock)?;
            (state.object_id, state.directory_handle, state.next_cookie)
        };
        let handle = match directory_handle {
            Some(handle) => handle,
            None => {
                let handle = self.vfs.open_directory(object_id)?;
                self.locks
                    .get_mut(&lock)
                    .expect("validated lock")
                    .directory_handle = Some(handle);
                handle
            }
        };
        // DOS lets a program create, delete and rename between ExNext calls
        // (`Delete #?`). The VFS cookie is bound to one generation, so a
        // stale cookie resumes after the entry returned last.
        let page = match self.vfs.read_directory(handle, cookie, 1) {
            Err(VfsError::Stale) => {
                let last = self.lock_state(lock)?.last_entry.clone();
                let cookie = self.vfs.resume_directory_after(handle, last.as_deref())?;
                self.vfs.read_directory(handle, cookie, 1)?
            }
            other => other?,
        };
        let Some(entry) = page.entries.into_iter().next() else {
            return Err(ArosError::NoMoreEntries);
        };
        let state = self.locks.get_mut(&lock).expect("validated lock");
        state.next_cookie = page.next_cookie;
        state.last_entry = Some(entry.name.clone());
        let stat = self.vfs.stat(entry.object_id)?;
        let name = self.encode_name(&entry.name)?;
        self.known_parents
            .insert(entry.object_id, (Some(object_id), name.clone()));
        self.file_info(stat, name, page.next_cookie)
    }

    pub fn rewind_directory(&mut self, lock: LockId) -> Result<(), ArosError> {
        let handle = self.lock_state(lock)?.directory_handle;
        if let Some(handle) = handle {
            self.vfs.close(handle)?;
        }
        let state = self.locks.get_mut(&lock).expect("validated lock");
        state.directory_handle = None;
        state.next_cookie = 0;
        state.last_entry = None;
        Ok(())
    }

    pub fn disk_info(&self) -> DiskInfo {
        let stat = self.vfs.statfs();
        DiskInfo {
            write_protected: self.vfs.mount_mode() != MountMode::ReadWrite,
            total_blocks: stat.total_blocks,
            // Classic DOS exposes total/used rather than a separate
            // privileged raw-free counter. Count emergency headroom as used
            // so applications see only normally allocatable capacity.
            used_blocks: stat.total_blocks.saturating_sub(stat.available_blocks),
            bytes_per_block: stat.block_size,
            disk_type: DISK_TYPE_AFS_PLUS,
            in_use: !self.files.is_empty() || !self.locks.is_empty(),
        }
    }

    /// The mounted volume's filesystem-neutral policy, for callers that must
    /// not infer it from the handler name.
    pub fn volume_policy(&self) -> VolumePolicy {
        VolumePolicy {
            capabilities: self.vfs.capabilities(),
            statfs: self.vfs.statfs(),
            mount_mode: self.vfs.mount_mode(),
            pending_intent_records: self.vfs.pending_intent_records(),
        }
    }

    pub fn into_vfs(mut self) -> Result<Vfs<D>, ArosError> {
        let file_ids: Vec<_> = self.files.keys().copied().collect();
        for handle in file_ids {
            self.close(handle)?;
        }
        let lock_ids: Vec<_> = self.locks.keys().copied().collect();
        for lock in lock_ids {
            self.free_lock(lock)?;
        }
        Ok(self.vfs)
    }

    fn file_info(&self, stat: Stat, name: Vec<u8>, disk_key: u64) -> Result<FileInfo, ArosError> {
        if name.len() > self.config.max_file_info_name_bytes {
            return Err(ArosError::ObjectTooLarge);
        }
        let entry_type = match stat.kind {
            NodeKind::File => EntryType::File,
            NodeKind::Directory if stat.object_id == OBJECT_ROOT => EntryType::Root,
            NodeKind::Directory => EntryType::Directory,
            NodeKind::Symlink => EntryType::SoftLink,
            NodeKind::Internal => return Err(ArosError::ObjectWrongType),
        };
        let block_size = u64::from(self.vfs.statfs().block_size);
        Ok(FileInfo {
            disk_key,
            directory_entry_type: entry_type,
            entry_type,
            name,
            protection: stat.protection as u32,
            size: stat.size,
            blocks: stat.allocated_size.div_ceil(block_size),
            modified: stat.modified,
            object_id: stat.object_id,
        })
    }

    fn decode_component(&self, bytes: &[u8]) -> Result<String, ArosError> {
        if bytes.is_empty() || bytes.iter().any(|byte| matches!(byte, 0 | b'/' | b':')) {
            return Err(ArosError::InvalidComponentName);
        }
        match self.config.name_encoding {
            NameEncoding::Utf8 => std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| ArosError::InvalidComponentName),
            NameEncoding::Latin1 => Ok(bytes.iter().map(|byte| char::from(*byte)).collect()),
        }
    }

    fn decode_text(&self, bytes: &[u8]) -> Result<String, ArosError> {
        if bytes.contains(&0) {
            return Err(ArosError::InvalidComponentName);
        }
        match self.config.name_encoding {
            NameEncoding::Utf8 => std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| ArosError::InvalidComponentName),
            NameEncoding::Latin1 => Ok(bytes.iter().map(|byte| char::from(*byte)).collect()),
        }
    }

    fn encode_text(&self, utf8: &[u8]) -> Result<Vec<u8>, ArosError> {
        match self.config.name_encoding {
            NameEncoding::Utf8 => Ok(utf8.to_vec()),
            NameEncoding::Latin1 => std::str::from_utf8(utf8)
                .map_err(|_| ArosError::NotDosDisk)?
                .chars()
                .map(|character| {
                    u8::try_from(u32::from(character)).map_err(|_| ArosError::ObjectTooLarge)
                })
                .collect(),
        }
    }

    fn encode_name(&self, utf8: &[u8]) -> Result<Vec<u8>, ArosError> {
        let encoded = match self.config.name_encoding {
            NameEncoding::Utf8 => utf8.to_vec(),
            NameEncoding::Latin1 => std::str::from_utf8(utf8)
                .map_err(|_| ArosError::NotDosDisk)?
                .chars()
                .map(|character| {
                    u8::try_from(u32::from(character)).map_err(|_| ArosError::ObjectTooLarge)
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        if encoded.len() > self.config.max_file_info_name_bytes {
            return Err(ArosError::ObjectTooLarge);
        }
        Ok(encoded)
    }

    fn lock_object_or_root(&self, lock: Option<LockId>) -> Result<ObjectId, ArosError> {
        match lock {
            Some(lock) => Ok(self.lock_state(lock)?.object_id),
            None => Ok(OBJECT_ROOT),
        }
    }

    fn lock_state(&self, lock: LockId) -> Result<&LockState, ArosError> {
        self.locks.get(&lock).ok_or(ArosError::InvalidLock)
    }

    fn file_state(&self, handle: FileHandleId) -> Result<&FileState, ArosError> {
        self.files.get(&handle).ok_or(ArosError::InvalidLock)
    }

    fn ensure_lock_capacity(&self) -> Result<(), ArosError> {
        if self.locks.len() >= self.config.max_locks {
            Err(ArosError::NoFreeStore)
        } else {
            Ok(())
        }
    }

    fn ensure_file_capacity(&self) -> Result<(), ArosError> {
        if self.files.len() >= self.config.max_file_handles {
            Err(ArosError::NoFreeStore)
        } else {
            Ok(())
        }
    }

    fn insert_lock(
        &mut self,
        object_id: ObjectId,
        parent: Option<ObjectId>,
        name: Vec<u8>,
        access: LockAccess,
    ) -> Result<LockId, ArosError> {
        self.acquire_object_lock(object_id, access)?;
        let lock = self.next_lock;
        self.next_lock = match self.next_lock.checked_add(1) {
            Some(next) => next,
            None => {
                self.release_object_lock(object_id, access);
                return Err(ArosError::NoFreeStore);
            }
        };
        self.locks.insert(
            lock,
            LockState {
                object_id,
                parent,
                name,
                access,
                directory_handle: None,
                next_cookie: 0,
                last_entry: None,
            },
        );
        Ok(lock)
    }

    fn acquire_object_lock(
        &mut self,
        object_id: ObjectId,
        access: LockAccess,
    ) -> Result<(), ArosError> {
        let counts = self.lock_counts.entry(object_id).or_default();
        let refused = match access {
            LockAccess::Shared => counts.exclusive,
            LockAccess::Exclusive => counts.exclusive || counts.shared != 0,
        };
        if refused {
            if counts.shared == 0 && !counts.exclusive {
                self.lock_counts.remove(&object_id);
            }
            return Err(ArosError::ObjectInUse);
        }
        match access {
            LockAccess::Shared => counts.shared += 1,
            LockAccess::Exclusive => counts.exclusive = true,
        }
        Ok(())
    }

    fn release_object_lock(&mut self, object_id: ObjectId, access: LockAccess) {
        if let Some(counts) = self.lock_counts.get_mut(&object_id) {
            match access {
                LockAccess::Shared => counts.shared = counts.shared.saturating_sub(1),
                LockAccess::Exclusive => counts.exclusive = false,
            }
            if counts.shared == 0 && !counts.exclusive {
                self.lock_counts.remove(&object_id);
            }
        }
    }

    fn allocate_file_id(&mut self) -> Result<FileHandleId, ArosError> {
        let handle = self.next_file;
        self.next_file = self
            .next_file
            .checked_add(1)
            .ok_or(ArosError::NoFreeStore)?;
        Ok(handle)
    }
}

fn add_signed(base: u64, offset: i64) -> Result<u64, ArosError> {
    let result = i128::from(base) + i128::from(offset);
    u64::try_from(result).map_err(|_| ArosError::SeekError)
}
