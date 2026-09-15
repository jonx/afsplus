//! Typed payload admission after valid generic tree decoding.
use afsplus_block::{BlockDevice, MemoryBackend, TraceBackend};
use afsplus_core::{allocation_root, extent_map, object_map};
use afsplus_format::{
    geometry::Geometry,
    tree::{TreeItem, TreeKind, TreeNode},
};

const GEO: Geometry = Geometry {
    block_size: 4096,
    total_blocks: 256,
    region_size: 256,
};

fn image(kind: TreeKind, key: Vec<u8>, value: Vec<u8>) -> TraceBackend<MemoryBackend> {
    let node = TreeNode {
        kind,
        owner: if kind == TreeKind::ExtentMap { 10 } else { 0 },
        level: 0,
        subtree_items: 1,
        leftmost_child: 0,
        leftmost_items: 0,
        items: vec![TreeItem { key, value }],
    };
    let bytes = node.encode(4096, 7).unwrap();
    let mut dev = MemoryBackend::new(4096, 256);
    dev.write_block(20, &bytes).unwrap();
    TraceBackend::new(dev)
}

#[test]
fn object_payload_width_and_physical_bounds_are_checked_by_caller() {
    for fault in 0..6 {
        let mut key = 10u64.to_be_bytes().to_vec();
        let mut value = 100u64.to_le_bytes().to_vec();
        match fault {
            0 => {}
            1 => {
                key.pop();
            }
            2 => {
                value.pop();
            }
            3 => value.push(0),
            4 => value = 0u64.to_le_bytes().to_vec(),
            5 => value = 256u64.to_le_bytes().to_vec(),
            _ => unreachable!(),
        }
        let mut dev = image(TreeKind::ObjectMap, key, value);
        let result = object_map::load_all(&mut dev, &GEO, 20, 7);
        if fault == 0 {
            assert_eq!(result.unwrap().lookup(10), Some(100));
        } else {
            assert!(result.is_err(), "fault {fault}");
        }
        assert_eq!(dev.stats().reads, 1);
        assert_eq!(dev.stats().writes, 0);
    }
}

#[test]
fn allocation_payload_relations_are_checked_independently_of_bitmaps() {
    for fault in 0..10 {
        let mut key = 0u32.to_be_bytes().to_vec();
        let mut value = vec![0; 16];
        value[4..8].copy_from_slice(&100u32.to_le_bytes());
        value[8..16].copy_from_slice(&7u64.to_le_bytes());
        match fault {
            0 => {}
            1 => {
                key.pop();
            }
            2 => {
                value.pop();
            }
            3 => value.push(0),
            4 => value[1] = 1,
            5 => value[0] = 3,
            6 => value[8..16].fill(0),
            7 => value[8..16].copy_from_slice(&8u64.to_le_bytes()),
            8 => value[4..8].copy_from_slice(&257u32.to_le_bytes()),
            9 => key = 1u32.to_be_bytes().to_vec(),
            _ => unreachable!(),
        }
        let mut dev = image(TreeKind::AllocationRoot, key, value);
        let result = allocation_root::load_all(&mut dev, &GEO, 20, 7);
        if fault == 0 {
            let loaded = result.unwrap();
            assert_eq!(loaded.records.len(), 1);
            assert_eq!(loaded.records[0].free_blocks, 100);
        } else {
            assert!(result.is_err(), "fault {fault}");
        }
        assert_eq!(dev.stats().reads, 1);
        assert_eq!(dev.stats().writes, 0);
    }
}

#[test]
fn extent_payload_width_reserved_bits_and_ranges_are_checked_by_caller() {
    for fault in 0..11 {
        let mut key = 0u64.to_be_bytes().to_vec();
        let mut value = vec![0; 24];
        value[0..8].copy_from_slice(&100u64.to_le_bytes());
        value[8..16].copy_from_slice(&2u64.to_le_bytes());
        match fault {
            0 => {}
            1 => {
                key.pop();
            }
            2 => {
                value.pop();
            }
            3 => value.push(0),
            4 => value[20] = 1,
            5 => value[19] = 128,
            6 => value[8..16].fill(0),
            7 => key = u64::MAX.to_be_bytes().to_vec(),
            8 => value[0..8].copy_from_slice(&u64::MAX.to_le_bytes()),
            9 => value[0..8].copy_from_slice(&255u64.to_le_bytes()),
            10 => value[0..8].fill(0),
            _ => unreachable!(),
        }
        let mut dev = image(TreeKind::ExtentMap, key, value);
        let result = extent_map::load_all(&mut dev, &GEO, 20, 10, 7);
        if fault == 0 {
            let loaded = result.unwrap();
            assert_eq!(loaded.extents.len(), 1);
            assert_eq!(loaded.allocated_blocks, 2);
            assert_eq!(loaded.extents[0].physical_start, 100);
        } else {
            assert!(result.is_err(), "fault {fault}");
        }
        assert_eq!(dev.stats().reads, 1);
        assert_eq!(dev.stats().writes, 0);
    }
}
