//! The mandatory allocator crash workload (step 6 / architecture blocker 3):
//!
//! ```text
//! G1: create A            -> data lands in block X
//! G2: delete A            -> X retired (quarantined, still allocated)
//! G3: X becomes reusable  -> create B reuses X
//! ```
//!
//! Power is cut after every write/flush of G2 and of G3. Allowed recovered
//! states, and nothing else:
//!
//! - generation of G1: A present with intact content, X owned by A
//! - generation of G2: A absent, X retired and NOT reused
//! - generation of G3: B present with intact content in X, A absent
//!
//! Forbidden (asserted through the state checks and the full checker):
//! A visible with B's reused content, two non-shared owners of one block,
//! any FREE block reachable from a selectable checkpoint.

use afsplus_block::{crash_states, MemoryBackend, RecordedOp, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::Timespec;

const BS: usize = 4096;
const PA: [u8; 4000] = [0xAAu8; 4000];
const PB: [u8; 4000] = [0xBBu8; 4000];

fn ts(seconds: i64) -> Timespec {
    Timespec { seconds, nanoseconds: 0 }
}

/// Formats a volume and runs G1 (create A). Returns (image, A's data block).
fn setup_g1() -> (MemoryBackend, u64) {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [42u8; 16],
            label: "AllocVol".into(),
            region_size: 64,
            timestamp: ts(0),
        },
    )
    .unwrap();
    let mut vol = mount(dev).unwrap();
    let a = vol.create_file_in_root("A", &PA, ts(1)).unwrap();
    let record = vol.stat(a).unwrap().unwrap();
    assert_eq!(record.data_blocks, 1);
    (vol.into_device(), record.data_root)
}

fn record_tx(
    base: &MemoryBackend,
    op: impl FnOnce(&mut Volume<RecordingBackend<MemoryBackend>>),
) -> Vec<RecordedOp> {
    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    op(&mut vol);
    vol.into_device().into_parts().1
}

fn checked_mount(context: &str, image: MemoryBackend) -> Volume<MemoryBackend> {
    let mut copy = image.clone();
    let report = check_device(&mut copy);
    assert!(report.is_clean(), "{context}: checker findings {:?}", report.errors);
    mount(image).unwrap_or_else(|e| panic!("{context}: mount failed: {e}"))
}

#[test]
fn quarantine_workload_g1_g2_g3_with_full_crash_matrix() {
    // --- G1: create A ----------------------------------------------------
    let (g1_image, x) = setup_g1();
    let g1_generation = 2;

    // --- G2: delete A, X goes into quarantine ---------------------------
    let g2_log = record_tx(&g1_image, |vol| {
        vol.delete_file_in_root("A", ts(2)).unwrap();
    });

    let g2_image = {
        let mut vol = mount(g1_image.clone()).unwrap();
        vol.delete_file_in_root("A", ts(2)).unwrap();
        assert!(vol.retired().contains(x), "X must be quarantined after the delete");
        assert_eq!(vol.lookup_root("A"), None);
        vol.into_device()
    };

    // --- G3: create B, which must reuse X --------------------------------
    let g3_log = record_tx(&g2_image, |vol| {
        let b = vol.create_file_in_root("B", &PB, ts(3)).unwrap();
        let record = vol.stat(b).unwrap().unwrap();
        assert_eq!(
            record.data_root, x,
            "test precondition: B must reuse the quarantined block X"
        );
    });

    // --- Crash matrix over G2 -------------------------------------------
    for crash_point in 0..=g2_log.len() {
        for state in crash_states(&g1_image, &g2_log, crash_point) {
            let context = format!("G2 {}", state.description);
            let mut vol = checked_mount(&context, state.image);
            match vol.generation() {
                g if g == g1_generation => {
                    // A present, content byte-for-byte intact: the delete
                    // transaction must never have touched X.
                    let a = vol.lookup_root("A").unwrap_or_else(|| panic!("{context}: A missing"));
                    assert_eq!(vol.stat(a).unwrap().unwrap().data_root, x, "{context}");
                    assert_eq!(vol.read_file(a).unwrap(), PA.to_vec(), "{context}: A content damaged");
                }
                g if g == g1_generation + 1 => {
                    assert_eq!(vol.lookup_root("A"), None, "{context}: A must be gone");
                    assert!(vol.retired().contains(x), "{context}: X must be retired, not reused");
                }
                g => panic!("{context}: recovered to disallowed generation {g}"),
            }
        }
    }

    // --- Crash matrix over G3 -------------------------------------------
    for crash_point in 0..=g3_log.len() {
        for state in crash_states(&g2_image, &g3_log, crash_point) {
            let context = format!("G3 {}", state.description);
            let mut vol = checked_mount(&context, state.image);
            match vol.generation() {
                g if g == g1_generation + 1 => {
                    // Pre-commit: nothing visible, X still quarantined —
                    // even though the crash state may already carry B's
                    // half-written bytes inside X, they are unreachable.
                    assert_eq!(vol.lookup_root("A"), None, "{context}");
                    assert_eq!(vol.lookup_root("B"), None, "{context}");
                    assert!(vol.retired().contains(x), "{context}: X left quarantine early");
                }
                g if g == g1_generation + 2 => {
                    assert_eq!(vol.lookup_root("A"), None, "{context}");
                    let b = vol.lookup_root("B").unwrap_or_else(|| panic!("{context}: B missing"));
                    let record = vol.stat(b).unwrap().unwrap();
                    assert_eq!(record.data_root, x, "{context}: B must own X");
                    assert_eq!(vol.read_file(b).unwrap(), PB.to_vec(), "{context}: B content damaged");
                    assert!(!vol.retired().contains(x), "{context}: X still retired after reuse");
                }
                g => panic!("{context}: recovered to disallowed generation {g}"),
            }
        }
    }
}

#[test]
fn reuse_needs_a_full_generation_of_quarantine() {
    // Directly after the delete commits, X is retired: the *same* committed
    // state may never hand it out, and the next transaction may.
    let (g1_image, x) = setup_g1();
    let mut vol = mount(g1_image).unwrap();
    vol.delete_file_in_root("A", ts(2)).unwrap();
    assert!(vol.retired().contains(x));

    // The very next transaction promotes and may reuse X.
    let b = vol.create_file_in_root("B", &PB, ts(3)).unwrap();
    assert_eq!(vol.stat(b).unwrap().unwrap().data_root, x);
    assert!(!vol.retired().contains(x));

    // Reclaim latency measured by the allocator: one generation, for every
    // promoted block.
    let stats = vol.last_commit_stats().unwrap();
    assert!(stats.alloc.blocks_promoted > 0);
    assert_eq!(
        stats.alloc.reclaim_latency_generations, stats.alloc.blocks_promoted,
        "every block must spend exactly one generation in quarantine"
    );

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}
