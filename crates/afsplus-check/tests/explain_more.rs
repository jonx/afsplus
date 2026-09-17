//! ExplainExtent, ExplainCheckpoint, ExplainReclaim, ExplainSpace and
//! ExplainFeature against witnesses that share no code with the explain
//! walk: the bytes the core reads, the core's mount selection, the checker's
//! committed state, and the identification block.
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_check::explain::{Explainer, ExtentState, FeatureClass};
use afsplus_core::mount::select_checkpoint;
use afsplus_core::verify::load_committed_state;
use afsplus_core::{
    mkfs, mkfs_with_security_descriptors, mkfs_with_snapshots_and_security_descriptors, mount,
    MkfsParams, NamePolicy,
};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, OBJECT_ROOT};

const BLOCK: usize = 4096;

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 0,
    }
}

fn params() -> MkfsParams {
    MkfsParams {
        uuid: [0xe9; 16],
        label: "ExplainMore".into(),
        region_size: 512,
        reclaim_caps: Default::default(),
        log_slots: 4,
        shared_extents: true,
        data_policy: true,
        name_policy: NamePolicy::Sensitive,
        timestamp: time(1),
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(29).wrapping_add(seed))
        .collect()
}

struct Image {
    dev: MemoryBackend,
    direct: u64,
    sparse: u64,
    clone: u64,
    dir: u64,
}

/// Two regions in use: a direct file, a sparse extent-tree file with a
/// preallocated tail, its clone, a directory, deletions for the reclaim
/// queue.
fn populate() -> Image {
    let mut dev = MemoryBackend::new(BLOCK, 1536);
    mkfs_with_security_descriptors(&mut dev, &params()).unwrap();
    let mut volume = mount(dev).unwrap();
    let direct = volume
        .create_file_in_directory(OBJECT_ROOT, "direct", &pattern(6000, 1), time(2))
        .unwrap();
    let sparse = volume
        .create_file_in_directory(OBJECT_ROOT, "sparse", &pattern(4096, 2), time(2))
        .unwrap();
    volume
        .write_file_at(sparse, 40 * BLOCK as u64, &pattern(9000, 3), time(2))
        .unwrap();
    volume
        .preallocate_file(sparse, 10 * BLOCK as u64, 3 * BLOCK as u64, time(2))
        .unwrap();
    let clone = volume
        .clone_file(sparse, OBJECT_ROOT, "clone", time(3))
        .unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(3))
        .unwrap();
    for index in 0..40 {
        volume
            .create_file_in_directory(
                dir,
                &format!("big-{index}"),
                &pattern(5 * BLOCK, 9),
                time(3),
            )
            .unwrap();
    }
    for index in 0..10 {
        volume
            .delete_file(dir, &format!("big-{index}"), time(4))
            .unwrap();
    }
    Image {
        dev: volume.into_device(),
        direct,
        sparse,
        clone,
        dir,
    }
}

#[test]
fn every_offset_of_a_file_is_where_the_core_reads_it() {
    let Image {
        mut dev,
        direct,
        sparse,
        clone,
        dir,
    } = populate();
    let explainer = Explainer::load(&mut dev).unwrap();
    assert_eq!(explainer.problems, Vec::<String>::new());
    let mut volume = mount(dev.clone()).unwrap();
    let mut states = [0u32; 5];
    for object in [direct, sparse, clone] {
        let size = volume.stat(object).unwrap().unwrap().size_bytes;
        // Every block boundary, the byte before it, and an odd offset inside.
        let mut offsets = vec![size - 1, size, size + 1, u64::MAX];
        for block in 0..=size / BLOCK as u64 {
            let base = block * BLOCK as u64;
            offsets.extend([base, base + 1234, base + BLOCK as u64 - 1]);
        }
        for offset in offsets {
            let explained = explainer.explain_extent(object, offset).unwrap();
            assert_eq!(explained.logical_block, offset / BLOCK as u64);
            assert_eq!(u64::from(explained.offset_in_block), offset % BLOCK as u64);
            if offset >= size {
                assert_eq!(explained.state, ExtentState::BeyondEnd, "{object} {offset}");
                states[0] += 1;
                continue;
            }
            let mut byte = [0u8; 1];
            assert_eq!(volume.read_file_at(object, offset, &mut byte).unwrap(), 1);
            match explained.state {
                ExtentState::BeyondEnd => panic!("{object} {offset} is inside the file"),
                ExtentState::Hole => {
                    assert_eq!(byte[0], 0);
                    states[1] += 1;
                }
                ExtentState::Mapped {
                    physical_block,
                    extent_logical_start,
                    extent_physical_start,
                    extent_blocks,
                    shared,
                    unwritten,
                } => {
                    assert!((extent_logical_start..extent_logical_start + extent_blocks)
                        .contains(&explained.logical_block));
                    assert_eq!(
                        physical_block - extent_physical_start,
                        explained.logical_block - extent_logical_start
                    );
                    if unwritten {
                        assert_eq!(byte[0], 0);
                        states[2] += 1;
                    } else {
                        let mut block = vec![0u8; BLOCK];
                        dev.read_block(physical_block, &mut block).unwrap();
                        assert_eq!(
                            block[explained.offset_in_block as usize], byte[0],
                            "{object} {offset}"
                        );
                        states[3] += 1;
                    }
                    // The sparse file and its clone share every written block.
                    if shared {
                        assert!(object == sparse || object == clone);
                        states[4] += 1;
                    } else {
                        assert!(object == direct || unwritten, "{object} {offset}");
                    }
                }
            }
        }
    }
    assert!(states.iter().all(|count| *count > 0), "{states:?}");
    assert!(explainer.explain_extent(dir, 0).is_err());
    assert!(explainer.explain_extent(999, 0).is_err());
}

#[test]
fn checkpoint_reclaim_and_space_agree_with_the_core_and_the_checker() {
    let Image { mut dev, dir, .. } = populate();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let explainer = Explainer::load(&mut dev).unwrap();

    // Checkpoint: the core's own selection and fields.
    let mut block = vec![0u8; BLOCK];
    dev.read_block(0, &mut block).unwrap();
    let ident = Identification::decode(&block).unwrap();
    let selection = select_checkpoint(&mut dev, &ident).unwrap();
    let explained = explainer.explain_checkpoint();
    let chosen = &selection.chosen;
    assert_eq!(
        (
            explained.generation,
            explained.committed_tx_id,
            explained.next_object_id,
            explained.free_blocks_total,
            explained.object_map_block,
            explained.allocation_root_block,
            explained.reclaim_root_block,
            explained.shared_extent_root_block,
            explained.label.as_str(),
            explained.snapshot_roots,
        ),
        (
            chosen.generation,
            chosen.committed_tx_id,
            chosen.next_object_id,
            chosen.free_blocks_total,
            chosen.object_map_block,
            chosen.allocation_root_block,
            chosen.reclaim_root_block,
            chosen.shared_extent_root_block,
            "ExplainMore",
            None,
        )
    );
    for slot in &explained.slots {
        assert_eq!(slot.block, 1 + u64::from(slot.slot));
        assert_eq!(
            slot.selected,
            usize::from(slot.slot) == selection.chosen_slot
        );
    }
    let other = selection.other.as_ref().unwrap();
    assert_eq!(
        explained.slots[1 - selection.chosen_slot].state,
        Ok(other.generation)
    );
    assert_eq!(
        explained.slots[selection.chosen_slot].state,
        Ok(chosen.generation)
    );

    // Reclaim: the checker's committed state.
    let state = load_committed_state(&mut dev, &ident, chosen).unwrap();
    assert!(!state.reclaim_runs.is_empty());
    let summary = explainer.explain_reclaim(None).unwrap();
    assert_eq!(summary.blocks, state.reclaim_pending_blocks);
    assert_eq!((summary.block, summary.run), (None, None));
    assert_eq!(
        summary.oldest_retire_generation,
        state
            .reclaim_runs
            .iter()
            .map(|run| run.retire_generation)
            .min()
    );
    let mut quarantined = std::collections::BTreeSet::new();
    for run in &state.reclaim_runs {
        for lba in run.start..run.start + u64::from(run.blocks) {
            quarantined.insert(lba);
            let found = explainer.explain_reclaim(Some(lba)).unwrap().run.unwrap();
            assert!((found.start..found.start + u64::from(found.blocks)).contains(&lba));
            assert_eq!(found.retire_generation, run.retire_generation);
            assert!(found.position < summary.runs);
        }
    }
    assert_eq!(quarantined.len() as u64, summary.blocks);
    for lba in 0..dev.total_blocks() {
        if !quarantined.contains(&lba) {
            assert_eq!(explainer.explain_reclaim(Some(lba)).unwrap().run, None);
        }
    }
    assert!(explainer.explain_reclaim(Some(dev.total_blocks())).is_err());

    // Space: three regions that add up to the volume and to the checkpoint.
    let regions: Vec<_> = (0..3)
        .map(|r| explainer.explain_space(r).unwrap())
        .collect();
    assert!(explainer.explain_space(3).is_err());
    assert_eq!(regions.iter().map(|r| r.blocks).sum::<u64>(), 1536);
    assert_eq!(
        regions.iter().map(|r| r.free_blocks).sum::<u64>(),
        chosen.free_blocks_total
    );
    assert_eq!(
        regions.iter().map(|r| r.quarantined_blocks).sum::<u64>(),
        summary.blocks
    );
    for region in &regions {
        assert_eq!(
            region.reserved_blocks + region.allocated_blocks + region.free_blocks,
            region.blocks
        );
        assert_eq!(region.first_block, u64::from(region.region) * 512);
        assert_eq!(region.unowned_blocks, 0);
        assert!(region.live_descriptor_slot.is_some());
        // The longest free run, by brute force over the core's bitmap.
        let mut best: Option<(u64, u64)> = None;
        let mut current: Option<(u64, u64)> = None;
        for lba in region.first_block..region.first_block + region.blocks {
            let free = ident.geometry().is_allocatable(lba) && !state.bitmaps.is_allocated(lba);
            current = match (free, current) {
                (true, Some((start, length))) => Some((start, length + 1)),
                (true, None) => Some((lba, 1)),
                (false, _) => None,
            };
            if let Some(run) = current {
                if best.is_none_or(|b| run.1 > b.1) {
                    best = Some(run);
                }
            }
        }
        assert_eq!(region.largest_free_run, best, "region {}", region.region);
    }
    assert!(regions[0].allocated_blocks > 0 && regions[2].free_blocks > 0);

    // One more commit flips the selected slot.
    let mut volume = mount(dev).unwrap();
    volume.delete_file(dir, "big-20", time(9)).unwrap();
    let mut dev = volume.into_device();
    let after = Explainer::load(&mut dev).unwrap().explain_checkpoint();
    assert_eq!(after.generation, explained.generation + 1);
    assert_ne!(after.slots[0].selected, explained.slots[0].selected);

    // A valid newest checkpoint in the snapshot-bearing form, on a volume
    // without the feature: the core refuses the volume, and so does explain.
    let mut foreign = dev.clone();
    let newest = usize::from(after.slots[1].selected);
    foreign.read_block(1 + newest as u64, &mut block).unwrap();
    let mut form = afsplus_format::checkpoint::Checkpoint::decode(&block, &[0xe9; 16]).unwrap();
    form.snapshot_roots = Some(afsplus_format::checkpoint::SnapshotRoots {
        registry: 100,
        lifetimes: 101,
    });
    foreign
        .write_block(1 + newest as u64, &form.encode(BLOCK).unwrap())
        .unwrap();
    assert!(select_checkpoint(&mut foreign, &ident).is_err());
    assert!(Explainer::load(&mut foreign).is_err());

    // A freshly formatted volume has one slot that was never written.
    let mut fresh = MemoryBackend::new(BLOCK, 1024);
    mkfs(&mut fresh, &params()).unwrap();
    let slots = Explainer::load(&mut fresh)
        .unwrap()
        .explain_checkpoint()
        .slots;
    assert_eq!(slots[0].state, Ok(1));
    assert_eq!(slots[1].state, Err("never written".to_owned()));

    // A slot that does not decode says why, and the other is selected.
    let mut block = vec![0u8; BLOCK];
    let selected = usize::from(after.slots[1].selected);
    dev.read_block(1 + selected as u64, &mut block).unwrap();
    block[40] ^= 1;
    dev.write_block(1 + selected as u64, &block).unwrap();
    let damaged = Explainer::load(&mut dev).unwrap().explain_checkpoint();
    assert!(damaged.slots[selected].state.is_err());
    assert!(damaged.slots[1 - selected].selected);
    assert_eq!(damaged.generation, explained.generation);
}

#[test]
fn features_are_the_bits_of_the_identification_block() {
    type Format = fn(&mut MemoryBackend, &MkfsParams) -> Result<(), afsplus_core::CoreError>;
    let cases: [(Format, &[&str]); 3] = [
        (
            mkfs,
            &[
                "org.aros.afsplus:data-policy",
                "org.aros.afsplus:shared-extents",
                "org.aros.afsplus:orphan-directory",
                "org.aros.afsplus:intent-log",
                "org.aros.afsplus:intent-log-data-updates",
            ],
        ),
        (
            mkfs_with_security_descriptors,
            &[
                "org.aros.afsplus:data-policy",
                "org.aros.afsplus:shared-extents",
                "org.aros.afsplus:orphan-directory",
                "org.aros.afsplus:intent-log",
                "org.aros.afsplus:intent-log-data-updates",
                "org.aros.afsplus:security-descriptors",
            ],
        ),
        (
            mkfs_with_snapshots_and_security_descriptors,
            &[
                "org.aros.afsplus:data-policy",
                "org.aros.afsplus:shared-extents",
                "org.aros.afsplus:orphan-directory",
                "org.aros.afsplus:persistent-snapshots",
                "org.aros.afsplus:security-descriptors",
            ],
        ),
    ];
    for (format, expected) in cases {
        let mut dev = MemoryBackend::new(BLOCK, 1024);
        let log_slots = if expected.contains(&"org.aros.afsplus:intent-log") {
            4
        } else {
            0
        };
        format(
            &mut dev,
            &MkfsParams {
                log_slots,
                ..params()
            },
        )
        .unwrap();
        let mut block = vec![0u8; BLOCK];
        dev.read_block(0, &mut block).unwrap();
        let ident = Identification::decode(&block).unwrap();
        let explainer = Explainer::load(&mut dev).unwrap();
        let features = explainer.explain_features();
        assert_eq!(features.len(), 7, "every known feature is listed");
        let enabled: Vec<_> = features
            .iter()
            .filter(|feature| feature.enabled)
            .map(|feature| feature.id.unwrap())
            .collect();
        assert_eq!(enabled, expected);
        // Each listed bit is the bit of the identification block.
        for feature in &features {
            let word = match feature.class {
                FeatureClass::Compat => ident.features.compat,
                FeatureClass::RoCompat => ident.features.ro_compat,
                FeatureClass::Incompat => ident.features.incompat,
            };
            assert_eq!(feature.enabled, word & (1 << feature.bit) != 0);
        }
        let one = explainer
            .explain_feature("org.aros.afsplus:security-descriptors")
            .unwrap();
        assert_eq!((one.class, one.bit), (FeatureClass::Incompat, 3));
        assert!(explainer
            .explain_feature("org.aros.afsplus:nothing")
            .is_err());

        // A bit nobody assigned is listed without an identity.
        let mut unknown = ident.clone();
        unknown.features.compat |= 1 << 40;
        dev.write_block(0, &unknown.encode(BLOCK).unwrap()).unwrap();
        let features = Explainer::load(&mut dev).unwrap().explain_features();
        let stranger: Vec<_> = features.iter().filter(|f| f.id.is_none()).collect();
        assert_eq!(stranger.len(), 1);
        assert_eq!(
            (stranger[0].class, stranger[0].bit, stranger[0].enabled),
            (FeatureClass::Compat, 40, true)
        );
    }
}
