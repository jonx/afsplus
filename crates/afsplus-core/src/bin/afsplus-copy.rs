//! Copy every live object of one AFS+ image into a freshly formatted one.
//!
//! The destination's format is whatever `mkafsplus` gave it, so this is how a
//! volume gains a format-time feature such as persistent snapshots: names,
//! bytes, holes, hard links, symlinks, comments, attributes, security
//! descriptors, protection, owners and times all carry over. Snapshots and
//! private restore stages do not: a source that holds either is refused.
//! A second pass compares the two trees and fails on any difference.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::volume::{
    AttributeWriteMode, DirectoryCursor, PreservedMetadata, SnapshotWorkLimits,
};
use afsplus_core::{
    mount_with_options, mount_with_snapshot_limits, MountMode, MountOptions, Volume,
};
use afsplus_format::ident::{Identification, INCOMPAT_PERSISTENT_SNAPSHOTS};
use afsplus_format::object::ObjectType;
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

const CHUNK: usize = 1 << 20;
const STAGE_ATTRIBUTE: &str = "system.restore.stage";

/// Comment, attributes and security descriptor (format, version, bytes).
type Extras = (String, Vec<(String, Vec<u8>)>, Option<(u32, u16, Vec<u8>)>);

#[derive(Default)]
struct Stats {
    directories: u64,
    files: u64,
    symlinks: u64,
    hard_links: u64,
    bytes: u64,
}

fn open(path: &Path, writable: bool) -> Result<Volume<FileBackend>, String> {
    let mut device = FileBackend::open_sized_by_file(path, DEFAULT_BLOCK_SIZE)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut block = vec![0; DEFAULT_BLOCK_SIZE];
    device
        .read_block(0, &mut block)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let ident = Identification::decode(&block)
        .map_err(|e| format!("cannot decode {}: {e}", path.display()))?;
    device.set_total_blocks(ident.total_blocks);
    let options = MountOptions {
        mode: if writable {
            MountMode::ReadWrite
        } else {
            MountMode::NoChanges
        },
        ..Default::default()
    };
    let mounted = if ident.features.incompat & INCOMPAT_PERSISTENT_SNAPSHOTS != 0 {
        // The destination is fresh; the source is only read, so any number
        // of registered views is admitted and refused below.
        let limits = SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: if writable { 8 } else { usize::MAX },
            reclaim_records: 64,
        };
        mount_with_snapshot_limits(device, options, limits)
    } else {
        mount_with_options(device, options)
    };
    mounted.map_err(|e| format!("cannot mount {}: {e}", path.display()))
}

fn entries(volume: &mut Volume<FileBackend>, directory: u64) -> Result<Vec<(String, u64)>, String> {
    let mut out = Vec::new();
    let mut cursor: Option<DirectoryCursor> = None;
    loop {
        let page = volume
            .read_directory_page(directory, cursor, 64)
            .map_err(|e| format!("cannot list directory {directory}: {e}"))?;
        for entry in page.entries {
            let name = String::from_utf8(entry.name)
                .map_err(|_| format!("non-UTF-8 name in directory {directory}"))?;
            out.push((name, entry.child_id));
        }
        if page.eof {
            return Ok(out);
        }
        cursor = Some(page.next);
    }
}

fn metadata(
    volume: &mut Volume<FileBackend>,
    id: u64,
) -> Result<afsplus_core::volume::ObjectMetadata, String> {
    volume
        .visible_metadata(id)
        .map_err(|e| format!("cannot stat object {id}: {e}"))?
        .ok_or_else(|| format!("object {id} vanished"))
}

/// Comment, attributes and security descriptor: everything but the record.
fn extras(src: &mut Volume<FileBackend>, id: u64) -> Result<Extras, String> {
    let comment = src
        .object_comment(id)
        .map_err(|e| format!("comment of {id}: {e}"))?;
    let mut attributes = Vec::new();
    for name in src
        .attribute_names(id)
        .map_err(|e| format!("attributes of {id}: {e}"))?
    {
        if name == STAGE_ATTRIBUTE {
            return Err(format!(
                "object {id} is part of a restore stage; reclaim it first"
            ));
        }
        let value = src
            .attribute(id, &name)
            .map_err(|e| format!("attribute {name} of {id}: {e}"))?
            .ok_or_else(|| format!("attribute {name} of {id} vanished"))?;
        attributes.push((name, value));
    }
    let security = src
        .security_descriptor(id)
        .map_err(|e| format!("security descriptor of {id}: {e}"))?
        .map(|d| (d.format, d.version, d.bytes));
    Ok((comment, attributes, security))
}

fn copy_extras(
    src: &mut Volume<FileBackend>,
    dst: &mut Volume<FileBackend>,
    from: u64,
    to: u64,
) -> Result<(), String> {
    let (comment, attributes, security) = extras(src, from)?;
    let now = Timespec::default();
    if !comment.is_empty() {
        dst.set_object_comment(to, &comment, now)
            .map_err(|e| format!("set comment on {to}: {e}"))?;
    }
    if !attributes.is_empty() {
        let changes: Vec<_> = attributes
            .iter()
            .map(|(n, v)| (n.as_str(), Some(v.as_slice())))
            .collect();
        dst.set_attributes(to, &changes, AttributeWriteMode::Create, now)
            .map_err(|e| format!("set attributes on {to}: {e}"))?;
    }
    if let Some((format, version, bytes)) = security {
        dst.set_security_descriptor(to, format, version, &bytes, now)
            .map_err(|e| format!("set security descriptor on {to}: {e}"))?;
    }
    Ok(())
}

fn copy_file(
    src: &mut Volume<FileBackend>,
    dst: &mut Volume<FileBackend>,
    from: u64,
    to: u64,
    size: u64,
) -> Result<(), String> {
    let mut buffer = vec![0u8; CHUNK];
    let mut offset = 0u64;
    while offset < size {
        let want = usize::try_from((size - offset).min(CHUNK as u64)).expect("chunk fits");
        let got = src
            .read_file_at(from, offset, &mut buffer[..want])
            .map_err(|e| format!("read object {from} at {offset}: {e}"))?;
        if got != want {
            return Err(format!("short read of object {from} at {offset}"));
        }
        // Only runs of non-zero blocks are written: a zero block stays a
        // hole, and the final size is set below.
        let block = DEFAULT_BLOCK_SIZE;
        let mut at = 0;
        while at < want {
            let zero = |start: usize| {
                buffer[start..(start + block).min(want)]
                    .iter()
                    .all(|&b| b == 0)
            };
            if zero(at) {
                at += block;
                continue;
            }
            let mut end = at + block;
            while end < want && !zero(end) {
                end += block;
            }
            let end = end.min(want);
            dst.write_file_at(
                to,
                offset + at as u64,
                &buffer[at..end],
                Timespec::default(),
            )
            .map_err(|e| format!("write object {to} at {}: {e}", offset + at as u64))?;
            at = end;
        }
        offset += want as u64;
    }
    dst.truncate_file(to, size, Timespec::default())
        .map_err(|e| format!("size object {to}: {e}"))?;
    Ok(())
}

fn read_link(volume: &mut Volume<FileBackend>, id: u64) -> Result<String, String> {
    let mut buffer = vec![0u8; 4096];
    let n = volume
        .read_link(id, &mut buffer)
        .map_err(|e| format!("read link {id}: {e}"))?;
    String::from_utf8(buffer[..n].to_vec()).map_err(|_| format!("non-UTF-8 link {id}"))
}

/// Copies the children of one directory; directory records are restored
/// after their children, which change a directory's times.
fn copy_directory(
    src: &mut Volume<FileBackend>,
    dst: &mut Volume<FileBackend>,
    from: u64,
    to: u64,
    links: &mut HashMap<u64, u64>,
    stats: &mut Stats,
) -> Result<(), String> {
    let now = Timespec::default();
    for (name, child) in entries(src, from)? {
        let meta = metadata(src, child)?;
        let created = match meta.object_type {
            ObjectType::Directory => {
                let id = dst
                    .create_directory(to, &name, now)
                    .map_err(|e| format!("mkdir {name:?}: {e}"))?;
                copy_directory(src, dst, child, id, links, stats)?;
                stats.directories += 1;
                id
            }
            ObjectType::File => {
                if let Some(&id) = links.get(&child) {
                    dst.link_file(id, to, &name, now)
                        .map_err(|e| format!("link {name:?}: {e}"))?;
                    stats.hard_links += 1;
                    continue;
                }
                let id = dst
                    .create_file_in_directory(to, &name, &[], now)
                    .map_err(|e| format!("create {name:?}: {e}"))?;
                copy_file(src, dst, child, id, meta.size_bytes)?;
                if meta.link_count > 1 {
                    links.insert(child, id);
                }
                stats.files += 1;
                stats.bytes += meta.size_bytes;
                id
            }
            ObjectType::Symlink => {
                let target = read_link(src, child)?;
                let id = dst
                    .create_symlink(to, &name, &target, now)
                    .map_err(|e| format!("symlink {name:?}: {e}"))?;
                stats.symlinks += 1;
                id
            }
            other => return Err(format!("{name:?}: unsupported object type {other:?}")),
        };
        copy_extras(src, dst, child, created)?;
        dst.restore_object_metadata(created, PreservedMetadata::from(meta))
            .map_err(|e| format!("metadata of {name:?}: {e}"))?;
    }
    Ok(())
}

/// The second pass: every name, type, byte and metadata field must match.
fn compare(
    src: &mut Volume<FileBackend>,
    dst: &mut Volume<FileBackend>,
    a: u64,
    b: u64,
    path: &str,
) -> Result<u64, String> {
    let (ma, mb) = (metadata(src, a)?, metadata(dst, b)?);
    if ma.object_type != mb.object_type
        || ma.size_bytes != mb.size_bytes
        || PreservedMetadata::from(ma) != PreservedMetadata::from(mb)
    {
        return Err(format!("{path}: record differs"));
    }
    if extras(src, a)? != extras(dst, b)? {
        return Err(format!(
            "{path}: comment, attributes or security descriptor differ"
        ));
    }
    let mut checked = 1;
    match ma.object_type {
        ObjectType::Directory => {
            let (ea, eb) = (entries(src, a)?, entries(dst, b)?);
            if ea.iter().map(|e| &e.0).ne(eb.iter().map(|e| &e.0)) {
                return Err(format!("{path}: names differ"));
            }
            for ((name, ca), (_, cb)) in ea.into_iter().zip(eb) {
                checked += compare(src, dst, ca, cb, &format!("{path}/{name}"))?;
            }
        }
        ObjectType::File => {
            let (mut ba, mut bb) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
            let mut offset = 0;
            while offset < ma.size_bytes {
                let want = usize::try_from((ma.size_bytes - offset).min(CHUNK as u64))
                    .expect("chunk fits");
                let ra = src
                    .read_file_at(a, offset, &mut ba[..want])
                    .map_err(|e| format!("{path}: {e}"))?;
                let rb = dst
                    .read_file_at(b, offset, &mut bb[..want])
                    .map_err(|e| format!("{path}: {e}"))?;
                if ra != want || rb != want || ba[..want] != bb[..want] {
                    return Err(format!("{path}: bytes differ at {offset}"));
                }
                offset += want as u64;
            }
        }
        ObjectType::Symlink if read_link(src, a)? != read_link(dst, b)? => {
            return Err(format!("{path}: link target differs"));
        }
        _ => {}
    }
    Ok(checked)
}

fn run(source: &Path, destination: &Path) -> Result<(), String> {
    let mut src = open(source, false)?;
    if src
        .snapshot_list(0, 1)
        .map(|page| !page.entries.is_empty())
        .unwrap_or(false)
    {
        return Err("the source holds snapshots, which a copy cannot carry".to_owned());
    }
    let mut dst = open(destination, true)?;
    if !entries(&mut dst, OBJECT_ROOT)?.is_empty() {
        return Err("refusing to copy into a non-empty volume".to_owned());
    }
    let mut stats = Stats::default();
    let mut links = HashMap::new();
    copy_directory(
        &mut src,
        &mut dst,
        OBJECT_ROOT,
        OBJECT_ROOT,
        &mut links,
        &mut stats,
    )?;
    // Each further name of a hard-linked file changed its record again.
    for (&from, &to) in &links {
        let meta = metadata(&mut src, from)?;
        dst.restore_object_metadata(to, PreservedMetadata::from(meta))
            .map_err(|e| format!("metadata of linked object {to}: {e}"))?;
    }
    copy_extras(&mut src, &mut dst, OBJECT_ROOT, OBJECT_ROOT)?;
    let root = metadata(&mut src, OBJECT_ROOT)?;
    dst.restore_object_metadata(OBJECT_ROOT, PreservedMetadata::from(root))
        .map_err(|e| format!("root metadata: {e}"))?;
    dst.sync()
        .map_err(|e| format!("cannot commit {}: {e}", destination.display()))?;
    drop(dst);
    let mut dst = open(destination, false)?;
    let checked = compare(&mut src, &mut dst, OBJECT_ROOT, OBJECT_ROOT, "")?;
    println!(
        "copied {} into {}: {} directories, {} files, {} symlinks, {} hard links, {} bytes; {} objects compared identical",
        source.display(),
        destination.display(),
        stats.directories,
        stats.files,
        stats.symlinks,
        stats.hard_links,
        stats.bytes,
        checked
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: afsplus-copy <source-image> <freshly-formatted-destination-image>");
        return ExitCode::from(2);
    }
    match run(Path::new(&args[0]), Path::new(&args[1])) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("afsplus-copy: {message}");
            ExitCode::FAILURE
        }
    }
}
