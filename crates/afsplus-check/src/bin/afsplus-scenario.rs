//! Private memory scenario runner. Fixed framed stdout; no filesystem output paths.
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::{
    replay_trace::{base_digest, Limits, Trace},
    scenario::{LinkedKind, Plan},
};
use std::io::{self, Read, Write};

fn hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        write!(&mut out, "{byte:02x}").expect("string write");
    }
    if out.is_empty() {
        "-".into()
    } else {
        out
    }
}
fn metadata(value: &afsplus_core::volume::ObjectMetadata) -> String {
    use afsplus_format::object::ObjectType;
    let kind = match value.object_type {
        ObjectType::File => "file",
        ObjectType::Directory => "directory",
        ObjectType::Symlink => "symlink",
        ObjectType::Internal => "internal",
    };
    format!(
        "{} {kind} {} {} {} {} {} {} {} {} {} {} {}",
        value.object_id,
        value.size_bytes,
        value.allocated_bytes,
        value.link_count,
        value.protection,
        value.created.seconds,
        value.created.nanoseconds,
        value.modified.seconds,
        value.modified.nanoseconds,
        value.changed.seconds,
        value.changed.nanoseconds,
        value.content_generation
    )
}

fn coverage(ranges: &[afsplus_core::volume::FileAllocationRange]) -> Result<String, String> {
    let merged = afsplus_check::scenario::normalize_allocation(ranges)?;
    if merged.is_empty() {
        return Ok("-".into());
    }
    Ok(merged
        .iter()
        .map(|(offset, length, unwritten)| format!("{offset}:{length}:{}", u8::from(*unwritten)))
        .collect::<Vec<_>>()
        .join(","))
}

fn captured_observation(
    result: Result<Vec<afsplus_check::scenario::captured::View>, String>,
    normalized: bool,
) -> Result<String, String> {
    let mut out = String::new();
    match result {
        Err(error) => out.push_str(&format!("snapshots error {}\n", hex(error.as_bytes()))),
        Ok(views) => {
            out.push_str(&format!("snapshots ok {}\n", views.len()));
            for view in views {
                out.push_str(&format!(
                    "snapshot {} {} {} {}\n",
                    view.info.id,
                    view.info.generation,
                    view.info.committed_tx_id,
                    view.entries.len()
                ));
                out.push_str(&format!("root {}\n", metadata(&view.root)));
                for entry in view.entries {
                    let path = entry
                        .path
                        .iter()
                        .map(|p| hex(p.as_bytes()))
                        .collect::<Vec<_>>()
                        .join(",");
                    if normalized {
                        // Version 9 compares layout-independent logical coverage.
                        out.push_str(&format!(
                            "entry {path} {} {} {}\n",
                            metadata(&entry.metadata),
                            hex(&entry.data),
                            coverage(&entry.allocation)?
                        ));
                        continue;
                    }
                    out.push_str(&format!(
                        "entry {path} {} {} {}\n",
                        metadata(&entry.metadata),
                        hex(&entry.data),
                        entry.allocation.len()
                    ));
                    for range in entry.allocation {
                        out.push_str(&format!(
                            "range {} {} {}\n",
                            range.offset,
                            range.length,
                            u8::from(range.unwritten)
                        ));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn header(out: &mut impl Write, name: &str, length: usize) -> io::Result<()> {
    out.write_all(&(name.len() as u16).to_le_bytes())?;
    out.write_all(name.as_bytes())?;
    out.write_all(&(length as u64).to_le_bytes())
}
fn artifact(out: &mut impl Write, name: &str, data: &[u8]) -> io::Result<()> {
    header(out, name, data.len())?;
    out.write_all(data)
}
fn image(out: &mut impl Write, name: &str, device: &mut MemoryBackend) -> Result<(), String> {
    header(
        out,
        name,
        device.total_blocks() as usize * device.block_size(),
    )
    .map_err(|e| e.to_string())?;
    let mut block = vec![0; device.block_size()];
    for lba in 0..device.total_blocks() {
        device
            .read_block(lba, &mut block)
            .map_err(|e| e.to_string())?;
        out.write_all(&block).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Wire code for one diagnostic event kind: its stable code
/// ([`EventKind::code`]). Codes 1 to 22 are the version-4/5/6 kinds; 23
/// onward are appended by the version-8 profile. An export profile that does
/// not select a scope refuses its kinds rather than emitting an unadmitted
/// code.
///
/// [`EventKind::code`]: afsplus_core::flight::EventKind::code
fn event_kind_code(
    kind: afsplus_core::flight::EventKind,
    plan: &afsplus_check::scenario::Plan,
) -> Result<u8, String> {
    let code = kind.code();
    let admitted = match code {
        1..=7 => true,
        8..=19 => plan.api_observation(),
        20..=22 => plan.object_observation(),
        _ => plan.lifecycle_observation(),
    };
    if !admitted {
        return Err("diagnostic event kind requires an extended export profile".into());
    }
    u8::try_from(code).map_err(|_| "diagnostic event code exceeds the bundle's byte".into())
}

/// One tagged fixed payload area: a presence tag, four enumeration bytes, one
/// 32-bit field and five 64-bit fields, all little endian. Every byte a class
/// does not name is zero, so absence is distinguishable from a zero value only
/// through the tag and the per-kind rules the consumer applies.
fn encode_payload(flight: &mut Vec<u8>, event: &afsplus_core::flight::Event) -> Result<(), String> {
    use afsplus_core::flight::{Category, LifecycleContext};
    let mut tag = 0u8;
    let mut bytes = [0u8; 4];
    let mut word32 = 0u32;
    let mut words = [0u64; 5];
    let present = u8::from(event.allocation.is_some())
        + u8::from(event.tree.is_some())
        + u8::from(event.reclaim.is_some())
        + u8::from(event.lifecycle.is_some());
    if present > 1 {
        return Err("event carries several diagnostic payloads".into());
    }
    if let Some(context) = event.allocation {
        tag = 1;
        words[0] = context.start;
        words[1] = context.blocks;
    } else if let Some(context) = event.tree {
        tag = 2;
        words[0] = context.owner;
        words[1] = context.block;
        words[2] = context.resident;
    } else if let Some(context) = event.reclaim {
        tag = 3;
        words[0] = context.root;
        words[1] = context.start;
        words[2] = context.blocks;
    } else if let Some(context) = event.lifecycle {
        match context {
            LifecycleContext::Mount(c) => {
                tag = 4;
                bytes = [
                    c.mode as u8,
                    c.stage as u8,
                    c.slot,
                    u8::from(c.damaged_tail),
                ];
                word32 = c.count;
                words[0] = c.other_generation;
            }
            LifecycleContext::Format(c) => {
                tag = 5;
                bytes[0] = c.stage as u8;
                words[0] = c.total_blocks;
                words[1] = c.block;
            }
            LifecycleContext::Verify(c) => {
                tag = 6;
                bytes = [
                    c.scope as u8,
                    c.phase.map_or(0, |phase| phase as u8),
                    c.finding.map_or(0, |finding| finding as u8),
                    0,
                ];
                word32 = c.region;
                words[0] = c.ordinal;
                words[1] = c.object_id;
                words[2] = c.block;
            }
            LifecycleContext::Data(c) => {
                tag = 7;
                bytes[0] = c.scope as u8;
                word32 = c.blocks;
                words[0] = c.object_id;
                words[1] = c.offset;
                words[2] = c.length;
                words[3] = c.start;
            }
            LifecycleContext::View(c) => {
                tag = 8;
                bytes[0] = c.path as u8;
                words[0] = c.view_id;
                words[1] = c.owner;
                words[2] = c.block;
            }
        }
    }
    let expected = match event.kind.category() {
        Category::Allocator => 1,
        Category::Tree => 2,
        Category::Reclaim => 3,
        Category::Mount => 4,
        Category::Format => 5,
        Category::Verify => 6,
        Category::Data => 7,
        Category::View => 8,
        _ => 0,
    };
    if tag != expected {
        return Err("diagnostic payload differs from event kind".into());
    }
    flight.push(tag);
    flight.extend_from_slice(&bytes);
    flight.extend_from_slice(&word32.to_le_bytes());
    for value in words {
        flight.extend_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

fn run() -> Result<(), String> {
    if std::env::args_os().len() != 1 {
        return Err("scenario runner accepts only stdin".into());
    }
    let mut input = Vec::new();
    io::stdin()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    let (fault, commands) = if input.starts_with(b"AFSCUT01 ") {
        let split = input
            .iter()
            .position(|&b| b == b'\n')
            .ok_or("cut header terminator")?;
        let header = std::str::from_utf8(&input[..split]).map_err(|_| "cut header encoding")?;
        let fields: Vec<_> = header.split(' ').collect();
        let ["AFSCUT01", operation, offset, variant] = fields.as_slice() else {
            return Err("cut header fields".into());
        };
        let number = |text: &str, maximum: usize| -> Result<usize, String> {
            if text.is_empty()
                || !text.bytes().all(|b| b.is_ascii_digit())
                || (text.len() > 1 && text.starts_with('0'))
            {
                return Err("cut integer encoding".into());
            }
            let value = text.parse::<usize>().map_err(|_| "cut integer")?;
            if value > maximum {
                return Err("cut integer admission".into());
            }
            Ok(value)
        };
        (
            Some((
                number(operation, 1023)?,
                number(offset, 65536)?,
                number(variant, 4131)?,
            )),
            &input[split + 1..],
        )
    } else {
        (None, input.as_slice())
    };
    let plan = Plan::parse(commands)?;
    let mut run = plan.run()?;
    if let Some((operation, offset, variant)) = fault {
        if run.failure.is_some() {
            return Err("fault scenario did not finish recording".into());
        }
        let event = run
            .events
            .get(operation)
            .ok_or("fault operation outside scenario")?;
        let cut = event
            .first_block_operation
            .checked_add(offset)
            .ok_or("fault cut overflow")?;
        if cut > event.end_block_operation {
            return Err("fault offset outside operation".into());
        }
        run.result = afsplus_check::crash_replay::select(&run.base, &run.log, cut, variant)?;
    }
    let limits = Limits {
        wire_bytes: 66 * 1024 * 1024,
        operations: 65536,
        block_size: 4096,
        base_blocks: 65536,
    };
    let trace = Trace {
        block_size: run.base.block_size(),
        blocks: run.base.total_blocks(),
        base_digest: base_digest(&mut run.base, limits)?,
        operations: run.log,
    }
    .encode(limits)?;
    let inspection = plan.inspect_checked_with_orphans(
        run.result.clone(),
        16 * 1024 * 1024,
        &run.orphan_candidates,
    );
    let mut observed = if let Some(pages) = plan.cache_profile() {
        if inspection
            .cache_pages
            .is_some_and(|effective| effective != pages)
        {
            return Err("inspection cache policy mismatch".into());
        }
        let profile = if pages == usize::MAX {
            "unlimited".to_owned()
        } else {
            pages.to_string()
        };
        format!(
            "{}\ncache-pages {profile}\n",
            if plan.linked_observation() {
                "AFSOBS05"
            } else if plan.snapshot_limits().is_some() {
                "AFSOBS04"
            } else {
                "AFSOBS03"
            }
        )
    } else {
        String::from("AFSOBS02\n")
    };
    match run.failure {
        Some((index, error)) => {
            observed.push_str(&format!("run error {index} {}\n", hex(error.as_bytes())))
        }
        None => observed.push_str("run ok\n"),
    }
    observed.push_str(&format!(
        "raw-check {}\n",
        hex(inspection.raw.render_json().as_bytes())
    ));
    observed.push_str(&format!(
        "recovered-check {}\n",
        inspection
            .recovered
            .as_ref()
            .map(|report| hex(report.render_json().as_bytes()))
            .unwrap_or_else(|| "-".into())
    ));
    if let Some(orphans) = inspection.orphans {
        observed.push_str(&match orphans {
            Err(error) => format!("orphans error {}\n", hex(error.as_bytes())),
            Ok((count, bytes)) => format!("orphans {count} {bytes}\n"),
        });
    }
    match (inspection.linked, inspection.entries) {
        (Some(Err(error)), _) | (None, Err(error)) => {
            observed.push_str(&format!("observe error {}\n", hex(error.as_bytes())))
        }
        (Some(Ok(entries)), _) => {
            observed.push_str("observe ok\n");
            // A file alias is the sorted index of the first path naming its object.
            let mut first = std::collections::BTreeMap::new();
            for (index, entry) in entries.iter().enumerate() {
                let path = entry
                    .path
                    .iter()
                    .map(|s| hex(s.as_bytes()))
                    .collect::<Vec<_>>()
                    .join(",");
                let (links, protection) = (entry.link_count, entry.protection);
                observed.push_str(&match entry.kind {
                    LinkedKind::Directory => format!("directory {path} {links} {protection}\n"),
                    LinkedKind::File => {
                        let alias = *first.entry(entry.object_id).or_insert(index);
                        let coverage = if entry.allocation.is_empty() {
                            "-".to_owned()
                        } else {
                            entry
                                .allocation
                                .iter()
                                .map(|(offset, length, unwritten)| {
                                    format!("{offset}:{length}:{}", u8::from(*unwritten))
                                })
                                .collect::<Vec<_>>()
                                .join(",")
                        };
                        format!(
                            "file {path} {links} {protection} {alias} {} {} {coverage}\n",
                            u8::from(entry.in_place),
                            hex(&entry.data)
                        )
                    }
                    LinkedKind::Symlink => {
                        format!("symlink {path} {links} {protection} {}\n", hex(&entry.data))
                    }
                });
            }
        }
        (None, Ok(entries)) => {
            observed.push_str("observe ok\n");
            for entry in entries {
                let path = entry
                    .path
                    .iter()
                    .map(|s| hex(s.as_bytes()))
                    .collect::<Vec<_>>()
                    .join(",");
                match entry.data {
                    Some(data) => observed.push_str(&format!("file {path} {}\n", hex(&data))),
                    None => observed.push_str(&format!("directory {path}\n")),
                }
            }
        }
    }
    if let Some(snapshots) = inspection.snapshots {
        observed.push_str(&captured_observation(snapshots, plan.linked_observation())?);
    }
    // Semantic flight records bind operation indices and resolved object IDs to
    // half-open successful block-log ranges. V2 additionally carries internal
    // commit-tail batches, including explicit ring-loss accounting.
    let mut flight = if plan.lifecycle_observation() {
        b"AFSFLT06"
    } else if plan.object_observation() {
        b"AFSFLT05"
    } else if plan.api_observation() {
        b"AFSFLT04"
    } else if plan.diagnostic_profile().is_some() {
        b"AFSFLT03"
    } else if plan.flight_capacity().is_some() {
        b"AFSFLT02"
    } else {
        b"AFSFLT01"
    }
    .to_vec();
    flight.extend_from_slice(&(run.events.len() as u32).to_le_bytes());
    if let Some(capacity) = plan.flight_capacity() {
        flight.extend_from_slice(&(capacity as u32).to_le_bytes());
    }
    if let Some(profile) = plan.diagnostic_profile() {
        flight.extend_from_slice(&(profile.categories as u32).to_le_bytes());
        flight.extend_from_slice(&(profile.sink_capacity as u32).to_le_bytes());
        flight.extend_from_slice(
            &profile
                .disconnect_before
                .map_or(u32::MAX, |n| n as u32)
                .to_le_bytes(),
        );
    }
    let encode_batch = |flight: &mut Vec<u8>,
                        batch: afsplus_check::scenario::FlightBatch|
     -> Result<(), String> {
        flight.extend_from_slice(&batch.dropped_total.to_le_bytes());
        if plan.diagnostic_profile().is_some() {
            for value in [
                batch.filtered_total,
                batch.sequence_total,
                batch.attempt_total,
                batch.delivered_total,
                batch.missed_total,
            ] {
                flight.extend_from_slice(&value.to_le_bytes());
            }
            flight.push(u8::from(batch.sink_closed));
        }
        flight.extend_from_slice(&(batch.events.len() as u32).to_le_bytes());
        for internal in batch.events {
            flight.extend_from_slice(&internal.sequence.to_le_bytes());
            flight.extend_from_slice(&internal.attempt.to_le_bytes());
            flight.extend_from_slice(&internal.generation.to_le_bytes());
            flight.push(event_kind_code(internal.kind, &plan)?);
            flight.push(u8::from(internal.requires_remount));
            if plan.api_observation() {
                for value in [
                    internal.api.operation,
                    internal.api.span,
                    internal.api.parent_span,
                ] {
                    flight.extend_from_slice(&value.to_le_bytes());
                }
                flight
                    .extend_from_slice(&internal.api.method.map_or(0, |m| m as u16).to_le_bytes());
                flight.extend_from_slice(&internal.window.to_le_bytes());
                flight.extend_from_slice(&internal.log_sequence.to_le_bytes());
            }
            if plan.object_observation() {
                flight.push(u8::from(internal.object.is_some()));
                let (object, block, view) = internal
                    .object
                    .map_or((0, 0, 0), |c| (c.object_id, c.record_block, c.view_id));
                for value in [object, block, view] {
                    flight.extend_from_slice(&value.to_le_bytes());
                }
            }
            if plan.lifecycle_observation() {
                encode_payload(flight, &internal)?;
            }
        }
        Ok(())
    };
    if plan.lifecycle_observation() {
        let batch = run
            .pre_mount
            .take()
            .ok_or("missing pre-mount flight batch")?;
        encode_batch(&mut flight, batch)?;
    }
    for event in run.events {
        flight.extend_from_slice(&(event.operation as u32).to_le_bytes());
        flight.extend_from_slice(&(event.first_block_operation as u64).to_le_bytes());
        flight.extend_from_slice(&(event.end_block_operation as u64).to_le_bytes());
        flight.extend_from_slice(&event.object_id.to_le_bytes());
        flight.push(u8::from(event.success));
        if plan.flight_capacity().is_some() {
            encode_batch(
                &mut flight,
                event.flight.ok_or("missing internal flight batch")?,
            )?;
        }
    }
    let mut out = io::BufWriter::new(io::stdout().lock());
    out.write_all(b"AFSRUN01").map_err(|e| e.to_string())?;
    artifact(&mut out, "actual.wire", observed.as_bytes()).map_err(|e| e.to_string())?;
    artifact(&mut out, "flight-recorder.bin", &flight).map_err(|e| e.to_string())?;
    artifact(&mut out, "block-io.afstrace", &trace).map_err(|e| e.to_string())?;
    image(&mut out, "start.img", &mut run.base)?;
    image(&mut out, "result.img", &mut run.result)?;
    out.flush().map_err(|e| e.to_string())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("scenario refused: {error}");
        std::process::exit(1);
    }
}
