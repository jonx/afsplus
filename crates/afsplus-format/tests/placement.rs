//! Placement of the permanently allocated areas, from geometry alone: the
//! bootstrap metadata, the allocation-root pool and the intent-log slots are
//! consecutive runs of allocatable blocks.
use afsplus_format::geometry::{Geometry, BOOTSTRAP_METADATA_BLOCKS};

fn geometry(total_blocks: u64, region_size: u32) -> Geometry {
    let geo = Geometry {
        block_size: 4096,
        total_blocks,
        region_size,
    };
    geo.validate().unwrap();
    geo
}

#[test]
fn a_one_page_region_layout_has_literal_positions() {
    // Region 0 head: identification, two checkpoints, three descriptor
    // slots, three slots of its single bitmap page: blocks 0..=8.
    let geo = geometry(2048, 512);
    assert_eq!(geo.region0_reserved_blocks(), 9);
    assert_eq!(BOOTSTRAP_METADATA_BLOCKS, 4);
    assert_eq!(geo.reserved_run(0, 4).unwrap(), [9, 10, 11, 12]);
    // Four regions fit one leaf: N = 1, pool = 3 blocks.
    assert_eq!(geo.allocation_root_logical_nodes().unwrap(), 1);
    assert_eq!(geo.allocation_root_pool_lbas().unwrap(), [13, 14, 15]);
    assert_eq!(
        geo.intent_log_slot_lbas(8).unwrap(),
        [16, 17, 18, 19, 20, 21, 22, 23]
    );
    assert_eq!(geo.intent_log_slot_lbas(0).unwrap(), Vec::<u64>::new());
}

#[test]
fn a_run_skips_the_reserved_head_of_the_next_region() {
    // 16-block regions: six reserved blocks per region, nine in region 0,
    // so region 0 offers blocks 9..=15 and region 1 starts at 16 + 6.
    let geo = geometry(128, 16);
    assert_eq!(geo.reserved_run(0, 4).unwrap(), [9, 10, 11, 12]);
    assert_eq!(geo.allocation_root_pool_lbas().unwrap(), [13, 14, 15]);
    assert_eq!(geo.intent_log_slot_lbas(4).unwrap(), [22, 23, 24, 25]);
    // Ten more slots run through region 1 into region 2 (base 32, head 6).
    assert_eq!(
        geo.intent_log_slot_lbas(14).unwrap(),
        [22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 38, 39, 40, 41]
    );
    for lba in geo.intent_log_slot_lbas(14).unwrap() {
        assert!(geo.is_allocatable(lba));
    }
}

#[test]
fn the_pool_grows_with_the_node_count_of_the_bulk_packed_tree() {
    // A 4 KiB leaf holds (4096 - header - fixed payload) / item records.
    // With more regions than one leaf holds, N = leaves + 1 root.
    let one_leaf = geometry(2048, 512);
    let leaf_capacity = (1u32..)
        .find(|regions| {
            geometry(u64::from(*regions) * 16, 16)
                .allocation_root_logical_nodes()
                .unwrap()
                > 1
        })
        .unwrap()
        - 1;
    assert!(leaf_capacity > 100, "{leaf_capacity}");
    let two_leaves = geometry(u64::from(leaf_capacity + 1) * 16, 16);
    assert_eq!(two_leaves.allocation_root_logical_nodes().unwrap(), 3);
    assert_eq!(two_leaves.allocation_root_pool_lbas().unwrap().len(), 9);
    assert_eq!(one_leaf.allocation_root_pool_lbas().unwrap().len(), 3);
}

#[test]
fn a_volume_too_small_for_its_areas_is_refused() {
    let geo = geometry(16, 16);
    assert!(geo.intent_log_slot_lbas(1).is_err());
    assert!(geo.reserved_run(0, 8).is_err());
    assert_eq!(geo.reserved_run(0, 7).unwrap().len(), 7);
}
