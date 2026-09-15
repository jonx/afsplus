//! Finite shared-reference properties. The oracle is a bounded array of live
//! mappings per physical block, independent of the production interval editor.
use std::collections::BTreeMap;

use afsplus_core::shared_extents::{decode_run, validate_canonical, RefEdit, SharedRun};
use afsplus_core::CoreError;
use afsplus_format::geometry::Geometry;

const BASE: u64 = 16;
const WIDTH: usize = 24;

fn records(counts: &[u32]) -> Vec<SharedRun> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < counts.len() {
        let first = index;
        let count = counts[index];
        index += 1;
        while index < counts.len() && counts[index] == count {
            index += 1;
        }
        if count >= 2 {
            result.push(SharedRun {
                physical_start: BASE + first as u64,
                block_count: (index - first) as u64,
                reference_count: count,
                flags: 0,
            });
        }
    }
    result
}

fn wire(run: SharedRun) -> ([u8; 8], [u8; 16]) {
    let mut value = [0; 16];
    value[..8].copy_from_slice(&run.block_count.to_le_bytes());
    value[8..12].copy_from_slice(&run.reference_count.to_le_bytes());
    value[12..].copy_from_slice(&run.flags.to_le_bytes());
    (run.physical_start.to_be_bytes(), value)
}

fn assert_view(edit: &RefEdit, counts: &[u32], start: usize, end: usize) {
    let parts = edit
        .resolve(BASE + start as u64, (end - start) as u64)
        .unwrap();
    let mut at = start;
    for part in parts {
        assert_eq!(part.physical_start, BASE + at as u64);
        assert!(part.block_count > 0);
        for _ in 0..part.block_count {
            assert!(at < end);
            assert_eq!(
                part.reference_count,
                (counts[at] >= 2).then_some(counts[at])
            );
            at += 1;
        }
    }
    assert_eq!(at, end);
}

fn assert_publication(edit: RefEdit, original: &[SharedRun], counts: &[u32]) -> Vec<SharedRun> {
    let publication = edit.finish().unwrap();
    let expected = records(counts);
    assert_eq!(publication.records, expected);
    // Apply the actual publication to independently serialized original state.
    // This catches correct final records accompanied by wrong deletes/upserts.
    let mut persisted: BTreeMap<_, _> = original.iter().copied().map(wire).collect();
    for key in publication.deletes {
        assert!(persisted.remove(&key).is_some());
    }
    for (key, value) in publication.upserts {
        persisted.insert(key, value);
    }
    assert_eq!(persisted, expected.iter().copied().map(wire).collect());
    expected
}

#[test]
fn exhaustive_small_mapping_states_and_every_edit_interval() {
    // 3^5 states x 15 intervals x acquire/release = 7,290 transactions.
    // Release can reach zero: only pre-release count one may be retired.
    for encoded in 0..243 {
        let mut n = encoded;
        let mut before = [1u32; 5];
        for count in &mut before {
            *count += n % 3;
            n /= 3;
        }
        let original = records(&before);
        for start in 0..5 {
            for end in start + 1..=5 {
                for acquire in [false, true] {
                    let mut edit = RefEdit::new();
                    edit.note_fetched(BASE, BASE + 5, original.clone());
                    let mut counts = before;
                    if acquire {
                        edit.acquire(BASE + start as u64, (end - start) as u64)
                            .unwrap();
                        for count in &mut counts[start..end] {
                            *count += 1;
                        }
                    } else {
                        let retired = edit
                            .release(BASE + start as u64, (end - start) as u64)
                            .unwrap();
                        let mut actual_retired = Vec::new();
                        for (lba, blocks) in retired {
                            assert!(blocks > 0);
                            actual_retired.extend(lba..lba + blocks);
                        }
                        let wanted: Vec<_> = (start..end)
                            .filter(|&i| before[i] == 1)
                            .map(|i| BASE + i as u64)
                            .collect();
                        assert_eq!(actual_retired, wanted);
                        for count in &mut counts[start..end] {
                            *count -= 1;
                        }
                    }
                    assert_view(&edit, &counts, 0, 5);
                    assert_publication(edit, &original, &counts);
                }
            }
        }
    }
}

#[test]
fn seeded_interacting_edits_preserve_counts_and_publication_deltas() {
    for seed in 1u64..=16 {
        let mut random = seed;
        let mut counts = [1u32; WIDTH];
        let mut original = Vec::new();
        let mut edit = RefEdit::new();
        edit.mark_all_fetched();
        for step in 0..256 {
            // Version-independent fixed integer generator, with bounded state.
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let start = random as usize % WIDTH;
            let end = start + 1 + (random >> 16) as usize % (WIDTH - start);
            let acquire = random & 0x100 != 0 || counts[start..end].contains(&1);
            if acquire {
                edit.acquire(BASE + start as u64, (end - start) as u64)
                    .unwrap();
                for count in &mut counts[start..end] {
                    *count += 1;
                }
            } else {
                assert!(edit
                    .release(BASE + start as u64, (end - start) as u64)
                    .unwrap()
                    .is_empty());
                for count in &mut counts[start..end] {
                    *count -= 1;
                }
            }
            assert_view(&edit, &counts, 0, WIDTH);
            assert_view(&edit, &counts, start, end);
            if step % 8 == 7 {
                original = assert_publication(edit, &original, &counts);
                edit = RefEdit::new();
                // Adjacent fetched windows merge; overlapping re-fetches must
                // retain edits rather than restore committed counts.
                edit.note_fetched(BASE, BASE + 12, original.clone());
                edit.note_fetched(BASE + 12, BASE + WIDTH as u64, Vec::new());
            } else {
                edit.note_fetched(BASE, BASE + WIDTH as u64, original.clone());
            }
        }
        assert_publication(edit, &original, &counts);
    }
}

#[test]
fn wire_admission_uses_literal_geometry_and_numeric_boundaries() {
    let geo = Geometry {
        block_size: 4096,
        total_blocks: 150,
        region_size: 64,
    };
    // Independently enumerated allocatable spans: 9..64, 70..128, 134..150.
    for start in [
        0,
        8,
        9,
        10,
        63,
        64,
        69,
        70,
        127,
        128,
        133,
        134,
        149,
        150,
        u64::MAX,
    ] {
        for blocks in [0, 1, 2, 16, 55, 56, 64, u64::MAX] {
            for references in [0, 1, 2, 3, u32::MAX] {
                for flags in [0, 1, u32::MAX] {
                    let run = SharedRun {
                        physical_start: start,
                        block_count: blocks,
                        reference_count: references,
                        flags,
                    };
                    let (key, value) = wire(run);
                    let expected = blocks > 0
                        && references >= 2
                        && flags == 0
                        && start.checked_add(blocks).is_some_and(|end| {
                            [(9, 64), (70, 128), (134, 150)]
                                .iter()
                                .any(|&(low, high)| low <= start && end <= high)
                        });
                    let result = decode_run(&key, &value, &geo);
                    assert_eq!(result.is_ok(), expected, "{run:?}");
                    if expected {
                        assert_eq!(result.unwrap(), run);
                    } else {
                        assert!(matches!(result, Err(CoreError::Corrupt(_))));
                    }
                }
            }
        }
    }
    let run = SharedRun {
        physical_start: BASE,
        block_count: 3,
        reference_count: 2,
        flags: 0,
    };
    let (key, value) = wire(run);
    for length in 0..=24 {
        if length != 8 {
            assert!(decode_run(&vec![0; length], &value, &geo).is_err());
        }
        if length != 16 {
            assert!(decode_run(&key, &vec![0; length], &geo).is_err());
        }
    }
}

#[test]
fn edit_boundaries_fail_closed_and_two_to_one_never_retires() {
    let mut edit = RefEdit::new();
    assert!(edit.acquire(BASE, 1).is_err());
    assert!(edit.release(BASE, 1).is_err());
    edit.note_fetched(BASE, BASE + 4, Vec::new());
    for (start, count) in [(BASE, 0), (BASE - 1, 1), (BASE + 3, 2), (u64::MAX, 2)] {
        assert!(edit.acquire(start, count).is_err());
        assert!(edit.release(start, count).is_err());
        assert!(edit.resolve(start, count).is_err());
    }
    assert_publication(edit, &[], &[1; 4]);

    let maximal = SharedRun {
        physical_start: BASE,
        block_count: 4,
        reference_count: u32::MAX,
        flags: 0,
    };
    let mut edit = RefEdit::new();
    edit.note_fetched(BASE, BASE + 4, vec![maximal]);
    assert!(matches!(
        edit.acquire(BASE + 1, 2),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert_view(&edit, &[u32::MAX; 4], 0, 4);
    assert_publication(edit, &[maximal], &[u32::MAX; 4]);

    let mut edit = RefEdit::new();
    edit.mark_all_fetched();
    edit.acquire(BASE, 4).unwrap();
    assert!(edit.release(BASE, 4).unwrap().is_empty());
    assert_eq!(edit.release(BASE, 4).unwrap(), vec![(BASE, 4)]);
    assert_publication(edit, &[], &[0; 4]);
    // A further release of zero live mappings is a caller error: RefEdit
    // represents absence as private, not as allocated versus already freed.
    // Wire counts 0/1 are rejected above; do not invent an editor guarantee.
}

#[test]
fn canonical_order_rejects_overlap_and_unmerged_equal_neighbors() {
    let run = |start, blocks, references| SharedRun {
        physical_start: start,
        block_count: blocks,
        reference_count: references,
        flags: 0,
    };
    for bad in [
        vec![run(BASE, 3, 2), run(BASE + 2, 2, 3)],
        vec![run(BASE + 2, 1, 2), run(BASE, 1, 3)],
        vec![run(BASE, 2, 2), run(BASE + 2, 2, 2)],
        vec![run(u64::MAX, 2, 2), run(0, 1, 3)],
    ] {
        assert!(validate_canonical(&bad).is_err());
    }
    assert!(validate_canonical(&[run(BASE, 2, 2), run(BASE + 2, 2, 3)]).is_ok());
    assert!(validate_canonical(&[run(BASE, 2, 2), run(BASE + 3, 2, 2)]).is_ok());
}
