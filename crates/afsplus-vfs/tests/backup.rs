use afsplus_format::Timespec;
use afsplus_vfs::backup::*;
use afsplus_vfs::{DirectoryEntry, NodeKind, ObjectId, Stat, VfsError};
use std::collections::BTreeMap;
use std::sync::{mpsc, Arc};

fn now(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 123,
    }
}
fn metadata(id: u64, kind: NodeKind, size: u64) -> Stat {
    Stat {
        object_id: id,
        kind,
        size,
        allocated_size: size,
        links: 1,
        protection: 7,
        mode: 0,
        owner_uid: 0,
        owner_gid: 0,
        created: now(1),
        modified: now(2),
        changed: now(3),
        content_generation: 4,
    }
}
#[derive(Clone)]
struct Captured {
    attributes: BTreeMap<String, (String, Vec<u8>)>,
    security: BTreeMap<String, (String, Vec<u8>)>,
    inventory: MetadataInventory,
    file: Stat,
    data: Vec<u8>,
}
struct MockFs {
    live: Captured,
    views: BTreeMap<u64, Arc<Captured>>,
    next: u64,
    calls: usize,
    metadata_page_override: Option<MetadataPage>,
    oversized_metadata_read: bool,
    block_read: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
}
impl MockFs {
    fn new() -> Self {
        Self {
            live: Captured {
                attributes: BTreeMap::from([
                    (
                        "user.comment".into(),
                        ("text/utf8;v=1".into(), "café".as_bytes().to_vec()),
                    ),
                    (
                        "vendor.unknown".into(),
                        ("vendor/binary;v=900".into(), vec![0, 255, 1, 0, 128]),
                    ),
                ]),
                security: BTreeMap::new(),
                inventory: MetadataInventory {
                    attributes: InventoryKnowledge::Present,
                    security: InventoryKnowledge::Empty,
                },
                file: metadata(2, NodeKind::File, 6),
                data: b"before".to_vec(),
            },
            views: BTreeMap::new(),
            next: 1,
            calls: 0,
            block_read: None,
            metadata_page_override: None,
            oversized_metadata_read: false,
        }
    }
}
impl SnapshotBackend for MockFs {
    type View = (u64, Arc<Captured>);
    type Cursor = bool;
    fn create(&mut self, _: Timespec) -> Result<u64, VfsError> {
        self.calls += 1;
        let id = self.next;
        self.next += 1;
        self.views.insert(id, Arc::new(self.live.clone()));
        Ok(id)
    }
    fn delete(&mut self, id: u64, _: Timespec) -> Result<(), VfsError> {
        self.calls += 1;
        if Arc::strong_count(self.views.get(&id).ok_or(VfsError::NotFound)?) > 1 {
            return Err(VfsError::Busy);
        }
        self.views.remove(&id);
        Ok(())
    }
    fn list(&mut self, low_id: u64, limit: usize) -> Result<ViewPage, VfsError> {
        self.calls += 1;
        Ok(ViewPage {
            entries: self
                .views
                .range(low_id..)
                .take(limit)
                .map(|(&id, _)| ViewInfo {
                    id,
                    revision: id,
                    root: 1,
                })
                .collect(),
            next_id: None,
        })
    }
    fn open(&mut self, id: u64) -> Result<Self::View, VfsError> {
        self.calls += 1;
        Ok((id, self.views.get(&id).ok_or(VfsError::NotFound)?.clone()))
    }
    fn info(&mut self, view: &Self::View) -> Result<ViewInfo, VfsError> {
        self.calls += 1;
        Ok(ViewInfo {
            id: view.0,
            revision: view.0,
            root: 1,
        })
    }
    fn stat(&mut self, view: &Self::View, object: u64) -> Result<Stat, VfsError> {
        self.calls += 1;
        match object {
            1 => Ok(metadata(1, NodeKind::Directory, 0)),
            2 => Ok(view.1.file.clone()),
            _ => Err(VfsError::NotFound),
        }
    }
    fn metadata_inventory(
        &mut self,
        view: &Self::View,
        object: ObjectId,
    ) -> Result<MetadataInventory, VfsError> {
        self.calls += 1;
        if object != 2 {
            return Err(VfsError::NotFound);
        }
        Ok(view.1.inventory)
    }
    fn metadata_page(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        class: MetadataClass,
        after: Option<&str>,
        limit: usize,
    ) -> Result<MetadataPage, VfsError> {
        self.calls += 1;
        if let Some(page) = &self.metadata_page_override {
            return Ok(page.clone());
        }
        if object != 2 {
            return Err(VfsError::NotFound);
        }
        let map = match class {
            MetadataClass::Attribute => &view.1.attributes,
            MetadataClass::Security => &view.1.security,
        };
        let mut iter = map
            .iter()
            .filter(|(key, _)| after.is_none_or(|after| key.as_str() > after));
        let entries = iter
            .by_ref()
            .take(limit)
            .map(|(key, (encoding, bytes))| MetadataEntry {
                key: key.clone(),
                encoding: encoding.clone(),
                size: bytes.len() as u64,
            })
            .collect();
        Ok(MetadataPage {
            entries,
            eof: iter.next().is_none(),
        })
    }
    fn metadata_read(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        class: MetadataClass,
        key: &str,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError> {
        self.calls += 1;
        if self.oversized_metadata_read {
            return Ok(out.len() + 1);
        }
        if object != 2 {
            return Err(VfsError::NotFound);
        }
        let map = match class {
            MetadataClass::Attribute => &view.1.attributes,
            MetadataClass::Security => &view.1.security,
        };
        let bytes = &map.get(key).ok_or(VfsError::NotFound)?.1;
        if offset >= bytes.len() as u64 {
            return Ok(0);
        }
        let offset = offset as usize;
        let count = out.len().min(bytes.len() - offset);
        out[..count].copy_from_slice(&bytes[offset..offset + count]);
        Ok(count)
    }
    fn read(
        &mut self,
        view: &Self::View,
        object: u64,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError> {
        self.calls += 1;
        if let Some((entered, resume)) = self.block_read.take() {
            entered.send(()).unwrap();
            resume.recv().unwrap();
        }
        if object != 2 {
            return Err(VfsError::NotFound);
        }
        let offset = usize::try_from(offset)
            .map_err(|_| VfsError::Invalid)?
            .min(view.1.data.len());
        let n = out.len().min(view.1.data.len() - offset);
        out[..n].copy_from_slice(&view.1.data[offset..offset + n]);
        Ok(n)
    }
    fn allocations(
        &mut self,
        view: &Self::View,
        object: ObjectId,
        start: u64,
        _limit: usize,
    ) -> Result<AllocationPage, VfsError> {
        self.calls += 1;
        if object != 2 {
            return Err(VfsError::NotFound);
        }
        if start > 1 {
            return Err(VfsError::Invalid);
        }
        Ok(AllocationPage {
            ranges: if start == 0 {
                vec![AllocationRange {
                    offset: 0,
                    length: view.1.data.len() as u64,
                    unwritten: false,
                }]
            } else {
                vec![]
            },
            next: 1,
            eof: true,
        })
    }
    fn directory(
        &mut self,
        _: &Self::View,
        object: u64,
        cursor: Option<bool>,
        _: usize,
    ) -> Result<ViewDirectoryPage<bool>, VfsError> {
        self.calls += 1;
        if object != 1 {
            return Err(VfsError::NotDirectory);
        }
        Ok(ViewDirectoryPage {
            entries: if cursor == Some(true) {
                vec![]
            } else {
                vec![DirectoryEntry {
                    name: b"file".to_vec(),
                    object_id: 2,
                    kind: NodeKind::File,
                }]
            },
            next: true,
            eof: true,
        })
    }
}

/// Same semantic consumer runs against an independent filesystem and AFS+.
/// Its source has no AFS+ disk structures or private Volume calls.
fn collect<P: SnapshotBackend>(
    client: &mut BackupClient<'_, P>,
    grant: &BackupGrant,
    id: u64,
) -> Vec<(Vec<u8>, Stat, Vec<u8>)> {
    let reader = client.open(grant, id).unwrap();
    let root = client.info(&reader).unwrap().root;
    let mut cursor = None;
    let mut result = Vec::new();
    loop {
        let page = client.directory(&reader, root, cursor, 1).unwrap();
        for entry in page.entries {
            let stat = client.stat(&reader, entry.object_id).unwrap();
            let mut data = Vec::new();
            loop {
                let mut chunk = [0; 3];
                let n = client
                    .read(&reader, entry.object_id, data.len() as u64, &mut chunk)
                    .unwrap();
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&chunk[..n]);
            }
            let mut sparse = vec![0; stat.size as usize];
            let mut ordinal = 0;
            loop {
                let allocations = client
                    .allocations(&reader, entry.object_id, ordinal, 1)
                    .unwrap();
                for range in allocations.ranges {
                    if range.unwritten || range.offset >= stat.size {
                        continue;
                    }
                    let end = (range.offset + range.length.min(stat.size - range.offset)) as usize;
                    let begin = range.offset as usize;
                    assert_eq!(
                        client
                            .read(
                                &reader,
                                entry.object_id,
                                range.offset,
                                &mut sparse[begin..end]
                            )
                            .unwrap(),
                        end - begin
                    );
                }
                if allocations.eof {
                    break;
                }
                assert!(allocations.next > ordinal);
                ordinal = allocations.next;
            }
            assert_eq!(sparse, data);
            result.push((entry.name, stat, data));
        }
        if page.eof {
            break;
        }
        cursor = Some(page.next);
    }
    reader.close();
    result
}

#[test]
fn neutral_backup_keeps_capture_after_live_permission_and_content_changes() {
    let (mut service, authority) = BackupService::new(MockFs::new(), 2).unwrap();
    let grant = authority.grant();
    let id = service.client().create(&grant, now(4)).unwrap();
    let expected = collect(&mut service.client(), &grant, id);
    service.backend_mut().live.file.protection = 0;
    service.backend_mut().live.data.clear();
    assert_eq!(collect(&mut service.client(), &grant, id), expected);
    assert_eq!(expected[0].2, b"before");
    assert_eq!(expected[0].1.protection, 7);
}

#[test]
fn revocation_denies_every_operation_without_backend_calls_or_buffer_changes() {
    let (mut service, authority) = BackupService::new(MockFs::new(), 1).unwrap();
    let grant = authority.grant();
    let id = service.create(&grant, now(4)).unwrap();
    assert!(matches!(
        service.open(&grant, id + 1),
        Err(BackupError::Filesystem(VfsError::NotFound))
    ));
    let reader = service.open(&grant, id).unwrap();
    let duplicate = reader.clone();
    assert!(matches!(
        service.open(&grant, id),
        Err(BackupError::Filesystem(VfsError::Limit(_)))
    ));
    authority.revoke(&grant).unwrap();
    let calls = service.backend_mut().calls;
    let mut buffer = [0xa5; 8];
    assert_eq!(service.create(&grant, now(5)), Err(BackupError::Denied));
    assert_eq!(service.delete(&grant, id, now(5)), Err(BackupError::Denied));
    assert!(matches!(
        service.list(&grant, 0, 1),
        Err(BackupError::Denied)
    ));
    assert!(matches!(service.open(&grant, id), Err(BackupError::Denied)));
    assert_eq!(service.info(&reader), Err(BackupError::Denied));
    assert_eq!(service.stat(&reader, 2), Err(BackupError::Denied));
    assert_eq!(
        service.read(&duplicate, 2, 0, &mut buffer),
        Err(BackupError::Denied)
    );
    assert!(matches!(
        service.directory(&reader, 1, None, 1),
        Err(BackupError::Denied)
    ));
    assert_eq!(buffer, [0xa5; 8]);
    assert_eq!(service.backend_mut().calls, calls);
    let fresh = authority.grant();
    assert_eq!(service.info(&reader), Err(BackupError::Denied));
    assert_eq!(
        service.delete(&fresh, id, now(5)),
        Err(BackupError::Filesystem(VfsError::Busy))
    );
    reader.close();
    assert!(matches!(
        service.open(&fresh, id),
        Err(BackupError::Filesystem(VfsError::Limit(_)))
    ));
    duplicate.close();
    let new_reader = service.open(&fresh, id).unwrap();
    assert!(service.info(&new_reader).is_ok());
    new_reader.close();
    service.delete(&fresh, id, now(5)).unwrap();
    assert!(matches!(
        service.list(&grant, 0, 1),
        Err(BackupError::Denied)
    ));
}

#[test]
fn wrong_service_grants_readers_and_revokers_are_rejected() {
    assert!(matches!(
        BackupService::new(MockFs::new(), 0),
        Err(BackupError::Filesystem(VfsError::Limit(_)))
    ));
    let (mut one, a) = BackupService::new(MockFs::new(), 2).unwrap();
    let (mut two, b) = BackupService::new(MockFs::new(), 2).unwrap();
    let grant = a.grant();
    let id = one.create(&grant, now(4)).unwrap();
    let reader = one.open(&grant, id).unwrap();
    assert_eq!(b.revoke(&grant), Err(BackupError::Denied));
    assert!(matches!(two.open(&grant, id), Err(BackupError::Denied)));
    assert_eq!(two.info(&reader), Err(BackupError::Denied));
    assert_eq!(two.backend_mut().calls, 0);
    assert!(one.info(&reader).is_ok());
}

#[test]
fn revocation_waits_for_admitted_read_and_blocks_the_next_read() {
    let (mut service, authority) = BackupService::new(MockFs::new(), 1).unwrap();
    let grant = authority.grant();
    let id = service.create(&grant, now(4)).unwrap();
    let reader = service.open(&grant, id).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    service.backend_mut().block_read = Some((entered_tx, resume_rx));
    std::thread::scope(|scope| {
        let reading = scope.spawn(|| {
            let mut out = [0; 6];
            assert_eq!(service.read(&reader, 2, 0, &mut out).unwrap(), 6);
            assert_eq!(&out, b"before");
        });
        entered_rx.recv().unwrap();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (revoked_tx, revoked_rx) = mpsc::channel();
        let revoking = scope.spawn(move || {
            attempt_tx.send(()).unwrap();
            authority.revoke(&grant).unwrap();
            revoked_tx.send(()).unwrap();
        });
        attempt_rx.recv().unwrap();
        assert!(matches!(
            revoked_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        resume_tx.send(()).unwrap();
        reading.join().unwrap();
        revoking.join().unwrap();
        revoked_rx.recv().unwrap();
    });
    let mut out = [0xa5; 6];
    assert_eq!(
        service.read(&reader, 2, 0, &mut out),
        Err(BackupError::Denied)
    );
    assert_eq!(out, [0xa5; 6]);
}

#[test]
fn afsplus_uses_the_same_consumer_and_retains_history_through_remount() {
    use afsplus_block::{MemoryBackend, TraceBackend};
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{
        mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MkfsParams, MountOptions,
        NamePolicy,
    };
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [81; 16],
            label: "BackupAPI".into(),
            region_size: 1024,
            reclaim_caps: Default::default(),
            log_slots: 8,
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
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 128,
        reclaim_records: 8,
    };
    let mut volume = mount_with_snapshot_limits(dev, MountOptions::default(), limits).unwrap();
    let file = volume
        .create_file_in_root("file", b"before", now(2))
        .unwrap();
    let (mut service, authority) = BackupService::new(volume, 4).unwrap();
    let grant = authority.grant();
    let id = service.client().create(&grant, now(3)).unwrap();
    let expected = collect(&mut service.client(), &grant, id);
    service
        .backend_mut()
        .write_file_at(file, 0, b"after!", now(4))
        .unwrap();
    assert_eq!(service.backend_mut().read_file(file).unwrap(), b"after!");
    service
        .backend_mut()
        .set_object_protection(file, 0xffff_ffff, now(5))
        .unwrap();
    assert_eq!(
        service
            .backend_mut()
            .stat(file)
            .unwrap()
            .unwrap()
            .protection,
        0xffff_ffff
    );
    assert_eq!(collect(&mut service.client(), &grant, id), expected);
    service
        .backend_mut()
        .delete_file_in_root("file", now(6))
        .unwrap();
    let old_reader = service.open(&grant, id).unwrap();
    let dev = service.into_backend().into_device();
    let volume =
        mount_with_snapshot_limits(TraceBackend::new(dev), MountOptions::default(), limits)
            .unwrap();
    let (mut service, authority) = BackupService::new(volume, 4).unwrap();
    assert_eq!(service.info(&old_reader), Err(BackupError::Denied));
    assert!(matches!(service.open(&grant, id), Err(BackupError::Denied)));
    old_reader.close();
    let grant = authority.grant();
    assert_eq!(collect(&mut service.client(), &grant, id), expected);
    assert_eq!(service.backend_mut().lookup_root("file").unwrap(), None);
    let reader = service.open(&grant, id).unwrap();
    assert_eq!(
        service.metadata_page(&reader, file, MetadataClass::Attribute, None, 1),
        Err(BackupError::Filesystem(VfsError::NotSupported))
    );
    assert_eq!(
        service.metadata_read(
            &reader,
            file,
            MetadataClass::Security,
            "descriptor",
            0,
            &mut [0; 1]
        ),
        Err(BackupError::Filesystem(VfsError::NotSupported))
    );
    // Missing inventory enumeration must not certify empty metadata after remount.
    assert_eq!(
        service.client().metadata_inventory(&reader, file),
        Ok(MetadataInventory {
            attributes: InventoryKnowledge::Uninspected,
            security: InventoryKnowledge::Uninspected,
        })
    );
    authority.revoke(&grant).unwrap();
    assert_eq!(
        service.client().metadata_inventory(&reader, file),
        Err(BackupError::Denied)
    );
    assert_eq!(service.stat(&reader, file), Err(BackupError::Denied));
    reader.close();
    let mut traced = service.into_backend().into_device();
    assert_eq!(traced.stats().writes, 0);
    assert_eq!(traced.stats().flushes, 0);
    assert!(afsplus_check::check_device(&mut traced).is_clean());
}

#[test]
fn missing_captured_directory_child_is_corruption_not_an_absent_lookup() {
    use afsplus_block::{MemoryBackend, TraceBackend};
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{
        mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MkfsParams, MountMode,
        MountOptions, NamePolicy,
    };
    use afsplus_format::tree::{key_u64, TreeNode};
    let mut dev = MemoryBackend::new(4096, 512);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [82; 16],
            label: "BackupDamage".into(),
            region_size: 512,
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
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 128,
        reclaim_records: 8,
    };
    let mut volume = mount_with_snapshot_limits(dev, MountOptions::default(), limits).unwrap();
    let file = volume.create_file_in_root("file", b"data", now(2)).unwrap();
    let captured_map = volume.checkpoint().object_map_block;
    let id = volume.snapshot_create(now(3)).unwrap();
    let mut dev = volume.into_device();
    let (mut node, generation) = TreeNode::decode(&dev.peek(captured_map)).unwrap();
    assert_eq!(node.level, 0);
    let before = node.items.len();
    node.items.retain(|item| item.key != key_u64(file));
    assert_eq!(node.items.len() + 1, before);
    node.subtree_items = node.items.len() as u64;
    dev.apply_raw(captured_map, &node.encode(4096, generation).unwrap());
    let volume = mount_with_snapshot_limits(
        TraceBackend::new(dev),
        MountOptions {
            mode: MountMode::NoChanges,
            ..Default::default()
        },
        limits,
    )
    .unwrap();
    let (mut service, authority) = BackupService::new(volume, 1).unwrap();
    let grant = authority.grant();
    let reader = service.open(&grant, id).unwrap();
    let root = service.info(&reader).unwrap().root;
    assert_eq!(
        service.stat(&reader, file),
        Err(BackupError::Filesystem(VfsError::NotFound))
    );
    assert!(matches!(
        service.directory(&reader, root, None, 1),
        Err(BackupError::Filesystem(VfsError::Corrupt(_)))
    ));
    reader.close();
    let mut traced = service.into_backend().into_device();
    assert_eq!(traced.stats().writes, 0);
    assert_eq!(traced.stats().flushes, 0);
    assert!(!afsplus_check::check_device(&mut traced).is_clean());
}

#[test]
fn allocation_enumeration_is_neutral_bounded_and_revocable() {
    let (mut service, authority) = BackupService::new(MockFs::new(), 1).unwrap();
    let grant = authority.grant();
    let id = service.create(&grant, now(1)).unwrap();
    let reader = service.open(&grant, id).unwrap();
    let page = service.client().allocations(&reader, 2, 0, 1).unwrap();
    assert_eq!(
        page.ranges,
        vec![AllocationRange {
            offset: 0,
            length: 6,
            unwritten: false
        }]
    );
    let calls = service.backend_mut().calls;
    for limit in [0, 65, usize::MAX] {
        assert!(service.allocations(&reader, 2, 0, limit).is_err());
    }
    authority.revoke(&grant).unwrap();
    assert_eq!(
        service.allocations(&reader, 2, 0, 1),
        Err(BackupError::Denied)
    );
    assert_eq!(service.backend_mut().calls, calls);
}

#[test]
fn inventory_knowledge_is_captured_and_denial_never_reaches_provider() {
    let (mut service, authority) = BackupService::new(MockFs::new(), 2).unwrap();
    let grant = authority.grant();
    let id = service.client().create(&grant, now(1)).unwrap();
    let reader = service.client().open(&grant, id).unwrap();
    let captured = MetadataInventory {
        attributes: InventoryKnowledge::Present,
        security: InventoryKnowledge::Empty,
    };
    assert_eq!(
        service.client().metadata_inventory(&reader, 2),
        Ok(captured)
    );
    service.backend_mut().live.inventory = MetadataInventory {
        attributes: InventoryKnowledge::Empty,
        security: InventoryKnowledge::Present,
    };
    assert_eq!(
        service.client().metadata_inventory(&reader, 2),
        Ok(captured)
    );
    assert_eq!(
        service.client().metadata_inventory(&reader, 99),
        Err(BackupError::Filesystem(VfsError::NotFound))
    );
    let calls = service.backend_mut().calls;
    authority.revoke(&grant).unwrap();
    assert_eq!(
        service.client().metadata_inventory(&reader, 2),
        Err(BackupError::Denied)
    );
    assert_eq!(service.backend_mut().calls, calls);
}

#[test]
fn opaque_values_page_and_stream_without_understanding_their_encoding() {
    let mut backend = MockFs::new();
    backend.live.security.insert(
        "descriptor".into(),
        ("vendor/acl;v=77".into(), vec![255, 0, 42, 128]),
    );
    backend.live.inventory.security = InventoryKnowledge::Present;
    backend
        .live
        .attributes
        .insert("user.empty".into(), ("vendor/empty;v=1".into(), vec![]));
    let expected = [
        backend.live.attributes.clone(),
        backend.live.security.clone(),
    ];
    let (mut service, authority) = BackupService::new(backend, 1).unwrap();
    let grant = authority.grant();
    let id = service.create(&grant, now(1)).unwrap();
    let reader = service.open(&grant, id).unwrap();
    service.backend_mut().live.attributes.clear();
    service.backend_mut().live.security.clear();
    for (class, expected) in [MetadataClass::Attribute, MetadataClass::Security]
        .into_iter()
        .zip(expected)
    {
        let mut after: Option<String> = None;
        let mut found = BTreeMap::new();
        loop {
            let page = service
                .client()
                .metadata_page(&reader, 2, class, after.as_deref(), 1)
                .unwrap();
            for entry in page.entries {
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 2];
                    let count = service
                        .client()
                        .metadata_read(
                            &reader,
                            2,
                            class,
                            &entry.key,
                            bytes.len() as u64,
                            &mut buffer,
                        )
                        .unwrap();
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                assert_eq!(bytes.len() as u64, entry.size);
                after = Some(entry.key.clone());
                found.insert(entry.key, (entry.encoding, bytes));
            }
            if page.eof {
                break;
            }
        }
        assert_eq!(found, expected);
    }
    let calls = service.backend_mut().calls;
    authority.revoke(&grant).unwrap();
    let mut untouched = [99; 3];
    assert_eq!(
        service.client().metadata_read(
            &reader,
            2,
            MetadataClass::Security,
            "descriptor",
            0,
            &mut untouched
        ),
        Err(BackupError::Denied)
    );
    assert_eq!(untouched, [99; 3]);
    assert_eq!(
        service
            .client()
            .metadata_page(&reader, 2, MetadataClass::Attribute, None, 1),
        Err(BackupError::Denied)
    );
    assert_eq!(service.backend_mut().calls, calls);
}

#[test]
fn metadata_admission_rejects_bad_requests_and_provider_responses() {
    let (mut service, authority) = BackupService::new(MockFs::new(), 1).unwrap();
    let grant = authority.grant();
    let id = service.create(&grant, now(1)).unwrap();
    let reader = service.open(&grant, id).unwrap();
    let class = MetadataClass::Attribute;
    let calls = service.backend_mut().calls;
    for limit in [0, 65] {
        assert!(service
            .metadata_page(&reader, 2, class, None, limit)
            .is_err());
    }
    for key in ["".to_owned(), "a\0b".to_owned(), "x".repeat(1025)] {
        assert!(service
            .metadata_page(&reader, 2, class, Some(&key), 1)
            .is_err());
        assert!(service
            .metadata_read(&reader, 2, class, &key, 0, &mut [0; 1])
            .is_err());
    }
    assert!(service
        .metadata_read(&reader, 2, class, "key", u64::MAX, &mut [0; 2])
        .is_err());
    assert_eq!(service.backend_mut().calls, calls);
    let entry = MetadataEntry {
        key: "a".into(),
        encoding: "vendor/v1".into(),
        size: 0,
    };
    for page in [
        MetadataPage {
            entries: vec![],
            eof: false,
        },
        MetadataPage {
            entries: vec![entry.clone(), entry.clone()],
            eof: true,
        },
        MetadataPage {
            entries: vec![MetadataEntry {
                key: "".into(),
                ..entry.clone()
            }],
            eof: true,
        },
        MetadataPage {
            entries: vec![MetadataEntry {
                encoding: "x".repeat(129),
                ..entry.clone()
            }],
            eof: true,
        },
    ] {
        service.backend_mut().metadata_page_override = Some(page);
        assert!(matches!(
            service.metadata_page(&reader, 2, class, None, 2),
            Err(BackupError::Filesystem(VfsError::Corrupt(_)))
        ));
    }
    service.backend_mut().metadata_page_override = Some(MetadataPage {
        entries: vec![entry],
        eof: true,
    });
    assert!(matches!(
        service.metadata_page(&reader, 2, class, Some("a"), 1),
        Err(BackupError::Filesystem(VfsError::Corrupt(_)))
    ));
    service.backend_mut().oversized_metadata_read = true;
    assert!(matches!(
        service.metadata_read(&reader, 2, class, "key", 0, &mut [0; 1]),
        Err(BackupError::Filesystem(VfsError::Corrupt(_)))
    ));
}

#[test]
fn missing_symlink_provider_refuses_without_inventing_an_empty_target() {
    let mut backend = MockFs::new();
    backend.live.file.kind = NodeKind::Symlink;
    let (mut service, authority) = BackupService::new(backend, 1).unwrap();
    let grant = authority.grant();
    let id = service.create(&grant, now(1)).unwrap();
    let reader = service.open(&grant, id).unwrap();
    let mut bytes = [0x55; 8];
    assert_eq!(
        service.read_link(&reader, 2, &mut bytes),
        Err(BackupError::Filesystem(VfsError::NotSupported))
    );
    assert_eq!(bytes, [0x55; 8]);
    let (mut foreign, _) = BackupService::new(MockFs::new(), 1).unwrap();
    assert_eq!(
        foreign.read_link(&reader, 2, &mut bytes),
        Err(BackupError::Denied)
    );
    assert_eq!(foreign.backend_mut().calls, 0);
}
