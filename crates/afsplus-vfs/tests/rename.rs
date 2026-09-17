//! Moving and renaming through the portable interface.
//!
//! POSIX rename replaces an existing target by default, so every ordinary `mv`
//! arrives with replace set. The core has two renames: a batched one that can
//! unlink a replaced target in the same checkpoint but supports files only, and
//! a general one that moves any object but cannot replace. Sending every
//! replacing rename to the batched one made a directory unmovable: `mkdir a &&
//! mv a b` failed with the implementation limit, surfacing on macOS as
//! "Value too large to be stored in data type".

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{Vfs, VfsError};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn mounted() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x5E; 16],
            label: "Rename".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    Vfs::mount(device, MountOptions::default()).unwrap()
}

const ROOT: u64 = OBJECT_ROOT;

#[test]
fn a_directory_can_be_renamed_in_place() {
    let mut vfs = mounted();
    let made = vfs.create_directory(ROOT, "a", at(1)).unwrap();
    vfs.rename(ROOT, "a", ROOT, "b", true, at(2)).unwrap();
    assert_eq!(vfs.lookup(ROOT, "b").unwrap(), made);
    assert!(matches!(vfs.lookup(ROOT, "a"), Err(VfsError::NotFound)));
}

#[test]
fn a_directory_moves_into_another_directory_and_keeps_its_children() {
    let mut vfs = mounted();
    let moved = vfs.create_directory(ROOT, "src", at(1)).unwrap();
    let child = vfs.create_file(moved, "inside.txt", at(1)).unwrap();
    let dest = vfs.create_directory(ROOT, "dest", at(1)).unwrap();

    vfs.rename(ROOT, "src", dest, "src", true, at(2)).unwrap();

    assert_eq!(vfs.lookup(dest, "src").unwrap(), moved);
    assert_eq!(vfs.lookup(moved, "inside.txt").unwrap(), child);
    assert!(matches!(vfs.lookup(ROOT, "src"), Err(VfsError::NotFound)));
}

#[test]
fn a_directory_still_cannot_be_moved_into_its_own_descendant() {
    let mut vfs = mounted();
    let outer = vfs.create_directory(ROOT, "outer", at(1)).unwrap();
    let inner = vfs.create_directory(outer, "inner", at(1)).unwrap();

    let refused = vfs.rename(ROOT, "outer", inner, "outer", true, at(2));

    assert!(
        refused.is_err(),
        "moving a directory under itself would detach the subtree"
    );
    assert_eq!(vfs.lookup(ROOT, "outer").unwrap(), outer);
    assert_eq!(vfs.lookup(outer, "inner").unwrap(), inner);
}

#[test]
fn renaming_a_file_over_an_existing_file_still_replaces_it() {
    let mut vfs = mounted();
    let keep = vfs.create_file(ROOT, "keep.txt", at(1)).unwrap();
    vfs.create_file(ROOT, "gone.txt", at(1)).unwrap();

    vfs.rename(ROOT, "keep.txt", ROOT, "gone.txt", true, at(2))
        .unwrap();

    assert_eq!(vfs.lookup(ROOT, "gone.txt").unwrap(), keep);
    assert!(matches!(
        vfs.lookup(ROOT, "keep.txt"),
        Err(VfsError::NotFound)
    ));
}

#[test]
fn a_rename_onto_a_name_already_taken_by_a_directory_is_still_refused() {
    let mut vfs = mounted();
    vfs.create_file(ROOT, "f.txt", at(1)).unwrap();
    let occupied = vfs.create_directory(ROOT, "taken", at(1)).unwrap();

    let refused = vfs.rename(ROOT, "f.txt", ROOT, "taken", true, at(2));

    assert!(refused.is_err(), "a directory is not silently discarded");
    assert_eq!(vfs.lookup(ROOT, "taken").unwrap(), occupied);
}
