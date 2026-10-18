use afsplus_block::MemoryBackend;
use afsplus_core::volume::{AttributeWriteMode, SnapshotWorkLimits, Volume};
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{RestoreRecoveryLimits, Vfs, VfsError};
const MARKER: &str = "system.restore.stage";
fn limits() -> RestoreRecoveryLimits {
    RestoreRecoveryLimits {
        scan_entries: 2,
        work_steps: 1,
        max_depth: 16,
    }
}
fn fixture() -> Vfs<MemoryBackend> {
    let mut dev = MemoryBackend::new(4096, 4096);
    mkfs_with_snapshots_and_security_descriptors(
        &mut dev,
        &MkfsParams {
            uuid: [0x93; 16],
            label: "Recovery".into(),
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
    mount(dev)
}
fn mount(dev: MemoryBackend) -> Vfs<MemoryBackend> {
    Vfs::mount_with_snapshot_limits(
        dev,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 64,
        },
    )
    .unwrap()
}
fn mark(volume: &mut Volume<MemoryBackend>, object: u64, root: u64) {
    volume
        .set_attributes(
            object,
            &[(MARKER, Some(&root.to_le_bytes()))],
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
}
#[test]
fn bounded_reclaim_resumes_after_remount_and_keeps_snapshot_data() {
    let mut volume = fixture().into_volume();
    let stage = volume
        .create_directory(OBJECT_ROOT, "arbitrary-private-name", Timespec::default())
        .unwrap();
    mark(&mut volume, stage, stage);
    let file = volume
        .create_file_in_directory(stage, "data", &[], Timespec::default())
        .unwrap();
    mark(&mut volume, file, stage);
    for n in 0..12 {
        volume
            .write_file_at(file, n * 8192, &[n as u8; 4096], Timespec::default())
            .unwrap();
    }
    let snapshot = volume.snapshot_create(Timespec::default()).unwrap();
    let visible = volume
        .create_file_in_root(".afsplus-restore-user-file", b"public", Timespec::default())
        .unwrap();
    let mut vfs = mount(volume.into_device());
    let mut cursor = Vec::new();
    let mut calls = 0;
    let mut advanced = 0;
    loop {
        let report = vfs
            .restore_reclaim_step(OBJECT_ROOT, &cursor, limits(), Timespec::default())
            .unwrap();
        assert!(report.scanned <= 2 && report.removed <= 1);
        assert_eq!(report.blocked, 0);
        advanced += report.advanced;
        calls += 1;
        if report.done {
            break;
        }
        cursor = report.cursor;
        vfs = mount(vfs.into_volume().into_device());
        assert!(calls < 100);
    }
    assert!(advanced > 2 && calls > 12);
    let mut volume = vfs.into_volume();
    assert!(volume.stat(stage).unwrap().is_none());
    assert!(volume.stat(file).unwrap().is_none());
    assert_eq!(volume.read_file(visible).unwrap(), b"public");
    let mut vfs = mount(volume.into_device());
    let reader = vfs.snapshot_open(snapshot, 8).unwrap();
    let mut bytes = [0; 4096];
    assert_eq!(
        vfs.snapshot_read(reader, file, 11 * 8192, &mut bytes)
            .unwrap(),
        4096
    );
    assert_eq!(bytes, [11; 4096]);
}
#[test]
fn malformed_and_unmarked_children_block_only_their_stage() {
    let mut volume = fixture().into_volume();
    let malformed = volume
        .create_file_in_root("bad", b"keep", Timespec::default())
        .unwrap();
    volume
        .set_attributes(
            malformed,
            &[(MARKER, Some(b"bad"))],
            AttributeWriteMode::Create,
            Timespec::default(),
        )
        .unwrap();
    let tree = volume
        .create_directory(OBJECT_ROOT, "tree", Timespec::default())
        .unwrap();
    mark(&mut volume, tree, tree);
    let child = volume
        .create_file_in_directory(tree, "unmarked", b"keep-child", Timespec::default())
        .unwrap();
    let good = volume
        .create_file_in_root("not-a-stage-filename", b"discard", Timespec::default())
        .unwrap();
    mark(&mut volume, good, good);
    let mut vfs = mount(volume.into_device());
    let mut cursor = Vec::new();
    let final_report = loop {
        let r = vfs
            .restore_reclaim_step(OBJECT_ROOT, &cursor, limits(), Timespec::default())
            .unwrap();
        if r.done {
            break r;
        }
        cursor = r.cursor;
    };
    assert_eq!(final_report.blocked, 2);
    let mut volume = vfs.into_volume();
    assert_eq!(volume.read_file(malformed).unwrap(), b"keep");
    assert_eq!(volume.read_file(child).unwrap(), b"keep-child");
    assert!(volume.stat(tree).unwrap().is_some());
    assert!(volume.stat(good).unwrap().is_none());
}
#[test]
fn published_derived_markers_are_never_reclaimed_and_stale_cursor_refuses() {
    let mut volume = fixture().into_volume();
    let published = volume
        .create_directory(OBJECT_ROOT, "published", Timespec::default())
        .unwrap();
    let child = volume
        .create_file_in_directory(published, "child", b"published", Timespec::default())
        .unwrap();
    mark(&mut volume, child, published);
    let mut vfs = mount(volume.into_device());
    let report = vfs
        .restore_reclaim_step(OBJECT_ROOT, &[], limits(), Timespec::default())
        .unwrap();
    assert!(!report.done);
    let mut volume = vfs.into_volume();
    volume
        .create_file_in_root("changed", b"x", Timespec::default())
        .unwrap();
    let mut vfs = mount(volume.into_device());
    assert!(matches!(
        vfs.restore_reclaim_step(OBJECT_ROOT, &report.cursor, limits(), Timespec::default()),
        Err(VfsError::Stale)
    ));
    let mut cursor = Vec::new();
    loop {
        let r = vfs
            .restore_reclaim_step(OBJECT_ROOT, &cursor, limits(), Timespec::default())
            .unwrap();
        assert_eq!(r.removed, 0);
        if r.done {
            break;
        }
        cursor = r.cursor;
    }
    assert_eq!(vfs.into_volume().read_file(child).unwrap(), b"published");
}
#[test]
fn scope_cursor_cannot_be_reused_for_another_directory_or_forged_edge() {
    let mut volume = fixture().into_volume();
    let scope = volume
        .create_directory(OBJECT_ROOT, "scope", Timespec::default())
        .unwrap();
    let stage = volume
        .create_file_in_directory(scope, "private", b"secret", Timespec::default())
        .unwrap();
    mark(&mut volume, stage, stage);
    let mut vfs = mount(volume.into_device());
    let mut one = limits();
    one.scan_entries = 1;
    let r = vfs
        .restore_reclaim_step(OBJECT_ROOT, &[], one, Timespec::default())
        .unwrap();
    assert!(!r.done);
    assert!(matches!(
        vfs.restore_reclaim_step(scope, &r.cursor, one, Timespec::default()),
        Err(VfsError::Stale)
    ));
    let mut bad = r.cursor;
    bad[20..28].copy_from_slice(&scope.to_le_bytes());
    assert!(vfs
        .restore_reclaim_step(scope, &bad, one, Timespec::default())
        .is_err());
    let mut cursor = Vec::new();
    loop {
        let r = vfs
            .restore_reclaim_step(scope, &cursor, one, Timespec::default())
            .unwrap();
        if r.done {
            break;
        }
        cursor = r.cursor;
    }
    assert!(vfs.into_volume().stat(stage).unwrap().is_none());
}

#[test]
fn full_volume_reclaims_private_data_incrementally_without_orphan_membership() {
    let mut volume = fixture().into_volume();
    let stage = volume
        .create_file_in_root("private", &vec![0x79; 512 * 1024], Timespec::default())
        .unwrap();
    mark(&mut volume, stage, stage);
    let before = volume.available_blocks();
    let mut filled = 0;
    for n in 0..256 {
        match volume.create_file_in_root(
            &format!("filler-{n}"),
            &vec![0x23; 128 * 1024],
            Timespec::default(),
        ) {
            Ok(_) => filled += 1,
            Err(afsplus_core::CoreError::NoSpace) => break,
            Err(e) => panic!("unexpected fill error {e:?}"),
        }
    }
    assert!(filled > 0 && volume.available_blocks() < before);
    let low = volume.available_blocks();
    let mut vfs = mount(volume.into_device());
    let mut cursor = Vec::new();
    let mut removed = 0;
    for _ in 0..500 {
        let r = vfs
            .restore_reclaim_step(OBJECT_ROOT, &cursor, limits(), Timespec::default())
            .unwrap();
        assert!(r.removed <= 1 && r.scanned <= 2);
        removed += r.removed;
        if r.done {
            break;
        }
        cursor = r.cursor;
        vfs = mount(vfs.into_volume().into_device());
    }
    let mut volume = vfs.into_volume();
    assert_eq!(removed, 1);
    assert!(volume.stat(stage).unwrap().is_none());
    assert!(volume.available_blocks() > low);
}

#[test]
fn published_child_survives_after_its_inactive_marker_root_is_deleted() {
    let mut volume = fixture().into_volume();
    let root = volume
        .create_directory(OBJECT_ROOT, "published", Timespec::default())
        .unwrap();
    let child = volume
        .create_file_in_directory(root, "child", b"preserve", Timespec::default())
        .unwrap();
    mark(&mut volume, child, root);
    volume
        .link_file(child, OBJECT_ROOT, "retained", Timespec::default())
        .unwrap();
    volume
        .delete_file(root, "child", Timespec::default())
        .unwrap();
    volume
        .remove_directory(OBJECT_ROOT, "published", Timespec::default())
        .unwrap();
    let mut vfs = mount(volume.into_device());
    let r = vfs
        .restore_reclaim_step(OBJECT_ROOT, &[], limits(), Timespec::default())
        .unwrap();
    assert!(r.done);
    assert_eq!((r.removed, r.blocked), (0, 0));
    assert_eq!(vfs.into_volume().read_file(child).unwrap(), b"preserve");
}

#[test]
fn feature_gate_refuses_recovery_without_mutating_an_ordinary_volume() {
    let mut dev = MemoryBackend::new(4096, 4096);
    afsplus_core::mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x94; 16],
            label: "NoSnapshots".into(),
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
    let vfs = Vfs::mount(dev, MountOptions::default()).unwrap();
    let mut volume = vfs.into_volume();
    let object = volume
        .create_file_in_root("ordinary", b"keep", Timespec::default())
        .unwrap();
    mark(&mut volume, object, object);
    let generation = volume.checkpoint().generation;
    let mut vfs = Vfs::new(volume);
    assert!(matches!(
        vfs.restore_reclaim_step(OBJECT_ROOT, &[], limits(), Timespec::default()),
        Err(VfsError::NotSupported)
    ));
    let mut volume = vfs.into_volume();
    assert_eq!(volume.checkpoint().generation, generation);
    assert_eq!(volume.read_file(object).unwrap(), b"keep");
}

#[test]
fn pending_writer_is_refused_without_publishing_unbudgeted_changes() {
    let mut vfs = fixture();
    vfs.set_durability(afsplus_vfs::Durability::DELAYED)
        .unwrap();
    vfs.create_file(OBJECT_ROOT, "pending", Timespec::default())
        .unwrap();
    let generation = vfs.generation();
    assert!(vfs.changes_pending());
    assert!(matches!(
        vfs.restore_reclaim_step(OBJECT_ROOT, &[], limits(), Timespec::default()),
        Err(VfsError::Busy)
    ));
    assert_eq!(vfs.generation(), generation);
    assert!(vfs.changes_pending());
}

fn reclaim_full_admitted_depth(depth: usize, file: bool) {
    use afsplus_format::object::ObjectType;
    let mut volume = fixture().into_volume();
    let root = volume
        .create_restore_stage(
            OBJECT_ROOT,
            "stage",
            if file {
                ObjectType::File
            } else {
                ObjectType::Directory
            },
            None,
            Timespec::default(),
        )
        .unwrap();
    let mut parent = root;
    for _ in 1..depth {
        parent = volume
            .create_restore_stage(
                parent,
                "next",
                ObjectType::Directory,
                Some(root),
                Timespec::default(),
            )
            .unwrap();
    }
    let mut vfs = mount(volume.into_device());
    let mut cursor = Vec::new();
    let mut done = false;
    for _ in 0..128 {
        let r = vfs
            .restore_reclaim_step(
                OBJECT_ROOT,
                &cursor,
                RestoreRecoveryLimits {
                    scan_entries: 64,
                    work_steps: 8,
                    max_depth: depth,
                },
                Timespec::default(),
            )
            .unwrap();
        assert_eq!(
            r.blocked, 0,
            "valid stage depth {depth} below live scope must fit admitted budget"
        );
        if r.done {
            done = true;
            break;
        }
        cursor = r.cursor;
    }
    assert!(done);
    assert!(vfs.into_volume().stat(root).unwrap().is_none());
}

#[test]
fn valid_sixty_four_level_stage_fits_scoped_depth_budget() {
    reclaim_full_admitted_depth(64, false);
}

#[test]
fn depth_one_reclaims_standalone_file_and_empty_stage_directory() {
    reclaim_full_admitted_depth(1, true);
    reclaim_full_admitted_depth(1, false);
}

#[test]
fn forged_cursor_cannot_enter_a_stage_below_its_root() {
    let mut volume = fixture().into_volume();
    let stage = volume
        .create_directory(OBJECT_ROOT, "stage", Timespec::default())
        .unwrap();
    let plain = volume
        .create_directory(OBJECT_ROOT, "plain", Timespec::default())
        .unwrap();
    // A marked descendant outside its stage: a scan blocks it.
    let child = volume
        .create_file_in_directory(plain, "child", b"secret", Timespec::default())
        .unwrap();
    mark(&mut volume, stage, stage);
    mark(&mut volume, child, stage);
    let mut vfs = mount(volume.into_device());
    let mut one = limits();
    one.scan_entries = 1;
    let r = vfs
        .restore_reclaim_step(OBJECT_ROOT, &[], one, Timespec::default())
        .unwrap();
    let generation = r.cursor[28..36].to_vec();
    let mut forged = b"RRC1".to_vec();
    forged.extend([0x93; 16]);
    forged.extend(OBJECT_ROOT.to_le_bytes());
    forged.extend(&generation);
    for value in [0u64, 3] {
        forged.extend(value.to_le_bytes());
    }
    for value in [0, OBJECT_ROOT, 2, 0, 0] {
        forged.extend(value.to_le_bytes());
    }
    for value in [OBJECT_ROOT, plain, 0, 0, 5] {
        forged.extend(value.to_le_bytes());
    }
    forged.extend(b"plain");
    for value in [plain, child, 0, stage, 5] {
        forged.extend(value.to_le_bytes());
    }
    forged.extend(b"child");
    forged.extend(afsplus_format::crc32c::crc32c(&forged).to_le_bytes());
    assert!(matches!(
        vfs.restore_reclaim_step(OBJECT_ROOT, &forged, one, Timespec::default()),
        Err(VfsError::Invalid)
    ));
    assert!(vfs.into_volume().stat(child).unwrap().is_some());
}
