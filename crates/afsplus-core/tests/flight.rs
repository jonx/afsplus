use afsplus_block::{
    BlockDevice, BlockError, FaultBackend, FaultPlan, MemoryBackend, TraceBackend,
};
use afsplus_core::flight::{EventKind, FlightRecorder};
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::Timespec;
use std::num::NonZeroUsize;

fn image() -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x39; 16],
            label: "flight".into(),
            region_size: 64,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: false,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    dev
}

fn recorder(capacity: usize) -> FlightRecorder {
    FlightRecorder::new(NonZeroUsize::new(capacity).unwrap()).unwrap()
}

#[test]
fn bounded_recorder_preserves_io_and_exact_image() {
    let original = image();
    let mut plain = mount(TraceBackend::new(original.clone())).unwrap();
    let mut observed = mount(TraceBackend::new(original)).unwrap();
    assert!(observed.flight_recorder().is_none());
    observed.replace_flight_recorder(Some(recorder(3)));
    for name in ["first", "second"] {
        plain
            .create_file_in_root(name, b"data", Timespec::default())
            .unwrap();
        observed
            .create_file_in_root(name, b"data", Timespec::default())
            .unwrap();
    }
    let ring = observed.replace_flight_recorder(None).unwrap();
    assert_eq!(ring.capacity(), 3);
    assert_eq!(ring.dropped(), 9);
    let events: Vec<_> = ring.events().copied().collect();
    assert_eq!(
        events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        [10, 11, 12]
    );
    assert!(events.iter().all(|e| e.attempt == 2));
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            EventKind::PublicationBegin,
            EventKind::CheckpointDurable,
            EventKind::Adopted
        ]
    );
    let plain = plain.into_device();
    let observed = observed.into_device();
    assert_eq!(plain.events(), observed.events());
    for lba in 0..256 {
        assert_eq!(plain.inner().peek(lba), observed.inner().peek(lba));
    }
    assert_eq!(
        mount(observed.into_inner())
            .unwrap()
            .list_root()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn failed_barriers_distinguish_retry_from_uncertain_publication() {
    for (flush, remount) in [(0, false), (1, true)] {
        let dev = FaultBackend::new(
            image(),
            FaultPlan {
                fail_flush_index: Some(flush),
                ..Default::default()
            },
        );
        let mut volume = mount(dev).unwrap();
        volume.replace_flight_recorder(Some(recorder(32)));
        let generation = volume.generation();
        assert!(volume
            .create_file_in_root("first", b"", Timespec::default())
            .is_err());
        let events: Vec<_> = volume
            .flight_recorder()
            .unwrap()
            .events()
            .copied()
            .collect();
        assert_eq!(events.last().unwrap().kind, EventKind::Failed);
        assert_eq!(events.last().unwrap().requires_remount, remount);
        assert!(!events
            .iter()
            .any(|e| e.kind == EventKind::CheckpointDurable));
        assert_eq!(
            events.iter().any(|e| e.kind == EventKind::PublicationBegin),
            remount
        );
        assert_eq!(volume.generation(), generation);
        let retry = volume.create_file_in_root("second", b"", Timespec::default());
        if remount {
            assert!(retry.is_err());
            assert_eq!(
                volume.flight_recorder().unwrap().events().len(),
                events.len()
            );
            let mut reopened = mount(volume.into_device()).unwrap();
            assert!(reopened.lookup_root("first").unwrap().is_some());
        } else {
            retry.unwrap();
            let last = *volume.flight_recorder().unwrap().events().last().unwrap();
            assert_eq!(last.attempt, 2);
            assert_eq!(last.generation, events[0].generation);
            assert_eq!(last.kind, EventKind::Adopted);
            assert!(!last.requires_remount);
        }
    }
}

/// Fail only while adopting roots after the successful final barrier.
struct AdoptionFault {
    inner: MemoryBackend,
    flushes: usize,
}

impl BlockDevice for AdoptionFault {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if self.flushes == 2 {
            return Err(BlockError::Injected("adoption read"));
        }
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, buf: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, buf)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()?;
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn successful_barrier_does_not_hide_failed_adoption() {
    let mut volume = mount(AdoptionFault {
        inner: image(),
        flushes: 0,
    })
    .unwrap();
    volume.replace_flight_recorder(Some(recorder(16)));
    assert!(volume
        .create_file_in_root("durable", b"", Timespec::default())
        .is_err());
    let events: Vec<_> = volume
        .flight_recorder()
        .unwrap()
        .events()
        .copied()
        .collect();
    assert_eq!(events[events.len() - 2].kind, EventKind::CheckpointDurable);
    assert_eq!(events.last().unwrap().kind, EventKind::Failed);
    assert!(events.last().unwrap().requires_remount);
    assert!(!events.iter().any(|e| e.kind == EventKind::Adopted));
    let mut recovered = mount(volume.into_device().inner).unwrap();
    assert!(recovered.lookup_root("durable").unwrap().is_some());
}

#[test]
fn checkpoint_write_failure_does_not_claim_durability() {
    let mut good = mount(TraceBackend::new(image())).unwrap();
    good.create_file_in_root("file", b"", Timespec::default())
        .unwrap();
    let writes = good.into_device().stats().writes;
    let mut volume = mount(FaultBackend::new(
        image(),
        FaultPlan {
            fail_write_index: Some(writes - 1),
            ..Default::default()
        },
    ))
    .unwrap();
    volume.replace_flight_recorder(Some(recorder(16)));
    assert!(volume
        .create_file_in_root("file", b"", Timespec::default())
        .is_err());
    let events: Vec<_> = volume
        .flight_recorder()
        .unwrap()
        .events()
        .copied()
        .collect();
    assert_eq!(events[events.len() - 2].kind, EventKind::PublicationBegin);
    assert_eq!(events.last().unwrap().kind, EventKind::Failed);
    assert!(events.last().unwrap().requires_remount);
    assert!(!events
        .iter()
        .any(|e| e.kind == EventKind::CheckpointDurable));
    assert!(mount(volume.into_device())
        .unwrap()
        .lookup_root("file")
        .unwrap()
        .is_none());
}

#[test]
fn every_category_selection_preserves_four_profile_io_images_and_failures() {
    use afsplus_core::flight::{Categories, Category};
    use afsplus_core::{mount_with_options, MountOptions};
    for pages in [2, 4, 8, usize::MAX] {
        for bits in 0u8..16 {
            let mut selected = Categories::NONE;
            for (bit, category) in [
                Category::Transaction,
                Category::Checkpoint,
                Category::Io,
                Category::Error,
            ]
            .into_iter()
            .enumerate()
            {
                if bits & (1 << bit) != 0 {
                    selected = selected.with(category);
                }
            }
            for failure in [None, Some(0), Some(1)] {
                let original = image();
                let options = MountOptions {
                    tree_cache_pages: NonZeroUsize::new(pages),
                    ..Default::default()
                };
                let backend = |image| {
                    TraceBackend::new(FaultBackend::new(
                        image,
                        FaultPlan {
                            fail_flush_index: failure,
                            ..Default::default()
                        },
                    ))
                };
                let mut plain = mount_with_options(backend(original.clone()), options).unwrap();
                let mut observed = mount_with_options(backend(original), options).unwrap();
                let mut ring = recorder(32);
                ring.set_categories(selected);
                observed.replace_flight_recorder(Some(ring));
                for name in ["first", "second"] {
                    let a = plain.create_file_in_root(name, b"data", Timespec::default());
                    let b = observed.create_file_in_root(name, b"data", Timespec::default());
                    assert_eq!(format!("{a:?}"), format!("{b:?}"));
                }
                let ring = observed.replace_flight_recorder(None).unwrap();
                assert_eq!(ring.dropped(), 0);
                assert!(ring.events().all(|e| selected.contains(e.kind.category())));
                if bits == 0 {
                    assert_eq!(ring.events().len(), 0);
                    assert!(ring.filtered() > 0);
                }
                if bits == 15 {
                    assert_eq!(ring.filtered(), 0);
                }
                let plain = plain.into_device();
                let observed = observed.into_device();
                assert_eq!(plain.events(), observed.events());
                let plain = plain.into_inner().into_inner();
                let observed = observed.into_inner().into_inner();
                for lba in 0..256 {
                    assert_eq!(plain.peek(lba), observed.peek(lba));
                }
            }
        }
    }
}

#[test]
fn bounded_live_consumer_survives_backpressure_and_disconnect_without_changing_volume() {
    use afsplus_core::flight::{Categories, Category, Event, SinkResult};
    use afsplus_core::{mount_with_options, MountOptions};
    use std::sync::mpsc::{sync_channel, TrySendError};
    for pages in [2, 4, 8, usize::MAX] {
        for failure in [None, Some(0), Some(1)] {
            let original = image();
            let options = MountOptions {
                tree_cache_pages: NonZeroUsize::new(pages),
                ..Default::default()
            };
            let backend = |image| {
                TraceBackend::new(FaultBackend::new(
                    image,
                    FaultPlan {
                        fail_flush_index: failure,
                        ..Default::default()
                    },
                ))
            };
            let mut plain = mount_with_options(backend(original.clone()), options).unwrap();
            let mut observed = mount_with_options(backend(original), options).unwrap();
            let (sender, receiver) = sync_channel::<Event>(1);
            let mut ring = recorder(1);
            ring.replace_sink(Some(Box::new(move |event: Event| {
                match sender.try_send(event) {
                    Ok(()) => SinkResult::Accepted,
                    Err(TrySendError::Full(_)) => SinkResult::Busy,
                    Err(TrySendError::Disconnected(_)) => SinkResult::Closed,
                }
            })));
            observed.replace_flight_recorder(Some(ring));
            for name in ["first", "second"] {
                let a = plain.create_file_in_root(name, b"data", Timespec::default());
                let b = observed.create_file_in_root(name, b"data", Timespec::default());
                assert_eq!(format!("{a:?}"), format!("{b:?}"));
                let ring = observed.flight_recorder().unwrap();
                if let Ok(event) = receiver.try_recv() {
                    assert_eq!(event.kind, EventKind::Begin);
                    assert!(event.sequence < ring.events().last().unwrap().sequence);
                }
            }
            drop(receiver);
            let mut ring = observed.replace_flight_recorder(None).unwrap();
            ring.set_categories(Categories::NONE.with(Category::Transaction));
            observed.replace_flight_recorder(Some(ring));
            let a = plain.create_file_in_root("third", b"data", Timespec::default());
            let b = observed.create_file_in_root("third", b"data", Timespec::default());
            assert_eq!(format!("{a:?}"), format!("{b:?}"));
            let ring = observed.replace_flight_recorder(None).unwrap();
            if failure.is_none() {
                assert_eq!(
                    (
                        ring.delivered(),
                        ring.missed(),
                        ring.filtered(),
                        ring.dropped()
                    ),
                    (2, 12, 4, 13)
                );
                assert!(ring.sink_closed());
                assert_eq!(ring.events().last().unwrap().sequence, 18);
            }
            let plain = plain.into_device();
            let observed = observed.into_device();
            assert_eq!(plain.events(), observed.events());
            let plain = plain.into_inner().into_inner();
            let observed = observed.into_inner().into_inner();
            for lba in 0..256 {
                assert_eq!(plain.peek(lba), observed.peek(lba));
            }
        }
    }
}

#[test]
fn api_spans_correlate_nested_calls_refusals_and_commits_without_changing_io() {
    use afsplus_core::flight::{ApiContext, Category};
    use afsplus_core::{mount_with_options, MountOptions};
    for pages in [2, 4, 8, usize::MAX] {
        for failure in [None, Some(0), Some(1), Some(2)] {
            let original = image();
            let options = MountOptions {
                tree_cache_pages: NonZeroUsize::new(pages),
                ..Default::default()
            };
            let backend = |image| {
                TraceBackend::new(FaultBackend::new(
                    image,
                    FaultPlan {
                        fail_flush_index: failure,
                        ..Default::default()
                    },
                ))
            };
            let mut plain = mount_with_options(backend(original.clone()), options).unwrap();
            let mut observed = mount_with_options(backend(original), options).unwrap();
            let mut ring = recorder(1024);
            ring.enable_api_observation();
            observed.replace_flight_recorder(Some(ring));
            for name in ["first", "first", "second"] {
                let a = plain.create_file_in_root(name, b"data", Timespec::default());
                let b = observed.create_file_in_root(name, b"data", Timespec::default());
                assert_eq!(format!("{a:?}"), format!("{b:?}"));
            }
            let ring = observed.replace_flight_recorder(None).unwrap();
            assert_eq!((ring.dropped(), ring.filtered()), (0, 0));
            let mut stack = Vec::<ApiContext>::new();
            let mut roots = Vec::new();
            let mut commits = std::collections::BTreeMap::new();
            let mut failures = 0;
            for event in ring.events() {
                assert!(event.api.method.is_some());
                if event.kind.category() == Category::Api {
                    assert_eq!(event.attempt, 0);
                    if event.kind == EventKind::ApiBegin {
                        if let Some(parent) = stack.last() {
                            assert_eq!(event.api.parent_span, parent.span);
                            assert_eq!(event.api.operation, parent.operation);
                        } else {
                            assert_eq!(event.api.parent_span, 0);
                            assert_eq!(event.api.operation, event.api.span);
                            roots.push(event.api.operation);
                        }
                        stack.push(event.api);
                    } else {
                        assert_eq!(stack.pop(), Some(event.api));
                        assert_ne!(event.kind, EventKind::ApiUnwound);
                        if event.kind == EventKind::ApiFailed {
                            failures += 1;
                        }
                    }
                } else {
                    assert_eq!(stack.last(), Some(&event.api));
                    assert!(event.attempt > 0);
                    if let Some(previous) = commits.insert(event.attempt, event.api.operation) {
                        assert_eq!(previous, event.api.operation);
                    }
                }
            }
            assert!(stack.is_empty());
            assert_eq!(roots.len(), 3);
            assert!(roots.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(failures > 0);
            // Non-empty files flush data, metadata, then the checkpoint.
            if failure == Some(2) {
                assert!(ring
                    .events()
                    .any(|e| e.kind == EventKind::ApiFailed && e.requires_remount));
            }
            let plain = plain.into_device();
            let observed = observed.into_device();
            assert_eq!(plain.events(), observed.events());
            let plain = plain.into_inner().into_inner();
            let observed = observed.into_inner().into_inner();
            for lba in 0..256 {
                assert_eq!(plain.peek(lba), observed.peek(lba));
            }
        }
    }
}

#[test]
fn api_guard_restores_context_after_a_provider_unwind_before_writes() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct PanicRead {
        inner: MemoryBackend,
        armed: Arc<AtomicBool>,
    }
    impl BlockDevice for PanicRead {
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }
        fn total_blocks(&self) -> u64 {
            self.inner.total_blocks()
        }
        fn read_block(&mut self, lba: u64, bytes: &mut [u8]) -> Result<(), BlockError> {
            assert!(
                !self.armed.swap(false, Ordering::SeqCst),
                "injected pre-write provider unwind"
            );
            self.inner.read_block(lba, bytes)
        }
        fn write_block(&mut self, lba: u64, bytes: &[u8]) -> Result<(), BlockError> {
            self.inner.write_block(lba, bytes)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            self.inner.flush()
        }
    }
    let original = image();
    let armed = Arc::new(AtomicBool::new(false));
    let mut volume = mount(PanicRead {
        inner: original.clone(),
        armed: armed.clone(),
    })
    .unwrap();
    let mut ring = recorder(256);
    ring.enable_api_observation();
    volume.replace_flight_recorder(Some(ring));
    armed.store(true, Ordering::SeqCst);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        volume.create_file_in_root("first", b"data", Timespec::default())
    }));
    assert!(failed.is_err());
    for lba in 0..256 {
        assert_eq!(volume.device_mut().inner.peek(lba), original.peek(lba));
    }
    let mut ring = volume.replace_flight_recorder(None).unwrap();
    let events: Vec<_> = ring.drain().collect();
    assert_eq!(events.last().unwrap().kind, EventKind::ApiUnwound);
    assert!(
        events
            .iter()
            .filter(|e| e.kind == EventKind::ApiUnwound)
            .count()
            >= 2
    );
    let previous_root = events[0].api.operation;
    volume.replace_flight_recorder(Some(ring));
    volume
        .create_file_in_root("second", b"data", Timespec::default())
        .unwrap();
    let first = volume.flight_recorder().unwrap().events().next().unwrap();
    assert_eq!(first.kind, EventKind::ApiBegin);
    assert_eq!(first.api.parent_span, 0);
    assert!(first.api.operation > previous_root);
}

#[test]
fn api_snapshot_and_metadata_calls_preserve_captured_state_and_busy_refusals() {
    use afsplus_core::flight::ApiMethod;
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MountOptions};
    for pages in [2, 4, 8, usize::MAX] {
        let mut original = MemoryBackend::new(4096, 2048);
        mkfs_with_options(
            &mut original,
            &MkfsParams {
                uuid: [74; 16],
                label: "API snapshots".into(),
                region_size: 2048,
                reclaim_caps: Default::default(),
                log_slots: 0,
                shared_extents: true,
                data_policy: true,
                name_policy: NamePolicy::Sensitive,
                timestamp: Timespec::default(),
            },
            MkfsOptions {
                persistent_snapshots: true,
            },
        )
        .unwrap();
        let mut devices = Vec::new();
        for observed in [false, true] {
            let mut volume = mount_with_snapshot_limits(
                TraceBackend::new(original.clone()),
                MountOptions {
                    tree_cache_pages: NonZeroUsize::new(pages),
                    ..Default::default()
                },
                SnapshotWorkLimits {
                    max_edit_records: 4096,
                    max_views: 16,
                    reclaim_records: 8,
                },
            )
            .unwrap();
            if observed {
                let mut ring = recorder(1024);
                ring.enable_api_observation();
                volume.replace_flight_recorder(Some(ring));
            }
            let now = Timespec::default();
            let file = volume
                .create_file_in_root("captured", b"data", now)
                .unwrap();
            volume.set_object_protection(file, 7, now).unwrap();
            let snapshot = volume.snapshot_create(now).unwrap();
            let handle = volume.snapshot_open(snapshot).unwrap();
            volume.set_object_protection(file, 9, now).unwrap();
            assert_eq!(
                volume
                    .snapshot_stat(&handle, file)
                    .unwrap()
                    .unwrap()
                    .protection,
                7
            );
            assert!(matches!(
                volume.snapshot_delete(snapshot, now),
                Err(afsplus_core::CoreError::Busy)
            ));
            drop(handle);
            volume.snapshot_delete(snapshot, now).unwrap();
            if observed {
                let ring = volume.replace_flight_recorder(None).unwrap();
                assert_eq!((ring.dropped(), ring.filtered()), (0, 0));
                for method in [
                    ApiMethod::SetObjectProtection,
                    ApiMethod::SnapshotCreate,
                    ApiMethod::SnapshotOpen,
                    ApiMethod::SnapshotStat,
                    ApiMethod::SnapshotDelete,
                ] {
                    assert!(
                        ring.events()
                            .any(|e| e.api.method == Some(method)
                                && e.kind == EventKind::ApiSucceeded)
                    );
                }
                assert!(ring
                    .events()
                    .any(|e| e.api.method == Some(ApiMethod::SnapshotDelete)
                        && e.kind == EventKind::ApiFailed));
            }
            devices.push(volume.into_device());
        }
        assert_eq!(devices[0].events(), devices[1].events());
        for lba in 0..2048 {
            assert_eq!(devices[0].inner().peek(lba), devices[1].inner().peek(lba));
        }
    }
}
