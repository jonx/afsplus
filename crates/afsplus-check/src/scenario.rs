//! Experimental Stage A semantic runner (ADR-099), confined to memory images.
pub mod captured;
use afsplus_block::{BlockDevice, BlockError, MemoryBackend, RecordedOp};
use afsplus_core::volume::{SnapshotHandle, SnapshotWorkLimits};
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
    snapshot_limits: Option<SnapshotWorkLimits>,
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
            )
        ) {
            return Err("scenario protocol version".into());
        }
        let mut header: Vec<_> = lines.next().ok_or("missing geometry")?.split(' ').collect();
        let snapshot_limits = if version == Some("AFSPSC07") {
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
            Some("AFSPSC04" | "AFSPSC05" | "AFSPSC06" | "AFSPSC07")
        ) {
            let disconnect_before = match header.pop().ok_or("missing disconnect index")? {
                "none" => None,
                value => Some(integer(value, 1024)? as usize),
            };
            let sink_capacity =
                integer(header.pop().ok_or("missing sink capacity")?, 256)? as usize;
            let maximum = match version {
                Some("AFSPSC06" | "AFSPSC07") => 127,
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
            Some("AFSPSC03" | "AFSPSC04" | "AFSPSC05" | "AFSPSC06" | "AFSPSC07")
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
            Some("AFSPSC02" | "AFSPSC03" | "AFSPSC04" | "AFSPSC05" | "AFSPSC06" | "AFSPSC07")
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
                        || matches!(version, Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07")) =>
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
                        || matches!(version, Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07")) =>
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
                    if version == Some("AFSPSC07") =>
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
                ["sync"] => Operation::Sync,
                ["window_fsync"]
                    if matches!(version, Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07")) =>
                {
                    Operation::WindowFsync
                }
                ["window_commit"]
                    if matches!(version, Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07")) =>
                {
                    Operation::WindowCommit
                }
                ["remount"] => Operation::Remount,
                _ => return Err("unknown scenario command or arity".into()),
            };
            match &op {
                Operation::Create { data, .. } => payload += data.len(),
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
            api_observation: matches!(version, Some("AFSPSC05" | "AFSPSC06" | "AFSPSC07")),
            object_observation: matches!(version, Some("AFSPSC06" | "AFSPSC07")),
            snapshot_limits,
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

    pub fn flight_capacity(&self) -> Option<usize> {
        self.flight_capacity
    }

    pub fn cache_profile(&self) -> Option<usize> {
        self.cache_profile
    }

    pub fn snapshot_limits(&self) -> Option<SnapshotWorkLimits> {
        self.snapshot_limits
    }

    fn mount_profile<D: BlockDevice>(
        &self,
        device: D,
    ) -> Result<afsplus_core::Volume<D>, afsplus_core::CoreError> {
        match self.snapshot_limits {
            Some(limits) => mount_with_snapshot_limits(device, self.mount_options(), limits),
            None => mount_with_options(device, self.mount_options()),
        }
    }

    fn mount_options(&self) -> MountOptions {
        MountOptions {
            tree_cache_pages: self.cache_profile.and_then(std::num::NonZeroUsize::new),
            ..Default::default()
        }
    }

    /// Observe/recover a selected result under the same policy as execution.
    pub fn inspect_checked(&self, image: MemoryBackend, max_bytes: usize) -> Inspection {
        inspect_checked_with_options(image, max_bytes, self.mount_options(), self.snapshot_limits)
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
                data_policy: false,
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
                | Operation::Remove { label, .. } => Some(label),
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
        Ok(finish(base, recorder, failure, events))
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

pub struct Inspection {
    /// Effective policy of the successfully mounted observation volume.
    pub cache_pages: Option<usize>,
    pub raw: crate::CheckReport,
    pub recovered: Option<crate::CheckReport>,
    pub entries: Result<Vec<Entry>, String>,
    pub snapshots: Option<Result<Vec<captured::View>, String>>,
}
impl Inspection {
    pub fn is_clean(&self) -> bool {
        self.raw.is_clean()
            && self
                .recovered
                .as_ref()
                .is_some_and(crate::CheckReport::is_clean)
            && self.entries.is_ok()
            && self.snapshots.as_ref().is_none_or(|views| views.is_ok())
    }
}

/// Full offline checks before and after recovery of an owned memory image.
/// The recovered checker covers the exact volume used for namespace observation.
pub fn inspect_checked(image: MemoryBackend, max_bytes: usize) -> Inspection {
    inspect_checked_with_options(image, max_bytes, MountOptions::default(), None)
}

fn inspect_checked_with_options(
    mut image: MemoryBackend,
    max_bytes: usize,
    options: MountOptions,
    snapshot_limits: Option<SnapshotWorkLimits>,
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
            snapshots: snapshot_limits.map(|_| Err(error.to_string())),
        },
        Ok(mut volume) => {
            let cache_pages = Some(volume.tree_cache_pages());
            let entries = inspect_volume(&mut volume, max_bytes);
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
            let recovered = Some(crate::check_device(&mut volume.into_device()));
            Inspection {
                cache_pages,
                raw,
                recovered,
                entries,
                snapshots,
            }
        }
    }
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
