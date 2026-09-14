//! ADR-084 opaque value transport through checked source and destination APIs.
use crate::{envelope, member, pax, stream, tar};
use afsplus_vfs::backup::{
    BackupClient, BackupError, BackupReader, MetadataClass, MetadataEntry, SnapshotBackend,
};
use afsplus_vfs::restore::{
    OpaqueRestoreBackend, RestoreClient, RestoreError, RestoreObject, RestoreUpload,
};
use std::io::{Read, Write};
use std::sync::Arc;
#[derive(Debug)]
pub enum Error {
    Envelope(envelope::Error),
    Stream(stream::Error),
    Pax(pax::Error),
    Backup(BackupError),
    Restore(RestoreError),
    Invalid,
    Limit,
    NeedsVerifiedReplay,
}
pub struct Binding<'a> {
    pub ordinal: u64,
    pub path: &'a str,
    pub class: MetadataClass,
    pub entry: &'a MetadataEntry,
}
pub struct Captured<'a, 'b, P: SnapshotBackend> {
    pub client: &'a mut BackupClient<'b, P>,
    pub reader: &'a BackupReader<P::View>,
    pub object: u64,
}
pub struct Target<'a, O> {
    pub ordinal: u64,
    pub path: &'a str,
    pub object: &'a RestoreObject<O>,
}
fn path(ordinal: u64, suffix: &str) -> String {
    format!("_AROS_BACKUP/metadata/value-{ordinal}.{suffix}")
}
pub(crate) fn header(path: String, kind: tar::Kind, size: u64) -> tar::Header {
    tar::Header {
        path,
        link: String::new(),
        kind,
        mode: 0o600,
        uid: 0,
        gid: 0,
        size,
        mtime: 0,
        uname: String::new(),
        gname: String::new(),
    }
}
fn valid(path: &str, key: &str, encoding: &str) -> bool {
    envelope::canonical(path, true, false)
        && !path.contains('\0')
        && !key.is_empty()
        && key.len() <= 1024
        && !key.contains('\0')
        && !encoding.is_empty()
        && encoding.len() <= 128
        && !encoding.contains('\0')
}
const KEYS: [&str; 6] = [
    "AROS.value.version",
    "AROS.value.path",
    "AROS.value.class",
    "AROS.value.key",
    "AROS.value.encoding",
    "AROS.value.size",
];
fn descriptor(binding: &Binding<'_>, limits: pax::Limits) -> Result<Vec<u8>, Error> {
    if !valid(binding.path, &binding.entry.key, &binding.entry.encoding) {
        return Err(Error::Invalid);
    }
    let size = binding.entry.size.to_string();
    let class = match binding.class {
        MetadataClass::Attribute => "attribute",
        MetadataClass::Security => "security",
    };
    let values = [
        "1",
        binding.path,
        class,
        &binding.entry.key,
        &binding.entry.encoding,
        &size,
    ];
    let records: [pax::Record<'_>; 6] = std::array::from_fn(|i| pax::Record {
        key: KEYS[i],
        value: values[i],
    });
    pax::encode(&records, limits).map_err(Error::Pax)
}
fn parse(bytes: &[u8], limits: pax::Limits) -> Result<(&str, MetadataClass, MetadataEntry), Error> {
    let records = pax::decode(bytes, limits).map_err(Error::Pax)?;
    if records.len() != KEYS.len() || records.iter().any(|r| !KEYS.contains(&r.key)) {
        return Err(Error::Invalid);
    }
    let get = |i: usize| {
        records
            .iter()
            .find(|r| r.key == KEYS[i])
            .map(|r| r.value)
            .ok_or(Error::Invalid)
    };
    if get(0)? != "1" || !valid(get(1)?, get(3)?, get(4)?) {
        return Err(Error::Invalid);
    }
    let class = match get(2)? {
        "attribute" => MetadataClass::Attribute,
        "security" => MetadataClass::Security,
        _ => return Err(Error::Invalid),
    };
    Ok((
        get(1)?,
        class,
        MetadataEntry {
            key: get(3)?.into(),
            encoding: get(4)?.into(),
            size: member::unsigned(get(5)?).map_err(|_| Error::Invalid)?,
        },
    ))
}
pub(crate) fn ordinary(m: &member::Member<'_>, expected: &str) -> bool {
    m.path == expected
        && m.kind == tar::Kind::File
        && m.mode == 0o600
        && m.uid == 0
        && m.gid == 0
        && m.mtime
            == (member::Timestamp {
                seconds: 0,
                nanos: 0,
            })
        && m.link.is_empty()
        && m.uname.is_empty()
        && m.gname.is_empty()
}
impl<P: SnapshotBackend> Captured<'_, '_, P> {
    /// Failure permanently withholds envelope completion, including source errors.
    pub fn export<W: Write>(
        &mut self,
        archive: &mut envelope::Writer<W>,
        binding: &Binding<'_>,
        scratch: &mut [u8],
        limits: pax::Limits,
    ) -> Result<(), Error> {
        let result = self.export_inner(archive, binding, scratch, limits);
        if result.is_err() {
            archive.invalidate();
        }
        result
    }
    fn export_inner<W: Write>(
        &mut self,
        archive: &mut envelope::Writer<W>,
        binding: &Binding<'_>,
        scratch: &mut [u8],
        limits: pax::Limits,
    ) -> Result<(), Error> {
        if scratch.is_empty() {
            return Err(Error::Limit);
        }
        let wire = descriptor(binding, limits)?;
        let size = binding.entry.size.to_string();
        let override_wire = pax::encode(
            &[pax::Record {
                key: "size",
                value: &size,
            }],
            limits,
        )
        .map_err(Error::Pax)?;
        // Preflight raw descriptor/override header sizes before archive mutation.
        let desc_header = header(
            path(binding.ordinal, "pax"),
            tar::Kind::File,
            wire.len() as u64,
        );
        let size_header = header(
            path(binding.ordinal, "size"),
            tar::Kind::PaxLocal,
            override_wire.len() as u64,
        );
        desc_header.encode().map_err(|_| Error::Limit)?;
        size_header.encode().map_err(|_| Error::Limit)?;
        archive.start(&desc_header, None).map_err(Error::Envelope)?;
        archive.write_payload(&wire).map_err(Error::Envelope)?;
        archive.start(&size_header, None).map_err(Error::Envelope)?;
        archive
            .write_payload(&override_wire)
            .map_err(Error::Envelope)?;
        archive
            .start(
                &header(path(binding.ordinal, "bin"), tar::Kind::File, 0),
                Some(binding.entry.size),
            )
            .map_err(Error::Envelope)?;
        let mut offset = 0;
        while offset < binding.entry.size {
            let want = (binding.entry.size - offset).min(scratch.len() as u64) as usize;
            let count = self
                .client
                .metadata_read(
                    self.reader,
                    self.object,
                    binding.class,
                    &binding.entry.key,
                    offset,
                    &mut scratch[..want],
                )
                .map_err(Error::Backup)?;
            if count == 0 || count > want {
                return Err(Error::Invalid);
            }
            archive
                .write_payload(&scratch[..count])
                .map_err(Error::Envelope)?;
            offset += count as u64;
        }
        if self
            .client
            .metadata_read(
                self.reader,
                self.object,
                binding.class,
                &binding.entry.key,
                offset,
                &mut scratch[..1],
            )
            .map_err(Error::Backup)?
            != 0
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
/// A private value awaiting verified input from its original reader.
pub struct Staged<U, O> {
    class: MetadataClass,
    entry: MetadataEntry,
    upload: RestoreUpload<U, O>,
    identity: Arc<()>,
}
impl<U, O> Staged<U, O> {
    pub fn description(&self) -> (MetadataClass, &MetadataEntry) {
        (self.class, &self.entry)
    }
    pub fn publish<R: Read, P: OpaqueRestoreBackend<Upload = U, Object = O>>(
        self,
        archive: &stream::Reader<R>,
        client: &mut RestoreClient<'_, P>,
    ) -> Result<(), Error> {
        if !archive.can_publish() || !Arc::ptr_eq(&self.identity, &archive.identity()) {
            return Err(Error::Invalid);
        }
        client.finish_opaque(self.upload).map_err(Error::Restore)
    }
}
/// Stage one bound value. Publication requires verified input from this exact reader.
/// The complete consumer also owns inventory uniqueness and final synchronization.
pub fn stage<R: Read, P: OpaqueRestoreBackend>(
    archive: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &Target<'_, P::Object>,
    scratch: &mut [u8],
    limits: pax::Limits,
) -> Result<Staged<P::Upload, P::Object>, Error> {
    let result = stage_inner(archive, client, target, scratch, limits);
    if result.is_err() {
        archive.invalidate();
    }
    result
}
fn stage_inner<R: Read, P: OpaqueRestoreBackend>(
    archive: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &Target<'_, P::Object>,
    scratch: &mut [u8],
    limits: pax::Limits,
) -> Result<Staged<P::Upload, P::Object>, Error> {
    if scratch.is_empty() {
        return Err(Error::Limit);
    }
    let m = archive
        .next_member()
        .map_err(Error::Stream)?
        .ok_or(Error::Invalid)?;
    if !ordinary(&m, &path(target.ordinal, "pax")) {
        return Err(Error::Invalid);
    }
    let size = usize::try_from(m.size).map_err(|_| Error::Limit)?;
    if size > limits.bytes {
        return Err(Error::Limit);
    }
    let mut wire = vec![0; size];
    if size != 0 && archive.read_payload(&mut wire).map_err(Error::Stream)? != size {
        return Err(Error::Invalid);
    }
    let (source, class, entry) = parse(&wire, limits)?;
    if source != target.path {
        return Err(Error::Invalid);
    }
    let m = archive
        .next_member()
        .map_err(Error::Stream)?
        .ok_or(Error::Invalid)?;
    if !ordinary(&m, &path(target.ordinal, "bin")) || m.size != entry.size {
        return Err(Error::Invalid);
    }
    let mut upload = client
        .begin_opaque(target.object, class, &entry)
        .map_err(Error::Restore)?;
    let mut left = entry.size;
    while left != 0 {
        let want = left.min(scratch.len() as u64) as usize;
        let count = archive
            .read_payload(&mut scratch[..want])
            .map_err(Error::Stream)?;
        if count == 0 || count > want {
            return Err(Error::Invalid);
        }
        client
            .write_opaque(&mut upload, &scratch[..count])
            .map_err(Error::Restore)?;
        left -= count as u64;
    }
    Ok(Staged {
        class,
        entry,
        upload,
        identity: archive.identity(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_fields_versions_and_budgets_are_not_optional() {
        let limits = pax::Limits {
            bytes: 4096,
            records: 16,
            key_bytes: 64,
            value_bytes: 2048,
        };
        let entry = MetadataEntry {
            key: "vendor.key".into(),
            encoding: "unknown/v9".into(),
            size: u64::MAX,
        };
        let wire = descriptor(
            &Binding {
                ordinal: u64::MAX,
                path: "files",
                class: MetadataClass::Attribute,
                entry: &entry,
            },
            limits,
        )
        .unwrap();
        let (source, class, got) = parse(&wire, limits).unwrap();
        assert_eq!(source, "files");
        assert_eq!(class, MetadataClass::Attribute);
        assert_eq!(got, entry);
        let records = pax::decode(&wire, limits).unwrap();
        for index in 0..records.len() {
            let mut changed = records.clone();
            changed.remove(index);
            assert!(parse(&pax::encode(&changed, limits).unwrap(), limits).is_err());
        }
        for (index, value) in [
            (0, "2"),
            (1, "../escape"),
            (2, "unknown"),
            (3, ""),
            (4, ""),
            (5, "18446744073709551616"),
        ] {
            let mut changed = records.clone();
            changed[index].value = value;
            assert!(parse(&pax::encode(&changed, limits).unwrap(), limits).is_err());
        }
        let mut changed = records.clone();
        changed[0].key = "future";
        assert!(parse(&pax::encode(&changed, limits).unwrap(), limits).is_err());
        let mut duplicate = wire.clone();
        duplicate.extend_from_slice(&wire);
        assert!(parse(&duplicate, limits).is_err());
        assert!(parse(
            &wire,
            pax::Limits {
                bytes: wire.len() - 1,
                ..limits
            }
        )
        .is_err());
        for end in 0..wire.len() {
            assert!(parse(&wire[..end], limits).is_err());
        }
    }
}
