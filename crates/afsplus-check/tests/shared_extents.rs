//! Shared-extent qualification for ADR-061.
//!
//! The production format/core implementation lands independently. This test
//! target starts with the interval-sweep oracle that will compare every live
//! file map with the authoritative shared-reference tree. Keeping the oracle
//! in the integration test avoids making the checker validate itself with its
//! own implementation.

use std::collections::BTreeMap;

use afsplus_block::{
    for_each_crash_state, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend,
};
use afsplus_check::check_device;
use afsplus_core::extent_map::EXTENT_SHARED;
use afsplus_core::shared_extents::{self, LoadedSharedExtents, SharedRun};
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::geometry::Geometry;
use afsplus_format::ident::{Identification, RO_COMPAT_SHARED_EXTENTS};
use afsplus_format::tree::{child_value, key_u64, ChildRef, TreeItem, TreeKind, TreeNode};
use afsplus_format::{le, Timespec, OBJECT_ROOT};

const BS: usize = 4096;

#[derive(Debug, Clone, Copy)]
struct CountedMapping {
    start: u64,
    blocks: u64,
    copies: u64,
}

impl CountedMapping {
    fn one(start: u64, blocks: u64) -> Self {
        Self {
            start,
            blocks,
            copies: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExpectedSharedRun {
    start: u64,
    blocks: u64,
    references: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Boundary {
    starts: u64,
    ends: u64,
}

/// Builds the canonical shared-reference records implied by all live maps.
///
/// `copies` is normally one. A weight makes the u32 wire-limit case testable
/// without allocating billions of synthetic mappings. The sweep stores two
/// events per input run, so memory is proportional to extent boundaries and
/// independent of the numerical volume size.
fn expected_shared_runs(
    mappings: &[CountedMapping],
) -> Result<Vec<ExpectedSharedRun>, &'static str> {
    let mut boundaries = BTreeMap::<u64, Boundary>::new();
    for mapping in mappings {
        if mapping.blocks == 0 || mapping.copies == 0 {
            return Err("mapping length and copy count must be non-zero");
        }
        let end = mapping
            .start
            .checked_add(mapping.blocks)
            .ok_or("mapping end overflows")?;
        let start_boundary = boundaries.entry(mapping.start).or_default();
        start_boundary.starts = start_boundary
            .starts
            .checked_add(mapping.copies)
            .ok_or("reference count overflows")?;
        let end_boundary = boundaries.entry(end).or_default();
        end_boundary.ends = end_boundary
            .ends
            .checked_add(mapping.copies)
            .ok_or("reference count overflows")?;
    }

    let mut expected = Vec::<ExpectedSharedRun>::new();
    let mut previous = None;
    let mut active = 0u64;
    for (at, boundary) in boundaries {
        if let Some(start) = previous {
            if start < at && active >= 2 {
                let references =
                    u32::try_from(active).map_err(|_| "reference count exceeds the wire limit")?;
                let blocks = at - start;
                if let Some(last) = expected.last_mut() {
                    let last_end = last
                        .start
                        .checked_add(last.blocks)
                        .ok_or("canonical run end overflows")?;
                    if last_end == start && last.references == references {
                        last.blocks = last
                            .blocks
                            .checked_add(blocks)
                            .ok_or("canonical run length overflows")?;
                    } else {
                        expected.push(ExpectedSharedRun {
                            start,
                            blocks,
                            references,
                        });
                    }
                } else {
                    expected.push(ExpectedSharedRun {
                        start,
                        blocks,
                        references,
                    });
                }
            }
        }
        active = active
            .checked_sub(boundary.ends)
            .ok_or("unbalanced interval endings")?;
        active = active
            .checked_add(boundary.starts)
            .ok_or("reference count overflows")?;
        previous = Some(at);
    }
    if active != 0 {
        return Err("unbalanced interval starts");
    }
    Ok(expected)
}

/// Validates an independently observed reference-tree sequence before it is
/// compared with the records derived from live file maps.
fn validate_observed_runs(records: &[ExpectedSharedRun]) -> Result<(), String> {
    for record in records {
        if record.blocks == 0 {
            return Err(format!("zero-length record at {}", record.start));
        }
        if record.references < 2 {
            return Err(format!(
                "record at {} has only {} reference(s)",
                record.start, record.references
            ));
        }
        record
            .start
            .checked_add(record.blocks)
            .ok_or_else(|| format!("record at {} overflows", record.start))?;
    }
    for pair in records.windows(2) {
        let left_end = pair[0]
            .start
            .checked_add(pair[0].blocks)
            .ok_or_else(|| format!("record at {} overflows", pair[0].start))?;
        if left_end > pair[1].start {
            return Err(format!(
                "records at {} and {} overlap or are out of order",
                pair[0].start, pair[1].start
            ));
        }
        if left_end == pair[1].start && pair[0].references == pair[1].references {
            return Err(format!(
                "adjacent count-{} records at {} and {} are not maximal",
                pair[0].references, pair[0].start, pair[1].start
            ));
        }
    }
    Ok(())
}

fn compare_reference_state(
    mappings: &[CountedMapping],
    observed: &[ExpectedSharedRun],
) -> Result<(), String> {
    validate_observed_runs(observed)?;
    let expected = expected_shared_runs(mappings).map_err(str::to_owned)?;
    if observed != expected {
        return Err(format!(
            "reference tree mismatch: expected {expected:?}, observed {observed:?}"
        ));
    }
    Ok(())
}

/// Deliberately simple block-by-block model used only to validate the sparse
/// interval sweep on a tiny address space. Keeping the two algorithms unlike
/// one another makes a shared boundary bug much less likely.
fn naive_shared_runs(
    mappings: &[CountedMapping],
    block_limit: usize,
) -> Result<Vec<ExpectedSharedRun>, &'static str> {
    let mut counts = vec![0u64; block_limit];
    for mapping in mappings {
        if mapping.blocks == 0 || mapping.copies == 0 {
            return Err("mapping length and copy count must be non-zero");
        }
        let end = mapping
            .start
            .checked_add(mapping.blocks)
            .ok_or("mapping end overflows")?;
        if end > block_limit as u64 {
            return Err("mapping exceeds naive model");
        }
        for count in &mut counts[mapping.start as usize..end as usize] {
            *count = count
                .checked_add(mapping.copies)
                .ok_or("reference count overflows")?;
        }
    }

    let mut runs = Vec::new();
    let mut cursor = 0usize;
    while cursor < counts.len() {
        let count = counts[cursor];
        if count < 2 {
            cursor += 1;
            continue;
        }
        let references = u32::try_from(count).map_err(|_| "reference count exceeds wire limit")?;
        let start = cursor;
        cursor += 1;
        while cursor < counts.len() && counts[cursor] == count {
            cursor += 1;
        }
        runs.push(ExpectedSharedRun {
            start: start as u64,
            blocks: (cursor - start) as u64,
            references,
        });
    }
    Ok(runs)
}

fn naive_overlap_partition(
    start: u64,
    blocks: u64,
    records: &[SharedRun],
) -> Vec<shared_extents::SubRun> {
    let end = start + blocks;
    let count_at = |block: u64| {
        records
            .iter()
            .find(|record| {
                record.physical_start <= block && block < record.physical_start + record.block_count
            })
            .map(|record| record.reference_count)
    };
    let mut segments = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let reference_count = count_at(cursor);
        let segment_start = cursor;
        cursor += 1;
        while cursor < end && count_at(cursor) == reference_count {
            cursor += 1;
        }
        segments.push(shared_extents::SubRun {
            physical_start: segment_start,
            block_count: cursor - segment_start,
            reference_count,
        });
    }
    segments
}

/// Runs the common ADR-061 recovery oracle over every state in the power-cut
/// model. Operation-specific checks receive whether the old or new checkpoint
/// won; the helper independently requires a clean full checker result.
fn run_powercut_matrix(
    base: &MemoryBackend,
    operations: &[RecordedOp],
    pre_generation: u64,
    mut verify: impl FnMut(&str, bool, &mut Volume<MemoryBackend>),
) {
    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;
    for crash_point in 0..=operations.len() {
        for_each_crash_state(base, operations, crash_point, |state| {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(
                report.is_clean(),
                "{context}: checker findings {:?}",
                report.errors
            );
            let mut volume =
                mount(image).unwrap_or_else(|error| panic!("{context}: mount failed: {error}"));
            let post = match volume.generation() {
                generation if generation == pre_generation => {
                    pre_outcomes += 1;
                    false
                }
                generation if generation == pre_generation + 1 => {
                    post_outcomes += 1;
                    true
                }
                generation => panic!(
                    "{context}: generation {generation} is neither pre {pre_generation} nor post {}",
                    pre_generation + 1
                ),
            };
            verify(&context, post, &mut volume);
        });
    }
    assert!(
        pre_outcomes > 0,
        "power-cut matrix produced no pre-transaction state"
    );
    assert!(
        post_outcomes > 0,
        "power-cut matrix produced no post-transaction state"
    );
}

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(label: &str) -> MemoryBackend {
    let mut device = MemoryBackend::new(BS, 128);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x61; 16],
            label: label.into(),
            region_size: 128,
            reclaim_caps: Default::default(),
            log_slots: 0,
            // Deliberately without the feature: the congruence tests plant
            // roots and flags on top and arm the bit themselves.
            shared_extents: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: timestamp(1),
        },
    )
    .unwrap();
    device
}

fn formatted_shared(label: &str) -> MemoryBackend {
    let mut device = MemoryBackend::new(BS, 256);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x62; 16],
            label: label.into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: timestamp(1),
        },
    )
    .unwrap();
    device
}

fn cloned_pair(label: &str) -> (Volume<MemoryBackend>, u64, u64) {
    let mut volume = mount(formatted_shared(label)).unwrap();
    let source = volume
        .create_file_in_root("source", &vec![0x5au8; BS], timestamp(2))
        .unwrap();
    let clone = volume
        .clone_file(source, OBJECT_ROOT, "clone", timestamp(3))
        .unwrap();
    (volume, source, clone)
}

fn rewrite_shared_leaf(
    volume: &mut Volume<MemoryBackend>,
    rewrite: impl FnOnce(&mut Vec<TreeItem>),
) {
    let root = volume.checkpoint().shared_extent_root_block;
    assert_ne!(root, 0);
    let generation = volume.generation();
    let mut block = vec![0u8; BS];
    volume.device_mut().read_block(root, &mut block).unwrap();
    let (mut node, _) = TreeNode::decode(&block).unwrap();
    assert!(
        node.is_leaf(),
        "small corruption fixtures require a leaf root"
    );
    rewrite(&mut node.items);
    node.subtree_items = node.items.len() as u64;
    volume
        .device_mut()
        .write_block(root, &node.encode(BS, generation).unwrap())
        .unwrap();
}

fn rewrite_first_extent_flags(
    volume: &mut Volume<MemoryBackend>,
    object_id: u64,
    rewrite: impl FnOnce(u32) -> u32,
) {
    let record = volume.stat(object_id).unwrap().unwrap();
    assert_ne!(
        record.flags & afsplus_format::object::OBJECT_FLAG_EXTENT_TREE,
        0
    );
    let generation = volume.generation();
    let mut block = vec![0u8; BS];
    volume
        .device_mut()
        .read_block(record.data_root, &mut block)
        .unwrap();
    let (mut node, _) = TreeNode::decode(&block).unwrap();
    let item = node.items.first_mut().expect("file has one mapped extent");
    let flags = le::get_u32(&item.value[16..20]);
    le::put_u32(&mut item.value[16..20], rewrite(flags));
    volume
        .device_mut()
        .write_block(record.data_root, &node.encode(BS, generation).unwrap())
        .unwrap();
}

fn require_checker_error(volume: Volume<MemoryBackend>, expected: &str) {
    let mut device = volume.into_device();
    let report = check_device(&mut device);
    assert!(!report.is_clean(), "forged image was accepted");
    assert!(
        report.errors.iter().any(|error| error.contains(expected)),
        "expected an error containing {expected:?}, got {:?}",
        report.errors
    );
}

fn raw_shared_item(start: u64, blocks: u64, references: u32, flags: u32) -> TreeItem {
    let mut value = vec![0u8; shared_extents::VALUE_SIZE];
    le::put_u64(&mut value[0..8], blocks);
    le::put_u32(&mut value[8..12], references);
    le::put_u32(&mut value[12..16], flags);
    TreeItem {
        key: key_u64(start).to_vec(),
        value,
    }
}

/// Builds a real checksummed AFST leaf. Invalid cases therefore exercise the
/// shared adapter's semantic validation rather than being rejected by a bad
/// containing-block checksum first.
fn load_raw_shared(items: Vec<TreeItem>) -> Result<LoadedSharedExtents, afsplus_core::CoreError> {
    const ROOT_LBA: u64 = 20;
    let geometry = test_geometry();
    let mut node = TreeNode::leaf(TreeKind::SharedExtents, 0);
    node.subtree_items = items.len() as u64;
    node.items = items;
    let mut device = MemoryBackend::new(BS, geometry.total_blocks);
    device.write_block(ROOT_LBA, &node.encode(BS, 1).unwrap())?;
    shared_extents::load_all(&mut device, &geometry, ROOT_LBA, 1)
}

fn test_geometry() -> Geometry {
    Geometry {
        block_size: BS,
        total_blocks: 128,
        region_size: 128,
    }
}

#[test]
fn oracle_omits_private_maps_and_counts_identical_maps() {
    assert_eq!(expected_shared_runs(&[]).unwrap(), []);
    assert_eq!(
        expected_shared_runs(&[CountedMapping::one(50, 8)]).unwrap(),
        []
    );
    assert_eq!(
        expected_shared_runs(&[CountedMapping::one(50, 8), CountedMapping::one(50, 8),]).unwrap(),
        [ExpectedSharedRun {
            start: 50,
            blocks: 8,
            references: 2,
        }]
    );
}

#[test]
fn oracle_partitions_partial_and_three_way_overlaps() {
    assert_eq!(
        expected_shared_runs(&[
            CountedMapping::one(10, 20),
            CountedMapping::one(0, 40),
            CountedMapping::one(20, 20),
        ])
        .unwrap(),
        [
            ExpectedSharedRun {
                start: 10,
                blocks: 10,
                references: 2,
            },
            ExpectedSharedRun {
                start: 20,
                blocks: 10,
                references: 3,
            },
            ExpectedSharedRun {
                start: 30,
                blocks: 10,
                references: 2,
            },
        ]
    );

    assert_eq!(
        expected_shared_runs(&[
            CountedMapping::one(0, 100),
            CountedMapping::one(0, 40),
            CountedMapping::one(60, 40),
        ])
        .unwrap(),
        [
            ExpectedSharedRun {
                start: 0,
                blocks: 40,
                references: 2,
            },
            ExpectedSharedRun {
                start: 60,
                blocks: 40,
                references: 2,
            },
        ]
    );
}

#[test]
fn oracle_merges_adjacent_equal_counts_with_different_peers() {
    assert_eq!(
        expected_shared_runs(&[
            CountedMapping::one(1_000, 20),
            CountedMapping::one(1_000, 10),
            CountedMapping::one(1_010, 10),
        ])
        .unwrap(),
        [ExpectedSharedRun {
            start: 1_000,
            blocks: 20,
            references: 2,
        }]
    );
}

#[test]
fn oracle_is_sparse_in_address_space_and_rejects_hostile_ranges() {
    let high = u64::MAX - 1_000;
    assert_eq!(
        expected_shared_runs(&[
            CountedMapping::one(high, 100),
            CountedMapping::one(high + 50, 25),
        ])
        .unwrap(),
        [ExpectedSharedRun {
            start: high + 50,
            blocks: 25,
            references: 2,
        }]
    );

    assert!(expected_shared_runs(&[CountedMapping::one(1, 0)]).is_err());
    assert!(expected_shared_runs(&[CountedMapping::one(u64::MAX, 2)]).is_err());
    assert!(expected_shared_runs(&[CountedMapping {
        start: 7,
        blocks: 1,
        copies: u32::MAX as u64 + 1,
    }])
    .is_err());
}

#[test]
fn oracle_rejects_missing_extra_wrong_and_noncanonical_records() {
    let mappings = [CountedMapping {
        start: 100,
        blocks: 20,
        copies: 2,
    }];
    let valid = [ExpectedSharedRun {
        start: 100,
        blocks: 20,
        references: 2,
    }];
    compare_reference_state(&mappings, &valid).unwrap();

    assert!(compare_reference_state(&mappings, &[]).is_err());
    assert!(compare_reference_state(&[CountedMapping::one(100, 20)], &valid).is_err());
    assert!(compare_reference_state(
        &mappings,
        &[ExpectedSharedRun {
            references: 3,
            ..valid[0]
        }]
    )
    .is_err());
    assert!(validate_observed_runs(&[
        ExpectedSharedRun {
            start: 100,
            blocks: 15,
            references: 2,
        },
        ExpectedSharedRun {
            start: 110,
            blocks: 10,
            references: 3,
        },
    ])
    .is_err());
    assert!(validate_observed_runs(&[
        ExpectedSharedRun {
            start: 100,
            blocks: 10,
            references: 2,
        },
        ExpectedSharedRun {
            start: 110,
            blocks: 10,
            references: 2,
        },
    ])
    .is_err());
    assert!(validate_observed_runs(&[ExpectedSharedRun {
        start: u64::MAX,
        blocks: 2,
        references: 2,
    }])
    .is_err());
}

#[test]
fn sparse_oracle_matches_an_independent_naive_model() {
    const BLOCKS: u64 = 32;
    let mut state = 0x0615_5eed_u64;
    for case in 0..4_096 {
        let mapping_count = (state as usize % 8) + 1;
        let mut mappings = Vec::with_capacity(mapping_count);
        for _ in 0..mapping_count {
            // Fixed LCG: deterministic, dependency-free, and adequate for
            // exercising coincident starts/ends, gaps and nested overlaps.
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let start = state % BLOCKS;
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let blocks = 1 + state % (BLOCKS - start);
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            mappings.push(CountedMapping {
                start,
                blocks,
                copies: 1 + state % 3,
            });
        }

        assert_eq!(
            expected_shared_runs(&mappings),
            naive_shared_runs(&mappings, BLOCKS as usize),
            "oracle disagreement in generated case {case}: {mappings:?}"
        );
    }
}

#[test]
fn checksummed_shared_tree_rejects_bad_counts_bounds_and_flags() {
    let valid = load_raw_shared(vec![raw_shared_item(40, 8, 2, 0)]).unwrap();
    assert_eq!(
        valid.records,
        [SharedRun {
            physical_start: 40,
            block_count: 8,
            reference_count: 2,
            flags: 0,
        }]
    );

    for (label, item) in [
        ("zero length", raw_shared_item(40, 0, 2, 0)),
        ("count below two", raw_shared_item(40, 8, 1, 0)),
        ("reserved flags", raw_shared_item(40, 8, 2, 1)),
        ("reserved address", raw_shared_item(1, 2, 2, 0)),
        ("outside volume", raw_shared_item(127, 2, 2, 0)),
    ] {
        assert!(
            load_raw_shared(vec![item]).is_err(),
            "checksummed {label} record was accepted"
        );
    }
}

#[test]
fn checksummed_shared_tree_rejects_overlap_and_nonmaximal_runs() {
    assert!(
        load_raw_shared(vec![
            raw_shared_item(40, 8, 2, 0),
            raw_shared_item(44, 8, 3, 0),
        ])
        .is_err(),
        "overlapping records were accepted"
    );
    assert!(
        load_raw_shared(vec![
            raw_shared_item(40, 4, 2, 0),
            raw_shared_item(44, 4, 2, 0),
        ])
        .is_err(),
        "adjacent equal-count records were accepted"
    );

    let distinct = load_raw_shared(vec![
        raw_shared_item(40, 4, 2, 0),
        raw_shared_item(44, 4, 3, 0),
    ])
    .unwrap();
    assert_eq!(distinct.records.len(), 2);
}

#[test]
fn shared_loader_rejects_wrong_identity_generation_and_unreadable_nodes() {
    const ROOT_LBA: u64 = 20;
    let geometry = test_geometry();

    for (label, kind, owner, generation) in [
        ("tree kind", TreeKind::ExtentMap, 0, 1),
        ("owner", TreeKind::SharedExtents, 99, 1),
        ("future generation", TreeKind::SharedExtents, 0, 2),
    ] {
        let mut node = TreeNode::leaf(kind, owner);
        node.items.push(raw_shared_item(40, 4, 2, 0));
        node.subtree_items = 1;
        let mut device = MemoryBackend::new(BS, geometry.total_blocks);
        device
            .write_block(ROOT_LBA, &node.encode(BS, generation).unwrap())
            .unwrap();
        assert!(
            shared_extents::load_all(&mut device, &geometry, ROOT_LBA, 1).is_err(),
            "wrong {label} was accepted"
        );
    }

    let mut blank_root = MemoryBackend::new(BS, geometry.total_blocks);
    assert!(
        shared_extents::load_all(&mut blank_root, &geometry, ROOT_LBA, 1).is_err(),
        "unreadable root was accepted"
    );

    // The root itself is sound and points to structurally plausible children;
    // the first child is deliberately blank. This distinguishes an unreadable
    // descendant from a bad root checksum.
    let internal = TreeNode {
        kind: TreeKind::SharedExtents,
        owner: 0,
        level: 1,
        subtree_items: 2,
        leftmost_child: 21,
        leftmost_items: 1,
        items: vec![TreeItem {
            key: key_u64(60).to_vec(),
            value: child_value(ChildRef {
                lba: 22,
                subtree_items: 1,
            })
            .unwrap(),
        }],
    };
    let mut right = TreeNode::leaf(TreeKind::SharedExtents, 0);
    right.items.push(raw_shared_item(60, 4, 2, 0));
    right.subtree_items = 1;
    let mut unreadable_child = MemoryBackend::new(BS, geometry.total_blocks);
    unreadable_child
        .write_block(ROOT_LBA, &internal.encode(BS, 1).unwrap())
        .unwrap();
    unreadable_child
        .write_block(22, &right.encode(BS, 1).unwrap())
        .unwrap();
    assert!(
        shared_extents::load_all(&mut unreadable_child, &geometry, ROOT_LBA, 1).is_err(),
        "unreadable descendant was accepted"
    );
}

#[test]
fn checker_enforces_shared_feature_root_congruence() {
    let mut root_without_feature = formatted("RootWithoutFeature");
    let mut checkpoint = Checkpoint::decode(&root_without_feature.peek(1), &[0x61; 16]).unwrap();
    checkpoint.shared_extent_root_block = 20;
    root_without_feature.apply_raw(1, &checkpoint.encode(BS).unwrap());
    let report = check_device(&mut root_without_feature);
    assert!(!report.is_clean());
    assert!(report.errors.iter().any(|error| {
        error.contains("shared-extent root present without the shared-extents feature")
    }));

    let mut enabled_but_unused = formatted("EnabledUnused");
    let mut identification = Identification::decode(&enabled_but_unused.peek(0)).unwrap();
    identification.features.ro_compat |= RO_COMPAT_SHARED_EXTENTS;
    enabled_but_unused.apply_raw(0, &identification.encode(BS).unwrap());
    let report = check_device(&mut enabled_but_unused);
    assert!(
        report.is_clean(),
        "enabled-but-unused volume rejected: {:?}",
        report.errors
    );
}

#[test]
fn checker_rejects_missing_extra_and_wrong_reference_records() {
    // C4: two flagged mappings remain live but their authoritative record is
    // removed from an otherwise checksummed tree.
    let (mut missing, _, _) = cloned_pair("MissingSharedRecord");
    rewrite_shared_leaf(&mut missing, Vec::clear);
    require_checker_error(missing, "reconstruct");

    // C6: the run and mappings agree, but the stored count is forged from two
    // references to three.
    let (mut wrong, _, _) = cloned_pair("WrongSharedCount");
    rewrite_shared_leaf(&mut wrong, |items| {
        assert_eq!(items.len(), 1);
        le::put_u32(&mut items[0].value[8..12], 3);
    });
    require_checker_error(wrong, "reconstruct");

    // C5: after rc=2 -> rc=1 the surviving mapping deliberately keeps its
    // conservative flag, but the reference tree must be empty. Reinsert an
    // orphan count-two record over that sole owner.
    let (mut extra, _, clone) = cloned_pair("ExtraSharedRecord");
    let root = extra.checkpoint().shared_extent_root_block;
    let generation = extra.generation();
    let geometry = extra.ident().geometry();
    let committed = shared_extents::load_all(extra.device_mut(), &geometry, root, generation)
        .unwrap()
        .records[0];
    extra
        .delete_file(OBJECT_ROOT, "source", timestamp(4))
        .unwrap();
    assert_eq!(extra.read_file(clone).unwrap(), vec![0x5au8; BS]);
    rewrite_shared_leaf(&mut extra, |items| {
        assert!(items.is_empty());
        items.push(raw_shared_item(
            committed.physical_start,
            committed.block_count,
            2,
            0,
        ));
    });
    require_checker_error(extra, "referenced twice");
}

#[test]
fn checker_rejects_unflagged_and_direct_shared_mappings() {
    // C7: clear the conservative marker on one of two live mappings while
    // leaving the reference record and all containing CRCs valid.
    let (mut unflagged, _, clone) = cloned_pair("UnflaggedSharedMapping");
    rewrite_first_extent_flags(&mut unflagged, clone, |flags| flags & !EXTENT_SHARED);
    require_checker_error(unflagged, "referenced twice");

    // C10: a direct-layout object cannot participate because its object
    // record has no extent flag word. Allocate the empty reference root with
    // an empty clone, then forge a record over a direct file's data.
    let mut direct = mount(formatted_shared("DirectSharedMapping")).unwrap();
    let data = direct
        .create_file_in_root("direct", &vec![0x6bu8; BS], timestamp(2))
        .unwrap();
    let direct_record = direct.stat(data).unwrap().unwrap();
    assert_eq!(
        direct_record.flags & afsplus_format::object::OBJECT_FLAG_EXTENT_TREE,
        0
    );
    let empty = direct
        .create_file_in_root("empty", b"", timestamp(3))
        .unwrap();
    direct
        .clone_file(empty, OBJECT_ROOT, "empty-clone", timestamp(4))
        .unwrap();
    rewrite_shared_leaf(&mut direct, |items| {
        assert!(items.is_empty());
        items.push(raw_shared_item(direct_record.data_root, 1, 2, 0));
    });
    require_checker_error(direct, "referenced twice");
}

#[test]
fn checker_rejects_a_shared_tree_block_claimed_as_data() {
    // C11: make the reference root describe its own block as shared data.
    // The block remains checksummed and allocatable, so the semantic ownership
    // collision — not a codec failure — must reject the image.
    let mut volume = mount(formatted_shared("SharedTreeDataCollision")).unwrap();
    let empty = volume
        .create_file_in_root("empty", b"", timestamp(2))
        .unwrap();
    volume
        .clone_file(empty, OBJECT_ROOT, "empty-clone", timestamp(3))
        .unwrap();
    let root = volume.checkpoint().shared_extent_root_block;
    rewrite_shared_leaf(&mut volume, |items| {
        assert!(items.is_empty());
        items.push(raw_shared_item(root, 1, 2, 0));
    });
    require_checker_error(volume, "referenced twice");
}

#[test]
fn overlap_resolver_matches_an_independent_per_block_model() {
    const BLOCKS: u64 = 32;
    let mut state = 0x0610_a11a_u64;
    for case in 0..4_096 {
        let mapping_count = (state as usize % 8) + 1;
        let mut mappings = Vec::with_capacity(mapping_count);
        for _ in 0..mapping_count {
            state = state
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493);
            let start = state % BLOCKS;
            state = state
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493);
            mappings.push(CountedMapping::one(start, 1 + state % (BLOCKS - start)));
        }
        let records: Vec<_> = expected_shared_runs(&mappings)
            .unwrap()
            .into_iter()
            .map(|record| SharedRun {
                physical_start: record.start,
                block_count: record.blocks,
                reference_count: record.references,
                flags: 0,
            })
            .collect();

        state = state
            .wrapping_mul(2_862_933_555_777_941_757)
            .wrapping_add(3_037_000_493);
        let query_start = state % BLOCKS;
        state = state
            .wrapping_mul(2_862_933_555_777_941_757)
            .wrapping_add(3_037_000_493);
        let query_blocks = 1 + state % (BLOCKS - query_start);
        assert_eq!(
            shared_extents::resolve_overlaps(query_start, query_blocks, &records).unwrap(),
            naive_overlap_partition(query_start, query_blocks, &records),
            "overlap disagreement in generated case {case}: query {query_start}+{query_blocks}, records {records:?}"
        );
    }
}

#[test]
fn powercut_harness_proves_both_states_and_checks_contents() {
    // This non-sharing transaction is a witness for the harness itself. The
    // ADR-061 cases use exactly the same path once CloneFile/CloneRange land.
    let base = formatted("MatrixWitness");
    let pre_generation = mount(base.clone()).unwrap().generation();
    let mut volume = mount(RecordingBackend::new(base.clone())).unwrap();
    let file_id = volume
        .create_file_in_root("witness", b"shared-matrix", timestamp(2))
        .unwrap();
    let (_, operations) = volume.into_device().into_parts();

    run_powercut_matrix(
        &base,
        &operations,
        pre_generation,
        |context, post, volume| {
            if post {
                assert_eq!(
                    volume.lookup_root("witness").unwrap(),
                    Some(file_id),
                    "{context}"
                );
                assert_eq!(
                    volume.read_file(file_id).unwrap(),
                    b"shared-matrix",
                    "{context}"
                );
            } else {
                assert_eq!(volume.lookup_root("witness").unwrap(), None, "{context}");
            }
        },
    );
}
