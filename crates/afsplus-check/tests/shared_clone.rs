//! CloneFile semantics over the shared-extent reference tree (ADR-061):
//! sharing, the write-splits-a-run partition, and the release table whose
//! middle row (two references falling to one) must privatise, never free.

use afsplus_block::MemoryBackend;
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
