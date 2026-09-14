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

struct Source(Vec<u8>);
impl SnapshotBackend for Source {
    type View = Vec<u8>;
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
        Ok(self.0.clone())
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
    fn metadata_page(
        &mut self,
        view: &Self::View,
        _: u64,
        _: MetadataClass,
        _: Option<&str>,
        _: usize,
    ) -> Result<MetadataPage, VfsError> {
        Ok(MetadataPage {
            entries: vec![entry(view.len() as u64)],
            eof: true,
        })
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
        assert_eq!(key, "vendor.descriptor");
        if offset >= view.len() as u64 {
            return Ok(0);
        }
        let offset = offset as usize;
        let count = out.len().min(view.len() - offset);
        out[..count].copy_from_slice(&view[offset..offset + count]);
        Ok(count)
    }
}
#[derive(Default)]
struct Destination {
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
    let (mut service, authority) = BackupService::new(Source(bytes.to_vec()), 1).unwrap();
    let grant = authority.grant();
    let reader = service.open(&grant, 1).unwrap();
    let entry = service
        .metadata_page(&reader, 1, MetadataClass::Security, None, 1)
        .unwrap()
        .entries
        .remove(0);
    service.backend_mut().0.fill(88); // The captured bytes must be used.
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
        let (mut source, authority) = BackupService::new(Source(vec![1, 2, 3]), 1).unwrap();
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
