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

fn window_image(snapshots: bool) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 512);
    afsplus_core::mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [75; 16],
            label: "Window flight".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: Timespec::default(),
        },
        afsplus_core::MkfsOptions {
            persistent_snapshots: snapshots,
        },
    )
    .unwrap();
    dev
}

#[test]
fn deferred_windows_join_api_calls_groups_and_commits_without_changing_io() {
    use afsplus_core::flight::{Category, Event};
    use afsplus_core::volume::BatchOp;
    use afsplus_core::{mount_with_options, MountOptions, Volume};
    use afsplus_format::OBJECT_ROOT;
    fn run(v: &mut Volume<TraceBackend<FaultBackend<MemoryBackend>>>) -> (Vec<String>, Vec<Event>) {
        let mut results = Vec::new();
        let mut events = Vec::new();
        macro_rules! operation {
            ($call:expr) => {{
                let result = $call;
                results.push(format!("{result:?}"));
                if let Some(ring) = v.flight_recorder_mut() {
                    events.extend(ring.drain());
                }
                result
            }};
        }
        let now = Timespec::default();
        let file = operation!(v.window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "first",
                content: b"first data",
            },
            now
        ))
        .unwrap()
        .unwrap();
        let _ = operation!(v.window_op(
            &BatchOp::DeleteFile {
                parent_id: file,
                name: "child"
            },
            now
        ));
        let _ = operation!(v.window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "second",
                content: b"second data",
            },
            now
        ));
        let _ = operation!(v.window_fsync());
        let _ = operation!(v.window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "third",
                content: b"third data",
            },
            now
        ));
        let _ = operation!(v.window_fsync());
        let _ = operation!(v.window_commit(now));
        let _ = operation!(v.create_file_in_root("after", b"direct", now));
        (results, events)
    }
    for pages in [2, 4, 8, usize::MAX] {
        for failure in [None, Some(0), Some(1), Some(2), Some(3)] {
            let original = window_image(false);
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
            let mut ring = recorder(2048);
            ring.enable_api_observation();
            observed.replace_flight_recorder(Some(ring));
            let (expected, _) = run(&mut plain);
            let (actual, events) = run(&mut observed);
            assert_eq!(actual, expected);
            let ring = observed.flight_recorder().unwrap();
            assert_eq!((ring.dropped(), ring.filtered()), (0, 0));
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.kind == EventKind::WindowOpened)
                    .count(),
                1
            );
            assert!(!events.iter().any(|e| matches!(
                e.kind,
                EventKind::WindowAttached | EventKind::WindowDetached
            )));
            let windows: std::collections::BTreeSet<_> = events
                .iter()
                .filter(|e| e.window != 0)
                .map(|e| e.window)
                .collect();
            assert_eq!(windows, [1].into());
            let roots: std::collections::BTreeSet<_> = events
                .iter()
                .filter(|e| e.window == 1)
                .map(|e| e.api.operation)
                .collect();
            assert!(roots.len() >= 4);
            assert!(events
                .iter()
                .filter(|e| e.kind.category() == Category::Window)
                .all(|e| e.attempt == 0));
            if failure.is_none() {
                let groups: Vec<_> = events
                    .iter()
                    .filter(|e| e.kind == EventKind::WindowLogDurable)
                    .map(|e| e.log_sequence)
                    .collect();
                assert_eq!(groups, [1, 2]);
                let commits: Vec<_> = events
                    .iter()
                    .filter(|e| e.kind == EventKind::Begin)
                    .map(|e| (e.window, e.log_sequence))
                    .collect();
                assert_eq!(commits, [(1, 2), (0, 0)]);
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e.kind == EventKind::WindowClosed)
                        .count(),
                    1
                );
            } else {
                assert!(events.iter().any(|e| matches!(
                    e.kind,
                    EventKind::WindowFailed | EventKind::WindowLogFailed
                ) && e.requires_remount));
            }
            let plain = plain.into_device();
            let observed = observed.into_device();
            assert_eq!(plain.events(), observed.events());
            let plain = plain.into_inner().into_inner();
            let observed = observed.into_inner().into_inner();
            for lba in 0..512 {
                assert_eq!(plain.peek(lba), observed.peek(lba));
            }
        }
    }
}

#[test]
fn draining_keeps_window_identity_but_reattachment_and_new_windows_do_not_reuse_it() {
    use afsplus_core::volume::BatchOp;
    use afsplus_format::OBJECT_ROOT;
    let mut volume = mount(window_image(false)).unwrap();
    let mut ring = recorder(512);
    ring.enable_api_observation();
    volume.replace_flight_recorder(Some(ring));
    let missing = BatchOp::DeleteFile {
        parent_id: OBJECT_ROOT,
        name: "missing",
    };
    assert!(volume.window_op(&missing, Timespec::default()).is_err());
    let first: Vec<_> = volume.flight_recorder_mut().unwrap().drain().collect();
    assert!(first
        .iter()
        .any(|e| e.kind == EventKind::WindowOpened && e.window == 1));
    let generation = volume.generation();
    volume.window_commit(Timespec::default()).unwrap();
    assert_eq!(volume.generation(), generation);
    assert!(volume.window_op(&missing, Timespec::default()).is_err());
    let second: Vec<_> = volume.flight_recorder_mut().unwrap().drain().collect();
    assert!(second
        .iter()
        .any(|e| e.kind == EventKind::WindowClosed && e.window == 1));
    assert!(second
        .iter()
        .any(|e| e.kind == EventKind::WindowOpened && e.window == 2));
    let mut ring = volume.replace_flight_recorder(None).unwrap();
    assert_eq!(
        ring.events().last().unwrap().kind,
        EventKind::WindowDetached
    );
    assert_eq!(ring.events().last().unwrap().window, 2);
    ring.drain().for_each(drop);
    volume.window_commit(Timespec::default()).unwrap();
    assert!(volume.window_op(&missing, Timespec::default()).is_err());
    volume.replace_flight_recorder(Some(ring));
    let attached = *volume.flight_recorder().unwrap().events().last().unwrap();
    assert_eq!(attached.kind, EventKind::WindowAttached);
    assert_eq!(attached.window, 3);
    assert_eq!(attached.log_sequence, 0);
    assert_eq!(attached.api.operation, 0);
    volume.window_commit(Timespec::default()).unwrap();
    assert!(volume
        .flight_recorder()
        .unwrap()
        .events()
        .any(|e| e.kind == EventKind::WindowClosed && e.window == 3));
}

#[test]
fn snapshot_publication_ends_the_window_before_its_separate_registry_commit() {
    use afsplus_core::volume::{BatchOp, SnapshotWorkLimits};
    use afsplus_core::{mount_with_snapshot_limits, MountOptions};
    use afsplus_format::OBJECT_ROOT;
    for pages in [2, 4, 8, usize::MAX] {
        let original = window_image(true);
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
                    max_views: 8,
                    reclaim_records: 8,
                },
            )
            .unwrap();
            if observed {
                let mut ring = recorder(2048);
                ring.enable_api_observation();
                volume.replace_flight_recorder(Some(ring));
            }
            let file = volume
                .window_op(
                    &BatchOp::CreateFile {
                        parent_id: OBJECT_ROOT,
                        name: "captured",
                        content: b"snapshot bytes",
                    },
                    Timespec::default(),
                )
                .unwrap()
                .unwrap();
            let id = volume.snapshot_create(Timespec::default()).unwrap();
            let handle = volume.snapshot_open(id).unwrap();
            let mut bytes = [0; 14];
            assert_eq!(
                volume
                    .snapshot_read_file_at(&handle, file, 0, &mut bytes)
                    .unwrap(),
                bytes.len()
            );
            assert_eq!(&bytes, b"snapshot bytes");
            if observed {
                let events: Vec<_> = volume.flight_recorder_mut().unwrap().drain().collect();
                let commits: Vec<_> = events
                    .iter()
                    .filter(|e| e.kind == EventKind::Begin)
                    .collect();
                assert_eq!(commits.len(), 2);
                assert_eq!((commits[0].window, commits[1].window), (1, 0));
                assert_eq!(commits[0].api.operation, commits[1].api.operation);
                let closed = events
                    .iter()
                    .find(|e| e.kind == EventKind::WindowClosed)
                    .unwrap();
                assert!(
                    commits[0].sequence < closed.sequence && closed.sequence < commits[1].sequence
                );
            }
            devices.push(volume.into_device());
        }
        assert_eq!(devices[0].events(), devices[1].events());
        for lba in 0..512 {
            assert_eq!(devices[0].inner().peek(lba), devices[1].inner().peek(lba));
        }
    }
}

#[test]
fn window_data_failures_distinguish_discarded_mutations_from_retained_fsync_state() {
    use afsplus_core::{mount_with_options, MountOptions};
    let mut formatted = mount(window_image(false)).unwrap();
    let file = formatted
        .create_file_in_root("existing", &[7; 5000], Timespec::default())
        .unwrap();
    let original = formatted.into_device();
    for pages in [2, 4, 8, usize::MAX] {
        for truncate in [false, true] {
            for failure in 0..4 {
                let plan = FaultPlan {
                    fail_write_index: (failure == 1).then_some(0),
                    fail_flush_index: match failure {
                        2 => Some(0),
                        3 => Some(1),
                        _ => None,
                    },
                    ..Default::default()
                };
                let mut outcomes = Vec::new();
                let mut devices = Vec::new();
                for observed in [false, true] {
                    let mut volume = mount_with_options(
                        TraceBackend::new(FaultBackend::new(original.clone(), plan)),
                        MountOptions {
                            tree_cache_pages: NonZeroUsize::new(pages),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    if observed {
                        let mut ring = recorder(2048);
                        ring.enable_api_observation();
                        volume.replace_flight_recorder(Some(ring));
                    }
                    let mutation = if truncate {
                        volume.window_truncate_file(file, 123, Timespec::default())
                    } else {
                        volume.window_write_file_at(file, 17, b"updated", Timespec::default())
                    };
                    assert_eq!(mutation.is_err(), failure == 1);
                    let sync = volume.window_fsync();
                    assert_eq!(sync.is_err(), failure != 0);
                    outcomes.push((format!("{mutation:?}"), format!("{sync:?}")));
                    if observed {
                        let events: Vec<_> =
                            volume.flight_recorder_mut().unwrap().drain().collect();
                        assert_eq!(
                            events
                                .iter()
                                .filter(|e| e.kind == EventKind::WindowOpened)
                                .count(),
                            1
                        );
                        assert_eq!(
                            events.iter().any(|e| e.kind == EventKind::WindowClosed),
                            failure == 1
                        );
                        assert_eq!(
                            events.iter().any(|e| e.kind == EventKind::WindowLogBegin),
                            failure == 0 || failure == 3
                        );
                        assert_eq!(
                            events.iter().any(|e| e.kind == EventKind::WindowLogDurable),
                            failure == 0
                        );
                        assert_eq!(
                            events.iter().any(|e| matches!(
                                e.kind,
                                EventKind::WindowFailed | EventKind::WindowLogFailed
                            ) && e.requires_remount),
                            failure != 0
                        );
                        let log_failure =
                            events.iter().find(|e| e.kind == EventKind::WindowLogFailed);
                        assert_eq!(log_failure.is_some(), failure == 3);
                        if let Some(event) = log_failure {
                            assert_eq!((event.window, event.log_sequence), (1, 1));
                            let returned = events
                                .iter()
                                .rev()
                                .find(|e| e.kind == EventKind::ApiFailed)
                                .unwrap();
                            assert_eq!((returned.window, returned.log_sequence), (1, 0));
                        }
                        assert_eq!(volume.flight_recorder().unwrap().dropped(), 0);
                    }
                    devices.push(volume.into_device());
                }
                assert_eq!(outcomes[0], outcomes[1]);
                assert_eq!(devices[0].events(), devices[1].events());
                let plain = devices.remove(0).into_inner().into_inner();
                let observed = devices.remove(0).into_inner().into_inner();
                for lba in 0..512 {
                    assert_eq!(plain.peek(lba), observed.peek(lba));
                }
            }
        }
    }
}

#[test]
fn enabling_observation_mid_window_attaches_before_the_next_read_api() {
    use afsplus_core::volume::BatchOp;
    use afsplus_format::OBJECT_ROOT;
    let mut volume = mount(window_image(false)).unwrap();
    assert!(volume
        .window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "absent",
            },
            Timespec::default()
        )
        .is_err());
    volume.replace_flight_recorder(Some(recorder(128)));
    assert_eq!(volume.flight_recorder().unwrap().events().len(), 0);
    volume
        .flight_recorder_mut()
        .unwrap()
        .enable_api_observation();
    assert!(volume.stat(OBJECT_ROOT).unwrap().is_some());
    let events: Vec<_> = volume.flight_recorder_mut().unwrap().drain().collect();
    assert_eq!(events[0].kind, EventKind::WindowAttached);
    assert_eq!((events[0].window, events[0].api.operation), (1, 0));
    assert_eq!(events[1].kind, EventKind::ApiBegin);
    assert_eq!(events[1].window, 1);
}

#[test]
fn publication_families_preserve_results_images_and_io_with_small_rings() {
    publication_family_equivalence(false);
}

#[test]
fn object_observation_preserves_publication_family_failures_and_images() {
    publication_family_equivalence(true);
}

fn publication_family_equivalence(observe_objects: bool) {
    use afsplus_core::flight::ApiMethod;
    use afsplus_core::{mount_with_options, MountOptions};
    use afsplus_format::OBJECT_ROOT;

    let now = Timespec::default();
    let mut fixture = mount(window_image(false)).unwrap();
    let source = fixture
        .create_file_in_root("source", b"original", now)
        .unwrap();
    let original = fixture.into_device();
    let methods = [
        ApiMethod::CreateDirectoryInRoot,
        ApiMethod::CreateSymlink,
        ApiMethod::LinkFile,
        ApiMethod::Rename,
        ApiMethod::CloneFile,
        ApiMethod::DeleteFileInRoot,
        ApiMethod::TruncateFile,
        ApiMethod::SetObjectProtection,
    ];
    for pages in [2, 4, 8, usize::MAX] {
        for (family, method) in methods.into_iter().enumerate() {
            for failure in [None, Some(0), Some(1)] {
                for capacity in [1, 1024] {
                    let mut devices = Vec::new();
                    let mut results = Vec::new();
                    for observed in [false, true] {
                        let mut volume = mount_with_options(
                            TraceBackend::new(FaultBackend::new(
                                original.clone(),
                                FaultPlan {
                                    fail_flush_index: failure,
                                    ..Default::default()
                                },
                            )),
                            MountOptions {
                                tree_cache_pages: NonZeroUsize::new(pages),
                                ..Default::default()
                            },
                        )
                        .unwrap();
                        if observed {
                            let mut ring = recorder(capacity);
                            ring.enable_api_observation();
                            if observe_objects {
                                ring.enable_object_observation();
                            }
                            volume.replace_flight_recorder(Some(ring));
                        }
                        let result = match family {
                            0 => volume.create_directory_in_root("directory", now).map(Some),
                            1 => volume
                                .create_symlink(OBJECT_ROOT, "symbolic", "source", now)
                                .map(Some),
                            2 => volume
                                .link_file(source, OBJECT_ROOT, "hard", now)
                                .map(|()| None),
                            3 => volume
                                .rename(OBJECT_ROOT, "source", OBJECT_ROOT, "renamed", now)
                                .map(|()| None),
                            4 => volume
                                .clone_file(source, OBJECT_ROOT, "clone", now)
                                .map(Some),
                            5 => volume.delete_file_in_root("source", now).map(|()| None),
                            6 => volume.truncate_file(source, 3, now).map(|()| None),
                            7 => volume.set_object_protection(source, 7, now).map(|()| None),
                            _ => unreachable!(),
                        };
                        assert_eq!(
                            result.is_ok(),
                            failure.is_none(),
                            "{method:?}, pages={pages}, failure={failure:?}"
                        );
                        results.push(format!("{result:?}"));
                        if observed {
                            let ring = volume.replace_flight_recorder(None).unwrap();
                            let last = ring.events().last().unwrap();
                            assert_eq!(last.api.method, Some(method));
                            assert_eq!(
                                last.kind,
                                if result.is_ok() {
                                    EventKind::ApiSucceeded
                                } else {
                                    EventKind::ApiFailed
                                }
                            );
                            if capacity == 1 {
                                assert!(ring.dropped() > 0);
                            } else {
                                assert_eq!(ring.dropped(), 0);
                                assert!(ring.events().any(|event| event.kind == EventKind::Begin));
                            }
                        }
                        devices.push(volume.into_device());
                    }
                    assert_eq!(results[0], results[1]);
                    assert_eq!(devices[0].events(), devices[1].events());
                    let images: Vec<_> = devices
                        .into_iter()
                        .map(|device| device.into_inner().into_inner())
                        .collect();
                    for lba in 0..original.total_blocks() {
                        assert_eq!(
                            images[0].peek(lba),
                            images[1].peek(lba),
                            "{method:?}, pages={pages}, failure={failure:?}, lba={lba}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn object_resolution_links_live_and_captured_blocks_without_extra_io() {
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mount_with_snapshot_limits, MountOptions};
    use afsplus_format::OBJECT_ROOT;
    for pages in [2, 4, 8, usize::MAX] {
        for capacity in [1, 2048] {
            let original = window_image(true);
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
                    let mut ring = recorder(capacity);
                    ring.enable_object_observation();
                    volume.replace_flight_recorder(Some(ring));
                }
                let now = Timespec::default();
                let file = volume.create_file_in_root("file", b"before", now).unwrap();
                let link = volume
                    .create_symlink(OBJECT_ROOT, "link", "file", now)
                    .unwrap();
                let snapshot = volume.snapshot_create(now).unwrap();
                let handle = volume.snapshot_open(snapshot).unwrap();
                volume.write_file_at(file, 0, b"after!", now).unwrap();
                assert!(volume.stat(file).unwrap().is_some());
                assert!(volume.stat(OBJECT_ROOT).unwrap().is_some());
                assert!(volume.stat(10_000).unwrap().is_none());
                assert!(volume.snapshot_stat(&handle, file).unwrap().is_some());
                assert!(volume
                    .snapshot_stat(&handle, OBJECT_ROOT)
                    .unwrap()
                    .is_some());
                assert!(volume.snapshot_stat(&handle, 10_000).unwrap().is_none());
                let mut captured = [0; 6];
                assert_eq!(
                    volume
                        .snapshot_read_file_at(&handle, file, 0, &mut captured)
                        .unwrap(),
                    6
                );
                assert_eq!(&captured, b"before");
                assert_eq!(volume.read_file(file).unwrap(), b"after!");
                assert_eq!(
                    volume
                        .snapshot_lookup(&handle, OBJECT_ROOT, "file")
                        .unwrap(),
                    Some(file)
                );
                let page = volume
                    .snapshot_read_directory_page(&handle, OBJECT_ROOT, None, 8)
                    .unwrap();
                assert!(page.eof);
                assert_eq!(
                    page.entries
                        .iter()
                        .map(|entry| (entry.name.clone(), entry.child_id))
                        .collect::<Vec<_>>(),
                    vec![(b"file".to_vec(), file), (b"link".to_vec(), link)]
                );
                let allocation = volume
                    .snapshot_allocation_page(&handle, file, 0, 64)
                    .unwrap();
                assert!(allocation.eof);
                assert_eq!(allocation.ranges.len(), 1);
                let mut target = [0; 4];
                assert_eq!(
                    volume
                        .snapshot_read_link(&handle, link, &mut target)
                        .unwrap(),
                    4
                );
                assert_eq!(&target, b"file");

                if observed {
                    let ring = volume.replace_flight_recorder(None).unwrap();
                    if capacity == 1 {
                        assert!(ring.dropped() > 0);
                    } else {
                        assert_eq!(ring.dropped(), 0);
                        let mut live = None;
                        let mut captured_block = None;
                        let mut missing_views = std::collections::BTreeSet::new();
                        let mut captured_methods = std::collections::BTreeSet::new();
                        for event in ring.events() {
                            if let Some(context) = event.object {
                                assert!(event.api.method.is_some());
                                assert_eq!(event.attempt, 0);
                                if context.view_id == snapshot {
                                    captured_methods.insert(event.api.method.unwrap() as u16);
                                }
                                match event.kind {
                                    EventKind::ObjectMapped => {
                                        assert_ne!(context.record_block, 0);
                                        if context.object_id == file {
                                            if context.view_id == 0 {
                                                live = Some(context.record_block);
                                            } else {
                                                assert_eq!(context.view_id, snapshot);
                                                assert_eq!(
                                                    event.generation,
                                                    handle.info().generation
                                                );
                                                captured_block = Some(context.record_block);
                                            }
                                        }
                                    }
                                    EventKind::ObjectLookup | EventKind::ObjectMissing => {
                                        assert_eq!(context.record_block, 0);
                                        if event.kind == EventKind::ObjectMissing {
                                            assert_eq!(context.object_id, 10_000);
                                            missing_views.insert(context.view_id);
                                        }
                                    }
                                    other => panic!("object context leaked to {other:?}"),
                                }
                            } else {
                                assert!(!matches!(
                                    event.kind,
                                    EventKind::ObjectLookup
                                        | EventKind::ObjectMapped
                                        | EventKind::ObjectMissing
                                ));
                            }
                        }
                        assert!(live.is_some() && captured_block.is_some());
                        assert_ne!(live, captured_block);
                        assert_eq!(missing_views, [0, snapshot].into_iter().collect());
                        for method in [
                            afsplus_core::flight::ApiMethod::SnapshotStat,
                            afsplus_core::flight::ApiMethod::SnapshotAllocationPage,
                            afsplus_core::flight::ApiMethod::SnapshotReadFileAt,
                            afsplus_core::flight::ApiMethod::SnapshotReadLink,
                            afsplus_core::flight::ApiMethod::SnapshotLookup,
                            afsplus_core::flight::ApiMethod::SnapshotReadDirectoryPage,
                        ] {
                            assert!(
                                captured_methods.contains(&(method as u16)),
                                "missing captured context for {method:?}"
                            );
                        }
                    }
                }
                devices.push(volume.into_device());
            }
            assert_eq!(devices[0].events(), devices[1].events());
            for lba in 0..original.total_blocks() {
                assert_eq!(devices[0].inner().peek(lba), devices[1].inner().peek(lba));
            }
        }
    }
}

struct LookupReadFault {
    inner: MemoryBackend,
    fail_read: u64,
    reads: u64,
    tripped: bool,
}
impl BlockDevice for LookupReadFault {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, bytes: &mut [u8]) -> Result<(), BlockError> {
        let index = self.reads;
        self.reads += 1;
        if index == self.fail_read {
            self.tripped = true;
            return Err(BlockError::Injected("object lookup read"));
        }
        self.inner.read_block(lba, bytes)
    }
    fn write_block(&mut self, lba: u64, bytes: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, bytes)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

#[test]
fn failed_object_lookup_is_not_reported_as_missing_and_retry_keeps_its_identity() {
    use afsplus_core::{mount_with_options, MountOptions};
    let mut fixture = mount(image()).unwrap();
    let file = fixture
        .create_file_in_root("file", b"stable", Timespec::default())
        .unwrap();
    let original = fixture.into_device();
    for pages in [2, 4, 8, usize::MAX] {
        let options = MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        };
        let reference = mount_with_options(TraceBackend::new(original.clone()), options).unwrap();
        let mount_reads = reference.into_device().stats().reads;
        let mut devices = Vec::new();
        let mut errors = Vec::new();
        for observed in [false, true] {
            let mut volume = mount_with_options(
                TraceBackend::new(LookupReadFault {
                    inner: original.clone(),
                    fail_read: mount_reads,
                    reads: 0,
                    tripped: false,
                }),
                options,
            )
            .unwrap();
            if observed {
                let mut ring = recorder(128);
                ring.enable_object_observation();
                volume.replace_flight_recorder(Some(ring));
            }
            let error = volume.stat(file).unwrap_err();
            errors.push(format!("{error:?}"));
            assert!(volume.device_mut().inner().tripped);
            let failed_root = if observed {
                let ring = volume.flight_recorder_mut().unwrap();
                let events: Vec<_> = ring.drain().collect();
                assert_eq!(events.last().unwrap().kind, EventKind::ApiFailed);
                let objects: Vec<_> = events
                    .iter()
                    .filter(|event| event.object.is_some())
                    .collect();
                assert_eq!(objects.len(), 1);
                assert_eq!(objects[0].kind, EventKind::ObjectLookup);
                assert_eq!(objects[0].object.unwrap().object_id, file);
                assert_eq!(objects[0].object.unwrap().record_block, 0);
                objects[0].api.operation
            } else {
                0
            };
            assert_eq!(volume.stat(file).unwrap().unwrap().object_id, file);
            if observed {
                let ring = volume.replace_flight_recorder(None).unwrap();
                assert_eq!(ring.dropped(), 0);
                let mapped = ring
                    .events()
                    .find(|event| event.kind == EventKind::ObjectMapped)
                    .unwrap();
                assert!(mapped.api.operation > failed_root);
                assert_eq!(mapped.object.unwrap().object_id, file);
                assert_ne!(mapped.object.unwrap().record_block, 0);
                assert_eq!(ring.events().last().unwrap().kind, EventKind::ApiSucceeded);
            }
            devices.push(volume.into_device());
        }
        assert_eq!(errors[0], errors[1]);
        assert_eq!(devices[0].events(), devices[1].events());
        let images: Vec<_> = devices
            .into_iter()
            .map(|device| device.into_inner().inner)
            .collect();
        for lba in 0..original.total_blocks() {
            assert_eq!(images[0].peek(lba), images[1].peek(lba));
        }
    }
}
