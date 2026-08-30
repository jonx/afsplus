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
use afsplus_format::ident::{Identification, INCOMPAT_INTENT_LOG};
use afsplus_format::{FormatError, DEFAULT_BLOCK_SIZE};

use crate::verify::load_mount_state;
use crate::volume::Volume;
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
}

pub const SUPPORTED_INCOMPAT_FEATURES: u64 = INCOMPAT_INTENT_LOG;
pub const SUPPORTED_RO_COMPAT_FEATURES: u64 = 0;

fn negotiate_features(ident: &Identification, mode: MountMode) -> Result<(), CoreError> {
    let unknown_incompat = ident.features.incompat & !SUPPORTED_INCOMPAT_FEATURES;
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

    match (candidates[0].take(), candidates[1].take()) {
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
    }
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

fn valid_checkpoint_status(mut generation: u64) -> String {
    const PREFIX: &str = "valid, generation ";
    let mut status = String::with_capacity(PREFIX.len() + 20);
    status.push_str(PREFIX);
    if generation == 0 {
        status.push('0');
        return status;
    }

    let mut reversed = [0u8; 20];
    let mut length = 0;
    while generation != 0 {
        reversed[length] = (generation % 10) as u8;
        length += 1;
        generation /= 10;
    }
    while length != 0 {
        length -= 1;
        status.push(char::from(b'0' + reversed[length]));
    }
    status
}

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

pub fn mount<D: BlockDevice>(dev: D) -> Result<Volume<D>, CoreError> {
    mount_with_options(dev, MountOptions::default())
}

pub fn mount_with_options<D: BlockDevice>(
    mut dev: D,
    options: MountOptions,
) -> Result<Volume<D>, CoreError> {
    if dev.block_size() != DEFAULT_BLOCK_SIZE {
        return Err(CoreError::UnsupportedGeometry(
            "prototype supports only 4 KiB blocks",
        ));
    }

    let mut buf = vec![0u8; dev.block_size()];
    dev.read_block(layout::IDENT_LBA, &mut buf)?;
    let ident = Identification::decode(&buf)?;
    negotiate_features(&ident, options.mode)?;
    if ident.total_blocks > dev.total_blocks() {
        return Err(CoreError::Corrupt(format!(
            "identification declares {} blocks but device has {}",
            ident.total_blocks,
            dev.total_blocks()
        )));
    }

    let selection = select_checkpoint(&mut dev, &ident)?;
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
    if options.mode.writes_during_mount() {
        volume.recover_intent_log()?;
    } else {
        volume.inspect_intent_log()?;
    }
    Ok(volume)
}

#[cfg(test)]
mod tests {
    use afsplus_format::FormatError;

    use super::{checkpoint_error_status, valid_checkpoint_status};

    #[test]
    fn checkpoint_status_formats_full_u64_range_without_fmt() {
        assert_eq!(valid_checkpoint_status(0), "valid, generation 0");
        assert_eq!(valid_checkpoint_status(42), "valid, generation 42");
        assert_eq!(
            valid_checkpoint_status(u64::MAX),
            "valid, generation 18446744073709551615"
        );
    }

    #[test]
    fn checkpoint_error_status_is_bounded_without_generic_formatting() {
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
            "invalid: wrong block type"
        );
    }
}
