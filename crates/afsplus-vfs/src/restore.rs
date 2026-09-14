//! Destination-scoped restoration, separately authorized under ADR-077.
use crate::authority::{Grant, Issuer, Scope};
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
    fn stat(&mut self, object: &Self::Object) -> Result<Stat, VfsError>;
    fn read(
        &mut self,
        object: &Self::Object,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError>;
    fn sync(&mut self) -> Result<(), VfsError>;
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

pub struct RestoreService<P: RestoreBackend> {
    backend: P,
    scope: Scope,
    active: Arc<AtomicUsize>,
    max_handles: usize,
    max_reservation_bytes: u64,
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
            },
            RestoreAuthority(issuer),
        ))
    }
    /// Host-owned per-operation admission limit. Zero disables reservation.
    /// Consumers cannot raise it through the checked facade.
    pub fn set_reservation_limit(&mut self, bytes: u64) {
        self.max_reservation_bytes = bytes;
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
impl<D: afsplus_block::BlockDevice> RestoreBackend for AfsRestoreDestination<D> {
    type Object = u64;
    fn root(&mut self) -> Result<u64, VfsError> {
        Ok(self.root)
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
        Ok(self.volume.truncate_file(*object, size, now)?)
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
        fn stat(&mut self, _: &()) -> Result<Stat, VfsError> {
            unreachable!()
        }
        fn read(&mut self, _: &(), _: u64, _: &mut [u8]) -> Result<usize, VfsError> {
            unreachable!()
        }
        fn sync(&mut self) -> Result<(), VfsError> {
            unreachable!()
        }
    }
    #[test]
    fn write_and_link_hold_all_admission_permits_through_backend_calls() {
        let (mut service, authority) = RestoreService::new(
            Probe {
                grants: vec![],
                calls: 0,
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
