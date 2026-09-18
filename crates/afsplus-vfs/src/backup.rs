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

/// Semantic allocation ranges; gaps are holes. Reservations and rounded tails
/// may extend beyond the logical size reported by stat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationRange {
    pub offset: u64,
    pub length: u64,
    pub unwritten: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationPage {
    pub ranges: Vec<AllocationRange>,
    pub next: u64,
    pub eof: bool,
}

/// Knowledge about one captured metadata inventory, not transport completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryKnowledge {
    /// The provider inspected the complete inventory and found no entries.
    Empty,
    /// Entries exist; their complete lossless transport is required separately.
    Present,
    /// The provider cannot establish whether entries exist or are complete.
    Uninspected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataInventory {
    pub attributes: InventoryKnowledge,
    pub security: InventoryKnowledge,
}

/// Preservation channel; security values are never treated as user attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataClass {
    Attribute,
    Security,
}
/// Exact opaque identity and encoding; neither string is a host pathname.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEntry {
    pub key: String,
    pub encoding: String,
    pub size: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataPage {
    pub entries: Vec<MetadataEntry>,
    pub eof: bool,
}
pub const MAX_METADATA_KEY_BYTES: usize = 1024;
pub const MAX_METADATA_ENCODING_BYTES: usize = 128;
pub const MAX_METADATA_PAGE_ENTRIES: usize = 64;
pub(crate) fn valid_metadata_text(text: &str, limit: usize) -> bool {
    !text.is_empty() && text.len() <= limit && !text.contains('\0')
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
    /// The captured object's comment, empty when it has none. A provider that
    /// cannot say refuses, and so does the backup: under ADR-076 a comment is
    /// preserved or the export refused, never dropped.
    fn comment(&mut self, _view: &Self::View, _object: ObjectId) -> Result<String, VfsError> {
        Err(VfsError::NotSupported)
    }
    /// Inspect the captured object's inventory knowledge. Missing enumeration
    /// never implies absence. The fallback validates object/view through stat.
    fn metadata_inventory(
        &mut self,
        view: &Self::View,
        object: ObjectId,
    ) -> Result<MetadataInventory, VfsError> {
        self.stat(view, object)?;
        Ok(MetadataInventory {
            attributes: InventoryKnowledge::Uninspected,
            security: InventoryKnowledge::Uninspected,
        })
    }
    /// Exact UTF-8 byte ordering, strictly after the supplied key. Unsupported
    /// enumeration returns NotSupported, never an empty successful page.
    fn metadata_page(
        &mut self,
        _view: &Self::View,
        _object: ObjectId,
        _class: MetadataClass,
        _after: Option<&str>,
        _limit: usize,
    ) -> Result<MetadataPage, VfsError> {
        Err(VfsError::NotSupported)
    }
    /// Read exact opaque bytes from the captured value, without interpretation.
    fn metadata_read(
        &mut self,
        _view: &Self::View,
        _object: ObjectId,
        _class: MetadataClass,
        _key: &str,
        _offset: u64,
        _out: &mut [u8],
    ) -> Result<usize, VfsError> {
        Err(VfsError::NotSupported)
    }
    /// Return required target bytes without changing a short buffer; never follow it.
    fn read_link(
        &mut self,
        _view: &Self::View,
        _object: ObjectId,
        _out: &mut [u8],
    ) -> Result<usize, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn read(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError>;
    fn allocations(
        &mut self,
        _view: &Self::View,
        _object: ObjectId,
        _start: u64,
        _limit: usize,
    ) -> Result<AllocationPage, VfsError> {
        Err(VfsError::NotSupported)
    }
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
    pub fn comment(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
    ) -> Result<String, BackupError> {
        let _permit = self.reader_permit(reader)?;
        Ok(self.backend.comment(&reader.0.view, object)?)
    }
    pub fn metadata_inventory(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
    ) -> Result<MetadataInventory, BackupError> {
        let _permit = self.reader_permit(reader)?;
        Ok(self.backend.metadata_inventory(&reader.0.view, object)?)
    }
    pub fn metadata_page(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        class: MetadataClass,
        after: Option<&str>,
        limit: usize,
    ) -> Result<MetadataPage, BackupError> {
        let _permit = self.reader_permit(reader)?;
        if limit == 0 || limit > MAX_METADATA_PAGE_ENTRIES {
            return Err(VfsError::Limit("metadata page limit out of range").into());
        }
        if after.is_some_and(|key| !valid_metadata_text(key, MAX_METADATA_KEY_BYTES)) {
            return Err(VfsError::Invalid.into());
        }
        let page = self
            .backend
            .metadata_page(&reader.0.view, object, class, after, limit)?;
        if page.entries.len() > limit || (page.entries.is_empty() && !page.eof) {
            return Err(VfsError::Corrupt("invalid metadata page size or progress".into()).into());
        }
        let mut previous = after;
        for entry in &page.entries {
            if !valid_metadata_text(&entry.key, MAX_METADATA_KEY_BYTES)
                || !valid_metadata_text(&entry.encoding, MAX_METADATA_ENCODING_BYTES)
                || previous.is_some_and(|key| entry.key.as_str() <= key)
            {
                return Err(
                    VfsError::Corrupt("invalid metadata identity or ordering".into()).into(),
                );
            }
            previous = Some(&entry.key);
        }
        Ok(page)
    }
    pub fn metadata_read(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        class: MetadataClass,
        key: &str,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, BackupError> {
        let _permit = self.reader_permit(reader)?;
        if !valid_metadata_text(key, MAX_METADATA_KEY_BYTES)
            || offset as u128 + out.len() as u128 > u64::MAX as u128 + 1
        {
            return Err(VfsError::Invalid.into());
        }
        let count = self
            .backend
            .metadata_read(&reader.0.view, object, class, key, offset, out)?;
        if count > out.len() {
            return Err(VfsError::Corrupt("metadata read exceeds caller buffer".into()).into());
        }
        Ok(count)
    }
    pub fn read_link(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        out: &mut [u8],
    ) -> Result<usize, BackupError> {
        let _permit = self.reader_permit(reader)?;
        if self.backend.stat(&reader.0.view, object)?.kind != crate::NodeKind::Symlink {
            return Err(VfsError::Invalid.into());
        }
        Ok(self.backend.read_link(&reader.0.view, object, out)?)
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
    pub fn allocations(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, BackupError> {
        let _permit = self.reader_permit(reader)?;
        if limit == 0 || limit > 64 {
            return Err(VfsError::Limit("allocation page limit out of range").into());
        }
        Ok(self
            .backend
            .allocations(&reader.0.view, object, start, limit)?)
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
    fn comment(&mut self, view: &Self::View, object: ObjectId) -> Result<String, VfsError> {
        self.snapshot_object_comment(view, object)?
            .ok_or(VfsError::NotFound)
    }
    fn read_link(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        out: &mut [u8],
    ) -> Result<usize, VfsError> {
        Ok(self.snapshot_read_link(view, object, out)?)
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
    fn allocations(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, VfsError> {
        let page = self.snapshot_allocation_page(view, object, start, limit)?;
        Ok(AllocationPage {
            ranges: page
                .ranges
                .into_iter()
                .map(|r| AllocationRange {
                    offset: r.offset,
                    length: r.length,
                    unwritten: r.unwritten,
                })
                .collect(),
            next: page.next,
            eof: page.eof,
        })
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
    pub fn comment(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
    ) -> Result<String, BackupError> {
        self.0.comment(reader, object)
    }
    pub fn metadata_inventory(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
    ) -> Result<MetadataInventory, BackupError> {
        self.0.metadata_inventory(reader, object)
    }
    pub fn metadata_page(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        class: MetadataClass,
        after: Option<&str>,
        limit: usize,
    ) -> Result<MetadataPage, BackupError> {
        self.0.metadata_page(reader, object, class, after, limit)
    }
    pub fn metadata_read(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        class: MetadataClass,
        key: &str,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, BackupError> {
        self.0
            .metadata_read(reader, object, class, key, offset, out)
    }
    pub fn read_link(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        out: &mut [u8],
    ) -> Result<usize, BackupError> {
        self.0.read_link(reader, object, out)
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
    pub fn allocations(
        &mut self,
        reader: &BackupReader<P::View>,
        object: ObjectId,
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, BackupError> {
        self.0.allocations(reader, object, start, limit)
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
        symlink: bool,
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
        fn stat(&mut self, _: &(), object: ObjectId) -> Result<Stat, VfsError> {
            assert!(matches!(
                self.grant.as_ref().unwrap().0 .0.active.try_write(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            if object != 1 {
                return Err(VfsError::NotFound);
            }
            Ok(Stat {
                object_id: object,
                kind: if self.symlink {
                    crate::NodeKind::Symlink
                } else {
                    crate::NodeKind::File
                },
                size: 0,
                allocated_size: 0,
                links: 1,
                protection: 0,
                mode: 0,
                owner_uid: 0,
                owner_gid: 0,
                created: Timespec {
                    seconds: 0,
                    nanoseconds: 0,
                },
                modified: Timespec {
                    seconds: 0,
                    nanoseconds: 0,
                },
                changed: Timespec {
                    seconds: 0,
                    nanoseconds: 0,
                },
                content_generation: 1,
            })
        }
        fn metadata_page(
            &mut self,
            _: &(),
            _: ObjectId,
            _: MetadataClass,
            _: Option<&str>,
            _: usize,
        ) -> Result<MetadataPage, VfsError> {
            assert!(matches!(
                self.grant.as_ref().unwrap().0 .0.active.try_write(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Err(VfsError::NotSupported)
        }
        fn metadata_read(
            &mut self,
            _: &(),
            _: ObjectId,
            _: MetadataClass,
            _: &str,
            _: u64,
            _: &mut [u8],
        ) -> Result<usize, VfsError> {
            assert!(matches!(
                self.grant.as_ref().unwrap().0 .0.active.try_write(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Err(VfsError::NotSupported)
        }
        fn allocations(
            &mut self,
            _: &(),
            _: ObjectId,
            _: u64,
            _: usize,
        ) -> Result<AllocationPage, VfsError> {
            assert!(matches!(
                self.grant.as_ref().unwrap().0 .0.active.try_write(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Ok(AllocationPage {
                ranges: vec![],
                next: 0,
                eof: true,
            })
        }
        fn read_link(&mut self, _: &(), _: ObjectId, _: &mut [u8]) -> Result<usize, VfsError> {
            assert!(matches!(
                self.grant.as_ref().unwrap().0 .0.active.try_write(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Ok(1)
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
    fn symlink_target_read_holds_grant_through_stat_and_read() {
        let (mut service, authority) = BackupService::new(
            Probe {
                grant: None,
                symlink: true,
            },
            1,
        )
        .unwrap();
        let grant = authority.grant();
        service.backend_mut().grant = Some(grant.clone());
        let reader = service.open(&grant, 1).unwrap();
        assert_eq!(service.client().read_link(&reader, 1, &mut []), Ok(1));
        authority.revoke(&grant).unwrap();
        assert_eq!(
            service.read_link(&reader, 1, &mut []),
            Err(BackupError::Denied)
        );
    }
    #[test]
    fn operation_permit_remains_held_inside_the_backend_call() {
        let (mut service, authority) = BackupService::new(
            Probe {
                grant: None,
                symlink: false,
            },
            1,
        )
        .unwrap();
        let grant = authority.grant();
        service.backend_mut().grant = Some(grant.clone());
        let reader = service.open(&grant, 1).unwrap();
        assert_eq!(service.read(&reader, 1, 0, &mut []), Ok(0));
        assert!(service.allocations(&reader, 1, 0, 1).unwrap().eof);
        assert_eq!(
            service.metadata_page(&reader, 1, MetadataClass::Attribute, None, 1),
            Err(BackupError::Filesystem(VfsError::NotSupported))
        );
        assert_eq!(
            service.metadata_read(
                &reader,
                1,
                MetadataClass::Security,
                "descriptor",
                0,
                &mut []
            ),
            Err(BackupError::Filesystem(VfsError::NotSupported))
        );
        authority.revoke(&grant).unwrap();
        assert_eq!(
            service.read(&reader, 1, 0, &mut []),
            Err(BackupError::Denied)
        );
    }
    #[test]
    fn missing_inventory_support_is_unknown_and_admission_covers_fallback() {
        let (mut service, authority) = BackupService::new(
            Probe {
                grant: None,
                symlink: false,
            },
            1,
        )
        .unwrap();
        let grant = authority.grant();
        service.backend_mut().grant = Some(grant.clone());
        let reader = service.client().open(&grant, 1).unwrap();
        let unknown = MetadataInventory {
            attributes: InventoryKnowledge::Uninspected,
            security: InventoryKnowledge::Uninspected,
        };
        assert_eq!(service.client().metadata_inventory(&reader, 1), Ok(unknown));
        assert_eq!(
            service.client().metadata_inventory(&reader, 2),
            Err(BackupError::Filesystem(VfsError::NotFound))
        );
        let (mut other, _) = BackupService::new(
            Probe {
                grant: None,
                symlink: false,
            },
            1,
        )
        .unwrap();
        // Its provider would panic if wrong-service admission reached stat.
        assert_eq!(
            other.client().metadata_inventory(&reader, 1),
            Err(BackupError::Denied)
        );
        authority.revoke(&grant).unwrap();
        assert_eq!(
            service.client().metadata_inventory(&reader, 1),
            Err(BackupError::Denied)
        );
        let fresh = authority.grant();
        assert_eq!(
            service.client().metadata_inventory(&reader, 1),
            Err(BackupError::Denied)
        );
        service.backend_mut().grant = Some(fresh.clone());
        reader.close();
        let reader = service.client().open(&fresh, 1).unwrap();
        assert_eq!(service.client().metadata_inventory(&reader, 1), Ok(unknown));
    }
}
