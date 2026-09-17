//! The mount diagnostics are tested the way the defect was found: mount a
//! volume, do what a person does, and require the diagnostic to NAME the
//! refusal. Nothing here inspects the recorder directly; the assertions read
//! the same report a person reads.

use std::sync::Arc;

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountMode, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::diagnostics::{self, Diagnostics};
use afsplus_fuse::{FuseAdapter, FuseConfig};
use afsplus_vfs::{AccessMode, Vfs, VfsError};

const BLOCK_SIZE: usize = 4096;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

/// A mounted volume with the diagnostics installed, as `afsplus-mount` does.
fn mounted(mode: MountMode) -> (FuseAdapter<MemoryBackend>, Arc<Diagnostics>) {
    mounted_with_diagnostics(mode, true)
}

fn mounted_with_diagnostics(
    mode: MountMode,
    observe: bool,
) -> (FuseAdapter<MemoryBackend>, Arc<Diagnostics>) {
    let mut device = MemoryBackend::new(BLOCK_SIZE, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xD1; 16],
            label: "Playground".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: timestamp(0),
        },
    )
    .unwrap();
    let mut vfs = Vfs::mount(
        device,
        MountOptions {
            mode,
            ..Default::default()
        },
    )
    .unwrap();
    let shared = diagnostics::install(&mut vfs).unwrap();
    if !observe {
        // The negative control removes the recorder the mount installs, and
        // nothing else.
        vfs.replace_flight_recorder(None);
    }
    let adapter = FuseAdapter::new(
        vfs,
        FuseConfig {
            uid: 501,
            gid: 20,
            ..FuseConfig::default()
        },
    );
    (adapter, shared)
}

/// Writing to a read-only volume is refused by the core, which is one of the
/// cases a person meets and is told nothing about.
fn write_to_a_read_only_volume(adapter: &mut FuseAdapter<MemoryBackend>) -> VfsError {
    adapter
        .create_file(
            OBJECT_ROOT,
            b"note.txt",
            AccessMode::ReadWrite,
            None,
            timestamp(1),
        )
        .map(|_| ())
        .expect_err("a read-only volume refuses a new file")
}

#[test]
fn the_report_names_the_call_the_core_refused() {
    let (mut adapter, shared) = mounted(MountMode::ReadOnly);

    let before = shared.report();
    assert!(
        before.contains("no refused call recorded"),
        "a mount that has refused nothing must not report a refusal: {before}"
    );

    assert_eq!(
        write_to_a_read_only_volume(&mut adapter),
        VfsError::ReadOnly
    );

    let after = shared.report();
    assert!(
        after.contains("CreateFileInDirectory"),
        "the report must NAME the call the core refused, which is the fact the \
         driver has and used to discard; got: {after}"
    );
    assert!(
        shared.counters().api_failed() >= 1,
        "the refusal is counted: {after}"
    );
    assert_eq!(
        shared.counters().missed(),
        0,
        "no detail was missed in a single-threaded run: {after}"
    );
}

/// The negative control. With the recorder removed the driver behaves
/// identically and the report says nothing, which proves the report is made of
/// observation and not of the test's own knowledge of what it just did.
#[test]
fn without_the_recorder_the_same_refusal_is_reported_by_nothing() {
    let (mut adapter, shared) = mounted_with_diagnostics(MountMode::ReadOnly, false);

    assert_eq!(
        write_to_a_read_only_volume(&mut adapter),
        VfsError::ReadOnly,
        "the refusal still happens; only the observation is gone"
    );

    let report = shared.report();
    assert!(
        !report.contains("CreateFileInDirectory"),
        "without a recorder there is nothing to name the refusal: {report}"
    );
    assert_eq!(
        shared.counters().api_failed(),
        0,
        "an uninstalled recorder counts nothing: {report}"
    );
}

/// A lookup that finds nothing is an answer, not a refusal. Were it counted,
/// every ordinary `ls` of a missing name would fill the report with alarms and
/// teach the reader to ignore it.
#[test]
fn a_name_that_does_not_exist_is_not_reported_as_a_refusal() {
    let (mut adapter, shared) = mounted(MountMode::ReadWrite);

    assert_eq!(
        adapter.lookup(OBJECT_ROOT, b"absent.txt").map(|_| ()),
        Err(VfsError::NotFound)
    );

    let report = shared.report();
    assert!(
        report.contains("no refused call recorded"),
        "an absent name is not a refusal: {report}"
    );
    assert_eq!(shared.counters().api_failed(), 0, "{report}");
}

/// The sequence that made the owner's volume unusable, which origin/main
/// e8228de fixed: list the root, write a file, list again. The listing now
/// SUCCEEDS, and the core still refuses the stale cursor once underneath
/// before the driver resumes the walk by name. The report must show that
/// refusal as what it is — refused by the core, recovered by the driver —
/// and must not present it as a failure the person suffered.
#[test]
fn a_recovered_refusal_is_reported_as_recovered_and_not_as_a_failure() {
    let (mut adapter, shared) = mounted(MountMode::ReadWrite);

    let handle = adapter.open_directory(OBJECT_ROOT).unwrap();
    let first = adapter
        .read_directory(OBJECT_ROOT, handle, 0, 16)
        .expect("listing a fresh mount works");
    let next_offset = first.last().expect("the root lists . and ..").next_offset;

    adapter
        .create_file(
            OBJECT_ROOT,
            b"note.txt",
            AccessMode::ReadWrite,
            None,
            timestamp(1),
        )
        .unwrap();
    adapter.sync_filesystem().unwrap();

    adapter
        .read_directory(OBJECT_ROOT, handle, next_offset, 16)
        .expect("the walk resumes by name across the write");

    let report = shared.report();
    assert!(
        report.contains("ReadDirectoryPage"),
        "the core did refuse the stale cursor once, and hiding that would make \
         the report disagree with the driver: {report}"
    );
    assert!(
        report.contains("The driver recovers from some of these"),
        "a refusal the person never saw must not read as a failure they \
         suffered: {report}"
    );
    assert!(
        shared.counters().observed() > 0,
        "the recorder is installed and observing: {report}"
    );
}
