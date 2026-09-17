//! Destination-scoped restoration, separately authorized under ADR-077.
use crate::authority::{Grant, Issuer, Scope};
use crate::backup::{
    valid_metadata_text, AllocationPage, AllocationRange, MetadataClass, MetadataEntry,
    MAX_METADATA_ENCODING_BYTES, MAX_METADATA_KEY_BYTES,
};
use crate::{Stat, VfsError};
use afsplus_format::Timespec;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, RwLockReadGuard,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreError {
    Denied,
    Filesystem(VfsError),
}
impl From<VfsError> for RestoreError {
    fn from(e: VfsError) -> Self {
        Self::Filesystem(e)
    }
}
impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => f.write_str("restore authority denied"),
            Self::Filesystem(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for RestoreError {}

/// Preservation values independent of destination identities and storage addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreMetadata {
    pub protection: u64,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
}
impl RestoreMetadata {
    fn validate(&self) -> Result<(), VfsError> {
        for time in [self.created, self.modified, self.changed] {
            time.validate().map_err(|_| VfsError::Invalid)?;
        }
        Ok(())
    }
}
/// Trusted provider for a host-selected destination. Names are exact single
/// components; never interpret them as host paths or follow existing links.
/// Consumers get only objects returned through the checked facade.
pub trait RestoreBackend {
    type Object;
    fn root(&mut self) -> Result<Self::Object, VfsError>;
    /// Resolve one entry in the exclusively owned, initially empty restore tree.
    /// Never follow a symlink or resolve a path outside the supplied directory.
    fn lookup_created(
        &mut self,
        _parent: &Self::Object,
        _name: &str,
    ) -> Result<Self::Object, VfsError> {
        Err(VfsError::NotSupported)
    }
    /// Inspect at most one entry; unsupported enumeration is not emptiness.
    fn directory_empty(&mut self, _directory: &Self::Object) -> Result<bool, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn create_symlink(
        &mut self,
        _parent: &Self::Object,
        _name: &str,
        _target: &str,
        _now: Timespec,
    ) -> Result<Self::Object, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn read_link(&mut self, _object: &Self::Object, _out: &mut [u8]) -> Result<usize, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn create_file(
        &mut self,
        parent: &Self::Object,
        name: &str,
        now: Timespec,
    ) -> Result<Self::Object, VfsError>;
    fn create_directory(
        &mut self,
        parent: &Self::Object,
        name: &str,
        now: Timespec,
    ) -> Result<Self::Object, VfsError>;
    fn write(
        &mut self,
        object: &Self::Object,
        offset: u64,
        bytes: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError>;
    /// Reserve exactly the requested byte coverage without extending size or
    /// modifying written contents. Refuse unsupported alignment/semantics.
    fn reserve(
        &mut self,
        _object: &Self::Object,
        _offset: u64,
        _length: u64,
        _now: Timespec,
    ) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }
    fn resize(&mut self, object: &Self::Object, size: u64, now: Timespec) -> Result<(), VfsError>;
    fn link(
        &mut self,
        object: &Self::Object,
        parent: &Self::Object,
        name: &str,
        now: Timespec,
    ) -> Result<(), VfsError>;
    fn metadata(
        &mut self,
        object: &Self::Object,
        metadata: RestoreMetadata,
    ) -> Result<(), VfsError>;
    /// Committed semantic allocation, with entry-ordinal cursors. Unsupported
    /// enumeration is not evidence of an empty allocation inventory.
    fn allocations(
        &mut self,
        _object: &Self::Object,
        _start: u64,
        _limit: usize,
    ) -> Result<AllocationPage, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn stat(&mut self, object: &Self::Object) -> Result<Stat, VfsError>;
    fn read(
        &mut self,
        object: &Self::Object,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError>;
    fn sync(&mut self) -> Result<(), VfsError>;
}
/// Optional provider extension. Upload Drop releases private staging, even
/// after revocation. Finish publishes an entire value atomically or refuses it.
pub trait OpaqueRestoreBackend: RestoreBackend {
    type Upload;
    fn begin_opaque(
        &mut self,
        _object: &Self::Object,
        _class: MetadataClass,
        _entry: &MetadataEntry,
    ) -> Result<Self::Upload, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn write_opaque(
        &mut self,
        _upload: &mut Self::Upload,
        _offset: u64,
        _bytes: &[u8],
    ) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }
    fn finish_opaque(&mut self, _upload: Self::Upload) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }
}

#[derive(Clone)]
pub struct RestoreGrant(Grant);
#[derive(Clone)]
pub struct RestoreAuthority(Issuer);
impl RestoreAuthority {
    pub fn grant(&self) -> RestoreGrant {
        RestoreGrant(self.0.grant())
    }
    /// Drains admitted work; does not undo writes already committed.
    /// Do not call synchronously from a backend operation under this grant.
    pub fn revoke(&self, grant: &RestoreGrant) -> Result<(), RestoreError> {
        self.0.revoke(&grant.0).map_err(|_| RestoreError::Denied)
    }
}
struct Budget(Arc<AtomicUsize>);
impl Drop for Budget {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct ObjectState<O> {
    object: O,
    grant: RestoreGrant,
    // Return budget only after the provider object has been released.
    _budget: Budget,
}
pub struct RestoreObject<O>(Arc<ObjectState<O>>);
impl<O> Clone for RestoreObject<O> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<O> RestoreObject<O> {
    pub fn close(self) {
        drop(self);
    }
}

/// Non-cloneable staged value bound to its original destination and grant.
pub struct RestoreUpload<U, O> {
    // Release staging before the object lease and its separate budget unit.
    staging: Option<U>,
    object: RestoreObject<O>,
    size: u64,
    written: u64,
    failed: bool,
    _budget: Budget,
}
impl<U, O> RestoreUpload<U, O> {
    pub fn abort(self) {
        drop(self);
    }
}

pub struct RestoreService<P: RestoreBackend> {
    backend: P,
    scope: Scope,
    active: Arc<AtomicUsize>,
    max_handles: usize,
    max_reservation_bytes: u64,
    max_metadata_bytes: Option<u64>,
}
impl<P: RestoreBackend> RestoreService<P> {
    pub fn new(backend: P, max_handles: usize) -> Result<(Self, RestoreAuthority), RestoreError> {
        if max_handles == 0 {
            return Err(VfsError::Limit("restore handle budget must be positive").into());
        }
        let (scope, issuer) = Scope::new();
        Ok((
            Self {
                backend,
                scope,
                active: Arc::new(AtomicUsize::new(0)),
                max_handles,
                max_reservation_bytes: 0,
                max_metadata_bytes: None,
            },
            RestoreAuthority(issuer),
        ))
    }
    /// Host-owned per-operation admission limit. Zero disables reservation.
    /// Consumers cannot raise it through the checked facade.
    pub fn set_reservation_limit(&mut self, bytes: u64) {
        self.max_reservation_bytes = bytes;
    }
    /// Host-only value admission. None disables uploads; Some(0) permits empty values.
    pub fn set_metadata_limit(&mut self, bytes: Option<u64>) {
        self.max_metadata_bytes = bytes;
    }
    pub fn client(&mut self) -> RestoreClient<'_, P> {
        RestoreClient(self)
    }
    /// Privileged host hook; never expose it through consumer IPC.
    pub fn backend_mut(&mut self) -> &mut P {
        &mut self.backend
    }
    pub fn into_backend(self) -> P {
        self.backend
    }
    fn admit<'a>(
        &self,
        grant: &'a RestoreGrant,
    ) -> Result<RwLockReadGuard<'a, bool>, RestoreError> {
        self.scope.admit(&grant.0).map_err(|_| RestoreError::Denied)
    }
    fn reserve_handle(&self) -> Result<Budget, RestoreError> {
        if self.active.load(Ordering::Acquire) >= self.max_handles {
            return Err(VfsError::Limit("restore handle budget exhausted").into());
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        Ok(Budget(self.active.clone()))
    }
    fn wrap(object: P::Object, grant: RestoreGrant, budget: Budget) -> RestoreObject<P::Object> {
        RestoreObject(Arc::new(ObjectState {
            object,
            grant,
            _budget: budget,
        }))
    }
    pub fn root(&mut self, grant: &RestoreGrant) -> Result<RestoreObject<P::Object>, RestoreError> {
        let _permit = self.admit(grant)?;
        let budget = self.reserve_handle()?;
        Ok(Self::wrap(self.backend.root()?, grant.clone(), budget))
    }
    /// Reopen a created entry without retaining every restore object handle.
    pub fn lookup_created(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        let _permit = self.admit(&parent.0.grant)?;
        component(name)?;
        let budget = self.reserve_handle()?;
        if self.backend.stat(&parent.0.object)?.kind != crate::NodeKind::Directory {
            return Err(VfsError::NotDirectory.into());
        }
        let object = self.backend.lookup_created(&parent.0.object, name)?;
        Ok(Self::wrap(object, parent.0.grant.clone(), budget))
    }
    pub fn directory_empty(
        &mut self,
        directory: &RestoreObject<P::Object>,
    ) -> Result<bool, RestoreError> {
        let _permit = self.admit(&directory.0.grant)?;
        if self.backend.stat(&directory.0.object)?.kind != crate::NodeKind::Directory {
            return Err(VfsError::NotDirectory.into());
        }
        Ok(self.backend.directory_empty(&directory.0.object)?)
    }
    fn create(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
        now: Timespec,
        directory: bool,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        let _permit = self.admit(&parent.0.grant)?;
        component(name)?;
        timestamp(now)?;
        let budget = self.reserve_handle()?;
        let object = if directory {
            self.backend.create_directory(&parent.0.object, name, now)?
        } else {
            self.backend.create_file(&parent.0.object, name, now)?
        };
        Ok(Self::wrap(object, parent.0.grant.clone(), budget))
    }
    pub fn create_symlink(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
        target: &str,
        now: Timespec,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        let _permit = self.admit(&parent.0.grant)?;
        component(name)?;
        timestamp(now)?;
        if target.is_empty() || target.as_bytes().contains(&0) {
            return Err(VfsError::Invalid.into());
        }
        let budget = self.reserve_handle()?;
        if self.backend.stat(&parent.0.object)?.kind != crate::NodeKind::Directory {
            return Err(VfsError::NotDirectory.into());
        }
        let object = self
            .backend
            .create_symlink(&parent.0.object, name, target, now)?;
        Ok(Self::wrap(object, parent.0.grant.clone(), budget))
    }
    pub fn read_link(
        &mut self,
        object: &RestoreObject<P::Object>,
        out: &mut [u8],
    ) -> Result<usize, RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        if self.backend.stat(&object.0.object)?.kind != crate::NodeKind::Symlink {
            return Err(VfsError::Invalid.into());
        }
        Ok(self.backend.read_link(&object.0.object, out)?)
    }
    pub fn create_file(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
        now: Timespec,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        self.create(parent, name, now, false)
    }
    pub fn create_directory(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
        now: Timespec,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        self.create(parent, name, now, true)
    }
    pub fn write(
        &mut self,
        object: &RestoreObject<P::Object>,
        offset: u64,
        bytes: &[u8],
        now: Timespec,
    ) -> Result<(), RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        timestamp(now)?;
        Ok(self.backend.write(&object.0.object, offset, bytes, now)?)
    }
    pub fn reserve(
        &mut self,
        object: &RestoreObject<P::Object>,
        offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        timestamp(now)?;
        if length == 0 || offset as u128 + length as u128 > (1u128 << 64) {
            return Err(VfsError::Invalid.into());
        }
        if length > self.max_reservation_bytes {
            return Err(VfsError::Limit("restore reservation byte budget exhausted").into());
        }
        Ok(self
            .backend
            .reserve(&object.0.object, offset, length, now)?)
    }
    pub fn resize(
        &mut self,
        object: &RestoreObject<P::Object>,
        size: u64,
        now: Timespec,
    ) -> Result<(), RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        timestamp(now)?;
        Ok(self.backend.resize(&object.0.object, size, now)?)
    }
    pub fn link(
        &mut self,
        object: &RestoreObject<P::Object>,
        parent: &RestoreObject<P::Object>,
        name: &str,
        now: Timespec,
    ) -> Result<(), RestoreError> {
        let _source_permit = self.admit(&object.0.grant)?;
        // Re-locking the same read lock while a revoker waits can deadlock on
        // writer-preferring platforms. One permit covers identical grants.
        let _parent_permit = if object.0.grant.0.same(&parent.0.grant.0) {
            None
        } else {
            Some(self.admit(&parent.0.grant)?)
        };
        component(name)?;
        timestamp(now)?;
        Ok(self
            .backend
            .link(&object.0.object, &parent.0.object, name, now)?)
    }
    pub fn metadata(
        &mut self,
        object: &RestoreObject<P::Object>,
        metadata: RestoreMetadata,
    ) -> Result<(), RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        metadata.validate()?;
        Ok(self.backend.metadata(&object.0.object, metadata)?)
    }
    pub fn allocations(
        &mut self,
        object: &RestoreObject<P::Object>,
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        if limit == 0 || limit > 64 {
            return Err(VfsError::Limit("allocation page limit out of range").into());
        }
        let page = self.backend.allocations(&object.0.object, start, limit)?;
        if page.ranges.len() > limit
            || (!page.eof && page.ranges.is_empty())
            || start.checked_add(page.ranges.len() as u64) != Some(page.next)
        {
            return Err(VfsError::Corrupt("invalid allocation page progress".into()).into());
        }
        let mut end = 0u128;
        for range in &page.ranges {
            if range.length == 0 || (range.offset as u128) < end {
                return Err(VfsError::Corrupt("invalid allocation range ordering".into()).into());
            }
            end = range.offset as u128 + range.length as u128;
            if end > (1u128 << 64) {
                return Err(VfsError::Corrupt("allocation byte range overflow".into()).into());
            }
        }
        Ok(page)
    }
    pub fn stat(&mut self, object: &RestoreObject<P::Object>) -> Result<Stat, RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        Ok(self.backend.stat(&object.0.object)?)
    }
    pub fn read(
        &mut self,
        object: &RestoreObject<P::Object>,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        Ok(self.backend.read(&object.0.object, offset, out)?)
    }
    pub fn sync(&mut self, grant: &RestoreGrant) -> Result<(), RestoreError> {
        let _permit = self.admit(grant)?;
        Ok(self.backend.sync()?)
    }
}
impl<P: OpaqueRestoreBackend> RestoreService<P> {
    pub fn begin_opaque(
        &mut self,
        object: &RestoreObject<P::Object>,
        class: MetadataClass,
        entry: &MetadataEntry,
    ) -> Result<RestoreUpload<P::Upload, P::Object>, RestoreError> {
        let _permit = self.admit(&object.0.grant)?;
        let limit = self.max_metadata_bytes.ok_or(VfsError::NotSupported)?;
        if entry.size > limit {
            return Err(VfsError::Limit("metadata value budget exhausted").into());
        }
        if !valid_metadata_text(&entry.key, MAX_METADATA_KEY_BYTES)
            || !valid_metadata_text(&entry.encoding, MAX_METADATA_ENCODING_BYTES)
        {
            return Err(VfsError::Invalid.into());
        }
        let budget = self.reserve_handle()?;
        let staging = self.backend.begin_opaque(&object.0.object, class, entry)?;
        Ok(RestoreUpload {
            staging: Some(staging),
            object: object.clone(),
            size: entry.size,
            written: 0,
            failed: false,
            _budget: budget,
        })
    }
    pub fn write_opaque(
        &mut self,
        upload: &mut RestoreUpload<P::Upload, P::Object>,
        bytes: &[u8],
    ) -> Result<(), RestoreError> {
        let _permit = self.admit(&upload.object.0.grant)?;
        if upload.failed {
            return Err(VfsError::Invalid.into());
        }
        if bytes.len() as u128 > (upload.size - upload.written) as u128 {
            return Err(VfsError::Invalid.into());
        }
        if bytes.is_empty() {
            return Ok(());
        }
        upload.failed = true;
        self.backend
            .write_opaque(upload.staging.as_mut().unwrap(), upload.written, bytes)?;
        upload.written += bytes.len() as u64;
        upload.failed = false;
        Ok(())
    }
    pub fn finish_opaque(
        &mut self,
        mut upload: RestoreUpload<P::Upload, P::Object>,
    ) -> Result<(), RestoreError> {
        let _permit = self.admit(&upload.object.0.grant)?;
        if upload.failed || upload.written != upload.size {
            return Err(VfsError::Invalid.into());
        }
        Ok(self.backend.finish_opaque(upload.staging.take().unwrap())?)
    }
}

impl<P: OpaqueRestoreBackend> RestoreClient<'_, P> {
    pub fn begin_opaque(
        &mut self,
        object: &RestoreObject<P::Object>,
        class: MetadataClass,
        entry: &MetadataEntry,
    ) -> Result<RestoreUpload<P::Upload, P::Object>, RestoreError> {
        self.0.begin_opaque(object, class, entry)
    }
    pub fn write_opaque(
        &mut self,
        upload: &mut RestoreUpload<P::Upload, P::Object>,
        bytes: &[u8],
    ) -> Result<(), RestoreError> {
        self.0.write_opaque(upload, bytes)
    }
    pub fn finish_opaque(
        &mut self,
        upload: RestoreUpload<P::Upload, P::Object>,
    ) -> Result<(), RestoreError> {
        self.0.finish_opaque(upload)
    }
}

fn timestamp(now: Timespec) -> Result<(), VfsError> {
    now.validate().map_err(|_| VfsError::Invalid)
}
fn component(name: &str) -> Result<(), VfsError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        Err(VfsError::Invalid)
    } else {
        Ok(())
    }
}

/// Consumer facade with no raw backend or handle constructors.
///
/// ```compile_fail
/// use afsplus_vfs::{backup::BackupGrant, restore::{RestoreClient, RestoreBackend}};
/// fn wrong_role<P: RestoreBackend>(client: &mut RestoreClient<'_, P>, backup: &BackupGrant) {
///     client.root(backup);
/// }
/// ```
/// ```compile_fail
/// use afsplus_vfs::{backup::{BackupClient, SnapshotBackend}, restore::RestoreGrant};
/// fn wrong_role<P: SnapshotBackend>(client: &mut BackupClient<'_, P>, restore: &RestoreGrant) {
///     client.list(restore, 0, 1);
/// }
/// ```
/// ```compile_fail
/// use afsplus_vfs::restore::{RestoreClient, RestoreBackend};
/// fn bypass<P: RestoreBackend>(client: &mut RestoreClient<'_, P>) { client.backend_mut(); }
/// ```
pub struct RestoreClient<'a, P: RestoreBackend>(&'a mut RestoreService<P>);
impl<P: RestoreBackend> RestoreClient<'_, P> {
    pub fn root(&mut self, grant: &RestoreGrant) -> Result<RestoreObject<P::Object>, RestoreError> {
        self.0.root(grant)
    }
    pub fn lookup_created(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        self.0.lookup_created(parent, name)
    }
    pub fn directory_empty(
        &mut self,
        directory: &RestoreObject<P::Object>,
    ) -> Result<bool, RestoreError> {
        self.0.directory_empty(directory)
    }
    pub fn create_symlink(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
        target: &str,
        now: Timespec,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        self.0.create_symlink(parent, name, target, now)
    }
    pub fn read_link(
        &mut self,
        object: &RestoreObject<P::Object>,
        out: &mut [u8],
    ) -> Result<usize, RestoreError> {
        self.0.read_link(object, out)
    }
    pub fn create_file(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
        now: Timespec,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        self.0.create_file(parent, name, now)
    }
    pub fn create_directory(
        &mut self,
        parent: &RestoreObject<P::Object>,
        name: &str,
        now: Timespec,
    ) -> Result<RestoreObject<P::Object>, RestoreError> {
        self.0.create_directory(parent, name, now)
    }
    pub fn write(
        &mut self,
        object: &RestoreObject<P::Object>,
        offset: u64,
        bytes: &[u8],
        now: Timespec,
    ) -> Result<(), RestoreError> {
        self.0.write(object, offset, bytes, now)
    }
    pub fn reserve(
        &mut self,
        object: &RestoreObject<P::Object>,
        offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), RestoreError> {
        self.0.reserve(object, offset, length, now)
    }
    pub fn resize(
        &mut self,
        object: &RestoreObject<P::Object>,
        size: u64,
        now: Timespec,
    ) -> Result<(), RestoreError> {
        self.0.resize(object, size, now)
    }
    pub fn link(
        &mut self,
        object: &RestoreObject<P::Object>,
        parent: &RestoreObject<P::Object>,
        name: &str,
        now: Timespec,
    ) -> Result<(), RestoreError> {
        self.0.link(object, parent, name, now)
    }
    pub fn metadata(
        &mut self,
        object: &RestoreObject<P::Object>,
        metadata: RestoreMetadata,
    ) -> Result<(), RestoreError> {
        self.0.metadata(object, metadata)
    }
    pub fn allocations(
        &mut self,
        object: &RestoreObject<P::Object>,
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, RestoreError> {
        self.0.allocations(object, start, limit)
    }
    pub fn stat(&mut self, object: &RestoreObject<P::Object>) -> Result<Stat, RestoreError> {
        self.0.stat(object)
    }
    pub fn read(
        &mut self,
        object: &RestoreObject<P::Object>,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, RestoreError> {
        self.0.read(object, offset, out)
    }
    pub fn sync(&mut self, grant: &RestoreGrant) -> Result<(), RestoreError> {
        self.0.sync(grant)
    }
}

/// Host-owned AFS+ provider for a selected empty directory. The provider owns
/// the Volume exclusively; consumers cannot introduce existing external objects.
pub struct AfsRestoreDestination<D: afsplus_block::BlockDevice> {
    volume: afsplus_core::Volume<D>,
    root: u64,
}
impl<D: afsplus_block::BlockDevice> AfsRestoreDestination<D> {
    pub fn new(mut volume: afsplus_core::Volume<D>, root: u64) -> Result<Self, VfsError> {
        if volume.mount_mode() != afsplus_core::MountMode::ReadWrite {
            return Err(VfsError::ReadOnly);
        }
        let page = volume.read_directory_page(root, None, 1)?;
        if !page.entries.is_empty() {
            return Err(VfsError::DirectoryNotEmpty);
        }
        Ok(Self { volume, root })
    }
    pub fn into_volume(self) -> afsplus_core::Volume<D> {
        self.volume
    }
}
impl<D: afsplus_block::BlockDevice> OpaqueRestoreBackend for AfsRestoreDestination<D> {
    type Upload = ();
}

impl<D: afsplus_block::BlockDevice> RestoreBackend for AfsRestoreDestination<D> {
    type Object = u64;
    fn root(&mut self) -> Result<u64, VfsError> {
        Ok(self.root)
    }
    fn lookup_created(&mut self, parent: &u64, name: &str) -> Result<u64, VfsError> {
        self.volume
            .lookup_in_directory(*parent, name)?
            .ok_or(VfsError::NotFound)
    }
    fn directory_empty(&mut self, directory: &u64) -> Result<bool, VfsError> {
        Ok(self
            .volume
            .read_directory_page(*directory, None, 1)?
            .entries
            .is_empty())
    }
    fn create_symlink(
        &mut self,
        parent: &u64,
        name: &str,
        target: &str,
        now: Timespec,
    ) -> Result<u64, VfsError> {
        Ok(self.volume.create_symlink(*parent, name, target, now)?)
    }
    fn read_link(&mut self, object: &u64, out: &mut [u8]) -> Result<usize, VfsError> {
        Ok(self.volume.read_link(*object, out)?)
    }
    fn create_file(&mut self, parent: &u64, name: &str, now: Timespec) -> Result<u64, VfsError> {
        Ok(self
            .volume
            .create_file_in_directory(*parent, name, &[], now)?)
    }
    fn create_directory(
        &mut self,
        parent: &u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, VfsError> {
        Ok(self.volume.create_directory(*parent, name, now)?)
    }
    fn write(
        &mut self,
        object: &u64,
        offset: u64,
        bytes: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError> {
        Ok(self.volume.write_file_at_bounded(
            *object,
            offset,
            bytes,
            now,
            afsplus_core::volume::FileEditLimits {
                max_blocks: 64,
                max_records: 64,
            },
        )?)
    }
    fn reserve(
        &mut self,
        object: &u64,
        offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let alignment = self.volume.ident().geometry().block_size as u64;
        if length == 0 || offset as u128 + length as u128 > (1u128 << 64) {
            return Err(VfsError::Invalid);
        }
        if !offset.is_multiple_of(alignment) || !length.is_multiple_of(alignment) {
            return Err(VfsError::NotSupported);
        }
        // Core preallocation accepts an exclusive end representable in u64;
        // requesting one byte less reserves the same final rounded block.
        let request = if offset.checked_add(length).is_none() {
            length - 1
        } else {
            length
        };
        Ok(self.volume.preallocate_file_bounded(
            *object,
            offset,
            request,
            now,
            afsplus_core::volume::FileEditLimits {
                max_blocks: length / alignment,
                max_records: 64,
            },
        )?)
    }
    fn resize(&mut self, object: &u64, size: u64, now: Timespec) -> Result<(), VfsError> {
        Ok(self.volume.truncate_file_bounded(
            *object,
            size,
            now,
            afsplus_core::volume::FileEditLimits {
                max_blocks: 64,
                max_records: 64,
            },
        )?)
    }
    fn link(
        &mut self,
        object: &u64,
        parent: &u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), VfsError> {
        Ok(self.volume.link_file(*object, *parent, name, now)?)
    }
    fn metadata(&mut self, object: &u64, metadata: RestoreMetadata) -> Result<(), VfsError> {
        let protection = u32::try_from(metadata.protection).map_err(|_| VfsError::Invalid)?;
        Ok(self.volume.restore_object_metadata(
            *object,
            afsplus_core::volume::PreservedMetadata {
                protection,
                created: metadata.created,
                modified: metadata.modified,
                changed: metadata.changed,
            },
        )?)
    }
    fn allocations(
        &mut self,
        object: &u64,
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, VfsError> {
        let page = self.volume.file_allocation_page(*object, start, limit)?;
        Ok(AllocationPage {
            ranges: page
                .ranges
                .into_iter()
                .map(|range| AllocationRange {
                    offset: range.offset,
                    length: range.length,
                    unwritten: range.unwritten,
                })
                .collect(),
            next: page.next,
            eof: page.eof,
        })
    }
    fn stat(&mut self, object: &u64) -> Result<Stat, VfsError> {
        Ok(self
            .volume
            .stat(*object)?
            .ok_or(VfsError::NotFound)
            .map(afsplus_core::volume::ObjectMetadata::from)?
            .into())
    }
    fn read(&mut self, object: &u64, offset: u64, out: &mut [u8]) -> Result<usize, VfsError> {
        Ok(self.volume.read_file_at(*object, offset, out)?)
    }
    fn sync(&mut self) -> Result<(), VfsError> {
        Ok(self.volume.sync()?)
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    struct Probe {
        grants: Vec<RestoreGrant>,
        calls: usize,
        symlink: bool,
    }
    impl Probe {
        fn check(&mut self) {
            for grant in &self.grants {
                assert!(matches!(
                    grant.0 .0.active.try_write(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
            }
            self.calls += 1;
        }
    }
    impl RestoreBackend for Probe {
        type Object = ();
        fn root(&mut self) -> Result<(), VfsError> {
            Ok(())
        }
        fn directory_empty(&mut self, _: &()) -> Result<bool, VfsError> {
            self.check();
            Ok(true)
        }
        fn create_symlink(
            &mut self,
            _: &(),
            _: &str,
            _: &str,
            _: Timespec,
        ) -> Result<(), VfsError> {
            self.check();
            self.symlink = true;
            Ok(())
        }
        fn read_link(&mut self, _: &(), out: &mut [u8]) -> Result<usize, VfsError> {
            self.check();
            if !out.is_empty() {
                out[0] = b'x';
            }
            Ok(1)
        }
        fn create_file(&mut self, _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
            Ok(())
        }
        fn create_directory(&mut self, _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
            unreachable!()
        }
        fn write(&mut self, _: &(), _: u64, _: &[u8], _: Timespec) -> Result<(), VfsError> {
            self.check();
            Ok(())
        }
        fn reserve(&mut self, _: &(), _: u64, _: u64, _: Timespec) -> Result<(), VfsError> {
            self.check();
            Ok(())
        }
        fn resize(&mut self, _: &(), _: u64, _: Timespec) -> Result<(), VfsError> {
            unreachable!()
        }
        fn link(&mut self, _: &(), _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
            self.check();
            Ok(())
        }
        fn metadata(&mut self, _: &(), _: RestoreMetadata) -> Result<(), VfsError> {
            unreachable!()
        }
        fn allocations(
            &mut self,
            _: &(),
            start: u64,
            _: usize,
        ) -> Result<AllocationPage, VfsError> {
            self.check();
            Ok(AllocationPage {
                ranges: vec![],
                next: start,
                eof: true,
            })
        }
        fn stat(&mut self, _: &()) -> Result<Stat, VfsError> {
            self.check();
            Ok(Stat {
                object_id: 1,
                kind: if self.symlink {
                    crate::NodeKind::Symlink
                } else {
                    crate::NodeKind::Directory
                },
                size: 0,
                allocated_size: 0,
                links: 1,
                protection: 0,
                mode: 0,
                owner_uid: 0,
                owner_gid: 0,
                created: Timespec::default(),
                modified: Timespec::default(),
                changed: Timespec::default(),
                content_generation: 1,
            })
        }
        fn lookup_created(&mut self, _: &(), _: &str) -> Result<(), VfsError> {
            self.check();
            Ok(())
        }
        fn read(&mut self, _: &(), _: u64, _: &mut [u8]) -> Result<usize, VfsError> {
            unreachable!()
        }
        fn sync(&mut self) -> Result<(), VfsError> {
            unreachable!()
        }
    }
    impl OpaqueRestoreBackend for Probe {
        type Upload = ();
        fn begin_opaque(
            &mut self,
            _: &(),
            _: MetadataClass,
            _: &MetadataEntry,
        ) -> Result<(), VfsError> {
            self.check();
            Ok(())
        }
        fn write_opaque(&mut self, _: &mut (), _: u64, _: &[u8]) -> Result<(), VfsError> {
            self.check();
            Ok(())
        }
        fn finish_opaque(&mut self, _: ()) -> Result<(), VfsError> {
            self.check();
            Ok(())
        }
    }
    #[test]
    fn symlink_creation_and_read_hold_original_grant_through_callbacks() {
        let (mut service, authority) = RestoreService::new(
            Probe {
                grants: vec![],
                calls: 0,
                symlink: false,
            },
            2,
        )
        .unwrap();
        let grant = authority.grant();
        let root = service.root(&grant).unwrap();
        service.backend_mut().grants = vec![grant.clone()];
        let link = service
            .client()
            .create_symlink(&root, "link", "x", Timespec::default())
            .unwrap();
        assert_eq!(service.backend_mut().calls, 2);
        assert_eq!(service.client().read_link(&link, &mut []), Ok(1));
        assert_eq!(service.backend_mut().calls, 4);
        authority.revoke(&grant).unwrap();
        assert_eq!(service.read_link(&link, &mut []), Err(RestoreError::Denied));
        assert_eq!(service.backend_mut().calls, 4);
    }
    #[test]
    fn directory_emptiness_holds_original_permit_without_a_new_handle() {
        let (mut service, authority) = RestoreService::new(
            Probe {
                grants: vec![],
                calls: 0,
                symlink: false,
            },
            1,
        )
        .unwrap();
        let grant = authority.grant();
        let root = service.root(&grant).unwrap();
        service.backend_mut().grants = vec![grant.clone()];
        assert!(service.client().directory_empty(&root).unwrap());
        assert_eq!(service.backend_mut().calls, 2);
        authority.revoke(&grant).unwrap();
        assert_eq!(service.directory_empty(&root), Err(RestoreError::Denied));
        assert_eq!(service.backend_mut().calls, 2);
    }
    #[test]
    fn created_lookup_holds_operation_permit_across_stat_and_lookup() {
        let (mut service, authority) = RestoreService::new(
            Probe {
                grants: vec![],
                calls: 0,
                symlink: false,
            },
            2,
        )
        .unwrap();
        let grant = authority.grant();
        let root = service.root(&grant).unwrap();
        service.backend_mut().grants = vec![grant];
        let child = service.client().lookup_created(&root, "child").unwrap();
        assert_eq!(service.backend_mut().calls, 2);
        drop(child);
    }
    #[test]
    fn allocation_readback_holds_operation_permit() {
        let (mut service, authority) = RestoreService::new(
            Probe {
                grants: vec![],
                calls: 0,
                symlink: false,
            },
            1,
        )
        .unwrap();
        let grant = authority.grant();
        let object = service.root(&grant).unwrap();
        service.backend_mut().grants = vec![grant];
        assert!(service.client().allocations(&object, 0, 1).unwrap().eof);
        assert_eq!(service.backend_mut().calls, 1);
    }
    #[test]
    fn opaque_upload_holds_admission_during_all_provider_calls() {
        let (mut service, authority) = RestoreService::new(
            Probe {
                grants: vec![],
                calls: 0,
                symlink: false,
            },
            2,
        )
        .unwrap();
        service.set_metadata_limit(Some(1));
        let grant = authority.grant();
        let object = service.root(&grant).unwrap();
        service.backend_mut().grants = vec![grant];
        let mut upload = service
            .begin_opaque(
                &object,
                MetadataClass::Security,
                &MetadataEntry {
                    key: "key".into(),
                    encoding: "format-v1".into(),
                    size: 1,
                },
            )
            .unwrap();
        service.write_opaque(&mut upload, &[0]).unwrap();
        service.finish_opaque(upload).unwrap();
        assert_eq!(service.backend_mut().calls, 3);
    }
    #[test]
    fn write_and_link_hold_all_admission_permits_through_backend_calls() {
        let (mut service, authority) = RestoreService::new(
            Probe {
                grants: vec![],
                calls: 0,
                symlink: false,
            },
            3,
        )
        .unwrap();
        let grant = authority.grant();
        let other = authority.grant();
        let now = Timespec {
            seconds: 1,
            nanoseconds: 0,
        };
        let root = service.root(&grant).unwrap();
        let file = service.create_file(&root, "file", now).unwrap();
        let other_root = service.root(&other).unwrap();
        service.backend_mut().grants = vec![grant.clone()];
        service.write(&file, 0, b"data", now).unwrap();
        service.set_reservation_limit(4096);
        service.reserve(&file, 0, 4096, now).unwrap();
        service.link(&file, &root, "same", now).unwrap();
        service.backend_mut().grants.push(other.clone());
        service.link(&file, &other_root, "other", now).unwrap();
        authority.revoke(&other).unwrap();
        assert_eq!(
            service.link(&file, &other_root, "denied", now),
            Err(RestoreError::Denied)
        );
        authority.revoke(&grant).unwrap();
        assert_eq!(
            service.write(&file, 0, b"denied", now),
            Err(RestoreError::Denied)
        );
        assert_eq!(service.backend_mut().calls, 4);
    }
}
