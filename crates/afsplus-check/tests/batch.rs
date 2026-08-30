//! Group commit / bounded atomic namespace batches (ADR-026): one
//! transaction, one checkpoint, all-or-nothing visibility.

use afsplus_block::{crash_states, FaultBackend, FaultPlan, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{mkfs, mount, CoreError, MkfsParams, Volume};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(total: u64) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, total);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [66u8; 16],
            label: "BatchVol".into(),
            region_size: 1024,
            reclaim_caps: Default::default(),
            log_slots: 8,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

fn listing(vol: &mut Volume<MemoryBackend>) -> Vec<(String, Vec<u8>)> {
    let mut rows = Vec::new();
    for (name, id) in vol.list_root().unwrap() {
        let content = vol.read_file(id).unwrap();
        rows.push((name, content));
    }
    rows
}

#[test]
fn batch_matches_the_equivalent_sequential_operations() {
    // Same operations, batched on one volume and sequential on its twin:
    // identical namespace and contents, but exactly one generation advance.
    let ops = [
        BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "a",
            content: b"alpha",
        },
        BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "b",
            content: b"beta",
        },
        BatchOp::Rename {
            source_parent_id: OBJECT_ROOT,
            source_name: "a",
            target_parent_id: OBJECT_ROOT,
            target_name: "c",
            replace: false,
        },
        BatchOp::DeleteFile {
            parent_id: OBJECT_ROOT,
            name: "b",
        },
        BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "d",
            content: b"delta",
        },
    ];

    let mut batched = mount(formatted(1024)).unwrap();
    let results = batched.run_batch(&ops, ts(1)).unwrap();
    assert_eq!(
        batched.generation(),
        2,
        "one checkpoint for the whole batch"
    );
    assert_eq!(results.iter().filter(|r| r.is_some()).count(), 3);

    let mut sequential = mount(formatted(1024)).unwrap();
    sequential
        .create_file_in_root("a", b"alpha", ts(1))
        .unwrap();
    sequential.create_file_in_root("b", b"beta", ts(1)).unwrap();
    sequential
        .rename(OBJECT_ROOT, "a", OBJECT_ROOT, "c", ts(1))
        .unwrap();
    sequential.delete_file_in_root("b", ts(1)).unwrap();
    sequential
        .create_file_in_root("d", b"delta", ts(1))
        .unwrap();

    assert_eq!(listing(&mut batched), listing(&mut sequential));

    for vol in [batched, sequential] {
        let mut dev = vol.into_device();
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "{:?}", report.errors);
        let mut vol = mount(dev).unwrap();
        assert!(
            vol.lookup_root("c").unwrap().is_some(),
            "remount must see the batch"
        );
    }
}

#[test]
fn same_batch_create_and_delete_cancel_without_quarantine() {
    let mut vol = mount(formatted(1024)).unwrap();
    let pending_before = vol.reclaim_pending_blocks();
    let results = vol
        .run_batch(
            &[
                BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name: "ephemeral",
                    content: &[7u8; 3 * BS],
                },
                BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name: "kept",
                    content: b"stay",
                },
                BatchOp::DeleteFile {
                    parent_id: OBJECT_ROOT,
                    name: "ephemeral",
                },
            ],
            ts(1),
        )
        .unwrap();
    assert!(results[0].is_some());
    assert_eq!(vol.lookup_root("ephemeral").unwrap(), None);
    assert!(vol.lookup_root("kept").unwrap().is_some());
    // The cancelled file's three data blocks were released, not retired: the
    // queue grew only by the batch's ordinary COW retirements.
    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.alloc.blocks_released, 3);
    assert!(
        vol.reclaim_pending_blocks() < pending_before + 8,
        "cancelled data must not enter quarantine"
    );
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn rename_replace_swaps_content_atomically_and_quarantines_the_old_target() {
    let mut vol = mount(formatted(1024)).unwrap();
    let old = vol
        .create_file_in_root("HEAD", &[0xAAu8; 2 * BS], ts(1))
        .unwrap();
    let old_data = vol.stat(old).unwrap().unwrap().data_root;
    vol.create_file_in_root("HEAD.lock", b"new ref", ts(2))
        .unwrap();

    vol.rename_replace(OBJECT_ROOT, "HEAD.lock", OBJECT_ROOT, "HEAD", ts(3))
        .unwrap();
    assert_eq!(vol.lookup_root("HEAD.lock").unwrap(), None);
    let head = vol.lookup_root("HEAD").unwrap().unwrap();
    assert_eq!(vol.read_file(head).unwrap(), b"new ref");
    assert!(
        vol.quarantine_contains(old_data).unwrap(),
        "old content quarantined"
    );
    assert!(vol.stat(old).unwrap().is_none(), "old object gone");

    // Replace of a multiply-linked file only drops one link.
    let kept = vol.create_file_in_root("shared", b"shared", ts(4)).unwrap();
    vol.link_file(kept, OBJECT_ROOT, "shared2", ts(5)).unwrap();
    vol.create_file_in_root("tmp", b"x", ts(6)).unwrap();
    vol.rename_replace(OBJECT_ROOT, "tmp", OBJECT_ROOT, "shared", ts(7))
        .unwrap();
    assert_eq!(vol.stat(kept).unwrap().unwrap().link_count, 1);
    assert_eq!(vol.read_file(kept).unwrap(), b"shared");

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn failing_operation_aborts_the_whole_batch() {
    let mut vol = mount(formatted(1024)).unwrap();
    vol.create_file_in_root("existing", b"x", ts(1)).unwrap();
    let error = vol
        .run_batch(
            &[
                BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name: "fresh",
                    content: b"y",
                },
                BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name: "existing",
                    content: b"z",
                },
            ],
            ts(2),
        )
        .unwrap_err();
    assert!(matches!(error, CoreError::AlreadyExists));
    assert_eq!(vol.generation(), 2, "failed batch must not advance state");
    assert_eq!(
        vol.lookup_root("fresh").unwrap(),
        None,
        "no partial visibility"
    );

    // Transient device fault mid-batch: nothing commits, retry succeeds.
    let dev = vol.into_device();
    let plan = FaultPlan {
        fail_write_index: Some(4),
        fail_flush_index: None,
        fail_hard: false,
    };
    let mut vol = mount(FaultBackend::new(dev, plan)).unwrap();
    let ops = [
        BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "one",
            content: b"1",
        },
        BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "two",
            content: b"2",
        },
    ];
    assert!(vol.run_batch(&ops, ts(3)).is_err());
    assert_eq!(vol.lookup_root("one").unwrap(), None);
    vol.run_batch(&ops, ts(4)).unwrap();
    assert!(vol.lookup_root("one").unwrap().is_some());
    assert!(vol.lookup_root("two").unwrap().is_some());

    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn every_crash_state_of_a_ref_update_batch_is_all_or_nothing() {
    // The Git pattern as ONE batch: create the lock, atomically replace the
    // ref. No modeled crash state may show the lock file, a missing HEAD, or
    // a torn mixture of old and new content.
    let base = {
        let mut vol = mount(formatted(1024)).unwrap();
        vol.create_file_in_root("HEAD", &[0x01u8; 2000], ts(1))
            .unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();

    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    vol.run_batch(
        &[
            BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "HEAD.lock",
                content: &[0x02u8; 2000],
            },
            BatchOp::Rename {
                source_parent_id: OBJECT_ROOT,
                source_name: "HEAD.lock",
                target_parent_id: OBJECT_ROOT,
                target_name: "HEAD",
                replace: true,
            },
        ],
        ts(2),
    )
    .unwrap();
    let (_, log) = vol.into_device().into_parts();

    let mut pre = 0u64;
    let mut post = 0u64;
    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let mut vol = mount(image).unwrap_or_else(|e| panic!("{context}: {e}"));
            assert_eq!(
                vol.lookup_root("HEAD.lock").unwrap(),
                None,
                "{context}: the lock file must never be visible"
            );
            let head = vol
                .lookup_root("HEAD")
                .unwrap()
                .unwrap_or_else(|| panic!("{context}: HEAD must always exist"));
            let content = vol.read_file(head).unwrap();
            match vol.generation() {
                g if g == pre_generation => {
                    pre += 1;
                    assert_eq!(content, vec![0x01u8; 2000], "{context}: old ref damaged");
                }
                g if g == pre_generation + 1 => {
                    post += 1;
                    assert_eq!(content, vec![0x02u8; 2000], "{context}: new ref damaged");
                }
                g => panic!("{context}: disallowed generation {g}"),
            }
        }
    }
    assert!(pre > 0 && post > 0, "matrix must produce both outcomes");
}

#[test]
fn a_large_batch_is_one_generation_and_bounded() {
    let mut vol = mount(formatted(4096)).unwrap();
    let names: Vec<String> = (0..256).map(|i| format!("pkg-{i:04}")).collect();
    let ops: Vec<BatchOp<'_>> = names
        .iter()
        .map(|name| BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name,
            content: b"payload",
        })
        .collect();
    vol.run_batch(&ops, ts(1)).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_eq!(vol.list_root().unwrap().len(), 256);
    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.flushes, 3, "one barrier set for 256 files");
    assert!(
        stats.metadata_blocks_written < 320,
        "shared tree paths must amortize: {} metadata blocks",
        stats.metadata_blocks_written
    );

    // Bound enforcement.
    let too_many: Vec<BatchOp<'_>> = (0..1025)
        .map(|_| BatchOp::DeleteFile {
            parent_id: OBJECT_ROOT,
            name: "pkg-0000",
        })
        .collect();
    assert!(matches!(
        vol.run_batch(&too_many, ts(2)),
        Err(CoreError::PrototypeLimit(_))
    ));

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}
