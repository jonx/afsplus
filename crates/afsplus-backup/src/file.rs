//! ADR-091 bound primary regular-file groups. Namespace/job completion is separate.
use crate::{
    allocation, attachment, envelope, inventory, member::Timestamp, metadata, pax, sparse, stream,
    tar,
};
use afsplus_format::Timespec;
use afsplus_vfs::{
    backup::{BackupError, InventoryKnowledge, MetadataInventory, SnapshotBackend},
    restore::{
        OpaqueRestoreBackend, RestoreBackend, RestoreClient, RestoreError, RestoreMetadata,
        RestoreObject,
    },
    NodeKind, Stat, VfsError,
};
use std::io::{Read, Write};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Full,
    Recovery,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub contents: sparse::consumer::Options,
    pub inventory: inventory::Limits,
}
#[derive(Clone, Copy)]
pub struct RestoreOptions {
    pub mode: Mode,
    pub allocation: allocation::RestoreOptions,
    pub inventory: inventory::Limits,
}
#[derive(Debug)]
pub enum Error {
    Invalid,
    Limit,
    Unsupported,
    NeedsVerifiedReplay,
    Envelope(envelope::Error),
    Stream(stream::Error),
    Metadata(metadata::Error),
    Source(BackupError),
    Destination(RestoreError),
    Allocation(sparse::Error),
    Inventory(attachment::Error),
    /// The destination cannot keep a value the archive carries for this
    /// object, so the restore refuses rather than drop it (ADR-076).
    Unpreservable {
        path: String,
        what: &'static str,
    },
}
#[derive(Debug, PartialEq, Eq)]
pub enum OpaqueDisposition {
    Preserved(inventory::Summary),
    Omitted {
        knowledge: MetadataInventory,
        transported: Option<inventory::Summary>,
    },
}
#[derive(Debug)]
pub struct ExportReport {
    pub mode: Mode,
    pub next_ordinal: Option<u64>,
    pub allocation: allocation::ExportReport,
    pub inventories: Option<inventory::Summary>,
    pub knowledge: MetadataInventory,
}
#[derive(Debug)]
pub struct Report {
    pub mode: Mode,
    pub archive_mode: Mode,
    pub next_ordinal: Option<u64>,
    pub allocation: allocation::Report,
    pub opaque: OpaqueDisposition,
}
/// What the archive says about an object's protection, owner and times.
pub(crate) fn preserved(object: &metadata::Object<'_>) -> RestoreMetadata {
    RestoreMetadata {
        protection: object.protection,
        owner_uid: object.uid,
        owner_gid: object.gid,
        created: timespec(object.created),
        modified: timespec(object.modified),
        changed: timespec(object.changed),
    }
}

/// Store an object's archived metadata: the comment first, since setting it
/// advances the change time, then protection, owner and times. A destination
/// that cannot keep the comment refuses, naming the object.
pub(crate) fn restore_preserved<P: RestoreBackend>(
    client: &mut RestoreClient<'_, P>,
    target: &RestoreObject<P::Object>,
    path: &str,
    object: &metadata::Object<'_>,
) -> Result<(), Error> {
    if !object.comment.is_empty() {
        match client.set_comment(target, object.comment, timespec(object.changed)) {
            Ok(()) => {}
            Err(RestoreError::Filesystem(VfsError::NotSupported | VfsError::Invalid)) => {
                return Err(Error::Unpreservable {
                    path: path.to_owned(),
                    what: "comment",
                })
            }
            Err(error) => return Err(Error::Destination(error)),
        }
    }
    client
        .metadata(target, preserved(object))
        .map_err(Error::Destination)
}

/// Whether the destination now holds exactly what the archive says. A
/// destination without comments satisfies an empty comment and nothing else.
pub(crate) fn same_preserved<P: RestoreBackend>(
    client: &mut RestoreClient<'_, P>,
    target: &RestoreObject<P::Object>,
    stat: &Stat,
    object: &metadata::Object<'_>,
) -> Result<bool, Error> {
    let comment = match client.comment(target) {
        Ok(comment) => comment,
        Err(RestoreError::Filesystem(VfsError::NotSupported)) if object.comment.is_empty() => {
            String::new()
        }
        Err(error) => return Err(Error::Destination(error)),
    };
    Ok(stat.protection == object.protection
        && stat.owner_uid == object.uid
        && stat.owner_gid == object.gid
        && timestamp(stat.created) == object.created
        && timestamp(stat.modified) == object.modified
        && timestamp(stat.changed) == object.changed
        && comment == object.comment)
}

fn name(mode: Mode, ordinal: u64) -> String {
    format!(
        "_AROS_BACKUP/metadata/file-v1-{}-{ordinal}.pax",
        if mode == Mode::Full {
            "full"
        } else {
            "recovery"
        }
    )
}
pub(crate) fn timestamp(t: Timespec) -> Timestamp {
    Timestamp {
        seconds: t.seconds,
        nanos: t.nanoseconds,
    }
}
pub(crate) fn timespec(t: Timestamp) -> Timespec {
    Timespec {
        seconds: t.seconds,
        nanoseconds: t.nanos,
    }
}
pub(crate) fn inventory_state(k: InventoryKnowledge) -> metadata::Inventory {
    match k {
        InventoryKnowledge::Empty => metadata::Inventory::Empty,
        InventoryKnowledge::Present => metadata::Inventory::Present,
        InventoryKnowledge::Uninspected => metadata::Inventory::Uninspected,
    }
}
pub(crate) fn knowledge(m: &metadata::Object<'_>) -> MetadataInventory {
    let convert = |k| match k {
        metadata::Inventory::Empty => InventoryKnowledge::Empty,
        metadata::Inventory::Present => InventoryKnowledge::Present,
        metadata::Inventory::Uninspected => InventoryKnowledge::Uninspected,
    };
    MetadataInventory {
        attributes: convert(m.attributes),
        security: convert(m.security),
    }
}
pub(crate) fn inspected(k: MetadataInventory) -> bool {
    k.attributes != InventoryKnowledge::Uninspected && k.security != InventoryKnowledge::Uninspected
}
fn ordinals(ordinal: u64, mode: Mode) -> Result<(u64, Option<u64>), Error> {
    let allocation = ordinal.checked_add(1).ok_or(Error::Limit)?;
    let sparse = allocation.checked_add(1).ok_or(Error::Limit)?;
    let inventory = if mode == Mode::Full {
        Some(sparse.checked_add(1).ok_or(Error::Limit)?)
    } else {
        None
    };
    Ok((allocation, inventory))
}
pub fn export<P: SnapshotBackend, W: Write>(
    source: &mut attachment::Captured<'_, '_, P>,
    writer: &mut envelope::Writer<W>,
    binding: (u64, &str),
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
    (ordinal, path): (u64, &str),
    mode: Mode,
    scratch: &mut [u8],
    limits: Limits,
) -> Result<ExportReport, Error> {
    let (allocation_ordinal, inventory_ordinal) = ordinals(ordinal, mode)?;
    if scratch.is_empty() {
        return Err(Error::Limit);
    }
    let stat = source
        .client
        .stat(source.reader, source.object)
        .map_err(Error::Source)?;
    if stat.kind != NodeKind::File {
        return Err(Error::Unsupported);
    }
    let captured = source
        .client
        .metadata_inventory(source.reader, source.object)
        .map_err(Error::Source)?;
    if mode == Mode::Full && !inspected(captured) {
        return Err(Error::Unsupported);
    }
    let comment = source
        .client
        .comment(source.reader, source.object)
        .map_err(Error::Source)?;
    let object = metadata::Object {
        path,
        kind: tar::Kind::File,
        protection: stat.protection,
        comment: &comment,
        uid: stat.owner_uid,
        gid: stat.owner_gid,
        created: timestamp(stat.created),
        modified: timestamp(stat.modified),
        changed: timestamp(stat.changed),
        attributes: inventory_state(captured.attributes),
        security: inventory_state(captured.security),
    };
    let wire = metadata::encode(&object, limits.contents.records).map_err(Error::Metadata)?;
    writer
        .start(
            &attachment::header(name(mode, ordinal), tar::Kind::File, wire.len() as u64),
            None,
        )
        .map_err(Error::Envelope)?;
    writer.write_payload(&wire).map_err(Error::Envelope)?;
    let allocation = allocation::export(
        source,
        writer,
        allocation_ordinal,
        path,
        scratch,
        limits.contents,
    )
    .map_err(Error::Allocation)?;
    let inventories = if let Some(next) = inventory_ordinal {
        if allocation.next_ordinal != Some(next) {
            return Err(Error::Invalid);
        }
        Some(
            inventory::export(source, writer, next, path, scratch, limits.inventory)
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
            != captured
    {
        return Err(Error::Invalid);
    }
    let next_ordinal = inventories
        .as_ref()
        .map_or(allocation.next_ordinal, |s| s.next_ordinal);
    Ok(ExportReport {
        mode,
        next_ordinal,
        allocation,
        inventories,
        knowledge: captured,
    })
}
fn read_object<R: Read>(
    reader: &mut stream::Reader<R>,
    ordinal: u64,
    limits: pax::Limits,
) -> Result<(Mode, Vec<u8>), Error> {
    let m = reader
        .next_member()
        .map_err(Error::Stream)?
        .ok_or(Error::Invalid)?;
    let mode = if attachment::ordinary(&m, &name(Mode::Full, ordinal)) {
        Mode::Full
    } else if attachment::ordinary(&m, &name(Mode::Recovery, ordinal)) {
        Mode::Recovery
    } else {
        return Err(Error::Invalid);
    };
    let size = usize::try_from(m.size).map_err(|_| Error::Limit)?;
    if !reader.raw_path_is(&name(mode, ordinal)) {
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
pub fn restore<R: Read, P: OpaqueRestoreBackend>(
    reader: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &attachment::Target<'_, P::Object>,
    scratch: &mut [u8],
    options: RestoreOptions,
    now: Timespec,
) -> Result<Report, Error> {
    let result = restore_inner(reader, client, target, scratch, options, now);
    if result.is_err() {
        reader.invalidate();
    }
    result
}
fn restore_inner<R: Read, P: OpaqueRestoreBackend>(
    reader: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &attachment::Target<'_, P::Object>,
    scratch: &mut [u8],
    options: RestoreOptions,
    now: Timespec,
) -> Result<Report, Error> {
    if !reader.can_publish() {
        return Err(Error::NeedsVerifiedReplay);
    }
    if scratch.is_empty() {
        return Err(Error::Limit);
    }
    let (archive_mode, wire) =
        read_object(reader, target.ordinal, options.allocation.limits.records)?;
    let (allocation_ordinal, inventory_ordinal) = ordinals(target.ordinal, archive_mode)?;
    if options.mode == Mode::Full && archive_mode != Mode::Full {
        return Err(Error::Unsupported);
    }
    let object =
        metadata::decode(&wire, options.allocation.limits.records).map_err(Error::Metadata)?;
    if object.path != target.path || object.kind != tar::Kind::File {
        return Err(Error::Invalid);
    }
    let captured = knowledge(&object);
    if archive_mode == Mode::Full && !inspected(captured) {
        return Err(Error::Invalid);
    }
    // The group mode is authoritative. A contradictory nested option must never
    // silently weaken a full request or preserve reservations in recovery.
    let expected_allocation = if options.mode == Mode::Full {
        allocation::Mode::PreserveAllocation
    } else {
        allocation::Mode::RecoverContents
    };
    if std::mem::discriminant(&options.allocation.mode)
        != std::mem::discriminant(&expected_allocation)
    {
        return Err(Error::Invalid);
    }
    let allocation = allocation::restore_bound(
        reader,
        client,
        &allocation::Target {
            ordinal: allocation_ordinal,
            path: target.path,
            object: target.object,
        },
        scratch,
        options.allocation,
        now,
        object.modified,
    )
    .map_err(Error::Allocation)?;
    let inventory = if let Some(ordinal) = inventory_ordinal {
        if allocation.next_ordinal != Some(ordinal) {
            return Err(Error::Invalid);
        }
        Some(
            if options.mode == Mode::Full {
                inventory::restore(
                    reader,
                    client,
                    &attachment::Target {
                        ordinal,
                        path: target.path,
                        object: target.object,
                    },
                    captured,
                    scratch,
                    options.inventory,
                )
            } else {
                inventory::discard(
                    reader,
                    ordinal,
                    target.path,
                    captured,
                    scratch,
                    options.inventory,
                )
            }
            .map_err(Error::Inventory)?,
        )
    } else {
        None
    };
    restore_preserved(client, target.object, target.path, &object)?;
    let actual = client.stat(target.object).map_err(Error::Destination)?;
    if actual.kind != NodeKind::File
        || actual.size != allocation.logical_bytes
        || !same_preserved(client, target.object, &actual, &object)?
    {
        return Err(Error::Invalid);
    }
    let next_ordinal = inventory
        .as_ref()
        .map_or(allocation.next_ordinal, |s| s.next_ordinal);
    let opaque = if options.mode == Mode::Full {
        OpaqueDisposition::Preserved(inventory.ok_or(Error::Invalid)?)
    } else {
        OpaqueDisposition::Omitted {
            knowledge: captured,
            transported: inventory,
        }
    };
    Ok(Report {
        mode: options.mode,
        archive_mode,
        next_ordinal,
        allocation,
        opaque,
    })
}
