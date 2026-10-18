//! Host-authorized snapshot primitives. The adapter checks authority at admission.
use super::*;
use afsplus_core::volume::{SnapshotInfo, SnapshotListPage};

impl<D: BlockDevice> Vfs<D> {
    pub fn snapshot_readers_active(&self) -> bool {
        !self.snapshot_readers.is_empty()
    }
    /// A reader or snapshot directory handle that is still open.
    pub fn snapshot_handle_open(&self, handle: Handle) -> bool {
        self.snapshot_readers.contains_key(&handle)
            || self.snapshot_directories.contains_key(&handle)
    }
    pub fn snapshot_create(&mut self, now: Timespec) -> Result<u64, VfsError> {
        self.sync_filesystem()?;
        Ok(self.volume.snapshot_create(now)?)
    }
    pub fn snapshot_delete(&mut self, id: u64, now: Timespec) -> Result<(), VfsError> {
        self.sync_filesystem()?;
        Ok(self.volume.snapshot_delete(id, now)?)
    }
    pub fn snapshot_list(
        &mut self,
        after: u64,
        limit: usize,
    ) -> Result<SnapshotListPage, VfsError> {
        if limit == 0 || limit > 64 {
            return Err(VfsError::Invalid);
        }
        Ok(self.volume.snapshot_list(after, limit)?)
    }
    pub fn snapshot_open(&mut self, id: u64, max_readers: usize) -> Result<Handle, VfsError> {
        if self.snapshot_readers.len() >= max_readers {
            return Err(VfsError::Limit("snapshot reader budget exhausted"));
        }
        let view = self.volume.snapshot_open(id)?;
        let handle = self.next_snapshot_handle()?;
        self.snapshot_readers.insert(handle, view);
        Ok(handle)
    }
    fn next_snapshot_handle(&mut self) -> Result<Handle, VfsError> {
        let handle = self.next_handle;
        self.next_handle = handle
            .checked_add(1)
            .ok_or(VfsError::Limit("handle identifiers exhausted"))?;
        Ok(handle)
    }
    pub fn snapshot_close(&mut self, reader: Handle) -> Result<(), VfsError> {
        self.snapshot_readers
            .remove(&reader)
            .ok_or(VfsError::Stale)?;
        self.snapshot_directories
            .retain(|_, state| state.0 != reader);
        Ok(())
    }
    pub fn snapshot_info(&self, reader: Handle) -> Result<SnapshotInfo, VfsError> {
        Ok(self
            .snapshot_readers
            .get(&reader)
            .ok_or(VfsError::Stale)?
            .info())
    }
    pub fn snapshot_stat(&mut self, reader: Handle, object: ObjectId) -> Result<Stat, VfsError> {
        let view = self.snapshot_readers.get(&reader).ok_or(VfsError::Stale)?;
        Ok(self
            .volume
            .snapshot_stat(view, object)?
            .ok_or(VfsError::NotFound)?
            .into())
    }
    pub fn snapshot_lookup(
        &mut self,
        reader: Handle,
        directory: ObjectId,
        name: &str,
    ) -> Result<ObjectId, VfsError> {
        let view = self.snapshot_readers.get(&reader).ok_or(VfsError::Stale)?;
        self.volume
            .snapshot_lookup(view, directory, name)?
            .ok_or(VfsError::NotFound)
    }
    pub fn snapshot_read(
        &mut self,
        reader: Handle,
        object: ObjectId,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError> {
        let view = self.snapshot_readers.get(&reader).ok_or(VfsError::Stale)?;
        Ok(self
            .volume
            .snapshot_read_file_at(view, object, offset, out)?)
    }
    pub fn snapshot_directory_open(
        &mut self,
        reader: Handle,
        object: ObjectId,
        max: usize,
    ) -> Result<Handle, VfsError> {
        if self.snapshot_directories.len() >= max {
            return Err(VfsError::Limit("snapshot directory budget exhausted"));
        }
        if self.snapshot_stat(reader, object)?.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let handle = self.next_snapshot_handle()?;
        self.snapshot_directories
            .insert(handle, (reader, object, None));
        Ok(handle)
    }
    pub fn snapshot_directory_close(&mut self, directory: Handle) -> Result<(), VfsError> {
        self.snapshot_directories
            .remove(&directory)
            .ok_or(VfsError::Stale)?;
        Ok(())
    }
    pub fn snapshot_directory_read(
        &mut self,
        directory: Handle,
        limit: usize,
    ) -> Result<DirectoryPage, VfsError> {
        self.snapshot_directory_read_encoded(directory, limit, |name| Ok(name.to_vec()))
    }
    pub fn snapshot_directory_read_encoded<F>(
        &mut self,
        directory: Handle,
        limit: usize,
        mut encode: F,
    ) -> Result<DirectoryPage, VfsError>
    where
        F: FnMut(&[u8]) -> Result<Vec<u8>, VfsError>,
    {
        if limit == 0 || limit > 64 {
            return Err(VfsError::Invalid);
        }
        let (reader, object, cursor) = *self
            .snapshot_directories
            .get(&directory)
            .ok_or(VfsError::Stale)?;
        let view = self.snapshot_readers.get(&reader).ok_or(VfsError::Stale)?;
        let page = self
            .volume
            .snapshot_read_directory_page(view, object, cursor, limit)?;
        let mut entries = Vec::with_capacity(page.entries.len());
        for entry in page.entries {
            let stat = self
                .volume
                .snapshot_stat(view, entry.child_id)?
                .ok_or(VfsError::NotFound)?;
            entries.push(DirectoryEntry {
                object_id: entry.child_id,
                kind: stat.object_type.into(),
                name: encode(&entry.name)?,
            });
        }
        self.snapshot_directories
            .get_mut(&directory)
            .ok_or(VfsError::Stale)?
            .2 = Some(page.next);
        Ok(DirectoryPage {
            entries,
            next_cookie: 0,
            eof: page.eof,
        })
    }
}
