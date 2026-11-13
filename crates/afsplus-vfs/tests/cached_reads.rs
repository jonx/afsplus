// SPDX-License-Identifier: BSD-2-Clause
//! What the mounted volume answers from memory against what the device
//! holds.
//!
//! A mount keeps decoded tree nodes and object records beside the cached
//! blocks they came from, remembers where the object map places a record
//! and that the orphan directory is empty, keeps which directory entries
//! it has checked, and keeps file data in a pool of its own. Every one of
//! those is only right while it describes the bytes on the device. Here a
//! volume is changed through every path that rewrites, retires or
//! republishes a record or a tree node, with the caches warm before each
//! change, and after each change everything a reader can ask is asked of
//! the mount and of a fresh, cacheless, read-only mount of a copy of the
//! same device. The two must agree to the last field.
use std::collections::BTreeMap;

use afsplus_block::{CachedDevice, MemoryBackend};
use afsplus_core::volume::{AttributeWriteMode, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountMode, MountOptions, NamePolicy,
};
use afsplus_format::Timespec;
use afsplus_vfs::{AccessMode, Durability, NodeKind, Vfs};

type Cached = Vfs<CachedDevice<MemoryBackend>>;

const LIMITS: SnapshotWorkLimits = SnapshotWorkLimits {
    max_edit_records: 4096,
    max_views: 8,
    reclaim_records: 64,
};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut image = MemoryBackend::new(4096, 8192);
    mkfs_with_snapshots_and_security_descriptors(
        &mut image,
        &MkfsParams {
            uuid: [0x5c; 16],
            label: "Cached".into(),
            region_size: 1024,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    image
}

fn mount(image: MemoryBackend, metadata: usize, data: usize, durability: Durability) -> Cached {
    let mut vfs = Vfs::mount_with_snapshot_limits(
        CachedDevice::with_capacities(image, metadata, data),
        MountOptions::default(),
        LIMITS,
    )
    .unwrap();
    vfs.set_durability(durability).unwrap();
    vfs
}

/// Everything a reader can learn below `directory`, by every read path:
/// both page sizes of the listing, the lookup of each name as stored and in
/// the other case, the stat, the comment, the attributes, the link target
/// and the content, whole and in ragged pieces.
fn observe<D: afsplus_block::BlockDevice>(
    vfs: &mut Vfs<D>,
    directory: u64,
    path: &str,
    out: &mut BTreeMap<String, String>,
) {
    let mut listings = Vec::new();
    for page_size in [1usize, 64] {
        let handle = vfs.open_directory(directory).unwrap();
        let mut entries = Vec::new();
        let mut cookie = 0;
        loop {
            let page = vfs.read_directory(handle, cookie, page_size).unwrap();
            entries.extend(
                page.entries
                    .into_iter()
                    .map(|entry| (entry.name, entry.object_id, entry.kind)),
            );
            cookie = page.next_cookie;
            if page.eof {
                break;
            }
        }
        vfs.close(handle).unwrap();
        listings.push(entries);
    }
    assert_eq!(listings[0], listings[1], "{path}: page sizes disagree");
    let stat = vfs.stat(directory).unwrap();
    out.insert(
        format!("{path}/"),
        format!("{stat:?} comment {:?}", vfs.comment(directory).unwrap()),
    );
    for (name, object, kind) in listings.swap_remove(0) {
        let name = String::from_utf8(name).unwrap();
        let here = format!("{path}/{name}");
        assert_eq!(
            vfs.lookup_entry(directory, &name).unwrap(),
            (object, name.clone()),
            "{here}: lookup"
        );
        assert_eq!(
            vfs.lookup_entry(directory, &name.to_uppercase()).unwrap(),
            (object, name.clone()),
            "{here}: lookup in the other case"
        );
        let stat = vfs.stat(object).unwrap();
        assert_eq!(stat.kind, kind, "{here}: listed kind");
        let mut line = format!("{stat:?} comment {:?}", vfs.comment(object).unwrap());
        for attribute in vfs.attribute_names(object).unwrap() {
            let value = vfs.attribute(object, &attribute).unwrap();
            line.push_str(&format!(" {attribute}={value:?}"));
        }
        match kind {
            NodeKind::Directory => {
                out.insert(format!("{here}/."), line);
                observe(vfs, object, &here, out);
                continue;
            }
            NodeKind::Symlink => {
                let mut target = [0u8; 256];
                let length = vfs.read_link(object, &mut target).unwrap();
                line.push_str(&format!(" -> {:?}", &target[..length]));
            }
            NodeKind::File => {
                let handle = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
                let mut content = vec![0u8; stat.size as usize];
                assert_eq!(vfs.read(handle, 0, &mut content).unwrap(), content.len());
                // The same bytes in pieces that start and end inside blocks.
                let mut pieces = vec![0u8; content.len()];
                let mut offset = 0;
                while offset < pieces.len() {
                    let end = pieces.len().min(offset + 5000);
                    let got = vfs
                        .read(handle, offset as u64, &mut pieces[offset..end])
                        .unwrap();
                    assert_eq!(got, end - offset, "{here}: short read");
                    offset = end;
                }
                assert!(content == pieces, "{here}: pieces differ from the whole");
                vfs.close(handle).unwrap();
                let sum = content.iter().fold(0xcbf2_9ce4_8422_2325u64, |sum, byte| {
                    (sum ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
                });
                line.push_str(&format!(" content {} bytes {sum:016x}", content.len()));
            }
            NodeKind::Internal => panic!("{here}: internal object listed"),
        }
        out.insert(here, line);
    }
}

fn observed<D: afsplus_block::BlockDevice>(vfs: &mut Vfs<D>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let root = vfs.root_object();
    observe(vfs, root, "", &mut out);
    out
}

/// The entries two views do not share, each side's line for them.
fn difference(first: &BTreeMap<String, String>, second: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for path in first.keys().chain(second.keys()) {
        if first.get(path) != second.get(path) && !out.contains(&format!("{path}\n")) {
            out.push_str(&format!(
                "{path}\n  {:?}\n  {:?}\n",
                first.get(path),
                second.get(path)
            ));
        }
    }
    out
}

/// Commits what waits, then compares the mount's answers with those of a
/// fresh mount of a copy of its device. The mount comes back with its
/// volume, and so with everything the volume and its device remember.
fn check(mut vfs: Cached, durability: Durability, what: &str) -> Cached {
    let before_commit = observed(&mut vfs);
    vfs.sync_filesystem().unwrap();
    let cached = observed(&mut vfs);
    let mut volume = vfs.into_volume();
    let image = volume.device_mut().inner().clone();
    let mut fresh = Vfs::mount_with_snapshot_limits(
        image,
        MountOptions {
            mode: MountMode::ReadOnly,
            ..Default::default()
        },
        LIMITS,
    )
    .unwrap();
    let on_device = observed(&mut fresh);
    assert!(
        cached == on_device,
        "after {what}, the mount and the device disagree:\n{}",
        difference(&cached, &on_device)
    );
    // What a reader saw before the commit is what the commit wrote: every
    // change is visible at once (ADR-121). Two fields are the commit's own
    // to settle, the space an object takes and the generation its content
    // was published in; a listing commits the directory it lists, so even
    // one walk sees both sides of that.
    let settle = |view: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        view.iter()
            .map(|(path, line)| {
                let mut line = line.clone();
                for (field, end) in [("allocated_size: ", ','), ("content_generation: ", ' ')] {
                    if let Some(start) = line.find(field) {
                        let value = start + field.len();
                        let end = value + line[value..].find(end).unwrap();
                        line.replace_range(value..end, "_");
                    }
                }
                (path.clone(), line)
            })
            .collect()
    };
    assert!(
        settle(&before_commit) == settle(&cached),
        "after {what}, a reader before the commit and one after it disagree:\n{}",
        difference(&settle(&before_commit), &settle(&cached))
    );
    let mut vfs = Vfs::new(volume);
    vfs.set_durability(durability).unwrap();
    // Warm again: the next change meets caches that hold this state.
    observed(&mut vfs);
    vfs
}

fn write(vfs: &mut Cached, object: u64, offset: u64, bytes: &[u8], now: i64) {
    let handle = vfs.open_file(object, AccessMode::ReadWrite).unwrap();
    assert_eq!(
        vfs.write(handle, offset, bytes, at(now)).unwrap(),
        bytes.len()
    );
    vfs.close(handle).unwrap();
}

fn bytes(seed: u8, length: usize) -> Vec<u8> {
    (0..length)
        .map(|i| seed.wrapping_add((i % 251) as u8))
        .collect()
}

/// One pass over every kind of change, each followed by the comparison.
fn every_change(metadata: usize, data: usize, durability: Durability) {
    let mut vfs = mount(formatted(), metadata, data, durability);
    let root = vfs.root_object();
    let step = |vfs: Cached, what: &str| check(vfs, durability, what);
    let mut now = 100;
    let mut tick = || {
        now += 1;
        now
    };

    // Creation, at depth, and enough entries for a directory of several
    // leaves and an object map of more than one node.
    let docs = vfs.create_directory(root, "Docs", at(tick())).unwrap();
    let deep = vfs.create_directory(docs, "Deep", at(tick())).unwrap();
    let mut vfs = step(vfs, "two directories");
    let small = vfs.create_file(docs, "Small.txt", at(tick())).unwrap();
    write(&mut vfs, small, 0, &bytes(1, 700), tick());
    let large = vfs.create_file(deep, "Large.bin", at(tick())).unwrap();
    write(&mut vfs, large, 0, &bytes(2, 150_000), tick());
    let mut vfs = step(vfs, "two files with content");
    for index in 0..400 {
        let name = format!("entry-{index:03}-with-a-name-long-enough-to-fill-leaves");
        vfs.create_file(deep, &name, at(tick())).unwrap();
        if index % 100 == 99 {
            vfs = step(vfs, "a hundred more entries");
        }
    }

    // Content: over the middle, past the end, a hole, shorter, longer.
    write(&mut vfs, large, 70_000, &bytes(3, 9_000), tick());
    let mut vfs = step(vfs, "an overwrite in the middle");
    write(&mut vfs, large, 149_000, &bytes(4, 40_000), tick());
    let mut vfs = step(vfs, "an append");
    write(&mut vfs, small, 300_000, &bytes(5, 10), tick());
    let mut vfs = step(vfs, "a write far past the end");
    let handle = vfs.open_file(large, AccessMode::ReadWrite).unwrap();
    vfs.truncate(handle, 33_333, at(tick())).unwrap();
    vfs.close(handle).unwrap();
    let mut vfs = step(vfs, "a truncation");
    let handle = vfs.open_file(large, AccessMode::ReadWrite).unwrap();
    vfs.truncate(handle, 90_000, at(tick())).unwrap();
    vfs.close(handle).unwrap();
    let mut vfs = step(vfs, "an extension");

    // Metadata, each kind on a file and on a directory.
    for object in [small, deep] {
        vfs.set_protection(object, 0x0f, at(tick())).unwrap();
        vfs = step(vfs, "a protection change");
        vfs.set_owner(object, Some(501), Some(20), at(tick()))
            .unwrap();
        vfs = step(vfs, "an owner change");
        vfs.set_times(object, at(77), at(tick())).unwrap();
        vfs = step(vfs, "a date change");
        vfs.set_comment(object, "first comment", at(tick()))
            .unwrap();
        vfs = step(vfs, "a comment");
        vfs.set_comment(object, "second, longer comment", at(tick()))
            .unwrap();
        vfs = step(vfs, "a changed comment");
        vfs.set_comment(object, "", at(tick())).unwrap();
        vfs = step(vfs, "a removed comment");
        vfs.set_attributes(
            object,
            &[
                ("user.one", Some(b"1")),
                ("user.two", Some(&bytes(9, 5000))),
            ],
            AttributeWriteMode::Create,
            at(tick()),
        )
        .unwrap();
        vfs = step(vfs, "two attributes");
        vfs.set_attributes(
            object,
            &[("user.one", None), ("user.two", Some(b"short"))],
            AttributeWriteMode::Replace,
            at(tick()),
        )
        .unwrap();
        vfs = step(vfs, "an attribute removed and one replaced");
    }

    // Names: rename in place, in the other case, across directories, over
    // an existing file; hard links; a symlink.
    vfs.rename(docs, "Small.txt", docs, "Renamed.txt", false, at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a rename");
    vfs.rename(docs, "Renamed.txt", docs, "RENAMED.TXT", false, at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a rename to the other case");
    vfs.rename(docs, "RENAMED.TXT", deep, "Moved.txt", false, at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a move to another directory");
    let victim = vfs.create_file(deep, "Victim", at(tick())).unwrap();
    write(&mut vfs, victim, 0, &bytes(6, 20_000), tick());
    let mut vfs = step(vfs, "a file to be replaced");
    vfs.rename(deep, "Moved.txt", deep, "Victim", true, at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a rename over an existing file");
    vfs.rename(root, "Docs", root, "Papers", false, at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a renamed directory");
    vfs.link_file(small, docs, "Second-name", at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a hard link");
    vfs.link_file(small, root, "Third-name", at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a second hard link");
    vfs.unlink_file(deep, "Victim", at(tick())).unwrap();
    let mut vfs = step(vfs, "one name of three removed");
    vfs.create_symlink(docs, "Link", "Deep/Large.bin", at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a symlink");
    let copy = vfs
        .clone_file(large, docs, "Clone.bin", at(tick()))
        .unwrap();
    let mut vfs = step(vfs, "a clone that shares its extents");
    write(&mut vfs, copy, 4096, &bytes(7, 4096), tick());
    let mut vfs = step(vfs, "a write to the clone");
    write(&mut vfs, large, 0, &bytes(8, 100), tick());
    let mut vfs = step(vfs, "a write to the clone's source");

    // Deletion: the last name, a file held open, many at once, a
    // directory; then the cleaning and the return of the space.
    vfs.unlink_file(docs, "Second-name", at(tick())).unwrap();
    vfs.unlink_file(root, "Third-name", at(tick())).unwrap();
    let mut vfs = step(vfs, "the last names of a file removed");
    let held = vfs.open_file(large, AccessMode::ReadOnly).unwrap();
    vfs.unlink_file(deep, "Large.bin", at(tick())).unwrap();
    let before = observed(&mut vfs);
    assert!(!before.contains_key("/Papers/Deep/Large.bin"));
    // Committed and still open, the file waits in the orphan directory,
    // which a moment ago was empty and remembered so: its identifier must
    // answer nothing, now and after every later commit.
    vfs.sync_filesystem().unwrap();
    assert!(vfs.stat(large).is_err(), "a deleted file answers by its ID");
    let other = vfs.create_file(docs, "while-it-waits", at(tick())).unwrap();
    vfs.sync_filesystem().unwrap();
    assert!(vfs.stat(large).is_err(), "a deleted file answers by its ID");
    assert!(vfs.stat(other).is_ok());
    vfs.close(held).unwrap();
    let mut vfs = step(vfs, "a file deleted while open, then closed");
    for index in (0..400).step_by(3) {
        let name = format!("entry-{index:03}-with-a-name-long-enough-to-fill-leaves");
        vfs.unlink_file(deep, &name, at(tick())).unwrap();
    }
    let mut vfs = step(vfs, "a third of a directory deleted");
    for _ in 0..8 {
        vfs.cleanup_orphans(16, at(tick())).unwrap();
        vfs.reclaim_space(16, at(tick())).unwrap();
        vfs = step(vfs, "orphans cleaned and space reclaimed");
    }
    for idle in 0..6 {
        vfs.commit_if_due(at(tick() + 100 * (idle + 1))).unwrap();
        vfs.maintenance_step(at(tick())).unwrap();
        vfs = step(vfs, "an idle tick and a maintenance step");
    }
    let doomed = vfs.create_directory(docs, "Doomed", at(tick())).unwrap();
    vfs.create_file(doomed, "inside", at(tick())).unwrap();
    let mut vfs = step(vfs, "a directory to be removed");
    assert!(vfs.remove_directory(docs, "Doomed", at(tick())).is_err());
    vfs.unlink_file(doomed, "inside", at(tick())).unwrap();
    vfs.remove_directory(docs, "Doomed", at(tick())).unwrap();
    let mut vfs = step(vfs, "a removed directory");

    // Snapshots: taken, changed after, read, restored from, deleted.
    let keep = vfs.create_file(docs, "Keep", at(tick())).unwrap();
    write(&mut vfs, keep, 0, &bytes(10, 30_000), tick());
    vfs.set_comment(keep, "as snapshotted", at(tick())).unwrap();
    let kept_tree = vfs.create_directory(docs, "Tree", at(tick())).unwrap();
    let leaf = vfs.create_file(kept_tree, "leaf", at(tick())).unwrap();
    write(&mut vfs, leaf, 0, &bytes(11, 9_000), tick());
    let mut vfs = step(vfs, "objects to snapshot");
    let snapshot = vfs.snapshot_create(at(tick())).unwrap();
    let mut vfs = step(vfs, "a snapshot");
    write(&mut vfs, keep, 100, &bytes(12, 12_000), tick());
    vfs.set_comment(keep, "changed since", at(tick())).unwrap();
    vfs.unlink_file(kept_tree, "leaf", at(tick())).unwrap();
    let mut vfs = step(vfs, "changes after the snapshot");
    let reader = vfs.snapshot_open(snapshot, 4).unwrap();
    let papers = vfs.snapshot_lookup(reader, root, "Papers").unwrap();
    let old_keep = vfs.snapshot_lookup(reader, papers, "Keep").unwrap();
    let old_tree = vfs.snapshot_lookup(reader, papers, "Tree").unwrap();
    vfs.snapshot_restore_file(reader, old_keep, docs, "Keep.restored", at(tick()))
        .unwrap();
    vfs.snapshot_close(reader).unwrap();
    let mut vfs = step(vfs, "a file restored from the snapshot");
    let reader = vfs.snapshot_open(snapshot, 4).unwrap();
    vfs.snapshot_restore_tree(
        reader,
        old_tree,
        docs,
        "Tree.restored",
        afsplus_vfs::TreeRestoreLimits {
            max_nodes: 64,
            max_depth: 8,
            max_metadata_bytes: 1 << 20,
        },
        at(tick()),
    )
    .unwrap();
    vfs.snapshot_close(reader).unwrap();
    let mut vfs = step(vfs, "a tree restored from the snapshot");
    let restored = vfs.lookup_entry(docs, "keep.RESTORED").unwrap().0;
    assert_eq!(vfs.comment(restored).unwrap(), "as snapshotted");
    vfs.snapshot_delete(snapshot, at(tick())).unwrap();
    let mut vfs = step(vfs, "the snapshot deleted");
    for _ in 0..6 {
        vfs.reclaim_space(32, at(tick())).unwrap();
        vfs = step(vfs, "the snapshot's space reclaimed");
    }
    vfs.set_volume_label("Relabelled", at(tick())).unwrap();
    let vfs = step(vfs, "a new label");
    drop(vfs);
}

#[test]
fn every_change_committed_at_once_with_room_to_cache_everything() {
    every_change(4096, 4096, Durability::Sync);
}

#[test]
fn every_change_delayed_with_room_to_cache_everything() {
    every_change(
        4096,
        4096,
        Durability::Delayed {
            idle_ms: 1000,
            max_age_ms: 5000,
        },
    );
}

/// A cache this small evicts and refills all the time, which is where a
/// reused slot or a reused block would show.
#[test]
fn every_change_delayed_with_a_cache_that_keeps_evicting() {
    every_change(
        24,
        24,
        Durability::Delayed {
            idle_ms: 1000,
            max_age_ms: 5000,
        },
    );
}

/// A long seeded mix of changes in one directory tree, compared after each
/// handful. Operations a state does not allow are refused, which is fine:
/// what is compared is what the volume holds, whatever got it there.
#[test]
fn a_long_random_mix_of_changes() {
    for (seed, metadata, data, durability) in [
        (0x1234_5678u32, 4096usize, 4096usize, Durability::Sync),
        (
            0x9e37_79b9,
            64,
            64,
            Durability::Delayed {
                idle_ms: 1000,
                max_age_ms: 5000,
            },
        ),
    ] {
        let mut state = seed;
        let mut next = move |bound: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % bound
        };
        let mut vfs = mount(formatted(), metadata, data, durability);
        let root = vfs.root_object();
        let mut directories = vec![root];
        for index in 0..3 {
            directories.push(
                vfs.create_directory(root, &format!("dir{index}"), at(1))
                    .unwrap(),
            );
        }
        let name = |pick: u32| format!("Name{}", pick % 12);
        for round in 0..600i64 {
            let now = at(1000 + round);
            let parent = directories[next(4) as usize];
            let other = directories[next(4) as usize];
            let (first, second) = (name(next(12)), name(next(12)));
            let target = vfs.lookup_entry(parent, &first).map(|(object, _)| object);
            let _ = match next(14) {
                0..=2 => vfs.create_file(parent, &first, now).map(|_| ()),
                3..=5 => target.and_then(|object| {
                    let handle = vfs.open_file(object, AccessMode::ReadWrite)?;
                    let offset = u64::from(next(60_000));
                    let result = vfs
                        .write(
                            handle,
                            offset,
                            &bytes(round as u8, 1 + next(20_000) as usize),
                            now,
                        )
                        .map(|_| ());
                    vfs.close(handle)?;
                    result
                }),
                6 => target.and_then(|object| {
                    let handle = vfs.open_file(object, AccessMode::ReadWrite)?;
                    let result = vfs.truncate(handle, u64::from(next(50_000)), now);
                    vfs.close(handle)?;
                    result
                }),
                7 => vfs.unlink_file(parent, &first, now),
                8..=9 => vfs.rename(parent, &first, other, &second, next(2) == 0, now),
                10 => target.and_then(|object| vfs.link_file(object, other, &second, now)),
                11 => target.and_then(|object| vfs.set_comment(object, &second, now)),
                12 => target.and_then(|object| vfs.set_protection(object, next(256), now)),
                _ => vfs
                    .commit_if_due(at(1000 + round + i64::from(next(8))))
                    .map(|_| ()),
            };
            if round % 5 == 4 {
                vfs = check(vfs, durability, &format!("round {round} of seed {seed:#x}"));
            }
        }
    }
}
