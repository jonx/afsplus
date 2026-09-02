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
fn decrement_at_a_record_start_merges_with_its_left_neighbour() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let content = vec![0x62u8; 2 * BS];
    let source = vol.create_file_in_root("origin", &content, ts(2)).unwrap();
    let first = vol.clone_file(source, OBJECT_ROOT, "copy1", ts(3)).unwrap();

    // Make copy1 private in the first block.  The second physical block
    // remains shared by source and copy1.
    vol.write_file_at(first, 0, &vec![0x91u8; BS], ts(4))
        .unwrap();

    // A second full clone of source creates adjacent records with different
    // counts: the first block is shared by source/copy2 (rc=2), the second by
    // source/copy1/copy2 (rc=3).
    let second = vol.clone_file(source, OBJECT_ROOT, "copy2", ts(5)).unwrap();
    let before = shared_records(&mut vol);
    assert_eq!(
        before.len(),
        2,
        "expected the rc=2/rc=3 boundary: {before:?}"
    );
    assert_eq!(before[0].reference_count, 2);
    assert_eq!(before[1].reference_count, 3);
    assert_eq!(before[0].physical_end().unwrap(), before[1].physical_start);

    // Dropping copy1 decrements exactly the second record at its own start.
    // The result must merge with the left rc=2 neighbour.  A prefetch based
    // on lookup_floor(start) misses that neighbour and leaves a non-canonical
    // pair of adjacent rc=2 records behind.
    vol.delete_file(OBJECT_ROOT, "copy1", ts(6)).unwrap();
    let after = shared_records(&mut vol);
    assert_eq!(
        after.len(),
        1,
        "adjacent equal counts must merge: {after:?}"
    );
    assert_eq!(after[0].block_count, 2);
    assert_eq!(after[0].reference_count, 2);
    assert_eq!(vol.read_file(source).unwrap(), content);
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

#[test]
fn clone_range_replaces_only_the_requested_aligned_blocks() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source_bytes: Vec<u8> = (0..5 * BS).map(|index| (index / BS) as u8 + 1).collect();
    let destination_bytes = vec![0x80u8; 6 * BS];
    let source = vol
        .create_file_in_root("source", &source_bytes, ts(2))
        .unwrap();
    let source_start = vol.stat(source).unwrap().unwrap().data_root;
    let destination = vol
        .create_file_in_root("destination", &destination_bytes, ts(3))
        .unwrap();

    vol.clone_range(
        source,
        BS as u64,
        destination,
        (2 * BS) as u64,
        (3 * BS) as u64,
        ts(4),
    )
    .unwrap();

    let mut expected = destination_bytes;
    expected[2 * BS..5 * BS].copy_from_slice(&source_bytes[BS..4 * BS]);
    assert_eq!(vol.read_file(destination).unwrap(), expected);
    assert_eq!(vol.read_file(source).unwrap(), source_bytes);
    assert_eq!(vol.last_commit_stats().unwrap().data_blocks_written, 0);
    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].physical_start, source_start + 1);
    assert_eq!(records[0].block_count, 3);
    assert_eq!(records[0].reference_count, 2);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn overlapping_clone_ranges_form_canonical_two_three_two_counts() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let content: Vec<u8> = (0..4 * BS).map(|index| (index / BS) as u8 + 1).collect();
    let source = vol.create_file_in_root("source", &content, ts(2)).unwrap();
    let first = vol.create_file_in_root("first", b"", ts(3)).unwrap();
    let second = vol.create_file_in_root("second", b"", ts(4)).unwrap();

    vol.clone_range(source, 0, first, 0, (3 * BS) as u64, ts(5))
        .unwrap();
    vol.clone_range(source, BS as u64, second, 0, (3 * BS) as u64, ts(6))
        .unwrap();

    assert_eq!(vol.read_file(first).unwrap(), content[..3 * BS]);
    assert_eq!(vol.read_file(second).unwrap(), content[BS..4 * BS]);
    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 3, "{records:?}");
    assert_eq!(records[0].reference_count, 2);
    assert_eq!(records[0].block_count, 1);
    assert_eq!(records[1].reference_count, 3);
    assert_eq!(records[1].block_count, 2);
    assert_eq!(records[2].reference_count, 2);
    assert_eq!(records[2].block_count, 1);
    assert_eq!(
        records[0].physical_end().unwrap(),
        records[1].physical_start
    );
    assert_eq!(
        records[1].physical_end().unwrap(),
        records[2].physical_start
    );

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn clone_range_copies_partial_boundaries_and_shares_the_interior() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source_bytes: Vec<u8> = (0..3 * BS).map(|index| (index % 251) as u8).collect();
    let destination_bytes = vec![0x5au8; 3 * BS];
    let source = vol
        .create_file_in_root("source", &source_bytes, ts(2))
        .unwrap();
    let destination = vol
        .create_file_in_root("destination", &destination_bytes, ts(3))
        .unwrap();
    let offset = 100usize;
    let length = 2 * BS + 200;

    vol.clone_range(
        source,
        offset as u64,
        destination,
        offset as u64,
        length as u64,
        ts(4),
    )
    .unwrap();

    let mut expected = destination_bytes;
    expected[offset..offset + length].copy_from_slice(&source_bytes[offset..offset + length]);
    assert_eq!(vol.read_file(destination).unwrap(), expected);
    assert_eq!(vol.read_file(source).unwrap(), source_bytes);
    assert_eq!(
        vol.last_commit_stats().unwrap().data_blocks_written,
        2,
        "only the two partial boundary blocks are copied"
    );
    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].block_count, 1);
    assert_eq!(records[0].reference_count, 2);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn clone_range_keeps_complete_source_holes_sparse() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source = vol.create_file_in_root("source", b"", ts(2)).unwrap();
    let payload = vec![0x73u8; BS];
    vol.write_file_at(source, (2 * BS) as u64, &payload, ts(3))
        .unwrap();
    let destination = vol.create_file_in_root("destination", b"", ts(4)).unwrap();

    vol.clone_range(source, 0, destination, 0, (3 * BS) as u64, ts(5))
        .unwrap();

    let mut expected = vec![0u8; 3 * BS];
    expected[2 * BS..].copy_from_slice(&payload);
    assert_eq!(vol.read_file(destination).unwrap(), expected);
    let destination_record = vol.stat(destination).unwrap().unwrap();
    assert_eq!(destination_record.data_blocks, 1, "holes must not allocate");
    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].block_count, 1);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn clone_range_replaces_a_shared_destination_without_reclaiming_its_peer() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source_bytes = vec![0x19u8; 2 * BS];
    let old_bytes = vec![0x2au8; 2 * BS];
    let source = vol
        .create_file_in_root("source", &source_bytes, ts(2))
        .unwrap();
    let source_peer = vol
        .clone_file(source, OBJECT_ROOT, "source-peer", ts(3))
        .unwrap();
    let destination = vol
        .create_file_in_root("destination", &old_bytes, ts(4))
        .unwrap();
    let old_peer = vol
        .clone_file(destination, OBJECT_ROOT, "old-peer", ts(5))
        .unwrap();

    vol.clone_range(source, 0, destination, 0, (2 * BS) as u64, ts(6))
        .unwrap();

    assert_eq!(vol.read_file(source).unwrap(), source_bytes);
    assert_eq!(vol.read_file(source_peer).unwrap(), source_bytes);
    assert_eq!(vol.read_file(destination).unwrap(), source_bytes);
    assert_eq!(vol.read_file(old_peer).unwrap(), old_bytes);
    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.shared_refs.blocks_reference_incremented, 2);
    assert_eq!(stats.shared_refs.blocks_privatized, 2);
    let records = shared_records(&mut vol);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].block_count, 2);
    assert_eq!(records[0].reference_count, 3);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn unrepresentable_or_same_file_clone_range_is_rejected_without_a_commit() {
    let dev = formatted(true);
    let mut vol = mount(dev).unwrap();
    let source = vol
        .create_file_in_root("source", &vec![0x31u8; 2 * BS], ts(2))
        .unwrap();
    let destination = vol
        .create_file_in_root("destination", &vec![0x42u8; 2 * BS], ts(3))
        .unwrap();
    let generation = vol.generation();
    let free_blocks = vol.free_blocks();
    let before = vol.read_file(destination).unwrap();

    assert!(matches!(
        vol.clone_range(source, 1, destination, 0, BS as u64, ts(4)),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert!(matches!(
        vol.clone_range(source, 0, source, BS as u64, BS as u64, ts(4)),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert_eq!(vol.generation(), generation);
    assert_eq!(vol.free_blocks(), free_blocks);
    assert_eq!(vol.read_file(destination).unwrap(), before);
    assert_eq!(vol.checkpoint().shared_extent_root_block, 0);
}
