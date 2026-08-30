//! Thin `fuser` request/reply translation.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use afsplus_block::BlockDevice;
use afsplus_format::Timespec;
use afsplus_vfs::{AccessMode, NodeKind, Vfs, VfsError};
use fuser::{
    BsdFileFlags, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation,
    INodeNo, LockOwner, OpenAccMode, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
    WriteFlags,
};

use crate::{FuseAdapter, FuseAttributes, FuseConfig};

const ATTRIBUTE_TTL: Duration = Duration::from_secs(1);
const DIRECTORY_BATCH: usize = 128;

pub struct FuserFilesystem<D: BlockDevice + Send> {
    adapter: Mutex<FuseAdapter<D>>,
}

impl<D: BlockDevice + Send> FuserFilesystem<D> {
    pub fn new(vfs: Vfs<D>, config: FuseConfig) -> Self {
        FuserFilesystem {
            adapter: Mutex::new(FuseAdapter::new(vfs, config)),
        }
    }

    pub fn from_adapter(adapter: FuseAdapter<D>) -> Self {
        FuserFilesystem {
            adapter: Mutex::new(adapter),
        }
    }

    pub fn into_adapter(self) -> Result<FuseAdapter<D>, VfsError> {
        self.adapter
            .into_inner()
            .map_err(|_| VfsError::Io("FUSE adapter mutex poisoned".into()))
    }

    fn lock(&self) -> Result<MutexGuard<'_, FuseAdapter<D>>, Errno> {
        self.adapter.lock().map_err(|_| Errno::EIO)
    }
}

impl<D: BlockDevice + Send + 'static> Filesystem for FuserFilesystem<D> {
    fn destroy(&mut self) {
        if let Ok(adapter) = self.adapter.get_mut() {
            let _ = adapter.sync_filesystem();
        }
    }

    fn lookup(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.lookup(parent.0, name.as_bytes()).map_err(errno));
        match result {
            Ok(attributes) => {
                reply.entry(&ATTRIBUTE_TTL, &file_attributes(&attributes), Generation(0))
            }
            Err(error) => reply.error(error),
        }
    }

    fn getattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        _handle: Option<FileHandle>,
        reply: ReplyAttr,
    ) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.attributes(inode.0).map_err(errno));
        match result {
            Ok(attributes) => reply.attr(&ATTRIBUTE_TTL, &file_attributes(&attributes)),
            Err(error) => reply.error(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn setattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        handle: Option<FileHandle>,
        _creation_time: Option<SystemTime>,
        _change_time: Option<SystemTime>,
        _backup_time: Option<SystemTime>,
        flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            let attributes = adapter.attributes(inode.0).map_err(errno)?;
            // macOS follows create/mkdir with a SETATTR that repeats the mode,
            // owner and empty BSD flags it just requested. AFS+ does not yet
            // persist those fields, but acknowledging an exact no-op is safe.
            let metadata_is_unchanged = mode
                .is_none_or(|value| value & 0o7777 == u32::from(attributes.mode))
                && uid.is_none_or(|value| value == attributes.uid)
                && gid.is_none_or(|value| value == attributes.gid)
                && flags.is_none_or(|value| value.is_empty());
            if !metadata_is_unchanged {
                return Err(Errno::EOPNOTSUPP);
            }
            if let Some(size) = size {
                adapter
                    .truncate(inode.0, handle.map(|value| value.0), size, now())
                    .map_err(errno)
            } else {
                Ok(attributes)
            }
        });
        match result {
            Ok(attributes) => reply.attr(&ATTRIBUTE_TTL, &file_attributes(&attributes)),
            Err(error) => reply.error(error),
        }
    }

    fn mknod(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        _device: u32,
        reply: ReplyEntry,
    ) {
        if mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFREG) {
            reply.error(Errno::EOPNOTSUPP);
            return;
        }
        let result = self.lock().and_then(|mut adapter| {
            let (attributes, handle) = adapter
                .create_file(parent.0, name.as_bytes(), AccessMode::WriteOnly, now())
                .map_err(errno)?;
            adapter.close(handle).map_err(errno)?;
            Ok(attributes)
        });
        match result {
            Ok(attributes) => {
                reply.entry(&ATTRIBUTE_TTL, &file_attributes(&attributes), Generation(0))
            }
            Err(error) => reply.error(error),
        }
    }

    fn mkdir(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .create_directory(parent.0, name.as_bytes(), now())
                .map_err(errno)
        });
        match result {
            Ok(attributes) => {
                reply.entry(&ATTRIBUTE_TTL, &file_attributes(&attributes), Generation(0))
            }
            Err(error) => reply.error(error),
        }
    }

    fn unlink(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .unlink_file(parent.0, name.as_bytes(), now())
                .map_err(errno)
        });
        empty_reply(result, reply);
    }

    fn rmdir(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .remove_directory(parent.0, name.as_bytes(), now())
                .map_err(errno)
        });
        empty_reply(result, reply);
    }

    fn rename(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        target_parent: INodeNo,
        target_name: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let replace = match rename_replace(flags) {
            Ok(replace) => replace,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .rename(
                    parent.0,
                    name.as_bytes(),
                    target_parent.0,
                    target_name.as_bytes(),
                    replace,
                    now(),
                )
                .map_err(errno)
        });
        empty_reply(result, reply);
    }

    fn link(
        &self,
        _request: &Request,
        inode: INodeNo,
        target_parent: INodeNo,
        target_name: &OsStr,
        reply: ReplyEntry,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .link_file(inode.0, target_parent.0, target_name.as_bytes(), now())
                .map_err(errno)
        });
        match result {
            Ok(attributes) => {
                reply.entry(&ATTRIBUTE_TTL, &file_attributes(&attributes), Generation(0))
            }
            Err(error) => reply.error(error),
        }
    }

    fn open(&self, _request: &Request, inode: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .open_file(
                    inode.0,
                    access_mode(flags),
                    flags.0 & libc::O_TRUNC != 0,
                    now(),
                )
                .map_err(errno)
        });
        match result {
            Ok(handle) => reply.opened(FileHandle(handle), FopenFlags::empty()),
            Err(error) => reply.error(error),
        }
    }

    fn read(
        &self,
        _request: &Request,
        _inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.read(handle.0, offset, size).map_err(errno));
        match result {
            Ok(data) => reply.data(&data),
            Err(error) => reply.error(error),
        }
    }

    fn write(
        &self,
        _request: &Request,
        _inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.write(handle.0, offset, data, now()).map_err(errno));
        match result {
            Ok(written) => match u32::try_from(written) {
                Ok(written) => reply.written(written),
                Err(_) => reply.error(Errno::EOVERFLOW),
            },
            Err(error) => reply.error(error),
        }
    }

    fn flush(
        &self,
        _request: &Request,
        _inode: INodeNo,
        _handle: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        // AFS+ commits writes synchronously. POSIX flush is not an fsync.
        reply.ok();
    }

    fn release(
        &self,
        _request: &Request,
        _inode: INodeNo,
        handle: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.close(handle.0).map_err(errno));
        empty_reply(result, reply);
    }

    fn fsync(
        &self,
        _request: &Request,
        _inode: INodeNo,
        handle: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.fsync(handle.0).map_err(errno));
        empty_reply(result, reply);
    }

    fn opendir(&self, _request: &Request, inode: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.open_directory(inode.0).map_err(errno));
        match result {
            Ok(handle) => reply.opened(FileHandle(handle), FopenFlags::empty()),
            Err(error) => reply.error(error),
        }
    }

    fn readdir(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .read_directory(inode.0, handle.0, offset, DIRECTORY_BATCH)
                .map_err(errno)
        });
        match result {
            Ok(entries) => {
                for entry in entries {
                    if reply.add(
                        INodeNo(entry.object_id),
                        entry.next_offset,
                        file_type(entry.kind),
                        OsStr::from_bytes(&entry.name),
                    ) {
                        break;
                    }
                }
                reply.ok();
            }
            Err(error) => reply.error(error),
        }
    }

    fn releasedir(
        &self,
        _request: &Request,
        _inode: INodeNo,
        handle: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.close(handle.0).map_err(errno));
        empty_reply(result, reply);
    }

    fn fsyncdir(
        &self,
        _request: &Request,
        _inode: INodeNo,
        handle: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.fsync(handle.0).map_err(errno));
        empty_reply(result, reply);
    }

    fn statfs(&self, _request: &Request, _inode: INodeNo, reply: ReplyStatfs) {
        match self.lock() {
            Ok(adapter) => {
                let stats = adapter.statfs();
                reply.statfs(
                    stats.total_blocks,
                    stats.free_blocks,
                    stats.available_blocks,
                    0,
                    0,
                    stats.block_size,
                    stats.max_name_bytes,
                    stats.block_size,
                );
            }
            Err(error) => reply.error(error),
        }
    }

    fn create(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let flags = OpenFlags(flags);
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .create_file(parent.0, name.as_bytes(), access_mode(flags), now())
                .map_err(errno)
        });
        match result {
            Ok((attributes, handle)) => reply.created(
                &ATTRIBUTE_TTL,
                &file_attributes(&attributes),
                Generation(0),
                FileHandle(handle),
                FopenFlags::empty(),
            ),
            Err(error) => reply.error(error),
        }
    }
}

fn empty_reply(result: Result<(), Errno>, reply: ReplyEmpty) {
    match result {
        Ok(()) => reply.ok(),
        Err(error) => reply.error(error),
    }
}

fn access_mode(flags: OpenFlags) -> AccessMode {
    match flags.acc_mode() {
        OpenAccMode::O_RDONLY => AccessMode::ReadOnly,
        OpenAccMode::O_WRONLY => AccessMode::WriteOnly,
        OpenAccMode::O_RDWR => AccessMode::ReadWrite,
    }
}

#[cfg(target_os = "linux")]
fn rename_replace(flags: RenameFlags) -> Result<bool, Errno> {
    if flags.is_empty() {
        Ok(true)
    } else if flags == RenameFlags::RENAME_NOREPLACE {
        Ok(false)
    } else {
        Err(Errno::EOPNOTSUPP)
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_replace(flags: RenameFlags) -> Result<bool, Errno> {
    if flags.is_empty() {
        Ok(true)
    } else {
        Err(Errno::EOPNOTSUPP)
    }
}

fn errno(error: VfsError) -> Errno {
    match error {
        VfsError::NotFound => Errno::ENOENT,
        VfsError::AlreadyExists => Errno::EEXIST,
        VfsError::NotDirectory => Errno::ENOTDIR,
        VfsError::IsDirectory => Errno::EISDIR,
        VfsError::DirectoryNotEmpty => Errno::ENOTEMPTY,
        VfsError::Invalid => Errno::EINVAL,
        VfsError::ReadOnly => Errno::EROFS,
        VfsError::NoSpace => Errno::ENOSPC,
        VfsError::Stale => Errno::ESTALE,
        VfsError::Busy => Errno::EBUSY,
        VfsError::NotSupported => Errno::EOPNOTSUPP,
        VfsError::Corrupt(_) | VfsError::Io(_) => Errno::EIO,
        VfsError::Limit(_) => Errno::EOVERFLOW,
    }
}

fn file_attributes(attributes: &FuseAttributes) -> FileAttr {
    FileAttr {
        ino: INodeNo(attributes.object_id),
        size: attributes.size,
        blocks: attributes.blocks_512,
        atime: system_time(attributes.modified),
        mtime: system_time(attributes.modified),
        ctime: system_time(attributes.changed),
        crtime: system_time(attributes.created),
        kind: file_type(attributes.kind),
        perm: attributes.mode,
        nlink: attributes.links,
        uid: attributes.uid,
        gid: attributes.gid,
        rdev: 0,
        blksize: attributes.block_size,
        flags: 0,
    }
}

fn file_type(kind: NodeKind) -> FileType {
    match kind {
        NodeKind::File => FileType::RegularFile,
        NodeKind::Directory => FileType::Directory,
        NodeKind::Symlink => FileType::Symlink,
        NodeKind::Internal => FileType::RegularFile,
    }
}

fn now() -> Timespec {
    timespec(SystemTime::now())
}

fn timespec(time: SystemTime) -> Timespec {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => Timespec {
            seconds: duration.as_secs().min(i64::MAX as u64) as i64,
            nanoseconds: duration.subsec_nanos(),
        },
        Err(error) => {
            let duration = error.duration();
            let whole_seconds = duration.as_secs().min(i64::MAX as u64) as i64;
            if duration.subsec_nanos() == 0 {
                Timespec {
                    seconds: -whole_seconds,
                    nanoseconds: 0,
                }
            } else {
                Timespec {
                    seconds: whole_seconds.saturating_add(1).saturating_neg(),
                    nanoseconds: 1_000_000_000 - duration.subsec_nanos(),
                }
            }
        }
    }
}

fn system_time(time: Timespec) -> SystemTime {
    if time.seconds < 0 {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(time.seconds.unsigned_abs()))
            .and_then(|whole| whole.checked_add(Duration::from_nanos(time.nanoseconds.into())))
            .unwrap_or(UNIX_EPOCH)
    } else {
        UNIX_EPOCH
            .checked_add(Duration::new(time.seconds as u64, time.nanoseconds))
            .unwrap_or(UNIX_EPOCH)
    }
}
