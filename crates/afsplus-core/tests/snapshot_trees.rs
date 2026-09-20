use afsplus_block::{BlockDevice, MemoryBackend, TraceBackend};
use afsplus_core::{allocation_root, snapshot::*};
use afsplus_format::geometry::Geometry;
use afsplus_format::snapshot::{LedgerState, LifetimeRecord, RegistryState, SnapshotRecord};
use afsplus_format::tree::{key_u64, TreeItem, TreeKind, TreeNode};

fn geometry() -> Geometry {
    Geometry {
        block_size: 4096,
        total_blocks: 4096,
        region_size: 1024,
    }
}

fn write_leaf(dev: &mut MemoryBackend, kind: TreeKind, records: Vec<(u64, Vec<u8>)>) {
    let mut node = TreeNode::leaf(kind, 0);
    node.items = records
        .into_iter()
        .map(|(key, value)| TreeItem {
            key: key_u64(key).into(),
            value: value.into(),
        })
        .collect();
    node.subtree_items = node.items.len() as u64;
    dev.write_block(50, &node.encode(4096, 10).unwrap())
        .unwrap();
}

fn registry(next: u64, ids: &[u64]) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 4096);
    let view = SnapshotRecord {
        generation: 5,
        committed_tx_id: 4,
        object_map_root: 80,
    };
    let mut records = vec![(
        0,
        RegistryState { next_id: next }.encode().unwrap().to_vec(),
    )];
    records.extend(
        ids.iter()
            .map(|&id| (id, view.encode(10, 4096).unwrap().to_vec())),
    );
    write_leaf(&mut dev, TreeKind::SnapshotRegistry, records);
    dev
}

fn ledger(position: u64, runs: &[(u64, u64, u64, u64)]) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 4096);
    let retained_blocks = runs.iter().filter(|run| run.3 != 0).map(|run| run.1).sum();
    let mut records = vec![(
        0,
        LedgerState {
            scan_position: position,
            retained_blocks,
        }
        .encode(4096)
        .unwrap()
        .to_vec(),
    )];
    records.extend(runs.iter().map(|&(start, blocks, birth, retirement)| {
        (
            start,
            LifetimeRecord {
                blocks,
                birth,
                retirement,
            }
            .encode(start, 10, 4096)
            .unwrap()
            .to_vec(),
        )
    }));
    write_leaf(&mut dev, TreeKind::SnapshotLifetimes, records);
    dev
}

fn replace(dev: &mut MemoryBackend, edit: impl FnOnce(&mut TreeNode)) {
    let (mut node, _) = TreeNode::decode(&dev.peek(50)).unwrap();
    edit(&mut node);
    dev.write_block(50, &node.encode(4096, 10).unwrap())
        .unwrap();
}

#[test]
fn registry_pages_preserve_sparse_ids_and_report_actual_reads() {
    let mut dev = TraceBackend::new(registry(20, &[1, 4, 9, 19]));
    let mut ids = Vec::new();
    let mut next = Some(0);
    while let Some(low) = next {
        dev.reset();
        let page = read_registry_page(&mut dev, &geometry(), 50, 10, low, 2).unwrap();
        assert_eq!(page.state.next_id, 20);
        assert_eq!(page.advertised_count, 4);
        assert_eq!(page.stats.pages_read, dev.stats().reads);
        assert_eq!(page.stats.pages_read, 2);
        assert_eq!(dev.stats().writes, 0);
        assert_eq!(dev.stats().flushes, 0);
        ids.extend(page.records.iter().map(|pair| pair.0));
        next = page.next_id;
    }
    assert_eq!(ids, [1, 4, 9, 19]);
    let mut empty = registry(1, &[]);
    assert_eq!(
        read_registry_state(&mut empty, &geometry(), 50, 10)
            .unwrap()
            .1,
        0
    );
    assert!(read_registry_page(&mut empty, &geometry(), 50, 10, 0, 1)
        .unwrap()
        .records
        .is_empty());
    assert!(read_registry_page(&mut empty, &geometry(), 50, 10, 0, 0).is_err());
}

#[test]
fn registry_rejects_control_identity_and_contextual_corruption() {
    for case in 0..9 {
        let mut dev = registry(5, &[1, 4]);
        replace(&mut dev, |node| match case {
            0 => {
                node.items.remove(0);
                node.subtree_items -= 1;
            }
            1 => node.kind = TreeKind::SnapshotLifetimes,
            2 => node.owner = 1,
            3 => node.items[0].value[8] = 1,
            4 => node.items[2].key = key_u64(5).into(),
            5 => node.items[1].value[0..8].copy_from_slice(&11u64.to_le_bytes()),
            6 => node.items[1].value[16..24].copy_from_slice(&3u64.to_le_bytes()),
            7 => node.items[1].value[16..24].copy_from_slice(
                &allocation_root::reserved_pool_bounds(&geometry())
                    .unwrap()
                    .0
                    .to_le_bytes(),
            ),
            8 => node.items[0].value[0..8].copy_from_slice(&2u64.to_le_bytes()),
            _ => unreachable!(),
        });
        assert!(
            read_registry_page(&mut dev, &geometry(), 50, 10, 0, 8).is_err(),
            "case {case}"
        );
    }
    let mut dev = registry(5, &[1]);
    assert!(read_registry_state(&mut dev, &geometry(), 50, 9).is_err());
}

#[test]
fn lifetime_cursor_validates_neighbors_and_wraps_without_skipping_successors() {
    let runs = [
        (100, 3, 1, 5),
        (110, 2, 2, 0),
        (120, 4, 3, 8),
        (140, 1, 4, 0),
    ];
    for (position, expected, next) in [
        (0, vec![100, 110], 111),
        (101, vec![110, 120], 121),
        (111, vec![120, 140], 0),
        (141, vec![], 0),
    ] {
        let mut dev = TraceBackend::new(ledger(position, &runs));
        let page = read_lifetime_page(&mut dev, &geometry(), 50, 10, 2).unwrap();
        assert_eq!(
            page.records.iter().map(|run| run.start).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(page.next_position, next);
        assert_eq!(page.state.retained_blocks, 7);
        assert_eq!(page.stats.pages_read, dev.stats().reads);
        assert_eq!(page.stats.pages_read, 3);
        assert_eq!(dev.stats().writes, 0);
        assert_eq!(dev.stats().flushes, 0);
    }
    // Simulate atomic cursor persistence with deleting records behind it.
    let mut dev = ledger(111, &runs[2..]);
    let page = read_lifetime_page(&mut dev, &geometry(), 50, 10, 1).unwrap();
    assert_eq!(page.records[0].start, 120);
    assert_eq!(page.next_position, 121);
    let mut wrapped = ledger(0, &[(90, 1, 9, 0), runs[3]]);
    assert_eq!(
        read_lifetime_page(&mut wrapped, &geometry(), 50, 10, 1)
            .unwrap()
            .records[0]
            .start,
        90
    );
}

#[test]
fn lifetime_pages_reject_overlaps_and_noncanonical_edges_outside_batch() {
    for runs in [
        vec![(100, 12, 1, 5), (110, 2, 2, 0)],
        vec![(100, 10, 1, 5), (110, 2, 1, 5)],
    ] {
        // Corruption crosses the right edge (lookahead) or left edge (predecessor).
        for cursor in [0, 110] {
            let mut dev = ledger(cursor, &runs);
            assert!(read_lifetime_page(&mut dev, &geometry(), 50, 10, 1).is_err());
        }
    }
    let pool = allocation_root::reserved_pool_bounds(&geometry())
        .unwrap()
        .0;
    for (start, blocks) in [(3, 1), (pool, 1), (1020, 20), (1024, 1)] {
        let mut dev = ledger(0, &[(start, blocks, 1, 0)]);
        assert!(read_lifetime_page(&mut dev, &geometry(), 50, 10, 1).is_err());
    }
    for limit in [0, usize::MAX] {
        let mut dev = ledger(0, &[]);
        assert!(read_lifetime_page(&mut dev, &geometry(), 50, 10, limit).is_err());
    }
}

#[test]
fn pool_bounds_match_enumeration_and_scale_without_materializing_the_pool() {
    for region_size in [16, 32, 256, 1024, 32768, 262144] {
        for regions in [1, 2, 3, 85, 200] {
            for tail in [0, 9] {
                let geo = Geometry {
                    block_size: 4096,
                    total_blocks: region_size as u64 * regions + tail,
                    region_size,
                };
                if geo.validate().is_err() {
                    continue;
                }
                let expected = allocation_root::reserved_pool_lbas(&geo);
                let actual = allocation_root::reserved_pool_bounds(&geo);
                match (expected, actual) {
                    (Ok(blocks), Ok(bounds)) => {
                        assert_eq!(bounds, (blocks[0], blocks.last().unwrap() + 1))
                    }
                    (Err(_), Err(_)) => (),
                    _ => panic!("pool bounds disagree for {geo:?}"),
                }
            }
        }
    }
    let geo = Geometry {
        block_size: 4096,
        total_blocks: 262144 * u32::MAX as u64,
        region_size: 262144,
    };
    let (start, end) = allocation_root::reserved_pool_bounds(&geo).unwrap();
    assert!(geo.is_allocatable(start));
    assert!(geo.is_allocatable(end - 1));
    assert!(end - start > 1_000_000);
}
