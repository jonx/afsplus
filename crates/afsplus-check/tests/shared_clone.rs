//! CloneFile semantics over the shared-extent reference tree (ADR-061):
//! sharing, the write-splits-a-run partition, and the release table whose
//! middle row (two references falling to one) must privatise, never free.

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::shared_extents;
use afsplus_core::{mkfs, mount, CoreError, MkfsParams};
use afsplus_format::object::OBJECT_FLAG_EXTENT_TREE;
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(shared_extents: bool) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [7u8; 16],
            label: "CloneVol".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(1),
        },
    )
    .unwrap();
    dev
}

fn shared_records(vol: &mut afsplus_core::Volume<MemoryBackend>) -> Vec<shared_extents::SharedRun> {
    let root = vol.checkpoint().shared_extent_root_block;
    if root == 0 {
        return Vec::new();
    }
    let geo = vol.ident().geometry();
    let generation = vol.generation();
    shared_extents::load_all(vol.device_mut(), &geo, root, generation)
        .unwrap()
        .records
}

#[test]
fn clone_shares_every_run_and_reads_identically() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let content = vec![0xA5u8; 3 * BS];
    let source = vol.create_file_in_root("origin", &content, ts(2)).unwrap();
    // The source starts as the cheap direct layout: no flag word exists
    // there, so the clone must promote it (ADR-061).
    assert_eq!(
        vol.stat(source).unwrap().unwrap().flags & OBJECT_FLAG_EXTENT_TREE,
        0
    );

    let clone = vol.clone_file(source, OBJECT_ROOT, "copy", ts(3)).unwrap();
    assert_eq!(vol.read_file(clone).unwrap(), content);
    assert_eq!(vol.read_file(source).unwrap(), content);

    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.layout_promotions, 1);
    assert_eq!(stats.shared_refs.blocks_newly_shared, 3);
    assert_eq!(stats.shared_refs.blocks_reference_incremented, 0);
    assert_eq!(stats.data_blocks_written, 0, "a clone copies no data");

    let source_record = vol.stat(source).unwrap().unwrap();
    assert_ne!(source_record.flags & OBJECT_FLAG_EXTENT_TREE, 0);
    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].block_count, 3);
    assert_eq!(records[0].reference_count, 2);

    // The committed image is checker-clean and survives a remount.
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.read_file(clone).unwrap(), content);
}

#[test]
fn deleting_one_clone_keeps_the_survivors_bytes() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let content: Vec<u8> = (0..2 * BS).map(|i| (i % 251) as u8).collect();
    let source = vol.create_file_in_root("origin", &content, ts(2)).unwrap();
    let clone = vol.clone_file(source, OBJECT_ROOT, "copy", ts(3)).unwrap();

    // The count-two case of the ADR-061 release table: the record leaves the
    // tree, the blocks do NOT — one live mapping remains.
    vol.delete_file(OBJECT_ROOT, "origin", ts(4)).unwrap();
    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.shared_refs.blocks_privatized, 2);
    assert_eq!(stats.shared_refs.blocks_reference_decremented, 0);
    assert!(shared_records(&mut vol).is_empty());
    // The root stays allocated once created (ADR-061), simply empty.
    assert_ne!(vol.checkpoint().shared_extent_root_block, 0);
    assert_eq!(vol.read_file(clone).unwrap(), content);

    // Remount and reread: the survivor's bytes were never quarantined.
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.read_file(clone).unwrap(), content);
    assert!(vol.stat(source).unwrap().is_none());

    // Dropping the last reference is an ordinary private retire.
    vol.delete_file(OBJECT_ROOT, "copy", ts(5)).unwrap();
    assert!(shared_records(&mut vol).is_empty());
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn writing_into_a_clone_splits_the_shared_run() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let content = vec![0x11u8; 4 * BS];
    let source = vol.create_file_in_root("origin", &content, ts(2)).unwrap();
    let clone = vol.clone_file(source, OBJECT_ROOT, "copy", ts(3)).unwrap();

    // Rewrite the middle two blocks of the clone: fresh private storage for
    // the clone, and the shared run splits into a head and a tail — the
    // ADR-061 example where an exact start lookup would miss the tail.
    let replacement = vec![0xEEu8; 2 * BS];
    vol.write_file_at(clone, BS as u64, &replacement, ts(4))
        .unwrap();
    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.shared_refs.blocks_privatized, 2);

    let mut expected = content.clone();
    expected[BS..3 * BS].copy_from_slice(&replacement);
    assert_eq!(vol.read_file(clone).unwrap(), expected);
    assert_eq!(vol.read_file(source).unwrap(), content, "source untouched");

    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 2, "head and tail stay shared: {records:?}");
    assert!(records.iter().all(|run| run.reference_count == 2));
    assert_eq!(records[0].block_count + records[1].block_count, 2);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.read_file(source).unwrap(), content);
    assert_eq!(vol.read_file(clone).unwrap(), expected);
}

#[test]
fn clone_of_clone_counts_three_references() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let content = vec![0x42u8; BS];
    let source = vol.create_file_in_root("origin", &content, ts(2)).unwrap();
    let first = vol.clone_file(source, OBJECT_ROOT, "copy1", ts(3)).unwrap();
    let second = vol.clone_file(first, OBJECT_ROOT, "copy2", ts(4)).unwrap();

    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.shared_refs.blocks_reference_incremented, 1);
    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reference_count, 3);

    // One deletion decrements; the survivors keep their bytes.
    vol.delete_file(OBJECT_ROOT, "origin", ts(5)).unwrap();
    let records = shared_records(&mut vol);
    assert_eq!(records[0].reference_count, 2);
    assert_eq!(vol.read_file(first).unwrap(), content);
    assert_eq!(vol.read_file(second).unwrap(), content);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn clone_without_the_feature_is_rejected() {
    let dev = formatted(false);
    let mut vol = mount(dev).unwrap();
    let source = vol.create_file_in_root("origin", b"data", ts(2)).unwrap();
    let error = vol
        .clone_file(source, OBJECT_ROOT, "copy", ts(3))
        .unwrap_err();
    assert!(matches!(error, CoreError::FeatureDisabled(_)), "{error:?}");
    assert_eq!(vol.checkpoint().shared_extent_root_block, 0);
}

#[test]
fn refusing_a_clone_leaves_an_open_window_untouched() {
    // F12: on a volume without the feature, a refused clone must have no
    // side effect at all — in particular it must not have committed the
    // open intent-log window first.
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [7u8; 16],
            label: "NoCloneVol".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(1),
        },
    )
    .unwrap();
    let mut vol = mount(dev).unwrap();
    let source = vol.create_file_in_root("origin", b"data", ts(2)).unwrap();
    let generation_before = vol.generation();
    let free_before = vol.free_blocks();
    vol.window_op(
        &afsplus_core::volume::BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "staged",
            content: b"staged",
        },
        ts(3),
    )
    .unwrap();

    let error = vol
        .clone_file(source, OBJECT_ROOT, "copy", ts(4))
        .unwrap_err();
    assert!(matches!(error, CoreError::FeatureDisabled(_)), "{error:?}");
    // The window is still open (an immediate-commit op is refused), and the
    // committed state did not move.
    assert!(matches!(
        vol.create_file_in_root("other", b"x", ts(5)),
        Err(CoreError::WindowOpen)
    ));
    assert_eq!(vol.generation(), generation_before);
    assert_eq!(vol.free_blocks(), free_before);
}

/// Plants `EXTENT_SHARED` on a committed extent-tree leaf, byte-for-byte on
/// the device, bypassing every write path.
fn plant_shared_flag(vol: &mut afsplus_core::Volume<MemoryBackend>, object_id: u64) {
    let record = vol.stat(object_id).unwrap().unwrap();
    assert_ne!(record.flags & OBJECT_FLAG_EXTENT_TREE, 0);
    let root = record.data_root;
    let dev = vol.device_mut();
    let mut block = vec![0u8; BS];
    dev.read_block(root, &mut block).unwrap();
    let (mut node, generation) = afsplus_format::tree::TreeNode::decode(&block).unwrap();
    for item in &mut node.items {
        let flags_offset = 16; // physical (8) + block_count (8), flags u32
        let mut flags = u32::from_le_bytes(
            item.value[flags_offset..flags_offset + 4]
                .try_into()
                .unwrap(),
        );
        flags |= afsplus_core::extent_map::EXTENT_SHARED;
        item.value[flags_offset..flags_offset + 4].copy_from_slice(&flags.to_le_bytes());
    }
    let encoded = node.encode(BS, generation).unwrap();
    dev.write_block(root, &encoded).unwrap();
}

#[test]
fn planted_flag_without_feature_fails_closed() {
    let dev = formatted(false);
    let mut vol = mount(dev).unwrap();
    let source = vol
        .create_file_in_root("origin", &vec![0x33u8; BS], ts(2))
        .unwrap();
    // Preallocation forces the extent-tree layout so there is a flag word.
    vol.preallocate_file(source, 4 * BS as u64, 2 * BS as u64, ts(3))
        .unwrap();
    let free_before = vol.free_blocks();
    plant_shared_flag(&mut vol, source);

    // Mutation: fail closed, nothing freed, nothing published.
    let error = vol.delete_file(OBJECT_ROOT, "origin", ts(4)).unwrap_err();
    assert!(
        matches!(&error, CoreError::Corrupt(text)
            if text.contains("without the shared-extents feature")),
        "{error:?}"
    );
    assert_eq!(vol.free_blocks(), free_before);
    assert!(vol.stat(source).unwrap().is_some());

    // Checker: the same congruence, reported.
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(!report.is_clean());
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("EXTENT_SHARED")),
        "checker findings: {:?}",
        report.errors
    );
}

#[test]
fn planted_flag_without_a_tree_fails_closed() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source = vol
        .create_file_in_root("origin", &vec![0x44u8; BS], ts(2))
        .unwrap();
    vol.preallocate_file(source, 4 * BS as u64, 2 * BS as u64, ts(3))
        .unwrap();
    assert_eq!(vol.checkpoint().shared_extent_root_block, 0);
    let free_before = vol.free_blocks();
    plant_shared_flag(&mut vol, source);

    let error = vol.delete_file(OBJECT_ROOT, "origin", ts(4)).unwrap_err();
    assert!(
        matches!(&error, CoreError::Corrupt(text)
            if text.contains("no reference tree")),
        "{error:?}"
    );
    assert_eq!(vol.free_blocks(), free_before);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(!report.is_clean());
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("EXTENT_SHARED")),
        "checker findings: {:?}",
        report.errors
    );
}

#[test]
fn cloning_an_empty_file_still_allocates_the_root() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source = vol.create_file_in_root("origin", b"", ts(2)).unwrap();
    let clone = vol.clone_file(source, OBJECT_ROOT, "copy", ts(3)).unwrap();

    // ADR-061: the first CloneFile allocates the root even when it shares
    // nothing, so root zero keeps meaning "this volume has never cloned".
    assert_ne!(vol.checkpoint().shared_extent_root_block, 0);
    assert!(shared_records(&mut vol).is_empty());
    assert_eq!(vol.read_file(clone).unwrap(), b"");
    assert_eq!(
        vol.last_commit_stats().unwrap().shared_tree_nodes_written,
        1
    );

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn cloning_a_sparse_file_shares_its_unwritten_runs() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source = vol
        .create_file_in_root("origin", &vec![0x55u8; BS], ts(2))
        .unwrap();
    vol.preallocate_file(source, 4 * BS as u64, 3 * BS as u64, ts(3))
        .unwrap();
    let clone = vol.clone_file(source, OBJECT_ROOT, "copy", ts(4)).unwrap();

    assert_ne!(vol.checkpoint().shared_extent_root_block, 0);
    let records = shared_records(&mut vol);
    let shared_blocks: u64 = records.iter().map(|run| run.block_count).sum();
    assert_eq!(
        shared_blocks, 4,
        "written + unwritten runs share: {records:?}"
    );
    assert!(records.iter().all(|run| run.reference_count == 2));
    assert_eq!(
        vol.read_file(clone).unwrap(),
        vol.read_file(source).unwrap()
    );

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}
