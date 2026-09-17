//! What the owner hit on the live mount: a mode, an owner and a modification
//! time that a POSIX caller sets and the volume keeps.
//!
//! Before this, `setattr` stored none of the three. A mode it could not keep
//! was refused with EOPNOTSUPP, which macOS prints as "Operation not
//! supported on socket" and the Finder renders as a read-only destination; a
//! time it could not keep was answered with SUCCESS and discarded, which is
//! worse, because the caller is told the write landed.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{FuseAdapter, FuseConfig};
use afsplus_vfs::{AccessMode, Vfs, VfsError};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn adapter() -> FuseAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xFB; 16],
            label: "PosixMetadata".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig::default(),
    )
}

/// The reproduction from the report, at the adapter: create with an explicit
/// mode, then chmod to each of the modes that used to fail.
#[test]
fn a_created_file_keeps_the_mode_it_was_created_with_and_every_chmod_after() {
    let mut fuse = adapter();
    let (attributes, handle) = fuse
        .create_file(
            OBJECT_ROOT,
            b"t.md",
            AccessMode::WriteOnly,
            Some(0o600),
            at(1),
        )
        .unwrap();
    fuse.close(handle).unwrap();
    assert_eq!(attributes.mode, 0o600, "creation mode was not kept");

    for mode in [0o644u16, 0o600, 0o755, 0o444, 0o700] {
        fuse.set_posix_mode(attributes.object_id, mode, at(2))
            .unwrap();
        assert_eq!(
            fuse.attributes(attributes.object_id).unwrap().mode,
            mode,
            "chmod {mode:#o} did not stick"
        );
    }
}

#[test]
fn a_created_directory_keeps_its_mode() {
    let mut fuse = adapter();
    let made = fuse
        .create_directory(OBJECT_ROOT, b"d", Some(0o700), at(1))
        .unwrap();
    assert_eq!(made.mode, 0o700);
    assert_eq!(fuse.attributes(made.object_id).unwrap().mode, 0o700);
}

/// The one that returned success and changed nothing.
#[test]
fn a_modification_time_a_caller_sets_is_the_one_read_back() {
    let mut fuse = adapter();
    let (attributes, handle) = fuse
        .create_file(OBJECT_ROOT, b"f", AccessMode::WriteOnly, None, at(1))
        .unwrap();
    fuse.close(handle).unwrap();

    let wanted = at(1_577_880_000);
    fuse.set_times(attributes.object_id, wanted, at(2)).unwrap();
    let after = fuse.attributes(attributes.object_id).unwrap();
    assert_eq!(after.modified, wanted, "touch -t was discarded again");
    assert_ne!(
        after.changed, wanted,
        "the metadata-change time is not the caller's to choose"
    );
}

#[test]
fn an_owner_is_stored_and_each_half_moves_alone() {
    let mut fuse = adapter();
    let (attributes, handle) = fuse
        .create_file(OBJECT_ROOT, b"f", AccessMode::WriteOnly, None, at(1))
        .unwrap();
    fuse.close(handle).unwrap();

    fuse.set_owner(attributes.object_id, Some(501), Some(20), at(2))
        .unwrap();
    let after = fuse.attributes(attributes.object_id).unwrap();
    assert_eq!((after.uid, after.gid), (501, 20));

    fuse.set_owner(attributes.object_id, None, Some(80), at(3))
        .unwrap();
    let after = fuse.attributes(attributes.object_id).unwrap();
    assert_eq!((after.uid, after.gid), (501, 80));
}

/// A bit the format cannot carry is refused, and refusing it does not stop a
/// file being created: the file exists and the caller sees the mode it got.
#[test]
fn sticky_is_refused_on_chmod_and_does_not_prevent_creation() {
    let mut fuse = adapter();
    let (attributes, handle) = fuse
        .create_file(
            OBJECT_ROOT,
            b"s",
            AccessMode::WriteOnly,
            Some(0o1755),
            at(1),
        )
        .unwrap();
    fuse.close(handle).unwrap();
    // Created despite the sticky bit, carrying the part that fits: 0o755,
    // not the 0o700 an untouched protection word would have read as.
    assert_eq!(
        attributes.mode, 0o755,
        "the representable part was not kept"
    );

    assert_eq!(
        fuse.set_posix_mode(attributes.object_id, 0o1777, at(2)),
        Err(VfsError::NotSupported)
    );
}
