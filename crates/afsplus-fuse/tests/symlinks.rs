//! Symlinks through the driver.
//!
//! The core and the portable interface have created and read symlinks all
//! along. The driver implemented neither handler, so it answered with the
//! trait's defaults and `ln -s` on a mounted volume failed with "Operation not
//! permitted", which reads as a permission problem rather than as a hole.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{FuseAdapter, FuseConfig};
use afsplus_vfs::{NodeKind, Vfs};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn mounted() -> FuseAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x51; 16],
            label: "Links".into(),
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
    let vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    FuseAdapter::new(vfs, FuseConfig::default())
}

#[test]
fn a_symlink_can_be_made_and_read_back_exactly() {
    let mut adapter = mounted();
    adapter
        .create_file(
            OBJECT_ROOT,
            b"target.txt",
            afsplus_vfs::AccessMode::WriteOnly,
            None,
            at(1),
        )
        .unwrap();

    let made = adapter
        .create_symlink(OBJECT_ROOT, b"link.txt", b"target.txt", at(2))
        .unwrap();
    assert_eq!(made.kind, NodeKind::Symlink);

    assert_eq!(
        adapter.read_link(made.object_id).unwrap(),
        b"target.txt".to_vec()
    );
}

#[test]
fn a_long_target_comes_back_whole() {
    // A short buffer leaves the target unread and reports the length it needed.
    // Reading the length first is the difference between the whole target and
    // an empty one, and a path is exactly where that bites.
    let mut adapter = mounted();
    let target = format!("../{}/deep/file.txt", "a-long-directory-name/".repeat(20));
    let made = adapter
        .create_symlink(OBJECT_ROOT, b"deep.link", target.as_bytes(), at(1))
        .unwrap();

    assert_eq!(
        adapter.read_link(made.object_id).unwrap(),
        target.as_bytes()
    );
}

#[test]
fn a_symlink_is_listed_as_one_and_deleting_it_leaves_its_target() {
    let mut adapter = mounted();
    adapter
        .create_file(
            OBJECT_ROOT,
            b"kept.txt",
            afsplus_vfs::AccessMode::WriteOnly,
            None,
            at(1),
        )
        .unwrap();
    adapter
        .create_symlink(OBJECT_ROOT, b"gone.link", b"kept.txt", at(2))
        .unwrap();

    let handle = adapter.open_directory(OBJECT_ROOT).unwrap();
    let listed = adapter.read_directory(OBJECT_ROOT, handle, 0, 16).unwrap();
    let link = listed
        .iter()
        .find(|entry| entry.name == b"gone.link")
        .expect("the link must appear in its directory");
    assert_eq!(link.kind, NodeKind::Symlink);
    adapter.close(handle).unwrap();

    adapter
        .unlink_file(OBJECT_ROOT, b"gone.link", at(4))
        .unwrap();
    assert!(adapter.lookup(OBJECT_ROOT, b"kept.txt").is_ok());
}
