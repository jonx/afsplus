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
//! COW discipline: a transaction writes only to blocks that are FREE or
//! promoted-from-quarantine in the committed allocation state, to bitmap
//! slot blocks referenced by no retained checkpoint, and to the alternate
//! checkpoint slot. No block reachable from a still-selectable checkpoint is
//! ever touched, which is what makes every crash state recover to exactly
//! the pre- or post-commit state. See `alloc` for the region allocator and
//! the retired-block quarantine.

pub mod alloc;
pub mod allocation_root;
pub mod cow_tree;
pub mod directory;
pub mod extent_map;
pub mod intent_log;
pub mod mkfs;
pub mod mount;
pub mod object_map;
pub mod reclaim;
pub mod tree;
pub mod verify;
pub mod volume;

use std::fmt;

use afsplus_block::BlockError;
use afsplus_format::FormatError;

pub use mkfs::{mkfs, MkfsParams};
pub use mount::{mount, mount_with_options, MountMode, MountOptions};
pub use volume::Volume;

/// Fixed prototype placement (`spec/disk-layout.md` marks exact offsets TBD;
/// these are prototype constants, not frozen format commitments). Region
/// region-descriptor and bitmap-slot placement lives in
/// `afsplus_format::geometry`.
pub mod layout {
    /// Identification block, immutable after mkfs.
    pub const IDENT_LBA: u64 = 0;
    /// Checkpoint slot A.
    pub const CKPT_SLOT_A: u64 = 1;
    /// Checkpoint slot B.
    pub const CKPT_SLOT_B: u64 = 2;
    /// Smallest volume the prototype will format.
    pub const MIN_TOTAL_BLOCKS: u64 = 16;
}

#[derive(Debug)]
pub enum CoreError {
    Block(BlockError),
    Format(FormatError),
    /// Neither checkpoint slot is structurally valid.
    NoValidCheckpoint {
        slot_a: String,
        slot_b: String,
    },
    /// Both slots carry a structurally valid checkpoint with the same
    /// generation — a state no correct commit sequence can produce.
    AmbiguousCheckpoints(u64),
    /// A structure referenced by the chosen checkpoint failed validation.
    Corrupt(String),
    /// Name already present in the directory.
    AlreadyExists,
    /// Name not present in the directory.
    NotFound,
    /// An operation expected a directory object.
    NotDirectory,
    /// An operation requiring a regular file was given a directory.
    IsDirectory,
    /// A directory can only be removed when it has no entries.
    DirectoryNotEmpty,
    /// A namespace move would create a cycle or otherwise violate topology.
    InvalidMove(&'static str),
    /// Component name rejected by format rules.
    InvalidName(FormatError),
    /// The volume ran out of blocks (the bootstrap allocator never reuses).
    NoSpace,
    /// A documented prototype limitation was hit (never silent misbehavior).
    PrototypeLimit(&'static str),
    /// Device geometry unsupported by the prototype.
    UnsupportedGeometry(&'static str),
    /// An operation window is open; immediate-commit operations are refused.
    WindowOpen,
    /// The open window failed mid-mutation; remount to recover from the log.
    WindowPoisoned,
    /// The selected mount mode does not permit filesystem mutations.
    ReadOnly,
    /// Unknown INCOMPAT bits prevent every kind of mount.
    UnsupportedIncompatFeatures(u64),
    /// Unknown RO_COMPAT bits require a read-only or NO_CHANGES mount.
    ReadOnlyRequiredFeatures(u64),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Block(e) => write!(f, "block device error: {e}"),
            CoreError::Format(e) => write!(f, "format error: {e}"),
            CoreError::NoValidCheckpoint { slot_a, slot_b } => {
                write!(
                    f,
                    "no valid checkpoint (slot A: {slot_a}; slot B: {slot_b})"
                )
            }
            CoreError::AmbiguousCheckpoints(generation) => {
                write!(
                    f,
                    "both checkpoint slots carry generation {generation}; volume is ambiguous"
                )
            }
            CoreError::Corrupt(what) => write!(f, "corrupt volume: {what}"),
            CoreError::AlreadyExists => write!(f, "name already exists"),
            CoreError::NotFound => write!(f, "name not found"),
            CoreError::NotDirectory => write!(f, "object is not a directory"),
            CoreError::IsDirectory => write!(f, "object is a directory"),
            CoreError::DirectoryNotEmpty => write!(f, "directory is not empty"),
            CoreError::InvalidMove(what) => write!(f, "invalid namespace move: {what}"),
            CoreError::InvalidName(e) => write!(f, "invalid name: {e}"),
            CoreError::NoSpace => write!(f, "no space left on volume"),
            CoreError::PrototypeLimit(what) => write!(f, "prototype limit: {what}"),
            CoreError::UnsupportedGeometry(what) => write!(f, "unsupported geometry: {what}"),
            CoreError::WindowOpen => {
                write!(f, "an operation window is open; fsync or commit it first")
            }
            CoreError::WindowPoisoned => {
                write!(
                    f,
                    "the operation window failed mid-mutation; remount to recover"
                )
            }
            CoreError::ReadOnly => write!(f, "volume is mounted read-only"),
            CoreError::UnsupportedIncompatFeatures(bits) => {
                write!(
                    f,
                    "unsupported incompatible filesystem features: {bits:#018x}"
                )
            }
            CoreError::ReadOnlyRequiredFeatures(bits) => write!(
                f,
                "filesystem features {bits:#018x} are unsupported for a writable mount"
            ),
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
