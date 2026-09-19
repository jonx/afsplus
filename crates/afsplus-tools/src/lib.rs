//! Official host-side AFS+ command-line tools.
//!
//! The formatter is intentionally separate from the prototype binaries in
//! `afsplus-core`. The inspectors open their image with an OS read-only file
//! descriptor and implement [`afsplus_block::BlockDevice`] only so they can
//! share the format/core validators; their write and flush methods always
//! fail closed.

mod common;
mod compat;
mod diff;
mod disk;
mod dump;
mod explain;
mod extract;
mod info;
mod mkfs;

use std::ffi::OsString;

pub use common::{EXIT_MEDIA, EXIT_OK, EXIT_USAGE_OR_IO};

/// Runs the `afsplus-disk` command: an AFS+ image in a bootable AROS GPT
/// partition, or out of one.
pub fn run_disk<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    disk::run(args)
}

/// Runs the `mkafsplus` command and returns its documented process status.
pub fn run_mkafsplus<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    mkfs::run(args)
}

/// Runs the `afsplus-info` command and returns its documented process status.
pub fn run_info<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    info::run(args)
}

/// Runs the `afsplus-dump` command and returns its documented process status.
pub fn run_dump<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    dump::run(args)
}

/// Runs the `afsplus-image-diff` command and returns its documented process
/// status. A reported difference is a normal result; an image that could
/// only be compared in part gives the media status.
pub fn run_image_diff<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    diff::run(args)
}

/// Runs the `afsplus-explain` command: what the committed state of an image
/// believes about one block, one object or one path. An answer from a walk
/// that could not decode every branch gives the media status.
pub fn run_explain<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    explain::run(args)
}

/// Extracts readable checkpoint objects into a new, separate directory.
pub fn run_extract<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    extract::run(args)
}
