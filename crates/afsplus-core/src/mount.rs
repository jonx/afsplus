//! Mount: checkpoint selection, then state loading — two distinct steps.
//!
//! Selection (`select_checkpoint`) is purely structural: decode both slots,
//! validate each against the geometry *without any further I/O*, and pick
//! the newest. Two structurally valid checkpoints with the same generation
//! are ambiguous and refuse to mount — a state no correct commit sequence
//! can produce.
//!
//! Loading bounded roots comes second; corrupt root metadata is reported and
//! never causes fallback. Descendant object records and allocation pages are
//! decoded on demand, so corruption outside those roots is reported by the
//! access that encounters it and by the exhaustive checker. Recovery from
//! such a volume is repair-tool territory.

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::ident::{
    Identification, INCOMPAT_INTENT_LOG, INCOMPAT_INTENT_LOG_DATA_UPDATES,
    INCOMPAT_PERSISTENT_SNAPSHOTS, RO_COMPAT_ORPHAN_DIRECTORY, RO_COMPAT_SHARED_EXTENTS,
};
use afsplus_format::{FormatError, DEFAULT_BLOCK_SIZE};

use crate::flight::{EventKind, FlightRecorder, LifecycleContext, MountContext, MountStage};
use crate::verify::load_mount_state;
use crate::volume::{SnapshotWorkLimits, Volume};
use crate::{layout, CoreError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MountMode {
    #[default]
    ReadWrite,
    ReadOnly,
    NoChanges,
    Recovery,
}

impl MountMode {
    pub(crate) fn allows_user_writes(self) -> bool {
        self == MountMode::ReadWrite
    }

    fn writes_during_mount(self) -> bool {
        matches!(self, MountMode::ReadWrite | MountMode::Recovery)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MountOptions {
    pub mode: MountMode,
    /// Resident staged images per COW tree mutation, including intent recovery.
    /// None preserves the unlimited modern-host default. Other memory has
    /// separate budgets; this is not a total-volume heap limit.
    pub tree_cache_pages: Option<std::num::NonZeroUsize>,
}

pub const SUPPORTED_INCOMPAT_FEATURES: u64 = INCOMPAT_INTENT_LOG | INCOMPAT_INTENT_LOG_DATA_UPDATES;
pub const SUPPORTED_RO_COMPAT_FEATURES: u64 = RO_COMPAT_SHARED_EXTENTS | RO_COMPAT_ORPHAN_DIRECTORY;

fn negotiate_features(
    ident: &Identification,
    mode: MountMode,
    snapshot_limits: Option<SnapshotWorkLimits>,
) -> Result<(), CoreError> {
    let supported = SUPPORTED_INCOMPAT_FEATURES
        | if snapshot_limits.is_some() {
            INCOMPAT_PERSISTENT_SNAPSHOTS
        } else {
            0
        };
    let unknown_incompat = ident.features.incompat & !supported;
    if unknown_incompat != 0 {
        return Err(CoreError::UnsupportedIncompatFeatures(unknown_incompat));
    }
    let unknown_ro_compat = ident.features.ro_compat & !SUPPORTED_RO_COMPAT_FEATURES;
    if unknown_ro_compat != 0 && mode.writes_during_mount() {
        return Err(CoreError::ReadOnlyRequiredFeatures(unknown_ro_compat));
    }
    Ok(())
}

/// Result of structural checkpoint selection.
pub struct Selection {
    pub chosen: Checkpoint,
    /// Slot index (0 = A, 1 = B) holding the chosen checkpoint.
    pub chosen_slot: usize,
    /// The other slot's checkpoint when it is also structurally valid
    /// (needed to keep its descriptor and bitmap slots untouched by the next commit).
    pub other: Option<Checkpoint>,
    /// Human-readable status per slot, for diagnostics and the checker.
    pub slot_status: [String; 2],
}

/// Structurally selects a checkpoint. Reads exactly three blocks
/// (identification was read by the caller; slots A and B here) and never
/// walks the filesystem.
pub fn select_checkpoint<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
) -> Result<Selection, CoreError> {
    let geo = ident.geometry();
    let mut buf = vec![0u8; geo.block_size];
    let mut candidates: [Option<Checkpoint>; 2] = [None, None];
    let mut slot_status = [String::new(), String::new()];

    read_checkpoint_candidate(
        dev,
        ident,
        &geo,
        0,
        &mut buf,
        &mut candidates[0],
        &mut slot_status,
    )?;
    read_checkpoint_candidate(
        dev,
        ident,
        &geo,
        1,
        &mut buf,
        &mut candidates[1],
        &mut slot_status,
    )?;

    let selection = match (candidates[0].take(), candidates[1].take()) {
        (None, None) => Err(CoreError::NoValidCheckpoint {
            slot_a: slot_status[0].clone(),
            slot_b: slot_status[1].clone(),
        }),
        (Some(chosen), None) => Ok(Selection {
            chosen,
            chosen_slot: 0,
            other: None,
            slot_status,
        }),
        (None, Some(chosen)) => Ok(Selection {
            chosen,
            chosen_slot: 1,
            other: None,
            slot_status,
        }),
        (Some(ckpt_a), Some(ckpt_b)) => {
            if ckpt_a.generation == ckpt_b.generation {
                return Err(CoreError::AmbiguousCheckpoints(ckpt_a.generation));
            }
            let (chosen_slot, chosen, other) = if ckpt_a.generation > ckpt_b.generation {
                (0, ckpt_a, ckpt_b)
            } else {
                (1, ckpt_b, ckpt_a)
            };
            Ok(Selection {
                chosen,
                chosen_slot,
                other: Some(other),
                slot_status,
            })
        }
    }?;
    let snapshots_enabled = ident.features.incompat & INCOMPAT_PERSISTENT_SNAPSHOTS != 0;
    if selection.chosen.snapshot_roots.is_some() != snapshots_enabled {
        return Err(CoreError::Corrupt(
            "selected checkpoint snapshot extension disagrees with incompatible feature".into(),
        ));
    }
    Ok(selection)
}

fn read_checkpoint_candidate<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    geo: &afsplus_format::geometry::Geometry,
    slot: usize,
    buf: &mut [u8],
    candidate: &mut Option<Checkpoint>,
    slot_status: &mut [String; 2],
) -> Result<(), CoreError> {
    dev.read_block(ident.checkpoint_slots[slot], buf)?;
    match Checkpoint::decode(buf, &ident.uuid) {
        Ok(checkpoint) => match checkpoint.validate_structural(geo) {
            Ok(()) => {
                slot_status[slot] = valid_checkpoint_status(checkpoint.generation);
                *candidate = Some(checkpoint);
            }
            Err(error) => {
                slot_status[slot] = checkpoint_error_status("structurally invalid: ", &error)
            }
        },
        Err(error) => slot_status[slot] = checkpoint_error_status("invalid: ", &error),
    }
    Ok(())
}

#[cfg(target_arch = "m68k")]
fn valid_checkpoint_status(generation: u64) -> String {
    // Keep this diagnostic path free of 64-bit division. Plain 68000 has no
    // native long division and the experimental backend's helper path is not
    // yet qualified; hexadecimal needs only byte extraction and shifts.
    const PREFIX: &str = "valid, generation 0x";
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut status = String::with_capacity(PREFIX.len() + 16);
    status.push_str(PREFIX);
    let mut significant = false;
    for byte in generation.to_be_bytes() {
        for nibble in [byte >> 4, byte & 0x0f] {
            if nibble != 0 || significant {
                significant = true;
                status.push(char::from(HEX[nibble as usize]));
            }
        }
    }
    if !significant {
        status.push('0');
    }
    status
}

#[cfg(not(target_arch = "m68k"))]
fn valid_checkpoint_status(generation: u64) -> String {
    format!("valid, generation {generation}")
}

#[cfg(target_arch = "m68k")]
fn checkpoint_error_status(prefix: &str, error: &FormatError) -> String {
    let (category, detail) = match error {
        FormatError::WrongBufferSize { .. } => ("wrong buffer size", None),
        FormatError::WrongBlockType { .. } => ("wrong block type", None),
        FormatError::UnsupportedHeaderVersion(_) => ("unsupported header version", None),
        FormatError::ChecksumMismatch { .. } => ("checksum mismatch", None),
        FormatError::PayloadTooLarge { .. } => ("payload too large", None),
        FormatError::Invalid(detail) => ("invalid structure: ", Some(*detail)),
        FormatError::InvalidUtf8 => ("name is not valid UTF-8", None),
        FormatError::Overflow(detail) => ("structure does not fit: ", Some(*detail)),
    };
    let detail_length = detail.map_or(0, str::len);
    let mut status = String::with_capacity(prefix.len() + category.len() + detail_length);
    status.push_str(prefix);
    status.push_str(category);
    if let Some(detail) = detail {
        status.push_str(detail);
    }
    status
}

#[cfg(not(target_arch = "m68k"))]
fn checkpoint_error_status(prefix: &str, error: &FormatError) -> String {
    format!("{prefix}{error}")
}

pub fn mount<D: BlockDevice>(dev: D) -> Result<Volume<D>, CoreError> {
    mount_with_options(dev, MountOptions::default())
}

pub fn mount_with_options<D: BlockDevice>(
    dev: D,
    options: MountOptions,
) -> Result<Volume<D>, CoreError> {
    mount_configured(dev, options, None)
}

/// Explicitly enable the persistent-snapshot experiment with caller-selected
/// budgets. Limits and registered-view admission are checked before recovery
/// can write. Ordinary mount entry points continue to reject snapshot images.
/// ReadOnly/NoChanges inspect without replay; Recovery replays then forbids
/// user mutations, just as for images without snapshots.
pub fn mount_with_snapshot_limits<D: BlockDevice>(
    dev: D,
    options: MountOptions,
    limits: SnapshotWorkLimits,
) -> Result<Volume<D>, CoreError> {
    mount_configured(dev, options, Some(limits))
}

/// Observe a mount with a caller-owned recorder. The recorder is attached
/// before checkpoint selection, moves into the returned volume with its
/// identities and counters intact, and comes back with the error when the
/// mount is refused. Attachment adds no device I/O and changes no mount
/// semantics; `mount_with_options` with the same options gives the same
/// result, the same block trace and the same image.
pub fn mount_observed<D: BlockDevice>(
    dev: D,
    options: MountOptions,
    recorder: FlightRecorder,
) -> Result<Volume<D>, Box<RefusedMount>> {
    mount_observed_configured(dev, options, None, recorder)
}

/// Observe a mount of the persistent-snapshot experiment (ADR-071) with the
/// budgets `mount_with_snapshot_limits` applies.
pub fn mount_observed_with_snapshot_limits<D: BlockDevice>(
    dev: D,
    options: MountOptions,
    limits: SnapshotWorkLimits,
    recorder: FlightRecorder,
) -> Result<Volume<D>, Box<RefusedMount>> {
    mount_observed_configured(dev, options, Some(limits), recorder)
}

fn mount_observed_configured<D: BlockDevice>(
    dev: D,
    options: MountOptions,
    snapshot_limits: Option<SnapshotWorkLimits>,
    mut recorder: FlightRecorder,
) -> Result<Volume<D>, Box<RefusedMount>> {
    let mut volume =
        match mount_before_intent_log(dev, options, snapshot_limits, Some(&mut recorder)) {
            Ok(volume) => volume,
            Err(error) => return Err(Box::new(RefusedMount { error, recorder })),
        };
    volume.replace_flight_recorder(Some(recorder));
    match recover_or_inspect_intent_log(&mut volume) {
        Ok(()) => Ok(volume),
        Err(error) => {
            let recorder = volume
                .replace_flight_recorder(None)
                .expect("the recorder installed above is still attached");
            Err(Box::new(RefusedMount { error, recorder }))
        }
    }
}

/// A refused observed mount: the refusal the unobserved entry points report,
/// and the recorder holding the stages that ran before it. Boxed because a
/// recorder is larger than a mount refusal.
#[derive(Debug)]
pub struct RefusedMount {
    pub error: CoreError,
    pub recorder: FlightRecorder,
}

/// Mount stages holding the observation before a volume exists.
struct MountObserver<'a> {
    recorder: Option<&'a mut FlightRecorder>,
    mode: MountMode,
    stage: MountStage,
    generation: u64,
    slot: u8,
    other_generation: u64,
}

impl MountObserver<'_> {
    fn event(&mut self, kind: EventKind) {
        let context = MountContext {
            mode: self.mode,
            stage: self.stage,
            slot: self.slot,
            other_generation: self.other_generation,
            count: 0,
            damaged_tail: false,
        };
        if let Some(recorder) = &mut self.recorder {
            recorder.lifecycle_event(
                self.generation,
                kind,
                false,
                0,
                LifecycleContext::Mount(context),
            );
        }
    }
}

fn mount_configured<D: BlockDevice>(
    dev: D,
    options: MountOptions,
    snapshot_limits: Option<SnapshotWorkLimits>,
) -> Result<Volume<D>, CoreError> {
    let mut volume = mount_before_intent_log(dev, options, snapshot_limits, None)?;
    recover_or_inspect_intent_log(&mut volume)?;
    Ok(volume)
}

/// Replays (writable modes) or inspects (read-only modes) the intent log and
/// reports the mounted volume. Only an attached recorder observes the steps.
fn recover_or_inspect_intent_log<D: BlockDevice>(volume: &mut Volume<D>) -> Result<(), CoreError> {
    let result = if volume.mount_mode().writes_during_mount() {
        volume.recover_intent_log()
    } else {
        volume.inspect_intent_log()
    };
    let generation = volume.generation();
    match result {
        Ok(count) => {
            // Scanned prefixes carry contiguous sequences from one, so the
            // count of replayed or pending records is the last group.
            volume.flight_mount_event(EventKind::MountComplete, count, false, count, generation);
            Ok(())
        }
        Err(error) => {
            volume.flight_mount_event(EventKind::MountFailed, 0, false, 0, generation);
            Err(error)
        }
    }
}

fn mount_before_intent_log<D: BlockDevice>(
    dev: D,
    options: MountOptions,
    snapshot_limits: Option<SnapshotWorkLimits>,
    recorder: Option<&mut FlightRecorder>,
) -> Result<Volume<D>, CoreError> {
    let mut observer = MountObserver {
        recorder,
        mode: options.mode,
        stage: MountStage::Identification,
        generation: 0,
        slot: 0,
        other_generation: 0,
    };
    observer.event(EventKind::MountBegin);
    match mount_stages(&mut observer, dev, options, snapshot_limits) {
        Ok(volume) => Ok(volume),
        Err(error) => {
            observer.event(EventKind::MountFailed);
            Err(error)
        }
    }
}

fn mount_stages<D: BlockDevice>(
    observer: &mut MountObserver<'_>,
    mut dev: D,
    options: MountOptions,
    snapshot_limits: Option<SnapshotWorkLimits>,
) -> Result<Volume<D>, CoreError> {
    if dev.block_size() != DEFAULT_BLOCK_SIZE {
        return Err(CoreError::UnsupportedGeometry(
            "prototype supports only 4 KiB blocks",
        ));
    }

    let mut buf = vec![0u8; dev.block_size()];
    dev.read_block(layout::IDENT_LBA, &mut buf)?;
    let ident = Identification::decode(&buf)?;
    observer.stage = MountStage::Negotiation;
    negotiate_features(&ident, options.mode, snapshot_limits)?;
    if ident.total_blocks > dev.total_blocks() {
        return Err(CoreError::Corrupt(format!(
            "identification declares {} blocks but device has {}",
            ident.total_blocks,
            dev.total_blocks()
        )));
    }

    observer.stage = MountStage::Selection;
    let selection = select_checkpoint(&mut dev, &ident)?;
    observer.generation = selection.chosen.generation;
    observer.slot = selection.chosen_slot as u8;
    observer.other_generation = selection
        .other
        .as_ref()
        .map_or(0, |checkpoint| checkpoint.generation);
    observer.event(EventKind::MountSelected);
    observer.stage = MountStage::RootState;
    // Feature/root congruence (ADR-061): the enabled-but-unused state (bit
    // set, root zero) is legal; a root without the feature is not.
    if selection.chosen.shared_extent_root_block != 0
        && ident.features.ro_compat & RO_COMPAT_SHARED_EXTENTS == 0
    {
        return Err(CoreError::Corrupt(
            "shared-extent root present without the shared-extents feature".into(),
        ));
    }
    #[cfg(target_arch = "m68k")]
    let state = load_mount_state(&mut dev, &ident, &selection.chosen)?;
    #[cfg(not(target_arch = "m68k"))]
    let state = load_mount_state(&mut dev, &ident, &selection.chosen).map_err(|error| {
        CoreError::Corrupt(format!(
            "checkpoint generation {} (slot {}) references invalid state: {error}",
            selection.chosen.generation,
            if selection.chosen_slot == 0 { "A" } else { "B" },
        ))
    })?;

    let mut volume = Volume::new(dev, ident, selection, state, options.mode);
    observer.stage = MountStage::Configuration;
    volume.set_tree_cache_pages(
        options
            .tree_cache_pages
            .map_or(usize::MAX, |pages| pages.get()),
    )?;
    if let Some(limits) = snapshot_limits {
        volume.set_snapshot_work_limits(limits)?;
    }
    observer.stage = MountStage::IntentLog;
    Ok(volume)
}

#[cfg(test)]
mod tests {
    use afsplus_format::FormatError;

    use super::{checkpoint_error_status, valid_checkpoint_status};

    #[test]
    fn checkpoint_status_formats_full_u64_range() {
        assert_eq!(valid_checkpoint_status(0), "valid, generation 0");
        assert_eq!(valid_checkpoint_status(42), "valid, generation 42");
        assert_eq!(
            valid_checkpoint_status(u64::MAX),
            "valid, generation 18446744073709551615"
        );
    }

    #[test]
    fn checkpoint_error_status_preserves_format_diagnostics() {
        assert_eq!(
            checkpoint_error_status(
                "structurally invalid: ",
                &FormatError::Invalid("checkpoint generation is zero"),
            ),
            "structurally invalid: invalid structure: checkpoint generation is zero"
        );
        assert_eq!(
            checkpoint_error_status(
                "invalid: ",
                &FormatError::WrongBlockType {
                    expected: 1,
                    actual: 0,
                },
            ),
            "invalid: wrong block type: expected 0x00000001, got 0x00000000"
        );
    }
}
