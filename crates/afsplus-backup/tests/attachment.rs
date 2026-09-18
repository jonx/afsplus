#![cfg(feature = "consumer")]
use afsplus_backup::{attachment::*, envelope, pax, stream, tar};
use afsplus_format::Timespec;
use afsplus_vfs::backup::*;
use afsplus_vfs::restore::*;
use afsplus_vfs::{Stat, VfsError};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone)]
struct Source {
    bytes: Vec<u8>,
    entries: usize,
    uninspected: bool,
    page_calls: usize,
    mutate_second_pass: bool,
    directory: bool,
    symlink: Option<String>,
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            entries: 1,
            uninspected: false,
            page_calls: 0,
            mutate_second_pass: false,
            directory: false,
            symlink: None,
        }
    }
}
impl SnapshotBackend for Source {
    type View = Self;
    type Cursor = ();
    fn create(&mut self, _: Timespec) -> Result<u64, VfsError> {
        Ok(1)
    }
    fn delete(&mut self, _: u64, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn list(&mut self, _: u64, _: usize) -> Result<ViewPage, VfsError> {
        unimplemented!()
    }
    fn open(&mut self, _: u64) -> Result<Self::View, VfsError> {
        Ok(self.clone())
    }
    fn info(&mut self, _: &Self::View) -> Result<ViewInfo, VfsError> {
        unimplemented!()
    }
    fn comment(&mut self, _: &Self::View, _: u64) -> Result<String, VfsError> {
        Ok(String::new())
    }
    fn stat(&mut self, view: &Self::View, _: u64) -> Result<Stat, VfsError> {
        let mut stat = file_stat();
        if view.directory {
            stat.kind = afsplus_vfs::NodeKind::Directory;
            stat.size = 0;
            stat.allocated_size = 0;
        }
        if let Some(target) = &view.symlink {
            stat.kind = afsplus_vfs::NodeKind::Symlink;
            stat.size = target.len() as u64;
            stat.allocated_size = 0;
        }
        Ok(stat)
    }
    fn read_link(&mut self, view: &Self::View, _: u64, out: &mut [u8]) -> Result<usize, VfsError> {
        let target = view.symlink.as_ref().ok_or(VfsError::NotSupported)?;
        if out.len() >= target.len() {
            out[..target.len()].copy_from_slice(target.as_bytes());
        }
        Ok(target.len())
    }
    fn read(
        &mut self,
        _: &Self::View,
        _: u64,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError> {
        let n = out.len().min((file_stat().size - offset) as usize);
        for (i, b) in out[..n].iter_mut().enumerate() {
            *b = match offset + i as u64 {
                4096 => b'c',
                4097 => 0,
                4098 => b'o',
                4099 => b'r',
                _ => 0,
            };
        }
        Ok(n)
    }
    fn allocations(
        &mut self,
        _: &Self::View,
        _: u64,
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, VfsError> {
        let all = [
            AllocationRange {
                offset: 4096,
                length: 4,
                unwritten: false,
            },
            AllocationRange {
                offset: 8192,
                length: 4096,
                unwritten: true,
            },
        ];
        let ranges: Vec<_> = all.into_iter().skip(start as usize).take(limit).collect();
        let next = start + ranges.len() as u64;
        Ok(AllocationPage {
            ranges,
            next,
            eof: next >= 2,
        })
    }
    fn directory(
        &mut self,
        _: &Self::View,
        _: u64,
        _: Option<()>,
        _: usize,
    ) -> Result<ViewDirectoryPage<()>, VfsError> {
        unimplemented!()
    }
    fn metadata_inventory(
        &mut self,
        view: &Self::View,
        _: u64,
    ) -> Result<MetadataInventory, VfsError> {
        Ok(MetadataInventory {
            attributes: InventoryKnowledge::Empty,
            security: if view.uninspected {
                InventoryKnowledge::Uninspected
            } else if view.entries == 0 {
                InventoryKnowledge::Empty
            } else {
                InventoryKnowledge::Present
            },
        })
    }
    fn metadata_page(
        &mut self,
        view: &Self::View,
        _: u64,
        class: MetadataClass,
        after: Option<&str>,
        limit: usize,
    ) -> Result<MetadataPage, VfsError> {
        assert_eq!(class, MetadataClass::Security);
        self.page_calls += 1;
        let mut entries = (0..view.entries)
            .map(|i| MetadataEntry {
                key: if view.entries == 1 {
                    "vendor.descriptor".into()
                } else {
                    format!("vendor.descriptor.{i:04}")
                },
                ..entry(view.bytes.len() as u64)
            })
            .filter(|e| after.is_none_or(|key| e.key.as_str() > key))
            .collect::<Vec<_>>();
        let eof = entries.len() <= limit;
        entries.truncate(limit);
        if self.mutate_second_pass && self.page_calls > view.entries.div_ceil(limit) {
            for e in &mut entries {
                e.encoding = "vendor/changed;v=99".into();
            }
        }
        Ok(MetadataPage { entries, eof })
    }
    fn metadata_read(
        &mut self,
        view: &Self::View,
        _: u64,
        class: MetadataClass,
        key: &str,
        offset: u64,
        out: &mut [u8],
    ) -> Result<usize, VfsError> {
        assert_eq!(class, MetadataClass::Security);
        assert!(key.starts_with("vendor.descriptor"));
        if offset >= view.bytes.len() as u64 {
            return Ok(0);
        }
        let offset = offset as usize;
        let count = out.len().min(view.bytes.len() - offset);
        out[..count].copy_from_slice(&view.bytes[offset..offset + count]);
        Ok(count)
    }
}
#[derive(Default)]
struct Destination {
    publications: usize,
    all: std::collections::BTreeMap<String, Vec<u8>>,
    installed: Option<(MetadataClass, MetadataEntry, Vec<u8>)>,
    active: Arc<AtomicUsize>,
    file: Vec<u8>,
    size: u64,
    ranges: Vec<AllocationRange>,
    core_metadata: Option<RestoreMetadata>,
    round_metadata: bool,
    reject_opaque: bool,
    directory: bool,
    symlink: Option<String>,
    empty: Option<bool>,
}
impl RestoreBackend for Destination {
    type Object = ();
    fn root(&mut self) -> Result<(), VfsError> {
        Ok(())
    }
    fn directory_empty(&mut self, _: &()) -> Result<bool, VfsError> {
        self.empty.ok_or(VfsError::NotSupported)
    }
    fn create_symlink(
        &mut self,
        _: &(),
        _: &str,
        target: &str,
        _: Timespec,
    ) -> Result<(), VfsError> {
        self.directory = false;
        self.symlink = Some(target.to_owned());
        self.size = target.len() as u64;
        Ok(())
    }
    fn read_link(&mut self, _: &(), out: &mut [u8]) -> Result<usize, VfsError> {
        let target = self.symlink.as_ref().ok_or(VfsError::NotSupported)?;
        if out.len() >= target.len() {
            out[..target.len()].copy_from_slice(target.as_bytes());
        }
        Ok(target.len())
    }
    fn create_file(&mut self, _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn create_directory(&mut self, _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn write(&mut self, _: &(), offset: u64, bytes: &[u8], _: Timespec) -> Result<(), VfsError> {
        let end = offset as usize + bytes.len();
        self.file.resize(self.file.len().max(end), 0);
        self.file[offset as usize..end].copy_from_slice(bytes);
        self.ranges.push(AllocationRange {
            offset,
            length: bytes.len() as u64,
            unwritten: false,
        });
        self.size = self.size.max(end as u64);
        Ok(())
    }
    fn reserve(&mut self, _: &(), offset: u64, length: u64, _: Timespec) -> Result<(), VfsError> {
        self.ranges.push(AllocationRange {
            offset,
            length,
            unwritten: true,
        });
        Ok(())
    }
    fn allocations(
        &mut self,
        _: &(),
        start: u64,
        limit: usize,
    ) -> Result<AllocationPage, VfsError> {
        self.ranges.sort_by_key(|r| r.offset);
        let ranges: Vec<_> = self
            .ranges
            .iter()
            .skip(start as usize)
            .take(limit)
            .copied()
            .collect();
        let next = start + ranges.len() as u64;
        Ok(AllocationPage {
            ranges,
            next,
            eof: next >= self.ranges.len() as u64,
        })
    }
    fn resize(&mut self, _: &(), size: u64, _: Timespec) -> Result<(), VfsError> {
        self.size = size;
        Ok(())
    }
    fn link(&mut self, _: &(), _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn metadata(&mut self, _: &(), mut metadata: RestoreMetadata) -> Result<(), VfsError> {
        if self.round_metadata {
            metadata.modified.nanoseconds = 0;
        }
        self.core_metadata = Some(metadata);
        Ok(())
    }
    fn stat(&mut self, _: &()) -> Result<Stat, VfsError> {
        let mut stat = file_stat();
        if self.directory {
            stat.kind = afsplus_vfs::NodeKind::Directory;
        }
        if self.symlink.is_some() {
            stat.kind = afsplus_vfs::NodeKind::Symlink;
        }
        stat.size = self.size;
        stat.allocated_size = self.ranges.iter().map(|r| r.length).sum();
        let metadata = self.core_metadata.unwrap_or(RestoreMetadata {
            protection: 0,
            owner_uid: 0,
            owner_gid: 0,
            created: Timespec::default(),
            modified: Timespec::default(),
            changed: Timespec::default(),
        });
        stat.protection = metadata.protection;
        stat.owner_uid = metadata.owner_uid;
        stat.owner_gid = metadata.owner_gid;
        stat.created = metadata.created;
        stat.modified = metadata.modified;
        stat.changed = metadata.changed;
        Ok(stat)
    }
    fn read(&mut self, _: &(), _: u64, _: &mut [u8]) -> Result<usize, VfsError> {
        unimplemented!()
    }
    fn sync(&mut self) -> Result<(), VfsError> {
        Ok(())
    }
}
struct Upload {
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
impl OpaqueRestoreBackend for Destination {
    type Upload = Upload;
    fn begin_opaque(
        &mut self,
        _: &(),
        class: MetadataClass,
        entry: &MetadataEntry,
    ) -> Result<Upload, VfsError> {
        if self.reject_opaque {
            return Err(VfsError::NotSupported);
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        Ok(Upload {
            class,
            entry: entry.clone(),
            bytes: vec![],
            active: self.active.clone(),
        })
    }
    fn write_opaque(
        &mut self,
        upload: &mut Upload,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), VfsError> {
        assert_eq!(offset, upload.bytes.len() as u64);
        upload.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn finish_opaque(&mut self, mut upload: Upload) -> Result<(), VfsError> {
        self.publications += 1;
        assert!(self
            .all
            .insert(upload.entry.key.clone(), upload.bytes.clone())
            .is_none());
        self.installed = Some((
            upload.class,
            upload.entry.clone(),
            std::mem::take(&mut upload.bytes),
        ));
        Ok(())
    }
}
fn entry(size: u64) -> MetadataEntry {
    MetadataEntry {
        key: "vendor.descriptor".into(),
        encoding: "vendor/unknown;v=99".into(),
        size,
    }
}
fn framing() -> tar::Limits {
    tar::Limits {
        members: 16,
        member_bytes: 65536,
        trailing_zero_blocks: 0,
    }
}
fn limits() -> pax::Limits {
    pax::Limits {
        bytes: 4096,
        records: 16,
        key_bytes: 64,
        value_bytes: 2048,
    }
}
fn destination() -> (
    RestoreService<Destination>,
    RestoreAuthority,
    RestoreGrant,
    RestoreObject<()>,
) {
    let (mut service, authority) = RestoreService::new(Destination::default(), 2).unwrap();
    service.set_metadata_limit(Some(65536));
    let grant = authority.grant();
    let object = service.root(&grant).unwrap();
    (service, authority, grant, object)
}
fn archive(bytes: &[u8]) -> Vec<u8> {
    let (mut service, authority) = BackupService::new(Source::new(bytes.to_vec()), 1).unwrap();
    let grant = authority.grant();
    let reader = service.open(&grant, 1).unwrap();
    let entry = service
        .metadata_page(&reader, 1, MetadataClass::Security, None, 1)
        .unwrap()
        .entries
        .remove(0);
    service.backend_mut().bytes.fill(88); // The captured bytes must be used.
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    Captured {
        client: &mut service.client(),
        reader: &reader,
        object: 1,
    }
    .export(
        &mut writer,
        &Binding {
            ordinal: 7,
            path: "files",
            class: MetadataClass::Security,
            entry: &entry,
        },
        &mut [0; 2],
        limits(),
    )
    .unwrap();
    writer.finish().unwrap().0
}
#[test]
fn captured_binary_metadata_crosses_archive_and_publishes_after_integrity() {
    for bytes in [vec![], vec![0, 255, 42, 128, 0, 1]] {
        let wire = archive(&bytes);
        let mut reader = stream::Reader::new(wire.as_slice(), framing(), limits()).unwrap();
        let (mut dest, _, grant, object) = destination();
        let staged = stage(
            &mut reader,
            &mut dest.client(),
            &Target {
                ordinal: 7,
                path: "files",
                object: &object,
            },
            &mut [0; 3],
            limits(),
        )
        .unwrap();
        assert!(dest.backend_mut().installed.is_none());
        assert!(reader.next_member().unwrap().is_none());
        staged.publish(&reader, &mut dest.client()).unwrap();
        dest.client().sync(&grant).unwrap();
        assert_eq!(
            dest.backend_mut().installed,
            Some((MetadataClass::Security, entry(bytes.len() as u64), bytes))
        );
        assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn source_length_and_revocation_errors_withhold_archive_completion() {
    for (size, revoke) in [(2, false), (4, false), (3, true)] {
        let (mut source, authority) = BackupService::new(Source::new(vec![1, 2, 3]), 1).unwrap();
        let grant = authority.grant();
        let reader = source.open(&grant, 1).unwrap();
        if revoke {
            authority.revoke(&grant).unwrap();
        }
        let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
        assert!(Captured {
            client: &mut source.client(),
            reader: &reader,
            object: 1
        }
        .export(
            &mut writer,
            &Binding {
                ordinal: 7,
                path: "files",
                class: MetadataClass::Security,
                entry: &entry(size)
            },
            &mut [0; 2],
            limits()
        )
        .is_err());
        assert!(writer.finish().is_err());
    }
}
#[test]
fn binding_digest_and_revocation_failures_never_publish() {
    let bytes = [0, 255, 42, 128, 0, 1];
    for fault in 0..5 {
        let mut wire = archive(&bytes);
        if fault == 1 {
            let i = wire.windows(bytes.len()).position(|w| w == bytes).unwrap();
            wire[i] ^= 1;
        }
        let mut reader = stream::Reader::new(wire.as_slice(), framing(), limits()).unwrap();
        let (mut dest, authority, grant, object) = destination();
        let result = stage(
            &mut reader,
            &mut dest.client(),
            &Target {
                ordinal: 7,
                path: if fault == 0 { "files/wrong" } else { "files" },
                object: &object,
            },
            &mut [0; 2],
            limits(),
        );
        if fault == 0 {
            assert!(result.is_err());
            assert!(reader.next_member().is_err());
        } else {
            let staged = result.unwrap();
            if fault != 2 {
                let completed = reader.next_member();
                assert_eq!(completed.is_ok(), fault != 1);
            }
            if fault == 3 {
                authority.revoke(&grant).unwrap();
            }
            if fault == 4 {
                let other_wire = archive(&bytes);
                let mut other =
                    stream::Reader::new(other_wire.as_slice(), framing(), limits()).unwrap();
                while let Some(m) = other.next_member().unwrap() {
                    let mut left = m.size;
                    while left != 0 {
                        left -= other.read_payload(&mut [0; 64]).unwrap() as u64;
                    }
                }
                assert!(staged.publish(&other, &mut dest.client()).is_err());
            } else {
                assert!(staged.publish(&reader, &mut dest.client()).is_err());
            }
        }
        assert!(dest.backend_mut().installed.is_none());
        assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn every_truncated_archive_refuses_publication() {
    let wire = archive(&[0, 255, 42, 128, 0, 1]);
    for end in 0..wire.len() {
        let Ok(mut reader) = stream::Reader::new(&wire[..end], framing(), limits()) else {
            continue;
        };
        let (mut dest, _, _, object) = destination();
        if let Ok(staged) = stage(
            &mut reader,
            &mut dest.client(),
            &Target {
                ordinal: 7,
                path: "files",
                object: &object,
            },
            &mut [0; 2],
            limits(),
        ) {
            assert!(reader.next_member().is_err());
            assert!(staged.publish(&reader, &mut dest.client()).is_err());
        }
        assert!(reader.receipt().is_none());
        assert!(dest.backend_mut().installed.is_none());
        assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn verified_spool_restores_many_values_with_one_upload_slot() {
    use afsplus_backup::spool::{Limits, Verified};
    use std::io::Cursor;
    let bytes = vec![0, 255, 42, 128, 0, 1];
    let (mut source, authority) = BackupService::new(Source::new(bytes.clone()), 1).unwrap();
    let grant = authority.grant();
    let captured = source.open(&grant, 1).unwrap();
    let framing = tar::Limits {
        members: 100,
        ..framing()
    };
    let mut writer = envelope::Writer::new(Vec::new(), framing).unwrap();
    for ordinal in 0..20 {
        let entry = MetadataEntry {
            key: format!("vendor.descriptor.{ordinal}"),
            ..entry(bytes.len() as u64)
        };
        Captured {
            client: &mut source.client(),
            reader: &captured,
            object: 1,
        }
        .export(
            &mut writer,
            &Binding {
                ordinal,
                path: "files",
                class: MetadataClass::Security,
                entry: &entry,
            },
            &mut [0; 2],
            limits(),
        )
        .unwrap();
    }
    let wire = writer.finish().unwrap().0;
    let mut spool = Verified::capture(
        wire.as_slice(),
        Cursor::new(Vec::new()),
        Limits {
            chunk_bytes: 512,
            archive_bytes: wire.len() as u64,
            store_bytes: wire.len() as u64 * 2,
        },
        framing,
        limits(),
    )
    .unwrap();
    let stats = spool.stats();
    assert!(stats.tree_levels < 16);
    assert_eq!(stats.chunk_bytes, 512);
    let mut reader = spool.reader(framing, limits()).unwrap();
    let (mut dest, _, _, object) = destination(); // Exactly root + one upload.
    for ordinal in 0..20 {
        let staged = stage(
            &mut reader,
            &mut dest.client(),
            &Target {
                ordinal,
                path: "files",
                object: &object,
            },
            &mut [0; 2],
            limits(),
        )
        .unwrap();
        assert!(reader.receipt().is_none());
        staged.publish(&reader, &mut dest.client()).unwrap();
        assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
        let installed = dest.backend_mut().installed.as_ref().unwrap();
        assert_eq!(installed.1.key, format!("vendor.descriptor.{ordinal}"));
        assert_eq!(installed.2, bytes);
    }
    assert!(reader.next_member().unwrap().is_none());
    assert!(reader.receipt().is_some());
    assert_eq!(dest.backend_mut().publications, 20);
}

fn inventory_limits(page_entries: usize) -> afsplus_backup::inventory::Limits {
    afsplus_backup::inventory::Limits {
        values: 128,
        value_bytes: 65536,
        page_entries,
        records: limits(),
    }
}
#[test]
fn complete_inventory_roundtrips_with_bounded_pages_and_one_upload() {
    use afsplus_backup::{inventory, spool};
    use std::io::Cursor;
    for count in [0, 1, 70] {
        for page in [1, 3, 64] {
            let bytes = vec![0, 255, 128, 1];
            let mut backend = Source::new(bytes.clone());
            backend.entries = count;
            let (mut source, authority) = BackupService::new(backend, 1).unwrap();
            let grant = authority.grant();
            let captured = source.open(&grant, 1).unwrap();
            let knowledge = source.metadata_inventory(&captured, 1).unwrap();
            let framing = tar::Limits {
                members: 300,
                ..framing()
            };
            let mut writer = envelope::Writer::new(Vec::new(), framing).unwrap();
            let expected = inventory::export(
                &mut Captured {
                    client: &mut source.client(),
                    reader: &captured,
                    object: 1,
                },
                &mut writer,
                7,
                "files",
                &mut [0; 1],
                inventory_limits(page),
            )
            .unwrap();
            assert_eq!(expected.attributes.count, 0);
            assert_eq!(expected.security.count, count as u64);
            assert_eq!(expected.security.bytes, count as u128 * bytes.len() as u128);
            assert_eq!(expected.next_ordinal, Some(8 + count as u64));
            assert_eq!(source.backend_mut().page_calls, 2 * count.div_ceil(page));
            let wire = writer.finish().unwrap().0;
            let mut spool = spool::Verified::capture(
                wire.as_slice(),
                Cursor::new(Vec::new()),
                spool::Limits {
                    chunk_bytes: 512,
                    archive_bytes: wire.len() as u64,
                    store_bytes: wire.len() as u64 * 2,
                },
                framing,
                limits(),
            )
            .unwrap();
            let mut reader = spool.reader(framing, limits()).unwrap();
            let (mut dest, _, _, object) = destination();
            let actual = inventory::restore(
                &mut reader,
                &mut dest.client(),
                &Target {
                    ordinal: 7,
                    path: "files",
                    object: &object,
                },
                knowledge,
                &mut [0; 1],
                inventory_limits(page),
            )
            .unwrap();
            assert_eq!(actual, expected);
            assert_eq!(dest.backend_mut().publications, count);
            assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
            for i in 0..count {
                let key = if count == 1 {
                    "vendor.descriptor".into()
                } else {
                    format!("vendor.descriptor.{i:04}")
                };
                assert_eq!(dest.backend_mut().all.get(&key), Some(&bytes));
            }
            assert!(reader.next_member().unwrap().is_none());
        }
    }
}
#[test]
fn inventory_source_admission_and_second_pass_failures_poison_completion() {
    use afsplus_backup::inventory;
    for fault in 0..6 {
        let mut backend = Source::new(vec![1, 2, 3]);
        backend.uninspected = fault == 0;
        backend.mutate_second_pass = fault == 1;
        let (mut source, authority) = BackupService::new(backend, 1).unwrap();
        let grant = authority.grant();
        let captured = source.open(&grant, 1).unwrap();
        let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
        let mut limits = inventory_limits(1);
        if fault == 2 {
            limits.values = 0;
        }
        if fault == 3 {
            limits.value_bytes = 2;
        }
        if fault == 4 {
            limits.page_entries = 0;
        }
        assert!(inventory::export(
            &mut Captured {
                client: &mut source.client(),
                reader: &captured,
                object: 1
            },
            &mut writer,
            if fault == 5 { u64::MAX } else { 0 },
            "files",
            &mut [0; 1],
            limits
        )
        .is_err());
        assert!(writer.finish().is_err());
    }
}

#[test]
fn malformed_inventory_never_returns_success_or_keeps_uploads() {
    use afsplus_backup::{inventory, spool};
    use sha2::{Digest, Sha512_256};
    use std::io::Cursor;
    for fault in 0..9 {
        let (mut source, authority) = BackupService::new(Source::new(vec![7]), 1).unwrap();
        let grant = authority.grant();
        let captured = source.open(&grant, 1).unwrap();
        let entries: Vec<_> = (0..2)
            .map(|i| MetadataEntry {
                key: format!("vendor.descriptor.{i}"),
                ..entry(1)
            })
            .collect();
        let digest = |class: u8, entries: &[MetadataEntry]| {
            let mut h = Sha512_256::new();
            h.update(b"AROS.inventory.v1\0");
            h.update([class]);
            for e in entries {
                for text in [&e.key, &e.encoding] {
                    h.update((text.len() as u64).to_le_bytes());
                    h.update(text.as_bytes());
                }
                h.update(e.size.to_le_bytes());
            }
            h.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let attr_hash = digest(0, &[]);
        let sec_hash = if fault == 0 {
            "0".repeat(64)
        } else {
            digest(1, &entries)
        };
        let wire = pax::encode(
            &[
                pax::Record {
                    key: "AROS.inventory.version",
                    value: "1",
                },
                pax::Record {
                    key: "AROS.inventory.path",
                    value: "files",
                },
                pax::Record {
                    key: "AROS.inventory.attributes",
                    value: if fault == 4 { "2" } else { "0" },
                },
                pax::Record {
                    key: "AROS.inventory.security",
                    value: if fault == 1 {
                        "3"
                    } else if fault == 4 {
                        "0"
                    } else {
                        "2"
                    },
                },
                pax::Record {
                    key: "AROS.inventory.attribute_bytes",
                    value: "0",
                },
                pax::Record {
                    key: "AROS.inventory.security_bytes",
                    value: if fault == 2 { "1" } else { "2" },
                },
                pax::Record {
                    key: "AROS.inventory.attribute_hash",
                    value: &attr_hash,
                },
                pax::Record {
                    key: "AROS.inventory.security_hash",
                    value: &sec_hash,
                },
            ],
            limits(),
        )
        .unwrap();
        let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
        writer
            .start(
                &tar::Header {
                    path: "_AROS_BACKUP/metadata/inventory-0.pax".into(),
                    kind: tar::Kind::File,
                    link: String::new(),
                    mode: 0o600,
                    uid: 0,
                    gid: 0,
                    size: wire.len() as u64,
                    mtime: 0,
                    uname: String::new(),
                    gname: String::new(),
                },
                None,
            )
            .unwrap();
        writer.write_payload(&wire).unwrap();
        for i in 0..2 {
            Captured {
                client: &mut source.client(),
                reader: &captured,
                object: 1,
            }
            .export(
                &mut writer,
                &Binding {
                    ordinal: i as u64 + 1,
                    path: "files",
                    class: MetadataClass::Security,
                    entry: &entries[if fault == 3 { 0 } else { i }],
                },
                &mut [0; 1],
                limits(),
            )
            .unwrap();
        }
        let wire = writer.finish().unwrap().0;
        let mut spool = spool::Verified::capture(
            wire.as_slice(),
            Cursor::new(Vec::new()),
            spool::Limits {
                chunk_bytes: 512,
                archive_bytes: wire.len() as u64,
                store_bytes: wire.len() as u64 * 2,
            },
            framing(),
            limits(),
        )
        .unwrap();
        let mut verified = spool.reader(framing(), limits()).unwrap();
        let mut unverified = stream::Reader::new(wire.as_slice(), framing(), limits()).unwrap();
        let (mut dest, authority, grant, object) = destination();
        if fault == 8 {
            authority.revoke(&grant).unwrap();
        }
        let knowledge = MetadataInventory {
            attributes: if fault == 4 {
                InventoryKnowledge::Present
            } else {
                InventoryKnowledge::Empty
            },
            security: if fault == 4 {
                InventoryKnowledge::Empty
            } else {
                InventoryKnowledge::Present
            },
        };
        let target = Target {
            ordinal: 0,
            path: if fault == 5 { "files/wrong" } else { "files" },
            object: &object,
        };
        let mut bound = inventory_limits(1);
        if fault == 6 {
            bound.values = 1;
        }
        let result = if fault == 7 {
            let result = inventory::restore(
                &mut unverified,
                &mut dest.client(),
                &target,
                knowledge,
                &mut [0; 1],
                bound,
            );
            assert!(unverified.next_member().is_err());
            result
        } else {
            let result = inventory::restore(
                &mut verified,
                &mut dest.client(),
                &target,
                knowledge,
                &mut [0; 1],
                bound,
            );
            assert!(verified.next_member().is_err());
            result
        };
        assert!(result.is_err(), "fault {fault}");
        assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
        assert!(dest.backend_mut().publications <= 2);
        if fault >= 4 {
            assert_eq!(dest.backend_mut().publications, 0);
        }
    }
}

fn file_stat() -> Stat {
    Stat {
        object_id: 1,
        kind: afsplus_vfs::NodeKind::File,
        size: (1 << 40) + 1,
        allocated_size: 4100,
        links: 1,
        protection: u64::MAX,
        mode: 0,
        owner_uid: 0,
        owner_gid: 0,
        created: Timespec {
            seconds: i64::MIN,
            nanoseconds: 999999999,
        },
        modified: Timespec {
            seconds: -42,
            nanoseconds: 123456789,
        },
        changed: Timespec {
            seconds: i64::MAX,
            nanoseconds: 1,
        },
        content_generation: 7,
    }
}
fn file_limits() -> afsplus_backup::file::Limits {
    afsplus_backup::file::Limits {
        contents: afsplus_backup::sparse::consumer::Options {
            map: afsplus_backup::sparse::Limits {
                entries: 64,
                map_bytes: 4096,
                data_bytes: 4096,
                logical_bytes: u64::MAX,
            },
            records: limits(),
            page_entries: 1,
        },
        inventory: afsplus_backup::inventory::Limits {
            values: 16,
            value_bytes: 4096,
            page_entries: 1,
            records: limits(),
        },
    }
}
fn file_archive(mode: afsplus_backup::file::Mode, uninspected: bool) -> Vec<u8> {
    let mut backend = Source::new(vec![0, 255, 42, 128, 1]);
    backend.uninspected = uninspected;
    let (mut source, authority) = BackupService::new(backend, 1).unwrap();
    let grant = authority.grant();
    let view = source.open(&grant, 1).unwrap();
    source.backend_mut().bytes.fill(99);
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let report = afsplus_backup::file::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: 1,
        },
        &mut writer,
        (0, "files/file"),
        mode,
        &mut [0; 1],
        file_limits(),
    )
    .unwrap();
    assert_eq!(
        report.next_ordinal,
        Some(if mode == afsplus_backup::file::Mode::Full {
            5
        } else {
            3
        })
    );
    writer.finish().unwrap().0
}
#[test]
fn bound_file_groups_preserve_exact_metadata_and_validate_explicit_recovery_losses() {
    use afsplus_backup::{allocation, file, spool};
    use std::io::Cursor;
    for archive_mode in [file::Mode::Full, file::Mode::Recovery] {
        let wire = file_archive(archive_mode, archive_mode == file::Mode::Recovery);
        for mode in [file::Mode::Full, file::Mode::Recovery] {
            let mut spool = spool::Verified::capture_sparse(
                wire.as_slice(),
                Cursor::new(Vec::new()),
                spool::Limits {
                    chunk_bytes: 512,
                    archive_bytes: wire.len() as u64,
                    store_bytes: wire.len() as u64 * 2,
                },
                framing(),
                limits(),
            )
            .unwrap();
            let mut reader = spool.reader(framing(), limits()).unwrap();
            let (mut dest, _, _, object) = destination();
            dest.set_reservation_limit(4096);
            // Recovery must work even when the provider refuses every opaque upload.
            dest.backend_mut().reject_opaque = mode == file::Mode::Recovery;
            let report = file::restore(
                &mut reader,
                &mut dest.client(),
                &Target {
                    ordinal: 0,
                    path: "files/file",
                    object: &object,
                },
                &mut [0; 1],
                file::RestoreOptions {
                    mode,
                    allocation: allocation::RestoreOptions {
                        limits: file_limits().contents,
                        mode: if mode == file::Mode::Full {
                            allocation::Mode::PreserveAllocation
                        } else {
                            allocation::Mode::RecoverContents
                        },
                        reservation_chunk: 4096,
                        reservation_bytes: 4096,
                        readback_entries: 64,
                    },
                    inventory: file_limits().inventory,
                },
                Timespec::default(),
            );
            if mode == file::Mode::Full && archive_mode == file::Mode::Recovery {
                assert!(report.is_err());
                assert!(reader.next_member().is_err());
                assert!(dest.backend_mut().file.is_empty());
                continue;
            }
            let report = report.unwrap();
            assert_eq!(report.archive_mode, archive_mode);
            assert_eq!(
                report.next_ordinal,
                Some(if archive_mode == file::Mode::Full {
                    5
                } else {
                    3
                })
            );
            let expected = file_stat();
            let actual = dest.stat(&object).unwrap();
            assert_eq!(
                (
                    actual.size,
                    actual.protection,
                    actual.created,
                    actual.modified,
                    actual.changed
                ),
                (
                    expected.size,
                    expected.protection,
                    expected.created,
                    expected.modified,
                    expected.changed
                )
            );
            assert_eq!(&dest.backend_mut().file[4096..], b"c\0or");
            if mode == file::Mode::Full {
                assert_eq!(
                    dest.backend_mut().installed.as_ref().unwrap().2,
                    vec![0, 255, 42, 128, 1]
                );
                assert!(matches!(
                    report.opaque,
                    file::OpaqueDisposition::Preserved(_)
                ));
                assert_eq!(
                    report.allocation.allocation,
                    allocation::Disposition::Preserved
                );
            } else {
                assert_eq!(dest.backend_mut().publications, 0);
                assert_eq!(
                    report.allocation.allocation,
                    allocation::Disposition::Discarded {
                        ranges: 1,
                        bytes: 4096
                    }
                );
                match report.opaque {
                    file::OpaqueDisposition::Omitted {
                        knowledge,
                        transported,
                    } => {
                        assert_eq!(knowledge.attributes, InventoryKnowledge::Empty);
                        if archive_mode == file::Mode::Full {
                            assert_eq!(knowledge.security, InventoryKnowledge::Present);
                            let summary = transported.unwrap();
                            assert_eq!((summary.security.count, summary.security.bytes), (1, 5));
                        } else {
                            assert_eq!(knowledge.security, InventoryKnowledge::Uninspected);
                            assert!(transported.is_none());
                        }
                    }
                    _ => panic!("recovery must report omissions"),
                }
            }
            assert!(reader.next_member().unwrap().is_none());
            assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
        }
    }
}

fn changed_file_archive(fault: u8) -> Vec<u8> {
    use afsplus_backup::{file, metadata};
    let wire = file_archive(file::Mode::Full, false);
    let mut input = envelope::Reader::new(wire.as_slice(), framing()).unwrap();
    let mut output = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let mut pending = None;
    while let Some(mut header) = input.next_header().unwrap() {
        let size_override = if header.kind == tar::Kind::File {
            pending.take()
        } else {
            None
        };
        let size = size_override.unwrap_or(header.size);
        input.begin_payload(size_override).unwrap();
        let mut payload = vec![0; size as usize];
        if !payload.is_empty() {
            assert_eq!(input.read_payload(&mut payload).unwrap(), payload.len());
        }
        if header.kind == tar::Kind::PaxLocal {
            pending = pax::decode(&payload, limits())
                .unwrap()
                .iter()
                .find(|r| r.key == "size")
                .map(|r| r.value.parse::<u64>().unwrap());
        }
        if header.path.contains("file-v1-full-") && fault <= 4 {
            let mut object = metadata::decode(&payload, limits()).unwrap();
            match fault {
                0 => object.path = "files/wrong",
                1 => object.kind = tar::Kind::Directory,
                2 => object.modified.nanos += 1,
                3 => object.security = metadata::Inventory::Empty,
                4 => header.path = header.path.replace("v1", "v2"),
                _ => unreachable!(),
            }
            payload = metadata::encode(&object, limits()).unwrap();
            header.size = payload.len() as u64;
        }
        if header.path.contains("inventory-") && fault == 5 {
            let mut records = pax::decode(&payload, limits()).unwrap();
            let bad = "0".repeat(64);
            records
                .iter_mut()
                .find(|r| r.key == "AROS.inventory.security_hash")
                .unwrap()
                .value = &bad;
            payload = pax::encode(&records, limits()).unwrap();
            header.size = payload.len() as u64;
        }
        if header.path.contains("value-") && header.path.ends_with(".pax") && fault == 6 {
            let mut records = pax::decode(&payload, limits()).unwrap();
            records
                .iter_mut()
                .find(|r| r.key == "AROS.value.path")
                .unwrap()
                .value = "files/wrong";
            payload = pax::encode(&records, limits()).unwrap();
            header.size = payload.len() as u64;
        }
        output.start(&header, size_override).unwrap();
        if !payload.is_empty() {
            output.write_payload(&payload).unwrap();
        }
    }
    output.finish().unwrap().0
}
#[test]
fn file_group_conflicts_rounding_and_revocation_never_report_preservation() {
    use afsplus_backup::{allocation, file, spool};
    use std::io::Cursor;
    for fault in 0..=12 {
        for mode in [file::Mode::Full, file::Mode::Recovery] {
            let wire = if fault <= 6 {
                changed_file_archive(fault)
            } else {
                file_archive(file::Mode::Full, false)
            };
            let mut spool = spool::Verified::capture_sparse(
                wire.as_slice(),
                Cursor::new(Vec::new()),
                spool::Limits {
                    chunk_bytes: 512,
                    archive_bytes: wire.len() as u64,
                    store_bytes: wire.len() as u64 * 2,
                },
                framing(),
                limits(),
            )
            .unwrap();
            let mut reader = spool.reader(framing(), limits()).unwrap();
            let (mut dest, authority, grant, object) = destination();
            dest.set_reservation_limit(4096);
            dest.backend_mut().round_metadata = fault == 7;
            if fault == 8 {
                authority.revoke(&grant).unwrap();
            }
            let allocation_mode = if (mode == file::Mode::Full) != (fault == 10) {
                allocation::Mode::PreserveAllocation
            } else {
                allocation::Mode::RecoverContents
            };
            let mut scratch = [0; 1];
            let result = file::restore(
                &mut reader,
                &mut dest.client(),
                &Target {
                    ordinal: 0,
                    path: "files/file",
                    object: &object,
                },
                &mut scratch[..usize::from(fault != 11)],
                file::RestoreOptions {
                    mode,
                    allocation: allocation::RestoreOptions {
                        limits: file_limits().contents,
                        mode: allocation_mode,
                        reservation_chunk: 4096,
                        reservation_bytes: 4096,
                        readback_entries: 64,
                    },
                    inventory: afsplus_backup::inventory::Limits {
                        values: if fault == 9 { 0 } else { 16 },
                        page_entries: if fault == 12 { 0 } else { 1 },
                        ..file_limits().inventory
                    },
                },
                Timespec::default(),
            );
            assert!(result.is_err(), "fault {fault}: {result:?}");
            assert!(reader.next_member().is_err());
            if matches!(fault, 0 | 1 | 2 | 4 | 8 | 10 | 11) {
                assert!(dest.backend_mut().file.is_empty(), "fault {fault}");
            }
            if fault != 7 {
                assert!(dest.backend_mut().core_metadata.is_none());
            }
            assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
            if mode == file::Mode::Recovery {
                assert_eq!(dest.backend_mut().publications, 0);
            }
        }
    }
}
#[test]
fn full_file_export_refuses_unknown_inventories_revocation_and_ordinal_exhaustion() {
    use afsplus_backup::file;
    for fault in 0..4 {
        let mut backend = Source::new(vec![0, 255]);
        backend.uninspected = fault == 0;
        backend.mutate_second_pass = fault == 3;
        let (mut source, authority) = BackupService::new(backend, 1).unwrap();
        let grant = authority.grant();
        let view = source.open(&grant, 1).unwrap();
        if fault == 1 {
            authority.revoke(&grant).unwrap();
        }
        let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
        let result = file::export(
            &mut Captured {
                client: &mut source.client(),
                reader: &view,
                object: 1,
            },
            &mut writer,
            (if fault == 2 { u64::MAX - 2 } else { 0 }, "files/file"),
            file::Mode::Full,
            &mut [0; 1],
            file_limits(),
        );
        assert!(result.is_err());
        assert!(writer.finish().is_err());
    }
}

fn directory_archive(mode: afsplus_backup::file::Mode, entries: usize) -> Vec<u8> {
    use afsplus_backup::{file, namespace};
    let mut backend = Source::new(vec![0, 255, 42]);
    backend.directory = true;
    backend.entries = entries;
    backend.uninspected = mode == file::Mode::Recovery;
    let (mut source, authority) = BackupService::new(backend, 1).unwrap();
    let grant = authority.grant();
    let view = source.open(&grant, 1).unwrap();
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let report = namespace::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: 1,
        },
        &mut writer,
        &namespace::Binding {
            ordinal: 0,
            path: "files",
            entry: namespace::Entry::Directory,
        },
        mode,
        &mut [0; 1],
        namespace::Limits {
            records: limits(),
            inventory: file_limits().inventory,
        },
    )
    .unwrap();
    assert_eq!(
        report.next_ordinal,
        Some(if mode == file::Mode::Full {
            3 + entries as u64
        } else {
            2
        })
    );
    writer.finish().unwrap().0
}
#[test]
fn directory_groups_preserve_opaque_values_or_validate_explicit_recovery_loss() {
    use afsplus_backup::{file, namespace, spool};
    use std::io::Cursor;
    for entries in [0, 1] {
        for archive_mode in [file::Mode::Full, file::Mode::Recovery] {
            for mode in [file::Mode::Full, file::Mode::Recovery] {
                let wire = directory_archive(archive_mode, entries);
                let mut spool = spool::Verified::capture(
                    wire.as_slice(),
                    Cursor::new(Vec::new()),
                    spool::Limits {
                        chunk_bytes: 512,
                        archive_bytes: wire.len() as u64,
                        store_bytes: wire.len() as u64 * 2,
                    },
                    framing(),
                    limits(),
                )
                .unwrap();
                let mut reader = spool.reader(framing(), limits()).unwrap();
                let (mut dest, _, _, object) = destination();
                dest.backend_mut().directory = true;
                dest.backend_mut().empty = Some(true);
                dest.backend_mut().reject_opaque = mode == file::Mode::Recovery || entries == 0;
                let result = namespace::restore_directory(
                    &mut reader,
                    &mut dest.client(),
                    &Target {
                        ordinal: 0,
                        path: "files",
                        object: &object,
                    },
                    &mut [0; 1],
                    namespace::RestoreOptions {
                        mode,
                        limits: namespace::Limits {
                            records: limits(),
                            inventory: file_limits().inventory,
                        },
                    },
                );
                if mode == file::Mode::Full && archive_mode == file::Mode::Recovery {
                    assert!(result.is_err());
                    assert!(reader.next_member().is_err());
                    assert!(dest.backend_mut().core_metadata.is_none());
                    continue;
                }
                let report = result.unwrap();
                let expected = file_stat();
                let stat = dest.stat(&object).unwrap();
                assert_eq!(stat.kind, afsplus_vfs::NodeKind::Directory);
                assert_eq!(
                    (stat.protection, stat.created, stat.modified, stat.changed),
                    (
                        expected.protection,
                        expected.created,
                        expected.modified,
                        expected.changed
                    )
                );
                if mode == file::Mode::Full {
                    assert_eq!(dest.backend_mut().publications, entries);
                    assert!(matches!(
                        report.opaque,
                        file::OpaqueDisposition::Preserved(_)
                    ));
                    if entries != 0 {
                        assert_eq!(
                            dest.backend_mut().installed.as_ref().unwrap().2,
                            vec![0, 255, 42]
                        );
                    }
                } else {
                    assert_eq!(dest.backend_mut().publications, 0);
                    match report.opaque {
                        file::OpaqueDisposition::Omitted {
                            knowledge,
                            transported,
                        } => {
                            if archive_mode == file::Mode::Full {
                                assert_eq!(transported.unwrap().security.count, entries as u64);
                            } else {
                                assert_eq!(knowledge.security, InventoryKnowledge::Uninspected);
                                assert!(transported.is_none());
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                assert!(reader.next_member().unwrap().is_none());
                assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
            }
        }
    }
}
fn changed_directory_archive(fault: u8) -> Vec<u8> {
    use afsplus_backup::{file, metadata};
    let wire = directory_archive(file::Mode::Full, 1);
    let mut input = envelope::Reader::new(wire.as_slice(), framing()).unwrap();
    let mut output = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let mut pending = None;
    while let Some(mut header) = input.next_header().unwrap() {
        let size_override = if header.kind == tar::Kind::File {
            pending.take()
        } else {
            None
        };
        let size = size_override.unwrap_or(header.size);
        input.begin_payload(size_override).unwrap();
        let mut payload = vec![0; size as usize];
        if !payload.is_empty() {
            assert_eq!(input.read_payload(&mut payload).unwrap(), payload.len());
        }
        if header.kind == tar::Kind::PaxLocal {
            pending = pax::decode(&payload, limits())
                .unwrap()
                .iter()
                .find(|r| r.key == "size")
                .map(|r| r.value.parse::<u64>().unwrap());
        }
        if header.path.contains("directory-v1-full") {
            let mut object = metadata::decode(&payload, limits()).unwrap();
            match fault {
                0 => object.path = "files/wrong",
                1 => object.kind = tar::Kind::File,
                2 => object.modified.nanos += 1,
                3 => object.security = afsplus_backup::metadata::Inventory::Empty,
                4 => header.path = header.path.replace("v1", "v2"),
                _ => {}
            }
            // A non-directory object cannot encode the root path, so use a valid
            // file path and verify the group rejects both conflicts.
            if fault == 1 {
                object.path = "files/wrong";
            }
            payload = metadata::encode(&object, limits()).unwrap();
            header.size = payload.len() as u64;
        }
        if header.path.contains("namespace-") && fault == 5 {
            header.path = header.path.replace("namespace-1", "namespace-9");
        }
        if header.kind == tar::Kind::Directory {
            if fault == 6 {
                header.path = "files/namespace.9".into();
            }
            if fault == 7 {
                header.mode = 0o600;
            }
        }
        output.start(&header, size_override).unwrap();
        if !payload.is_empty() {
            output.write_payload(&payload).unwrap();
        }
    }
    output.finish().unwrap().0
}
#[test]
fn directory_bindings_empty_destination_and_authority_fail_closed() {
    use afsplus_backup::{file, namespace, spool};
    use std::io::Cursor;
    for fault in 0..=12 {
        let wire = if fault < 8 {
            changed_directory_archive(fault)
        } else {
            directory_archive(file::Mode::Full, 1)
        };
        let mut spool = spool::Verified::capture(
            wire.as_slice(),
            Cursor::new(Vec::new()),
            spool::Limits {
                chunk_bytes: 512,
                archive_bytes: wire.len() as u64,
                store_bytes: wire.len() as u64 * 2,
            },
            framing(),
            limits(),
        )
        .unwrap();
        let mut reader = spool.reader(framing(), limits()).unwrap();
        let (mut dest, authority, grant, object) = destination();
        dest.backend_mut().directory = fault != 8;
        dest.backend_mut().empty = if fault == 9 { None } else { Some(fault != 10) };
        dest.backend_mut().round_metadata = fault == 11;
        if fault == 12 {
            authority.revoke(&grant).unwrap();
        }
        let result = namespace::restore_directory(
            &mut reader,
            &mut dest.client(),
            &Target {
                ordinal: 0,
                path: "files",
                object: &object,
            },
            &mut [0; 1],
            namespace::RestoreOptions {
                mode: file::Mode::Full,
                limits: namespace::Limits {
                    records: limits(),
                    inventory: file_limits().inventory,
                },
            },
        );
        assert!(result.is_err(), "fault {fault}");
        assert!(reader.next_member().is_err());
        if fault != 11 {
            assert!(dest.backend_mut().core_metadata.is_none());
            assert_eq!(dest.backend_mut().publications, 0);
        }
        assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn namespace_export_failures_poison_completion() {
    use afsplus_backup::{file, namespace};
    for fault in 0..8 {
        let mut backend = Source::new(vec![0, 255]);
        backend.directory = fault != 1 && fault != 4;
        backend.uninspected = fault == 0;
        backend.mutate_second_pass = fault == 5;
        let (mut source, authority) = BackupService::new(backend, 1).unwrap();
        let grant = authority.grant();
        let view = source.open(&grant, 1).unwrap();
        if fault == 2 {
            authority.revoke(&grant).unwrap();
        }
        let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
        let mut scratch = [0; 1];
        let result = namespace::export(
            &mut Captured {
                client: &mut source.client(),
                reader: &view,
                object: 1,
            },
            &mut writer,
            &namespace::Binding {
                ordinal: if fault == 3 { u64::MAX } else { 0 },
                path: if fault == 4 { "files/alias" } else { "files" },
                entry: if fault == 4 {
                    namespace::Entry::HardLink {
                        primary_path: "files/alias",
                    }
                } else {
                    namespace::Entry::Directory
                },
            },
            file::Mode::Full,
            &mut scratch[..usize::from(fault != 7)],
            namespace::Limits {
                records: pax::Limits {
                    bytes: if fault == 6 { 0 } else { 4096 },
                    ..limits()
                },
                inventory: file_limits().inventory,
            },
        );
        assert!(result.is_err(), "fault {fault}");
        assert!(writer.finish().is_err());
    }
}

#[test]
fn symlink_groups_preserve_binary_security_or_report_recovery_loss() {
    use afsplus_backup::{file, namespace, spool};
    use std::io::Cursor;
    for entries in [0, 1] {
        for archive_mode in [file::Mode::Full, file::Mode::Recovery] {
            for mode in [file::Mode::Full, file::Mode::Recovery] {
                let wire = symlink_archive(archive_mode, entries);
                let mut spool = spool::Verified::capture(
                    wire.as_slice(),
                    Cursor::new(Vec::new()),
                    spool::Limits {
                        chunk_bytes: 512,
                        archive_bytes: wire.len() as u64,
                        store_bytes: wire.len() as u64 * 2,
                    },
                    framing(),
                    limits(),
                )
                .unwrap();
                let mut reader = spool.reader(framing(), limits()).unwrap();
                // Parent, created symlink and one staged opaque upload each hold a slot.
                let (mut dest, restore_authority) =
                    RestoreService::new(Destination::default(), 3).unwrap();
                dest.set_metadata_limit(Some(65536));
                let restore_grant = restore_authority.grant();
                let parent = dest.root(&restore_grant).unwrap();
                dest.backend_mut().directory = true;
                dest.backend_mut().reject_opaque = mode == file::Mode::Recovery || entries == 0;
                let result = namespace::restore_symlink(
                    &mut reader,
                    &mut dest.client(),
                    &namespace::SymlinkTarget {
                        ordinal: 0,
                        path: "files/link",
                        parent: &parent,
                        name: "link",
                    },
                    &mut [0; 1],
                    namespace::RestoreOptions {
                        mode,
                        limits: namespace::Limits {
                            records: limits(),
                            inventory: file_limits().inventory,
                        },
                    },
                    Timespec::default(),
                );
                if mode == file::Mode::Full && archive_mode == file::Mode::Recovery {
                    assert!(result.is_err());
                    assert!(reader.next_member().is_err());
                    assert!(dest.backend_mut().symlink.is_none());
                    continue;
                }
                let report = result.unwrap();
                assert!(reader.next_member().unwrap().is_none());
                assert_eq!(
                    dest.backend_mut().symlink.as_deref(),
                    Some("../café/target")
                );
                assert_eq!(
                    dest.stat(&report.object).unwrap().protection,
                    file_stat().protection
                );
                if mode == file::Mode::Full {
                    assert_eq!(dest.backend_mut().publications, entries);
                    assert!(matches!(
                        report.opaque,
                        file::OpaqueDisposition::Preserved(_)
                    ));
                    if entries != 0 {
                        assert_eq!(
                            dest.backend_mut().installed.as_ref().unwrap().2,
                            vec![0, 255, 42]
                        );
                    }
                } else {
                    assert_eq!(dest.backend_mut().publications, 0);
                    match report.opaque {
                        file::OpaqueDisposition::Omitted {
                            knowledge,
                            transported,
                        } => {
                            if archive_mode == file::Mode::Full {
                                assert_eq!(transported.unwrap().security.count, entries as u64);
                            } else {
                                assert_eq!(knowledge.security, InventoryKnowledge::Uninspected);
                                assert!(transported.is_none());
                            }
                        }
                        _ => panic!("recovery must report omitted metadata"),
                    }
                }
            }
        }
    }
}

fn symlink_archive(archive_mode: afsplus_backup::file::Mode, entries: usize) -> Vec<u8> {
    use afsplus_backup::{file, namespace};
    let mut backend = Source::new(vec![0, 255, 42]);
    backend.symlink = Some("../café/target".into());
    backend.entries = entries;
    backend.uninspected = archive_mode == file::Mode::Recovery;
    let (mut source, authority) = BackupService::new(backend, 1).unwrap();
    let grant = authority.grant();
    let view = source.open(&grant, 1).unwrap();
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    namespace::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: 1,
        },
        &mut writer,
        &namespace::Binding {
            ordinal: 0,
            path: "files/link",
            entry: namespace::Entry::Symlink,
        },
        archive_mode,
        &mut [0; 1],
        namespace::Limits {
            records: limits(),
            inventory: file_limits().inventory,
        },
    )
    .unwrap();
    writer.finish().unwrap().0
}

fn changed_symlink_archive(fault: u8) -> Vec<u8> {
    use afsplus_backup::{file, metadata};
    let wire = symlink_archive(file::Mode::Full, 1);
    let mut input = envelope::Reader::new(wire.as_slice(), framing()).unwrap();
    let mut output = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let mut pending = None;
    while let Some(mut header) = input.next_header().unwrap() {
        let size_override = if header.kind == tar::Kind::File {
            pending.take()
        } else {
            None
        };
        input.begin_payload(size_override).unwrap();
        let mut payload = vec![0; size_override.unwrap_or(header.size) as usize];
        if !payload.is_empty() {
            assert_eq!(input.read_payload(&mut payload).unwrap(), payload.len());
        }
        if header.kind == tar::Kind::PaxLocal {
            pending = pax::decode(&payload, limits())
                .unwrap()
                .iter()
                .find(|r| r.key == "size")
                .map(|r| r.value.parse::<u64>().unwrap());
        }
        if header.path.contains("symlink-v1-full-") && fault <= 3 {
            let mut object = metadata::decode(&payload, limits()).unwrap();
            match fault {
                0 => object.path = "files/wrong",
                1 => object.kind = tar::Kind::Directory,
                2 => object.modified.nanos += 1,
                3 => header.path = header.path.replace("v1", "v2"),
                _ => unreachable!(),
            }
            payload = metadata::encode(&object, limits()).unwrap();
        }
        if header.path.contains("namespace-1.pax") && (4..=6).contains(&fault) {
            let mut records = pax::decode(&payload, limits()).unwrap();
            match fault {
                4 => records.iter_mut().find(|r| r.key == "path").unwrap().value = "files/wrong",
                5 => records.retain(|r| r.key != "linkpath"),
                6 => records.iter_mut().find(|r| r.key == "mtime").unwrap().value = "123",
                _ => unreachable!(),
            }
            payload = pax::encode(&records, limits()).unwrap();
        }
        if header.kind == tar::Kind::Symlink {
            match fault {
                7 => header.kind = tar::Kind::HardLink,
                8 => header.mode = 0o777,
                9 => header.path = "files/namespace.2".into(),
                _ => (),
            }
        }
        if header.path.contains("inventory-") && fault == 10 {
            let mut records = pax::decode(&payload, limits()).unwrap();
            let bad = "0".repeat(64);
            records
                .iter_mut()
                .find(|r| r.key == "AROS.inventory.security_hash")
                .unwrap()
                .value = &bad;
            payload = pax::encode(&records, limits()).unwrap();
        }
        if size_override.is_none() {
            header.size = payload.len() as u64;
        }
        output.start(&header, size_override).unwrap();
        if !payload.is_empty() {
            output.write_payload(&payload).unwrap();
        }
    }
    output.finish().unwrap().0
}
#[test]
fn malformed_symlink_groups_refuse_success_and_release_uploads() {
    use afsplus_backup::{file, namespace, spool};
    use std::io::Cursor;
    for fault in 0..=10 {
        for mode in [file::Mode::Full, file::Mode::Recovery] {
            let wire = changed_symlink_archive(fault);
            // Re-sign the envelope so this exercises semantic group validation.
            let captured = spool::Verified::capture(
                wire.as_slice(),
                Cursor::new(Vec::new()),
                spool::Limits {
                    chunk_bytes: 512,
                    archive_bytes: wire.len() as u64,
                    store_bytes: wire.len() as u64 * 2,
                },
                framing(),
                limits(),
            );
            if fault == 7 {
                // A relative parent traversal is invalid as a hard-link source;
                // the replay spool rejects it before destination admission.
                assert!(captured.is_err());
                continue;
            }
            let mut spool = captured.unwrap();
            let mut reader = spool.reader(framing(), limits()).unwrap();
            let (mut dest, authority) = RestoreService::new(Destination::default(), 3).unwrap();
            dest.set_metadata_limit(Some(65536));
            let grant = authority.grant();
            let parent = dest.root(&grant).unwrap();
            dest.backend_mut().directory = true;
            let result = namespace::restore_symlink(
                &mut reader,
                &mut dest.client(),
                &namespace::SymlinkTarget {
                    ordinal: 0,
                    path: "files/link",
                    parent: &parent,
                    name: "link",
                },
                &mut [0; 1],
                namespace::RestoreOptions {
                    mode,
                    limits: namespace::Limits {
                        records: limits(),
                        inventory: file_limits().inventory,
                    },
                },
                Timespec::default(),
            );
            assert!(result.is_err(), "fault {fault}, mode {mode:?}");
            assert!(reader.next_member().is_err());
            assert_eq!(dest.backend_mut().active.load(Ordering::SeqCst), 0);
            assert!(dest.backend_mut().core_metadata.is_none());
            if fault < 10 {
                assert!(dest.backend_mut().symlink.is_none());
            }
            // The failed group's created handle and upload slots have been released.
            let spare = dest.root(&grant).unwrap();
            let last = dest.root(&grant).unwrap();
            drop((spare, last));
        }
    }
}
