//! Portable AFS+ core — first prototype.
//!
//! Implements steps 3–5 of the first-contributor plan
//! (`implementation/peer-review-prototype-plan.md`): the smallest mountable
//! image, checkpoint A/B selection with fallback, and the first writable
//! COW transaction (create one empty file object, commit, remount, verify).
//!
//! Commit ordering follows `docs/08-transactions-and-journal.md` §3:
//!
//! 1. write all changed metadata copy-on-write to fresh blocks
//! 2. durability barrier
//! 3. write the *alternate* checkpoint slot with generation + 1
//! 4. durability barrier, then report durable commit
//!
//! (Step "write new user data first" is vacuous for now: prototype files are
//! empty. The data-update policy is architecture blocker 1 and deliberately
//! not decided here.)
//!
//! COW discipline: a transaction only ever writes to blocks at or above the
//! committed checkpoint's `next_free_block` high-water mark, plus the
//! alternate checkpoint slot. No block reachable from the previous committed
//! checkpoint is touched, which is what makes every crash state recover to
//! exactly the pre- or post-commit state. The bump allocator never reuses
//! storage (pre-blocker-3 scaffolding; see `afsplus_format::checkpoint`).

pub mod mkfs;
pub mod mount;
pub mod verify;
pub mod volume;

use std::fmt;

use afsplus_block::BlockError;
use afsplus_format::FormatError;

pub use mkfs::{mkfs, MkfsParams};
pub use mount::mount;
pub use volume::Volume;

/// Fixed prototype placement (`spec/disk-layout.md` marks exact offsets TBD;
/// these are prototype constants, not frozen format commitments).
pub mod layout {
    /// Identification block, immutable after mkfs.
    pub const IDENT_LBA: u64 = 0;
    /// Checkpoint slot A.
    pub const CKPT_SLOT_A: u64 = 1;
    /// Checkpoint slot B.
    pub const CKPT_SLOT_B: u64 = 2;
    /// First copy-on-write metadata block.
    pub const METADATA_START: u64 = 3;
    /// Smallest volume the prototype will format.
    pub const MIN_TOTAL_BLOCKS: u64 = 8;
}

#[derive(Debug)]
pub enum CoreError {
    Block(BlockError),
    Format(FormatError),
    /// Neither checkpoint slot yields a valid, fully validated state.
    NoValidCheckpoint {
        slot_a: String,
        slot_b: String,
    },
    /// A structure referenced by the chosen checkpoint failed validation.
    Corrupt(String),
    /// Name already present in the directory.
    AlreadyExists,
    /// Component name rejected by format rules.
    InvalidName(FormatError),
    /// The volume ran out of blocks (the bootstrap allocator never reuses).
    NoSpace,
    /// A documented prototype limitation was hit (never silent misbehavior).
    PrototypeLimit(&'static str),
    /// Device geometry unsupported by the prototype.
    UnsupportedGeometry(&'static str),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Block(e) => write!(f, "block device error: {e}"),
            CoreError::Format(e) => write!(f, "format error: {e}"),
            CoreError::NoValidCheckpoint { slot_a, slot_b } => {
                write!(f, "no valid checkpoint (slot A: {slot_a}; slot B: {slot_b})")
            }
            CoreError::Corrupt(what) => write!(f, "corrupt volume: {what}"),
            CoreError::AlreadyExists => write!(f, "name already exists"),
            CoreError::InvalidName(e) => write!(f, "invalid name: {e}"),
            CoreError::NoSpace => write!(f, "no space left on volume"),
            CoreError::PrototypeLimit(what) => write!(f, "prototype limit: {what}"),
            CoreError::UnsupportedGeometry(what) => write!(f, "unsupported geometry: {what}"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<BlockError> for CoreError {
    fn from(e: BlockError) -> Self {
        CoreError::Block(e)
    }
}

impl From<FormatError> for CoreError {
    fn from(e: FormatError) -> Self {
        CoreError::Format(e)
    }
}
