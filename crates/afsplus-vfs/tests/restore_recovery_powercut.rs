// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! Real ADR-133 maintenance writes, not synthetic per-call failure points.
//! Whole-tail subsets model loss/reordering; sampled single in-order tears
//! at64/2048/4064 bytes do not cover simultaneous or sub-block reordered tears.
use afsplus_block::{
    for_each_crash_state_with_budget, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend,
};
use afsplus_core::volume::{AttributeWriteMode, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{object::ObjectType, Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, RestoreRecoveryLimits, Stat, Vfs, VfsError};

const TAIL_BUDGET: usize = 12;
const MARKER: &str = "system.restore.stage";
const ORDINARY: &[u8] = b"ordinary-neighbor\0unchanged";
const PUBLISHED: &[u8] = b"published\0derived-marker-child";
fn mount<D: BlockDevice>(device: D) -> Vfs<D> {
    let mut vfs = Vfs::mount_with_snapshot_limits(
        device,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap();
    // The recorded campaign is exclusively the requested maintenance, and
    // verification reads/close must not run unrelated background maintenance.
    vfs.set_idle_maintenance(false);
    vfs.set_inline_maintenance(false);
    vfs
}
fn limits() -> RestoreRecoveryLimits {
    RestoreRecoveryLimits {
        scan_entries: 64,
        work_steps: 1,
        max_depth: 8,
    }
}
struct Fixture {
    image: MemoryBackend,
    snapshot: u64,
    stage: u64,
    file: u64,
    nodes: Vec<u64>,
    payload: Vec<u8>,
    ordinary: u64,
    published: u64,
    published_child: u64,
    ordinary_stat: Stat,
    published_stat: Stat,
    published_child_stat: Stat,
}
fn fixture(tree: bool) -> Fixture {
    let mut image = MemoryBackend::new(4096, 1024);
    mkfs_with_snapshots_and_security_descriptors(
        &mut image,
        &MkfsParams {
            uuid: [0x96; 16],
            label: "RecoveryCuts".into(),
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
    let ordinary = volume
        .create_file_in_root("Ordinary", ORDINARY, Timespec::default())
        .unwrap();
    volume
        .set_object_comment(ordinary, "ordinary comment", Timespec::default())
        .unwrap();
    volume
        .set_attributes(
            ordinary,
            &[("user.keep", Some(b"opaque\0neighbor"))],
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
    let published = volume
        .create_restore_stage(
            OBJECT_ROOT,
            "publish-stage",
            ObjectType::Directory,
            None,
            Timespec::default(),
        )
        .unwrap();
    let published_child = volume
        .create_restore_stage(
            published,
            "cfg",
            ObjectType::File,
            Some(published),
            Timespec::default(),
        )
        .unwrap();
    volume
        .write_file_at(published_child, 0, PUBLISHED, Timespec::default())
        .unwrap();
    volume
        .publish_restored_object(
            OBJECT_ROOT,
            "publish-stage",
            "Published",
            Timespec::default(),
        )
        .unwrap();
    assert!(volume.attribute(published, MARKER).unwrap().is_none());
    assert_eq!(
        volume.attribute(published_child, MARKER).unwrap(),
        Some(published.to_le_bytes().to_vec())
    );
    let stage = volume
        .create_restore_stage(
            OBJECT_ROOT,
            "Retained",
            if tree {
                ObjectType::Directory
            } else {
                ObjectType::File
            },
            None,
            Timespec::default(),
        )
        .unwrap();
    let mut nodes = vec![stage];
    let file = if tree {
        let file = volume
            .create_restore_stage(
                stage,
                "payload",
                ObjectType::File,
                Some(stage),
                Timespec::default(),
            )
            .unwrap();
        let empty = volume
            .create_restore_stage(
                stage,
                "empty",
                ObjectType::Directory,
                Some(stage),
                Timespec::default(),
            )
            .unwrap();
        nodes.extend([file, empty]);
        file
    } else {
        stage
    };
    // Three logical runs separated by holes force real incremental shrink.
    let mut payload = vec![0; 5 * 4096];
    for n in 0..3usize {
        let data = vec![0x31 + n as u8; 4096];
        let offset = n * 8192;
        volume
            .write_file_at(file, offset as u64, &data, Timespec::default())
            .unwrap();
        payload[offset..offset + 4096].copy_from_slice(&data);
    }
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let mut vfs = mount(volume.into_device());
    let ordinary_stat = vfs.stat(ordinary).unwrap();
    let published_stat = vfs.stat(published).unwrap();
    let published_child_stat = vfs.stat(published_child).unwrap();
    Fixture {
        image: vfs.into_volume().into_device(),
        snapshot,
        stage,
        file,
        nodes,
        payload,
        ordinary,
        published,
        published_child,
        ordinary_stat,
        published_stat,
        published_child_stat,
    }
}
fn public_bytes(vfs: &mut Vfs<MemoryBackend>, object: u64, expected: &[u8]) {
    let file = vfs.open_file(object, AccessMode::ReadOnly).unwrap();
    let mut bytes = vec![0; expected.len() + 1];
    assert_eq!(vfs.read(file, 0, &mut bytes).unwrap(), expected.len());
    assert_eq!(&bytes[..expected.len()], expected);
    vfs.close(file).unwrap();
}
fn neighbors(vfs: &mut Vfs<MemoryBackend>, f: &Fixture) {
    assert_eq!(vfs.lookup(OBJECT_ROOT, "Ordinary").unwrap(), f.ordinary);
    assert_eq!(vfs.lookup(OBJECT_ROOT, "Published").unwrap(), f.published);
    assert_eq!(vfs.lookup(f.published, "cfg").unwrap(), f.published_child);
    assert_eq!(vfs.stat(f.ordinary).unwrap(), f.ordinary_stat);
    assert_eq!(vfs.stat(f.published).unwrap(), f.published_stat);
    assert_eq!(vfs.stat(f.published_child).unwrap(), f.published_child_stat);
    assert_eq!(vfs.comment(f.ordinary).unwrap(), "ordinary comment");
    assert_eq!(
        vfs.attribute(f.ordinary, "user.keep").unwrap(),
        Some(b"opaque\0neighbor".to_vec())
    );
    public_bytes(vfs, f.ordinary, ORDINARY);
    public_bytes(vfs, f.published_child, PUBLISHED);
}
fn historical(vfs: &mut Vfs<MemoryBackend>, f: &Fixture) {
    let reader = vfs.snapshot_open(f.snapshot, 8).unwrap();
    assert_eq!(
        vfs.snapshot_stat(reader, f.file).unwrap().size,
        f.payload.len() as u64
    );
    let mut bytes = vec![0; f.payload.len() + 1];
    assert_eq!(
        vfs.snapshot_read(reader, f.file, 0, &mut bytes).unwrap(),
        f.payload.len()
    );
    assert_eq!(&bytes[..f.payload.len()], f.payload);
    assert_eq!(
        vfs.snapshot_lookup(reader, OBJECT_ROOT, "Retained")
            .unwrap(),
        f.stage
    );
    vfs.snapshot_close(reader).unwrap();
}
#[derive(Default)]
struct Counts {
    states: usize,
    tears: usize,
    private: usize,
    absent: usize,
    resumed: usize,
    advanced: usize,
    removed: usize,
}
fn verify(image: MemoryBackend, f: &Fixture, description: &str, counts: &mut Counts) {
    // Each modeled image is freshly mounted. Never continue a failed backend.
    let mut raw = mount(image).into_volume();
    for object in &f.nodes {
        match raw.stat(*object).unwrap() {
            Some(_) => {
                assert_eq!(
                    raw.attribute(*object, MARKER).unwrap(),
                    Some(f.stage.to_le_bytes().to_vec()),
                    "marker {description}"
                );
                counts.private += 1;
            }
            None => counts.absent += 1,
        }
    }
    let mut vfs = mount(raw.into_device());
    assert_eq!(
        vfs.lookup(OBJECT_ROOT, "Retained"),
        Err(VfsError::NotFound),
        "private path {description}"
    );
    for object in &f.nodes {
        assert_eq!(
            vfs.stat(*object),
            Err(VfsError::NotFound),
            "direct stat {object}: {description}"
        );
        assert_eq!(
            vfs.open_file(*object, AccessMode::ReadOnly),
            Err(VfsError::NotFound),
            "direct open {description}"
        );
        assert_eq!(
            vfs.set_protection(*object, 0, Timespec::default()),
            Err(VfsError::NotFound),
            "direct mutation {description}"
        );
    }
    neighbors(&mut vfs, f);
    historical(&mut vfs, f);
    // Cursor loss is explicitly safe. Restart on every crash image, and
    // prove bounded recovery reaches completion for every modeled state.
    let mut cursor = Vec::new();
    let mut done = false;
    for _ in 0..16 {
        let report = vfs
            .restore_reclaim_step(OBJECT_ROOT, &cursor, limits(), Timespec::default())
            .unwrap();
        assert!(
            report.scanned <= 64 && report.removed <= 1 && report.advanced <= 1,
            "budget {description}"
        );
        assert_eq!(report.blocked, 0, "valid stage blocked {description}");
        if report.done {
            done = true;
            break;
        }
        cursor = report.cursor;
    }
    assert!(done, "resume did not complete: {description}");
    counts.resumed += 1;
    neighbors(&mut vfs, f);
    historical(&mut vfs, f);
    let mut raw = vfs.into_volume();
    for object in &f.nodes {
        assert!(
            raw.stat(*object).unwrap().is_none(),
            "resumed object remains {description}"
        );
    }
}
fn campaign(tree: bool) {
    let f = fixture(tree);
    let mut vfs = mount(RecordingBackend::new(f.image.clone()));
    let mut cursor = Vec::new();
    let mut done = false;
    let mut recorded = Counts::default();
    for _ in 0..16 {
        let r = vfs
            .restore_reclaim_step(OBJECT_ROOT, &cursor, limits(), Timespec::default())
            .unwrap();
        assert_eq!(r.blocked, 0);
        recorded.advanced += r.advanced as usize;
        recorded.removed += r.removed as usize;
        if r.done {
            done = true;
            break;
        }
        cursor = r.cursor;
    }
    assert!(done && recorded.advanced >= 2 && recorded.removed == f.nodes.len());
    let (_, log) = vfs.into_volume().into_device().into_parts();
    let writes = log
        .iter()
        .filter(|o| matches!(o, RecordedOp::Write { .. }))
        .count();
    let flushes = log.len() - writes;
    assert!(writes > 0 && flushes > 0);
    let mut expected = 0usize;
    let mut tail = 0;
    let mut max_tail = 0;
    for cut in 0..=log.len() {
        if cut > 0 {
            match log[cut - 1] {
                RecordedOp::Flush => tail = 0,
                RecordedOp::Write { .. } => tail += 1,
            }
        }
        max_tail = max_tail.max(tail);
        assert!(
            tail <= TAIL_BUDGET,
            "tail{tail} exceeds explicitbudget atcut{cut}"
        );
        expected += (1usize << tail) + 3 * tail;
    }
    println!("reclaim tree={tree} writes={writes} flushes={flushes} operations={} cuts={} max_tail={max_tail} budget={TAIL_BUDGET} modeled_states={expected} advanced={} removed={}",log.len(),log.len()+1,recorded.advanced,recorded.removed);
    let mut counts = Counts::default();
    for cut in 0..=log.len() {
        for_each_crash_state_with_budget(&f.image, &log, cut, TAIL_BUDGET, |state| {
            counts.states += 1;
            counts.tears += usize::from(state.description.contains("torn"));
            verify(state.image, &f, &state.description, &mut counts);
        });
    }
    assert_eq!(counts.states, expected);
    assert_eq!(counts.resumed, expected);
    assert!(counts.tears > 0 && counts.private > 0 && counts.absent > 0);
    println!("PASS reclaim tree={tree} states={} representative_tears={} private_checks={} absent_checks={} resumed_complete={}",counts.states,counts.tears,counts.private,counts.absent,counts.resumed);
}
#[test]
fn private_file_reclaim_every_operation_cut_tail_subset_and_sampled_tear() {
    campaign(false);
}
#[test]
fn private_tree_reclaim_every_operation_cut_tail_subset_and_sampled_tear() {
    campaign(true);
}
