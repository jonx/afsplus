//! Experimental Stage A semantic runner (ADR-099), confined to memory images.
pub mod captured;
use afsplus_block::{BlockDevice, BlockError, MemoryBackend, RecordedOp};
use afsplus_core::volume::{
    BatchOp, DataUpdatePolicy, FileEditLimits, PreservedMetadata, SnapshotHandle,
    SnapshotWorkLimits,
};
use afsplus_core::{
    mkfs_with_options, mount, mount_with_options, mount_with_snapshot_limits, MkfsOptions,
    MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{validate_name, Timespec, OBJECT_ROOT};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[derive(Debug)]
pub enum SnapshotAction {
    Create,
    Open,
    Close,
    Delete,
    Inspect,
}

/// One member of a version-9 atomic batch or one staged window namespace op.
/// Fields are colon separated inside a single space-separated wire field.
#[derive(Debug)]
pub enum BatchItem {
    Create {
        label: String,
        parent: String,
        name: String,
        data: Vec<u8>,
    },
    Delete {
        label: String,
    },
    Rename {
        label: String,
        parent: String,
        name: String,
        victim: Option<String>,
    },
}

#[derive(Debug)]
pub enum Operation {
    Snapshot {
        action: SnapshotAction,
        label: String,
    },
    Create {
        label: String,
        parent: String,
        name: String,
        data: Vec<u8>,
        directory: bool,
    },
    Write {
        deferred: bool,
        label: String,
        offset: u64,
        data: Vec<u8>,
    },
    Truncate {
        deferred: bool,
        label: String,
        size: u64,
    },
    Rename {
        label: String,
        parent: String,
        name: String,
    },
    Remove {
        label: String,
        directory: bool,
    },
    /// Version 9: another directory link to an existing regular file.
    Link {
        label: String,
        source: String,
        parent: String,
        name: String,
    },
    /// Version 9: an opaque UTF-8 target, never resolved by the runner.
    Symlink {
        label: String,
        parent: String,
        name: String,
        target: String,
    },
    CloneFile {
        label: String,
        source: String,
        parent: String,
        name: String,
    },
    CloneRange {
        source: String,
        source_offset: u64,
        destination: String,
        destination_offset: u64,
        length: u64,
    },
    SetProtection {
        label: String,
        protection: u32,
    },
    UnlinkSymlink {
        label: String,
    },
    /// Version 9: atomic file-over-file replacement. `victim` names the label
    /// whose entry `parent`/`name` the source replaces; `orphan` selects
    /// `rename_replace_orphan_target`.
    RenameReplace {
        label: String,
        victim: String,
        parent: String,
        name: String,
        orphan: bool,
    },
    /// Version 9: move a file's final visible link into the reserved directory.
    OrphanFile {
        label: String,
    },
    /// Version 9: one budgeted orphan cleanup step for an orphaned label.
    CleanupOrphan {
        label: String,
    },
    /// Version 9: reserve physical capacity without changing the logical size.
    Preallocate {
        label: String,
        offset: u64,
        length: u64,
        limits: Option<(u64, usize)>,
    },
    /// Version 9: persistent per-file data-update policy.
    SetDataPolicy {
        label: String,
        in_place: bool,
    },
    /// Version 9: restore archived protection and timestamps verbatim.
    RestoreMetadata {
        label: String,
        protection: u32,
        created: i64,
        modified: i64,
        changed: i64,
    },
    /// Version 9: one bounded reclaim or snapshot-maintenance step.
    ReclaimStep,
    SnapshotMaintenanceStep,
    /// Version 9: one atomic `run_batch`, or one staged window namespace op.
    Batch {
        items: Vec<BatchItem>,
        deferred: bool,
    },
    Sync,
    WindowFsync,
    WindowCommit,
    Remount,
}
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticProfile {
    pub categories: u8,
    pub sink_capacity: usize,
    pub disconnect_before: Option<usize>,
}

pub struct Plan {
    // None identifies the original version-1 default profile.
    cache_profile: Option<usize>,
    flight_capacity: Option<usize>,
    diagnostic_profile: Option<DiagnosticProfile>,
    api_observation: bool,
    object_observation: bool,
    linked_observation: bool,
    snapshot_limits: Option<SnapshotWorkLimits>,
    /// Version 9: volume data-policy feature and orphan cleanup extent budget.
    data_policy: bool,
    orphan_extents: usize,
    blocks: u64,
    region: u32,
    log_slots: u16,
    operations: Vec<Operation>,
}
#[derive(Debug)]
pub struct Event {
    pub operation: usize,
    pub first_block_operation: usize,
    pub end_block_operation: usize,
    pub object_id: u64,
    pub success: bool,
    pub flight: Option<FlightBatch>,
}

/// Common commit-tail events for this semantic operation. Mount recovery is
/// outside this first diagnostic scope; an empty batch is not coverage proof.
#[derive(Debug)]
pub struct FlightBatch {
    pub events: Vec<afsplus_core::flight::Event>,
    pub dropped_total: u64,
    pub filtered_total: u64,
    pub sequence_total: u64,
    pub attempt_total: u64,
    pub delivered_total: u64,
    pub missed_total: u64,
    pub sink_closed: bool,
}

fn drain_flight(ring: &mut afsplus_core::flight::FlightRecorder) -> FlightBatch {
    FlightBatch {
        events: ring.drain().collect(),
        dropped_total: ring.dropped(),
        filtered_total: ring.filtered(),
        sequence_total: ring.sequence(),
        attempt_total: ring.attempt(),
        delivered_total: ring.delivered(),
        missed_total: ring.missed(),
        sink_closed: ring.sink_closed(),
    }
}

fn capture_flight<D: BlockDevice>(volume: &mut afsplus_core::Volume<D>) -> Option<FlightBatch> {
    volume
        .flight_recorder_mut()
        .map(|mut recorder| drain_flight(&mut recorder))
}
pub struct Run {
    pub events: Vec<Event>,
    pub base: MemoryBackend,
    pub result: MemoryBackend,
    pub log: Vec<RecordedOp>,
    pub failure: Option<(usize, String)>,
    /// Objects this run placed in the reserved orphan directory. Observation
    /// sums the sizes of those the remounted volume still names.
    pub orphan_candidates: Vec<u64>,
}
/// Admission caps for captured block operations across all remounts.
/// These bound retained log payload and count, not the filesystem's total RAM.
#[derive(Clone, Copy)]
pub struct RecordingLimits {
    pub operations: usize,
    pub payload_bytes: usize,
}
impl Default for RecordingLimits {
    fn default() -> Self {
        Self {
            operations: 65536,
            payload_bytes: 64 * 1024 * 1024,
        }
    }
}
struct Capture {
    image: MemoryBackend,
    log: Vec<RecordedOp>,
    bytes: usize,
    limits: RecordingLimits,
}
#[derive(Clone)]
struct Recorder(Rc<RefCell<Capture>>);
impl Capture {
    fn reserve(&mut self, bytes: usize) -> Result<(), BlockError> {
        if self.log.len() >= self.limits.operations
            || bytes > self.limits.payload_bytes.saturating_sub(self.bytes)
        {
            return Err(BlockError::Injected("scenario recording limit"));
        }
        self.log
            .try_reserve(1)
            .map_err(|_| BlockError::Injected("scenario log allocation"))
    }
}
impl BlockDevice for Recorder {
    fn block_size(&self) -> usize {
        self.0.borrow().image.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.0.borrow().image.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.0.borrow_mut().image.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        let mut state = self.0.borrow_mut();
        state.reserve(data.len())?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(data.len())
            .map_err(|_| BlockError::Injected("scenario payload allocation"))?;
        copy.extend_from_slice(data);
        state.image.write_block(lba, data)?;
        state.bytes += data.len();
        state.log.push(RecordedOp::Write { lba, data: copy });
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        let mut state = self.0.borrow_mut();
        state.reserve(0)?;
        state.image.flush()?;
        state.log.push(RecordedOp::Flush);
        Ok(())
    }
}
fn finish(
    base: MemoryBackend,
    recorder: Recorder,
    failure: Option<(usize, String)>,
    events: Vec<Event>,
    orphan_candidates: Vec<u64>,
) -> Run {
    let state = Rc::try_unwrap(recorder.0)
        .ok()
        .expect("volume released recorder")
        .into_inner();
    Run {
        events,
        base,
        result: state.image,
        log: state.log,
        failure,
        orphan_candidates,
    }
}
fn integer(s: &str, maximum: u64) -> Result<u64, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return Err("noncanonical scenario integer".into());
    }
    let n = s.parse::<u64>().map_err(|_| "scenario integer overflow")?;
    if n > maximum {
        return Err("scenario integer limit".into());
    }
    Ok(n)
}
fn label(s: &str) -> Result<String, String> {
    if s.is_empty()
        || s.len() > 64
        || !s
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    {
        return Err("invalid scenario label".into());
    }
    Ok(s.into())
}
fn hex(s: &str) -> Result<Vec<u8>, String> {
    if s == "-" {
        return Ok(Vec::new());
    }
    if s.is_empty()
        || !s.len().is_multiple_of(2)
        || s.len() > 2 * 1024 * 1024
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid scenario hex data".into());
    }
    let mut data = Vec::new();
    data.try_reserve_exact(s.len() / 2)
        .map_err(|_| "scenario data allocation")?;
    for pair in s.as_bytes().as_chunks::<2>().0 {
        let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        data.push(digit(pair[0]) * 16 + digit(pair[1]));
    }
    Ok(data)
}
fn name(s: &str) -> Result<String, String> {
    let bytes = hex(s)?;
    validate_name(&bytes).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
/// Harness admission for an opaque target; the core applies its own record limit.
fn symlink_target(s: &str) -> Result<String, String> {
    let bytes = hex(s)?;
    if bytes.is_empty() || bytes.len() > 1024 || bytes.contains(&0) {
        return Err("invalid scenario symlink target".into());
    }
    String::from_utf8(bytes).map_err(|_| "scenario symlink target UTF-8".into())
}
/// Version 9: one wire field carries a bounded colon/comma separated batch.
fn batch_items(field: &str) -> Result<Vec<BatchItem>, String> {
    let mut items = Vec::new();
    for raw in field.split(',') {
        if items.len() >= 16 {
            return Err("scenario batch item limit".into());
        }
        let parts: Vec<_> = raw.split(':').collect();
        items.push(match parts.as_slice() {
            ["create", l, p, n, d] => BatchItem::Create {
                label: label(l)?,
                parent: label(p)?,
                name: name(n)?,
                data: hex(d)?,
            },
            ["delete", l] => BatchItem::Delete { label: label(l)? },
            ["rename", l, p, n] => BatchItem::Rename {
                label: label(l)?,
                parent: label(p)?,
                name: name(n)?,
                victim: None,
            },
            ["replace", l, v, p, n] => BatchItem::Rename {
                label: label(l)?,
                parent: label(p)?,
                name: name(n)?,
                victim: Some(label(v)?),
            },
            _ => return Err("invalid scenario batch item".into()),
        });
    }
    Ok(items)
}

fn file_range(offset: &str, length: u64) -> Result<u64, String> {
    let offset = integer(offset, 16 * 1024 * 1024)?;
    if offset + length > 16 * 1024 * 1024 {
        return Err("scenario clone range".into());
    }
    Ok(offset)
}
impl Plan {
    pub fn parse(wire: &[u8]) -> Result<Self, String> {
        if wire.len() > 4 * 1024 * 1024 || !wire.ends_with(b"\n") {
            return Err("scenario wire admission".into());
        }
        let input = std::str::from_utf8(wire).map_err(|_| "scenario wire UTF-8")?;
        let mut lines = input.lines();
        let version = lines.next();
        if !matches!(
            version,
            Some(
                "AFSPSC01"
                    | "AFSPSC02"
                    | "AFSPSC03"
                    | "AFSPSC04"
                    | "AFSPSC05"
                    | "AFSPSC06"
                    | "AFSPSC07"
                    | "AFSPSC09"
            )
        ) {
            return Err("scenario protocol version".into());
        }
        // Version 9 inherits the version-7 profile and adds linked namespace commands.
        let linked = version == Some("AFSPSC09");
        let mut header: Vec<_> = lines.next().ok_or("missing geometry")?.split(' ').collect();
        // Version 9 appends its volume feature and maintenance budgets last, so
        // the reverse-order header scan reads them before the version-7 fields.
        let (data_policy, orphan_extents) = if linked {
            let extents =
                integer(header.pop().ok_or("missing orphan extent budget")?, 64)? as usize;
            let policy = integer(header.pop().ok_or("missing data policy flag")?, 1)?;
            if extents == 0 {
                return Err("zero orphan extent budget".into());
            }
            (policy == 1, extents)
        } else {
            (false, 1)
        };
        let snapshot_limits = if matches!(version, Some("AFSPSC07" | "AFSPSC09")) {
            let reclaim_records =
                integer(header.pop().ok_or("missing snapshot reclaim limit")?, 4096)? as usize;
            let max_views =
                integer(header.pop().ok_or("missing snapshot view limit")?, 16)? as usize;
            let max_edit_records =
                integer(header.pop().ok_or("missing snapshot edit limit")?, 4096)? as usize;
            if reclaim_records == 0 || max_views == 0 || max_edit_records == 0 {
                return Err("zero snapshot limit".into());
            }
            Some(SnapshotWorkLimits {
                max_edit_records,
                max_views,
                reclaim_records,
            })
        } else {
            None
        };
        let diagnostic_profile = if matches!(
            version,
            Some("AFSPSC04" | "AFSPSC05" | "AFSPSC06" | "AFSPSC07" | "AFSPSC09")
        ) {
            let disconnect_before = match header.pop().ok_or("missing disconnect index")? {
                "none" => None,
                value => Some(integer(value, 1024)? as usize),
            };
            let sink_capacity =
                integer(header.pop().ok_or("missing sink capacity")?, 256)? as usize;
            let maximum = match version {
                Some("AFSPSC06" | "AFSPSC07" | "AFSPSC09") => 127,
                Some("AFSPSC05") => 63,
                _ => 15,
            };
            let categories = integer(header.pop().ok_or("missing category mask")?, maximum)? as u8;
            if sink_capacity == 0 && disconnect_before.is_some() {
                return Err("disconnect requires an attached sink".into());
            }
            Some(DiagnosticProfile {
                categories,
                sink_capacity,
                disconnect_before,
            })
        } else {
            None
        };
        let flight_capacity = if matches!(
            version,
            Some("AFSPSC03" | "AFSPSC04" | "AFSPSC05" | "AFSPSC06" | "AFSPSC07" | "AFSPSC09")
        ) {
            let capacity = integer(header.pop().ok_or("missing flight capacity")?, 256)? as usize;
            if capacity == 0 {
                return Err("zero flight capacity".into());
            }
            Some(capacity)
        } else {
            None
        };
        let cache_profile = if matches!(
            version,
            Some(
                "AFSPSC02"
                    | "AFSPSC03"
                    | "AFSPSC04"
                    | "AFSPSC05"
                    | "AFSPSC06"
                    | "AFSPSC07"
                    | "AFSPSC09"
            )
        ) {
            Some(match header.pop() {
                Some("2") => 2,
                Some("4") => 4,
                Some("8") => 8,
                Some("unlimited") => usize::MAX,
                _ => return Err("scenario tree cache profile".into()),
            })
        } else {
            None
        };
        let ["format", "4096", blocks, region, log] = header.as_slice() else {
            return Err("scenario format profile".into());
        };
        let blocks = integer(blocks, 65536)?;
        let region = integer(region, 16384)? as u32;
        let log_slots = integer(log, 64)? as u16;
        if blocks < 64
            || region < 64
            || !region.is_power_of_two()
            || u64::from(region) > blocks
            || log_slots < 8
        {
            return Err("scenario geometry admission".into());
        }
        let mut operations = Vec::new();
        let mut payload = 0usize;
        for line in lines {
            if operations.len() >= 1024 {
                return Err("scenario operation limit".into());
            }
            let args: Vec<_> = line.split(' ').collect();
            let op = match args.as_slice() {
                ["mkdir", l, p, n] => Operation::Create {
                    label: label(l)?,
                    parent: label(p)?,
                    name: name(n)?,
                    data: Vec::new(),
                    directory: true,
                },
                ["create", l, p, n, d] => Operation::Create {
                    label: label(l)?,
                    parent: label(p)?,
                    name: name(n)?,
                    data: hex(d)?,
                    directory: false,
                },
                [kind @ ("write" | "window_write"), l, o, d]
                    if *kind == "write"
                        || matches!(
                            version,
                            Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07" | "AFSPSC09")
                        ) =>
                {
                    Operation::Write {
                        deferred: *kind == "window_write",
                        label: label(l)?,
                        offset: integer(o, 16 * 1024 * 1024)?,
                        data: hex(d)?,
                    }
                }
                [kind @ ("truncate" | "window_truncate"), l, s]
                    if *kind == "truncate"
                        || matches!(
                            version,
                            Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07" | "AFSPSC09")
                        ) =>
                {
                    Operation::Truncate {
                        deferred: *kind == "window_truncate",
                        label: label(l)?,
                        size: integer(s, 16 * 1024 * 1024)?,
                    }
                }
                ["rename", l, p, n] => Operation::Rename {
                    label: label(l)?,
                    parent: label(p)?,
                    name: name(n)?,
                },
                ["unlink", l] => Operation::Remove {
                    label: label(l)?,
                    directory: false,
                },
                ["rmdir", l] => Operation::Remove {
                    label: label(l)?,
                    directory: true,
                },
                [kind @ ("snapshot_create" | "snapshot_open" | "snapshot_close"
                | "snapshot_delete" | "snapshot_inspect"), name]
                    if matches!(version, Some("AFSPSC07" | "AFSPSC09")) =>
                {
                    Operation::Snapshot {
                        action: match *kind {
                            "snapshot_create" => SnapshotAction::Create,
                            "snapshot_open" => SnapshotAction::Open,
                            "snapshot_close" => SnapshotAction::Close,
                            "snapshot_delete" => SnapshotAction::Delete,
                            _ => SnapshotAction::Inspect,
                        },
                        label: label(name)?,
                    }
                }
                ["link", l, s, p, n] if linked => Operation::Link {
                    label: label(l)?,
                    source: label(s)?,
                    parent: label(p)?,
                    name: name(n)?,
                },
                ["symlink", l, p, n, t] if linked => Operation::Symlink {
                    label: label(l)?,
                    parent: label(p)?,
                    name: name(n)?,
                    target: symlink_target(t)?,
                },
                ["clone_file", l, s, p, n] if linked => Operation::CloneFile {
                    label: label(l)?,
                    source: label(s)?,
                    parent: label(p)?,
                    name: name(n)?,
                },
                ["clone_range", s, so, d, dof, len] if linked => {
                    let length = integer(len, 16 * 1024 * 1024)?;
                    Operation::CloneRange {
                        source: label(s)?,
                        source_offset: file_range(so, length)?,
                        destination: label(d)?,
                        destination_offset: file_range(dof, length)?,
                        length,
                    }
                }
                ["set_protection", l, value] if linked => Operation::SetProtection {
                    label: label(l)?,
                    protection: integer(value, u64::from(u32::MAX))? as u32,
                },
                ["unlink_symlink", l] if linked => Operation::UnlinkSymlink { label: label(l)? },
                [kind @ ("rename_replace" | "rename_replace_orphan"), l, v, p, n] if linked => {
                    Operation::RenameReplace {
                        label: label(l)?,
                        victim: label(v)?,
                        parent: label(p)?,
                        name: name(n)?,
                        orphan: *kind == "rename_replace_orphan",
                    }
                }
                ["orphan_file", l] if linked => Operation::OrphanFile { label: label(l)? },
                ["cleanup_orphan", l] if linked => Operation::CleanupOrphan { label: label(l)? },
                ["preallocate", l, o, n] if linked => Operation::Preallocate {
                    label: label(l)?,
                    length: integer(n, 16 * 1024 * 1024)?,
                    offset: file_range(o, integer(n, 16 * 1024 * 1024)?)?,
                    limits: None,
                },
                ["preallocate_bounded", l, o, n, b, r] if linked => Operation::Preallocate {
                    label: label(l)?,
                    length: integer(n, 16 * 1024 * 1024)?,
                    offset: file_range(o, integer(n, 16 * 1024 * 1024)?)?,
                    limits: Some((integer(b, 4096)?, integer(r, 4096)? as usize)),
                },
                ["set_data_policy", l, v] if linked => Operation::SetDataPolicy {
                    label: label(l)?,
                    in_place: integer(v, 1)? == 1,
                },
                ["restore_metadata", l, p, c, m, h] if linked => Operation::RestoreMetadata {
                    label: label(l)?,
                    protection: integer(p, u64::from(u32::MAX))? as u32,
                    created: integer(c, 1 << 31)? as i64,
                    modified: integer(m, 1 << 31)? as i64,
                    changed: integer(h, 1 << 31)? as i64,
                },
                ["reclaim_step"] if linked => Operation::ReclaimStep,
                ["snapshot_maintenance_step"] if linked => Operation::SnapshotMaintenanceStep,
                [kind @ ("batch" | "window_batch"), items] if linked => Operation::Batch {
                    items: batch_items(items)?,
                    deferred: *kind == "window_batch",
                },
                ["sync"] => Operation::Sync,
                ["window_fsync"]
                    if matches!(
                        version,
                        Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07" | "AFSPSC09")
                    ) =>
                {
                    Operation::WindowFsync
                }
                ["window_commit"]
                    if matches!(
                        version,
                        Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07" | "AFSPSC09")
                    ) =>
                {
                    Operation::WindowCommit
                }
                ["remount"] => Operation::Remount,
                _ => return Err("unknown scenario command or arity".into()),
            };
            match &op {
                Operation::Create { data, .. } => payload += data.len(),
                Operation::Symlink { target, .. } => payload += target.len(),
                Operation::Batch { items, .. } => {
                    for item in items {
                        if let BatchItem::Create { data, .. } = item {
                            payload += data.len();
                        }
                    }
                }
                Operation::Write { offset, data, .. } => {
                    payload += data.len();
                    if *offset + data.len() as u64 > 16 * 1024 * 1024 {
                        return Err("scenario write range".into());
                    }
                }
                _ => (),
            }
            if payload > 1024 * 1024 {
                return Err("scenario aggregate payload".into());
            }
            operations.push(op);
        }
        Ok(Self {
            cache_profile,
            flight_capacity,
            diagnostic_profile,
            api_observation: matches!(
                version,
                Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07" | "AFSPSC09")
            ),
            object_observation: matches!(version, Some("AFSPSC06" | "AFSPSC07" | "AFSPSC09")),
            linked_observation: linked,
            snapshot_limits,
            data_policy,
            orphan_extents,
            blocks,
            region,
            log_slots,
            operations,
        })
    }
    pub fn run(&self) -> Result<Run, String> {
        self.run_with_limits(RecordingLimits::default())
    }
    /// Explicit v2 resource profile; v1 keeps its original unlimited contract.
    pub fn diagnostic_profile(&self) -> Option<DiagnosticProfile> {
        self.diagnostic_profile
    }

    pub fn object_observation(&self) -> bool {
        self.object_observation
    }

    pub fn api_observation(&self) -> bool {
        self.api_observation
    }

    /// Version 9 observes link counts, protection and symlink targets.
    pub fn linked_observation(&self) -> bool {
        self.linked_observation
    }

    pub fn flight_capacity(&self) -> Option<usize> {
        self.flight_capacity
    }

    pub fn cache_profile(&self) -> Option<usize> {
        self.cache_profile
    }

    pub fn snapshot_limits(&self) -> Option<SnapshotWorkLimits> {
        self.snapshot_limits
    }

    /// Version 9: the volume data-policy feature and the orphan cleanup budget.
    pub fn data_policy(&self) -> bool {
        self.data_policy
    }

    pub fn orphan_extents(&self) -> usize {
        self.orphan_extents
    }

    fn mount_profile<D: BlockDevice>(
        &self,
        device: D,
    ) -> Result<afsplus_core::Volume<D>, afsplus_core::CoreError> {
        let mut volume = match self.snapshot_limits {
            Some(limits) => mount_with_snapshot_limits(device, self.mount_options(), limits)?,
            None => mount_with_options(device, self.mount_options())?,
        };
        // Runtime-only budget, reapplied at every mount. The call happens
        // before the flight recorder is attached, so it records no event.
        volume.set_orphan_cleanup_extent_budget(self.orphan_extents);
        Ok(volume)
    }

    fn mount_options(&self) -> MountOptions {
        MountOptions {
            tree_cache_pages: self.cache_profile.and_then(std::num::NonZeroUsize::new),
            ..Default::default()
        }
    }

    /// Observe/recover a selected result under the same policy as execution.
    pub fn inspect_checked(&self, image: MemoryBackend, max_bytes: usize) -> Inspection {
        self.inspect_checked_with_orphans(image, max_bytes, &[])
    }

    /// Observe a selected result together with the reserved-directory state
    /// of the objects the executed run placed there.
    pub fn inspect_checked_with_orphans(
        &self,
        image: MemoryBackend,
        max_bytes: usize,
        orphan_candidates: &[u64],
    ) -> Inspection {
        inspect_checked_with_options(
            image,
            max_bytes,
            self.mount_options(),
            self.snapshot_limits,
            self.linked_observation,
            orphan_candidates,
        )
    }
    pub fn run_with_limits(&self, limits: RecordingLimits) -> Result<Run, String> {
        match self.flight_capacity {
            Some(capacity) => self.run_with_flight(limits, capacity),
            None => self.run_observed(limits, None, None, None),
        }
    }

    /// Retain at most 256 internal events per operation (1024 operations max).
    /// The recorder is carried across remounts, but mount-time recovery events
    /// are not instrumented here. Profiles 1–4 do not enable API/window scope.
    pub fn run_with_flight(&self, limits: RecordingLimits, capacity: usize) -> Result<Run, String> {
        if !(1..=256).contains(&capacity) {
            return Err("scenario flight capacity must be 1..=256".into());
        }
        let mut ring = afsplus_core::flight::FlightRecorder::new(
            std::num::NonZeroUsize::new(capacity).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        if self.api_observation {
            ring.enable_api_observation();
        }
        if self.object_observation {
            ring.enable_object_observation();
        }
        let mut receiver = None;
        let mut disconnect_before = None;
        if let Some(profile) = self.diagnostic_profile {
            use afsplus_core::flight::{Categories, Category, Event, SinkResult};
            use std::sync::mpsc::{sync_channel, TrySendError};
            let mut categories = Categories::NONE;
            for (bit, category) in [
                Category::Transaction,
                Category::Checkpoint,
                Category::Io,
                Category::Error,
                Category::Api,
                Category::Window,
                Category::Object,
            ]
            .into_iter()
            .enumerate()
            {
                if profile.categories & (1 << bit) != 0 {
                    categories = categories.with(category);
                }
            }
            ring.set_categories(categories);
            if profile.sink_capacity != 0 {
                let (sender, rx) = sync_channel::<Event>(profile.sink_capacity);
                ring.replace_sink(Some(Box::new(move |event| match sender.try_send(event) {
                    Ok(()) => SinkResult::Accepted,
                    Err(TrySendError::Full(_)) => SinkResult::Busy,
                    Err(TrySendError::Disconnected(_)) => SinkResult::Closed,
                })));
                receiver = Some(rx);
                disconnect_before = profile.disconnect_before;
            }
        }
        self.run_observed(limits, Some(ring), receiver, disconnect_before)
    }

    fn run_observed(
        &self,
        limits: RecordingLimits,
        flight: Option<afsplus_core::flight::FlightRecorder>,
        mut receiver: Option<std::sync::mpsc::Receiver<afsplus_core::flight::Event>>,
        disconnect_before: Option<usize>,
    ) -> Result<Run, String> {
        let mut base = MemoryBackend::new(4096, self.blocks);
        mkfs_with_options(
            &mut base,
            &MkfsParams {
                uuid: [0x53; 16],
                label: "Scenario".into(),
                region_size: self.region,
                reclaim_caps: Default::default(),
                log_slots: self.log_slots,
                shared_extents: true,
                data_policy: self.data_policy,
                name_policy: NamePolicy::Sensitive,
                timestamp: Timespec::default(),
            },
            MkfsOptions {
                persistent_snapshots: self.snapshot_limits.is_some(),
            },
        )
        .map_err(|e| e.to_string())?;
        let recorder = Recorder(Rc::new(RefCell::new(Capture {
            image: base.clone(),
            log: Vec::new(),
            bytes: 0,
            limits,
        })));
        let mut volume = match self.mount_profile(recorder.clone()) {
            Ok(volume) => volume,
            Err(error) => {
                return Ok(finish(
                    base,
                    recorder,
                    Some((0, error.to_string())),
                    Vec::new(),
                    Vec::new(),
                ))
            }
        };
        volume.replace_flight_recorder(flight);
        // id, parent id, original name, directory kind; labels never select host paths.
        let mut labels = BTreeMap::from([(
            "root".to_owned(),
            (OBJECT_ROOT, OBJECT_ROOT, String::new(), true),
        )]);
        let mut used = std::collections::BTreeSet::from(["root".to_owned()]);
        let mut snapshot_ids = BTreeMap::<String, u64>::new();
        let mut snapshot_handles = BTreeMap::<String, SnapshotHandle>::new();
        // Labels whose object left the visible namespace into object 2.
        let mut orphans = BTreeMap::<String, u64>::new();
        let mut orphan_candidates = Vec::<u64>::new();
        let mut failure = None;
        let mut events = Vec::with_capacity(self.operations.len());
        for (index, op) in self.operations.iter().enumerate() {
            // No concurrent consumer: service/disconnection is a deterministic
            // scenario input, outside the filesystem operation being observed.
            if let Some(rx) = &receiver {
                while rx.try_recv().is_ok() {}
            }
            if disconnect_before == Some(index) {
                receiver = None;
            }
            let now = Timespec {
                seconds: index as i64 + 1,
                nanoseconds: 0,
            };
            let first_block_operation = recorder.0.borrow().log.len();
            let object_label = match op {
                Operation::Create { label, .. }
                | Operation::Write { label, .. }
                | Operation::Truncate { label, .. }
                | Operation::Rename { label, .. }
                | Operation::Remove { label, .. }
                | Operation::Link { label, .. }
                | Operation::Symlink { label, .. }
                | Operation::CloneFile { label, .. }
                | Operation::SetProtection { label, .. }
                | Operation::UnlinkSymlink { label }
                | Operation::RenameReplace { label, .. }
                | Operation::OrphanFile { label }
                | Operation::CleanupOrphan { label }
                | Operation::Preallocate { label, .. }
                | Operation::SetDataPolicy { label, .. }
                | Operation::RestoreMetadata { label, .. }
                | Operation::CloneRange {
                    destination: label, ..
                } => Some(label),
                _ => None,
            };
            let previous_id = object_label
                .and_then(|label| labels.get(label))
                .map(|v| v.0)
                .unwrap_or(0);
            if matches!(op, Operation::Remount) {
                let mut flight = volume.replace_flight_recorder(None);
                snapshot_handles.clear();
                drop(volume);
                volume = match self.mount_profile(recorder.clone()) {
                    Ok(volume) => volume,
                    Err(error) => {
                        events.push(Event {
                            operation: index,
                            first_block_operation,
                            end_block_operation: recorder.0.borrow().log.len(),
                            object_id: 0,
                            success: false,
                            flight: flight.as_mut().map(drain_flight),
                        });
                        return Ok(finish(
                            base,
                            recorder,
                            Some((index, error.to_string())),
                            events,
                            orphan_candidates,
                        ));
                    }
                };
                volume.replace_flight_recorder(flight);
                events.push(Event {
                    operation: index,
                    first_block_operation,
                    end_block_operation: recorder.0.borrow().log.len(),
                    object_id: 0,
                    success: true,
                    flight: capture_flight(&mut volume),
                });
                continue;
            }
            let result = (|| -> Result<(), String> {
                let get = |l: &str| {
                    labels
                        .get(l)
                        .cloned()
                        .ok_or_else(|| format!("unknown label {l}"))
                };
                match op {
                    Operation::Snapshot { action, label } => match action {
                        SnapshotAction::Create => {
                            if snapshot_ids.contains_key(label) {
                                return Err("reused snapshot label".into());
                            }
                            let id = volume.snapshot_create(now).map_err(|e| e.to_string())?;
                            snapshot_ids.insert(label.clone(), id);
                        }
                        SnapshotAction::Open => {
                            if snapshot_handles.contains_key(label) {
                                return Err("snapshot handle already open".into());
                            }
                            let id = *snapshot_ids.get(label).ok_or("unknown snapshot label")?;
                            snapshot_handles.insert(
                                label.clone(),
                                volume.snapshot_open(id).map_err(|e| e.to_string())?,
                            );
                        }
                        SnapshotAction::Close => {
                            snapshot_handles
                                .remove(label)
                                .ok_or("snapshot handle not open")?;
                        }
                        SnapshotAction::Delete => {
                            let id = *snapshot_ids.get(label).ok_or("unknown snapshot label")?;
                            volume.snapshot_delete(id, now).map_err(|e| e.to_string())?;
                        }
                        SnapshotAction::Inspect => {
                            let handle = snapshot_handles
                                .get(label)
                                .ok_or("snapshot handle not open")?;
                            captured::inspect(
                                &mut volume,
                                handle,
                                captured::Limits {
                                    entries: 1024,
                                    bytes: 16 * 1024 * 1024,
                                    ranges: 4096,
                                    page_entries: 2,
                                },
                            )?;
                        }
                    },
                    Operation::Create {
                        label,
                        parent,
                        name,
                        data,
                        directory,
                    } => {
                        if used.contains(label) {
                            return Err("reused scenario label".into());
                        }
                        let p = get(parent)?;
                        if !p.3 {
                            return Err("parent is not a directory".into());
                        }
                        let id = if *directory {
                            volume.create_directory(p.0, name, now)
                        } else {
                            volume.create_file_in_directory(p.0, name, data, now)
                        }
                        .map_err(|e| e.to_string())?;
                        labels.insert(label.clone(), (id, p.0, name.clone(), *directory));
                        used.insert(label.clone());
                    }
                    Operation::Write {
                        deferred,
                        label,
                        offset,
                        data,
                    } => {
                        let id = get(label)?.0;
                        if *deferred {
                            volume.window_write_file_at(id, *offset, data, now)
                        } else {
                            volume.write_file_at(id, *offset, data, now)
                        }
                        .map_err(|e| e.to_string())?;
                    }
                    Operation::Truncate {
                        deferred,
                        label,
                        size,
                    } => {
                        let id = get(label)?.0;
                        if *deferred {
                            volume.window_truncate_file(id, *size, now)
                        } else {
                            volume.truncate_file(id, *size, now)
                        }
                        .map_err(|e| e.to_string())?;
                    }
                    Operation::Rename {
                        label,
                        parent,
                        name,
                    } => {
                        if label == "root" {
                            return Err("reserved root label".into());
                        }
                        let old = get(label)?;
                        let p = get(parent)?;
                        volume
                            .rename(old.1, &old.2, p.0, name, now)
                            .map_err(|e| e.to_string())?;
                        labels.insert(label.clone(), (old.0, p.0, name.clone(), old.3));
                    }
                    Operation::Remove { label, directory } => {
                        if label == "root" {
                            return Err("reserved root label".into());
                        }
                        let old = get(label)?;
                        if old.3 != *directory {
                            return Err("removal kind mismatch".into());
                        }
                        if *directory {
                            volume.remove_directory(old.1, &old.2, now)
                        } else {
                            volume.delete_file(old.1, &old.2, now)
                        }
                        .map_err(|e| e.to_string())?;
                        labels.remove(label);
                    }
                    Operation::Link {
                        label,
                        source,
                        parent,
                        name,
                    } => {
                        if used.contains(label) {
                            return Err("reused scenario label".into());
                        }
                        let file = get(source)?;
                        let p = get(parent)?;
                        if !p.3 {
                            return Err("parent is not a directory".into());
                        }
                        volume
                            .link_file(file.0, p.0, name, now)
                            .map_err(|e| e.to_string())?;
                        labels.insert(label.clone(), (file.0, p.0, name.clone(), false));
                        used.insert(label.clone());
                    }
                    Operation::Symlink {
                        label,
                        parent,
                        name,
                        target,
                    } => {
                        if used.contains(label) {
                            return Err("reused scenario label".into());
                        }
                        let p = get(parent)?;
                        if !p.3 {
                            return Err("parent is not a directory".into());
                        }
                        let id = volume
                            .create_symlink(p.0, name, target, now)
                            .map_err(|e| e.to_string())?;
                        labels.insert(label.clone(), (id, p.0, name.clone(), false));
                        used.insert(label.clone());
                    }
                    Operation::CloneFile {
                        label,
                        source,
                        parent,
                        name,
                    } => {
                        if used.contains(label) {
                            return Err("reused scenario label".into());
                        }
                        let file = get(source)?;
                        let p = get(parent)?;
                        if !p.3 {
                            return Err("parent is not a directory".into());
                        }
                        let id = volume
                            .clone_file(file.0, p.0, name, now)
                            .map_err(|e| e.to_string())?;
                        labels.insert(label.clone(), (id, p.0, name.clone(), false));
                        used.insert(label.clone());
                    }
                    Operation::CloneRange {
                        source,
                        source_offset,
                        destination,
                        destination_offset,
                        length,
                    } => {
                        let source = get(source)?.0;
                        let destination = get(destination)?.0;
                        volume
                            .clone_range(
                                source,
                                *source_offset,
                                destination,
                                *destination_offset,
                                *length,
                                now,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    Operation::SetProtection { label, protection } => {
                        if label == "root" {
                            return Err("reserved root label".into());
                        }
                        volume
                            .set_object_protection(get(label)?.0, *protection, now)
                            .map_err(|e| e.to_string())?;
                    }
                    Operation::UnlinkSymlink { label } => {
                        if label == "root" {
                            return Err("reserved root label".into());
                        }
                        let old = get(label)?;
                        if old.3 {
                            return Err("removal kind mismatch".into());
                        }
                        volume
                            .unlink_symlink(old.1, &old.2, now)
                            .map_err(|e| e.to_string())?;
                        labels.remove(label);
                    }
                    Operation::RenameReplace {
                        label,
                        victim,
                        parent,
                        name,
                        orphan,
                    } => {
                        if label == "root" || victim == "root" {
                            return Err("reserved root label".into());
                        }
                        let source = get(label)?;
                        let old = get(victim)?;
                        let p = get(parent)?;
                        if !p.3 {
                            return Err("parent is not a directory".into());
                        }
                        // The victim label must name the entry being replaced.
                        if (old.1, old.2.as_str()) != (p.0, name.as_str()) {
                            return Err("victim label does not name the replaced entry".into());
                        }
                        if *orphan {
                            volume.rename_replace_orphan_target(source.1, &source.2, p.0, name, now)
                        } else {
                            volume.rename_replace(source.1, &source.2, p.0, name, now)
                        }
                        .map_err(|e| e.to_string())?;
                        labels.insert(label.clone(), (source.0, p.0, name.clone(), false));
                        labels.remove(victim);
                        if *orphan {
                            orphans.insert(victim.clone(), old.0);
                            orphan_candidates.push(old.0);
                        }
                    }
                    Operation::OrphanFile { label } => {
                        if label == "root" {
                            return Err("reserved root label".into());
                        }
                        let old = get(label)?;
                        if old.3 {
                            return Err("removal kind mismatch".into());
                        }
                        let id = volume
                            .orphan_file(old.1, &old.2, now)
                            .map_err(|e| e.to_string())?;
                        labels.remove(label);
                        orphans.insert(label.clone(), id);
                        orphan_candidates.push(id);
                    }
                    Operation::CleanupOrphan { label } => {
                        let id = *orphans.get(label).ok_or("unknown orphan label")?;
                        let progress = volume.cleanup_orphan(id, now).map_err(|e| e.to_string())?;
                        if progress.object_removed {
                            orphans.remove(label);
                        }
                    }
                    Operation::Preallocate {
                        label,
                        offset,
                        length,
                        limits,
                    } => {
                        let id = get(label)?.0;
                        match limits {
                            Some((max_blocks, max_records)) => volume.preallocate_file_bounded(
                                id,
                                *offset,
                                *length,
                                now,
                                FileEditLimits {
                                    max_blocks: *max_blocks,
                                    max_records: *max_records,
                                },
                            ),
                            None => volume.preallocate_file(id, *offset, *length, now),
                        }
                        .map_err(|e| e.to_string())?;
                    }
                    Operation::SetDataPolicy { label, in_place } => {
                        let id = get(label)?.0;
                        let policy = if *in_place {
                            DataUpdatePolicy::InPlacePrivate
                        } else {
                            DataUpdatePolicy::FullCow
                        };
                        volume
                            .set_file_data_policy(id, policy, now)
                            .map_err(|e| e.to_string())?;
                    }
                    Operation::RestoreMetadata {
                        label,
                        protection,
                        created,
                        modified,
                        changed,
                    } => {
                        if label == "root" {
                            return Err("reserved root label".into());
                        }
                        let stamp = |seconds: i64| Timespec {
                            seconds,
                            nanoseconds: 0,
                        };
                        volume
                            .restore_object_metadata(
                                get(label)?.0,
                                PreservedMetadata {
                                    protection: *protection,
                                    created: stamp(*created),
                                    modified: stamp(*modified),
                                    changed: stamp(*changed),
                                },
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    Operation::ReclaimStep => {
                        volume.reclaim_step(now).map_err(|e| e.to_string())?;
                    }
                    Operation::SnapshotMaintenanceStep => {
                        volume
                            .snapshot_maintenance_step(now)
                            .map_err(|e| e.to_string())?;
                    }
                    Operation::Batch { items, deferred } => {
                        // Resolve every label first: a refused batch changes nothing.
                        let mut resolved = Vec::new();
                        for item in items {
                            resolved.push(match item {
                                BatchItem::Create {
                                    label,
                                    parent,
                                    name,
                                    ..
                                } => {
                                    if used.contains(label) {
                                        return Err("reused scenario label".into());
                                    }
                                    let p = get(parent)?;
                                    if !p.3 {
                                        return Err("parent is not a directory".into());
                                    }
                                    (p.0, name.clone(), 0, String::new(), 0, 0)
                                }
                                BatchItem::Delete { label } => {
                                    let old = get(label)?;
                                    if old.3 {
                                        return Err("removal kind mismatch".into());
                                    }
                                    (old.1, old.2.clone(), 0, String::new(), old.0, 0)
                                }
                                BatchItem::Rename {
                                    label,
                                    parent,
                                    name,
                                    victim,
                                } => {
                                    let old = get(label)?;
                                    let p = get(parent)?;
                                    if !p.3 {
                                        return Err("parent is not a directory".into());
                                    }
                                    let mut replaced = 0;
                                    if let Some(target) = victim {
                                        let target = get(target)?;
                                        if (target.1, target.2.as_str()) != (p.0, name.as_str()) {
                                            return Err(
                                                "victim label does not name the replaced entry"
                                                    .into(),
                                            );
                                        }
                                        replaced = target.0;
                                    }
                                    (old.1, old.2.clone(), p.0, name.clone(), old.0, replaced)
                                }
                            });
                        }
                        let ops: Vec<BatchOp<'_>> = items
                            .iter()
                            .zip(&resolved)
                            .map(|(item, entry)| match item {
                                BatchItem::Create { data, .. } => BatchOp::CreateFile {
                                    parent_id: entry.0,
                                    name: &entry.1,
                                    content: data,
                                },
                                BatchItem::Delete { .. } => BatchOp::DeleteFile {
                                    parent_id: entry.0,
                                    name: &entry.1,
                                },
                                BatchItem::Rename { victim, .. } => BatchOp::Rename {
                                    source_parent_id: entry.0,
                                    source_name: &entry.1,
                                    target_parent_id: entry.2,
                                    target_name: &entry.3,
                                    replace: victim.is_some(),
                                },
                            })
                            .collect();
                        let results = if *deferred {
                            let mut staged = Vec::new();
                            for op in &ops {
                                staged.push(volume.window_op(op, now).map_err(|e| e.to_string())?);
                            }
                            staged
                        } else {
                            volume.run_batch(&ops, now).map_err(|e| e.to_string())?
                        };
                        drop(ops);
                        // Every label was resolved before execution, so the
                        // bookkeeping below reads no further committed state.
                        for (index, (item, entry)) in items.iter().zip(&resolved).enumerate() {
                            match item {
                                BatchItem::Create { label, .. } => {
                                    let id = *results
                                        .get(index)
                                        .and_then(|result| result.as_ref())
                                        .ok_or("batch create produced no object")?;
                                    labels.insert(
                                        label.clone(),
                                        (id, entry.0, entry.1.clone(), false),
                                    );
                                    used.insert(label.clone());
                                }
                                BatchItem::Delete { label } => {
                                    labels.remove(label);
                                    if *deferred {
                                        orphans.insert(label.clone(), entry.4);
                                        orphan_candidates.push(entry.4);
                                    }
                                }
                                BatchItem::Rename { label, victim, .. } => {
                                    labels.insert(
                                        label.clone(),
                                        (entry.4, entry.2, entry.3.clone(), false),
                                    );
                                    if let Some(target) = victim {
                                        labels.remove(target);
                                        // A window unlinks the final link into the
                                        // reserved directory; a batch destroys it.
                                        if *deferred {
                                            orphans.insert(target.clone(), entry.5);
                                            orphan_candidates.push(entry.5);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Operation::WindowFsync => volume.window_fsync().map_err(|e| e.to_string())?,
                    Operation::WindowCommit => {
                        volume.window_commit(now).map_err(|e| e.to_string())?
                    }
                    Operation::Sync => volume.sync().map_err(|e| e.to_string())?,
                    Operation::Remount => unreachable!(),
                }
                Ok(())
            })();
            events.push(Event {
                operation: index,
                first_block_operation,
                end_block_operation: recorder.0.borrow().log.len(),
                object_id: object_label
                    .and_then(|label| labels.get(label))
                    .map(|v| v.0)
                    .unwrap_or(previous_id),
                success: result.is_ok(),
                flight: capture_flight(&mut volume),
            });
            if let Err(error) = result {
                failure = Some((index, error));
                break;
            }
        }
        drop(volume);
        Ok(finish(base, recorder, failure, events, orphan_candidates))
    }
}

/// Exact namespace/content observation for the admitted file/directory profile.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: Vec<String>,
    pub data: Option<Vec<u8>>,
}

/// Inspect a private image copy, with explicit aggregate output limits.
/// Unsupported object kinds and cycles are errors, never omitted entries.
pub fn inspect(image: MemoryBackend, max_bytes: usize) -> Result<Vec<Entry>, String> {
    let mut volume = mount(image).map_err(|e| e.to_string())?;
    inspect_volume(&mut volume, max_bytes)
}

/// Object kinds admitted by the version-9 linked namespace observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedKind {
    File,
    Directory,
    Symlink,
}

/// One path of the linked profile. Several paths may name one file object.
#[derive(Debug, PartialEq, Eq)]
pub struct LinkedEntry {
    pub path: Vec<String>,
    pub kind: LinkedKind,
    pub object_id: u64,
    pub link_count: u64,
    pub protection: u32,
    /// File contents or opaque symlink target; empty for a directory.
    pub data: Vec<u8>,
    /// Persistent per-file data-update policy; false for every other kind.
    pub in_place: bool,
    /// Normalized logical allocation coverage of a file: adjacent ranges with
    /// an equal unwritten flag merged into maximal logical intervals.
    pub allocation: Vec<(u64, u64, bool)>,
}

/// Merge adjacent ranges with an equal unwritten flag. The result is
/// layout independent: it depends on which logical bytes are reserved and
/// whether they read as zeros, never on extent record boundaries.
pub fn normalize_allocation(
    ranges: &[afsplus_core::volume::FileAllocationRange],
) -> Result<Vec<(u64, u64, bool)>, String> {
    let mut merged: Vec<(u64, u64, bool)> = Vec::new();
    for range in ranges {
        let end = range
            .offset
            .checked_add(range.length)
            .ok_or("scenario allocation range overflow")?;
        match merged.last_mut() {
            Some(last) if last.0 + last.1 == range.offset && last.2 == range.unwritten => {
                last.1 = end - last.0;
            }
            Some(last) if last.0 + last.1 > range.offset => {
                return Err("scenario allocation ranges overlap or regress".into());
            }
            _ => merged.push((range.offset, range.length, range.unwritten)),
        }
    }
    Ok(merged)
}

pub struct Inspection {
    /// Effective policy of the successfully mounted observation volume.
    pub cache_pages: Option<usize>,
    pub raw: crate::CheckReport,
    pub recovered: Option<crate::CheckReport>,
    /// File/directory projection. The linked profile refuses this projection
    /// explicitly and reports its namespace through `linked`.
    pub entries: Result<Vec<Entry>, String>,
    pub linked: Option<Result<Vec<LinkedEntry>, String>>,
    pub snapshots: Option<Result<Vec<captured::View>, String>>,
    /// Version 9: reserved-directory entry count and candidate byte total.
    pub orphans: Option<Result<(u64, u64), String>>,
}
impl Inspection {
    pub fn is_clean(&self) -> bool {
        self.raw.is_clean()
            && self
                .recovered
                .as_ref()
                .is_some_and(crate::CheckReport::is_clean)
            && match &self.linked {
                Some(linked) => linked.is_ok(),
                None => self.entries.is_ok(),
            }
            && self.snapshots.as_ref().is_none_or(|views| views.is_ok())
            && self.orphans.as_ref().is_none_or(Result::is_ok)
    }
}

/// Full offline checks before and after recovery of an owned memory image.
/// The recovered checker covers the exact volume used for namespace observation.
pub fn inspect_checked(image: MemoryBackend, max_bytes: usize) -> Inspection {
    inspect_checked_with_options(image, max_bytes, MountOptions::default(), None, false, &[])
}

fn inspect_checked_with_options(
    mut image: MemoryBackend,
    max_bytes: usize,
    options: MountOptions,
    snapshot_limits: Option<SnapshotWorkLimits>,
    linked_profile: bool,
    orphan_candidates: &[u64],
) -> Inspection {
    let raw = crate::check_device(&mut image);
    let mounted = match snapshot_limits {
        Some(limits) => mount_with_snapshot_limits(image, options, limits),
        None => mount_with_options(image, options),
    };
    match mounted {
        Err(error) => Inspection {
            cache_pages: None,
            raw,
            recovered: None,
            entries: Err(error.to_string()),
            linked: linked_profile.then(|| Err(error.to_string())),
            snapshots: snapshot_limits.map(|_| Err(error.to_string())),
            orphans: linked_profile.then(|| Err(error.to_string())),
        },
        Ok(mut volume) => {
            let cache_pages = Some(volume.tree_cache_pages());
            let (entries, linked) = if linked_profile {
                (
                    Err("linked namespace observation profile".into()),
                    Some(inspect_linked(&mut volume, max_bytes)),
                )
            } else {
                (inspect_volume(&mut volume, max_bytes), None)
            };
            let snapshots = snapshot_limits.map(|limits| {
                captured::inspect_all(
                    &mut volume,
                    limits.max_views,
                    captured::Limits {
                        entries: 1024,
                        bytes: max_bytes,
                        ranges: 4096,
                        page_entries: 2,
                    },
                )
            });
            let orphans = linked_profile.then(|| observe_orphans(&mut volume, orphan_candidates));
            let recovered = Some(crate::check_device(&mut volume.into_device()));
            Inspection {
                cache_pages,
                raw,
                recovered,
                entries,
                linked,
                snapshots,
                orphans,
            }
        }
    }
}

/// Page the complete committed allocation of one file under explicit bounds.
fn read_allocation(
    volume: &mut afsplus_core::Volume<MemoryBackend>,
    object_id: u64,
) -> Result<Vec<afsplus_core::volume::FileAllocationRange>, String> {
    let mut ranges = Vec::new();
    let mut cursor = 0u64;
    loop {
        let page = volume
            .file_allocation_page(object_id, cursor, 8)
            .map_err(|e| e.to_string())?;
        if ranges.len() + page.ranges.len() > 4096 {
            return Err("scenario allocation observation limit".into());
        }
        let eof = page.eof;
        if !eof && (page.ranges.is_empty() || page.next <= cursor) {
            return Err("scenario allocation made no progress".into());
        }
        cursor = page.next;
        ranges.extend(page.ranges);
        if eof {
            return Ok(ranges);
        }
    }
}

/// Count the reserved directory and sum the sizes of the candidate objects it
/// still names. Candidates come from the executed run, never from a scan.
fn observe_orphans(
    volume: &mut afsplus_core::Volume<MemoryBackend>,
    candidates: &[u64],
) -> Result<(u64, u64), String> {
    let count = volume.orphan_count().map_err(|e| e.to_string())?;
    let mut bytes = 0u64;
    let mut named = 0u64;
    let mut seen = std::collections::BTreeSet::new();
    for id in candidates {
        if !seen.insert(*id) || !volume.orphan_object(*id).map_err(|e| e.to_string())? {
            continue;
        }
        named += 1;
        let record = volume
            .stat(*id)
            .map_err(|e| e.to_string())?
            .ok_or("orphan candidate missing from the object map")?;
        bytes = bytes
            .checked_add(record.size_bytes)
            .ok_or("orphan byte total overflow")?;
    }
    // Every orphan of a generated case comes from one of its own operations,
    // so an entry outside the candidate set is an unbindable observation.
    if count != named {
        return Err("reserved directory names an object outside the scenario".into());
    }
    Ok((count, bytes))
}

/// Linked observation: repeated file objects are hard-link aliases; a repeated
/// directory, an internal object or an exhausted budget is an error.
fn inspect_linked(
    volume: &mut afsplus_core::Volume<MemoryBackend>,
    max_bytes: usize,
) -> Result<Vec<LinkedEntry>, String> {
    use afsplus_format::object::ObjectType;
    let mut queue = std::collections::VecDeque::from([(OBJECT_ROOT, Vec::<String>::new())]);
    let mut directories = std::collections::BTreeSet::from([OBJECT_ROOT]);
    let mut entries = Vec::new();
    let mut bytes = 0usize;
    while let Some((directory, path)) = queue.pop_front() {
        let children = volume
            .list_directory(directory)
            .map_err(|e| e.to_string())?;
        for (name, id) in children {
            if entries.len() >= 1024 || path.len() >= 64 {
                return Err("scenario namespace admission".into());
            }
            let mut child = path.clone();
            child.push(name);
            let record = volume
                .stat(id)
                .map_err(|e| e.to_string())?
                .ok_or("scenario missing object")?;
            let size = usize::try_from(record.size_bytes).map_err(|_| "scenario object size")?;
            let mut in_place = false;
            let mut allocation = Vec::new();
            let (kind, data) = match record.object_type {
                ObjectType::Directory => {
                    if !directories.insert(id) {
                        return Err("scenario repeated directory".into());
                    }
                    queue.push_back((id, child.clone()));
                    (LinkedKind::Directory, Vec::new())
                }
                ObjectType::File | ObjectType::Symlink => {
                    if size > max_bytes.saturating_sub(bytes) {
                        return Err("scenario observation byte limit".into());
                    }
                    bytes += size;
                    if record.object_type == ObjectType::File {
                        let data = volume.read_file(id).map_err(|e| e.to_string())?;
                        in_place = volume.file_data_policy(id).map_err(|e| e.to_string())?
                            == DataUpdatePolicy::InPlacePrivate;
                        allocation = normalize_allocation(&read_allocation(volume, id)?)?;
                        (LinkedKind::File, data)
                    } else {
                        let mut target = vec![0; size];
                        if volume
                            .read_link(id, &mut target)
                            .map_err(|e| e.to_string())?
                            != size
                        {
                            return Err("scenario symlink length".into());
                        }
                        (LinkedKind::Symlink, target)
                    }
                }
                _ => return Err("unsupported scenario object kind".into()),
            };
            entries.push(LinkedEntry {
                path: child,
                kind,
                object_id: id,
                link_count: u64::from(record.link_count),
                protection: record.protection,
                data,
                in_place,
                allocation,
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn inspect_volume(
    volume: &mut afsplus_core::Volume<MemoryBackend>,
    max_bytes: usize,
) -> Result<Vec<Entry>, String> {
    use afsplus_format::object::ObjectType;
    let mut queue = std::collections::VecDeque::from([(OBJECT_ROOT, Vec::<String>::new())]);
    let mut visited = std::collections::BTreeSet::from([OBJECT_ROOT]);
    let mut entries = Vec::new();
    let mut bytes = 0usize;
    while let Some((directory, path)) = queue.pop_front() {
        let children = volume
            .list_directory(directory)
            .map_err(|e| e.to_string())?;
        for (name, id) in children {
            if entries.len() >= 1024 || path.len() >= 64 || !visited.insert(id) {
                return Err("scenario namespace admission or repeated object".into());
            }
            let mut child = path.clone();
            child.push(name);
            let record = volume
                .stat(id)
                .map_err(|e| e.to_string())?
                .ok_or("scenario missing object")?;
            let data = match record.object_type {
                ObjectType::Directory => {
                    queue.push_back((id, child.clone()));
                    None
                }
                ObjectType::File => {
                    let size =
                        usize::try_from(record.size_bytes).map_err(|_| "scenario file size")?;
                    if size > max_bytes.saturating_sub(bytes) {
                        return Err("scenario observation byte limit".into());
                    }
                    bytes += size;
                    Some(volume.read_file(id).map_err(|e| e.to_string())?)
                }
                _ => return Err("unsupported scenario object kind".into()),
            };
            entries.push(Entry { path: child, data });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}
