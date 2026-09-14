//! Borrowed batch payloads: cancellation, padding, failure and crash semantics.
use afsplus_block::{
    for_each_crash_state, BlockDevice, BlockError, FaultBackend, FaultPlan, MemoryBackend,
    RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{mkfs, mount_with_options, MkfsParams, MountOptions, Volume};
use afsplus_format::{Timespec, OBJECT_ROOT};
use std::num::NonZeroUsize;

const BS: usize = 4096;
fn ts() -> Timespec {
    Timespec {
        seconds: 1,
        nanoseconds: 0,
    }
}
fn options(pages: usize) -> MountOptions {
    MountOptions {
        tree_cache_pages: NonZeroUsize::new(pages),
        ..Default::default()
    }
}
fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x61; 16],
            label: "Borrowed".into(),
            region_size: 64,
            timestamp: ts(),
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
        },
    )
    .unwrap();
    dev
}
fn create<'a>(name: &'a str, content: &'a [u8]) -> BatchOp<'a> {
    BatchOp::CreateFile {
        parent_id: OBJECT_ROOT,
        name,
        content,
    }
}
fn delete(name: &str) -> BatchOp<'_> {
    BatchOp::DeleteFile {
        parent_id: OBJECT_ROOT,
        name,
    }
}
fn verify<D: BlockDevice>(volume: &mut Volume<D>, expected: &[(&str, &[u8])]) {
    assert_eq!(volume.list_root().unwrap().len(), expected.len());
    for (name, bytes) in expected {
        let id = volume.lookup_root(name).unwrap().unwrap();
        assert_eq!(volume.read_file(id).unwrap(), *bytes);
    }
}

struct ObserveBorrowing {
    inner: MemoryBackend,
    expected: Vec<usize>,
    direct: usize,
}
impl BlockDevice for ObserveBorrowing {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, bytes: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(lba, bytes)
    }
    fn write_block(&mut self, lba: u64, bytes: &[u8]) -> Result<(), BlockError> {
        if self.expected.contains(&(bytes.as_ptr() as usize)) {
            self.direct += 1;
        }
        self.inner.write_block(lba, bytes)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

#[test]
fn surviving_payloads_borrow_full_blocks_and_zero_each_tail_in_all_profiles() {
    let discarded = vec![0x11; BS * 3];
    let aligned = vec![0x22; BS * 3];
    let long_tail = vec![0x33; BS + 101];
    let short_tail = vec![0x44; 7];
    let expected_addresses = aligned
        .as_chunks::<BS>()
        .0
        .iter()
        .chain(long_tail.as_chunks::<BS>().0.iter())
        .map(|b| b.as_ptr() as usize)
        .collect::<Vec<_>>();
    for pages in [2, 4, 8, usize::MAX] {
        let dev = ObserveBorrowing {
            inner: formatted(),
            expected: expected_addresses.clone(),
            direct: 0,
        };
        let mut volume = mount_with_options(TraceBackend::new(dev), options(pages)).unwrap();
        volume.device_mut().reset();
        volume
            .run_batch(
                &[
                    create("cancelled", &discarded),
                    delete("cancelled"),
                    create("cancelled", &aligned),
                    BatchOp::Rename {
                        source_parent_id: OBJECT_ROOT,
                        source_name: "cancelled",
                        target_parent_id: OBJECT_ROOT,
                        target_name: "aligned",
                        replace: false,
                    },
                    create("tail", &long_tail),
                    create("short", &short_tail),
                    create("empty", b""),
                    BatchOp::Rename {
                        source_parent_id: OBJECT_ROOT,
                        source_name: "tail",
                        target_parent_id: OBJECT_ROOT,
                        target_name: "renamed",
                        replace: false,
                    },
                ],
                ts(),
            )
            .unwrap();
        let stats = volume.last_commit_stats().unwrap();
        assert_eq!(stats.data_blocks_written, 6);
        assert_eq!(stats.flushes, 3);
        assert_eq!(
            stats.bytes_written,
            volume.device_mut().stats().bytes_written
        );
        assert_eq!(
            volume.device_mut().inner().direct,
            4,
            "full blocks were copied"
        );
        verify(
            &mut volume,
            &[
                ("aligned", &aligned),
                ("renamed", &long_tail),
                ("short", &short_tail),
                ("empty", b""),
            ],
        );
        for (name, offset, prefix) in [("renamed", 1, 101), ("short", 0, 7)] {
            let id = volume.lookup_root(name).unwrap().unwrap();
            let record = volume.stat(id).unwrap().unwrap();
            let mut bytes = vec![0xff; BS];
            volume
                .device_mut()
                .read_block(record.data_root + offset, &mut bytes)
                .unwrap();
            assert!(
                bytes[prefix..].iter().all(|b| *b == 0),
                "stale tail of {name}"
            );
        }
        let mut dev = volume.into_device().into_inner().inner;
        assert!(check_device(&mut dev).is_clean());
        let mut volume = mount_with_options(dev, options(pages)).unwrap();
        verify(
            &mut volume,
            &[
                ("aligned", &aligned),
                ("renamed", &long_tail),
                ("short", &short_tail),
                ("empty", b""),
            ],
        );
    }
}

#[test]
fn borrowed_write_and_data_barrier_errors_preserve_old_state_and_allow_retry() {
    let payload = vec![0x91; BS + 17];
    let ops = [
        create("cancel", &payload),
        delete("cancel"),
        create("kept", &payload),
        create("small", b"seven77"),
    ];
    for pages in [2, 4, 8, usize::MAX] {
        for fault in 0..4 {
            let plan = FaultPlan {
                fail_write_index: (fault < 3).then_some(fault),
                fail_flush_index: (fault == 3).then_some(0),
                fail_hard: false,
            };
            let mut volume =
                mount_with_options(FaultBackend::new(formatted(), plan), options(pages)).unwrap();
            let generation = volume.generation();
            assert!(volume.run_batch(&ops, ts()).is_err());
            assert!(volume.device_mut().tripped());
            assert_eq!(volume.generation(), generation);
            verify(&mut volume, &[]);
            assert!(check_device(volume.device_mut()).is_clean());
            // A transient pre-publication failure is retryable without remount.
            volume.run_batch(&ops, ts()).unwrap();
            verify(&mut volume, &[("kept", &payload), ("small", b"seven77")]);
            let mut image = volume.into_device().into_inner();
            assert!(check_device(&mut image).is_clean());
            let mut recovered = mount_with_options(image, options(pages)).unwrap();
            verify(&mut recovered, &[("kept", &payload), ("small", b"seven77")]);
        }
    }
}

#[test]
fn borrowed_payload_crash_matrix_preserves_exact_old_or_new_state() {
    let payload = vec![0x52; BS + 13];
    let ops = [
        create("cancel", &payload),
        delete("cancel"),
        create("kept", &payload),
        create("small", b"short"),
    ];
    for pages in [2, 4, 8, usize::MAX] {
        let base = formatted();
        let mut volume =
            mount_with_options(RecordingBackend::new(base.clone()), options(pages)).unwrap();
        let generation = volume.generation();
        volume.run_batch(&ops, ts()).unwrap();
        let (_, log) = volume.into_device().into_parts();
        let mut before = 0;
        let mut after = 0;
        for point in 0..=log.len() {
            for_each_crash_state(&base, &log, point, |state| {
                let mut image = state.image;
                assert!(check_device(&mut image).is_clean(), "{}", state.description);
                let mut recovered = mount_with_options(image, options(pages)).unwrap();
                if recovered.generation() == generation {
                    before += 1;
                    verify(&mut recovered, &[]);
                } else {
                    assert_eq!(recovered.generation(), generation + 1);
                    after += 1;
                    verify(&mut recovered, &[("kept", &payload), ("small", b"short")]);
                }
            });
        }
        assert!(before > 0 && after > 0);
    }
}

#[test]
fn cancelled_and_invalid_batches_do_not_issue_payload_writes() {
    let payload = vec![0x84; BS * 4 + 1];
    for pages in [2, 4, 8, usize::MAX] {
        let mut volume =
            mount_with_options(TraceBackend::new(formatted()), options(pages)).unwrap();
        let generation = volume.generation();
        volume.device_mut().reset();
        volume
            .run_batch(&[create("gone", &payload), delete("gone")], ts())
            .unwrap();
        assert_eq!(volume.generation(), generation);
        assert_eq!(volume.device_mut().stats().writes, 0);
        assert_eq!(volume.device_mut().stats().flushes, 0);
        assert!(volume
            .run_batch(
                &[create("duplicate", &payload), create("duplicate", b"bad")],
                ts()
            )
            .is_err());
        assert_eq!(volume.generation(), generation);
        assert_eq!(volume.device_mut().stats().writes, 0);
        assert_eq!(volume.device_mut().stats().flushes, 0);
        verify(&mut volume, &[]);
        assert!(check_device(&mut volume.into_device().into_inner()).is_clean());
    }
}
