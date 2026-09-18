//! Thin `fuser` request/reply translation.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use afsplus_block::BlockDevice;
use afsplus_format::Timespec;
use afsplus_vfs::{AccessMode, NodeKind, Vfs, VfsError};
use fuser::{
    BsdFileFlags, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation,
    INodeNo, LockOwner, OpenAccMode, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr,
    Request, TimeOrNow, WriteFlags,
};

use crate::{AttributeWriteMode, FuseAdapter, FuseAttributes, FuseConfig};

const ATTRIBUTE_TTL: Duration = Duration::from_secs(1);

/// The permission bits of a `mode_t`, without the file-type bits a caller of
/// `mknod` or `create` also puts there. The kernel has already applied the
/// process umask by the time the request reaches us.
fn time_or_now(value: TimeOrNow) -> Timespec {
    match value {
        TimeOrNow::SpecificTime(time) => timespec(time),
        TimeOrNow::Now => now(),
    }
}

fn permission_bits(mode: u32) -> u16 {
    (mode & 0o7777) as u16
}
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

    /// Closing a file is where the mount catches up on maintenance.
    ///
    /// It used to happen when a host asked how much space was free, which read
    /// well and was wrong: the Finder asks that many times a second, each call
    /// did up to sixteen checkpoint commits, and the volume became unusable to
    /// look at. A close is a person finishing something, it is not polled, and
    /// it happens often enough for the backlog to drain.
    fn release_maintenance(&self) {
        if let Ok(mut adapter) = self.lock() {
            adapter.run_maintenance(now());
        }
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
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        handle: Option<FileHandle>,
        _creation_time: Option<SystemTime>,
        _change_time: Option<SystemTime>,
        _backup_time: Option<SystemTime>,
        flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            // Every field here is now either stored or refused. What is no
            // longer done is answering success and keeping nothing, which is
            // what this function did with the times: `touch -t` returned 0
            // and the modification time did not move.
            //
            // The access time is the one field with no write, and that is a
            // declared property rather than a silent drop: this format keeps
            // no access time and the volume reports the modification time in
            // its place, so there is nothing to store and nothing claimed.
            //
            // The change time is not settable through POSIX. It follows any
            // metadata write, which the core does on its own.
            if let Some(mode) = mode {
                adapter
                    .set_posix_mode(inode.0, permission_bits(mode), now())
                    .map_err(errno)?;
            }
            if uid.is_some() || gid.is_some() {
                adapter.set_owner(inode.0, uid, gid, now()).map_err(errno)?;
            }
            if let Some(mtime) = mtime {
                adapter
                    .set_times(inode.0, time_or_now(mtime), now())
                    .map_err(errno)?;
            }
            // BSD flags have no carrier. An empty set is a no-op; anything
            // else is refused rather than dropped.
            if flags.is_some_and(|value| !value.is_empty()) {
                return Err(Errno::EOPNOTSUPP);
            }
            if let Some(size) = size {
                adapter
                    .truncate(inode.0, handle.map(|value| value.0), size, now())
                    .map_err(errno)
            } else {
                adapter.attributes(inode.0).map_err(errno)
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
                .create_file(
                    parent.0,
                    name.as_bytes(),
                    AccessMode::WriteOnly,
                    Some(permission_bits(mode)),
                    now(),
                )
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

    /// Both of these were missing, so the driver answered with the trait's
    /// defaults: `ln -s` failed with EPERM and reading a link with ENOSYS,
    /// while the core and the portable interface had done symlinks all along.
    fn symlink(
        &self,
        _request: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .create_symlink(
                    parent.0,
                    link_name.as_bytes(),
                    target.as_os_str().as_bytes(),
                    now(),
                )
                .map_err(errno)
        });
        match result {
            Ok(attributes) => {
                reply.entry(&ATTRIBUTE_TTL, &file_attributes(&attributes), Generation(0))
            }
            Err(error) => reply.error(error),
        }
    }

    fn readlink(&self, _request: &Request, inode: INodeNo, reply: ReplyData) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.read_link(inode.0).map_err(errno));
        match result {
            Ok(target) => reply.data(&target),
            Err(error) => reply.error(error),
        }
    }

    fn mkdir(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .create_directory(
                    parent.0,
                    name.as_bytes(),
                    Some(permission_bits(mode)),
                    now(),
                )
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
        // POSIX flush is not a durability request. Pending writes remain in
        // the VFS window until FSYNC, a later committing operation or unmount.
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
        if result.is_ok() {
            self.release_maintenance();
        }
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

    fn setxattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        name: &OsStr,
        value: &[u8],
        flags: i32,
        position: u32,
        reply: ReplyEmpty,
    ) {
        let result = xattr_write_mode(flags, position).and_then(|mode| {
            self.lock().and_then(|mut adapter| {
                adapter
                    .set_attribute(inode.0, name.as_bytes(), value, mode, now())
                    .map_err(xattr_errno)
            })
        });
        empty_reply(result, reply);
    }

    fn getxattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        name: &OsStr,
        size: u32,
        reply: ReplyXattr,
    ) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .get_attribute(inode.0, name.as_bytes())
                .map_err(xattr_errno)
        });
        xattr_reply(result, size, reply);
    }

    fn listxattr(&self, _request: &Request, inode: INodeNo, size: u32, reply: ReplyXattr) {
        let result = self
            .lock()
            .and_then(|mut adapter| adapter.list_attributes(inode.0).map_err(xattr_errno));
        xattr_reply(result, size, reply);
    }

    fn removexattr(&self, _request: &Request, inode: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .remove_attribute(inode.0, name.as_bytes(), now())
                .map_err(xattr_errno)
        });
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
        mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let flags = OpenFlags(flags);
        let result = self.lock().and_then(|mut adapter| {
            adapter
                .create_file(
                    parent.0,
                    name.as_bytes(),
                    access_mode(flags),
                    Some(permission_bits(mode)),
                    now(),
                )
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

/// `XATTR_CREATE` and `XATTR_REPLACE` as the host's `setxattr(2)` numbers
/// them; the two hosts disagree.
#[cfg(target_os = "macos")]
const XATTR_FLAGS: (i32, i32) = (0x0002, 0x0004);
#[cfg(not(target_os = "macos"))]
const XATTR_FLAGS: (i32, i32) = (0x1, 0x2);

/// The write mode of a `setxattr` request. A nonzero position addresses a
/// part of a value, which only a resource fork uses and this volume does not
/// store; any other flag bit is refused instead of ignored.
fn xattr_write_mode(flags: i32, position: u32) -> Result<AttributeWriteMode, Errno> {
    let (create, replace) = XATTR_FLAGS;
    if position != 0 {
        return Err(Errno::ENOTSUP);
    }
    match flags {
        0 => Ok(AttributeWriteMode::Upsert),
        bits if bits == create => Ok(AttributeWriteMode::Create),
        bits if bits == replace => Ok(AttributeWriteMode::Replace),
        _ => Err(Errno::EINVAL),
    }
}

/// Attribute calls have their own words for absence and size.
fn xattr_errno(error: VfsError) -> Errno {
    match error {
        VfsError::NotFound => Errno::NO_XATTR,
        VfsError::Limit(_) => Errno::E2BIG,
        VfsError::NotSupported => Errno::ENOTSUP,
        other => errno(other),
    }
}

/// The size-probing protocol of `getxattr` and `listxattr`: size zero asks
/// for the length, a buffer too small is `ERANGE`, never a truncated value.
fn xattr_reply(result: Result<Vec<u8>, Errno>, size: u32, reply: ReplyXattr) {
    match result {
        Err(error) => reply.error(error),
        Ok(bytes) => match u32::try_from(bytes.len()) {
            Err(_) => reply.error(Errno::E2BIG),
            Ok(length) if size == 0 => reply.size(length),
            Ok(length) if length > size => reply.error(Errno::ERANGE),
            Ok(_) => reply.data(&bytes),
        },
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
        // Staged writes could not be published and were abandoned so the
        // volume stays usable. EIO is the truthful errno: data a caller was
        // told had been written is gone. How much is in the diagnostics
        // report, which is where a person can read a reason rather than a
        // three-letter code.
        VfsError::WindowLost(_) => Errno::EIO,
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

#[cfg(test)]
mod xattr_tests {
    use super::*;

    #[test]
    fn setxattr_flags_follow_the_host_and_nothing_is_ignored() {
        let (create, replace) = XATTR_FLAGS;
        assert_eq!(xattr_write_mode(0, 0), Ok(AttributeWriteMode::Upsert));
        assert_eq!(xattr_write_mode(create, 0), Ok(AttributeWriteMode::Create));
        assert_eq!(
            xattr_write_mode(replace, 0),
            Ok(AttributeWriteMode::Replace)
        );
        assert_eq!(xattr_write_mode(create | replace, 0), Err(Errno::EINVAL));
        assert_eq!(xattr_write_mode(0x4000, 0), Err(Errno::EINVAL));
        assert_eq!(xattr_write_mode(0, 1), Err(Errno::ENOTSUP));
    }

    #[test]
    fn attribute_errors_use_the_attribute_vocabulary() {
        assert_eq!(xattr_errno(VfsError::NotFound), Errno::NO_XATTR);
        assert_eq!(xattr_errno(VfsError::Limit("x")), Errno::E2BIG);
        assert_eq!(xattr_errno(VfsError::NotSupported), Errno::ENOTSUP);
        assert_eq!(xattr_errno(VfsError::AlreadyExists), Errno::EEXIST);
        assert_eq!(xattr_errno(VfsError::ReadOnly), Errno::EROFS);
    }
}
