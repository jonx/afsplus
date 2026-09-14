//! Read-only checker negotiation for experimental snapshot images.
use afsplus_block::{BlockDevice, MemoryBackend, RecordingBackend};
use afsplus_core::{mkfs_with_options, mount, MkfsOptions, MkfsParams, NamePolicy};
use afsplus_format::ident::Identification;
use afsplus_format::snapshot::{LedgerState, RegistryState, SnapshotRecord};
use afsplus_format::tree::{key_u64, TreeItem, TreeNode};

#[test]
fn checker_validates_snapshot_images_without_enabling_writable_mounts() {
    let mut dev = MemoryBackend::new(4096, 512);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [74; 16],
            label: "CheckerSnapshots".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: Default::default(),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let mut buf = vec![0; 4096];
    dev.read_block(0, &mut buf).unwrap();
    let ident = Identification::decode(&buf).unwrap();
    let cp = afsplus_core::mount::select_checkpoint(&mut dev, &ident)
        .unwrap()
        .chosen;
    let roots = cp.snapshot_roots.unwrap();
    // Independently add an initial view without changing physical ownership.
    dev.read_block(roots.registry, &mut buf).unwrap();
    let (mut tree, generation) = TreeNode::decode(&buf).unwrap();
    tree.items[0].value = RegistryState { next_id: 2 }.encode().unwrap().to_vec();
    tree.items.push(TreeItem {
        key: key_u64(1).to_vec(),
        value: SnapshotRecord {
            generation: cp.generation,
            committed_tx_id: cp.committed_tx_id,
            object_map_root: cp.object_map_block,
        }
        .encode(cp.generation, ident.total_blocks)
        .unwrap()
        .to_vec(),
    });
    tree.subtree_items = 2;
    dev.write_block(roots.registry, &tree.encode(4096, generation).unwrap())
        .unwrap();
    assert!(matches!(
        mount(dev.clone()),
        Err(afsplus_core::CoreError::UnsupportedIncompatFeatures(_))
    ));
    let mut recording = RecordingBackend::new(dev.clone());
    let report = afsplus_check::check_device(&mut recording);
    assert!(report.is_clean(), "{:?}", report.errors);
    let (_, operations) = recording.into_parts();
    assert!(operations.is_empty(), "checker must not write or flush");
    dev.read_block(roots.lifetimes, &mut buf).unwrap();
    let (mut tree, generation) = TreeNode::decode(&buf).unwrap();
    tree.items[0].value = LedgerState {
        scan_position: 0,
        retained_blocks: 1,
    }
    .encode(512)
    .unwrap()
    .to_vec();
    dev.write_block(roots.lifetimes, &tree.encode(4096, generation).unwrap())
        .unwrap();
    let report = afsplus_check::check_device(&mut dev);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("retained total")),
        "{:?}",
        report.errors
    );
}
