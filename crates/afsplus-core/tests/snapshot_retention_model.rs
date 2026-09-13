// SPDX-License-Identifier: BSD-2-Clause
//! Isolated Q4 accounting experiment. This does not exercise Volume, disk
//! encoding, crash recovery, metadata allocation, or the reserved allocator pool.
//! A block lifetime is [birth, retirement). All updates in this model are COW.
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy)]
enum Policy {
    OldestFifo,
    LifetimeScan,
}

struct Retired {
    block: usize,
    birth: u64,
    retirement: u64,
}

struct View {
    generation: u64,
    // Captured addresses and independent byte copies, never lifetime predicates.
    files: BTreeMap<u64, (usize, [u8; 32])>,
}

struct Model {
    generation: u64,
    media: Vec<[u8; 32]>,
    free: Vec<usize>,
    live: BTreeMap<u64, (usize, u64)>,
    retired: VecDeque<Retired>,
    views: BTreeMap<u64, View>,
    examined: usize,
}

impl Model {
    fn new(blocks: usize) -> Self {
        Self {
            generation: 1,
            media: vec![[0; 32]; blocks],
            free: (0..blocks).collect(),
            live: BTreeMap::new(),
            retired: VecDeque::new(),
            views: BTreeMap::new(),
            examined: 0,
        }
    }

    fn put(&mut self, file: u64, value: u8) -> Result<(), ()> {
        let block = self.free.pop().ok_or(())?;
        self.generation += 1;
        self.media[block] = [value; 32];
        if let Some((old, birth)) = self.live.insert(file, (block, self.generation)) {
            self.retired.push_back(Retired {
                block: old,
                birth,
                retirement: self.generation,
            });
        }
        Ok(())
    }

    fn unlink(&mut self, file: u64) {
        self.generation += 1;
        let (block, birth) = self.live.remove(&file).unwrap();
        self.retired.push_back(Retired {
            block,
            birth,
            retirement: self.generation,
        });
    }

    fn capture(&mut self, id: u64) {
        let files = self
            .live
            .iter()
            .map(|(&file, &(block, _))| (file, (block, self.media[block])))
            .collect();
        assert!(self
            .views
            .insert(
                id,
                View {
                    generation: self.generation,
                    files
                }
            )
            .is_none());
    }

    fn verify(&self) {
        for view in self.views.values() {
            for &(block, expected) in view.files.values() {
                assert!(!self.free.contains(&block), "snapshot block became free");
                assert_eq!(self.media[block], expected, "snapshot bytes changed");
            }
        }
        let mut ownership = vec![0; self.media.len()];
        for &block in &self.free {
            ownership[block] += 1;
        }
        for &(block, _) in self.live.values() {
            ownership[block] += 1;
        }
        for entry in &self.retired {
            ownership[entry.block] += 1;
        }
        assert!(ownership.iter().all(|&owners| owners == 1));
    }

    fn reclaim(&mut self, policy: Policy, budget: usize) -> usize {
        let mut released = 0;
        // Budget measures entries examined, including protected entries.
        for _ in 0..budget.min(self.retired.len()) {
            let entry = self.retired.pop_front().unwrap();
            self.examined += 1;
            let protected = match policy {
                Policy::OldestFifo => self
                    .views
                    .values()
                    .map(|v| v.generation)
                    .min()
                    .is_some_and(|oldest| entry.retirement > oldest),
                Policy::LifetimeScan => self
                    .views
                    .values()
                    .any(|v| entry.birth <= v.generation && v.generation < entry.retirement),
            };
            if protected {
                match policy {
                    Policy::OldestFifo => {
                        self.retired.push_front(entry);
                        break;
                    }
                    Policy::LifetimeScan => self.retired.push_back(entry),
                }
            } else {
                // Independent reachability check catches unsafe candidate rules.
                assert!(!self
                    .views
                    .values()
                    .any(|v| v.files.values().any(|&(block, _)| block == entry.block)));
                self.free.push(entry.block);
                released += 1;
            }
        }
        self.verify();
        released
    }
}

#[test]
fn unrelated_churn_exhausts_conservative_retention_but_lifetime_scan_is_bounded() {
    for capacity in [64, 256] {
        for policy in [Policy::OldestFifo, Policy::LifetimeScan] {
            let mut m = Model::new(capacity);
            m.put(1, 7).unwrap();
            m.capture(1);
            m.put(1, 8).unwrap(); // Protected FIFO head; only one snapshot-owned block.
            let mut completed = 0;
            for cycle in 0..capacity * 4 {
                if m.put(2, cycle as u8).is_err() {
                    break;
                }
                m.unlink(2);
                let before = m.examined;
                m.reclaim(policy, 8);
                assert!(m.examined - before <= 8);
                completed += 1;
            }
            match policy {
                Policy::OldestFifo => {
                    assert_eq!(completed, capacity - 2);
                    assert!(m.free.is_empty());
                    assert_eq!(m.retired.len(), capacity - 1);
                    let generation = m.generation;
                    assert!(m.put(1, 99).is_err());
                    assert_eq!(m.generation, generation);
                    assert_eq!(m.media[m.live[&1].0], [8; 32]);
                }
                Policy::LifetimeScan => {
                    assert_eq!(completed, capacity * 4);
                    assert_eq!(m.retired.len(), 1);
                    assert_eq!(m.free.len(), capacity - 2);
                }
            }
            println!("capacity={capacity} policy={} completed={completed} retained={} free={} examined={}",
                match policy { Policy::OldestFifo => "oldest-fifo", Policy::LifetimeScan => "lifetime-scan" },
                m.retired.len(), m.free.len(), m.examined);
            m.views.clear();
            let queued = m.retired.len();
            let before = m.examined;
            for _ in 0..queued.div_ceil(8) {
                m.reclaim(policy, 8);
            }
            assert!(m.retired.is_empty());
            assert_eq!(m.examined - before, queued);
            assert_eq!(m.free.len(), capacity - 1);
        }
    }
}

#[test]
fn lifetime_boundaries_and_out_of_order_release_preserve_each_view() {
    for order in [[1, 2, 3], [3, 1, 2], [2, 3, 1]] {
        let mut m = Model::new(16);
        m.put(1, 11).unwrap();
        m.capture(1); // birth == snapshot: must retain.
        m.put(1, 22).unwrap();
        m.capture(2); // previous retirement == snapshot: not its owner.
        m.put(1, 33).unwrap();
        m.capture(3);
        m.unlink(1);
        assert_eq!(m.reclaim(Policy::LifetimeScan, 8), 0);
        for id in order {
            m.views.remove(&id);
            assert_eq!(m.reclaim(Policy::LifetimeScan, 8), 1);
            // Overwrite released media; retained views must keep their bytes.
            m.put(99, 99).unwrap();
            m.verify();
            m.unlink(99);
            assert_eq!(m.reclaim(Policy::LifetimeScan, 8), 1);
        }
        assert_eq!(m.free.len(), 16);
    }
}

#[test]
fn bounded_scan_reaches_unowned_entries_behind_protected_runs() {
    let mut m = Model::new(80);
    for file in 0..32 {
        m.put(file, file as u8).unwrap();
    }
    m.capture(1);
    for file in 0..32 {
        m.unlink(file);
    }
    m.put(99, 99).unwrap();
    m.unlink(99);
    let before = m.examined;
    for _ in 0..5 {
        m.reclaim(Policy::LifetimeScan, 8);
    }
    assert_eq!(m.retired.len(), 32);
    assert!(m.examined - before <= 40);
    m.verify();
}

#[test]
fn retirement_at_snapshot_boundary_is_reclaimable() {
    for policy in [Policy::OldestFifo, Policy::LifetimeScan] {
        let mut m = Model::new(4);
        m.put(1, 1).unwrap();
        m.put(1, 2).unwrap();
        m.capture(1);
        assert_eq!(m.reclaim(policy, 1), 1);
        m.put(2, 3).unwrap();
        m.verify();
    }
}

#[test]
fn reachability_oracle_rejects_early_reuse() {
    let mut m = Model::new(4);
    m.put(1, 11).unwrap();
    m.capture(1);
    m.put(1, 22).unwrap();
    let old = m.retired.pop_front().unwrap();
    m.free.push(old.block); // Deliberately broken reclamation.
    m.put(2, 99).unwrap();
    assert!(std::panic::catch_unwind(|| m.verify()).is_err());
}
