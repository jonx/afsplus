use afsplus_format::Timespec;
use afsplus_vfs::backup::{AllocationPage, AllocationRange, MetadataClass, MetadataEntry};
use afsplus_vfs::restore::*;
use afsplus_vfs::{NodeKind, Stat, VfsError};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::{collections::BTreeMap, sync::mpsc};

fn now(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 123,
    }
}
fn metadata() -> RestoreMetadata {
    RestoreMetadata {
        protection: 0x8000_0001,
        created: now(-50),
        modified: now(70),
        changed: now(-20),
    }
}
fn stat(id: u64, kind: NodeKind) -> Stat {
    Stat {
        object_id: id,
        kind,
        size: 0,
        allocated_size: 0,
        links: 1,
        protection: 0,
        created: now(0),
        modified: now(0),
        changed: now(0),
        content_generation: 1,
    }
}
struct Node {
    stat: Stat,
    data: Vec<u8>,
}
struct Mock {
    opaque: BTreeMap<(u64, bool, String), (String, Vec<u8>)>,
    staging_active: Arc<AtomicUsize>,
    metadata_fault: u8,
    nodes: BTreeMap<u64, Node>,
    names: BTreeMap<(u64, String), u64>,
    next: u64,
    calls: usize,
    blocked_write: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    reservations: Vec<(u64, u64, u64)>,
    reservation_support: bool,
    allocation_reply: Option<AllocationPage>,
}
impl Mock {
    fn new() -> Self {
        Self {
            opaque: BTreeMap::new(),
            staging_active: Arc::new(AtomicUsize::new(0)),
            metadata_fault: 0,
            nodes: BTreeMap::from([(
                1,
                Node {
                    stat: stat(1, NodeKind::Directory),
                    data: vec![],
                },
            )]),
            names: BTreeMap::new(),
            next: 2,
            calls: 0,
            blocked_write: None,
            reservations: vec![],
            reservation_support: true,
            allocation_reply: None,
        }
    }
    fn create(&mut self, parent: u64, name: &str, kind: NodeKind) -> Result<u64, VfsError> {
        self.calls += 1;
        if self.nodes.get(&parent).ok_or(VfsError::NotFound)?.stat.kind != NodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        if self.names.contains_key(&(parent, name.into())) {
            return Err(VfsError::AlreadyExists);
        }
        let id = self.next;
        self.next += 1;
        self.nodes.insert(
            id,
            Node {
                stat: stat(id, kind),
                data: vec![],
            },
        );
        self.names.insert((parent, name.into()), id);
        Ok(id)
    }
}
impl RestoreBackend for Mock {
    type Object = u64;
    fn allocations(&mut self, object: &u64, _: u64, _: usize) -> Result<AllocationPage, VfsError> {
        self.calls += 1;
        self.nodes.get(object).ok_or(VfsError::NotFound)?;
        self.allocation_reply.clone().ok_or(VfsError::NotSupported)
    }
    fn root(&mut self) -> Result<u64, VfsError> {
        self.calls += 1;
        Ok(1)
    }
    fn create_file(&mut self, parent: &u64, name: &str, _: Timespec) -> Result<u64, VfsError> {
        self.create(*parent, name, NodeKind::File)
    }
    fn create_directory(&mut self, parent: &u64, name: &str, _: Timespec) -> Result<u64, VfsError> {
        self.create(*parent, name, NodeKind::Directory)
    }
    fn write(
        &mut self,
        object: &u64,
        offset: u64,
        bytes: &[u8],
        _: Timespec,
    ) -> Result<(), VfsError> {
        self.calls += 1;
        if let Some((entered, resume)) = self.blocked_write.take() {
            entered.send(()).unwrap();
            resume.recv().unwrap();
        }
        let node = self.nodes.get_mut(object).ok_or(VfsError::NotFound)?;
        let offset = usize::try_from(offset).map_err(|_| VfsError::Invalid)?;
        let end = offset.checked_add(bytes.len()).ok_or(VfsError::Invalid)?;
        node.data.resize(node.data.len().max(end), 0);
        node.data[offset..end].copy_from_slice(bytes);
        node.stat.size = node.data.len() as u64;
        Ok(())
    }
    fn reserve(
        &mut self,
        object: &u64,
        offset: u64,
        length: u64,
        _: Timespec,
    ) -> Result<(), VfsError> {
        self.calls += 1;
        if !self.reservation_support {
            return Err(VfsError::NotSupported);
        }
        let node = self.nodes.get_mut(object).ok_or(VfsError::NotFound)?;
        if node.stat.kind != NodeKind::File {
            return Err(VfsError::IsDirectory);
        }
        self.reservations.push((*object, offset, length));
        Ok(())
    }
    fn resize(&mut self, object: &u64, size: u64, _: Timespec) -> Result<(), VfsError> {
        self.calls += 1;
        let node = self.nodes.get_mut(object).ok_or(VfsError::NotFound)?;
        node.data
            .resize(usize::try_from(size).map_err(|_| VfsError::Invalid)?, 0);
        node.stat.size = size;
        Ok(())
    }
    fn link(
        &mut self,
        object: &u64,
        parent: &u64,
        name: &str,
        _: Timespec,
    ) -> Result<(), VfsError> {
        self.calls += 1;
        if self.names.contains_key(&(*parent, name.into())) {
            return Err(VfsError::AlreadyExists);
        }
        self.nodes
            .get_mut(object)
            .ok_or(VfsError::NotFound)?
            .stat
            .links += 1;
        self.names.insert((*parent, name.into()), *object);
        Ok(())
    }
    fn metadata(&mut self, object: &u64, m: RestoreMetadata) -> Result<(), VfsError> {
        self.calls += 1;
        let s = &mut self.nodes.get_mut(object).ok_or(VfsError::NotFound)?.stat;
        s.protection = m.protection;
        s.created = m.created;
        s.modified = m.modified;
        s.changed = m.changed;
        Ok(())
    }
    fn stat(&mut self, object: &u64) -> Result<Stat, VfsError> {
        self.calls += 1;
        Ok(self
            .nodes
            .get(object)
            .ok_or(VfsError::NotFound)?
            .stat
            .clone())
    }
    fn read(&mut self, object: &u64, offset: u64, out: &mut [u8]) -> Result<usize, VfsError> {
        self.calls += 1;
        let data = &self.nodes.get(object).ok_or(VfsError::NotFound)?.data;
        let offset = usize::try_from(offset)
            .map_err(|_| VfsError::Invalid)?
            .min(data.len());
        let n = out.len().min(data.len() - offset);
        out[..n].copy_from_slice(&data[offset..offset + n]);
        Ok(n)
    }
    fn sync(&mut self) -> Result<(), VfsError> {
        self.calls += 1;
        Ok(())
    }
}
struct Upload {
    object: u64,
    class: MetadataClass,
    entry: MetadataEntry,
    bytes: Vec<u8>,
    active: Arc<AtomicUsize>,
}
impl Drop for Upload {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl OpaqueRestoreBackend for Mock {
    type Upload = Upload;
    fn begin_opaque(
        &mut self,
        object: &u64,
        class: MetadataClass,
        entry: &MetadataEntry,
    ) -> Result<Upload, VfsError> {
        self.calls += 1;
        if !self.nodes.contains_key(object) {
            return Err(VfsError::NotFound);
        }
        self.staging_active.fetch_add(1, Ordering::SeqCst);
        Ok(Upload {
            object: *object,
            class,
            entry: entry.clone(),
            bytes: vec![],
            active: self.staging_active.clone(),
        })
    }
    fn write_opaque(
        &mut self,
        upload: &mut Upload,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), VfsError> {
        self.calls += 1;
        assert_eq!(offset, upload.bytes.len() as u64);
        if self.metadata_fault == 1 {
            upload.bytes.extend_from_slice(&bytes[..1]);
            return Err(VfsError::Invalid);
        }
        upload.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn finish_opaque(&mut self, mut upload: Upload) -> Result<(), VfsError> {
        self.calls += 1;
        assert_eq!(upload.bytes.len() as u64, upload.entry.size);
        if self.metadata_fault == 2 {
            return Err(VfsError::Invalid);
        }
        self.opaque.insert(
            (
                upload.object,
                upload.class == MetadataClass::Security,
                upload.entry.key.clone(),
            ),
            (
                upload.entry.encoding.clone(),
                std::mem::take(&mut upload.bytes),
            ),
        );
        if self.metadata_fault == 3 {
            return Err(VfsError::Invalid);
        }
        Ok(())
    }
}
fn assert_metadata(stat: &Stat) {
    let m = metadata();
    assert_eq!(stat.protection, m.protection);
    assert_eq!(stat.created, m.created);
    assert_eq!(stat.modified, m.modified);
    assert_eq!(stat.changed, m.changed);
}
/// The same logical restore job runs against the independent provider and AFS+.
fn restore_job<P: RestoreBackend>(
    client: &mut RestoreClient<'_, P>,
    grant: &RestoreGrant,
) -> (u64, u64) {
    let root = client.root(grant).unwrap();
    let dir = client.create_directory(&root, "restored", now(1)).unwrap();
    let file = client.create_file(&dir, "file", now(2)).unwrap();
    client.write(&file, 0, b"head", now(3)).unwrap();
    client.write(&file, 8192, b"tail", now(4)).unwrap();
    client.resize(&file, 12288, now(5)).unwrap();
    client.reserve(&file, 4096, 4096, now(5)).unwrap();
    client.reserve(&file, 16384, 8192, now(5)).unwrap();
    client.link(&file, &root, "alias", now(6)).unwrap();
    for object in [&file, &dir, &root] {
        client.metadata(object, metadata()).unwrap();
        assert_metadata(&client.stat(object).unwrap());
    }
    let file_stat = client.stat(&file).unwrap();
    assert_eq!(file_stat.links, 2);
    assert_eq!(file_stat.size, 12288);
    let dir_id = client.stat(&dir).unwrap().object_id;
    let mut out = vec![0xa5; 12288];
    assert_eq!(client.read(&file, 0, &mut out).unwrap(), out.len());
    let mut expected = vec![0; 12288];
    expected[..4].copy_from_slice(b"head");
    expected[8192..8196].copy_from_slice(b"tail");
    assert_eq!(out, expected);
    client.sync(grant).unwrap();
    file.close();
    dir.close();
    root.close();
    (file_stat.object_id, dir_id)
}
#[test]
fn neutral_restore_preserves_content_links_and_metadata() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 3).unwrap();
    let grant = authority.grant();
    service.set_reservation_limit(8192);
    restore_job(&mut service.client(), &grant);
    assert_eq!(service.backend_mut().names.len(), 3);
}
#[test]
fn handle_budget_and_name_validation_precede_creation_and_cleanup_restores_budget() {
    assert!(matches!(
        RestoreService::new(Mock::new(), 0),
        Err(RestoreError::Filesystem(VfsError::Limit(_)))
    ));
    let (mut service, authority) = RestoreService::new(Mock::new(), 2).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    for name in ["", ".", "..", "/outside", "a/b", "a\0b"] {
        let before = service.backend_mut().calls;
        assert!(matches!(
            service.create_file(&root, name, now(1)),
            Err(RestoreError::Filesystem(VfsError::Invalid))
        ));
        assert_eq!(service.backend_mut().calls, before);
    }
    let file = service.create_file(&root, "file", now(1)).unwrap();
    let duplicate = file.clone();
    let before = service.backend_mut().calls;
    assert!(matches!(
        service.create_directory(&root, "not-created", now(1)),
        Err(RestoreError::Filesystem(VfsError::Limit(_)))
    ));
    assert_eq!(service.backend_mut().calls, before);
    file.close();
    assert!(matches!(
        service.create_file(&root, "not-created", now(1)),
        Err(RestoreError::Filesystem(VfsError::Limit(_)))
    ));
    duplicate.close();
    assert!(matches!(
        service.create_file(&root, "file", now(1)),
        Err(RestoreError::Filesystem(VfsError::AlreadyExists))
    ));
    let other = service.create_file(&root, "other", now(1)).unwrap();
    other.close();
    root.close();
    assert!(!service
        .backend_mut()
        .names
        .contains_key(&(1, "not-created".into())));
}
#[test]
fn every_revoked_operation_is_denied_without_backend_effects() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 3).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let file = service.create_file(&root, "file", now(1)).unwrap();
    let duplicate = file.clone();
    authority.revoke(&grant).unwrap();
    let before = service.backend_mut().calls;
    let mut out = [0xa5; 8];
    assert!(matches!(service.root(&grant), Err(RestoreError::Denied)));
    assert!(matches!(
        service.create_file(&root, "x", now(2)),
        Err(RestoreError::Denied)
    ));
    assert!(matches!(
        service.create_directory(&root, "x", now(2)),
        Err(RestoreError::Denied)
    ));
    assert_eq!(
        service.write(&file, 0, b"x", now(2)),
        Err(RestoreError::Denied)
    );
    assert_eq!(service.resize(&file, 0, now(2)), Err(RestoreError::Denied));
    assert_eq!(
        service.link(&file, &root, "x", now(2)),
        Err(RestoreError::Denied)
    );
    assert_eq!(
        service.metadata(&file, metadata()),
        Err(RestoreError::Denied)
    );
    assert_eq!(service.stat(&file), Err(RestoreError::Denied));
    assert_eq!(
        service.read(&duplicate, 0, &mut out),
        Err(RestoreError::Denied)
    );
    assert_eq!(service.sync(&grant), Err(RestoreError::Denied));
    assert_eq!(out, [0xa5; 8]);
    assert_eq!(service.backend_mut().calls, before);
    let fresh = authority.grant();
    assert_eq!(service.stat(&file), Err(RestoreError::Denied));
    file.close();
    duplicate.close();
    root.close();
    service.root(&fresh).unwrap().close();
}
#[test]
fn foreign_handles_and_revoked_link_parents_cannot_expand_destination_authority() {
    let (mut one, a) = RestoreService::new(Mock::new(), 4).unwrap();
    let (mut two, b) = RestoreService::new(Mock::new(), 4).unwrap();
    let grant = a.grant();
    let root = one.root(&grant).unwrap();
    let file = one.create_file(&root, "file", now(1)).unwrap();
    assert_eq!(b.revoke(&grant), Err(RestoreError::Denied));
    assert!(matches!(two.root(&grant), Err(RestoreError::Denied)));
    assert_eq!(
        two.write(&file, 0, b"outside", now(2)),
        Err(RestoreError::Denied)
    );
    assert_eq!(two.backend_mut().calls, 0);
    let other = a.grant();
    let parent = one.root(&other).unwrap();
    one.link(&file, &parent, "allowed", now(2)).unwrap();
    a.revoke(&other).unwrap();
    let before = one.backend_mut().calls;
    assert_eq!(
        one.link(&file, &parent, "denied", now(2)),
        Err(RestoreError::Denied)
    );
    assert_eq!(one.backend_mut().calls, before);
    one.link(&file, &root, "same-grant", now(2)).unwrap();
}
#[test]
fn concurrent_revocation_drains_a_write_then_blocks_further_writes() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 2).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let file = service.create_file(&root, "file", now(1)).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    service.backend_mut().blocked_write = Some((entered_tx, resume_rx));
    std::thread::scope(|scope| {
        let writing = scope.spawn(|| service.write(&file, 0, b"admitted", now(2)).unwrap());
        entered_rx.recv().unwrap();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let revoking = scope.spawn(move || {
            attempt_tx.send(()).unwrap();
            authority.revoke(&grant).unwrap();
            done_tx.send(()).unwrap();
        });
        attempt_rx.recv().unwrap();
        assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        resume_tx.send(()).unwrap();
        writing.join().unwrap();
        revoking.join().unwrap();
        done_rx.recv().unwrap();
    });
    assert_eq!(
        service.write(&file, 0, b"denied", now(3)),
        Err(RestoreError::Denied)
    );
    assert_eq!(service.backend_mut().nodes[&2].data, b"admitted");
}

fn afs_volume() -> afsplus_core::Volume<afsplus_block::MemoryBackend> {
    use afsplus_core::{
        mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MkfsParams, MountOptions,
        NamePolicy,
    };
    let mut dev = afsplus_block::MemoryBackend::new(4096, 1024);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [84; 16],
            label: "RestoreScope".into(),
            region_size: 1024,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: now(1),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    mount_with_snapshot_limits(dev, MountOptions::default(), limits()).unwrap()
}
fn limits() -> afsplus_core::volume::SnapshotWorkLimits {
    afsplus_core::volume::SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 128,
        reclaim_records: 8,
    }
}
#[test]
fn afs_restore_is_confined_preserves_snapshot_and_survives_remount() {
    let mut volume = afs_volume();
    let outside = volume
        .create_file_in_root("outside", b"untouched", now(2))
        .unwrap();
    let destination = volume
        .create_directory_in_root("destination", now(3))
        .unwrap();
    let old_snapshot = volume.snapshot_create(now(4)).unwrap();
    let backend = AfsRestoreDestination::new(volume, destination).unwrap();
    let (mut service, authority) = RestoreService::new(backend, 3).unwrap();
    let grant = authority.grant();
    service.set_reservation_limit(8192);
    let (file, dir) = restore_job(&mut service.client(), &grant);
    let volume = service.into_backend().into_volume();
    let mut volume = afsplus_core::mount_with_snapshot_limits(
        volume.into_device(),
        afsplus_core::MountOptions::default(),
        limits(),
    )
    .unwrap();
    assert_eq!(volume.read_file(outside).unwrap(), b"untouched");
    assert_eq!(
        volume.lookup_in_directory(destination, "alias").unwrap(),
        Some(file)
    );
    assert_eq!(volume.lookup_in_directory(dir, "file").unwrap(), Some(file));
    for object in [destination, dir, file] {
        assert_metadata(&afsplus_vfs::Stat::from(
            afsplus_core::volume::ObjectMetadata::from(volume.stat(object).unwrap().unwrap()),
        ));
    }
    assert_eq!(volume.stat(file).unwrap().unwrap().allocated_bytes, 20480);
    let view = volume.snapshot_open(old_snapshot).unwrap();
    assert!(volume
        .snapshot_read_directory_page(&view, destination, None, 1)
        .unwrap()
        .entries
        .is_empty());
    let mut bytes = [0; 9];
    assert_eq!(
        volume
            .snapshot_read_file_at(&view, outside, 0, &mut bytes)
            .unwrap(),
        9
    );
    assert_eq!(&bytes, b"untouched");
    drop(view);
    let mut dev = volume.into_device();
    let report = afsplus_check::check_device(&mut dev);
    assert!(report.errors.is_empty());
    assert!(report.warnings.is_empty());
}
#[test]
fn afs_rejects_existing_destination_and_unrepresentable_metadata_without_writes() {
    use afsplus_block::TraceBackend;
    let mut volume = afs_volume();
    volume
        .create_file_in_root("existing", b"keep", now(2))
        .unwrap();
    let dev = volume.into_device();
    let volume = afsplus_core::mount_with_snapshot_limits(
        dev.clone(),
        afsplus_core::MountOptions::default(),
        limits(),
    )
    .unwrap();
    assert!(matches!(
        AfsRestoreDestination::new(volume, afsplus_format::OBJECT_ROOT),
        Err(VfsError::DirectoryNotEmpty)
    ));
    let mut volume = afsplus_core::mount_with_snapshot_limits(
        dev,
        afsplus_core::MountOptions::default(),
        limits(),
    )
    .unwrap();
    let dir = volume.create_directory_in_root("empty", now(3)).unwrap();
    let volume = afsplus_core::mount_with_snapshot_limits(
        TraceBackend::new(volume.into_device()),
        afsplus_core::MountOptions::default(),
        limits(),
    )
    .unwrap();
    let (mut service, authority) =
        RestoreService::new(AfsRestoreDestination::new(volume, dir).unwrap(), 2).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let mut too_wide = metadata();
    too_wide.protection = u64::MAX;
    assert_eq!(
        service.metadata(&root, too_wide),
        Err(RestoreError::Filesystem(VfsError::Invalid))
    );
    let mut invalid = metadata();
    invalid.modified.nanoseconds = 1_000_000_000;
    assert_eq!(
        service.metadata(&root, invalid),
        Err(RestoreError::Filesystem(VfsError::Invalid))
    );
    service.set_reservation_limit(4096);
    assert_eq!(
        service.reserve(&root, 1, 4096, now(4)),
        Err(RestoreError::Filesystem(VfsError::NotSupported))
    );
    assert_eq!(
        service.reserve(&root, 0, 4095, now(4)),
        Err(RestoreError::Filesystem(VfsError::NotSupported))
    );
    service.set_metadata_limit(Some(4));
    assert!(matches!(
        service.begin_opaque(&root, MetadataClass::Security, &opaque_entry(4)),
        Err(RestoreError::Filesystem(VfsError::NotSupported))
    ));
    let trace = service.into_backend().into_volume().into_device();
    assert_eq!(trace.stats().writes, 0);
    assert_eq!(trace.stats().flushes, 0);
}

#[test]
fn reservation_admission_is_bounded_revocable_and_reports_unsupported_providers() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 2).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let file = service.create_file(&root, "file", now(1)).unwrap();
    let before = service.backend_mut().calls;
    assert!(matches!(
        service.reserve(&file, 0, 4096, now(2)),
        Err(RestoreError::Filesystem(VfsError::Limit(_)))
    ));
    service.set_reservation_limit(8192);
    for (offset, length) in [(0, 0), (u64::MAX, 2), (0, 12288)] {
        assert!(service.reserve(&file, offset, length, now(2)).is_err());
    }
    assert_eq!(service.backend_mut().calls, before);
    service
        .client()
        .reserve(&file, 16384, 8192, now(2))
        .unwrap();
    assert_eq!(service.stat(&file).unwrap().size, 0);
    assert_eq!(service.backend_mut().reservations, vec![(2, 16384, 8192)]);
    service.backend_mut().reservation_support = false;
    assert_eq!(
        service.reserve(&file, 0, 4096, now(2)),
        Err(RestoreError::Filesystem(VfsError::NotSupported))
    );
    authority.revoke(&grant).unwrap();
    let before = service.backend_mut().calls;
    assert_eq!(
        service.reserve(&file, 0, 4096, now(2)),
        Err(RestoreError::Denied)
    );
    assert_eq!(service.backend_mut().calls, before);
}

#[test]
fn afs_restore_reservations_preserve_size_written_bytes_and_final_address_block() {
    let (mut service, authority) = RestoreService::new(
        AfsRestoreDestination::new(afs_volume(), afsplus_format::OBJECT_ROOT).unwrap(),
        2,
    )
    .unwrap();
    service.set_reservation_limit(8192);
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let file = service.create_file(&root, "reserved", now(2)).unwrap();
    service.write(&file, 0, b"keep", now(3)).unwrap();
    service.reserve(&file, 0, 8192, now(4)).unwrap();
    service.reserve(&file, 16384, 8192, now(4)).unwrap();
    service
        .reserve(&file, u64::MAX - 4095, 4096, now(4))
        .unwrap();
    let id = service.stat(&file).unwrap().object_id;
    assert_eq!(service.stat(&file).unwrap().size, 4);
    let mut volume = service.into_backend().into_volume();
    let snapshot = volume.snapshot_create(now(5)).unwrap();
    let mut volume = afsplus_core::mount_with_snapshot_limits(
        volume.into_device(),
        afsplus_core::MountOptions::default(),
        limits(),
    )
    .unwrap();
    assert_eq!(volume.read_file(id).unwrap(), b"keep");
    assert_eq!(volume.stat(id).unwrap().unwrap().size_bytes, 4);
    let view = volume.snapshot_open(snapshot).unwrap();
    let page = volume.snapshot_allocation_page(&view, id, 0, 64).unwrap();
    assert!(page.eof);
    let actual: Vec<_> = page
        .ranges
        .iter()
        .map(|r| (r.offset, r.length, r.unwritten))
        .collect();
    assert_eq!(
        actual,
        vec![
            (0, 4096, false),
            (4096, 4096, true),
            (16384, 8192, true),
            (u64::MAX - 4095, 4096, true)
        ]
    );
    let report = afsplus_check::check_device(&mut volume.into_device());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty());
}

fn opaque_entry(size: u64) -> MetadataEntry {
    MetadataEntry {
        key: "descriptor".into(),
        encoding: "vendor/unknown;v=7".into(),
        size,
    }
}
#[test]
fn staged_metadata_is_exact_private_and_releases_resources() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 2).unwrap();
    service.set_metadata_limit(Some(4));
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let mut upload = service
        .client()
        .begin_opaque(&root, MetadataClass::Security, &opaque_entry(4))
        .unwrap();
    root.close(); // Upload retains the object lease and both budget units.
    assert!(matches!(
        service.root(&grant),
        Err(RestoreError::Filesystem(VfsError::Limit(_)))
    ));
    service
        .client()
        .write_opaque(&mut upload, &[0, 255])
        .unwrap();
    assert!(service.backend_mut().opaque.is_empty());
    service
        .client()
        .write_opaque(&mut upload, &[128, 1])
        .unwrap();
    assert!(service.backend_mut().opaque.is_empty());
    service.client().finish_opaque(upload).unwrap();
    assert_eq!(
        service.backend_mut().opaque[&(1, true, "descriptor".into())],
        ("vendor/unknown;v=7".into(), vec![0, 255, 128, 1])
    );
    assert_eq!(
        service.backend_mut().staging_active.load(Ordering::SeqCst),
        0
    );
    let root = service.root(&grant).unwrap();
    let upload = service
        .begin_opaque(&root, MetadataClass::Attribute, &opaque_entry(0))
        .unwrap();
    service.finish_opaque(upload).unwrap();
    assert_eq!(
        service.backend_mut().opaque[&(1, false, "descriptor".into())].1,
        Vec::<u8>::new()
    );
}
#[test]
fn staged_metadata_admission_abort_and_revocation_are_explicit() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 2).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let calls = service.backend_mut().calls;
    assert!(matches!(
        service.begin_opaque(&root, MetadataClass::Security, &opaque_entry(0)),
        Err(RestoreError::Filesystem(VfsError::NotSupported))
    ));
    service.set_metadata_limit(Some(4));
    assert!(service
        .begin_opaque(&root, MetadataClass::Security, &opaque_entry(5))
        .is_err());
    assert!(service
        .begin_opaque(
            &root,
            MetadataClass::Security,
            &MetadataEntry {
                key: "".into(),
                ..opaque_entry(1)
            }
        )
        .is_err());
    assert_eq!(service.backend_mut().calls, calls);
    let mut upload = service
        .begin_opaque(&root, MetadataClass::Security, &opaque_entry(2))
        .unwrap();
    let calls = service.backend_mut().calls;
    assert!(service.write_opaque(&mut upload, &[0; 3]).is_err());
    assert_eq!(service.backend_mut().calls, calls);
    service.write_opaque(&mut upload, &[1]).unwrap();
    assert!(service.finish_opaque(upload).is_err());
    assert!(service.backend_mut().opaque.is_empty());
    assert_eq!(
        service.backend_mut().staging_active.load(Ordering::SeqCst),
        0
    );
    let mut upload = service
        .begin_opaque(&root, MetadataClass::Security, &opaque_entry(1))
        .unwrap();
    let (mut foreign, _) = RestoreService::new(Mock::new(), 2).unwrap();
    assert_eq!(
        foreign.write_opaque(&mut upload, &[1]),
        Err(RestoreError::Denied)
    );
    authority.revoke(&grant).unwrap();
    let calls = service.backend_mut().calls;
    assert_eq!(
        service.write_opaque(&mut upload, &[1]),
        Err(RestoreError::Denied)
    );
    assert_eq!(service.finish_opaque(upload), Err(RestoreError::Denied));
    assert_eq!(service.backend_mut().calls, calls);
    assert_eq!(
        service.backend_mut().staging_active.load(Ordering::SeqCst),
        0
    );
    assert!(service.backend_mut().opaque.is_empty());
}
#[test]
fn staging_write_failures_poison_upload_and_finish_errors_never_expose_partial_values() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 2).unwrap();
    service.set_metadata_limit(Some(2));
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    for fault in 1..=3 {
        let mut upload = service
            .begin_opaque(&root, MetadataClass::Security, &opaque_entry(2))
            .unwrap();
        service.backend_mut().metadata_fault = fault;
        if fault == 1 {
            assert!(service.write_opaque(&mut upload, &[0, 255]).is_err());
            let calls = service.backend_mut().calls;
            assert!(service.write_opaque(&mut upload, &[0, 255]).is_err());
            assert!(service.finish_opaque(upload).is_err());
            assert_eq!(service.backend_mut().calls, calls);
        } else {
            service.write_opaque(&mut upload, &[0, 255]).unwrap();
            assert!(service.finish_opaque(upload).is_err());
        }
        assert_eq!(
            service.backend_mut().staging_active.load(Ordering::SeqCst),
            0
        );
        if fault < 3 {
            assert!(service.backend_mut().opaque.is_empty());
        } else {
            assert_eq!(
                service.backend_mut().opaque[&(1, true, "descriptor".into())].1,
                vec![0, 255]
            );
        }
    }
    service.backend_mut().metadata_fault = 0;
    let upload = service
        .begin_opaque(&root, MetadataClass::Attribute, &opaque_entry(1))
        .unwrap();
    upload.abort();
    assert_eq!(
        service.backend_mut().staging_active.load(Ordering::SeqCst),
        0
    );
}

#[test]
fn allocation_readback_is_bounded_revocable_and_never_defaults_to_empty() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 4).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    assert_eq!(
        service.client().allocations(&root, 0, 1),
        Err(RestoreError::Filesystem(VfsError::NotSupported))
    );
    let before = service.backend_mut().calls;
    for limit in [0, 65] {
        assert!(service.client().allocations(&root, 0, limit).is_err());
    }
    assert_eq!(service.backend_mut().calls, before);
    let valid = AllocationPage {
        ranges: vec![AllocationRange {
            offset: u64::MAX - 4095,
            length: 4096,
            unwritten: true,
        }],
        next: 1,
        eof: true,
    };
    service.backend_mut().allocation_reply = Some(valid.clone());
    assert_eq!(service.client().allocations(&root, 0, 1).unwrap(), valid);
    let (mut other, _) = RestoreService::new(Mock::new(), 2).unwrap();
    assert_eq!(
        other.client().allocations(&root, 0, 1),
        Err(RestoreError::Denied)
    );
    assert_eq!(other.backend_mut().calls, 0);
    authority.revoke(&grant).unwrap();
    let before = service.backend_mut().calls;
    assert_eq!(
        service.client().allocations(&root, 0, 1),
        Err(RestoreError::Denied)
    );
    assert_eq!(service.backend_mut().calls, before);
}
#[test]
fn malformed_allocation_readback_pages_are_refused() {
    let (mut service, authority) = RestoreService::new(Mock::new(), 2).unwrap();
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    for fault in 0..6 {
        let mut page = AllocationPage {
            ranges: vec![AllocationRange {
                offset: 5,
                length: 2,
                unwritten: false,
            }],
            next: 1,
            eof: true,
        };
        match fault {
            0 => {
                page.ranges.push(page.ranges[0]);
                page.next = 2;
            }
            1 => {
                page.ranges.clear();
                page.next = 0;
                page.eof = false;
            }
            2 => page.next = 0,
            3 => page.ranges[0].length = 0,
            4 => {
                page.ranges.push(AllocationRange {
                    offset: 6,
                    length: 2,
                    unwritten: true,
                });
                page.next = 2;
            }
            _ => {
                page.ranges[0].offset = u64::MAX;
                page.ranges[0].length = 2;
            }
        }
        service.backend_mut().allocation_reply = Some(page);
        assert!(
            matches!(
                service
                    .client()
                    .allocations(&root, 0, if fault == 4 { 2 } else { 1 }),
                Err(RestoreError::Filesystem(VfsError::Corrupt(_)))
            ),
            "fault {fault}"
        );
    }
}
#[test]
fn afs_allocation_readback_matches_committed_pages_and_survives_remount() {
    let backend = AfsRestoreDestination::new(afs_volume(), afsplus_format::OBJECT_ROOT).unwrap();
    let (mut service, authority) = RestoreService::new(backend, 2).unwrap();
    service.set_reservation_limit(8192);
    let grant = authority.grant();
    let root = service.root(&grant).unwrap();
    let file = service.create_file(&root, "layout", now(2)).unwrap();
    service.write(&file, 4096, b"written", now(3)).unwrap();
    service.reserve(&file, 4 * 4096, 8192, now(4)).unwrap();
    service
        .reserve(&file, u64::MAX - 4095, 4096, now(5))
        .unwrap();
    let id = service.stat(&file).unwrap().object_id;
    let expected = vec![
        AllocationRange {
            offset: 4096,
            length: 4096,
            unwritten: false,
        },
        AllocationRange {
            offset: 4 * 4096,
            length: 8192,
            unwritten: true,
        },
        AllocationRange {
            offset: u64::MAX - 4095,
            length: 4096,
            unwritten: true,
        },
    ];
    let mut actual = Vec::new();
    let mut cursor = 0;
    loop {
        let page = service.client().allocations(&file, cursor, 1).unwrap();
        actual.extend(page.ranges);
        cursor = page.next;
        if page.eof {
            break;
        }
    }
    assert_eq!(actual, expected);
    service.client().sync(&grant).unwrap();
    let volume = service.into_backend().into_volume();
    let mut volume = afsplus_core::mount_with_snapshot_limits(
        volume.into_device(),
        afsplus_core::MountOptions::default(),
        limits(),
    )
    .unwrap();
    let page = volume.file_allocation_page(id, 0, 64).unwrap();
    assert_eq!(
        page.ranges
            .iter()
            .map(|r| (r.offset, r.length, r.unwritten))
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|r| (r.offset, r.length, r.unwritten))
            .collect::<Vec<_>>()
    );
    assert_eq!(volume.stat(id).unwrap().unwrap().size_bytes, 4103);
}
