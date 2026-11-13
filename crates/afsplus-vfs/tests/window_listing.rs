//! A directory is listed while the delayed window holds changes to it.
//!
//! The listing used to commit the window first: a checkpoint and a device
//! flush for every listing of a directory just written to. A program that
//! creates a file and then looks a name up without regard to case lists the
//! directory each time. The listing now shows the committed entries with
//! the window's on top, and commits nothing.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{Durability, NodeKind, Vfs, WINDOW_LISTING_MAX};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn delayed() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 16384);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x71; 16],
            label: "Listing".into(),
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
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    vfs.set_durability(Durability::Delayed {
        idle_ms: 1_000,
        max_age_ms: 5_000,
    })
    .unwrap();
    vfs
}

/// The whole directory, `step` entries a call, as (name, kind).
fn list(vfs: &mut Vfs<MemoryBackend>, directory: u64, step: usize) -> Vec<(String, NodeKind)> {
    let handle = vfs.open_directory(directory).unwrap();
    let mut names = Vec::new();
    let mut cookie = 0;
    loop {
        let page = vfs.read_directory(handle, cookie, step).unwrap();
        for entry in &page.entries {
            names.push((String::from_utf8(entry.name.clone()).unwrap(), entry.kind));
        }
        cookie = page.next_cookie;
        if page.eof {
            break;
        }
    }
    // An adapter asks once more after the end, and is told so again.
    let after = vfs.read_directory(handle, cookie, step).unwrap();
    assert!(after.entries.is_empty() && after.eof);
    vfs.close(handle).unwrap();
    names
}

fn names(listing: &[(String, NodeKind)]) -> Vec<&str> {
    listing.iter().map(|(name, _)| name.as_str()).collect()
}

#[test]
fn a_listing_shows_what_the_window_holds_and_commits_nothing() {
    let mut vfs = delayed();
    for name in ["alpha", "Charlie", "echo"] {
        vfs.create_file(OBJECT_ROOT, name, at(1)).unwrap();
    }
    vfs.sync_filesystem().unwrap();
    let committed = vfs.generation();

    vfs.create_file(OBJECT_ROOT, "Bravo", at(2)).unwrap();
    vfs.unlink_file(OBJECT_ROOT, "charlie", at(2)).unwrap();
    let delta = vfs.create_directory(OBJECT_ROOT, "delta", at(2)).unwrap();
    vfs.create_file(delta, "inside", at(2)).unwrap();
    assert!(vfs.changes_pending());

    for step in [1, 2, 64] {
        let listing = list(&mut vfs, OBJECT_ROOT, step);
        assert_eq!(
            names(&listing),
            ["alpha", "Bravo", "delta", "echo"],
            "{step} a call"
        );
        assert_eq!(listing[2].1, NodeKind::Directory);
        assert_eq!(listing[1].1, NodeKind::File);
    }
    // A directory the window created is listed too.
    assert_eq!(names(&list(&mut vfs, delta, 1)), ["inside"]);
    assert!(vfs.changes_pending(), "the listings committed the window");

    // What the window showed is what the commit publishes.
    vfs.sync_filesystem().unwrap();
    assert!(vfs.generation() > committed);
    assert_eq!(
        names(&list(&mut vfs, OBJECT_ROOT, 1)),
        ["alpha", "Bravo", "delta", "echo"]
    );
    assert_eq!(names(&list(&mut vfs, delta, 64)), ["inside"]);
}

#[test]
fn a_pass_shows_the_directory_as_it_was_when_it_began() {
    let mut vfs = delayed();
    vfs.create_file(OBJECT_ROOT, "a", at(1)).unwrap();
    vfs.sync_filesystem().unwrap();
    vfs.create_file(OBJECT_ROOT, "c", at(2)).unwrap();

    let handle = vfs.open_directory(OBJECT_ROOT).unwrap();
    let first = vfs.read_directory(handle, 0, 1).unwrap();
    assert_eq!(first.entries[0].name, b"a");
    // Names arrive and leave during the pass, and the window is committed.
    vfs.create_file(OBJECT_ROOT, "b", at(3)).unwrap();
    vfs.unlink_file(OBJECT_ROOT, "c", at(3)).unwrap();
    vfs.sync_filesystem().unwrap();
    let second = vfs.read_directory(handle, first.next_cookie, 8).unwrap();
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].name, b"c");
    assert!(second.eof);
    // A rewind starts a pass on the directory of now.
    let again = vfs.read_directory(handle, 0, 8).unwrap();
    let names: Vec<_> = again
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    assert_eq!(names, [b"a".to_vec(), b"b".to_vec()]);
    vfs.close(handle).unwrap();
}

#[test]
fn a_large_directory_is_committed_and_listed_from_its_tree() {
    let mut vfs = delayed();
    let large = vfs.create_directory(OBJECT_ROOT, "large", at(1)).unwrap();
    for index in 0..=WINDOW_LISTING_MAX {
        vfs.create_file(large, &format!("file{index:05}"), at(1))
            .unwrap();
    }
    vfs.sync_filesystem().unwrap();
    vfs.create_file(large, "last", at(2)).unwrap();
    assert!(vfs.changes_pending());

    let listing = list(&mut vfs, large, 64);
    assert_eq!(listing.len(), WINDOW_LISTING_MAX + 2);
    assert_eq!(listing.last().unwrap().0, "last");
    assert!(
        !vfs.changes_pending(),
        "a directory this large is listed after a commit"
    );
}
