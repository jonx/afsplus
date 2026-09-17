//! ExplainBlock against three independent witnesses on populated images:
//! the checker's committed state and findings, the live bitmap, and the file
//! bytes the core returns. The explain walk shares no code with the checker,
//! so every agreement below is a cross-check and every disagreement a
//! finding about one of the two.
use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_check::explain::{Allocation, BlockRole, Explainer, VolumeTree};
use afsplus_core::mount::select_checkpoint;
use afsplus_core::verify::load_committed_state;
use afsplus_core::{mkfs_with_security_descriptors, mount, MkfsParams, NamePolicy};
use afsplus_format::header::{block_type, BlockHeader};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};

const BLOCK: usize = 4096;

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 3,
    }
}

fn formatted(log_slots: u16) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BLOCK, 2048);
    mkfs_with_security_descriptors(
        &mut dev,
        &MkfsParams {
            uuid: [0xe7; 16],
            label: "Explain".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    dev
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(17).wrapping_add(seed))
        .collect()
}

struct Populated {
    dev: MemoryBackend,
    direct: u64,
    sparse: u64,
    clone: u64,
    dir: u64,
    orphan: u64,
    /// Segments 1 and 2 of the deleted file's chain, left unproven.
    leaked: Vec<u64>,
}

/// One image with every case: direct and extent-tree files, a sparse file, a
/// directory with enough entries for more than one tree node, a symlink, a
/// clone that shares data and owns a copied three-segment descriptor chain,
/// an orphan waiting in directory 2, a reclaim quarantine, and a file
/// unlinked while the middle segment of its chain was damaged.
fn populate() -> Populated {
    let mut volume = mount(formatted(8)).unwrap();
    let direct = volume
        .create_file_in_directory(OBJECT_ROOT, "direct", &pattern(6000, 1), time(2))
        .unwrap();
    let sparse = volume
        .create_file_in_directory(OBJECT_ROOT, "sparse", &pattern(4096, 2), time(2))
        .unwrap();
    volume
        .write_file_at(sparse, 40 * BLOCK as u64, &pattern(9000, 3), time(2))
        .unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(2))
        .unwrap();
    for index in 0..300 {
        volume
            .create_file_in_directory(
                dir,
                &format!("entry-with-a-long-name-{index:04}"),
                b"",
                time(2),
            )
            .unwrap();
    }
    volume
        .create_symlink(OBJECT_ROOT, "link", "direct", time(2))
        .unwrap();
    let descriptor = pattern(9000, 9);
    volume
        .set_security_descriptor(sparse, 0x7fff_0001, 1, &descriptor, time(3))
        .unwrap();
    let clone = volume
        .clone_file(sparse, OBJECT_ROOT, "clone", time(4))
        .unwrap();
    let doomed = volume
        .create_file_in_directory(OBJECT_ROOT, "doomed", &pattern(100, 4), time(4))
        .unwrap();
    volume
        .set_security_descriptor(doomed, 0x7fff_0002, 1, &descriptor, time(4))
        .unwrap();
    let orphan_source = volume
        .create_file_in_directory(OBJECT_ROOT, "orphaned", &pattern(5000, 5), time(4))
        .unwrap();
    assert_eq!(
        volume
            .orphan_file(OBJECT_ROOT, "orphaned", time(5))
            .unwrap(),
        orphan_source
    );
    let mut dev = volume.into_device();

    // Damage the middle segment of the doomed file's chain, found through
    // explain itself on the still-clean image.
    let explainer = Explainer::load(&mut dev).unwrap();
    assert_eq!(explainer.problems, Vec::<String>::new());
    let mut chain = BTreeMap::new();
    for lba in 0..dev.total_blocks() {
        for role in explainer.explain_block(&mut dev, lba).unwrap().roles {
            if let BlockRole::SecuritySegment { object_id, index } = role {
                if object_id == doomed {
                    chain.insert(index, lba);
                }
            }
        }
    }
    assert_eq!(chain.len(), 3);
    let mut block = vec![0u8; BLOCK];
    dev.read_block(chain[&1], &mut block).unwrap();
    let header = BlockHeader::verify(&block, block_type::SECURITY_DESCRIPTOR).unwrap();
    BlockHeader {
        owner: 0xdead,
        ..header
    }
    .seal(&mut block);
    dev.write_block(chain[&1], &block).unwrap();

    let mut volume = mount(dev).unwrap();
    volume.delete_file(OBJECT_ROOT, "doomed", time(6)).unwrap();
    while volume
        .cleanup_orphan(doomed, time(6))
        .unwrap()
        .still_pending
    {}
    // A plain deletion, so the reclaim queue holds ordinary runs too.
    volume
        .delete_file(dir, "entry-with-a-long-name-0007", time(6))
        .unwrap();
    Populated {
        dev: volume.into_device(),
        direct,
        sparse,
        clone,
        dir,
        orphan: orphan_source,
        leaked: vec![chain[&1], chain[&2]],
    }
}

#[test]
fn every_block_explanation_agrees_with_the_checker_the_bitmap_and_the_file_bytes() {
    let mut image = populate();
    let dev = &mut image.dev;
    let explainer = Explainer::load(dev).unwrap();
    assert_eq!(explainer.problems, Vec::<String>::new());

    // Witness one: the checker's own committed state and findings.
    let mut block = vec![0u8; BLOCK];
    dev.read_block(0, &mut block).unwrap();
    let ident = Identification::decode(&block).unwrap();
    let selection = select_checkpoint(dev, &ident).unwrap();
    assert_eq!(explainer.generation, selection.chosen.generation);
    let state = load_committed_state(dev, &ident, &selection.chosen).unwrap();
    let metadata: BTreeSet<u64> = state.metadata_blocks.iter().copied().collect();
    let data: BTreeSet<u64> = state.data_blocks.iter().copied().collect();
    let quarantined: BTreeSet<u64> = state
        .reclaim_runs
        .iter()
        .flat_map(|run| run.start..run.start + u64::from(run.blocks))
        .collect();
    let report = check_device(dev);
    let leaks: BTreeSet<u64> = report
        .errors
        .iter()
        .chain(&report.warnings)
        .filter_map(|finding| {
            finding
                .strip_prefix("block ")?
                .strip_suffix(" is allocated but owned by nothing (leak)")?
                .parse()
                .ok()
        })
        .collect();
    assert!(
        !quarantined.is_empty(),
        "the image holds a reclaim quarantine"
    );
    assert_eq!(
        leaks,
        image.leaked.iter().copied().collect::<BTreeSet<_>>(),
        "the checker names exactly the two unproven segments"
    );

    let geo = ident.geometry();
    let mut volume_bytes: BTreeMap<(u64, u64), u64> = BTreeMap::new();
    let mut seen = BTreeMap::<&'static str, u64>::new();
    let mut unowned = BTreeSet::new();
    for lba in 0..dev.total_blocks() {
        let explanation = explainer.explain_block(dev, lba).unwrap();
        // Witness two: the live bitmap, through the core's loader.
        let expected_allocation = if !geo.is_allocatable(lba) {
            Allocation::Reserved
        } else if state.bitmaps.is_allocated(lba) {
            Allocation::Allocated
        } else {
            Allocation::Free
        };
        assert_eq!(explanation.allocation, expected_allocation, "block {lba}");

        let mut is_data = false;
        let mut is_metadata = false;
        let mut is_quarantined = false;
        for role in &explanation.roles {
            let class = match role {
                BlockRole::Data {
                    object_id,
                    logical_block,
                    unwritten,
                    ..
                } => {
                    is_data = true;
                    if !unwritten {
                        volume_bytes.insert((*object_id, *logical_block), lba);
                    }
                    "data"
                }
                BlockRole::Quarantined { .. } => {
                    is_quarantined = true;
                    "quarantined"
                }
                BlockRole::ObjectRecord { .. } => {
                    is_metadata = true;
                    "object record"
                }
                BlockRole::DirectoryNode { .. } => {
                    is_metadata = true;
                    "directory node"
                }
                BlockRole::ExtentNode { .. } => {
                    is_metadata = true;
                    "extent node"
                }
                BlockRole::SecuritySegment { .. } => {
                    is_metadata = true;
                    "security segment"
                }
                BlockRole::AttributeSegment { .. } => {
                    is_metadata = true;
                    "attribute segment"
                }
                BlockRole::VolumeTreeNode { .. }
                | BlockRole::ReclaimRoot
                | BlockRole::ReclaimTable
                | BlockRole::ReclaimSegment => {
                    is_metadata = true;
                    "volume structure"
                }
                BlockRole::Identification
                | BlockRole::CheckpointSlot { .. }
                | BlockRole::RegionDescriptorSlot { .. }
                | BlockRole::BitmapSlot { .. }
                | BlockRole::AllocationRootPool
                | BlockRole::IntentLogSlot { .. }
                | BlockRole::Reserved => "reserved",
            };
            *seen.entry(class).or_default() += 1;
        }
        assert_eq!(is_data, data.contains(&lba), "data verdict, block {lba}");
        assert_eq!(
            is_metadata,
            metadata.contains(&lba),
            "metadata verdict, block {lba}: {:?}",
            explanation.roles
        );
        assert_eq!(
            is_quarantined,
            quarantined.contains(&lba),
            "quarantine verdict, block {lba}"
        );
        // Ownership and the bitmap: an owned or quarantined block is
        // allocated or reserved, a free block has no role.
        if explanation.allocation == Allocation::Free {
            assert!(explanation.roles.is_empty(), "free block {lba} has roles");
        }
        if explanation.is_unowned() {
            unowned.insert(lba);
        }
        // A reachable metadata block describes itself with a valid checksum.
        if is_metadata {
            assert!(explanation.identity.unwrap().checksum_valid, "block {lba}");
        }
    }
    assert_eq!(unowned, leaks, "unowned blocks are the checker's leaks");
    for class in [
        "data",
        "quarantined",
        "object record",
        "directory node",
        "extent node",
        "security segment",
        "volume structure",
        "reserved",
    ] {
        assert!(
            seen.get(class).copied().unwrap_or(0) > 0,
            "no {class} block"
        );
    }

    // Literal expectations that no witness supplies.
    let first = explainer.explain_block(dev, 0).unwrap();
    assert_eq!(first.roles, vec![BlockRole::Identification]);
    assert_eq!(&first.identity.unwrap().magic, b"AFSI");
    let slots: Vec<bool> = (1..=2)
        .map(
            |lba| match explainer.explain_block(dev, lba).unwrap().roles[..] {
                [BlockRole::CheckpointSlot { selected, .. }] => selected,
                _ => panic!("checkpoint slot {lba}"),
            },
        )
        .collect();
    assert_eq!(slots.iter().filter(|selected| **selected).count(), 1);
    assert!(explainer.explain_block(dev, dev.total_blocks()).is_err());
    for leaked in &image.leaked {
        let explanation = explainer.explain_block(dev, *leaked).unwrap();
        assert!(explanation.is_unowned());
        assert_eq!(&explanation.identity.unwrap().magic, b"AFSX");
    }

    // The clone owns three segments of its own and shares its data with the
    // source: both objects appear on each shared data block.
    let mut segments: BTreeMap<u64, BTreeSet<u16>> = BTreeMap::new();
    let mut shared_blocks = 0;
    let mut tree_levels = BTreeSet::new();
    for lba in 0..dev.total_blocks() {
        let roles = explainer.explain_block(dev, lba).unwrap().roles;
        let owners: BTreeSet<u64> = roles
            .iter()
            .filter_map(|role| match role {
                BlockRole::Data {
                    object_id, shared, ..
                } => {
                    assert_eq!(
                        *shared,
                        roles.len() > 1 || *shared,
                        "block {lba}: {roles:?}"
                    );
                    Some(*object_id)
                }
                _ => None,
            })
            .collect();
        if owners.len() == 2 {
            assert_eq!(owners, BTreeSet::from([image.sparse, image.clone]));
            shared_blocks += 1;
        }
        for role in roles {
            match role {
                BlockRole::SecuritySegment { object_id, index } => {
                    assert!(segments.entry(object_id).or_default().insert(index));
                }
                BlockRole::DirectoryNode { object_id, level } if object_id == image.dir => {
                    tree_levels.insert(level);
                }
                BlockRole::DirectoryNode { object_id, .. } => {
                    assert!([OBJECT_ROOT, OBJECT_ORPHAN_DIRECTORY].contains(&object_id));
                }
                BlockRole::VolumeTreeNode {
                    kind: VolumeTree::SnapshotRegistry | VolumeTree::SnapshotLifetimes,
                    ..
                } => panic!("no snapshot tree on this volume"),
                _ => {}
            }
        }
    }
    assert_eq!(
        segments,
        BTreeMap::from([
            (image.sparse, BTreeSet::from([0, 1, 2])),
            (image.clone, BTreeSet::from([0, 1, 2])),
        ])
    );
    assert_eq!(shared_blocks, 4, "one block at 0 and three from block 40");
    assert!(tree_levels.len() > 1, "the directory tree has inner nodes");

    // Witness three: the bytes. Every written data block explain attributes
    // to (object, logical block) holds what the core reads at that offset.
    let mut volume = mount(std::mem::replace(dev, MemoryBackend::new(BLOCK, 16))).unwrap();
    let mut checked = 0;
    let mut expected = vec![0u8; BLOCK];
    let targets = [image.direct, image.sparse, image.clone, image.orphan];
    let mut raw = Vec::new();
    for ((object_id, logical), lba) in &volume_bytes {
        if targets.contains(object_id) {
            raw.push((*object_id, *logical, *lba));
        }
    }
    for (object_id, logical, lba) in raw {
        expected.fill(0);
        let read = volume
            .read_file_at(object_id, logical * BLOCK as u64, &mut expected)
            .unwrap();
        volume.device_mut().read_block(lba, &mut block).unwrap();
        assert_eq!(
            block[..read],
            expected[..read],
            "object {object_id} block {logical}"
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        2 + 4 + 4 + 2,
        "direct, sparse, clone and orphan blocks"
    );
}

#[test]
fn a_disagreeing_image_is_noticed() {
    // Negative control: clear the allocation of one owned block in the
    // explainer's own input and the agreement above cannot hold. The cheap
    // way to show it is a block the walk owns while a forged bitmap frees
    // it: explain then reports a role on a free block.
    let mut image = populate();
    let explainer = Explainer::load(&mut image.dev).unwrap();
    let mut owned_free = 0;
    let mut unowned = 0;
    for lba in 0..image.dev.total_blocks() {
        let explanation = explainer.explain_block(&mut image.dev, lba).unwrap();
        if explanation.allocation == Allocation::Free && !explanation.roles.is_empty() {
            owned_free += 1;
        }
        if explanation.is_unowned() {
            unowned += 1;
        }
    }
    // The clean walk has no owned free block and exactly the two leaks.
    assert_eq!((owned_free, unowned), (0, 2));

    // Reseal the directory's record under another object ID: the walk loses
    // the whole directory tree, and every block of it turns up unowned.
    let mut block = vec![0u8; BLOCK];
    let record = (0..image.dev.total_blocks())
        .find(|lba| {
            explainer
                .explain_block(&mut image.dev, *lba)
                .unwrap()
                .roles
                .contains(&BlockRole::ObjectRecord {
                    object_id: image.dir,
                })
        })
        .unwrap();
    image.dev.read_block(record, &mut block).unwrap();
    let header = BlockHeader::verify(&block, block_type::OBJECT).unwrap();
    BlockHeader {
        owner: 0xbad,
        ..header
    }
    .seal(&mut block);
    image.dev.write_block(record, &block).unwrap();
    let damaged = Explainer::load(&mut image.dev).unwrap();
    assert_eq!(damaged.problems.len(), 1, "{:?}", damaged.problems);
    let unowned_now = (0..image.dev.total_blocks())
        .filter(|lba| {
            damaged
                .explain_block(&mut image.dev, *lba)
                .unwrap()
                .is_unowned()
        })
        .count();
    assert!(unowned_now > 2 + 1, "the lost directory tree is unowned");
}

#[test]
fn on_a_snapshot_volume_unowned_blocks_are_the_ones_a_retained_view_owns() {
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MountOptions};

    let mut dev = MemoryBackend::new(BLOCK, 2048);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [0xe8; 16],
            label: "ExplainSnap".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let mut volume = mount_with_snapshot_limits(
        dev,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 8,
            reclaim_records: 8,
        },
    )
    .unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &pattern(3 * BLOCK, 1), time(2))
        .unwrap();
    volume.snapshot_create(time(3)).unwrap();
    // The live file moves on; the view keeps the three old data blocks.
    volume
        .write_file_at(file, 0, &pattern(3 * BLOCK, 2), time(4))
        .unwrap();
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let explainer = Explainer::load(&mut dev).unwrap();
    assert_eq!(explainer.problems, Vec::<String>::new());
    assert!(explainer.has_snapshots);
    let mut block = vec![0u8; BLOCK];
    dev.read_block(0, &mut block).unwrap();
    let ident = Identification::decode(&block).unwrap();
    let selection = select_checkpoint(&mut dev, &ident).unwrap();
    let state = load_committed_state(&mut dev, &ident, &selection.chosen).unwrap();
    let retained: BTreeSet<u64> = state.snapshot_owned_blocks.iter().copied().collect();

    let mut snapshot_nodes = 0;
    let mut unowned = BTreeSet::new();
    for lba in 0..dev.total_blocks() {
        let explanation = explainer.explain_block(&mut dev, lba).unwrap();
        if explanation.roles.iter().any(|role| {
            matches!(
                role,
                BlockRole::VolumeTreeNode {
                    kind: VolumeTree::SnapshotRegistry | VolumeTree::SnapshotLifetimes,
                    ..
                }
            )
        }) {
            snapshot_nodes += 1;
        }
        if explanation.is_unowned() {
            unowned.insert(lba);
        }
    }
    assert!(snapshot_nodes >= 2, "registry and lifetime roots");
    // The live walk cannot own what only history reaches: those blocks, and
    // only those, are allocated without a live role.
    assert!(unowned.len() >= 3, "{unowned:?}");
    assert!(
        unowned.is_subset(&retained),
        "unowned but not retained: {:?}",
        unowned.difference(&retained).collect::<Vec<_>>()
    );
}

#[test]
fn explain_object_and_path_state_what_the_image_was_built_with() {
    use afsplus_format::object::ObjectType;

    let mut volume = mount(formatted(0)).unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "Docs", time(2))
        .unwrap();
    let sub = volume.create_directory(dir, "Été", time(2)).unwrap();
    let file = volume
        .create_file_in_directory(sub, "note.txt", &pattern(4096, 1), time(3))
        .unwrap();
    // Extent tree: a second, distant run; then one preallocated block.
    volume
        .write_file_at(file, 100 * BLOCK as u64, &pattern(2 * BLOCK, 2), time(4))
        .unwrap();
    volume.set_object_protection(file, 0x15, time(5)).unwrap();
    volume.set_object_comment(file, "Résumé", time(6)).unwrap();
    volume
        .set_security_descriptor(file, 0x7fff_0009, 3, &pattern(5000, 7), time(7))
        .unwrap();
    volume
        .link_file(file, OBJECT_ROOT, "alias", time(8))
        .unwrap();
    let clone = volume.clone_file(file, dir, "copy", time(9)).unwrap();
    let link = volume
        .create_symlink(dir, "shortcut", "Été/note.txt", time(9))
        .unwrap();
    let empty = volume
        .create_file_in_directory(OBJECT_ROOT, "empty", b"", time(9))
        .unwrap();
    // Second witness: what the core itself answers.
    let core_stat = volume.stat(file).unwrap().unwrap();
    let core_comment = volume.object_comment(file).unwrap();
    let mut dev = volume.into_device();

    let explainer = Explainer::load(&mut dev).unwrap();
    assert_eq!(explainer.problems, Vec::<String>::new());

    let explained = explainer.explain_object(file).unwrap();
    assert_eq!(explained.object_type, ObjectType::File);
    assert_eq!(explained.link_count, 2);
    assert_eq!(explained.size_bytes, 102 * BLOCK as u64);
    assert_eq!(explained.allocated_bytes, 3 * BLOCK as u64);
    assert_eq!(explained.protection, 0x15);
    assert_eq!(explained.created, time(3));
    assert_eq!(explained.modified, time(4));
    assert_eq!(explained.changed, time(8));
    assert!(explained.extent_tree && !explained.data_in_place);
    assert_eq!(explained.comment, "Résumé");
    assert_eq!(explained.symlink_target, None);
    assert_eq!(explained.tree_nodes, 1);
    assert_eq!(
        (explained.data.extents, explained.data.mapped_blocks),
        (2, 3)
    );
    // The clone shares all three blocks.
    assert_eq!(explained.data.shared_blocks, 3);
    assert_eq!(explained.data.unwritten_blocks, 0);
    let security = explained.security.as_ref().unwrap();
    assert_eq!(
        (
            security.total_len,
            security.segments_expected,
            security.segments_found,
            security.format,
            security.projection_diverged
        ),
        (5000, 2, 2, Some((0x7fff_0009, 3)), false)
    );
    let mut names = explained.names.clone();
    names.sort();
    assert_eq!(
        names,
        vec![
            (OBJECT_ROOT, "alias".to_owned()),
            (sub, "note.txt".to_owned())
        ]
    );
    // Agreement with the core's own answers for the same object.
    assert_eq!(
        (
            explained.size_bytes,
            explained.allocated_bytes,
            explained.link_count,
            explained.protection,
            explained.content_generation
        ),
        (
            core_stat.size_bytes,
            core_stat.allocated_bytes,
            core_stat.link_count,
            core_stat.protection,
            core_stat.content_generation
        )
    );
    assert_eq!(explained.comment, core_comment);

    // The clone: own identity, copied comment and descriptor, one name.
    let cloned = explainer.explain_object(clone).unwrap();
    assert_eq!((cloned.link_count, cloned.comment.as_str()), (1, "Résumé"));
    assert_eq!(cloned.security.as_ref().unwrap().segments_found, 2);
    assert_eq!(cloned.names, vec![(dir, "copy".to_owned())]);
    assert_ne!(cloned.record_block, explained.record_block);

    let directory = explainer.explain_object(dir).unwrap();
    assert_eq!(
        (
            directory.object_type,
            directory.entries,
            directory.tree_nodes
        ),
        (ObjectType::Directory, 3, 1)
    );
    assert_eq!(directory.names, vec![(OBJECT_ROOT, "Docs".to_owned())]);
    let root = explainer.explain_object(OBJECT_ROOT).unwrap();
    assert_eq!((root.entries, root.names.len()), (3, 0));
    let shortcut = explainer.explain_object(link).unwrap();
    assert_eq!(shortcut.symlink_target.as_deref(), Some("Été/note.txt"));
    let nothing = explainer.explain_object(empty).unwrap();
    assert_eq!(
        (
            nothing.size_bytes,
            nothing.data,
            nothing.tree_nodes,
            &nothing.security
        ),
        (0, Default::default(), 0, &None)
    );
    assert!(explainer.explain_object(9999).is_err());

    // Paths: every component with the object it names.
    let path = explainer.explain_path("Docs/Été/note.txt").unwrap();
    assert_eq!(
        path.components,
        vec![
            ("Docs".to_owned(), dir),
            ("Été".to_owned(), sub),
            ("note.txt".to_owned(), file)
        ]
    );
    assert_eq!(&path.object, explained);
    assert_eq!(
        explainer.explain_path("/alias").unwrap().object.object_id,
        file
    );
    assert_eq!(
        explainer.explain_path("").unwrap().object.object_id,
        OBJECT_ROOT
    );
    // Refusals name the component that failed.
    let missing = explainer.explain_path("Docs/été/note.txt").unwrap_err();
    assert!(missing.contains("\"été\""), "{missing}");
    let through_a_file = explainer.explain_path("alias/x").unwrap_err();
    assert!(through_a_file.contains("\"x\""), "{through_a_file}");

    // Negative control: reseal the clone's second segment under a foreign
    // owner. The walk then finds one consistent segment of two, says why,
    // and every other statement about the clone stands.
    let second = (0..dev.total_blocks())
        .find(|lba| {
            explainer
                .explain_block(&mut dev, *lba)
                .unwrap()
                .roles
                .contains(&BlockRole::SecuritySegment {
                    object_id: clone,
                    index: 1,
                })
        })
        .unwrap();
    let mut damaged = dev.clone();
    let mut block = vec![0u8; BLOCK];
    damaged.read_block(second, &mut block).unwrap();
    let header = BlockHeader::verify(&block, block_type::SECURITY_DESCRIPTOR).unwrap();
    BlockHeader {
        owner: 0xbad,
        ..header
    }
    .seal(&mut block);
    damaged.write_block(second, &block).unwrap();
    let after = Explainer::load(&mut damaged).unwrap();
    assert_eq!(after.problems.len(), 1, "{:?}", after.problems);
    let summary = after
        .explain_object(clone)
        .unwrap()
        .security
        .clone()
        .unwrap();
    assert_eq!((summary.segments_found, summary.segments_expected), (1, 2));
    assert_eq!(after.explain_object(clone).unwrap().comment, "Résumé");
    assert_eq!(
        after
            .explain_object(file)
            .unwrap()
            .security
            .as_ref()
            .unwrap()
            .segments_found,
        2
    );

    // The record block explain_object names is the block explain_block
    // attributes to that object.
    assert_eq!(
        explainer
            .explain_block(&mut dev, explained.record_block)
            .unwrap()
            .roles,
        vec![BlockRole::ObjectRecord { object_id: file }]
    );
}

#[test]
fn a_descriptor_chain_with_two_format_identities_is_not_proven() {
    use afsplus_format::security::SecuritySegment;
    let mut volume = mount(formatted(4)).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    volume
        .set_security_descriptor(file, 0x7fff_0001, 1, &pattern(9000, 3), time(3))
        .unwrap();
    let mut dev = volume.into_device();
    let explainer = Explainer::load(&mut dev).unwrap();
    let summary = explainer.explain_object(file).unwrap().security.clone();
    assert_eq!(summary.unwrap().segments_found, 3);

    // Rewrite the middle segment under another format version, same
    // generation, valid checksum.
    let mut block = vec![0u8; 4096];
    let middle = (0..dev.total_blocks())
        .find(|lba| {
            dev.read_block(*lba, &mut block).unwrap();
            SecuritySegment::decode(&block).is_ok_and(|(s, _)| s.object_id == file && s.index == 1)
        })
        .unwrap();
    let (segment, generation) = SecuritySegment::decode(&block).unwrap();
    let forged = SecuritySegment {
        version: segment.version + 1,
        ..segment
    }
    .encode(4096, generation)
    .unwrap();
    dev.write_block(middle, &forged).unwrap();

    let explainer = Explainer::load(&mut dev).unwrap();
    let summary = explainer.explain_object(file).unwrap().security.clone();
    assert_eq!(summary.unwrap().segments_found, 1);
    assert!(explainer
        .explain_block(&mut dev, middle)
        .unwrap()
        .is_unowned());
    // The checker, through the core's walk, refuses the same chain.
    assert!(!check_device(&mut dev).errors.is_empty());
}
