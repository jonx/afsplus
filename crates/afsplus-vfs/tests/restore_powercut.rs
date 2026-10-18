// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! Real restore write traces: all operation cuts, full tail subsets and tears.
use afsplus_block::{
    for_each_crash_state_with_budget, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend,
};
use afsplus_core::volume::{AttributeWriteMode, PreservedMetadata, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, TreeRestoreLimits, Vfs, VfsError};
use std::collections::BTreeSet;

const STAGE: &str = "system.restore.stage";
const PAYLOAD: &[u8] = b"historical\0private bytes";
const MAX_TAIL: usize = 12;
fn work() -> SnapshotWorkLimits {
    SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 64,
    }
}
fn mount<D: BlockDevice>(image: D) -> Vfs<D> {
    Vfs::mount_with_snapshot_limits(image, MountOptions::default(), work()).unwrap()
}
fn preserved(seed: i64) -> PreservedMetadata {
    PreservedMetadata {
        protection: 0x2050,
        owner_uid: 70_000 + seed as u32,
        owner_gid: 4_000_000_000,
        created: Timespec {
            seconds: seed,
            nanoseconds: 123,
        },
        modified: Timespec {
            seconds: seed + 10,
            nanoseconds: 456,
        },
        changed: Timespec {
            seconds: seed + 20,
            nanoseconds: 789,
        },
    }
}
struct Fixture {
    image: MemoryBackend,
    snapshot: u64,
    source: u64,
    file: u64,
    first_new: u64,
}
fn fixture(tree: bool) -> Fixture {
    let mut image = MemoryBackend::new(4096, 4096);
    mkfs_with_snapshots_and_security_descriptors(
        &mut image,
        &MkfsParams {
            uuid: [0x83; 16],
            label: "PowercutRestore".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    let mut volume = mount(image).into_volume();
    let source = if tree {
        volume
            .create_directory(OBJECT_ROOT, "Captured", Timespec::default())
            .unwrap()
    } else {
        volume
            .create_file_in_root("Captured", PAYLOAD, Timespec::default())
            .unwrap()
    };
    let file = if tree {
        volume
            .create_file_in_directory(source, "é.cfg", PAYLOAD, Timespec::default())
            .unwrap()
    } else {
        source
    };
    let mut nodes = vec![(source, 1)];
    if tree {
        let empty = volume
            .create_directory(source, "Empty", Timespec::default())
            .unwrap();
        nodes.extend([(file, 2), (empty, 3)]);
    }
    for (object, seed) in nodes {
        volume
            .set_object_comment(object, &format!("saved-{seed}"), Timespec::default())
            .unwrap();
        volume
            .set_attributes(
                object,
                &[("user.kept", Some(b"opaque\0attribute"))],
                AttributeWriteMode::Create,
                Timespec::default(),
            )
            .unwrap();
        volume
            .restore_object_metadata(object, preserved(seed))
            .unwrap();
    }
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    // The restored bytes and metadata must come from the capture, not the
    // still present live source with the same object ID.
    volume
        .write_file_at(file, 0, b"LIVE", Timespec::default())
        .unwrap();
    volume
        .set_object_comment(file, "live comment", Timespec::default())
        .unwrap();
    volume.restore_object_metadata(file, preserved(99)).unwrap();
    let first_new = volume.checkpoint().next_object_id;
    Fixture {
        image: volume.into_device(),
        snapshot,
        source,
        file,
        first_new,
    }
}
fn restore<D: BlockDevice>(
    vfs: &mut Vfs<D>,
    reader: u64,
    source: u64,
    name: &str,
    tree: bool,
) -> u64 {
    let now = Timespec {
        seconds: 900,
        nanoseconds: 0,
    };
    if tree {
        vfs.snapshot_restore_tree(
            reader,
            source,
            OBJECT_ROOT,
            name,
            TreeRestoreLimits {
                max_nodes: 3,
                max_depth: 2,
                max_metadata_bytes: 4096,
            },
            now,
        )
        .unwrap()
    } else {
        vfs.snapshot_restore_file(reader, source, OBJECT_ROOT, name, now)
            .unwrap()
    }
}
fn verify_node(vfs: &mut Vfs<MemoryBackend>, object: u64, seed: i64, bytes: Option<&[u8]>) {
    let stat = vfs.stat(object).unwrap();
    let expected = preserved(seed);
    assert_eq!(
        (stat.protection, stat.owner_uid, stat.owner_gid),
        (
            u64::from(expected.protection),
            expected.owner_uid,
            expected.owner_gid
        )
    );
    assert_eq!(
        (stat.created, stat.modified, stat.changed),
        (expected.created, expected.modified, expected.changed)
    );
    assert_eq!(vfs.comment(object).unwrap(), format!("saved-{seed}"));
    assert_eq!(
        vfs.attribute(object, "user.kept").unwrap(),
        Some(b"opaque\0attribute".to_vec())
    );
    assert_eq!(
        vfs.attribute_names(object).unwrap(),
        vec!["user.kept".to_string()]
    );
    assert_eq!(stat.links, 1);
    if let Some(bytes) = bytes {
        assert_eq!(stat.size, bytes.len() as u64);
        let handle = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
        let mut actual = vec![0; bytes.len() + 1];
        assert_eq!(vfs.read(handle, 0, &mut actual).unwrap(), bytes.len());
        assert_eq!(&actual[..bytes.len()], bytes);
        vfs.close(handle).unwrap();
    }
}
#[derive(Default)]
struct Counts {
    states: usize,
    tears: usize,
    private_objects: usize,
    empty_private: usize,
    unmarked_private: usize,
    complete: usize,
}
fn verify_state(
    image: MemoryBackend,
    fixture: &Fixture,
    name: &str,
    tree: bool,
    description: &str,
    counts: &mut Counts,
) {
    let mut raw = mount(image).into_volume();
    let next = raw.checkpoint().next_object_id;
    let raw_final = raw.lookup_in_directory(OBJECT_ROOT, name).unwrap();
    let mut candidates = Vec::new();
    for object in fixture.first_new..next {
        let metadata = raw.visible_metadata(object).unwrap();
        if let Some(metadata) = metadata {
            let marker = raw.attribute(object, STAGE).unwrap();
            // A private staging root must never occupy the final comparison
            // key, even when the ordinary VFS hides it. This witnesses the
            // real durable stage and makes the Unicode-filter mutation fail.
            if marker.as_deref() == Some(object.to_le_bytes().as_slice()) {
                assert_ne!(
                    raw_final,
                    Some(object),
                    "private stage occupies final key: {description}"
                );
            }
            candidates.push((object, metadata.size_bytes, marker.is_none()));
        }
    }
    let mut vfs = mount(raw.into_device());
    let mut published = BTreeSet::new();
    match vfs.lookup(OBJECT_ROOT, name) {
        Err(VfsError::NotFound) => {}
        Ok(root) => {
            published.insert(root);
            verify_node(&mut vfs, root, 1, if tree { None } else { Some(PAYLOAD) });
            if tree {
                let file = vfs.lookup(root, "é.cfg").unwrap();
                let empty = vfs.lookup(root, "Empty").unwrap();
                published.extend([file, empty]);
                verify_node(&mut vfs, file, 2, Some(PAYLOAD));
                verify_node(&mut vfs, empty, 3, None);
                let dir = vfs.open_directory(root).unwrap();
                let page = vfs.read_directory(dir, 0, 8).unwrap();
                assert!(page.eof);
                assert_eq!(page.entries.len(), 2);
                vfs.close(dir).unwrap();
                let dir = vfs.open_directory(empty).unwrap();
                let page = vfs.read_directory(dir, 0, 8).unwrap();
                assert!(page.eof && page.entries.is_empty());
                vfs.close(dir).unwrap();
            }
            counts.complete += 1;
        }
        error => panic!("unexpected final result {error:?}: {description}"),
    }
    for (object, size, unmarked) in candidates {
        if !published.contains(&object) {
            counts.private_objects += 1;
            counts.empty_private += usize::from(size == 0);
            counts.unmarked_private += usize::from(unmarked);
            assert_eq!(
                vfs.stat(object),
                Err(VfsError::NotFound),
                "direct stat leaks staging object {object}: {description}"
            );
            assert_eq!(
                vfs.open_file(object, AccessMode::ReadOnly),
                Err(VfsError::NotFound),
                "direct open: {description}"
            );
            assert_eq!(
                vfs.create_file(object, "Injected", Timespec::default()),
                Err(VfsError::NotFound),
                "create under private object: {description}"
            );
            assert_eq!(
                vfs.create_directory(object, "InjectedDir", Timespec::default()),
                Err(VfsError::NotFound),
                "mkdir under private object: {description}"
            );
            assert_eq!(
                vfs.set_protection(object, 0, Timespec::default()),
                Err(VfsError::NotFound),
                "mutate private object: {description}"
            );
        }
    }
    let reader = vfs.snapshot_open(fixture.snapshot, 8).unwrap();
    let mut old = vec![0; PAYLOAD.len() + 1];
    assert_eq!(
        vfs.snapshot_read(reader, fixture.file, 0, &mut old)
            .unwrap(),
        PAYLOAD.len()
    );
    assert_eq!(
        &old[..PAYLOAD.len()],
        PAYLOAD,
        "snapshot changed: {description}"
    );
    vfs.snapshot_close(reader).unwrap();
}
fn campaign(tree: bool, alias: bool) {
    let fixture = fixture(tree);
    let mut vfs = mount(RecordingBackend::new(fixture.image.clone()));
    let reader = vfs.snapshot_open(fixture.snapshot, 8).unwrap();
    let name = if alias {
        format!(".afſpluſ-restore-{reader:016x}-{:016x}-00", fixture.source)
    } else {
        "Final".into()
    };
    restore(&mut vfs, reader, fixture.source, &name, tree);
    let (_, log) = vfs.into_volume().into_device().into_parts();
    let mut expected = 0;
    let mut max_tail = 0;
    let mut tail = 0;
    for cut in 0..=log.len() {
        if cut > 0 {
            match &log[cut - 1] {
                RecordedOp::Flush => tail = 0,
                RecordedOp::Write { .. } => tail += 1,
            }
        }
        max_tail = max_tail.max(tail);
        assert!(
            tail <= MAX_TAIL,
            "tail {tail} exceeds deliberate budget at cut{cut}"
        );
        expected += (1usize << tail) + 3 * tail;
    }
    println!("restore crash campaign tree={tree} alias={alias} operations={} cuts={} max_tail={max_tail} modeled_states={expected}",log.len(),log.len()+1);
    let mut counts = Counts::default();
    for cut in 0..=log.len() {
        for_each_crash_state_with_budget(&fixture.image, &log, cut, MAX_TAIL, |state| {
            counts.states += 1;
            counts.tears += usize::from(state.description.contains("torn"));
            verify_state(
                state.image,
                &fixture,
                &name,
                tree,
                &state.description,
                &mut counts,
            );
        });
    }
    assert_eq!(counts.states, expected);
    assert!(counts.tears > 0 && counts.empty_private > 0 && counts.complete > 0);
    assert_eq!(
        counts.unmarked_private, 0,
        "never persist an unmarked staging object"
    );
    println!("PASS all{} cuts/{} states ({} representative tears), {} private-object checks ({} empty), {} complete states",log.len()+1,counts.states,counts.tears,counts.private_objects,counts.empty_private,counts.complete);
}
#[test]
fn file_restore_powercuts_cover_every_persisted_prefix_loss_subset_and_tear() {
    campaign(false, false)
}
#[test]
fn tree_restore_powercuts_cover_every_persisted_prefix_loss_subset_and_tear() {
    campaign(true, false)
}
#[test]
fn unicode_casefold_file_stage_never_occupies_the_final_comparison_key() {
    campaign(false, true)
}
