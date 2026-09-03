//! Official host-side AFS+ command-line tools.
//!
//! The formatter is intentionally separate from the prototype binaries in
//! `afsplus-core`. The inspectors open their image with an OS read-only file
//! descriptor and implement [`afsplus_block::BlockDevice`] only so they can
//! share the format/core validators; their write and flush methods always
//! fail closed.

mod common;
mod dump;
mod info;
mod mkfs;

use std::ffi::OsString;

pub use common::{EXIT_MEDIA, EXIT_OK, EXIT_USAGE_OR_IO};

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
