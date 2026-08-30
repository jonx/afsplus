//! Host-neutral FUSE protocol adapter for AFS+.
//!
//! [`FuseAdapter`] contains the semantics that can be tested without a FUSE
//! kernel driver. The optional `fuser-adapter` feature only translates fuser
//! request/reply types around this layer.

use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Handle, NodeKind, ObjectId, Stat, StatFs, Vfs, VfsError};

#[cfg(feature = "fuser-adapter")]
pub mod fuser_adapter;

const FIRST_REAL_DIRECTORY_OFFSET: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuseConfig {
    pub uid: u32,
    pub gid: u32,
    pub file_mode: u16,
    pub directory_mode: u16,
}

impl Default for FuseConfig {
    fn default() -> Self {
        FuseConfig {
            uid: 0,
            gid: 0,
            file_mode: 0o644,
            directory_mode: 0o755,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuseAttributes {
    pub object_id: ObjectId,
    pub kind: NodeKind,
    pub size: u64,
    /// Allocated storage in POSIX 512-byte units.
    pub blocks_512: u64,
    pub links: u32,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
    pub block_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuseDirectoryEntry {
    pub name: Vec<u8>,
    pub object_id: ObjectId,
    pub kind: NodeKind,
    /// Opaque value supplied back by the kernel to resume after this entry.
    pub next_offset: u64,
}

/// Stateful translation from FUSE operations to the portable AFS+ VFS API.
///
/// Object IDs are stable and map one-to-one to FUSE inode numbers. Directory
/// parents are session metadata used only to synthesize `..`; they never alter
/// the on-disk namespace.
pub struct FuseAdapter<D: BlockDevice> {
    vfs: Vfs<D>,
    config: FuseConfig,
    parents: BTreeMap<ObjectId, ObjectId>,
}

impl<D: BlockDevice> FuseAdapter<D> {
    pub fn new(vfs: Vfs<D>, config: FuseConfig) -> Self {
        let mut parents = BTreeMap::new();
        parents.insert(OBJECT_ROOT, OBJECT_ROOT);
        FuseAdapter {
            vfs,
            config,
            parents,
        }
    }

    pub fn config(&self) -> FuseConfig {
        self.config
    }

    pub fn root_object(&self) -> ObjectId {
        self.vfs.root_object()
    }

    pub fn lookup(&mut self, parent: ObjectId, name: &[u8]) -> Result<FuseAttributes, VfsError> {
        let object_id = match name {
            b"." => parent,
            b".." => self.parent_of(parent),
            _ => self.vfs.lookup(parent, utf8_name(name)?)?,
        };
        self.parents.entry(object_id).or_insert(parent);
        self.attributes(object_id)
    }

    pub fn attributes(&mut self, object_id: ObjectId) -> Result<FuseAttributes, VfsError> {
        let stat = self.vfs.stat(object_id)?;
        Ok(self.map_attributes(stat))
    }

    pub fn statfs(&self) -> StatFs {
        self.vfs.statfs()
    }

    pub fn open_file(
        &mut self,
        object_id: ObjectId,
        access: AccessMode,
        truncate: bool,
        now: Timespec,
    ) -> Result<Handle, VfsError> {
        let handle = self.vfs.open_file(object_id, access)?;
        if truncate {
            if let Err(error) = self.vfs.truncate(handle, 0, now) {
                let _ = self.vfs.close(handle);
                return Err(error);
            }
        }
        Ok(handle)
    }

    pub fn open_directory(&mut self, object_id: ObjectId) -> Result<Handle, VfsError> {
        self.vfs.open_directory(object_id)
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), VfsError> {
        self.vfs.close(handle)
    }

    pub fn read(&mut self, handle: Handle, offset: u64, size: u32) -> Result<Vec<u8>, VfsError> {
        let mut data = vec![0; size as usize];
        let read = self.vfs.read(handle, offset, &mut data)?;
        data.truncate(read);
        Ok(data)
    }

    pub fn write(
        &mut self,
        handle: Handle,
        offset: u64,
        data: &[u8],
        now: Timespec,
    ) -> Result<usize, VfsError> {
        self.vfs.write(handle, offset, data, now)
    }

    pub fn truncate(
        &mut self,
        object_id: ObjectId,
        handle: Option<Handle>,
        size: u64,
        now: Timespec,
    ) -> Result<FuseAttributes, VfsError> {
        if let Some(handle) = handle {
            self.vfs.truncate(handle, size, now)?;
        } else {
            let temporary = self.vfs.open_file(object_id, AccessMode::WriteOnly)?;
            let result = self.vfs.truncate(temporary, size, now);
            let close_result = self.vfs.close(temporary);
            result?;
            close_result?;
        }
        self.attributes(object_id)
    }

    pub fn create_file(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        access: AccessMode,
        now: Timespec,
    ) -> Result<(FuseAttributes, Handle), VfsError> {
        let object_id = self.vfs.create_file(parent, utf8_name(name)?, now)?;
        self.parents.insert(object_id, parent);
        let handle = self.vfs.open_file(object_id, access)?;
        Ok((self.attributes(object_id)?, handle))
    }

    pub fn create_directory(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        now: Timespec,
    ) -> Result<FuseAttributes, VfsError> {
        let object_id = self.vfs.create_directory(parent, utf8_name(name)?, now)?;
        self.parents.insert(object_id, parent);
        self.attributes(object_id)
    }

    pub fn unlink_file(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError> {
        let object_id = self.vfs.lookup(parent, utf8_name(name)?)?;
        self.vfs.unlink_file(parent, utf8_name(name)?, now)?;
        self.parents.remove(&object_id);
        Ok(())
    }

    pub fn remove_directory(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError> {
        let object_id = self.vfs.lookup(parent, utf8_name(name)?)?;
        self.vfs.remove_directory(parent, utf8_name(name)?, now)?;
        self.parents.remove(&object_id);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rename(
        &mut self,
        source_parent: ObjectId,
        source_name: &[u8],
        target_parent: ObjectId,
        target_name: &[u8],
        replace: bool,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let source_name = utf8_name(source_name)?;
        let target_name = utf8_name(target_name)?;
        let object_id = self.vfs.lookup(source_parent, source_name)?;
        self.vfs.rename(
            source_parent,
            source_name,
            target_parent,
            target_name,
            replace,
            now,
        )?;
        self.parents.insert(object_id, target_parent);
        Ok(())
    }

    pub fn link_file(
        &mut self,
        object_id: ObjectId,
        target_parent: ObjectId,
        target_name: &[u8],
        now: Timespec,
    ) -> Result<FuseAttributes, VfsError> {
        self.vfs
            .link_file(object_id, target_parent, utf8_name(target_name)?, now)?;
        self.attributes(object_id)
    }

    /// Returns at most `max_entries`, including synthesized dot entries.
    pub fn read_directory(
        &mut self,
        directory: ObjectId,
        handle: Handle,
        offset: u64,
        max_entries: usize,
    ) -> Result<Vec<FuseDirectoryEntry>, VfsError> {
        if max_entries == 0 {
            return Ok(Vec::new());
        }

        let mut entries = Vec::with_capacity(max_entries);
        if offset == 0 {
            entries.push(FuseDirectoryEntry {
                name: b".".to_vec(),
                object_id: directory,
                kind: NodeKind::Directory,
                next_offset: 1,
            });
        }
        if offset <= 1 && entries.len() < max_entries {
            entries.push(FuseDirectoryEntry {
                name: b"..".to_vec(),
                object_id: self.parent_of(directory),
                kind: NodeKind::Directory,
                next_offset: 2,
            });
        }
        if entries.len() == max_entries {
            return Ok(entries);
        }

        let underlying_cookie = offset.saturating_sub(2);
        let page =
            self.vfs
                .read_directory(handle, underlying_cookie, max_entries - entries.len())?;
        for (index, entry) in page.entries.into_iter().enumerate() {
            self.parents.entry(entry.object_id).or_insert(directory);
            entries.push(FuseDirectoryEntry {
                name: entry.name,
                object_id: entry.object_id,
                kind: entry.kind,
                next_offset: FIRST_REAL_DIRECTORY_OFFSET + underlying_cookie + index as u64,
            });
        }
        Ok(entries)
    }

    pub fn fsync(&mut self, handle: Handle) -> Result<(), VfsError> {
        self.vfs.fsync(handle)
    }

    pub fn sync_filesystem(&mut self) -> Result<(), VfsError> {
        self.vfs.sync_filesystem()
    }

    pub fn into_vfs(self) -> Vfs<D> {
        self.vfs
    }

    fn parent_of(&self, object_id: ObjectId) -> ObjectId {
        self.parents.get(&object_id).copied().unwrap_or(OBJECT_ROOT)
    }

    fn map_attributes(&self, stat: Stat) -> FuseAttributes {
        let block_size = self.vfs.statfs().block_size;
        FuseAttributes {
            object_id: stat.object_id,
            kind: stat.kind,
            size: stat.size,
            blocks_512: stat.allocated_size.div_ceil(512),
            links: stat.links,
            mode: match stat.kind {
                NodeKind::Directory => self.config.directory_mode,
                _ => self.config.file_mode,
            },
            uid: self.config.uid,
            gid: self.config.gid,
            created: stat.created,
            modified: stat.modified,
            changed: stat.changed,
            block_size,
        }
    }
}

fn utf8_name(name: &[u8]) -> Result<&str, VfsError> {
    std::str::from_utf8(name).map_err(|_| VfsError::Invalid)
}
