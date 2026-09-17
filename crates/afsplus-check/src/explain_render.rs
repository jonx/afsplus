//! Human and machine-readable renderings of the explain records. The JSON
//! documents carry [`EXPLAIN_SCHEMA_VERSION`]; a field is added only with a
//! version step (ADR-025).
use std::fmt::Write as _;

use afsplus_format::object::ObjectType;
use afsplus_format::Timespec;

use crate::explain::{
    Allocation, BlockExplanation, BlockRole, Explainer, ObjectExplanation, PathExplanation,
    VolumeTree, EXPLAIN_SCHEMA_VERSION,
};

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn tree_name(kind: VolumeTree) -> &'static str {
    match kind {
        VolumeTree::ObjectMap => "object-map",
        VolumeTree::AllocationRoot => "allocation-root",
        VolumeTree::SharedExtents => "shared-extents",
        VolumeTree::SnapshotRegistry => "snapshot-registry",
        VolumeTree::SnapshotLifetimes => "snapshot-lifetimes",
    }
}

fn type_name(kind: ObjectType) -> &'static str {
    match kind {
        ObjectType::File => "file",
        ObjectType::Directory => "directory",
        ObjectType::Symlink => "symlink",
        ObjectType::Internal => "internal",
    }
}

fn allocation_name(allocation: Allocation) -> &'static str {
    match allocation {
        Allocation::Reserved => "reserved",
        Allocation::Allocated => "allocated",
        Allocation::Free => "free",
    }
}

/// Stable role name and its fields as (name, JSON value) pairs.
fn role_fields(role: &BlockRole) -> (&'static str, Vec<(&'static str, String)>) {
    let n = |value: u64| value.to_string();
    match *role {
        BlockRole::Identification => ("identification", vec![]),
        BlockRole::CheckpointSlot { slot, selected } => (
            "checkpoint-slot",
            vec![("slot", n(slot.into())), ("selected", selected.to_string())],
        ),
        BlockRole::RegionDescriptorSlot { region, slot, live } => (
            "region-descriptor-slot",
            vec![
                ("region", n(region.into())),
                ("slot", n(slot.into())),
                ("live", live.to_string()),
            ],
        ),
        BlockRole::BitmapSlot {
            region,
            page,
            slot,
            live,
        } => (
            "bitmap-slot",
            vec![
                ("region", n(region.into())),
                ("page", n(page.into())),
                ("slot", n(slot.into())),
                ("live", live.to_string()),
            ],
        ),
        BlockRole::AllocationRootPool => ("allocation-root-pool", vec![]),
        BlockRole::IntentLogSlot { index } => ("intent-log-slot", vec![("index", n(index.into()))]),
        BlockRole::Reserved => ("reserved", vec![]),
        BlockRole::VolumeTreeNode { kind, level } => (
            "volume-tree-node",
            vec![
                ("tree", json_string(tree_name(kind))),
                ("level", n(level.into())),
            ],
        ),
        BlockRole::ObjectRecord { object_id } => ("object-record", vec![("object", n(object_id))]),
        BlockRole::DirectoryNode { object_id, level } => (
            "directory-node",
            vec![("object", n(object_id)), ("level", n(level.into()))],
        ),
        BlockRole::ExtentNode { object_id, level } => (
            "extent-node",
            vec![("object", n(object_id)), ("level", n(level.into()))],
        ),
        BlockRole::Data {
            object_id,
            logical_block,
            shared,
            unwritten,
        } => (
            "data",
            vec![
                ("object", n(object_id)),
                ("logical_block", n(logical_block)),
                ("shared", shared.to_string()),
                ("unwritten", unwritten.to_string()),
            ],
        ),
        BlockRole::SecuritySegment { object_id, index } => (
            "security-segment",
            vec![("object", n(object_id)), ("index", n(index.into()))],
        ),
        BlockRole::AttributeSegment { object_id, index } => (
            "attribute-segment",
            vec![("object", n(object_id)), ("index", n(index.into()))],
        ),
        BlockRole::ReclaimRoot => ("reclaim-root", vec![]),
        BlockRole::ReclaimTable => ("reclaim-table", vec![]),
        BlockRole::ReclaimSegment => ("reclaim-segment", vec![]),
        BlockRole::Quarantined { retire_generation } => (
            "quarantined",
            vec![("retire_generation", n(retire_generation))],
        ),
    }
}

fn time_json(time: Timespec) -> String {
    format!(
        "{{\"seconds\":{},\"nanoseconds\":{}}}",
        time.seconds, time.nanoseconds
    )
}

fn envelope(explainer: &Explainer, kind: &str, body: &str) -> String {
    let problems: Vec<String> = explainer.problems.iter().map(|p| json_string(p)).collect();
    format!(
        "{{\"schema_version\":{EXPLAIN_SCHEMA_VERSION},\"kind\":{},\"generation\":{},\"total_blocks\":{},\"has_snapshots\":{},\"partial\":{},\"problems\":[{}],{body}}}",
        json_string(kind),
        explainer.generation,
        explainer.total_blocks,
        explainer.has_snapshots,
        !explainer.problems.is_empty(),
        problems.join(","),
    )
}

fn block_json(block: &BlockExplanation, has_snapshots: bool) -> String {
    let roles: Vec<String> = block
        .roles
        .iter()
        .map(|role| {
            let (name, fields) = role_fields(role);
            let mut out = format!("{{\"role\":{}", json_string(name));
            for (field, value) in fields {
                let _ = write!(out, ",\"{field}\":{value}");
            }
            out.push('}');
            out
        })
        .collect();
    let identity = block.identity.map_or("null".to_owned(), |identity| {
        format!(
            "{{\"magic\":{},\"owner\":{},\"generation\":{},\"checksum_valid\":{}}}",
            json_string(&String::from_utf8_lossy(&identity.magic)),
            identity.owner,
            identity.generation,
            identity.checksum_valid
        )
    });
    format!(
        "{{\"lba\":{},\"allocation\":{},\"unowned\":{},\"verdict\":{},\"roles\":[{}],\"identity\":{identity}}}",
        block.lba,
        json_string(allocation_name(block.allocation)),
        block.is_unowned(),
        json_string(block_verdict(block, has_snapshots)),
        roles.join(","),
    )
}

/// One phrase for the ownership state of a block.
fn block_verdict(block: &BlockExplanation, has_snapshots: bool) -> &'static str {
    match (block.allocation, block.roles.is_empty()) {
        (Allocation::Free, true) => "free",
        (Allocation::Free, false) => "owned-but-free",
        (Allocation::Allocated, true) if has_snapshots => "retained-or-leaked",
        (Allocation::Allocated, true) => "leaked",
        _ => "owned",
    }
}

fn object_json(object: &ObjectExplanation) -> String {
    let names: Vec<String> = object
        .names
        .iter()
        .map(|(parent, name)| format!("{{\"parent\":{parent},\"name\":{}}}", json_string(name)))
        .collect();
    let security = object.security.as_ref().map_or("null".to_owned(), |s| {
        format!(
            "{{\"total_len\":{},\"segments_expected\":{},\"segments_found\":{},\"format\":{},\"version\":{},\"projection_diverged\":{}}}",
            s.total_len,
            s.segments_expected,
            s.segments_found,
            s.format.map_or("null".to_owned(), |f| f.0.to_string()),
            s.format.map_or("null".to_owned(), |f| f.1.to_string()),
            s.projection_diverged
        )
    });
    let attributes = object.attributes.as_ref().map_or("null".to_owned(), |a| {
        let list = a.attributes.as_ref().map_or("null".to_owned(), |list| {
            let items: Vec<String> = list
                .iter()
                .map(|(name, len)| {
                    format!("{{\"name\":{},\"value_len\":{len}}}", json_string(name))
                })
                .collect();
            format!("[{}]", items.join(","))
        });
        format!(
            "{{\"total_len\":{},\"segments_expected\":{},\"segments_found\":{},\"attributes\":{list}}}",
            a.total_len, a.segments_expected, a.segments_found
        )
    });
    format!(
        "{{\"id\":{},\"type\":{},\"record_block\":{},\"link_count\":{},\"size_bytes\":{},\"allocated_bytes\":{},\"protection\":{},\"created\":{},\"modified\":{},\"changed\":{},\"content_generation\":{},\"extent_tree\":{},\"data_in_place\":{},\"comment\":{},\"symlink_target\":{},\"tree_nodes\":{},\"entries\":{},\"data\":{{\"extents\":{},\"mapped_blocks\":{},\"shared_blocks\":{},\"unwritten_blocks\":{}}},\"security\":{security},\"attributes\":{attributes},\"names\":[{}]}}",
        object.object_id,
        json_string(type_name(object.object_type)),
        object.record_block,
        object.link_count,
        object.size_bytes,
        object.allocated_bytes,
        object.protection,
        time_json(object.created),
        time_json(object.modified),
        time_json(object.changed),
        object.content_generation,
        object.extent_tree,
        object.data_in_place,
        json_string(&object.comment),
        object
            .symlink_target
            .as_deref()
            .map_or("null".to_owned(), json_string),
        object.tree_nodes,
        object.entries,
        object.data.extents,
        object.data.mapped_blocks,
        object.data.shared_blocks,
        object.data.unwritten_blocks,
        names.join(","),
    )
}

pub fn render_block_json(explainer: &Explainer, block: &BlockExplanation) -> String {
    envelope(
        explainer,
        "block",
        &format!("\"block\":{}", block_json(block, explainer.has_snapshots)),
    )
}

pub fn render_object_json(explainer: &Explainer, object: &ObjectExplanation) -> String {
    envelope(
        explainer,
        "object",
        &format!("\"object\":{}", object_json(object)),
    )
}

pub fn render_path_json(explainer: &Explainer, path: &PathExplanation) -> String {
    let components: Vec<String> = path
        .components
        .iter()
        .map(|(name, id)| format!("{{\"name\":{},\"object\":{id}}}", json_string(name)))
        .collect();
    envelope(
        explainer,
        "path",
        &format!(
            "\"components\":[{}],\"object\":{}",
            components.join(","),
            object_json(&path.object)
        ),
    )
}

fn problems_human(explainer: &Explainer, out: &mut String) {
    for problem in &explainer.problems {
        let _ = writeln!(out, "problem: {problem}");
    }
}

pub fn render_block_human(explainer: &Explainer, block: &BlockExplanation) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "block {} of {} at generation {}: {}, {}",
        block.lba,
        explainer.total_blocks,
        explainer.generation,
        allocation_name(block.allocation),
        block_verdict(block, explainer.has_snapshots)
    );
    for role in &block.roles {
        let (name, fields) = role_fields(role);
        let fields: Vec<String> = fields
            .into_iter()
            .map(|(field, value)| format!("{field}={}", value.trim_matches('"')))
            .collect();
        let _ = writeln!(out, "  role: {name} {}", fields.join(" "));
    }
    if block.is_unowned() && explainer.has_snapshots {
        let _ = writeln!(
            out,
            "  no live role: a retained snapshot may own this block; the checker decides"
        );
    }
    if let Some(identity) = block.identity {
        let _ = writeln!(
            out,
            "  header: magic {} owner {} generation {} checksum {}",
            String::from_utf8_lossy(&identity.magic),
            identity.owner,
            identity.generation,
            if identity.checksum_valid {
                "valid"
            } else {
                "INVALID"
            }
        );
    }
    problems_human(explainer, &mut out);
    out
}

fn object_human(object: &ObjectExplanation, out: &mut String) {
    let _ = writeln!(
        out,
        "object {}: {}, record at block {}, {} link(s)",
        object.object_id,
        type_name(object.object_type),
        object.record_block,
        object.link_count
    );
    let _ = writeln!(
        out,
        "  size {} bytes, allocated {} bytes, protection {:#010x}, content generation {}",
        object.size_bytes, object.allocated_bytes, object.protection, object.content_generation
    );
    for (label, time) in [
        ("created", object.created),
        ("modified", object.modified),
        ("changed", object.changed),
    ] {
        let _ = writeln!(out, "  {label} {}.{:09}", time.seconds, time.nanoseconds);
    }
    match object.object_type {
        ObjectType::Directory => {
            let _ = writeln!(
                out,
                "  directory: {} entries in {} tree node(s)",
                object.entries, object.tree_nodes
            );
        }
        ObjectType::File => {
            let _ = writeln!(
                out,
                "  data: {} extent(s), {} mapped, {} shared, {} unwritten block(s), {} extent tree node(s){}{}",
                object.data.extents,
                object.data.mapped_blocks,
                object.data.shared_blocks,
                object.data.unwritten_blocks,
                object.tree_nodes,
                if object.extent_tree { "" } else { ", direct extent" },
                if object.data_in_place { ", data in place" } else { "" }
            );
        }
        _ => {}
    }
    if let Some(target) = &object.symlink_target {
        let _ = writeln!(out, "  symlink target: {target}");
    }
    if !object.comment.is_empty() {
        let _ = writeln!(out, "  comment: {}", object.comment);
    }
    if let Some(s) = &object.security {
        let format = s.format.map_or("unknown".to_owned(), |(format, version)| {
            format!("{format:#010x} version {version}")
        });
        let _ = writeln!(
            out,
            "  security descriptor: {} bytes, {} of {} segment(s) proven, format {format}{}",
            s.total_len,
            s.segments_found,
            s.segments_expected,
            if s.projection_diverged {
                ", projection diverged"
            } else {
                ""
            }
        );
    }
    if let Some(a) = &object.attributes {
        let _ = writeln!(
            out,
            "  attributes: {} bytes, {} of {} segment(s) proven",
            a.total_len, a.segments_found, a.segments_expected
        );
        match &a.attributes {
            Some(list) => {
                for (name, len) in list {
                    let _ = writeln!(out, "    {name} ({len} bytes)");
                }
            }
            None => {
                let _ = writeln!(out, "    the set is unreadable");
            }
        }
    }
    if object.names.is_empty() {
        let _ = writeln!(out, "  named by no directory entry");
    }
    for (parent, name) in &object.names {
        let _ = writeln!(out, "  named {name:?} in directory {parent}");
    }
}

pub fn render_object_human(explainer: &Explainer, object: &ObjectExplanation) -> String {
    let mut out = format!("generation {}\n", explainer.generation);
    object_human(object, &mut out);
    problems_human(explainer, &mut out);
    out
}

pub fn render_path_human(explainer: &Explainer, path: &PathExplanation) -> String {
    let mut out = format!("generation {}\n", explainer.generation);
    for (name, id) in &path.components {
        let _ = writeln!(out, "{name:?} -> object {id}");
    }
    object_human(&path.object, &mut out);
    problems_human(explainer, &mut out);
    out
}
