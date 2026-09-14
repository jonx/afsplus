//! ADR-093 directory/alias components; enclosing job owns graph completeness.
use crate::{
    attachment, envelope,
    file::{self, Error, Mode},
    inventory, metadata, pax, stream, tar,
};
use afsplus_format::Timespec;
use afsplus_vfs::{
    backup::{MetadataInventory, SnapshotBackend},
    restore::{
        OpaqueRestoreBackend, RestoreBackend, RestoreClient, RestoreError, RestoreMetadata,
        RestoreObject,
    },
    NodeKind, Stat, VfsError,
};
use std::io::{Read, Write};
#[derive(Clone, Copy)]
pub enum Entry<'a> {
    Directory,
    HardLink { primary_path: &'a str },
}
pub struct Binding<'a> {
    pub ordinal: u64,
    pub path: &'a str,
    pub entry: Entry<'a>,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub records: pax::Limits,
    pub inventory: inventory::Limits,
}
#[derive(Clone, Copy)]
pub struct RestoreOptions {
    pub mode: Mode,
    pub limits: Limits,
}
#[derive(Debug)]
pub struct ExportReport {
    pub next_ordinal: Option<u64>,
    pub knowledge: MetadataInventory,
    pub inventory: Option<inventory::Summary>,
}
#[derive(Debug)]
pub struct DirectoryReport {
    pub next_ordinal: Option<u64>,
    pub mode: Mode,
    pub archive_mode: Mode,
    pub opaque: file::OpaqueDisposition,
}
pub struct Primary<'a, O> {
    pub object: &'a RestoreObject<O>,
    pub path: &'a str,
    pub knowledge: MetadataInventory,
}
pub struct AliasTarget<'a, O> {
    pub ordinal: u64,
    pub path: &'a str,
    pub parent: &'a RestoreObject<O>,
    pub name: &'a str,
    pub primary: Primary<'a, O>,
}
#[derive(Debug)]
pub struct AliasReport {
    pub next_ordinal: Option<u64>,
    pub object_id: u64,
    pub links: u32,
    pub mode: Mode,
    pub archive_mode: Mode,
}
fn name(kind: tar::Kind, mode: Mode, n: u64) -> String {
    format!(
        "_AROS_BACKUP/metadata/{}-v1-{}-{n}.pax",
        if kind == tar::Kind::Directory {
            "directory"
        } else {
            "hardlink"
        },
        if mode == Mode::Full {
            "full"
        } else {
            "recovery"
        }
    )
}
fn raw_name(n: u64) -> String {
    format!("files/namespace.{n}")
}
fn slots(n: u64, kind: tar::Kind, mode: Mode) -> Result<(u64, Option<u64>), Error> {
    let body = n.checked_add(1).ok_or(Error::Limit)?;
    let inventory = if kind == tar::Kind::Directory && mode == Mode::Full {
        Some(body.checked_add(1).ok_or(Error::Limit)?)
    } else {
        None
    };
    Ok((body, inventory))
}
fn emit_body<W: Write>(
    writer: &mut envelope::Writer<W>,
    n: u64,
    object: &metadata::Object<'_>,
    link: &str,
    limits: pax::Limits,
) -> Result<(), Error> {
    let time = object.modified.decimal().map_err(|_| Error::Invalid)?;
    let mut records = vec![
        pax::Record {
            key: "path",
            value: object.path,
        },
        pax::Record {
            key: "mtime",
            value: &time,
        },
    ];
    if object.kind == tar::Kind::HardLink {
        records.push(pax::Record {
            key: "linkpath",
            value: link,
        });
    }
    let wire =
        pax::encode(&records, limits).map_err(|e| Error::Metadata(metadata::Error::Pax(e)))?;
    writer
        .start(
            &attachment::header(
                format!("_AROS_BACKUP/metadata/namespace-{n}.pax"),
                tar::Kind::PaxLocal,
                wire.len() as u64,
            ),
            None,
        )
        .map_err(Error::Envelope)?;
    writer.write_payload(&wire).map_err(Error::Envelope)?;
    let mut header = attachment::header(raw_name(n), object.kind, 0);
    if object.kind == tar::Kind::Directory {
        header.mode = 0o700;
    } else {
        header.link = "files/primary".into();
    }
    writer.start(&header, None).map_err(Error::Envelope)
}
pub fn export<P: SnapshotBackend, W: Write>(
    source: &mut attachment::Captured<'_, '_, P>,
    writer: &mut envelope::Writer<W>,
    binding: &Binding<'_>,
    mode: Mode,
    scratch: &mut [u8],
    limits: Limits,
) -> Result<ExportReport, Error> {
    let result = export_inner(source, writer, binding, mode, scratch, limits);
    if result.is_err() {
        writer.invalidate();
    }
    result
}
fn export_inner<P: SnapshotBackend, W: Write>(
    source: &mut attachment::Captured<'_, '_, P>,
    writer: &mut envelope::Writer<W>,
    binding: &Binding<'_>,
    mode: Mode,
    scratch: &mut [u8],
    limits: Limits,
) -> Result<ExportReport, Error> {
    if scratch.is_empty() {
        return Err(Error::Limit);
    }
    let (kind, expected_kind, link) = match binding.entry {
        Entry::Directory => (tar::Kind::Directory, NodeKind::Directory, ""),
        Entry::HardLink { primary_path } => (tar::Kind::HardLink, NodeKind::File, primary_path),
    };
    if kind == tar::Kind::HardLink
        && (!envelope::canonical(link, false, false) || link == binding.path)
    {
        return Err(Error::Invalid);
    }
    let (body, inventory_ordinal) = slots(binding.ordinal, kind, mode)?;
    let stat = source
        .client
        .stat(source.reader, source.object)
        .map_err(Error::Source)?;
    if stat.kind != expected_kind {
        return Err(Error::Unsupported);
    }
    let knowledge = source
        .client
        .metadata_inventory(source.reader, source.object)
        .map_err(Error::Source)?;
    if mode == Mode::Full && !file::inspected(knowledge) {
        return Err(Error::Unsupported);
    }
    let object = metadata::Object {
        path: binding.path,
        kind,
        protection: stat.protection,
        created: file::timestamp(stat.created),
        modified: file::timestamp(stat.modified),
        changed: file::timestamp(stat.changed),
        attributes: file::inventory_state(knowledge.attributes),
        security: file::inventory_state(knowledge.security),
    };
    let wire = metadata::encode(&object, limits.records).map_err(Error::Metadata)?;
    writer
        .start(
            &attachment::header(
                name(kind, mode, binding.ordinal),
                tar::Kind::File,
                wire.len() as u64,
            ),
            None,
        )
        .map_err(Error::Envelope)?;
    writer.write_payload(&wire).map_err(Error::Envelope)?;
    emit_body(writer, body, &object, link, limits.records)?;
    let inventory = if let Some(n) = inventory_ordinal {
        Some(
            inventory::export(source, writer, n, binding.path, scratch, limits.inventory)
                .map_err(Error::Inventory)?,
        )
    } else {
        None
    };
    if source
        .client
        .stat(source.reader, source.object)
        .map_err(Error::Source)?
        != stat
        || source
            .client
            .metadata_inventory(source.reader, source.object)
            .map_err(Error::Source)?
            != knowledge
    {
        return Err(Error::Invalid);
    }
    Ok(ExportReport {
        next_ordinal: inventory
            .as_ref()
            .map_or(body.checked_add(1), |s| s.next_ordinal),
        knowledge,
        inventory,
    })
}
fn read_object<R: Read>(
    reader: &mut stream::Reader<R>,
    n: u64,
    kind: tar::Kind,
    limits: pax::Limits,
) -> Result<(Mode, Vec<u8>), Error> {
    let m = reader
        .next_member()
        .map_err(Error::Stream)?
        .ok_or(Error::Invalid)?;
    if m.sparse_size.is_some() {
        return Err(Error::Invalid);
    }
    let mode = if attachment::ordinary(&m, &name(kind, Mode::Full, n)) {
        Mode::Full
    } else if attachment::ordinary(&m, &name(kind, Mode::Recovery, n)) {
        Mode::Recovery
    } else {
        return Err(Error::Invalid);
    };
    let size = usize::try_from(m.size).map_err(|_| Error::Limit)?;
    if !reader.raw_path_is(&name(kind, mode, n)) {
        return Err(Error::Invalid);
    }
    if size > limits.bytes {
        return Err(Error::Limit);
    }
    let mut wire = Vec::new();
    wire.try_reserve_exact(size).map_err(|_| Error::Limit)?;
    wire.resize(size, 0);
    if reader.read_payload(&mut wire).map_err(Error::Stream)? != size {
        return Err(Error::Invalid);
    }
    Ok((mode, wire))
}
fn read_body<R: Read>(
    reader: &mut stream::Reader<R>,
    n: u64,
    object: &metadata::Object<'_>,
    link: &str,
) -> Result<(), Error> {
    let m = reader
        .next_member()
        .map_err(Error::Stream)?
        .ok_or(Error::Invalid)?;
    let mode = if object.kind == tar::Kind::Directory {
        0o700
    } else {
        0o600
    };
    if m.kind != object.kind
        || m.path != object.path
        || m.link != link
        || m.mode != mode
        || m.uid != 0
        || m.gid != 0
        || !m.uname.is_empty()
        || !m.gname.is_empty()
        || m.size != 0
        || m.sparse_size.is_some()
        || m.mtime != object.modified
    {
        return Err(Error::Invalid);
    }
    if !reader.raw_path_is(&raw_name(n))
        || !reader.local_path_is(&format!("_AROS_BACKUP/metadata/namespace-{n}.pax"))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn core_metadata(object: &metadata::Object<'_>) -> RestoreMetadata {
    RestoreMetadata {
        protection: object.protection,
        created: file::timespec(object.created),
        modified: file::timespec(object.modified),
        changed: file::timespec(object.changed),
    }
}
fn same_metadata(stat: &Stat, object: &metadata::Object<'_>) -> bool {
    stat.protection == object.protection
        && file::timestamp(stat.created) == object.created
        && file::timestamp(stat.modified) == object.modified
        && file::timestamp(stat.changed) == object.changed
}
pub fn restore_directory<R: Read, P: OpaqueRestoreBackend>(
    reader: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &attachment::Target<'_, P::Object>,
    scratch: &mut [u8],
    options: RestoreOptions,
) -> Result<DirectoryReport, Error> {
    let result = restore_directory_inner(reader, client, target, scratch, options);
    if result.is_err() {
        reader.invalidate();
    }
    result
}
fn restore_directory_inner<R: Read, P: OpaqueRestoreBackend>(
    reader: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &attachment::Target<'_, P::Object>,
    scratch: &mut [u8],
    options: RestoreOptions,
) -> Result<DirectoryReport, Error> {
    if !reader.can_publish() {
        return Err(Error::NeedsVerifiedReplay);
    }
    if scratch.is_empty() {
        return Err(Error::Limit);
    }
    let (archive_mode, wire) = read_object(
        reader,
        target.ordinal,
        tar::Kind::Directory,
        options.limits.records,
    )?;
    if options.mode == Mode::Full && archive_mode != Mode::Full {
        return Err(Error::Unsupported);
    }
    let object = metadata::decode(&wire, options.limits.records).map_err(Error::Metadata)?;
    if object.kind != tar::Kind::Directory || object.path != target.path {
        return Err(Error::Invalid);
    }
    let knowledge = file::knowledge(&object);
    if archive_mode == Mode::Full && !file::inspected(knowledge) {
        return Err(Error::Invalid);
    }
    let (body, inventory_ordinal) = slots(target.ordinal, object.kind, archive_mode)?;
    read_body(reader, body, &object, "")?;
    if !client
        .directory_empty(target.object)
        .map_err(Error::Destination)?
    {
        return Err(Error::Destination(RestoreError::Filesystem(
            VfsError::DirectoryNotEmpty,
        )));
    }
    let inventory = if let Some(n) = inventory_ordinal {
        Some(
            if options.mode == Mode::Full {
                inventory::restore(
                    reader,
                    client,
                    &attachment::Target {
                        ordinal: n,
                        path: target.path,
                        object: target.object,
                    },
                    knowledge,
                    scratch,
                    options.limits.inventory,
                )
            } else {
                inventory::discard(
                    reader,
                    n,
                    target.path,
                    knowledge,
                    scratch,
                    options.limits.inventory,
                )
            }
            .map_err(Error::Inventory)?,
        )
    } else {
        None
    };
    client
        .metadata(target.object, core_metadata(&object))
        .map_err(Error::Destination)?;
    let stat = client.stat(target.object).map_err(Error::Destination)?;
    if stat.kind != NodeKind::Directory || !same_metadata(&stat, &object) {
        return Err(Error::Invalid);
    }
    let next_ordinal = inventory
        .as_ref()
        .map_or(body.checked_add(1), |s| s.next_ordinal);
    let opaque = if options.mode == Mode::Full {
        file::OpaqueDisposition::Preserved(inventory.ok_or(Error::Invalid)?)
    } else {
        file::OpaqueDisposition::Omitted {
            knowledge,
            transported: inventory,
        }
    };
    Ok(DirectoryReport {
        next_ordinal,
        mode: options.mode,
        archive_mode,
        opaque,
    })
}
pub fn restore_alias<R: Read, P: RestoreBackend>(
    reader: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &AliasTarget<'_, P::Object>,
    options: RestoreOptions,
    now: Timespec,
) -> Result<AliasReport, Error> {
    let result = restore_alias_inner(reader, client, target, options, now);
    if result.is_err() {
        reader.invalidate();
    }
    result
}
fn restore_alias_inner<R: Read, P: RestoreBackend>(
    reader: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &AliasTarget<'_, P::Object>,
    options: RestoreOptions,
    now: Timespec,
) -> Result<AliasReport, Error> {
    if !reader.can_publish() {
        return Err(Error::NeedsVerifiedReplay);
    }
    let (archive_mode, wire) = read_object(
        reader,
        target.ordinal,
        tar::Kind::HardLink,
        options.limits.records,
    )?;
    if options.mode == Mode::Full && archive_mode != Mode::Full {
        return Err(Error::Unsupported);
    }
    let object = metadata::decode(&wire, options.limits.records).map_err(Error::Metadata)?;
    if object.kind != tar::Kind::HardLink
        || object.path != target.path
        || target.path.rsplit('/').next() != Some(target.name)
        || target.path == target.primary.path
        || !envelope::canonical(target.primary.path, false, false)
    {
        return Err(Error::Invalid);
    }
    let knowledge = file::knowledge(&object);
    if knowledge != target.primary.knowledge
        || (archive_mode == Mode::Full && !file::inspected(knowledge))
    {
        return Err(Error::Invalid);
    }
    let (body, _) = slots(target.ordinal, object.kind, archive_mode)?;
    read_body(reader, body, &object, target.primary.path)?;
    let before = client
        .stat(target.primary.object)
        .map_err(Error::Destination)?;
    if before.kind != NodeKind::File || !same_metadata(&before, &object) {
        return Err(Error::Invalid);
    }
    let links = before.links.checked_add(1).ok_or(Error::Limit)?;
    // This also proves a spare lookup handle slot and provider support before
    // mutation. The exclusively borrowed service cannot add other handles here.
    match client.lookup_created(target.parent, target.name) {
        Err(RestoreError::Filesystem(VfsError::NotFound)) => {}
        Err(e) => return Err(Error::Destination(e)),
        Ok(_) => {
            return Err(Error::Destination(RestoreError::Filesystem(
                VfsError::AlreadyExists,
            )))
        }
    }
    client
        .link(target.primary.object, target.parent, target.name, now)
        .map_err(Error::Destination)?;
    let alias = client
        .lookup_created(target.parent, target.name)
        .map_err(Error::Destination)?;
    client
        .metadata(target.primary.object, core_metadata(&object))
        .map_err(Error::Destination)?;
    let after = client.stat(&alias).map_err(Error::Destination)?;
    if after.kind != NodeKind::File
        || after.object_id != before.object_id
        || after.size != before.size
        || after.allocated_size != before.allocated_size
        || after.links != links
        || !same_metadata(&after, &object)
    {
        return Err(Error::Invalid);
    }
    Ok(AliasReport {
        next_ordinal: body.checked_add(1),
        object_id: after.object_id,
        links,
        mode: options.mode,
        archive_mode,
    })
}
