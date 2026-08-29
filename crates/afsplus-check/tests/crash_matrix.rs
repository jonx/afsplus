//! Step 5 of the first-contributor plan and the success criterion of the
//! next-phase roadmap:
//!
//! ```text
//! format image -> mutate -> checkpoint -> kill power at every point
//!              -> remount -> verify exact allowed state
//! ```
//!
//! Power is cut after every recorded operation of the first transaction; at
//! each point every durable state the device model allows (unflushed writes
//! independently lost, applied, or torn) must remount to *exactly* the
//! pre-commit or post-commit state, pass the full checker, and honor the
//! allocation/object invariants.
//!
//! The transaction's write log is also checked structurally for the COW
//! discipline: only fresh blocks above the committed high-water mark plus
//! the alternate checkpoint slot are ever written, and the barrier ordering
//! metadata -> flush -> checkpoint -> flush is exact. (With the bootstrap
//! bump allocator storage is never reused, so stale-but-valid block content
//! cannot masquerade as current; the allocator prototype for architecture
//! blocker 3 must re-run this matrix once reuse exists.)

use afsplus_block::{crash_states, MemoryBackend, RecordedOp, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::{layout, mkfs, mount, MkfsParams};
use afsplus_format::object::ObjectType;
use afsplus_format::Timespec;

const BS: usize = 4096;

#[test]
fn every_crash_state_of_the_first_transaction_recovers_to_an_allowed_state() {
    let mut base = MemoryBackend::new(BS, 64);
    mkfs(
        &mut base,
        &MkfsParams {
            uuid: [42u8; 16],
            label: "CrashVol".into(),
            timestamp: Timespec { seconds: 1_780_000_000, nanoseconds: 0 },
        },
    )
    .unwrap();

    // Pre-state facts, needed to verify "exact allowed state" below.
    let pre_generation = 1u64;
    let pre_next_free = mount(base.clone()).unwrap().checkpoint().next_free_block;
    assert_eq!(pre_next_free, layout::METADATA_START + 3);

    // Record the first transaction.
    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    let file_id = vol
        .create_file_in_root("hello.txt", Timespec { seconds: 1_780_000_100, nanoseconds: 0 })
        .unwrap();
    let (_, log) = vol.into_device().into_parts();

    // --- Structural discipline of the commit sequence -------------------
    // Expected: 4 COW metadata writes, barrier, checkpoint write, barrier.
    let shape: Vec<&'static str> = log
        .iter()
        .map(|op| match op {
            RecordedOp::Write { .. } => "w",
            RecordedOp::Flush => "F",
        })
        .collect();
    assert_eq!(shape, ["w", "w", "w", "w", "F", "w", "F"], "commit sequence changed");
    for (i, op) in log.iter().enumerate() {
        if let RecordedOp::Write { lba, .. } = op {
            if i < 4 {
                assert!(
                    *lba >= pre_next_free,
                    "write {i} touches block {lba} inside the committed state"
                );
            } else {
                assert_eq!(*lba, layout::CKPT_SLOT_B, "checkpoint must go to the alternate slot");
            }
        }
    }

    // --- The matrix ------------------------------------------------------
    let mut total_states = 0u64;
    let mut pre_outcomes = 0u64;
    let mut post_outcomes = 0u64;

    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            total_states += 1;
            let context = &state.description;

            // The full checker must accept every crash state.
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: checker findings {:?}", report.errors);

            // Mount must succeed and land on exactly one allowed state.
            let vol = mount(image).unwrap_or_else(|e| panic!("{context}: mount failed: {e}"));
            match vol.generation() {
                g if g == pre_generation => {
                    pre_outcomes += 1;
                    assert!(
                        vol.list_root().is_empty(),
                        "{context}: pre-commit state must not show the new file"
                    );
                    assert_eq!(vol.checkpoint().next_free_block, pre_next_free, "{context}");
                }
                g if g == pre_generation + 1 => {
                    post_outcomes += 1;
                    assert_eq!(
                        vol.lookup_root("hello.txt"),
                        Some(file_id),
                        "{context}: post-commit state must show the new file"
                    );
                    let record = vol.stat(file_id).unwrap();
                    assert_eq!(record.object_type, ObjectType::File);
                    assert_eq!(record.link_count, 1);
                    assert_eq!(vol.checkpoint().next_free_block, pre_next_free + 4, "{context}");
                }
                g => panic!("{context}: recovered to disallowed generation {g}"),
            }
        }
    }

    // Sanity: the matrix must actually exercise both outcomes, and the
    // subset/tear enumeration must produce a meaningful number of states.
    assert!(pre_outcomes > 0, "matrix never produced a pre-commit recovery");
    assert!(post_outcomes > 0, "matrix never produced a post-commit recovery");
    assert!(total_states > 50, "matrix unexpectedly small: {total_states} states");
}

#[test]
fn crash_matrix_across_a_second_transaction() {
    // Same property for a transaction that starts from a non-trivial state
    // and commits back into slot A.
    let mut base = MemoryBackend::new(BS, 64);
    mkfs(
        &mut base,
        &MkfsParams {
            uuid: [42u8; 16],
            label: "CrashVol2".into(),
            timestamp: Timespec { seconds: 1_780_000_000, nanoseconds: 0 },
        },
    )
    .unwrap();
    let mut vol = mount(base).unwrap();
    let first_id = vol
        .create_file_in_root("first.txt", Timespec { seconds: 1, nanoseconds: 0 })
        .unwrap();
    let base = vol.into_device();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    let second_id = vol
        .create_file_in_root("second.txt", Timespec { seconds: 2, nanoseconds: 0 })
        .unwrap();
    let (_, log) = vol.into_device().into_parts();

    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            let context = &state.description;
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: checker findings {:?}", report.errors);
            let vol = mount(image).unwrap_or_else(|e| panic!("{context}: mount failed: {e}"));
            match vol.generation() {
                2 => {
                    assert_eq!(vol.lookup_root("first.txt"), Some(first_id), "{context}");
                    assert_eq!(vol.lookup_root("second.txt"), None, "{context}");
                }
                3 => {
                    assert_eq!(vol.lookup_root("first.txt"), Some(first_id), "{context}");
                    assert_eq!(vol.lookup_root("second.txt"), Some(second_id), "{context}");
                }
                g => panic!("{context}: recovered to disallowed generation {g}"),
            }
        }
    }
}
