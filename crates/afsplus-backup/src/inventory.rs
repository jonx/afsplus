//! ADR-086 full-preservation object inventory orchestration.
use crate::{
    attachment::{self, Binding, Captured, Error, Target},
    envelope, member, pax, stream, tar,
};
use afsplus_vfs::{
    backup::{
        BackupError, InventoryKnowledge, MetadataClass, MetadataEntry, MetadataInventory,
        SnapshotBackend,
    },
    restore::{OpaqueRestoreBackend, RestoreClient},
    VfsError,
};
use sha2::{Digest, Sha512_256};
use std::io::{Read, Write};
#[derive(Clone, Copy)]
pub struct Limits {
    pub values: u64,
    pub value_bytes: u128,
    pub page_entries: usize,
    pub records: pax::Limits,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub count: u64,
    pub bytes: u128,
    pub hash: [u8; 32],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub attributes: Channel,
    pub security: Channel,
    pub next_ordinal: Option<u64>,
}
struct Accumulator {
    count: u64,
    bytes: u128,
    hash: Sha512_256,
    previous: Option<String>,
}
impl Accumulator {
    fn new(class: MetadataClass) -> Self {
        let mut hash = Sha512_256::new();
        hash.update(b"AROS.inventory.v1\0");
        hash.update([if class == MetadataClass::Attribute {
            0
        } else {
            1
        }]);
        Self {
            count: 0,
            bytes: 0,
            hash,
            previous: None,
        }
    }
    fn add(&mut self, entry: &MetadataEntry, limit: &Limits) -> Result<(), Error> {
        if self.previous.as_ref().is_some_and(|p| entry.key <= *p) {
            return Err(Error::Invalid);
        }
        self.count = self.count.checked_add(1).ok_or(Error::Limit)?;
        self.bytes = self
            .bytes
            .checked_add(entry.size as u128)
            .ok_or(Error::Limit)?;
        if self.count > limit.values || self.bytes > limit.value_bytes {
            return Err(Error::Limit);
        }
        for text in [&entry.key, &entry.encoding] {
            self.hash.update((text.len() as u64).to_le_bytes());
            self.hash.update(text.as_bytes());
        }
        self.hash.update(entry.size.to_le_bytes());
        self.previous = Some(entry.key.clone());
        Ok(())
    }
    fn summary(&self) -> Channel {
        Channel {
            count: self.count,
            bytes: self.bytes,
            hash: self.hash.clone().finalize().into(),
        }
    }
}
fn known(knowledge: InventoryKnowledge, count: u64) -> Result<(), Error> {
    match knowledge {
        InventoryKnowledge::Empty if count == 0 => Ok(()),
        InventoryKnowledge::Present if count != 0 => Ok(()),
        InventoryKnowledge::Uninspected => Err(Error::Backup(BackupError::Filesystem(
            VfsError::NotSupported,
        ))),
        _ => Err(Error::Invalid),
    }
}
fn admit(summary: &Summary, ordinal: u64, limits: &Limits) -> Result<(), Error> {
    if limits.page_entries == 0 || limits.page_entries > 64 {
        return Err(Error::Limit);
    }
    let count = summary
        .attributes
        .count
        .checked_add(summary.security.count)
        .ok_or(Error::Limit)?;
    let bytes = summary
        .attributes
        .bytes
        .checked_add(summary.security.bytes)
        .ok_or(Error::Limit)?;
    if count > limits.values || bytes > limits.value_bytes || ordinal.checked_add(count).is_none() {
        return Err(Error::Limit);
    }
    Ok(())
}
fn finish(
    attributes: Channel,
    security: Channel,
    ordinal: u64,
    limits: &Limits,
) -> Result<Summary, Error> {
    let count = attributes
        .count
        .checked_add(security.count)
        .ok_or(Error::Limit)?;
    let summary = Summary {
        attributes,
        security,
        next_ordinal: ordinal.checked_add(count).and_then(|n| n.checked_add(1)),
    };
    admit(&summary, ordinal, limits)?;
    Ok(summary)
}
fn channel<P: SnapshotBackend>(
    source: &mut Captured<'_, '_, P>,
    class: MetadataClass,
    knowledge: InventoryKnowledge,
    limits: &Limits,
) -> Result<Channel, Error> {
    let mut acc = Accumulator::new(class);
    if knowledge == InventoryKnowledge::Uninspected {
        known(knowledge, 0)?;
    }
    if knowledge == InventoryKnowledge::Present {
        loop {
            let page = source
                .client
                .metadata_page(
                    source.reader,
                    source.object,
                    class,
                    acc.previous.as_deref(),
                    limits.page_entries,
                )
                .map_err(Error::Backup)?;
            for entry in page.entries {
                acc.add(&entry, limits)?;
            }
            if page.eof {
                break;
            }
        }
    }
    known(knowledge, acc.count)?;
    Ok(acc.summary())
}
fn name(ordinal: u64) -> String {
    format!("_AROS_BACKUP/metadata/inventory-{ordinal}.pax")
}
fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
const KEYS: [&str; 8] = [
    "AROS.inventory.version",
    "AROS.inventory.path",
    "AROS.inventory.attributes",
    "AROS.inventory.security",
    "AROS.inventory.attribute_bytes",
    "AROS.inventory.security_bytes",
    "AROS.inventory.attribute_hash",
    "AROS.inventory.security_hash",
];
fn encode(path: &str, summary: &Summary, limits: pax::Limits) -> Result<Vec<u8>, Error> {
    if !envelope::canonical(path, true, false) {
        return Err(Error::Invalid);
    }
    if path.len() > limits.value_bytes || path.len() > limits.bytes {
        return Err(Error::Limit);
    }
    let strings = [
        "1".to_owned(),
        path.to_owned(),
        summary.attributes.count.to_string(),
        summary.security.count.to_string(),
        summary.attributes.bytes.to_string(),
        summary.security.bytes.to_string(),
        hex(&summary.attributes.hash),
        hex(&summary.security.hash),
    ];
    let records: [pax::Record<'_>; 8] = std::array::from_fn(|i| pax::Record {
        key: KEYS[i],
        value: &strings[i],
    });
    pax::encode(&records, limits).map_err(Error::Pax)
}
fn decimal(text: &str) -> Result<u128, Error> {
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
        return Err(Error::Invalid);
    }
    text.bytes().try_fold(0u128, |n, c| {
        if c.is_ascii_digit() {
            n.checked_mul(10)
                .and_then(|n| n.checked_add((c - b'0') as u128))
                .ok_or(Error::Invalid)
        } else {
            Err(Error::Invalid)
        }
    })
}
fn hash(text: &str) -> Result<[u8; 32], Error> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid);
    }
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| Error::Invalid)?;
    }
    Ok(out)
}
fn decode<'a>(bytes: &'a [u8], ordinal: u64, limits: &Limits) -> Result<(&'a str, Summary), Error> {
    let records = pax::decode(bytes, limits.records).map_err(Error::Pax)?;
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
    if get(0)? != "1" || !envelope::canonical(get(1)?, true, false) {
        return Err(Error::Invalid);
    }
    let attributes = Channel {
        count: member::unsigned(get(2)?).map_err(|_| Error::Invalid)?,
        bytes: decimal(get(4)?)?,
        hash: hash(get(6)?)?,
    };
    let security = Channel {
        count: member::unsigned(get(3)?).map_err(|_| Error::Invalid)?,
        bytes: decimal(get(5)?)?,
        hash: hash(get(7)?)?,
    };
    Ok((get(1)?, finish(attributes, security, ordinal, limits)?))
}
/// Full preservation only. Unknown inventory knowledge is not an empty manifest.
pub fn export<P: SnapshotBackend, W: Write>(
    source: &mut Captured<'_, '_, P>,
    archive: &mut envelope::Writer<W>,
    ordinal: u64,
    path: &str,
    scratch: &mut [u8],
    limits: Limits,
) -> Result<Summary, Error> {
    let result = export_inner(source, archive, ordinal, path, scratch, limits);
    if result.is_err() {
        archive.invalidate();
    }
    result
}
fn export_inner<P: SnapshotBackend, W: Write>(
    source: &mut Captured<'_, '_, P>,
    archive: &mut envelope::Writer<W>,
    ordinal: u64,
    path: &str,
    scratch: &mut [u8],
    limits: Limits,
) -> Result<Summary, Error> {
    if limits.page_entries == 0 || limits.page_entries > 64 || scratch.is_empty() {
        return Err(Error::Limit);
    }
    let knowledge = source
        .client
        .metadata_inventory(source.reader, source.object)
        .map_err(Error::Backup)?;
    let attributes = channel(
        source,
        MetadataClass::Attribute,
        knowledge.attributes,
        &limits,
    )?;
    let security = channel(source, MetadataClass::Security, knowledge.security, &limits)?;
    let expected = finish(attributes, security, ordinal, &limits)?;
    let wire = encode(path, &expected, limits.records)?;
    archive
        .start(
            &attachment::header(name(ordinal), tar::Kind::File, wire.len() as u64),
            None,
        )
        .map_err(Error::Envelope)?;
    archive.write_payload(&wire).map_err(Error::Envelope)?;
    let mut next = ordinal;
    for (class, planned) in [
        (MetadataClass::Attribute, &expected.attributes),
        (MetadataClass::Security, &expected.security),
    ] {
        let mut actual = Accumulator::new(class);
        if planned.count != 0 {
            loop {
                let page = source
                    .client
                    .metadata_page(
                        source.reader,
                        source.object,
                        class,
                        actual.previous.as_deref(),
                        limits.page_entries,
                    )
                    .map_err(Error::Backup)?;
                for entry in page.entries {
                    actual.add(&entry, &limits)?;
                    if actual.count > planned.count || actual.bytes > planned.bytes {
                        return Err(Error::Invalid);
                    }
                    next = next.checked_add(1).ok_or(Error::Limit)?;
                    source.export(
                        archive,
                        &Binding {
                            ordinal: next,
                            path,
                            class,
                            entry: &entry,
                        },
                        scratch,
                        limits.records,
                    )?;
                }
                if page.eof {
                    break;
                }
            }
        }
        if actual.summary() != *planned {
            return Err(Error::Invalid);
        }
    }
    Ok(expected)
}
/// Requires verified replay. Failure may leave prior published values, but
/// never returns an inventory-success summary or permits further publication.
pub fn restore<R: Read, P: OpaqueRestoreBackend>(
    archive: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &Target<'_, P::Object>,
    knowledge: MetadataInventory,
    scratch: &mut [u8],
    limits: Limits,
) -> Result<Summary, Error> {
    let result = restore_inner(archive, client, target, knowledge, scratch, limits);
    if result.is_err() {
        archive.invalidate();
    }
    result
}
fn restore_inner<R: Read, P: OpaqueRestoreBackend>(
    archive: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &Target<'_, P::Object>,
    knowledge: MetadataInventory,
    scratch: &mut [u8],
    limits: Limits,
) -> Result<Summary, Error> {
    if !archive.can_publish() {
        return Err(Error::NeedsVerifiedReplay);
    }
    if scratch.is_empty() {
        return Err(Error::Limit);
    }
    let m = archive
        .next_member()
        .map_err(Error::Stream)?
        .ok_or(Error::Invalid)?;
    if !attachment::ordinary(&m, &name(target.ordinal)) {
        return Err(Error::Invalid);
    }
    let size = usize::try_from(m.size).map_err(|_| Error::Limit)?;
    if size > limits.records.bytes {
        return Err(Error::Limit);
    }
    let mut wire = vec![0; size];
    if size != 0 && archive.read_payload(&mut wire).map_err(Error::Stream)? != size {
        return Err(Error::Invalid);
    }
    let (path, expected) = decode(&wire, target.ordinal, &limits)?;
    if path != target.path {
        return Err(Error::Invalid);
    }
    known(knowledge.attributes, expected.attributes.count)?;
    known(knowledge.security, expected.security.count)?;
    let mut next = target.ordinal;
    for (class, planned) in [
        (MetadataClass::Attribute, &expected.attributes),
        (MetadataClass::Security, &expected.security),
    ] {
        let mut actual = Accumulator::new(class);
        for _ in 0..planned.count {
            next = next.checked_add(1).ok_or(Error::Limit)?;
            let staged = attachment::stage(
                archive,
                client,
                &Target {
                    ordinal: next,
                    path: target.path,
                    object: target.object,
                },
                scratch,
                limits.records,
            )?;
            let (actual_class, entry) = staged.description();
            if actual_class != class {
                return Err(Error::Invalid);
            }
            actual.add(entry, &limits)?;
            if actual.bytes > planned.bytes {
                return Err(Error::Invalid);
            }
            if actual.count == planned.count && actual.summary() != *planned {
                return Err(Error::Invalid);
            }
            staged.publish(archive, client)?;
        }
        if actual.summary() != *planned {
            return Err(Error::Invalid);
        }
    }
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> Limits {
        Limits {
            values: u64::MAX,
            value_bytes: u128::MAX,
            page_entries: 1,
            records: pax::Limits {
                bytes: 4096,
                records: 8,
                key_bytes: 64,
                value_bytes: 2048,
            },
        }
    }
    fn empty() -> Summary {
        finish(
            Accumulator::new(MetadataClass::Attribute).summary(),
            Accumulator::new(MetadataClass::Security).summary(),
            0,
            &limits(),
        )
        .unwrap()
    }
    #[test]
    fn manifest_strict_fields_numbers_hashes_and_ordinal_boundaries() {
        let expected = empty();
        let wire = encode("files", &expected, limits().records).unwrap();
        assert_eq!(
            decode(&wire, 0, &limits()).unwrap(),
            ("files", expected.clone())
        );
        assert_eq!(
            decode(&wire, u64::MAX, &limits()).unwrap().1.next_ordinal,
            None
        );
        let records = pax::decode(&wire, limits().records).unwrap();
        for missing in 0..8 {
            let mut bad = records.clone();
            bad.remove(missing);
            let wire = pax::encode(&bad, limits().records).unwrap();
            assert!(decode(&wire, 0, &limits()).is_err());
        }
        for (field, value) in [
            (0, "2"),
            (1, "../files"),
            (2, "01"),
            (3, "-1"),
            (4, "+0"),
            (5, "340282366920938463463374607431768211456"),
            (6, "ABCDEF"),
            (7, "00"),
        ] {
            let mut bad = records.clone();
            bad[field].value = value;
            let wire = pax::encode(&bad, limits().records).unwrap();
            assert!(decode(&wire, 0, &limits()).is_err());
        }
        let mut expected = expected;
        expected.security.count = 1;
        assert!(finish(
            expected.attributes.clone(),
            expected.security.clone(),
            u64::MAX,
            &limits()
        )
        .is_err());
        let mut bounded = limits();
        bounded.values = 0;
        assert!(admit(&expected, 0, &bounded).is_err());
        expected.security.count = u64::MAX;
        expected.attributes.count = 1;
        assert!(admit(&expected, 0, &limits()).is_err());
        expected.security.count = 0;
        expected.attributes.count = 0;
        expected.security.bytes = u128::MAX;
        expected.attributes.bytes = 1;
        assert!(admit(&expected, 0, &limits()).is_err());
        for end in 0..wire.len() {
            assert!(decode(&wire[..end], 0, &limits()).is_err());
        }
    }
    #[test]
    fn descriptor_order_and_complete_hash_binding() {
        let entry = MetadataEntry {
            key: "alpha".into(),
            encoding: "opaque/v1".into(),
            size: 7,
        };
        let mut acc = Accumulator::new(MetadataClass::Security);
        acc.add(&entry, &limits()).unwrap();
        let original = acc.summary();
        assert!(acc.add(&entry, &limits()).is_err());
        for change in 0..3 {
            let mut changed = entry.clone();
            match change {
                0 => changed.key = "beta".into(),
                1 => changed.encoding = "opaque/v2".into(),
                _ => changed.size += 1,
            }
            let mut acc = Accumulator::new(MetadataClass::Security);
            acc.add(&changed, &limits()).unwrap();
            assert_ne!(acc.summary().hash, original.hash);
        }
        let mut other = Accumulator::new(MetadataClass::Attribute);
        other.add(&entry, &limits()).unwrap();
        assert_ne!(other.summary().hash, original.hash);
        let mut acc = Accumulator::new(MetadataClass::Security);
        let mut bounded = limits();
        bounded.value_bytes = 6;
        assert!(acc.add(&entry, &bounded).is_err());
    }
}
