//! The mandatory allocator crash workload (step 6 / architecture blocker 3):
//!
//! ```text
//! G1: create A            -> data lands in block X
//! G2: delete A            -> X retired (quarantined, still allocated)
//! G3: maintenance          -> protect G1 until its slot is replaced
//! G4: X becomes reusable  -> B transaction reuses X for data or metadata
//! ```
//!
//! Power is cut after every write/flush of G2, G3 and G4. Allowed recovered
//! states, and nothing else:
//!
//! - generation of G1: A present with intact content, X owned by A
//! - generation of G2: A absent, X retired and NOT reused
//! - generation of G4: B content intact, X owned by the new state, A absent
//!
//! Forbidden (asserted through the state checks and the full checker):
//! A visible with B's reused content, two non-shared owners of one block,
//! any FREE block reachable from a selectable checkpoint.

use afsplus_block::{crash_states, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::Timespec;

const BS: usize = 4096;
const PA: [u8; 4000] = [0xAAu8; 4000];
const PB: [u8; 4000] = [0xBBu8; 4000];

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
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
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
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
    assert!(
        report.is_clean(),
        "{context}: checker findings {:?}",
        report.errors
    );
    assert!(
        report.warnings.is_empty(),
        "{context}: older checkpoint findings {:?}",
        report.warnings
    );
    mount(image).unwrap_or_else(|e| panic!("{context}: mount failed: {e}"))
}

fn assert_reused<D: BlockDevice>(volume: &mut Volume<D>, lba: u64) {
    let ident = volume.ident().clone();
    let cp = volume.checkpoint().clone();
    let state =
        afsplus_core::verify::load_committed_state(volume.device_mut(), &ident, &cp).unwrap();
    assert!(
        state.data_blocks.contains(&lba) || state.metadata_blocks.contains(&lba),
        "fixture must actually reuse block {lba} for data or metadata"
    );
    assert!(!volume.quarantine_contains(lba).unwrap());
}

#[test]
fn quarantine_workload_g1_g2_g3_g4_with_full_crash_matrix() {
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
        assert!(
            vol.quarantine_contains(x).unwrap(),
            "X must be quarantined after the delete"
        );
        assert_eq!(vol.lookup_root("A").unwrap(), None);
        vol.into_device()
    };

    // G3 advances the retention boundary without releasing X.
    let maintenance_log = record_tx(&g2_image, |vol| {
        vol.reclaim_step(ts(3)).unwrap();
    });
    let g3_image = {
        let mut vol = mount(g2_image.clone()).unwrap();
        vol.reclaim_step(ts(3)).unwrap();
        assert!(vol.quarantine_contains(x).unwrap());
        vol.into_device()
    };
    for cut in 0..=maintenance_log.len() {
        for state in crash_states(&g2_image, &maintenance_log, cut) {
            let mut vol = checked_mount("G3 maintenance", state.image);
            assert!(vol.quarantine_contains(x).unwrap());
            let mut raw = [0; BS];
            vol.device_mut().read_block(x, &mut raw).unwrap();
            assert_eq!(&raw[..PA.len()], &PA, "protected G3 bytes changed");
            assert!(vol.lookup_root("A").unwrap().is_none());
            assert!(vol.lookup_root("B").unwrap().is_none());
        }
    }
    let g4_log = record_tx(&g3_image, |vol| {
        vol.create_file_in_root("B", &PB, ts(4)).unwrap();
        assert_reused(vol, x);
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
                    let a = vol
                        .lookup_root("A")
                        .unwrap()
                        .unwrap_or_else(|| panic!("{context}: A missing"));
                    assert_eq!(vol.stat(a).unwrap().unwrap().data_root, x, "{context}");
                    assert_eq!(
                        vol.read_file(a).unwrap(),
                        PA.to_vec(),
                        "{context}: A content damaged"
                    );
                }
                g if g == g1_generation + 1 => {
                    assert_eq!(
                        vol.lookup_root("A").unwrap(),
                        None,
                        "{context}: A must be gone"
                    );
                    assert!(
                        vol.quarantine_contains(x).unwrap(),
                        "{context}: X must be retired, not reused"
                    );
                }
                g => panic!("{context}: recovered to disallowed generation {g}"),
            }
        }
    }

    // --- Crash matrix over G4 -------------------------------------------
    for crash_point in 0..=g4_log.len() {
        for state in crash_states(&g3_image, &g4_log, crash_point) {
            let context = format!("G4 {}", state.description);
            let mut vol = checked_mount(&context, state.image);
            match vol.generation() {
                g if g == g1_generation + 2 => {
                    // Pre-commit: nothing visible, X still quarantined —
                    // even though the crash state may already carry B's
                    // half-written bytes inside X, they are unreachable.
                    assert_eq!(vol.lookup_root("A").unwrap(), None, "{context}");
                    assert_eq!(vol.lookup_root("B").unwrap(), None, "{context}");
                    assert!(
                        vol.quarantine_contains(x).unwrap(),
                        "{context}: X left quarantine early"
                    );
                }
                g if g == g1_generation + 3 => {
                    assert_eq!(vol.lookup_root("A").unwrap(), None, "{context}");
                    let b = vol
                        .lookup_root("B")
                        .unwrap()
                        .unwrap_or_else(|| panic!("{context}: B missing"));
                    assert_reused(&mut vol, x);
                    assert_eq!(
                        vol.read_file(b).unwrap(),
                        PB.to_vec(),
                        "{context}: B content damaged"
                    );
                    assert!(
                        !vol.quarantine_contains(x).unwrap(),
                        "{context}: X still retired after reuse"
                    );
                }
                g => panic!("{context}: recovered to disallowed generation {g}"),
            }
        }
    }
}

#[test]
fn reuse_waits_until_the_previous_checkpoint_cannot_reference_it() {
    // Directly after the delete commits, X is retired: the *same* committed
    // state may never hand it out; the previous checkpoint also retains it.
    let (g1_image, x) = setup_g1();
    let mut vol = mount(g1_image).unwrap();
    vol.delete_file_in_root("A", ts(2)).unwrap();
    assert!(vol.quarantine_contains(x).unwrap());

    vol.reclaim_step(ts(3)).unwrap();
    assert!(vol.quarantine_contains(x).unwrap());
    // After slot replacement the next transaction can promote X.
    vol.create_file_in_root("B", &PB, ts(4)).unwrap();
    assert_reused(&mut vol, x);
    assert!(!vol.quarantine_contains(x).unwrap());

    // Reclaim latency measured by the allocator: two generations, for every
    // promoted block.
    let stats = vol.last_commit_stats().unwrap();
    assert!(stats.alloc.blocks_promoted > 0);
    assert_eq!(
        stats.alloc.reclaim_latency_generations,
        2 * stats.alloc.blocks_promoted,
        "every promoted fixture block must spend two generations in quarantine"
    );

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}
