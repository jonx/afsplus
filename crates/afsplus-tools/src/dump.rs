use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;

use afsplus_core::extent_map::{self, Extent};
use afsplus_core::intent_log;
use afsplus_core::mount::SUPPORTED_INCOMPAT_FEATURES;
use afsplus_core::verify::{full_sweep, load_committed_state, CommittedState};
use afsplus_core::volume::emergency_headroom_for_volume;
use afsplus_format::ident::INCOMPAT_INTENT_LOG_DATA_UPDATES;
use afsplus_format::intent_log::{LogOp, LogRecord};
use afsplus_format::object::{ObjectType, OBJECT_FLAG_EXTENT_TREE};
use afsplus_format::{Timespec, OBJECT_ORPHAN_DIRECTORY};

use crate::common::{
    core_failure, header_json, hex, json_string, read_header, report_failure, Failure, HeaderView,
    ReadOnlyImage, EXIT_MEDIA, EXIT_OK,
};

const TOOL: &str = "afsplus-dump";
const USAGE: &str = "usage: afsplus-dump [--json] <image>";

struct Options {
    image: PathBuf,
    json: bool,
}

enum ParseResult {
    Options(Options),
    Help,
}

struct DumpResult {
    output: String,
    findings: usize,
}

pub fn run<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    let options = match parse(args) {
        Ok(ParseResult::Options(options)) => options,
        Ok(ParseResult::Help) => {
            println!("{USAGE}");
            println!("Exhaustively decode committed metadata without writing to the image.");
            return EXIT_OK;
        }
        Err(failure) => return report_failure(TOOL, failure),
    };
    match execute(&options) {
        Ok(result) => {
            println!("{}", result.output);
            if result.findings == 0 {
                EXIT_OK
            } else {
                eprintln!(
                    "{TOOL}: error[E_INVARIANT]: {} committed-state invariant finding(s); details are in the dump",
                    result.findings
                );
                EXIT_MEDIA
            }
        }
        Err(failure) => report_failure(TOOL, failure),
    }
}

fn parse<I>(args: I) -> Result<ParseResult, Failure>
where
    I: IntoIterator<Item = OsString>,
{
    let mut image = None;
    let mut json = false;
    let mut positional_only = false;
    for argument in args {
        if !positional_only {
            match argument.to_str() {
                Some("--help" | "-h") => return Ok(ParseResult::Help),
                Some("--json") => {
                    if json {
                        return Err(Failure::usage("--json was specified more than once"));
                    }
                    json = true;
                    continue;
                }
                Some("--") => {
                    positional_only = true;
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(Failure::usage(format!("unknown option {value:?}; {USAGE}")));
                }
                _ => {}
            }
        }
        if image.replace(PathBuf::from(&argument)).is_some() {
            return Err(Failure::usage(format!(
                "exactly one image path is required; {USAGE}"
            )));
        }
    }
    let image = image.ok_or_else(|| Failure::usage(format!("missing image path; {USAGE}")))?;
    Ok(ParseResult::Options(Options { image, json }))
}

fn execute(options: &Options) -> Result<DumpResult, Failure> {
    let (mut device, view) = read_header(&options.image)?;
    let unknown_incompat = view.ident.features.incompat & !SUPPORTED_INCOMPAT_FEATURES;
    if unknown_incompat != 0 {
        return Err(Failure::media(
            "E_FEATURE",
            format!("cannot decode state with unknown INCOMPAT bits {unknown_incompat:#018x}"),
        ));
    }
    let state = load_committed_state(&mut device, &view.ident, &view.selection.chosen).map_err(
        |error| {
            core_failure(
                "E_STATE",
                &format!(
                    "checkpoint generation {} references invalid state",
                    view.selection.chosen.generation
                ),
                error,
            )
        },
    )?;
    let mut findings = full_sweep(&state, &view.ident.geometry(), &view.selection.chosen);
    let scanned = intent_log::scan(
        &mut device,
        &view.ident.geometry(),
        view.ident.log_slots,
        &view.ident.uuid,
        view.selection.chosen.generation,
        view.ident.features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0,
    )
    .map_err(|error| core_failure("E_INTENT_LOG", "cannot scan intent log", error))?;
    for record in &scanned.records {
        for operation in &record.ops {
            for (start, blocks) in operation.data_extents() {
                for lba in *start..*start + u64::from(*blocks) {
                    if state.bitmaps.is_allocated(lba) {
                        findings.push(format!(
                            "intent-log record {} references committed allocated block {lba}",
                            record.sequence
                        ));
                    }
                }
            }
        }
    }
    let extent_maps = collect_extents(&mut device, &view, &state)?;
    let output = if options.json {
        render_json(
            &view,
            &state,
            &extent_maps,
            &scanned.records,
            scanned.tail_note.as_deref(),
            &findings,
        )
    } else {
        render_text(
            &view,
            &state,
            &extent_maps,
            &scanned.records,
            scanned.tail_note.as_deref(),
            &findings,
        )
    };
    Ok(DumpResult {
        output,
        findings: findings.len(),
    })
}

fn collect_extents(
    device: &mut ReadOnlyImage,
    view: &HeaderView,
    state: &CommittedState,
) -> Result<std::collections::BTreeMap<u64, Vec<Extent>>, Failure> {
    let mut maps = std::collections::BTreeMap::new();
    for (&object_id, record) in &state.objects {
        if record.object_type != ObjectType::File || record.data_blocks == 0 {
            continue;
        }
        let extents = if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            extent_map::load_all(
                device,
                &view.ident.geometry(),
                record.data_root,
                object_id,
                view.selection.chosen.generation,
            )
            .map_err(|error| {
                core_failure(
                    "E_EXTENT_MAP",
                    &format!("cannot decode extent map for object {object_id}"),
                    error,
                )
            })?
            .extents
        } else {
            vec![Extent {
                logical_start: 0,
                physical_start: record.data_root,
                block_count: record.data_blocks,
                flags: 0,
            }]
        };
        maps.insert(object_id, extents);
    }
    Ok(maps)
}

fn render_json(
    view: &HeaderView,
    state: &CommittedState,
    extent_maps: &std::collections::BTreeMap<u64, Vec<Extent>>,
    log_records: &[LogRecord],
    log_tail: Option<&str>,
    findings: &[String],
) -> String {
    let mut output = format!(
        "{{\"schema_version\":1,\"tool\":\"afsplus-dump\",{},\"consistent\":{},\"counts\":{{\"objects\":{},\"directories\":{},\"directory_entries\":{},\"orphan_entries\":{},\"allocation_regions\":{},\"metadata_blocks\":{},\"data_blocks\":{},\"reclaim_runs\":{},\"reclaim_pending_blocks\":{},\"shared_runs\":{},\"intent_records\":{}}},",
        header_json(view),
        findings.is_empty(),
        state.objects.len(),
        state.directories.len(),
        state
            .directories
            .values()
            .map(|directory| directory.entries.len())
            .sum::<usize>(),
        state
            .directories
            .get(&OBJECT_ORPHAN_DIRECTORY)
            .map_or(0, |directory| directory.entries.len()),
        state.allocation_records.len(),
        state.metadata_blocks.len(),
        state.data_blocks.len(),
        state.reclaim_runs.len(),
        state.reclaim_pending_blocks,
        state.shared_records.len(),
        log_records.len(),
    );

    output.push_str("\"block_sets\":{");
    write!(
        output,
        "\"metadata\":{},\"data\":{},\"allocation_pool\":{},\"intent_log_area\":{},\"shared_tree\":{}",
        sorted_u64_json(&state.metadata_blocks),
        sorted_u64_json(&state.data_blocks),
        sorted_u64_json(&state.allocation_pool_blocks),
        sorted_u64_json(&state.log_area_blocks),
        sorted_u64_json(&state.shared_tree_blocks),
    )
    .expect("writing to String cannot fail");
    output.push_str("},\"allocation_regions\":[");
    for (region, record) in state.allocation_records.iter().enumerate() {
        if region != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"region\":{},\"descriptor_slot\":{},\"descriptor_generation\":{},\"free_blocks\":{},\"bitmap_pages\":[",
            region, record.descriptor_slot, record.descriptor_generation, record.free_blocks,
        )
        .expect("writing to String cannot fail");
        for (page_index, page) in state.bitmaps.pages[region].iter().enumerate() {
            if page_index != 0 {
                output.push(',');
            }
            let free_blocks = page.free_blocks();
            write!(
                output,
                "{{\"page\":{},\"first_block\":{},\"valid_blocks\":{},\"allocated_blocks\":{},\"free_blocks\":{}}}",
                page.page_index,
                page.first_block,
                page.valid_blocks,
                page.valid_blocks - free_blocks,
                free_blocks,
            )
            .expect("writing to String cannot fail");
        }
        output.push_str("]}");
    }
    output.push_str("],\"objects\":[");
    for (index, (&object_id, record)) in state.objects.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let record_lba = state.object_map.lookup(object_id).unwrap_or(0);
        let internal_role = if object_id == OBJECT_ORPHAN_DIRECTORY {
            json_string("orphan-directory")
        } else {
            "null".into()
        };
        write!(
            output,
            "{{\"object_id\":{},\"record_lba\":{},\"type\":{},\"internal_role\":{},\"flags\":\"{:#06x}\",\"link_count\":{},\"size_bytes\":{},\"allocated_bytes\":{},\"protection\":\"{:#010x}\",\"content_generation\":{},\"data_root\":{},\"data_blocks\":{},\"created\":{},\"modified\":{},\"changed\":{},\"extents\":{}}}",
            object_id,
            record_lba,
            json_string(object_type_name(record.object_type)),
            internal_role,
            record.flags,
            record.link_count,
            record.size_bytes,
            record.allocated_bytes,
            record.protection,
            record.content_generation,
            record.data_root,
            record.data_blocks,
            timestamp_json(record.created),
            timestamp_json(record.modified),
            timestamp_json(record.changed),
            extents_json(extent_maps.get(&object_id).map(Vec::as_slice).unwrap_or(&[])),
        )
        .expect("writing to String cannot fail");
    }
    output.push_str("],\"directories\":[");
    for (directory_index, (&owner, directory)) in state.directories.iter().enumerate() {
        if directory_index != 0 {
            output.push(',');
        }
        let role = if owner == OBJECT_ORPHAN_DIRECTORY {
            json_string("orphan-directory")
        } else {
            "null".into()
        };
        write!(
            output,
            "{{\"owner\":{owner},\"internal_role\":{role},\"entries\":["
        )
        .expect("writing to String cannot fail");
        for (entry_index, entry) in directory.entries.iter().enumerate() {
            if entry_index != 0 {
                output.push(',');
            }
            write!(
                output,
                "{{\"name\":{},\"key_hex\":{},\"child_id\":{},\"child_type_hint\":{}}}",
                json_string(&String::from_utf8_lossy(&entry.name)),
                json_string(&hex(&entry.key)),
                entry.child_id,
                entry.child_type_hint,
            )
            .expect("writing to String cannot fail");
        }
        output.push_str("]}");
    }
    output.push_str("],\"reclaim_runs\":[");
    for (index, run) in state.reclaim_runs.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"start\":{},\"blocks\":{},\"retire_generation\":{}}}",
            run.start, run.blocks, run.retire_generation
        )
        .expect("writing to String cannot fail");
    }
    output.push_str("],\"shared_runs\":[");
    for (index, run) in state.shared_records.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"physical_start\":{},\"blocks\":{},\"reference_count\":{},\"flags\":\"{:#010x}\"}}",
            run.physical_start, run.block_count, run.reference_count, run.flags
        )
        .expect("writing to String cannot fail");
    }
    output.push_str("],\"intent_log\":{");
    write!(
        output,
        "\"tail_note\":{},\"records\":[",
        log_tail.map_or_else(|| "null".to_owned(), json_string)
    )
    .expect("writing to String cannot fail");
    for (index, record) in log_records.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&log_record_json(record));
    }
    output.push_str("]},\"findings\":[");
    for (index, finding) in findings.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&json_string(finding));
    }
    output.push_str("]}");
    output
}

fn render_text(
    view: &HeaderView,
    state: &CommittedState,
    extent_maps: &std::collections::BTreeMap<u64, Vec<Extent>>,
    log_records: &[LogRecord],
    log_tail: Option<&str>,
    findings: &[String],
) -> String {
    let emergency_headroom = emergency_headroom_for_volume(view.ident.total_blocks);
    let available_blocks = view
        .selection
        .chosen
        .free_blocks_total
        .saturating_sub(emergency_headroom);
    let mut output = format!(
        "AFS+ metadata dump schema 1\nvolume {} label {:?}\ncheckpoint slot {} generation {} transaction {}\nobjects {} directories {} metadata blocks {} data blocks {} raw free {} emergency headroom {} normally available {}\n",
        crate::common::uuid_hex(&view.ident.uuid),
        view.ident.label,
        if view.selection.chosen_slot == 0 { "A" } else { "B" },
        view.selection.chosen.generation,
        view.selection.chosen.committed_tx_id,
        state.objects.len(),
        state.directories.len(),
        state.metadata_blocks.len(),
        state.data_blocks.len(),
        view.selection.chosen.free_blocks_total,
        emergency_headroom,
        available_blocks,
    );
    for (&object_id, record) in &state.objects {
        let role = if object_id == OBJECT_ORPHAN_DIRECTORY {
            " [internal orphan-directory]"
        } else {
            ""
        };
        writeln!(
            output,
            "object {object_id}{role} @{} {} flags {:#06x} links {} size {} allocated {} data {}/{} extents {}",
            state.object_map.lookup(object_id).unwrap_or(0),
            object_type_name(record.object_type),
            record.flags,
            record.link_count,
            record.size_bytes,
            record.allocated_bytes,
            record.data_root,
            record.data_blocks,
            extent_maps.get(&object_id).map_or(0, Vec::len),
        )
        .expect("writing to String cannot fail");
    }
    for (&owner, directory) in &state.directories {
        let role = if owner == OBJECT_ORPHAN_DIRECTORY {
            " [internal orphan-directory]"
        } else {
            ""
        };
        writeln!(
            output,
            "directory {owner}{role} entries {}",
            directory.entries.len()
        )
        .expect("writing to String cannot fail");
        for entry in &directory.entries {
            writeln!(
                output,
                "  {:?} -> {} type-hint {} key {}",
                String::from_utf8_lossy(&entry.name),
                entry.child_id,
                entry.child_type_hint,
                hex(&entry.key),
            )
            .expect("writing to String cannot fail");
        }
    }
    writeln!(
        output,
        "reclaim: {} runs / {} blocks; shared: {} runs; intent: {} records",
        state.reclaim_runs.len(),
        state.reclaim_pending_blocks,
        state.shared_records.len(),
        log_records.len(),
    )
    .expect("writing to String cannot fail");
    if let Some(note) = log_tail {
        writeln!(output, "intent tail: {note}").expect("writing to String cannot fail");
    }
    for finding in findings {
        writeln!(output, "finding: {finding}").expect("writing to String cannot fail");
    }
    output.push_str(if findings.is_empty() {
        "consistent"
    } else {
        "INCONSISTENT"
    });
    output
}

fn object_type_name(object_type: ObjectType) -> &'static str {
    match object_type {
        ObjectType::File => "file",
        ObjectType::Directory => "directory",
        ObjectType::Symlink => "symlink",
        ObjectType::Internal => "internal",
    }
}

fn timestamp_json(timestamp: Timespec) -> String {
    format!(
        "{{\"seconds\":{},\"nanoseconds\":{}}}",
        timestamp.seconds, timestamp.nanoseconds
    )
}

fn extents_json(extents: &[Extent]) -> String {
    let mut output = String::from("[");
    for (index, extent) in extents.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"logical_start\":{},\"physical_start\":{},\"blocks\":{},\"flags\":\"{:#010x}\"}}",
            extent.logical_start, extent.physical_start, extent.block_count, extent.flags
        )
        .expect("writing to String cannot fail");
    }
    output.push(']');
    output
}

fn log_record_json(record: &LogRecord) -> String {
    let mut output = format!(
        "{{\"base_generation\":{},\"sequence\":{},\"ops\":[",
        record.base_generation, record.sequence
    );
    for (index, operation) in record.ops.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&log_op_json(operation));
    }
    output.push_str("]}");
    output
}

fn log_op_json(operation: &LogOp) -> String {
    match operation {
        LogOp::Create {
            parent_id,
            name,
            expected_object_id,
            size_bytes,
            content_crc,
            timestamp,
            extents,
        } => format!(
            "{{\"op\":\"create\",\"parent_id\":{},\"name\":{},\"expected_object_id\":{},\"size_bytes\":{},\"content_crc\":\"{:#010x}\",\"timestamp\":{},\"extents\":{}}}",
            parent_id,
            json_string(&String::from_utf8_lossy(name)),
            expected_object_id,
            size_bytes,
            content_crc,
            timestamp_json(*timestamp),
            physical_extents_json(extents),
        ),
        LogOp::Delete {
            parent_id,
            name,
            timestamp,
        } => format!(
            "{{\"op\":\"delete\",\"parent_id\":{},\"name\":{},\"timestamp\":{}}}",
            parent_id,
            json_string(&String::from_utf8_lossy(name)),
            timestamp_json(*timestamp),
        ),
        LogOp::Rename {
            source_parent_id,
            source_name,
            target_parent_id,
            target_name,
            replace,
            timestamp,
        } => format!(
            "{{\"op\":\"rename\",\"source_parent_id\":{},\"source_name\":{},\"target_parent_id\":{},\"target_name\":{},\"replace\":{},\"timestamp\":{}}}",
            source_parent_id,
            json_string(&String::from_utf8_lossy(source_name)),
            target_parent_id,
            json_string(&String::from_utf8_lossy(target_name)),
            replace,
            timestamp_json(*timestamp),
        ),
        LogOp::Write {
            object_id,
            logical_start,
            expected_size_bytes,
            new_size_bytes,
            content_crc,
            timestamp,
            extents,
        } => update_op_json(
            "write",
            *object_id,
            *logical_start,
            *expected_size_bytes,
            *new_size_bytes,
            *content_crc,
            *timestamp,
            extents,
        ),
        LogOp::Truncate {
            object_id,
            logical_start,
            expected_size_bytes,
            new_size_bytes,
            content_crc,
            timestamp,
            extents,
        } => update_op_json(
            "truncate",
            *object_id,
            *logical_start,
            *expected_size_bytes,
            *new_size_bytes,
            *content_crc,
            *timestamp,
            extents,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn update_op_json(
    name: &str,
    object_id: u64,
    logical_start: u64,
    expected_size_bytes: u64,
    new_size_bytes: u64,
    content_crc: u32,
    timestamp: Timespec,
    extents: &[(u64, u32)],
) -> String {
    format!(
        "{{\"op\":{},\"object_id\":{},\"logical_start\":{},\"expected_size_bytes\":{},\"new_size_bytes\":{},\"content_crc\":\"{:#010x}\",\"timestamp\":{},\"extents\":{}}}",
        json_string(name),
        object_id,
        logical_start,
        expected_size_bytes,
        new_size_bytes,
        content_crc,
        timestamp_json(timestamp),
        physical_extents_json(extents),
    )
}

fn physical_extents_json(extents: &[(u64, u32)]) -> String {
    let mut output = String::from("[");
    for (index, (start, blocks)) in extents.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(output, "{{\"start\":{start},\"blocks\":{blocks}}}")
            .expect("writing to String cannot fail");
    }
    output.push(']');
    output
}

fn sorted_u64_json(values: &[u64]) -> String {
    let mut values = values.to_vec();
    values.sort_unstable();
    let mut output = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(output, "{value}").expect("writing to String cannot fail");
    }
    output.push(']');
    output
}
