//! Real bitmap/quarantine/checkpoint integration for lifetime accounting.
//! Namespace file operations and supported snapshot mounts remain separate gates.
use afsplus_block::{
    for_each_crash_state, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend,
};
use afsplus_core::alloc::{Bitmaps, FinishedAlloc, TxAllocator};
use afsplus_core::mount::select_checkpoint;
use afsplus_core::snapshot::{lifetime_spec, read_ledger_state, LifetimeRun};
use afsplus_core::tree::visit_tree_nodes;
use afsplus_core::{allocation_root, mkfs, mount, reclaim, CoreError, MkfsParams, NamePolicy};
use afsplus_format::checkpoint::{Checkpoint, SnapshotRoots};
use afsplus_format::geometry::Geometry;
use afsplus_format::ident::{Identification, INCOMPAT_PERSISTENT_SNAPSHOTS};
use afsplus_format::snapshot::{LedgerState, LifetimeRecord, RegistryState};
use afsplus_format::tree::{key_u64, TreeItem, TreeKind, TreeNode};
use afsplus_format::Timespec;

struct Harness {
    dev: MemoryBackend,
    ident: Identification,
    checkpoint: Checkpoint,
    older: Option<Checkpoint>,
    slot: usize,
}

fn leaf(
    dev: &mut MemoryBackend,
    lba: u64,
    kind: TreeKind,
    generation: u64,
    records: Vec<(u64, Vec<u8>)>,
) {
    let mut node = TreeNode::leaf(kind, 0);
    node.items = records
        .into_iter()
        .map(|(key, value)| TreeItem {
            key: key_u64(key).to_vec(),
            value,
        })
        .collect();
    node.subtree_items = node.items.len() as u64;
    dev.write_block(lba, &node.encode(4096, generation).unwrap())
        .unwrap();
}

// A single-region test publisher uses the real descriptor/bitmap and reserved
// allocation-root slots, then the common metadata-before-checkpoint barriers.
fn publish<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    current: &Checkpoint,
    older: Option<&Checkpoint>,
    slot: usize,
    mut done: FinishedAlloc,
) -> Checkpoint {
    let geo = ident.geometry();
    assert_eq!(geo.region_count(), 1);
    let mut allocation_root_block = current.allocation_root_block;
    if let Some((region, record)) = done.dirty_records.first() {
        assert_eq!(*region, 0);
        assert_eq!(done.dirty_records.len(), 1);
        allocation_root_block = allocation_root::reserved_pool_lbas(&geo)
            .unwrap()
            .into_iter()
            .find(|&lba| {
                lba != current.allocation_root_block
                    && older.is_none_or(|old| lba != old.allocation_root_block)
            })
            .unwrap();
        let node = allocation_root::initial_leaf(0, *record).unwrap();
        dev.write_block(
            allocation_root_block,
            &node.encode(4096, current.generation + 1).unwrap(),
        )
        .unwrap();
    }
    if let Some(mutation) = done.snapshot_lifetimes.take() {
        assert_eq!(
            done.snapshot_roots.unwrap().lifetimes,
            mutation.tree.root_lba
        );
        for (lba, bytes) in mutation.tree.writes {
            dev.write_block(lba, &bytes).unwrap();
        }
    }
    for (lba, bytes) in done
        .reclaim_writes
        .iter()
        .chain(&done.bitmap_writes)
        .chain(&done.descriptor_writes)
    {
        dev.write_block(*lba, bytes).unwrap();
    }
    dev.flush().unwrap();
    let next = Checkpoint {
        generation: current.generation + 1,
        committed_tx_id: current.generation + 1,
        allocation_root_block,
        reclaim_root_block: done.reclaim_root_lba,
        free_blocks_total: done.free_blocks_total,
        snapshot_roots: done.snapshot_roots,
        ..current.clone()
    };
    dev.write_block(
        ident.checkpoint_slots[1 - slot],
        &next.encode(4096).unwrap(),
    )
    .unwrap();
    dev.flush().unwrap();
    next
}

impl Harness {
    fn new() -> Self {
        let mut dev = MemoryBackend::new(4096, 4096);
        mkfs(
            &mut dev,
            &MkfsParams {
                uuid: [71; 16],
                label: "SnapshotAllocator".into(),
                region_size: 4096,
                reclaim_caps: Default::default(),
                log_slots: 0,
                shared_extents: false,
                data_policy: false,
                name_policy: NamePolicy::Sensitive,
                timestamp: Timespec::default(),
            },
        )
        .unwrap();
        let volume = mount(dev).unwrap();
        let mut ident = volume.ident().clone();
        let checkpoint = volume.checkpoint().clone();
        let mut dev = volume.into_device();
        let geo = ident.geometry();
        // Fixture bootstrap, before tracking is enabled. No production
        // conversion from a feature-absent filesystem is implied.
        let mut seed = TxAllocator::begin(&mut dev, &geo, &checkpoint, None, 2, 0, 0).unwrap();
        let registry = seed.allocate(&mut dev).unwrap();
        let lifetimes = seed.allocate(&mut dev).unwrap();
        leaf(
            &mut dev,
            registry,
            TreeKind::SnapshotRegistry,
            2,
            vec![(0, RegistryState { next_id: 1 }.encode().unwrap().to_vec())],
        );
        let start = geo.region0_reserved_blocks();
        leaf(
            &mut dev,
            lifetimes,
            TreeKind::SnapshotLifetimes,
            2,
            vec![
                (
                    0,
                    LedgerState {
                        scan_position: 0,
                        retained_blocks: 0,
                    }
                    .encode(4096)
                    .unwrap()
                    .to_vec(),
                ),
                (
                    start,
                    LifetimeRecord {
                        blocks: 3,
                        birth: 1,
                        retirement: 0,
                    }
                    .encode(start, 2, 4096)
                    .unwrap()
                    .to_vec(),
                ),
            ],
        );
        let mut done = seed.finish(&mut dev).unwrap();
        done.snapshot_roots = Some(SnapshotRoots {
            registry,
            lifetimes,
        });
        ident.features.incompat |= INCOMPAT_PERSISTENT_SNAPSHOTS;
        dev.write_block(0, &ident.encode(4096).unwrap()).unwrap();
        let next = publish(&mut dev, &ident, &checkpoint, None, 0, done);
        let h = Self {
            dev,
            ident,
            checkpoint: next,
            older: Some(checkpoint),
            slot: 1,
        };
        assert!(matches!(
            mount(h.dev.clone()),
            Err(CoreError::UnsupportedIncompatFeatures(_))
        ));
        h
    }

    fn geo(&self) -> Geometry {
        self.ident.geometry()
    }
    fn begin(&mut self, batch: u64) -> TxAllocator {
        let geo = self.geo();
        TxAllocator::begin(
            &mut self.dev,
            &geo,
            &self.checkpoint,
            self.older.as_ref(),
            self.checkpoint.generation + 1,
            batch,
            0,
        )
        .unwrap()
    }
    fn commit(&mut self, tx: TxAllocator) {
        let done = tx.finish(&mut self.dev).unwrap();
        let next = publish(
            &mut self.dev,
            &self.ident,
            &self.checkpoint,
            self.older.as_ref(),
            self.slot,
            done,
        );
        self.older = Some(std::mem::replace(&mut self.checkpoint, next));
        self.slot = 1 - self.slot;
        let selected = select_checkpoint(&mut self.dev, &self.ident).unwrap();
        assert_eq!(
            selected.chosen.snapshot_roots,
            self.checkpoint.snapshot_roots
        );
        assert_eq!(selected.chosen.generation, self.checkpoint.generation);
    }
    fn tracked(&mut self, lba: u64) -> Option<LifetimeRun> {
        tracked(&mut self.dev, &self.ident, &self.checkpoint, lba)
    }
    fn bit(&mut self, lba: u64) -> bool {
        let geo = self.geo();
        Bitmaps::load(&mut self.dev, &geo, &self.checkpoint)
            .unwrap()
            .is_allocated(lba)
    }
    fn queued(&mut self, lba: u64) -> Option<u64> {
        queued(&mut self.dev, &self.ident, &self.checkpoint, lba)
    }
}

fn tracked<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    lba: u64,
) -> Option<LifetimeRun> {
    let mut found = None;
    visit_tree_nodes(
        dev,
        &ident.geometry(),
        checkpoint.snapshot_roots.unwrap().lifetimes,
        lifetime_spec(checkpoint.generation),
        |_, node| {
            if node.is_leaf() {
                for item in &node.items {
                    let start = u64::from_be_bytes(item.key.as_slice().try_into().unwrap());
                    if start != 0 {
                        let record = LifetimeRecord::decode(
                            &item.value,
                            start,
                            checkpoint.generation,
                            ident.total_blocks,
                        )?;
                        if start <= lba && lba < start + record.blocks {
                            assert!(found.replace(LifetimeRun { start, record }).is_none());
                        }
                    }
                }
            }
            Ok(())
        },
    )
    .unwrap();
    found
}
fn queued<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    lba: u64,
) -> Option<u64> {
    reclaim::load_all(
        dev,
        &ident.geometry(),
        checkpoint.reclaim_root_block,
        checkpoint.generation,
    )
    .unwrap()
    .runs
    .into_iter()
    .find(|run| run.start <= lba && lba < run.start + run.blocks as u64)
    .map(|run| run.retire_generation)
}

fn seal(h: &mut Harness, tx: &mut TxAllocator, transfers: Vec<LifetimeRun>) {
    tx.begin_snapshot_housekeeping().unwrap();
    tx.seal_snapshot_lifetimes(&mut h.dev, transfers, Some(0), 128, 32)
        .unwrap();
}

#[test]
fn namespace_births_exclude_released_sacrificed_and_housekeeping_blocks() {
    println!(
        "snapshot_allocator inline_bytes={}",
        std::mem::size_of::<TxAllocator>()
    );
    let mut h = Harness::new();
    let mut tx = h.begin(0);
    let live = tx.allocate_run(&mut h.dev, 3).unwrap();
    tx.release_uncommitted(&mut h.dev, live + 1).unwrap();
    let sacrificed = tx.allocate(&mut h.dev).unwrap();
    tx.abandon_uncommitted_run(&mut h.dev, sacrificed, 1)
        .unwrap();
    let exact = 100;
    tx.allocate_exact_run(&mut h.dev, exact, 2).unwrap();
    tx.begin_snapshot_housekeeping().unwrap();
    let housekeeping = tx.allocate(&mut h.dev).unwrap();
    tx.seal_snapshot_lifetimes(&mut h.dev, vec![], None, 128, 32)
        .unwrap();
    assert!(tx.release_uncommitted(&mut h.dev, live).is_err());
    assert!(tx.allocate(&mut h.dev).is_err());
    assert!(tx
        .retire(&mut h.dev, h.checkpoint.object_map_block)
        .is_err());
    h.commit(tx);
    for block in [live, live + 2, exact, exact + 1] {
        assert_eq!(h.tracked(block).unwrap().record.birth, 3);
        assert!(h.bit(block));
    }
    assert!(h.tracked(sacrificed).is_none());
    assert_eq!(h.queued(sacrificed), Some(3));
    assert!(h.bit(sacrificed));
    assert!(h.tracked(housekeeping).is_none());
    assert!(h.bit(housekeeping));
    let ledger = h.checkpoint.snapshot_roots.unwrap().lifetimes;
    assert!(
        h.tracked(ledger).is_none(),
        "ledger nodes must not recursively own lifetimes"
    );
    assert!(h.bit(ledger));
}

#[test]
fn finalization_refuses_unsealed_or_failed_snapshot_accounting() {
    let mut h = Harness::new();
    let tx = h.begin(0);
    assert!(tx.finish(&mut h.dev).is_err());
    let mut tx = h.begin(0);
    tx.allocate(&mut h.dev).unwrap();
    tx.begin_snapshot_housekeeping().unwrap();
    assert!(tx.finish(&mut h.dev).is_err());
    let mut tx = h.begin(0);
    tx.allocate(&mut h.dev).unwrap();
    tx.begin_snapshot_housekeeping().unwrap();
    assert!(tx
        .seal_snapshot_lifetimes(&mut h.dev, vec![], None, 0, 32)
        .is_err());
    assert!(tx.allocate(&mut h.dev).is_err());
    assert!(tx.allocate_exact_run(&mut h.dev, 100, 1).is_err());
    assert!(tx.begin_snapshot_housekeeping().is_err());
    assert!(tx
        .seal_snapshot_lifetimes(&mut h.dev, vec![], None, 128, 32)
        .is_err());
    assert!(tx.finish(&mut h.dev).is_err());
}

fn retire_one() -> (Harness, u64, LifetimeRun) {
    let mut h = Harness::new();
    let mut tx = h.begin(0);
    let block = tx.allocate(&mut h.dev).unwrap();
    seal(&mut h, &mut tx, vec![]);
    h.commit(tx);
    let mut tx = h.begin(0);
    tx.retire(&mut h.dev, block).unwrap();
    seal(&mut h, &mut tx, vec![]);
    h.commit(tx);
    assert!(h.bit(block));
    assert!(h.queued(block).is_none());
    let run = h.tracked(block).unwrap();
    assert_eq!((run.record.birth, run.record.retirement), (3, 4));
    (h, block, run)
}

#[test]
fn retired_snapshot_storage_reaches_free_only_after_committed_quarantine_transfer() {
    let (mut h, block, run) = retire_one();
    let mut tx = h.begin(u64::MAX);
    assert!(tx.allocate_exact_run(&mut h.dev, block, 1).is_err());
    // The failed exact claim changes no state; discard before the valid test.
    let mut tx = h.begin(u64::MAX);
    seal(&mut h, &mut tx, vec![run]);
    h.commit(tx);
    assert!(h.tracked(block).is_none());
    assert!(h.bit(block));
    assert_eq!(h.queued(block), Some(5));
    let mut tx = h.begin(u64::MAX);
    assert!(tx.allocate_exact_run(&mut h.dev, block, 1).is_err());
    // G4 remains protected while G6 is prepared. Advance the checkpoint
    // boundary without claiming storage queued at G5.
    let mut tx = h.begin(u64::MAX);
    seal(&mut h, &mut tx, vec![]);
    h.commit(tx);
    assert!(h.bit(block));
    assert_eq!(h.queued(block), Some(5));
    let mut tx = h.begin(u64::MAX);
    tx.allocate_exact_run(&mut h.dev, block, 1).unwrap();
    seal(&mut h, &mut tx, vec![]);
    h.commit(tx);
    assert_eq!(h.tracked(block).unwrap().record.birth, 7);
    assert!(h.bit(block));
    assert!(h.queued(block).is_none());
}

#[test]
fn every_transfer_publication_cut_preserves_ledger_or_quarantine_ownership() {
    let (h, block, run) = retire_one();
    let mut dev = RecordingBackend::new(h.dev.clone());
    let mut tx =
        TxAllocator::begin(&mut dev, &h.geo(), &h.checkpoint, h.older.as_ref(), 5, 0, 0).unwrap();
    tx.begin_snapshot_housekeeping().unwrap();
    tx.seal_snapshot_lifetimes(&mut dev, vec![run], Some(0), 128, 32)
        .unwrap();
    let done = tx.finish(&mut dev).unwrap();
    publish(
        &mut dev,
        &h.ident,
        &h.checkpoint,
        h.older.as_ref(),
        h.slot,
        done,
    );
    let (_, log) = dev.into_parts();
    let mut cases = 0;
    let mut generations = std::collections::BTreeSet::new();
    for cut in 0..=log.len() {
        for_each_crash_state(&h.dev, &log, cut, |state| {
            let mut dev = state.image;
            let checkpoint = select_checkpoint(&mut dev, &h.ident).unwrap().chosen;
            generations.insert(checkpoint.generation);
            assert!(Bitmaps::load(&mut dev, &h.geo(), &checkpoint)
                .unwrap()
                .is_allocated(block));
            let lifetime = tracked(&mut dev, &h.ident, &checkpoint, block);
            let queue = queued(&mut dev, &h.ident, &checkpoint, block);
            let retained = read_ledger_state(
                &mut dev,
                &h.geo(),
                checkpoint.snapshot_roots.unwrap().lifetimes,
                checkpoint.generation,
            )
            .unwrap()
            .0
            .retained_blocks;
            match checkpoint.generation {
                4 => {
                    assert_eq!(lifetime, Some(run));
                    assert_eq!(queue, None);
                    assert_eq!(retained, 1);
                }
                5 => {
                    assert_eq!(lifetime, None);
                    assert_eq!(queue, Some(5));
                    assert_eq!(retained, 0);
                }
                other => panic!("unexpected generation {other}"),
            }
            cases += 1;
        });
    }
    assert_eq!(generations, [4, 5].into_iter().collect());
    assert!(cases > 20);
    // Negative control: omit the metadata barrier, allowing a durable-looking
    // new checkpoint to race ahead of its ledger/bitmap/quarantine state.
    let mut misordered = log.clone();
    let barrier = misordered
        .iter()
        .position(|op| matches!(op, RecordedOp::Flush))
        .unwrap();
    misordered.remove(barrier);
    let mut detected = false;
    for_each_crash_state(&h.dev, &misordered, misordered.len() - 1, |state| {
        let mut dev = state.image;
        if let Ok(selection) = select_checkpoint(&mut dev, &h.ident) {
            let checkpoint = selection.chosen;
            if checkpoint.generation == 5 {
                detected |= read_ledger_state(
                    &mut dev,
                    &h.geo(),
                    checkpoint.snapshot_roots.unwrap().lifetimes,
                    5,
                )
                .is_err()
                    || Bitmaps::load(&mut dev, &h.geo(), &checkpoint).is_err()
                    || reclaim::load_all(&mut dev, &h.geo(), checkpoint.reclaim_root_block, 5)
                        .is_err();
            }
        }
    });
    assert!(
        detected,
        "oracle must catch checkpoint publication without metadata barrier"
    );
    println!(
        "snapshot_transfer_cut cases={cases} generations=4,5 missing_barrier_detected={detected}"
    );
}

#[test]
fn missing_older_slot_adds_no_extra_quarantine_generation() {
    let (mut h, block, run) = retire_one();
    let mut tx = h.begin(u64::MAX);
    seal(&mut h, &mut tx, vec![run]);
    h.commit(tx);
    h.dev
        .write_block(h.ident.checkpoint_slots[1 - h.slot], &[0; 4096])
        .unwrap();
    let selected = select_checkpoint(&mut h.dev, &h.ident).unwrap();
    assert!(selected.other.is_none());
    h.older = selected.other;
    let mut tx = h.begin(u64::MAX);
    tx.allocate_exact_run(&mut h.dev, block, 1).unwrap();
    seal(&mut h, &mut tx, vec![]);
    h.commit(tx);
    assert_eq!(h.tracked(block).unwrap().record.birth, 6);
}
