//! Filesystem-neutral trusted-backup service (ADR-075).
//!
//! The host owns the authority and backend. Consumers receive grants and call
//! this service; OS adapters must authenticate callers before granting access.
//! This in-process primitive is not itself an operating-system security boundary.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, RwLockReadGuard,
};

use crate::authority::{Grant, Issuer, Scope};
use crate::{DirectoryEntry, ObjectId, Stat, VfsError};
use afsplus_format::Timespec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupError {
    Denied,
    Filesystem(VfsError),
}
impl From<VfsError> for BackupError {
    fn from(value: VfsError) -> Self {
        Self::Filesystem(value)
    }
}
impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => f.write_str("backup authority denied"),
            Self::Filesystem(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for BackupError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewInfo {
    pub id: u64,
    pub revision: u64,
    pub root: ObjectId,
}
pub struct ViewPage {
    pub entries: Vec<ViewInfo>,
    pub next_id: Option<u64>,
}
pub struct ViewDirectoryPage<C> {
    pub entries: Vec<DirectoryEntry>,
    pub next: C,
    pub eof: bool,
}

/// Trusted provider contract. Implementations expose semantic objects, never
/// allocation addresses. Hosts must not expose this unchecked interface to
/// untrusted backup clients. A cursor alone must not retain a view.
pub trait SnapshotBackend {
    type View;
    type Cursor: Clone;
    fn create(&mut self, now: Timespec) -> Result<u64, VfsError>;
    fn delete(&mut self, id: u64, now: Timespec) -> Result<(), VfsError>;
    fn list(&mut self, low_id: u64, limit: usize) -> Result<ViewPage, VfsError>;
    fn open(&mut self, id: u64) -> Result<Self::View, VfsError>;
    fn info(&mut self, view: &Self::View) -> Result<ViewInfo, VfsError>;
    fn stat(&mut self, view: &Self::View, object: ObjectId) -> Result<Stat, VfsError>;
    fn read(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError>;
    fn directory(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        cursor: Option<Self::Cursor>,
        limit: usize,
    ) -> Result<ViewDirectoryPage<Self::Cursor>, VfsError>;
}

/// Opaque authority delegated by the host. Copies share revocation state.
#[derive(Clone)]
pub struct BackupGrant(Grant);
/// Keep this issuer in the trusted host. Creating a new grant never reactivates
/// an old grant or a reader opened under it.
#[derive(Clone)]
pub struct BackupAuthority(Issuer);
impl BackupAuthority {
    pub fn grant(&self) -> BackupGrant {
        BackupGrant(self.0.grant())
    }
    /// Wait for admitted operations to finish, then revoke. Do not invoke
    /// synchronously from an operation using this grant.
    pub fn revoke(&self, grant: &BackupGrant) -> Result<(), BackupError> {
        self.0.revoke(&grant.0).map_err(|_| BackupError::Denied)
    }
}

struct ReaderState<V> {
    view: V,
    grant: BackupGrant,
    // Fields drop in declaration order: release the provider view before
    // returning its budget, even if provider cleanup takes time.
    _budget: ReaderBudget,
}
struct ReaderBudget(Arc<AtomicUsize>);
impl Drop for ReaderBudget {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
/// Opaque reader lease. Copies share a single server-side reader allocation;
/// deletion remains busy until the last copy closes, even after revocation.
pub struct BackupReader<V>(Arc<ReaderState<V>>);
impl<V> Clone for BackupReader<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<V> BackupReader<V> {
    /// Resource cleanup is deliberately independent of authorization.
    pub fn close(self) {
        drop(self);
    }
}

pub struct BackupService<P: SnapshotBackend> {
    backend: P,
    scope: Scope,
    active: Arc<AtomicUsize>,
    max_readers: usize,
}
impl<P: SnapshotBackend> BackupService<P> {
    /// Trusted host construction; callers choose a measured reader budget.
    pub fn new(backend: P, max_readers: usize) -> Result<(Self, BackupAuthority), BackupError> {
        if max_readers == 0 {
            return Err(VfsError::Limit("backup reader budget must be positive").into());
        }
        let (scope, issuer) = Scope::new();
        let authority = BackupAuthority(issuer);
        Ok((
            Self {
                backend,
                scope,
                active: Arc::new(AtomicUsize::new(0)),
                max_readers,
            },
            authority,
        ))
    }
    /// Give the consumer only checked operations, without the host backend hook.
    pub fn client(&mut self) -> BackupClient<'_, P> {
        BackupClient(self)
    }
    /// Privileged host integration hook, not part of the consumer interface.
    pub fn backend_mut(&mut self) -> &mut P {
        &mut self.backend
    }
    pub fn into_backend(self) -> P {
        self.backend
    }
    fn admit<'a>(&self, grant: &'a BackupGrant) -> Result<RwLockReadGuard<'a, bool>, BackupError> {
        self.scope.admit(&grant.0).map_err(|_| BackupError::Denied)
    }

    fn reader_permit<'a>(
        &self,
        reader: &'a BackupReader<P::View>,
    ) -> Result<RwLockReadGuard<'a, bool>, BackupError> {
        self.admit(&reader.0.grant)
    }
    pub fn create(&mut self, grant: &BackupGrant, now: Timespec) -> Result<u64, BackupError> {
        let _permit = self.admit(grant)?;
        Ok(self.backend.create(now)?)
    }
    pub fn delete(
        &mut self,
        grant: &BackupGrant,
        id: u64,
        now: Timespec,
    ) -> Result<(), BackupError> {
        let _permit = self.admit(grant)?;
        Ok(self.backend.delete(id, now)?)
    }
    pub fn list(
        &mut self,
        grant: &BackupGrant,
        low_id: u64,
        limit: usize,
    ) -> Result<ViewPage, BackupError> {
        let _permit = self.admit(grant)?;
        Ok(self.backend.list(low_id, limit)?)
    }
    pub fn open(
        &mut self,
        grant: &BackupGrant,
        id: u64,
    ) -> Result<BackupReader<P::View>, BackupError> {
        let _permit = self.admit(grant)?;
        // Only this &mut service increments; concurrent drops can only reduce it.
        if self.active.load(Ordering::Acquire) >= self.max_readers {
            return Err(VfsError::Limit("backup reader budget exhausted").into());
        }
        let view = self.backend.open(id)?;
        self.active.fetch_add(1, Ordering::AcqRel);
        Ok(BackupReader(Arc::new(ReaderState {
            view,
            grant: grant.clone(),
            _budget: ReaderBudget(self.active.clone()),
        })))
    }
    pub fn info(&mut self, reader: &BackupReader<P::View>) -> Result<ViewInfo, BackupError> {
        let _permit = self.reader_permit(reader)?;
        Ok(self.backend.info(&reader.0.view)?)
    }
    pub fn stat(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
    ) -> Result<Stat, BackupError> {
        let _permit = self.reader_permit(reader)?;
        Ok(self.backend.stat(&reader.0.view, object)?)
    }
    pub fn read(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, BackupError> {
        let _permit = self.reader_permit(reader)?;
        Ok(self.backend.read(&reader.0.view, object, offset, out)?)
    }
    pub fn directory(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        cursor: Option<P::Cursor>,
        limit: usize,
    ) -> Result<ViewDirectoryPage<P::Cursor>, BackupError> {
        let _permit = self.reader_permit(reader)?;
        Ok(self
            .backend
            .directory(&reader.0.view, object, cursor, limit)?)
    }
}

impl<D: afsplus_block::BlockDevice> SnapshotBackend for afsplus_core::Volume<D> {
    type View = afsplus_core::volume::SnapshotHandle;
    type Cursor = afsplus_core::volume::SnapshotDirectoryCursor;
    fn create(&mut self, now: Timespec) -> Result<u64, VfsError> {
        Ok(self.snapshot_create(now)?)
    }
    fn delete(&mut self, id: u64, now: Timespec) -> Result<(), VfsError> {
        Ok(self.snapshot_delete(id, now)?)
    }
    fn list(&mut self, low_id: u64, limit: usize) -> Result<ViewPage, VfsError> {
        let page = self.snapshot_list(low_id, limit)?;
        Ok(ViewPage {
            entries: page
                .entries
                .into_iter()
                .map(|s| ViewInfo {
                    id: s.id,
                    revision: s.generation,
                    root: afsplus_format::OBJECT_ROOT,
                })
                .collect(),
            next_id: page.next_id,
        })
    }
    fn open(&mut self, id: u64) -> Result<Self::View, VfsError> {
        Ok(self.snapshot_open(id)?)
    }
    fn info(&mut self, view: &Self::View) -> Result<ViewInfo, VfsError> {
        // Validate mount ownership through a checked root access.
        self.snapshot_stat(view, afsplus_format::OBJECT_ROOT)?
            .ok_or_else(|| VfsError::Corrupt("snapshot root object is missing".into()))?;
        let info = view.info();
        Ok(ViewInfo {
            id: info.id,
            revision: info.generation,
            root: afsplus_format::OBJECT_ROOT,
        })
    }
    fn stat(&mut self, view: &Self::View, object: ObjectId) -> Result<Stat, VfsError> {
        Ok(self
            .snapshot_stat(view, object)?
            .ok_or(VfsError::NotFound)?
            .into())
    }
    fn read(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError> {
        Ok(self.snapshot_read_file_at(view, object, offset, out)?)
    }
    fn directory(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        cursor: Option<Self::Cursor>,
        limit: usize,
    ) -> Result<ViewDirectoryPage<Self::Cursor>, VfsError> {
        let page = self.snapshot_read_directory_page(view, object, cursor, limit)?;
        let mut entries = Vec::with_capacity(page.entries.len());
        for entry in page.entries {
            let metadata = self.snapshot_stat(view, entry.child_id)?.ok_or_else(|| {
                VfsError::Corrupt("snapshot directory references a missing object".into())
            })?;
            entries.push(DirectoryEntry {
                name: entry.name,
                object_id: entry.child_id,
                kind: metadata.object_type.into(),
            });
        }
        Ok(ViewDirectoryPage {
            entries,
            next: page.next,
            eof: page.eof,
        })
    }
}

/// Consumer-facing facade. Its backend and grant issuer are inaccessible.
///
/// ```compile_fail
/// use afsplus_vfs::backup::{BackupClient, SnapshotBackend};
/// fn bypass<P: SnapshotBackend>(client: &mut BackupClient<'_, P>) {
///     client.backend_mut();
/// }
/// ```
pub struct BackupClient<'a, P: SnapshotBackend>(&'a mut BackupService<P>);
impl<P: SnapshotBackend> BackupClient<'_, P> {
    pub fn create(&mut self, grant: &BackupGrant, now: Timespec) -> Result<u64, BackupError> {
        self.0.create(grant, now)
    }
    pub fn delete(
        &mut self,
        grant: &BackupGrant,
        id: u64,
        now: Timespec,
    ) -> Result<(), BackupError> {
        self.0.delete(grant, id, now)
    }
    pub fn list(
        &mut self,
        grant: &BackupGrant,
        low_id: u64,
        limit: usize,
    ) -> Result<ViewPage, BackupError> {
        self.0.list(grant, low_id, limit)
    }
    pub fn open(
        &mut self,
        grant: &BackupGrant,
        id: u64,
    ) -> Result<BackupReader<P::View>, BackupError> {
        self.0.open(grant, id)
    }
    pub fn info(&mut self, reader: &BackupReader<P::View>) -> Result<ViewInfo, BackupError> {
        self.0.info(reader)
    }
    pub fn stat(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
    ) -> Result<Stat, BackupError> {
        self.0.stat(reader, object)
    }
    pub fn read(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, BackupError> {
        self.0.read(reader, object, offset, out)
    }
    pub fn directory(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        cursor: Option<P::Cursor>,
        limit: usize,
    ) -> Result<ViewDirectoryPage<P::Cursor>, BackupError> {
        self.0.directory(reader, object, cursor, limit)
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    struct Probe {
        grant: Option<BackupGrant>,
    }
    impl SnapshotBackend for Probe {
        type View = ();
        type Cursor = ();
        fn create(&mut self, _: Timespec) -> Result<u64, VfsError> {
            unreachable!()
        }
        fn delete(&mut self, _: u64, _: Timespec) -> Result<(), VfsError> {
            unreachable!()
        }
        fn list(&mut self, _: u64, _: usize) -> Result<ViewPage, VfsError> {
            unreachable!()
        }
        fn open(&mut self, _: u64) -> Result<(), VfsError> {
            Ok(())
        }
        fn info(&mut self, _: &()) -> Result<ViewInfo, VfsError> {
            unreachable!()
        }
        fn stat(&mut self, _: &(), _: ObjectId) -> Result<Stat, VfsError> {
            unreachable!()
        }
        fn read(&mut self, _: &(), _: ObjectId, _: u64, _: &mut [u8]) -> Result<usize, VfsError> {
            // A check followed by an immediately dropped guard fails here:
            // revocation must remain excluded during the actual backend call.
            assert!(matches!(
                self.grant.as_ref().unwrap().0 .0.active.try_write(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Ok(0)
        }
        fn directory(
            &mut self,
            _: &(),
            _: ObjectId,
            _: Option<()>,
            _: usize,
        ) -> Result<ViewDirectoryPage<()>, VfsError> {
            unreachable!()
        }
    }
    #[test]
    fn operation_permit_remains_held_inside_the_backend_call() {
        let (mut service, authority) = BackupService::new(Probe { grant: None }, 1).unwrap();
        let grant = authority.grant();
        service.backend_mut().grant = Some(grant.clone());
        let reader = service.open(&grant, 1).unwrap();
        assert_eq!(service.read(&reader, 1, 0, &mut []), Ok(0));
        authority.revoke(&grant).unwrap();
        assert_eq!(
            service.read(&reader, 1, 0, &mut []),
            Err(BackupError::Denied)
        );
    }
}
