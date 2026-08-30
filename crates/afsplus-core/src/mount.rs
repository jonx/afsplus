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
use afsplus_format::DEFAULT_BLOCK_SIZE;

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
    let mut candidates: Vec<(usize, Checkpoint)> = Vec::new();
    let mut slot_status = [String::new(), String::new()];

    for (slot, lba) in ident.checkpoint_slots.iter().enumerate() {
        dev.read_block(*lba, &mut buf)?;
        match Checkpoint::decode(&buf, &ident.uuid) {
            Ok(checkpoint) => match checkpoint.validate_structural(&geo) {
                Ok(()) => {
                    slot_status[slot] = format!("valid, generation {}", checkpoint.generation);
                    candidates.push((slot, checkpoint));
                }
                Err(e) => slot_status[slot] = format!("structurally invalid: {e}"),
            },
            Err(e) => slot_status[slot] = format!("invalid: {e}"),
        }
    }

    match candidates.len() {
        0 => Err(CoreError::NoValidCheckpoint {
            slot_a: slot_status[0].clone(),
            slot_b: slot_status[1].clone(),
        }),
        1 => {
            let (slot, chosen) = candidates.pop().unwrap();
            Ok(Selection {
                chosen,
                chosen_slot: slot,
                other: None,
                slot_status,
            })
        }
        _ => {
            let (slot_b, ckpt_b) = candidates.pop().unwrap();
            let (slot_a, ckpt_a) = candidates.pop().unwrap();
            if ckpt_a.generation == ckpt_b.generation {
                return Err(CoreError::AmbiguousCheckpoints(ckpt_a.generation));
            }
            let ((chosen_slot, chosen), (_, other)) = if ckpt_a.generation > ckpt_b.generation {
                ((slot_a, ckpt_a), (slot_b, ckpt_b))
            } else {
                ((slot_b, ckpt_b), (slot_a, ckpt_a))
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
    let state = load_mount_state(&mut dev, &ident, &selection.chosen).map_err(|e| {
        CoreError::Corrupt(format!(
            "checkpoint generation {} (slot {}) references invalid state: {e}",
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
