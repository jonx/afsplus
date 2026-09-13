//! Persistent AFST edit tests. This fixture has a test-only tree allocator;
//! it does not qualify Volume checkpoints, bitmap quarantine or snapshot reads.
use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::{BlockDevice, MemoryBackend, TraceBackend};
use afsplus_core::allocation_root::ReservedTreePool;
use afsplus_core::cow_tree::TreeAllocator;
use afsplus_core::snapshot::edit::{mutate_lifetimes, LifetimeChanges, LifetimeMutation};
use afsplus_core::snapshot::{lifetime_spec, read_ledger_state, LifetimeRun};
use afsplus_core::tree::visit_tree_nodes;
use afsplus_core::CoreError;
use afsplus_format::checkpoint::SnapshotRoots;
use afsplus_format::geometry::Geometry;
use afsplus_format::snapshot::{LedgerState, LifetimeRecord, RegistryState, SnapshotRecord};
use afsplus_format::tree::{key_u64, TreeItem, TreeKind, TreeNode};

struct Harness {
    dev: TraceBackend<MemoryBackend>,
    geo: Geometry,
    roots: SnapshotRoots,
    generation: u64,
    protected_nodes: BTreeSet<u64>,
}

impl Harness {
    fn new() -> Self {
        let mut h = Self {
            dev: TraceBackend::new(MemoryBackend::new(4096, 8192)),
            geo: Geometry {
                block_size: 4096,
                total_blocks: 8192,
                region_size: 8192,
            },
            roots: SnapshotRoots {
                registry: 201,
                lifetimes: 200,
            },
            generation: 1,
            protected_nodes: BTreeSet::from([200]),
        };
        h.leaf(
            200,
            TreeKind::SnapshotLifetimes,
            vec![(
                0,
                LedgerState {
                    scan_position: 0,
                    retained_blocks: 0,
                }
                .encode(8192)
                .unwrap()
                .to_vec(),
            )],
        );
        h.views(&[]);
        h
    }

    fn leaf(&mut self, lba: u64, kind: TreeKind, records: Vec<(u64, Vec<u8>)>) {
        let mut node = TreeNode::leaf(kind, 0);
        node.items = records
            .into_iter()
            .map(|(key, value)| TreeItem {
                key: key_u64(key).to_vec(),
                value,
            })
            .collect();
        node.subtree_items = node.items.len() as u64;
        self.dev
            .write_block(lba, &node.encode(4096, self.generation).unwrap())
            .unwrap();
    }

    // Fixture replacement only: registry transaction integration is a later gate.
    fn views(&mut self, generations: &[u64]) {
        let mut records = vec![(
            0,
            RegistryState {
                next_id: generations.len() as u64 + 1,
            }
            .encode()
            .unwrap()
            .to_vec(),
        )];
        records.extend(generations.iter().enumerate().map(|(i, &generation)| {
            (
                i as u64 + 1,
                SnapshotRecord {
                    generation,
                    committed_tx_id: generation,
                    object_map_root: 90,
                }
                .encode(self.generation, 8192)
                .unwrap()
                .to_vec(),
            )
        }));
        self.leaf(201, TreeKind::SnapshotRegistry, records);
    }

    fn runs(&mut self) -> Vec<LifetimeRun> {
        let mut records = Vec::new();
        visit_tree_nodes(
            &mut self.dev,
            &self.geo,
            self.roots.lifetimes,
            lifetime_spec(self.generation),
            |_, node| {
                if node.is_leaf() {
                    for item in &node.items {
                        let start = u64::from_be_bytes(item.key.as_slice().try_into().unwrap());
                        if start != 0 {
                            records.push(LifetimeRun {
                                start,
                                record: LifetimeRecord::decode(
                                    &item.value,
                                    start,
                                    self.generation,
                                    8192,
                                )?,
                            });
                        }
                    }
                }
                Ok(())
            },
        )
        .unwrap();
        records
    }

    fn apply(&mut self, changes: LifetimeChanges, max_records: usize) -> LifetimeMutation {
        let mut old_nodes = Vec::new();
        visit_tree_nodes(
            &mut self.dev,
            &self.geo,
            self.roots.lifetimes,
            lifetime_spec(self.generation),
            |lba, _| {
                old_nodes.push(lba);
                Ok(())
            },
        )
        .unwrap();
        self.protected_nodes.extend(old_nodes.iter().copied());
        let old_bytes: Vec<_> = old_nodes
            .iter()
            .map(|&lba| (lba, self.dev.inner().peek(lba)))
            .collect();
        let protected: Vec<_> = self.protected_nodes.iter().copied().collect();
        let mut allocator =
            ReservedTreePool::new((200..1000).filter(|&lba| lba != 201), &protected, &[]).unwrap();
        self.dev.reset();
        let result = mutate_lifetimes(
            &mut self.dev,
            &self.geo,
            &mut allocator,
            self.roots,
            self.generation,
            self.generation + 1,
            &changes,
            max_records,
            100,
        )
        .unwrap();
        assert_eq!(self.dev.stats().writes, 0, "preparation must stage writes");
        assert_eq!(self.dev.stats().flushes, 0);
        assert_eq!(
            self.dev.stats().reads,
            result.reads.pages_read + result.tree.stats.device_reads
        );
        for (lba, bytes) in &result.tree.writes {
            assert!(!self.protected_nodes.contains(lba));
            self.dev.write_block(*lba, bytes).unwrap();
        }
        for (lba, bytes) in old_bytes {
            assert_eq!(self.dev.inner().peek(lba), bytes);
        }
        self.roots.lifetimes = result.tree.root_lba;
        self.generation += 1;
        result
    }

    fn assert_oracle(&mut self, expected: &BTreeMap<u64, (u64, u64)>) {
        let mut actual = BTreeMap::new();
        let mut retained = 0;
        let runs = self.runs();
        for run in &runs {
            for block in run.start..run.start + run.record.blocks {
                assert!(actual
                    .insert(block, (run.record.birth, run.record.retirement))
                    .is_none());
                if run.record.retirement != 0 {
                    retained += 1;
                }
            }
        }
        assert_eq!(&actual, expected);
        assert_eq!(
            read_ledger_state(
                &mut self.dev,
                &self.geo,
                self.roots.lifetimes,
                self.generation
            )
            .unwrap()
            .0
            .retained_blocks,
            retained
        );
        for pair in runs.windows(2) {
            assert!(
                pair[0].start + pair[0].record.blocks != pair[1].start
                    || pair[0].record.birth != pair[1].record.birth
                    || pair[0].record.retirement != pair[1].record.retirement
            );
        }
    }
}

#[test]
fn split_retirement_preserves_birth_and_half_open_snapshot_ownership() {
    let mut h = Harness::new();
    let mut oracle: BTreeMap<_, _> = (1000..1010).map(|block| (block, (2, 0))).collect();
    h.apply(
        LifetimeChanges {
            allocations: vec![(1000, 4), (1004, 6)],
            ..Default::default()
        },
        10,
    );
    assert_eq!(h.runs().len(), 1);
    h.assert_oracle(&oracle);
    h.apply(
        LifetimeChanges {
            retirements: vec![(1003, 4)],
            ..Default::default()
        },
        10,
    );
    for block in 1003..1007 {
        oracle.get_mut(&block).unwrap().1 = 3;
    }
    h.assert_oracle(&oracle);
    h.apply(
        LifetimeChanges {
            retirements: vec![(1000, 3), (1007, 3)],
            next_scan_position: Some(1007),
            ..Default::default()
        },
        10,
    );
    for block in (1000..1003).chain(1007..1010) {
        oracle.get_mut(&block).unwrap().1 = 4;
    }
    h.assert_oracle(&oracle);
    assert_eq!(
        read_ledger_state(&mut h.dev, &h.geo, h.roots.lifetimes, h.generation)
            .unwrap()
            .0
            .scan_position,
        1007
    );
    let middle = h.runs()[1];
    h.views(&[2]);
    refuse(
        &mut h,
        &LifetimeChanges {
            transfers: vec![middle],
            ..Default::default()
        },
        10,
        100,
    );
    // A view at retirement sees the new namespace, so it cannot own this run.
    h.views(&[3]);
    let result = h.apply(
        LifetimeChanges {
            transfers: vec![middle],
            next_scan_position: Some(0),
            ..Default::default()
        },
        10,
    );
    assert_eq!(result.quarantine, [(1003, 4)]);
    for block in 1003..1007 {
        oracle.remove(&block);
    }
    h.assert_oracle(&oracle);
    let remaining = h.runs();
    refuse(
        &mut h,
        &LifetimeChanges {
            transfers: remaining.clone(),
            ..Default::default()
        },
        10,
        100,
    );
    h.views(&[]);
    h.apply(
        LifetimeChanges {
            transfers: remaining,
            ..Default::default()
        },
        10,
    );
    h.assert_oracle(&BTreeMap::new());
    // Reallocation establishes a new birth after the ownership gap.
    h.apply(
        LifetimeChanges {
            allocations: vec![(1000, 10)],
            ..Default::default()
        },
        10,
    );
    let expected = (1000..1010)
        .map(|block| (block, (h.generation, 0)))
        .collect();
    h.assert_oracle(&expected);
}

#[test]
fn sparse_large_ledger_edits_touch_only_neighbors_and_cow_paths() {
    let mut h = Harness::new();
    let starts: Vec<_> = (0..600).map(|index| 1000 + index * 2).collect();
    h.apply(
        LifetimeChanges {
            allocations: starts.iter().map(|&start| (start, 1)).collect(),
            ..Default::default()
        },
        700,
    );
    let mut oracle: BTreeMap<_, _> = starts.iter().map(|&start| (start, (2, 0))).collect();
    h.assert_oracle(&oracle);
    let result = h.apply(
        LifetimeChanges {
            retirements: vec![(1600, 1)],
            ..Default::default()
        },
        3,
    );
    println!("snapshot_lifetime_edit ledger_records=600 loaded_records={} preparation_reads={} preparation_peak_pages={} mutation_reads={} written_nodes={} mutation_peak_pages={}",
        result.loaded_records, result.reads.pages_read, result.reads.peak_page_buffers,
        result.tree.stats.device_reads, result.tree.writes.len(), result.tree.stats.max_live_decoded_nodes);
    assert_eq!(result.loaded_records, 3);
    assert!(result.reads.pages_read <= 9, "{:?}", result.reads);
    assert!(
        result.tree.stats.device_reads < 20,
        "{:?}",
        result.tree.stats
    );
    oracle.get_mut(&1600).unwrap().1 = 3;
    h.assert_oracle(&oracle);
}

struct NoAllocation;
impl<D: BlockDevice> TreeAllocator<D> for NoAllocation {
    fn allocate_tree_block(&mut self, _: &mut D) -> Result<u64, CoreError> {
        panic!("invalid preparation reached allocation")
    }
    fn retire_tree_block(&mut self, _: &mut D, _: u64) -> Result<(), CoreError> {
        panic!("invalid preparation reached retirement")
    }
    fn release_tree_block(&mut self, _: &mut D, _: u64) -> Result<(), CoreError> {
        panic!("invalid preparation reached release")
    }
}

fn refuse(h: &mut Harness, changes: &LifetimeChanges, max_records: usize, max_views: usize) {
    h.dev.reset();
    assert!(mutate_lifetimes(
        &mut h.dev,
        &h.geo,
        &mut NoAllocation,
        h.roots,
        h.generation,
        h.generation + 1,
        changes,
        max_records,
        max_views
    )
    .is_err());
    assert_eq!(h.dev.stats().writes, 0);
    assert_eq!(h.dev.stats().flushes, 0);
}

#[test]
fn invalid_ownership_and_resource_limits_fail_before_allocator_mutation() {
    let mut h = Harness::new();
    h.apply(
        LifetimeChanges {
            allocations: vec![(1000, 4)],
            ..Default::default()
        },
        10,
    );
    let live = h.runs()[0];
    for changes in [
        LifetimeChanges {
            allocations: vec![(1002, 1)],
            ..Default::default()
        },
        LifetimeChanges {
            retirements: vec![(1002, 4)],
            ..Default::default()
        },
        LifetimeChanges {
            retirements: vec![(1000, 2), (1001, 2)],
            ..Default::default()
        },
        LifetimeChanges {
            allocations: vec![(1020, 1)],
            retirements: vec![(1020, 1)],
            ..Default::default()
        },
        LifetimeChanges {
            transfers: vec![live],
            ..Default::default()
        },
        LifetimeChanges {
            next_scan_position: Some(8192),
            ..Default::default()
        },
    ] {
        refuse(&mut h, &changes, 10, 100);
    }
    refuse(
        &mut h,
        &LifetimeChanges {
            retirements: vec![(1001, 1)],
            ..Default::default()
        },
        1,
        100,
    );
    h.apply(
        LifetimeChanges {
            retirements: vec![(1000, 4)],
            ..Default::default()
        },
        10,
    );
    let retired = h.runs()[0];
    for transfers in [
        vec![retired, retired],
        vec![LifetimeRun {
            record: LifetimeRecord {
                blocks: 3,
                ..retired.record
            },
            ..retired
        }],
    ] {
        refuse(
            &mut h,
            &LifetimeChanges {
                transfers,
                ..Default::default()
            },
            10,
            100,
        );
    }
    refuse(
        &mut h,
        &LifetimeChanges {
            transfers: vec![retired],
            ..Default::default()
        },
        0,
        100,
    );
    h.assert_oracle(&(1000..1004).map(|block| (block, (2, 3))).collect());
}

#[test]
fn transfer_checks_views_beyond_first_page_and_never_truncates_to_budget() {
    let mut h = Harness::new();
    h.apply(
        LifetimeChanges {
            allocations: vec![(1000, 1)],
            ..Default::default()
        },
        10,
    );
    h.apply(
        LifetimeChanges {
            retirements: vec![(1000, 1)],
            ..Default::default()
        },
        10,
    );
    let retired = h.runs()[0];
    let mut views = vec![1; 80];
    views[79] = 2;
    h.views(&views);
    let changes = LifetimeChanges {
        transfers: vec![retired],
        ..Default::default()
    };
    refuse(&mut h, &changes, 10, 80);
    h.views(&[1; 80]);
    refuse(&mut h, &changes, 10, 79);
    let result = h.apply(changes, 10);
    assert_eq!(result.quarantine, [(1000, 1)]);
    h.assert_oracle(&BTreeMap::new());
}

#[test]
fn corrupt_retained_total_cannot_authorize_a_transfer() {
    let mut h = Harness::new();
    h.apply(
        LifetimeChanges {
            allocations: vec![(1000, 2)],
            ..Default::default()
        },
        10,
    );
    h.apply(
        LifetimeChanges {
            retirements: vec![(1000, 2)],
            ..Default::default()
        },
        10,
    );
    let run = h.runs()[0];
    let lba = h.roots.lifetimes;
    let (mut node, _) = TreeNode::decode(&h.dev.inner().peek(lba)).unwrap();
    node.items[0].value[8..16].copy_from_slice(&0u64.to_le_bytes());
    h.dev
        .write_block(lba, &node.encode(4096, h.generation).unwrap())
        .unwrap();
    refuse(
        &mut h,
        &LifetimeChanges {
            transfers: vec![run],
            ..Default::default()
        },
        10,
        100,
    );
}

#[test]
fn unordered_adjacent_retirements_coalesce_without_resetting_birth() {
    let mut h = Harness::new();
    h.apply(
        LifetimeChanges {
            allocations: vec![(1000, 20)],
            ..Default::default()
        },
        20,
    );
    h.apply(
        LifetimeChanges {
            retirements: vec![(1008, 4), (1000, 4), (1012, 8), (1004, 4)],
            ..Default::default()
        },
        20,
    );
    h.assert_oracle(&(1000..1020).map(|block| (block, (2, 3))).collect());
    let runs = h.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].record.blocks, 20);
    // Ledger release is a quarantine transfer, never immediate allocation.
    refuse(
        &mut h,
        &LifetimeChanges {
            allocations: vec![(1000, 20)],
            transfers: runs,
            ..Default::default()
        },
        20,
        100,
    );
}
