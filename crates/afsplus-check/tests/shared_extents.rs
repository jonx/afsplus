//! Shared-extent qualification for ADR-061.
//!
//! The production format/core implementation lands independently. This test
//! target starts with the interval-sweep oracle that will compare every live
//! file map with the authoritative shared-reference tree. Keeping the oracle
//! in the integration test avoids making the checker validate itself with its
//! own implementation.

use std::collections::BTreeMap;

use afsplus_block::{for_each_crash_state, MemoryBackend, RecordedOp, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::Timespec;

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
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: timestamp(1),
        },
    )
    .unwrap();
    device
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
