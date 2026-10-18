//! Historical operations for a host-authorized packet admission.
use super::*;
use afsplus_core::volume::SnapshotListPage;

impl<D: BlockDevice> ArosAdapter<D> {
    pub fn snapshot_create(&mut self, now: Timespec) -> Result<u64, ArosError> {
        self.ensure_writable()?;
        Ok(self.vfs.snapshot_create(now)?)
    }
    pub fn snapshot_delete(&mut self, id: u64, now: Timespec) -> Result<(), ArosError> {
        self.ensure_writable()?;
        Ok(self.vfs.snapshot_delete(id, now)?)
    }
    pub fn snapshot_list(
        &mut self,
        after: u64,
        limit: usize,
    ) -> Result<SnapshotListPage, ArosError> {
        Ok(self.vfs.snapshot_list(after, limit)?)
    }
    pub fn snapshot_open(&mut self, id: u64) -> Result<u64, ArosError> {
        Ok(self.vfs.snapshot_open(id, self.config.max_file_handles)?)
    }
    pub fn snapshot_close(&mut self, reader: u64) -> Result<(), ArosError> {
        Ok(self.vfs.snapshot_close(reader)?)
    }
    pub fn snapshot_lookup(
        &mut self,
        reader: u64,
        directory: u64,
        name: &[u8],
    ) -> Result<u64, ArosError> {
        let name = self.decode_component(name)?;
        Ok(self.vfs.snapshot_lookup(reader, directory, &name)?)
    }
    pub fn snapshot_read(
        &mut self,
        reader: u64,
        object: u64,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, ArosError> {
        Ok(self.vfs.snapshot_read(reader, object, offset, out)?)
    }
    pub fn snapshot_stat(
        &mut self,
        reader: u64,
        object: u64,
    ) -> Result<afsplus_vfs::Stat, ArosError> {
        Ok(self.vfs.snapshot_stat(reader, object)?)
    }
    pub fn snapshot_directory_open(&mut self, reader: u64, object: u64) -> Result<u64, ArosError> {
        Ok(self
            .vfs
            .snapshot_directory_open(reader, object, self.config.max_locks)?)
    }
    pub fn snapshot_directory_read(
        &mut self,
        directory: u64,
        limit: usize,
    ) -> Result<afsplus_vfs::DirectoryPage, ArosError> {
        let encoding = self.config.name_encoding;
        Ok(self
            .vfs
            .snapshot_directory_read_encoded(directory, limit, |bytes| match encoding {
                NameEncoding::Utf8 => Ok(bytes.to_vec()),
                NameEncoding::Latin1 => std::str::from_utf8(bytes)
                    .map_err(|_| afsplus_vfs::VfsError::Invalid)?
                    .chars()
                    .map(|ch| {
                        u8::try_from(u32::from(ch)).map_err(|_| afsplus_vfs::VfsError::Invalid)
                    })
                    .collect(),
            })?)
    }
    pub fn snapshot_directory_close(&mut self, directory: u64) -> Result<(), ArosError> {
        Ok(self.vfs.snapshot_directory_close(directory)?)
    }
}

impl<D: BlockDevice> ArosAdapter<D> {
    /// Caller admission must supply independent backup and restore authority.
    pub fn snapshot_restore_file(
        &mut self,
        reader: u64,
        source: u64,
        destination: Option<LockId>,
        name: &[u8],
        now: Timespec,
    ) -> Result<u64, ArosError> {
        self.ensure_writable()?;
        let parent = self.lock_object_or_root(destination)?;
        let name = self.decode_component(name)?;
        let object = self
            .vfs
            .snapshot_restore_file(reader, source, parent, &name, now)?;
        self.touch(parent, &name);
        Ok(object)
    }
}

impl<D: BlockDevice> ArosAdapter<D> {
    pub fn snapshot_restore_tree(
        &mut self,
        reader: u64,
        source: u64,
        destination: Option<LockId>,
        name: &[u8],
        limits: afsplus_vfs::TreeRestoreLimits,
        now: Timespec,
    ) -> Result<u64, ArosError> {
        self.ensure_writable()?;
        let parent = self.lock_object_or_root(destination)?;
        let name = self.decode_component(name)?;
        let object = self
            .vfs
            .snapshot_restore_tree(reader, source, parent, &name, limits, now)?;
        self.touch(parent, &name);
        Ok(object)
    }
}
