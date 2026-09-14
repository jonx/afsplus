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
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            entries: 1,
            uninspected: false,
            page_calls: 0,
            mutate_second_pass: false,
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
    fn stat(&mut self, _: &Self::View, _: u64) -> Result<Stat, VfsError> {
        unimplemented!()
    }
    fn read(&mut self, _: &Self::View, _: u64, _: u64, _: &mut [u8]) -> Result<usize, VfsError> {
        unimplemented!()
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
}
impl RestoreBackend for Destination {
    type Object = ();
    fn root(&mut self) -> Result<(), VfsError> {
        Ok(())
    }
    fn create_file(&mut self, _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn create_directory(&mut self, _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn write(&mut self, _: &(), _: u64, _: &[u8], _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn resize(&mut self, _: &(), _: u64, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn link(&mut self, _: &(), _: &(), _: &str, _: Timespec) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn metadata(&mut self, _: &(), _: RestoreMetadata) -> Result<(), VfsError> {
        unimplemented!()
    }
    fn stat(&mut self, _: &()) -> Result<Stat, VfsError> {
        unimplemented!()
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
