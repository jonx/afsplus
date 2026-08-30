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
use afsplus_vfs::{AccessMode, Handle, NodeKind, ObjectId, Stat, Vfs, VfsError};

pub type LockId = u64;
pub type FileHandleId = u64;

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

/// AROS DOS secondary result values used by the Alpha-0 packet bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ArosError {
    Unknown = 100,
    NoFreeStore = 103,
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
    NotDosDisk = 225,
    NoMoreEntries = 232,
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
}

#[derive(Debug, Clone)]
struct FileState {
    vfs_handle: Handle,
    object_id: ObjectId,
    parent: ObjectId,
    name: Vec<u8>,
    position: u64,
}

pub struct ArosAdapter<D: BlockDevice> {
    vfs: Vfs<D>,
    config: ArosConfig,
    locks: BTreeMap<LockId, LockState>,
    lock_counts: BTreeMap<ObjectId, LockCounts>,
    known_parents: BTreeMap<ObjectId, (Option<ObjectId>, Vec<u8>)>,
    files: BTreeMap<FileHandleId, FileState>,
    next_lock: LockId,
    next_file: FileHandleId,
}

impl<D: BlockDevice> ArosAdapter<D> {
    pub fn new(vfs: Vfs<D>, config: ArosConfig) -> Self {
        let mut known_parents = BTreeMap::new();
        known_parents.insert(OBJECT_ROOT, (None, config.volume_name.clone()));
        ArosAdapter {
            vfs,
            config,
            locks: BTreeMap::new(),
            lock_counts: BTreeMap::new(),
            known_parents,
            files: BTreeMap::new(),
            next_lock: 1,
            next_file: 1,
        }
    }

    pub fn root_object(&self) -> ObjectId {
        OBJECT_ROOT
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
        self.insert_lock(parent, grandparent, name, LockAccess::Shared)
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
                self.vfs.create_file(parent, &decoded, now)?
            }
            (_, Err(error)) => return Err(error.into()),
        };
        if self.vfs.stat(object_id)?.kind != NodeKind::File {
            return Err(ArosError::ObjectWrongType);
        }
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
        let handle = self.allocate_file_id()?;
        self.files.insert(
            handle,
            FileState {
                vfs_handle,
                object_id,
                parent,
                name: name.to_vec(),
                position: 0,
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

    pub fn close(&mut self, handle: FileHandleId) -> Result<(), ArosError> {
        let state = self.files.remove(&handle).ok_or(ArosError::InvalidLock)?;
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
        self.files
            .get_mut(&handle)
            .expect("validated handle")
            .position = position
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
        Ok(size)
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
        match self.vfs.stat(object)?.kind {
            NodeKind::File => Ok(self.vfs.unlink_file(parent, &decoded, now)?),
            NodeKind::Directory => Ok(self.vfs.remove_directory(parent, &decoded, now)?),
            _ => Err(ArosError::ActionNotKnown),
        }
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
        Ok(self
            .vfs
            .link_file(source_object, target_parent, &target, now)?)
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
        let page = self.vfs.read_directory(handle, cookie, 1)?;
        let Some(entry) = page.entries.into_iter().next() else {
            return Err(ArosError::NoMoreEntries);
        };
        self.locks
            .get_mut(&lock)
            .expect("validated lock")
            .next_cookie = page.next_cookie;
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
        Ok(())
    }

    pub fn disk_info(&self) -> DiskInfo {
        let stat = self.vfs.statfs();
        DiskInfo {
            write_protected: self.vfs.mount_mode() != MountMode::ReadWrite,
            total_blocks: stat.total_blocks,
            used_blocks: stat.total_blocks.saturating_sub(stat.free_blocks),
            bytes_per_block: stat.block_size,
            disk_type: DISK_TYPE_AFS_PLUS,
            in_use: !self.files.is_empty() || !self.locks.is_empty(),
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
        let counts = self.lock_counts.entry(object_id).or_default();
        match access {
            LockAccess::Shared if counts.exclusive => return Err(ArosError::ObjectInUse),
            LockAccess::Exclusive if counts.exclusive || counts.shared != 0 => {
                return Err(ArosError::ObjectInUse);
            }
            LockAccess::Shared => counts.shared += 1,
            LockAccess::Exclusive => counts.exclusive = true,
        }
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
            },
        );
        Ok(lock)
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
