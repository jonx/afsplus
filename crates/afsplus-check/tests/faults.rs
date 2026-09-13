//! Deterministic fault injection (write/flush errors, as opposed to power
//! cuts): a failed transaction must leave the committed state untouched, and
//! a transient fault must be retryable.

use afsplus_block::{FaultBackend, FaultPlan, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::Timespec;

const BS: usize = 4096;

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [42u8; 16],
            label: "FaultVol".into(),
            region_size: 64,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: Timespec {
                seconds: 1_780_000_000,
                nanoseconds: 0,
            },
        },
    )
    .unwrap();
    dev
}

fn ts() -> Timespec {
    Timespec {
        seconds: 1_780_000_100,
        nanoseconds: 0,
    }
}

#[test]
fn failed_write_at_every_index_leaves_committed_state_untouched() {
    // An empty-file transaction performs 7 writes (5 metadata + 1 bitmap
    // page + 1 checkpoint) and 2 flushes.
    for write_index in 0..7 {
        let plan = FaultPlan {
            fail_write_index: Some(write_index),
            fail_flush_index: None,
            fail_hard: false,
        };
        let mut vol = mount(FaultBackend::new(formatted(), plan)).unwrap();
        let err = vol.create_file_in_root("hello.txt", b"", ts()).unwrap_err();
        assert!(
            err.to_string().contains("injected"),
            "write {write_index}: expected injected fault, got {err}"
        );
        assert_eq!(
            vol.generation(),
            1,
            "write {write_index}: state must not advance"
        );

        let mut dev = vol.into_device().into_inner();
        let report = check_device(&mut dev);
        assert!(
            report.is_clean(),
            "write {write_index}: {:?}",
            report.errors
        );
        let mut vol = mount(dev).unwrap();
        assert_eq!(vol.generation(), 1);
        assert!(vol.list_root().unwrap().is_empty());
    }
}

#[test]
fn failed_flush_at_each_barrier_leaves_committed_state_untouched() {
    for flush_index in 0..2 {
        let plan = FaultPlan {
            fail_write_index: None,
            fail_flush_index: Some(flush_index),
            fail_hard: false,
        };
        let mut vol = mount(FaultBackend::new(formatted(), plan)).unwrap();
        assert!(vol.create_file_in_root("hello.txt", b"", ts()).is_err());
        assert_eq!(
            vol.generation(),
            1,
            "flush {flush_index}: state must not advance"
        );
    }
}

#[test]
fn transient_fault_is_retryable() {
    let plan = FaultPlan {
        fail_write_index: Some(2),
        fail_flush_index: None,
        fail_hard: false,
    };
    let mut vol = mount(FaultBackend::new(formatted(), plan)).unwrap();
    assert!(vol
        .create_file_in_root("hello.txt", b"payload", ts())
        .is_err());

    // Same volume, same operation: must succeed now and be fully consistent.
    let id = vol
        .create_file_in_root("hello.txt", b"payload", ts())
        .unwrap();
    assert_eq!(vol.generation(), 2);

    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.lookup_root("hello.txt").unwrap(), Some(id));
}

#[test]
fn uncertain_checkpoint_publication_requires_remount_before_more_writes() {
    let mut baseline = mount(afsplus_block::TraceBackend::new(formatted())).unwrap();
    baseline
        .create_file_in_root("first", b"committed bytes", ts())
        .unwrap();
    let flushes = baseline.device_mut().stats().flushes;
    // MemoryBackend models a complete checkpoint write visible even when the
    // final durability acknowledgement fails. The outcome is uncertain to
    // the mounted writer, so retrying from its old allocator is unsafe.
    let mut vol = mount(FaultBackend::new(
        formatted(),
        FaultPlan {
            fail_write_index: None,
            fail_flush_index: Some(flushes - 1),
            fail_hard: false,
        },
    ))
    .unwrap();
    assert!(vol
        .create_file_in_root("first", b"committed bytes", ts())
        .is_err());
    assert!(
        vol.create_file_in_root("second", b"must not overwrite first", ts())
            .is_err(),
        "writer continued after uncertain checkpoint publication"
    );
    let mut image = vol.into_device().into_inner();
    let report = check_device(&mut image);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut recovered = mount(image).unwrap();
    let first = recovered.lookup_root("first").unwrap().unwrap();
    assert_eq!(recovered.read_file(first).unwrap(), b"committed bytes");
    assert_eq!(recovered.lookup_root("second").unwrap(), None);
    recovered
        .create_file_in_root("after-remount", b"safe", ts())
        .unwrap();
}

/// A device may complete a write and then report an error. Read failure after
/// publication also must not let the writer continue from its previous roots.
#[test]
fn completed_checkpoint_write_and_adoption_read_errors_block_mutations() {
    use afsplus_block::{BlockDevice, BlockError};
    struct AmbiguousDevice {
        inner: MemoryBackend,
        checkpoints: [u64; 2],
        published: bool,
        fail_write: bool,
    }
    impl BlockDevice for AmbiguousDevice {
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }
        fn total_blocks(&self) -> u64 {
            self.inner.total_blocks()
        }
        fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
            if self.published && !self.fail_write {
                return Err(BlockError::Injected("post-publication read"));
            }
            self.inner.read_block(lba, buf)
        }
        fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
            self.inner.write_block(lba, data)?;
            if self.checkpoints.contains(&lba) {
                self.published = true;
                if self.fail_write {
                    return Err(BlockError::Injected("completed write"));
                }
            }
            Ok(())
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            self.inner.flush()
        }
    }
    for fail_write in [true, false] {
        let vol = mount(formatted()).unwrap();
        let checkpoints = vol.ident().checkpoint_slots;
        let mut vol = mount(AmbiguousDevice {
            inner: vol.into_device(),
            checkpoints,
            published: false,
            fail_write,
        })
        .unwrap();
        assert!(vol.create_file_in_root("first", b"intact", ts()).is_err());
        assert!(vol.device_mut().published);
        assert!(matches!(
            vol.create_file_in_root("second", b"unsafe", ts()),
            Err(afsplus_core::CoreError::WindowPoisoned)
        ));
        let mut image = vol.into_device().inner;
        assert!(check_device(&mut image).is_clean());
        let mut recovered = mount(image).unwrap();
        let id = recovered.lookup_root("first").unwrap().unwrap();
        assert_eq!(recovered.read_file(id).unwrap(), b"intact");
        assert_eq!(recovered.lookup_root("second").unwrap(), None);
    }
}

#[test]
fn replacement_write_and_flush_failures_preserve_complete_namespace_and_bytes() {
    use afsplus_block::TraceBackend;
    use afsplus_format::OBJECT_ROOT;
    let mut initial = mount(formatted()).unwrap();
    let source = initial
        .create_file_in_root("source", b"source bytes", ts())
        .unwrap();
    let target = initial
        .create_file_in_root("target", b"target bytes", ts())
        .unwrap();
    let base_generation = initial.generation();
    let base = initial.into_device();
    let mut reference = mount(TraceBackend::new(base.clone())).unwrap();
    reference
        .rename_replace(OBJECT_ROOT, "source", OBJECT_ROOT, "target", ts())
        .unwrap();
    let stats = reference.device_mut().stats();
    for plan in (0..stats.writes)
        .map(|i| FaultPlan {
            fail_write_index: Some(i),
            fail_flush_index: None,
            fail_hard: false,
        })
        .chain((0..stats.flushes).map(|i| FaultPlan {
            fail_write_index: None,
            fail_flush_index: Some(i),
            fail_hard: false,
        }))
    {
        let mut vol = mount(FaultBackend::new(base.clone(), plan)).unwrap();
        assert!(
            vol.rename_replace(OBJECT_ROOT, "source", OBJECT_ROOT, "target", ts())
                .is_err(),
            "{plan:?}"
        );
        assert!(vol.device_mut().tripped(), "{plan:?}");
        let mut image = vol.into_device().into_inner();
        let report = check_device(&mut image);
        assert!(report.is_clean(), "{plan:?}: {:?}", report.errors);
        let mut recovered = mount(image).unwrap();
        // This backend fails writes before forwarding, but a final flush
        // error follows a complete checkpoint. Only that case selects new.
        if plan.fail_flush_index == Some(stats.flushes - 1) {
            assert_eq!(recovered.generation(), base_generation + 1, "{plan:?}");
            assert_eq!(recovered.lookup_root("source").unwrap(), None);
            assert_eq!(recovered.lookup_root("target").unwrap(), Some(source));
            assert_eq!(recovered.read_file(source).unwrap(), b"source bytes");
        } else {
            assert_eq!(recovered.generation(), base_generation, "{plan:?}");
            assert_eq!(recovered.lookup_root("source").unwrap(), Some(source));
            assert_eq!(recovered.lookup_root("target").unwrap(), Some(target));
            assert_eq!(recovered.read_file(source).unwrap(), b"source bytes");
            assert_eq!(recovered.read_file(target).unwrap(), b"target bytes");
        }
    }
}
