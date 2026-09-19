use afsplus_block::{
    BlockDevice, BlockError, FaultBackend, FaultPlan, MemoryBackend, TraceBackend,
};
use afsplus_core::flight::{EventKind, FlightRecorder};
use afsplus_core::{
    mkfs, mount, AttributeWriteMode, MkfsParams, NamePolicy, SecurityProjectionPolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
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

struct PanicRead {
    inner: MemoryBackend,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
            !self.armed.swap(false, std::sync::atomic::Ordering::SeqCst),
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

#[test]
fn api_guard_restores_context_after_a_provider_unwind_before_writes() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
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
    let first = *volume.flight_recorder().unwrap().events().next().unwrap();
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
                if let Some(mut ring) = v.flight_recorder_mut() {
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
            drop(ring);
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
                let mut ring = volume.flight_recorder_mut().unwrap();
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

#[test]
fn allocator_observation_preserves_four_profile_io_and_orders_preparation() {
    for pages in [2, 4, 8, usize::MAX] {
        let options = afsplus_core::MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        };
        let base = image();
        let mut devices = Vec::new();
        for enabled in [false, true] {
            let mut volume =
                afsplus_core::mount_with_options(TraceBackend::new(base.clone()), options).unwrap();
            if enabled {
                let mut ring = recorder(4096);
                ring.enable_subsystem_observation();
                volume.replace_flight_recorder(Some(ring));
            }
            let id = volume
                .create_file_in_root("allocated", b"old bytes", Timespec::default())
                .unwrap();
            volume
                .write_file_at(id, 0, b"new", Timespec::default())
                .unwrap();
            assert_eq!(volume.read_file(id).unwrap(), b"new bytes");
            volume
                .delete_file_in_root("allocated", Timespec::default())
                .unwrap();
            assert!(volume.list_root().unwrap().is_empty());
            if enabled {
                let ring = volume.replace_flight_recorder(None).unwrap();
                assert_eq!(ring.dropped(), 0);
                let events: Vec<_> = ring.events().copied().collect();
                let begin = events
                    .iter()
                    .position(|e| e.kind == EventKind::AllocationBegin)
                    .unwrap();
                let publication = events
                    .iter()
                    .position(|e| e.kind == EventKind::Begin)
                    .unwrap();
                assert!(begin < publication);
                assert!(events.iter().any(|e| e.kind == EventKind::AllocationGranted
                    && e.allocation.is_some_and(|a| a.start != 0 && a.blocks != 0)));
                assert!(events
                    .iter()
                    .any(|e| e.kind == EventKind::AllocationRetired));
                for event in events.iter().filter(|e| e.allocation.is_some()) {
                    assert_ne!(event.api.operation, 0);
                    assert_eq!(event.attempt, 0);
                }
            }
            devices.push(volume.into_device());
        }
        assert_eq!(devices[0].events(), devices[1].events());
        for lba in 0..256 {
            assert_eq!(devices[0].inner().peek(lba), devices[1].inner().peek(lba));
        }
    }
}

#[test]
fn allocator_observer_follows_recorder_replacement_in_open_window() {
    use afsplus_core::volume::BatchOp;
    use afsplus_format::OBJECT_ROOT;
    let mut volume = mount(window_image(false)).unwrap();
    let mut first = recorder(512);
    first.enable_subsystem_observation();
    volume.replace_flight_recorder(Some(first));
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "first",
                content: b"one",
            },
            Timespec::default(),
        )
        .unwrap();
    let mut second = recorder(512);
    second.enable_subsystem_observation();
    let first = volume.replace_flight_recorder(Some(second)).unwrap();
    let detached_sequence = first.sequence();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "second",
                content: b"two",
            },
            Timespec::default(),
        )
        .unwrap();
    volume.window_commit(Timespec::default()).unwrap();
    let second = volume.replace_flight_recorder(None).unwrap();
    assert_eq!(first.sequence(), detached_sequence);
    assert!(second
        .events()
        .any(|event| event.kind == EventKind::AllocationGranted));
    let first_id = volume.lookup_root("first").unwrap().unwrap();
    let second_id = volume.lookup_root("second").unwrap().unwrap();
    assert_eq!(volume.read_file(first_id).unwrap(), b"one");
    assert_eq!(volume.read_file(second_id).unwrap(), b"two");
}

#[test]
fn tree_spill_observation_preserves_bounded_cache_io_and_images() {
    use afsplus_core::flight::{Categories, Category};
    use afsplus_core::volume::BatchOp;
    let names: Vec<_> = (0..192)
        .map(|i| format!("{i:04}-{}", "n".repeat(180)))
        .collect();
    let ops: Vec<_> = names
        .iter()
        .map(|name| BatchOp::CreateFile {
            parent_id: 1,
            name,
            content: b"payload",
        })
        .collect();
    let mut base = MemoryBackend::new(4096, 4096);
    mkfs(
        &mut base,
        &MkfsParams {
            uuid: [0x41; 16],
            label: "tree flight".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: false,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    for pages in [2, 4, 8, usize::MAX] {
        let options = afsplus_core::MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        };
        let mut devices = Vec::new();
        for enabled in [false, true] {
            let mut volume =
                afsplus_core::mount_with_options(TraceBackend::new(base.clone()), options).unwrap();
            if enabled {
                let mut ring = recorder(32768);
                ring.enable_subsystem_observation();
                ring.set_categories(Categories::NONE.with(Category::Tree));
                volume.replace_flight_recorder(Some(ring));
            }
            volume.run_batch(&ops, Timespec::default()).unwrap();
            let stats = volume.last_commit_stats().unwrap().tree_mutations;
            if pages != usize::MAX {
                assert!(stats.staged_spill_writes > 31);
                assert!(stats.max_resident_staged_nodes <= pages as u64);
            }
            if enabled {
                let ring = volume.replace_flight_recorder(None).unwrap();
                assert_eq!(ring.dropped(), 0);
                let events: Vec<_> = ring.events().copied().collect();
                assert!(events
                    .iter()
                    .all(|e| e.tree.is_some() && e.attempt == 0 && e.api.operation != 0));
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e.kind == EventKind::TreeSpillComplete)
                        .count() as u64,
                    stats.staged_spill_writes
                );
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e.kind == EventKind::TreeSpillBegin)
                        .count() as u64,
                    stats.staged_spill_writes
                );
                assert!(events.iter().any(|e| e.kind == EventKind::TreeReadComplete));
            }
            for name in &names {
                let id = volume.lookup_root(name).unwrap().unwrap();
                assert_eq!(volume.read_file(id).unwrap(), b"payload");
            }
            devices.push(volume.into_device());
        }
        assert_eq!(devices[0].events(), devices[1].events());
        for lba in 0..4096 {
            assert_eq!(devices[0].inner().peek(lba), devices[1].inner().peek(lba));
        }
    }
}

#[test]
fn recorder_preserves_transferable_volume() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<afsplus_core::volume::Volume<afsplus_block::MemoryBackend>>();
}

#[test]
fn captured_provider_unwind_restores_observer_and_allows_retry() {
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mkfs_with_options, mount_with_snapshot_limits, MkfsOptions, MountOptions};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let mut original = MemoryBackend::new(4096, 2048);
    mkfs_with_options(
        &mut original,
        &MkfsParams {
            uuid: [93; 16],
            label: "observer unwind".into(),
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
    let armed = Arc::new(AtomicBool::new(false));
    let mut volume = mount_with_snapshot_limits(
        PanicRead {
            inner: original,
            armed: armed.clone(),
        },
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 16,
            reclaim_records: 8,
        },
    )
    .unwrap();
    let now = Timespec::default();
    let file = volume
        .create_file_in_root("captured", b"saved", now)
        .unwrap();
    let snapshot = volume.snapshot_create(now).unwrap();
    let handle = volume.snapshot_open(snapshot).unwrap();
    let before = volume.device_mut().inner.clone();
    let mut ring = recorder(128);
    ring.enable_object_observation();
    ring.enable_subsystem_observation();
    volume.replace_flight_recorder(Some(ring));
    armed.store(true, Ordering::SeqCst);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        volume.snapshot_stat(&handle, file)
    }))
    .is_err());
    let failed_operation = {
        let ring = volume.flight_recorder().unwrap();
        let last = ring.events().last().unwrap();
        assert_eq!(last.kind, EventKind::ApiUnwound);
        last.api.operation
    };
    assert!(volume.snapshot_stat(&handle, file).unwrap().is_some());
    let ring = volume.replace_flight_recorder(None).unwrap();
    assert!(ring.events().any(
        |event| event.kind == EventKind::ApiSucceeded && event.api.operation > failed_operation
    ));
    volume.replace_flight_recorder(Some(ring));
    let a = volume.flight_recorder().unwrap();
    let b = volume.flight_recorder().unwrap();
    assert_eq!(a.events().count(), b.events().count());
    drop((a, b));
    for lba in 0..2048 {
        assert_eq!(volume.device_mut().inner.peek(lba), before.peek(lba));
    }
}

// ---------------------------------------------------------------------------
// Mount, format and verification observation.
// ---------------------------------------------------------------------------

/// A device whose block trace and image outlive it, so a refused mount or an
/// interrupted format can be compared with the run that was not observed.
/// It injects its own faults, keeping the failing operation in the trace.
#[derive(Clone)]
struct SharedDevice {
    image: std::sync::Arc<std::sync::Mutex<MemoryBackend>>,
    log: std::sync::Arc<std::sync::Mutex<Vec<(char, u64)>>>,
    fail_write: Option<usize>,
    fail_flush: Option<usize>,
    writes: usize,
    flushes: usize,
}

impl SharedDevice {
    fn new(image: &MemoryBackend) -> Self {
        SharedDevice {
            image: std::sync::Arc::new(std::sync::Mutex::new(image.clone())),
            log: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            fail_write: None,
            fail_flush: None,
            writes: 0,
            flushes: 0,
        }
    }

    fn failing(
        image: &MemoryBackend,
        fail_write: Option<usize>,
        fail_flush: Option<usize>,
    ) -> Self {
        SharedDevice {
            fail_write,
            fail_flush,
            ..SharedDevice::new(image)
        }
    }

    /// A second handle on the same image and trace, performing no I/O itself.
    /// Arm a fault at an operation counted from the next one, so preparation
    /// stays clean and only the observed family meets the fault.
    fn arm(&mut self, write: Option<usize>, flush: Option<usize>) {
        self.fail_write = write.map(|index| self.writes + index);
        self.fail_flush = flush.map(|index| self.flushes + index);
    }

    fn handle(&self) -> Self {
        SharedDevice {
            fail_write: None,
            fail_flush: None,
            writes: 0,
            flushes: 0,
            ..self.clone()
        }
    }

    fn trace(&self) -> Vec<(char, u64)> {
        self.log.lock().unwrap().clone()
    }

    fn image(&self) -> MemoryBackend {
        self.image.lock().unwrap().clone()
    }

    fn blocks(&self) -> Vec<Vec<u8>> {
        let image = self.image.lock().unwrap();
        (0..image.total_blocks())
            .map(|lba| image.peek(lba))
            .collect()
    }

    fn record(&self, operation: char, lba: u64) {
        self.log.lock().unwrap().push((operation, lba));
    }
}

impl BlockDevice for SharedDevice {
    fn block_size(&self) -> usize {
        self.image.lock().unwrap().block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.image.lock().unwrap().total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.record('r', lba);
        self.image.lock().unwrap().read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, buf: &[u8]) -> Result<(), BlockError> {
        self.record('w', lba);
        let index = self.writes;
        self.writes += 1;
        if self.fail_write == Some(index) {
            return Err(BlockError::Injected("interrupted format write"));
        }
        self.image.lock().unwrap().write_block(lba, buf)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.record('f', 0);
        let index = self.flushes;
        self.flushes += 1;
        if self.fail_flush == Some(index) {
            return Err(BlockError::Injected("interrupted format barrier"));
        }
        self.image.lock().unwrap().flush()
    }
}

fn log_image() -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 512);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x41; 16],
            label: "recovery".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: false,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    dev
}

/// Two durable intent groups with no checkpoint publishing them.
fn pending_log_image() -> MemoryBackend {
    use afsplus_core::volume::BatchOp;
    use afsplus_format::OBJECT_ROOT;
    let mut volume = mount(log_image()).unwrap();
    for name in ["first", "second"] {
        volume
            .window_op(
                &BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name,
                    content: b"logged",
                },
                Timespec::default(),
            )
            .unwrap();
        volume.window_fsync().unwrap();
    }
    volume.into_device()
}

fn log_slot_lbas(image: &MemoryBackend) -> Vec<u64> {
    use afsplus_core::{mount_with_options, MountMode, MountOptions};
    let volume = mount_with_options(
        image.clone(),
        MountOptions {
            mode: MountMode::NoChanges,
            ..Default::default()
        },
    )
    .unwrap();
    let ident = volume.ident();
    afsplus_core::intent_log::log_slot_lbas(&ident.geometry(), ident.log_slots).unwrap()
}

/// A durable first group followed by a nonzero unreadable second slot.
fn damaged_log_image() -> MemoryBackend {
    let mut image = pending_log_image();
    let slots = log_slot_lbas(&image);
    image.apply_raw(slots[1], &vec![0x5a; 4096]);
    image
}

/// Both slots carry a structurally valid checkpoint of the same generation.
fn ambiguous_image() -> MemoryBackend {
    let mut image = image();
    let slot_a = image.peek(1);
    image.apply_raw(2, &slot_a);
    image
}

fn describe_mount<D: BlockDevice>(
    result: &Result<afsplus_core::Volume<D>, afsplus_core::CoreError>,
) -> String {
    match result {
        Ok(volume) => format!(
            "generation {} pending {} mode {:?}",
            volume.generation(),
            volume.pending_intent_records(),
            volume.mount_mode()
        ),
        Err(error) => format!("refused {error}"),
    }
}

fn mount_context(event: &afsplus_core::flight::Event) -> afsplus_core::flight::MountContext {
    match event.lifecycle {
        Some(afsplus_core::flight::LifecycleContext::Mount(context)) => context,
        other => panic!("expected a mount context, found {other:?}"),
    }
}

#[test]
fn mount_observation_preserves_results_traces_and_images_across_profiles() {
    use afsplus_core::flight::{Category, MountStage};
    use afsplus_core::{mount_observed, mount_with_options, MountMode, MountOptions};
    for pages in [2, 4, 8, usize::MAX] {
        let cases: Vec<(&str, MemoryBackend, MountMode)> = vec![
            ("clean", image(), MountMode::ReadWrite),
            ("replay", pending_log_image(), MountMode::ReadWrite),
            ("inspect", pending_log_image(), MountMode::NoChanges),
            ("damaged", damaged_log_image(), MountMode::ReadWrite),
            ("ambiguous", ambiguous_image(), MountMode::ReadWrite),
            ("unsupported", window_image(true), MountMode::ReadWrite),
        ];
        for (name, base, mode) in cases {
            let options = MountOptions {
                mode,
                tree_cache_pages: NonZeroUsize::new(pages),
            };
            let plain_device = SharedDevice::new(&base);
            let observed_device = SharedDevice::new(&base);
            let plain_handle = plain_device.handle();
            let observed_handle = observed_device.handle();
            let plain = mount_with_options(plain_device, options);
            let observed = mount_observed(observed_device, options, recorder(512));
            let (events, actual) = match observed {
                Ok(mut volume) => {
                    let ring = volume.replace_flight_recorder(None).unwrap();
                    let described = describe_mount(&Ok(volume));
                    assert_eq!(ring.filtered(), 0, "{name}");
                    assert_eq!(ring.dropped(), 0, "{name}");
                    (ring.events().copied().collect::<Vec<_>>(), described)
                }
                Err(refused) => (
                    refused.recorder.events().copied().collect(),
                    describe_mount::<SharedDevice>(&Err(refused.error)),
                ),
            };
            assert_eq!(actual, describe_mount(&plain), "{name} at {pages} pages");
            assert_eq!(
                observed_handle.trace(),
                plain_handle.trace(),
                "{name} at {pages} pages"
            );
            assert_eq!(
                observed_handle.blocks(),
                plain_handle.blocks(),
                "{name} at {pages} pages"
            );

            let mount_events: Vec<_> = events
                .iter()
                .filter(|event| event.kind.category() == Category::Mount)
                .collect();
            assert!(
                mount_events.iter().all(|event| event.attempt == 0),
                "{name}: mount events carry no commit attempt"
            );
            let kinds: Vec<_> = mount_events.iter().map(|event| event.kind).collect();
            assert_eq!(mount_events[0].kind, EventKind::MountBegin, "{name}");
            assert_eq!(mount_events[0].generation, 0, "{name}");
            assert_eq!(
                mount_context(mount_events[0]).stage,
                MountStage::Identification,
                "{name}"
            );
            assert!(
                mount_events
                    .iter()
                    .all(|event| mount_context(event).mode == mode),
                "{name}"
            );
            match name {
                "clean" => {
                    assert_eq!(
                        kinds,
                        [
                            EventKind::MountBegin,
                            EventKind::MountSelected,
                            EventKind::MountComplete
                        ]
                    );
                    let selected = mount_context(mount_events[1]);
                    assert_eq!(mount_events[1].generation, 1);
                    assert_eq!((selected.slot, selected.other_generation), (0, 0));
                    assert_eq!(mount_context(mount_events[2]).count, 0);
                }
                "replay" => {
                    assert_eq!(
                        kinds,
                        [
                            EventKind::MountBegin,
                            EventKind::MountSelected,
                            EventKind::MountIntentBegin,
                            EventKind::MountIntentScanned,
                            EventKind::MountIntentReplayed,
                            EventKind::MountIntentReplayed,
                            EventKind::MountComplete
                        ]
                    );
                    assert_eq!(mount_context(mount_events[2]).count, 8, "log slots");
                    let scanned = mount_context(mount_events[3]);
                    assert_eq!((scanned.count, scanned.damaged_tail), (2, false));
                    assert_eq!(mount_events[3].log_sequence, 2);
                    let groups: Vec<_> = mount_events[4..6]
                        .iter()
                        .map(|event| (event.log_sequence, mount_context(event).count))
                        .collect();
                    assert_eq!(groups, [(1, 1), (2, 1)]);
                    assert_eq!(mount_context(mount_events[6]).count, 2);
                    assert!(
                        mount_events[6].generation > mount_events[1].generation,
                        "replay publishes a checkpoint"
                    );
                    assert!(events
                        .iter()
                        .any(|event| event.kind == EventKind::CheckpointDurable));
                }
                "inspect" => {
                    assert_eq!(
                        kinds,
                        [
                            EventKind::MountBegin,
                            EventKind::MountSelected,
                            EventKind::MountIntentBegin,
                            EventKind::MountIntentScanned,
                            EventKind::MountComplete
                        ]
                    );
                    assert_eq!(mount_context(mount_events[4]).count, 2, "pending records");
                    assert_eq!(
                        mount_events[4].generation, mount_events[1].generation,
                        "inspection publishes nothing"
                    );
                    assert!(!events
                        .iter()
                        .any(|event| event.kind == EventKind::CheckpointDurable));
                }
                "damaged" => {
                    let scanned = mount_context(mount_events[3]);
                    assert_eq!(mount_events[3].kind, EventKind::MountIntentScanned);
                    assert_eq!((scanned.count, scanned.damaged_tail), (1, true));
                    assert_eq!(
                        kinds
                            .iter()
                            .filter(|kind| **kind == EventKind::MountIntentReplayed)
                            .count(),
                        1
                    );
                    assert_eq!(mount_context(mount_events[5]).count, 1);
                }
                "ambiguous" => {
                    assert_eq!(kinds, [EventKind::MountBegin, EventKind::MountFailed]);
                    assert_eq!(mount_context(mount_events[1]).stage, MountStage::Selection);
                }
                "unsupported" => {
                    assert_eq!(kinds, [EventKind::MountBegin, EventKind::MountFailed]);
                    assert_eq!(
                        mount_context(mount_events[1]).stage,
                        MountStage::Negotiation
                    );
                }
                other => panic!("unnamed mount case {other}"),
            }
        }
    }
}

#[test]
fn small_mount_rings_report_loss_without_changing_recovery() {
    use afsplus_core::flight::Category;
    use afsplus_core::{mount_observed, MountOptions};
    let base = pending_log_image();
    let full = {
        let mut volume =
            mount_observed(base.clone(), MountOptions::default(), recorder(512)).unwrap();
        let ring = volume.replace_flight_recorder(None).unwrap();
        assert_eq!((ring.dropped(), ring.filtered()), (0, 0));
        ring.sequence()
    };
    for capacity in [1, 2, 4] {
        let mut volume =
            mount_observed(base.clone(), MountOptions::default(), recorder(capacity)).unwrap();
        assert_eq!(volume.list_root().unwrap().len(), 2);
        let ring = volume.replace_flight_recorder(None).unwrap();
        assert_eq!(
            ring.sequence(),
            full,
            "identities are independent of capacity"
        );
        assert_eq!(ring.events().len(), capacity);
        assert_eq!(ring.dropped(), full - capacity as u64);
        assert_eq!(ring.filtered(), 0);
        assert!(ring
            .events()
            .any(|event| event.kind.category() == Category::Mount));
    }
}

#[test]
fn mount_category_selection_filters_after_identity_assignment() {
    use afsplus_core::flight::{Categories, Category};
    use afsplus_core::{mount_observed, MountOptions};
    let base = pending_log_image();
    let mut ring = recorder(512);
    ring.set_categories(Categories::NONE.with(Category::Mount));
    let mut volume = mount_observed(base.clone(), MountOptions::default(), ring).unwrap();
    let selected = volume.replace_flight_recorder(None).unwrap();

    let mut volume_all = mount_observed(base, MountOptions::default(), recorder(512)).unwrap();
    let all = volume_all.replace_flight_recorder(None).unwrap();
    assert_eq!(selected.sequence(), all.sequence());
    assert_eq!(selected.dropped(), 0);
    assert_eq!(
        selected.filtered(),
        all.events().len() as u64 - selected.events().len() as u64
    );
    let expected: Vec<_> = all
        .events()
        .filter(|event| event.kind.category() == Category::Mount)
        .copied()
        .collect();
    assert_eq!(selected.events().copied().collect::<Vec<_>>(), expected);
    assert_eq!(volume.list_root().unwrap(), volume_all.list_root().unwrap());
}

fn format_params() -> MkfsParams {
    MkfsParams {
        uuid: [0x6d; 16],
        label: "format".into(),
        region_size: 64,
        reclaim_caps: Default::default(),
        log_slots: 4,
        shared_extents: true,
        data_policy: true,
        name_policy: NamePolicy::Sensitive,
        timestamp: Timespec::default(),
    }
}

fn format_context(event: &afsplus_core::flight::Event) -> afsplus_core::flight::FormatContext {
    match event.lifecycle {
        Some(afsplus_core::flight::LifecycleContext::Format(context)) => context,
        other => panic!("expected a format context, found {other:?}"),
    }
}

#[test]
fn format_observation_preserves_results_traces_and_images_including_interruptions() {
    use afsplus_core::flight::FormatStage;
    use afsplus_core::{mkfs_observed, mkfs_with_options, MkfsOptions};
    let blank = MemoryBackend::new(4096, 256);
    let params = format_params();
    let mut probe = SharedDevice::new(&blank);
    let probe_handle = probe.handle();
    mkfs_with_options(&mut probe, &params, MkfsOptions::default()).unwrap();
    let writes = probe_handle
        .trace()
        .iter()
        .filter(|(operation, _)| *operation == 'w')
        .count();
    let flushes = probe_handle
        .trace()
        .iter()
        .filter(|(operation, _)| *operation == 'f')
        .count();
    assert_eq!(flushes, 2, "metadata barrier then publication barrier");

    let mut plans: Vec<(Option<usize>, Option<usize>)> = vec![(None, None)];
    plans.extend((0..writes).map(|write| (Some(write), None)));
    plans.extend((0..flushes).map(|flush| (None, Some(flush))));
    for (fail_write, fail_flush) in plans {
        let mut plain = SharedDevice::failing(&blank, fail_write, fail_flush);
        let plain_handle = plain.handle();
        let mut observed = SharedDevice::failing(&blank, fail_write, fail_flush);
        let observed_handle = observed.handle();
        let mut ring = recorder(64);
        let expected = mkfs_with_options(&mut plain, &params, MkfsOptions::default());
        let actual = mkfs_observed(&mut observed, &params, MkfsOptions::default(), &mut ring);
        let plan = format!("write {fail_write:?} flush {fail_flush:?}");
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"), "{plan}");
        assert_eq!(observed_handle.trace(), plain_handle.trace(), "{plan}");
        assert_eq!(observed_handle.blocks(), plain_handle.blocks(), "{plan}");
        assert_eq!(
            describe_mount(&mount(observed_handle.image())),
            describe_mount(&mount(plain_handle.image())),
            "{plan}"
        );
        assert_eq!((ring.dropped(), ring.filtered()), (0, 0), "{plan}");
        let events: Vec<_> = ring.events().copied().collect();
        assert!(events.iter().all(|event| event.generation == 1
            && event.attempt == 0
            && format_context(event).total_blocks == 256));
        let kinds: Vec<_> = events.iter().map(|event| event.kind).collect();
        if actual.is_ok() {
            assert_eq!(
                kinds,
                [
                    EventKind::FormatBegin,
                    EventKind::FormatMetadataDurable,
                    EventKind::FormatPublicationBegin,
                    EventKind::FormatCheckpointDurable
                ],
                "{plan}"
            );
            assert_eq!(
                events
                    .iter()
                    .map(|event| format_context(event).block)
                    .collect::<Vec<_>>(),
                [0, 0, 1, 1]
            );
            continue;
        }
        assert_eq!(events[0].kind, EventKind::FormatBegin, "{plan}");
        assert_eq!(*kinds.last().unwrap(), EventKind::FormatFailed, "{plan}");
        let failure = format_context(events.last().unwrap());
        let expected_stage = match (fail_write, fail_flush) {
            (_, Some(0)) => FormatStage::MetadataBarrier,
            (_, Some(1)) => FormatStage::PublicationBarrier,
            (Some(write), None) if write + 1 == writes => FormatStage::Publication,
            _ => FormatStage::Metadata,
        };
        assert_eq!(failure.stage, expected_stage, "{plan}");
        assert_eq!(
            failure.block,
            match expected_stage {
                FormatStage::Publication | FormatStage::PublicationBarrier => 1,
                _ => 0,
            },
            "{plan}"
        );
        assert_eq!(
            kinds.contains(&EventKind::FormatMetadataDurable),
            !matches!(
                expected_stage,
                FormatStage::Metadata | FormatStage::MetadataBarrier
            ),
            "{plan}"
        );
        assert!(
            !kinds.contains(&EventKind::FormatCheckpointDurable),
            "{plan}"
        );
    }
}

#[test]
fn small_format_rings_report_loss_and_selection_filters_format_events() {
    use afsplus_core::flight::{Categories, Category};
    use afsplus_core::{mkfs_observed, MkfsOptions};
    let params = format_params();
    let mut device = MemoryBackend::new(4096, 256);
    let mut ring = recorder(1);
    mkfs_observed(&mut device, &params, MkfsOptions::default(), &mut ring).unwrap();
    assert_eq!(ring.events().len(), 1);
    assert_eq!(ring.dropped(), 3);
    assert_eq!(ring.sequence(), 4);
    assert_eq!(
        ring.events().next().unwrap().kind,
        EventKind::FormatCheckpointDurable
    );

    let mut device = MemoryBackend::new(4096, 256);
    let mut ring = recorder(16);
    ring.set_categories(Categories::NONE.with(Category::Io));
    mkfs_observed(&mut device, &params, MkfsOptions::default(), &mut ring).unwrap();
    assert_eq!(ring.events().len(), 0);
    assert_eq!(
        (ring.filtered(), ring.dropped(), ring.sequence()),
        (4, 0, 4)
    );
    assert!(mount(device).is_ok());
}

fn verify_image() -> MemoryBackend {
    let mut volume = mount(image()).unwrap();
    for name in ["alpha", "beta"] {
        volume
            .create_file_in_root(name, b"verified content", Timespec::default())
            .unwrap();
    }
    volume.into_device()
}

fn ident_and_checkpoint<D: BlockDevice>(
    dev: &mut D,
) -> (
    afsplus_format::ident::Identification,
    afsplus_format::checkpoint::Checkpoint,
) {
    let mut buf = vec![0u8; dev.block_size()];
    dev.read_block(0, &mut buf).unwrap();
    let ident = afsplus_format::ident::Identification::decode(&buf).unwrap();
    let selection = afsplus_core::mount::select_checkpoint(dev, &ident).unwrap();
    (ident, selection.chosen)
}

/// A checksum-valid object record whose link count contradicts the namespace.
fn wrong_link_count_image() -> MemoryBackend {
    use afsplus_format::object::ObjectRecord;
    use afsplus_format::OBJECT_FIRST_DYNAMIC;
    let mut image = verify_image();
    let (ident, checkpoint) = ident_and_checkpoint(&mut image);
    let state =
        afsplus_core::verify::load_committed_state(&mut image, &ident, &checkpoint).unwrap();
    let entry = state
        .object_map
        .entries
        .iter()
        .find(|entry| entry.object_id >= OBJECT_FIRST_DYNAMIC)
        .expect("a created file");
    let buf = image.peek(entry.block);
    let (mut record, generation) = ObjectRecord::decode_metadata_with_generation(&buf).unwrap();
    record.link_count = 7;
    image.apply_raw(entry.block, &record.encode(4096, generation).unwrap());
    image
}

/// An unreadable object-map root, refusing every load along that path.
fn unreadable_object_map_image() -> MemoryBackend {
    let mut image = verify_image();
    let (_, checkpoint) = ident_and_checkpoint(&mut image);
    image.apply_raw(checkpoint.object_map_block, &vec![0u8; 4096]);
    image
}

fn describe_mount_state(
    result: &Result<afsplus_core::verify::MountState, afsplus_core::CoreError>,
) -> String {
    match result {
        Ok(state) => format!(
            "roots {} {} links {}",
            state.root_record_lba, state.root_directory_root_lba, state.root_object.link_count
        ),
        Err(error) => format!("refused {error}"),
    }
}

fn describe_committed_state(
    result: &Result<afsplus_core::verify::CommittedState, afsplus_core::CoreError>,
) -> String {
    match result {
        Ok(state) => format!(
            "objects {} metadata {} data {} runs {}",
            state.objects.len(),
            state.metadata_blocks.len(),
            state.data_blocks.len(),
            state.reclaim_runs.len()
        ),
        Err(error) => format!("refused {error}"),
    }
}

fn verify_context(event: &afsplus_core::flight::Event) -> afsplus_core::flight::VerifyContext {
    match event.lifecycle {
        Some(afsplus_core::flight::LifecycleContext::Verify(context)) => context,
        other => panic!("expected a verify context, found {other:?}"),
    }
}

#[test]
fn verification_observation_matches_findings_and_io_on_clean_and_corrupted_images() {
    use afsplus_core::flight::{Category, FindingKind, VerifyPhase, VerifyScope};
    use afsplus_core::verify::{
        full_sweep, full_sweep_observed, load_committed_state, load_committed_state_observed,
        load_mount_state, load_mount_state_observed,
    };
    for (name, base) in [
        ("clean", verify_image()),
        ("link-count", wrong_link_count_image()),
        ("unreadable-map", unreadable_object_map_image()),
    ] {
        let mut plain = SharedDevice::new(&base);
        let plain_handle = plain.handle();
        let mut observed = SharedDevice::new(&base);
        let observed_handle = observed.handle();
        let (ident, checkpoint) = ident_and_checkpoint(&mut plain);
        let (_, observed_checkpoint) = ident_and_checkpoint(&mut observed);
        assert_eq!(checkpoint.generation, observed_checkpoint.generation);
        let mut ring = recorder(4096);

        let expected = load_mount_state(&mut plain, &ident, &checkpoint);
        let actual = load_mount_state_observed(&mut observed, &ident, &checkpoint, &mut ring);
        assert_eq!(
            describe_mount_state(&actual),
            describe_mount_state(&expected),
            "{name}"
        );

        let expected_state = load_committed_state(&mut plain, &ident, &checkpoint);
        let actual_state =
            load_committed_state_observed(&mut observed, &ident, &checkpoint, &mut ring);
        assert_eq!(
            describe_committed_state(&actual_state),
            describe_committed_state(&expected_state),
            "{name}"
        );
        assert_eq!(observed_handle.trace(), plain_handle.trace(), "{name}");
        assert_eq!(observed_handle.blocks(), plain_handle.blocks(), "{name}");

        let events: Vec<_> = ring.events().copied().collect();
        assert_eq!((ring.dropped(), ring.filtered()), (0, 0), "{name}");
        assert!(events
            .iter()
            .all(|event| event.kind.category() == Category::Verify
                && event.attempt == 0
                && event.generation == checkpoint.generation));
        let mount_scope: Vec<_> = events
            .iter()
            .filter(|event| verify_context(event).scope == VerifyScope::MountState)
            .collect();
        let committed_scope: Vec<_> = events
            .iter()
            .filter(|event| verify_context(event).scope == VerifyScope::CommittedState)
            .collect();
        for scope in [&mount_scope, &committed_scope] {
            assert_eq!(scope[0].kind, EventKind::VerifyBegin, "{name}");
            assert!(verify_context(scope[0]).phase.is_none(), "{name}");
            assert!(scope[1..scope.len() - 1]
                .iter()
                .all(|event| event.kind == EventKind::VerifyPhase));
        }
        let phases = |scope: &Vec<&afsplus_core::flight::Event>| -> Vec<VerifyPhase> {
            scope
                .iter()
                .filter(|event| event.kind == EventKind::VerifyPhase)
                .map(|event| verify_context(event).phase.unwrap())
                .collect()
        };
        if name == "unreadable-map" {
            for scope in [&mount_scope, &committed_scope] {
                assert_eq!(
                    scope.last().unwrap().kind,
                    EventKind::VerifyFailed,
                    "{name}"
                );
                assert_eq!(
                    verify_context(scope.last().unwrap()).phase,
                    Some(VerifyPhase::ObjectMap),
                    "{name}"
                );
            }
            continue;
        }
        for scope in [&mount_scope, &committed_scope] {
            assert_eq!(
                scope.last().unwrap().kind,
                EventKind::VerifyComplete,
                "{name}"
            );
        }
        assert_eq!(
            phases(&mount_scope),
            [
                VerifyPhase::ObjectMap,
                VerifyPhase::RootObject,
                VerifyPhase::RootDirectory,
                VerifyPhase::ReclaimQueue
            ],
            "{name}"
        );
        assert_eq!(
            phases(&committed_scope),
            [
                VerifyPhase::AllocationRoot,
                VerifyPhase::ObjectMap,
                VerifyPhase::ObjectRecords,
                VerifyPhase::SharedMappings,
                VerifyPhase::Namespace,
                VerifyPhase::ReclaimQueue,
                VerifyPhase::AllocationBitmaps,
                VerifyPhase::IntentLogArea,
                VerifyPhase::Snapshots
            ],
            "{name}"
        );
        assert_eq!(
            verify_context(mount_scope[1]).block,
            checkpoint.object_map_block,
            "{name}"
        );

        // Full sweep: identical findings, one event per finding at its ordinal.
        let state = expected_state.unwrap();
        let geo = ident.geometry();
        let expected_findings = full_sweep(&state, &geo, &checkpoint);
        let mut sweep_ring = recorder(4096);
        let findings = full_sweep_observed(&state, &geo, &checkpoint, &mut sweep_ring);
        assert_eq!(findings, expected_findings, "{name}");
        let sweep: Vec<_> = sweep_ring.events().copied().collect();
        assert_eq!(sweep[0].kind, EventKind::VerifyBegin, "{name}");
        let complete = sweep.last().unwrap();
        assert_eq!(complete.kind, EventKind::VerifyComplete, "{name}");
        assert_eq!(verify_context(complete).ordinal, findings.len() as u64);
        assert_eq!(
            sweep
                .iter()
                .filter(|event| event.kind == EventKind::VerifyPhase)
                .map(|event| verify_context(event).phase.unwrap())
                .collect::<Vec<_>>(),
            [
                VerifyPhase::LinkCounts,
                VerifyPhase::QuarantinedRuns,
                VerifyPhase::BitmapAccounting,
                VerifyPhase::FreeCounts
            ],
            "{name}"
        );
        let reported: Vec<_> = sweep
            .iter()
            .filter(|event| event.kind == EventKind::VerifyFinding)
            .map(verify_context)
            .collect();
        assert_eq!(reported.len(), findings.len(), "{name}");
        for (ordinal, context) in reported.iter().enumerate() {
            assert_eq!(context.ordinal, ordinal as u64, "{name}");
            assert!(context.finding.is_some(), "{name}");
            assert!(context.phase.is_some(), "{name}");
            if context.object_id != 0 {
                assert!(
                    findings[ordinal].contains(&format!("object {}", context.object_id)),
                    "{name}: {} does not name object {}",
                    findings[ordinal],
                    context.object_id
                );
            }
        }
        match name {
            "clean" => assert!(findings.is_empty(), "{findings:?}"),
            "link-count" => {
                assert_eq!(findings.len(), 1, "{findings:?}");
                assert_eq!(reported[0].finding, Some(FindingKind::LinkCount));
                assert_eq!(reported[0].phase, Some(VerifyPhase::LinkCounts));
                assert!(reported[0].object_id >= afsplus_format::OBJECT_FIRST_DYNAMIC);
            }
            other => panic!("unnamed verification case {other}"),
        }
    }
}

#[test]
fn small_verification_rings_report_loss_without_changing_findings() {
    use afsplus_core::verify::{full_sweep, full_sweep_observed, load_committed_state};
    let mut image = wrong_link_count_image();
    let (ident, checkpoint) = ident_and_checkpoint(&mut image);
    let state = load_committed_state(&mut image, &ident, &checkpoint).unwrap();
    let geo = ident.geometry();
    let expected = full_sweep(&state, &geo, &checkpoint);
    let mut full = recorder(4096);
    full_sweep_observed(&state, &geo, &checkpoint, &mut full);
    for capacity in [1, 2, 4] {
        let mut ring = recorder(capacity);
        assert_eq!(
            full_sweep_observed(&state, &geo, &checkpoint, &mut ring),
            expected
        );
        assert_eq!(ring.sequence(), full.sequence());
        assert_eq!(ring.events().len(), capacity);
        assert_eq!(ring.dropped(), full.sequence() - capacity as u64);
        assert_eq!(ring.filtered(), 0);
    }
}

#[test]
fn observed_snapshot_mount_keeps_its_budgets_results_and_image() {
    use afsplus_core::flight::{Category, MountStage};
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mount_observed_with_snapshot_limits, mount_with_snapshot_limits};
    use afsplus_core::{MountMode, MountOptions};
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    let base = window_image(true);
    for mode in [MountMode::ReadWrite, MountMode::NoChanges] {
        let options = MountOptions {
            mode,
            ..Default::default()
        };
        let plain_device = SharedDevice::new(&base);
        let observed_device = SharedDevice::new(&base);
        let plain_handle = plain_device.handle();
        let observed_handle = observed_device.handle();
        let plain = mount_with_snapshot_limits(plain_device, options, limits);
        let mut observed =
            mount_observed_with_snapshot_limits(observed_device, options, limits, recorder(256))
                .unwrap();
        let ring = observed.replace_flight_recorder(None).unwrap();
        assert_eq!(describe_mount(&Ok(observed)), describe_mount(&plain));
        assert_eq!(observed_handle.trace(), plain_handle.trace());
        assert_eq!(observed_handle.blocks(), plain_handle.blocks());
        let kinds: Vec<_> = ring
            .events()
            .filter(|event| event.kind.category() == Category::Mount)
            .map(|event| event.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                EventKind::MountBegin,
                EventKind::MountSelected,
                EventKind::MountIntentBegin,
                EventKind::MountIntentScanned,
                EventKind::MountComplete
            ],
            "{mode:?}"
        );
        let last = ring.events().last().unwrap();
        assert_eq!(mount_context(last).stage, MountStage::IntentLog);
        assert_eq!(mount_context(last).mode, mode);
    }
}

// ---------------------------------------------------------------------------
// Pre-tail data writes and read-only view descents.
// ---------------------------------------------------------------------------

fn data_context(event: &afsplus_core::flight::Event) -> afsplus_core::flight::DataContext {
    match event.lifecycle {
        Some(afsplus_core::flight::LifecycleContext::Data(context)) => context,
        other => panic!("expected a data context, found {other:?}"),
    }
}

fn view_context(event: &afsplus_core::flight::Event) -> afsplus_core::flight::ViewReadContext {
    match event.lifecycle {
        Some(afsplus_core::flight::LifecycleContext::View(context)) => context,
        other => panic!("expected a view context, found {other:?}"),
    }
}

/// Window create, existing-file write, truncate tail zeroing, an fsync group
/// which needs the data barrier, and an fsync group with nothing to log.
fn staged_write_workload<D: BlockDevice>(volume: &mut afsplus_core::Volume<D>) -> Vec<String> {
    use afsplus_core::volume::BatchOp;
    use afsplus_format::OBJECT_ROOT;
    let now = Timespec::default();
    let mut results = Vec::new();
    let created = volume.window_op(
        &BatchOp::CreateFile {
            parent_id: OBJECT_ROOT,
            name: "staged",
            content: &[7u8; 9000],
        },
        now,
    );
    results.push(format!("{created:?}"));
    // A refused create leaves the later calls to refuse identically.
    let file = created.ok().flatten().unwrap_or(0);
    results.push(format!("{:?}", volume.window_fsync()));
    results.push(format!("{:?}", volume.window_commit(now)));
    // Existing-file updates need a committed file, so they follow the create.
    results.push(format!(
        "{:?}",
        volume.window_write_file_at(file, 100, &[9u8; 5000], now)
    ));
    results.push(format!(
        "{:?}",
        volume.window_truncate_file(file, 4500, now)
    ));
    results.push(format!("{:?}", volume.window_fsync()));
    results.push(format!("{:?}", volume.window_fsync()));
    results.push(format!("{:?}", volume.window_commit(now)));
    let mut buffer = vec![0u8; 4500];
    results.push(format!("{:?}", volume.read_file_at(file, 0, &mut buffer)));
    results.push(format!("{:?}", &buffer[..200]));
    results
}

#[test]
fn staged_writes_name_each_object_range_and_run_without_changing_io() {
    use afsplus_core::flight::{Category, DataScope};
    use afsplus_core::{mount_with_options, MountOptions};
    for pages in [2, 4, 8, usize::MAX] {
        for failure in [None, Some(0), Some(2)] {
            let base = window_image(false);
            let options = MountOptions {
                tree_cache_pages: NonZeroUsize::new(pages),
                ..Default::default()
            };
            let plain_device = SharedDevice::failing(&base, failure, None);
            let observed_device = SharedDevice::failing(&base, failure, None);
            let plain_handle = plain_device.handle();
            let observed_handle = observed_device.handle();
            let mut plain = mount_with_options(plain_device, options).unwrap();
            let mut observed = mount_with_options(observed_device, options).unwrap();
            let mut ring = recorder(4096);
            ring.enable_data_observation();
            observed.replace_flight_recorder(Some(ring));
            let expected = staged_write_workload(&mut plain);
            let actual = staged_write_workload(&mut observed);
            let ring = observed.replace_flight_recorder(None).unwrap();
            let plan = format!("{pages} pages, write fault {failure:?}");
            assert_eq!(actual, expected, "{plan}");
            assert_eq!(observed_handle.trace(), plain_handle.trace(), "{plan}");
            assert_eq!(observed_handle.blocks(), plain_handle.blocks(), "{plan}");
            assert_eq!((ring.dropped(), ring.filtered()), (0, 0), "{plan}");

            let staged: Vec<_> = ring
                .events()
                .filter(|event| event.kind.category() == Category::Data)
                .copied()
                .collect();
            assert!(staged
                .iter()
                .all(|event| event.attempt == 0 && event.window != 0));
            if failure.is_some() {
                let failed = staged
                    .iter()
                    .find(|event| event.kind == EventKind::DataWriteFailed)
                    .unwrap_or_else(|| panic!("{plan}: a staged write must report its failure"));
                assert_eq!(data_context(failed).blocks, 1);
                continue;
            }
            let scopes: Vec<_> = staged
                .iter()
                .map(|event| (event.kind, data_context(event).scope))
                .collect();
            assert_eq!(
                scopes,
                [
                    (EventKind::DataWriteBegin, DataScope::CreateWriteThrough),
                    (EventKind::DataWriteComplete, DataScope::CreateWriteThrough),
                    (EventKind::DataWriteBegin, DataScope::ExistingFileWrite),
                    (EventKind::DataWriteComplete, DataScope::ExistingFileWrite),
                    (EventKind::DataWriteBegin, DataScope::TruncateTailZero),
                    (EventKind::DataWriteComplete, DataScope::TruncateTailZero),
                ],
                "{plan}"
            );
            let created = data_context(&staged[0]);
            assert_eq!((created.offset, created.length), (0, 9000));
            assert_eq!(created.blocks, 3);
            assert!(created.start != 0);
            let written = data_context(&staged[2]);
            assert_eq!((written.offset, written.length), (100, 5000));
            assert_eq!(written.blocks, 2);
            let written_run = data_context(&staged[3]);
            assert_eq!(written_run.object_id, written.object_id);
            assert_eq!(written_run.offset % 4096, 0);
            let zeroed = data_context(&staged[4]);
            assert_eq!((zeroed.blocks, zeroed.length), (1, 4096));
            assert_eq!(zeroed.offset, 4096);

            // The two fsync barriers own distinct I/O events.
            let barriers: Vec<_> = ring
                .events()
                .filter(|event| {
                    matches!(
                        event.kind,
                        EventKind::IntentDataDurable | EventKind::IntentEmptyFlush
                    )
                })
                .map(|event| event.kind)
                .collect();
            assert_eq!(
                barriers,
                [EventKind::IntentDataDurable, EventKind::IntentEmptyFlush],
                "{plan}"
            );
        }
    }
}

#[test]
fn staged_write_selection_and_small_rings_report_loss_without_changing_results() {
    use afsplus_core::flight::{Categories, Category};
    use afsplus_core::{mount_with_options, MountOptions};
    let base = window_image(false);
    let options = MountOptions::default();
    let mut full = mount_with_options(base.clone(), options).unwrap();
    let mut ring = recorder(4096);
    ring.enable_data_observation();
    full.replace_flight_recorder(Some(ring));
    let expected = staged_write_workload(&mut full);
    let ring = full.replace_flight_recorder(None).unwrap();
    let total = ring.sequence();
    let data_events = ring
        .events()
        .filter(|event| event.kind.category() == Category::Data)
        .count();
    assert!(data_events >= 6);

    let mut volume = mount_with_options(base.clone(), options).unwrap();
    let mut ring = recorder(1);
    ring.enable_data_observation();
    volume.replace_flight_recorder(Some(ring));
    assert_eq!(staged_write_workload(&mut volume), expected);
    let ring = volume.replace_flight_recorder(None).unwrap();
    assert_eq!(ring.sequence(), total);
    assert_eq!(ring.events().len(), 1);
    assert_eq!(ring.dropped(), total - 1);

    let mut volume = mount_with_options(base, options).unwrap();
    let mut ring = recorder(4096);
    ring.enable_data_observation();
    ring.set_categories(Categories::NONE.with(Category::Data));
    volume.replace_flight_recorder(Some(ring));
    assert_eq!(staged_write_workload(&mut volume), expected);
    let ring = volume.replace_flight_recorder(None).unwrap();
    assert_eq!(ring.sequence(), total);
    assert_eq!(ring.events().len(), data_events);
    assert_eq!(ring.filtered(), total - data_events as u64);
    assert_eq!(ring.dropped(), 0);
}

/// Lookup, enumeration, bounded paging and file reads over one view.
fn read_workload<D: BlockDevice>(
    volume: &mut afsplus_core::Volume<D>,
    handle: Option<&afsplus_core::volume::SnapshotHandle>,
    file: u64,
) -> Vec<String> {
    use afsplus_format::OBJECT_ROOT;
    let mut results = Vec::new();
    let mut buffer = vec![0u8; 64];
    match handle {
        None => {
            results.push(format!("{:?}", volume.lookup_root("kept")));
            results.push(format!(
                "{:?}",
                volume.lookup_in_directory(OBJECT_ROOT, "gone")
            ));
            results.push(format!("{:?}", volume.list_root()));
            results.push(format!(
                "{:?}",
                volume.read_directory_page(OBJECT_ROOT, None, 1)
            ));
            results.push(format!("{:?}", volume.read_file_at(file, 0, &mut buffer)));
            results.push(format!(
                "{:?}",
                volume.read_file_at(u64::MAX, 0, &mut buffer)
            ));
        }
        Some(handle) => {
            results.push(format!(
                "{:?}",
                volume.snapshot_lookup(handle, OBJECT_ROOT, "kept")
            ));
            results.push(format!(
                "{:?}",
                volume.snapshot_read_directory_page(handle, OBJECT_ROOT, None, 1)
            ));
            results.push(format!(
                "{:?}",
                volume.snapshot_read_file_at(handle, file, 0, &mut buffer)
            ));
            results.push(format!(
                "{:?}",
                volume.snapshot_read_file_at(handle, u64::MAX, 0, &mut buffer)
            ));
        }
    }
    results.push(format!("{:?}", &buffer[..8]));
    results
}

#[test]
fn read_paths_name_view_owner_and_root_without_changing_io() {
    use afsplus_core::flight::{Category, ReadPath};
    use afsplus_core::{mount_with_options, MountOptions};
    for pages in [2, 4, 8, usize::MAX] {
        let base = {
            let mut volume = mount(image()).unwrap();
            volume
                .create_file_in_root("kept", b"view bytes", Timespec::default())
                .unwrap();
            volume.into_device()
        };
        let file = mount(base.clone())
            .unwrap()
            .lookup_root("kept")
            .unwrap()
            .unwrap();
        let options = MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        };
        let plain_device = SharedDevice::new(&base);
        let observed_device = SharedDevice::new(&base);
        let plain_handle = plain_device.handle();
        let observed_handle = observed_device.handle();
        let mut plain = mount_with_options(plain_device, options).unwrap();
        let mut observed = mount_with_options(observed_device, options).unwrap();
        let mut ring = recorder(4096);
        ring.enable_view_observation();
        observed.replace_flight_recorder(Some(ring));
        let expected = read_workload(&mut plain, None, file);
        let actual = read_workload(&mut observed, None, file);
        let ring = observed.replace_flight_recorder(None).unwrap();
        assert_eq!(actual, expected, "{pages} pages");
        assert_eq!(
            observed_handle.trace(),
            plain_handle.trace(),
            "{pages} pages"
        );
        assert_eq!(
            observed_handle.blocks(),
            plain_handle.blocks(),
            "{pages} pages"
        );
        assert_eq!((ring.dropped(), ring.filtered()), (0, 0));

        let reads: Vec<_> = ring
            .events()
            .filter(|event| event.kind.category() == Category::View)
            .copied()
            .collect();
        assert!(reads.iter().all(|event| event.attempt == 0
            && view_context(event).view_id == 0
            && event.api.method.is_some()));
        let paths: Vec<_> = reads
            .iter()
            .map(|event| (event.kind, view_context(event).path))
            .collect();
        assert_eq!(
            paths,
            [
                (EventKind::ViewReadBegin, ReadPath::Lookup),
                (EventKind::ViewReadComplete, ReadPath::Lookup),
                (EventKind::ViewReadBegin, ReadPath::Lookup),
                (EventKind::ViewReadComplete, ReadPath::Lookup),
                (EventKind::ViewReadBegin, ReadPath::Enumeration),
                (EventKind::ViewReadComplete, ReadPath::Enumeration),
                (EventKind::ViewReadBegin, ReadPath::Enumeration),
                (EventKind::ViewReadComplete, ReadPath::Enumeration),
                (EventKind::ViewReadBegin, ReadPath::FileData),
                (EventKind::ViewReadComplete, ReadPath::FileData),
                (EventKind::ViewReadBegin, ReadPath::FileData),
                (EventKind::ViewReadFailed, ReadPath::FileData),
            ],
            "{pages} pages"
        );
        assert!(
            reads[..8].iter().all(
                |event| view_context(event).owner == afsplus_format::OBJECT_ROOT
                    && view_context(event).block != 0
            ),
            "directory descents name the root directory and its tree root"
        );
        assert_eq!(view_context(&reads[8]).owner, file);
        assert_eq!(
            view_context(&reads[8]).block,
            0,
            "file data resolves its own root"
        );
        // Every read-path event sits inside the API span of its own call.
        let spans: std::collections::BTreeSet<_> =
            reads.iter().map(|event| event.api.span).collect();
        assert_eq!(spans.len(), 6);
    }
}

#[test]
fn captured_view_reads_carry_their_snapshot_identity() {
    use afsplus_core::flight::{Category, ReadPath};
    use afsplus_core::mount_with_snapshot_limits;
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{MkfsOptions, MountOptions};
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    for pages in [2, 4, 8, usize::MAX] {
        let mut base = MemoryBackend::new(4096, 2048);
        afsplus_core::mkfs_with_options(
            &mut base,
            &MkfsParams {
                uuid: [0x5c; 16],
                label: "captured reads".into(),
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
        let options = MountOptions {
            tree_cache_pages: NonZeroUsize::new(pages),
            ..Default::default()
        };
        let mut results = Vec::new();
        let mut traces = Vec::new();
        for observed in [false, true] {
            let device = SharedDevice::new(&base);
            let handle_device = device.handle();
            let mut volume = mount_with_snapshot_limits(device, options, limits).unwrap();
            let file = volume
                .create_file_in_root("kept", b"captured bytes", Timespec::default())
                .unwrap();
            let id = volume.snapshot_create(Timespec::default()).unwrap();
            let snapshot = volume.snapshot_open(id).unwrap();
            if observed {
                let mut ring = recorder(4096);
                ring.enable_view_observation();
                volume.replace_flight_recorder(Some(ring));
            }
            results.push(read_workload(&mut volume, Some(&snapshot), file));
            if observed {
                let ring = volume.replace_flight_recorder(None).unwrap();
                let reads: Vec<_> = ring
                    .events()
                    .filter(|event| event.kind.category() == Category::View)
                    .copied()
                    .collect();
                assert!(reads.iter().all(|event| view_context(event).view_id == id));
                assert_eq!(
                    reads
                        .iter()
                        .map(|event| (event.kind, view_context(event).path))
                        .collect::<Vec<_>>(),
                    [
                        (EventKind::ViewReadBegin, ReadPath::Lookup),
                        (EventKind::ViewReadComplete, ReadPath::Lookup),
                        (EventKind::ViewReadBegin, ReadPath::Enumeration),
                        (EventKind::ViewReadComplete, ReadPath::Enumeration),
                        (EventKind::ViewReadBegin, ReadPath::FileData),
                        (EventKind::ViewReadComplete, ReadPath::FileData),
                        (EventKind::ViewReadBegin, ReadPath::FileData),
                        (EventKind::ViewReadFailed, ReadPath::FileData),
                    ],
                    "{pages} pages"
                );
                assert_eq!(view_context(&reads[4]).owner, file);
            }
            drop(volume);
            traces.push((handle_device.trace(), handle_device.blocks()));
        }
        assert_eq!(results[0], results[1], "{pages} pages");
        assert_eq!(traces[0].0, traces[1].0, "{pages} pages");
        assert_eq!(traces[0].1, traces[1].1, "{pages} pages");
    }
}

// ---------------------------------------------------------------------------
// Executed API guard coverage.
// ---------------------------------------------------------------------------

/// Every registered method executed once under observation. Results are
/// deliberately unchecked: the guard owns the call whatever it returns.
#[test]
fn every_registered_api_method_executes_under_its_own_guard() {
    use afsplus_core::flight::{ApiMethod, Category};
    use afsplus_core::volume::{
        BatchOp, DataUpdatePolicy, FileEditLimits, PreservedMetadata, SnapshotWorkLimits,
    };
    use afsplus_core::{mount_with_snapshot_limits, MkfsOptions, MountOptions};
    use afsplus_format::OBJECT_ROOT;
    let mut base = MemoryBackend::new(4096, 4096);
    afsplus_core::mkfs_with_options(
        &mut base,
        &MkfsParams {
            uuid: [0x2a; 16],
            label: "api coverage".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
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
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    let mut volume =
        mount_with_snapshot_limits(TraceBackend::new(base), MountOptions::default(), limits)
            .unwrap();
    let now = Timespec::default();
    let file = volume.create_file_in_root("file", b"content", now).unwrap();
    let directory = volume.create_directory_in_root("dir", now).unwrap();
    let symlink = volume
        .create_symlink(OBJECT_ROOT, "link", "file", now)
        .unwrap();
    let snapshot_id = volume.snapshot_create(now).unwrap();
    let handle = volume.snapshot_open(snapshot_id).unwrap();
    let metadata: PreservedMetadata = volume
        .visible_metadata(file)
        .unwrap()
        .expect("the created file is visible")
        .into();
    let edit = FileEditLimits {
        max_blocks: 64,
        max_records: 64,
    };

    let mut ring = recorder(8192);
    ring.enable_api_observation();
    volume.replace_flight_recorder(Some(ring));
    let mut bytes = vec![0u8; 64];

    volume.set_tree_cache_pages(8).ok();
    volume.set_reclaim_batch_blocks(8);
    volume.set_orphan_cleanup_extent_budget(8);
    volume.set_data_update_policy(DataUpdatePolicy::FullCow);
    volume.set_snapshot_work_limits(limits).ok();
    volume.quarantine_contains(1).ok();
    volume.reclaim_step(now).ok();
    volume.file_data_policy(file).ok();
    volume
        .set_file_data_policy(file, DataUpdatePolicy::FullCow, now)
        .ok();
    volume.sync().ok();
    volume.lookup_root("file").ok();
    volume.lookup_in_directory(OBJECT_ROOT, "file").ok();
    volume.list_root().ok();
    volume.list_directory(OBJECT_ROOT).ok();
    volume.read_directory_page(OBJECT_ROOT, None, 2).ok();
    volume.stat(file).ok();
    volume.file_allocation_page(file, 0, 2).ok();
    volume.visible_metadata(file).ok();
    volume.read_file(file).ok();
    volume.read_file_at(file, 0, &mut bytes).ok();
    volume.write_file_at(file, 0, b"written", now).ok();
    volume
        .write_file_at_bounded(file, 0, b"bounded", now, edit)
        .ok();
    volume.truncate_file(file, 4, now).ok();
    volume.truncate_file_bounded(file, 3, now, edit).ok();
    volume.preallocate_file(file, 0, 4096, now).ok();
    volume
        .preallocate_file_bounded(file, 0, 4096, now, edit)
        .ok();
    volume.clone_file(file, OBJECT_ROOT, "clone", now).ok();
    volume.clone_range(file, 0, file, 0, 4096, now).ok();
    volume.create_file_in_root("second", b"", now).ok();
    volume
        .create_file_in_directory(directory, "child", b"", now)
        .ok();
    volume
        .create_symlink(OBJECT_ROOT, "link2", "file", now)
        .ok();
    volume.create_directory_in_root("dir2", now).ok();
    volume.create_directory(directory, "nested", now).ok();
    volume.delete_file_in_root("second", now).ok();
    volume.delete_file(directory, "child", now).ok();
    volume.remove_directory(directory, "nested", now).ok();
    volume.orphan_file(OBJECT_ROOT, "clone", now).ok();
    volume.orphan_object(file).ok();
    volume.orphan_count().ok();
    let orphan = volume.first_orphan().ok().flatten().unwrap_or(file);
    volume.cleanup_orphan(orphan, now).ok();
    volume.cleanup_orphans(4, &mut |_| false, now).ok();
    volume.link_file(file, OBJECT_ROOT, "hard", now).ok();
    volume
        .rename(OBJECT_ROOT, "hard", OBJECT_ROOT, "moved", now)
        .ok();
    volume
        .rename_replace(OBJECT_ROOT, "moved", OBJECT_ROOT, "file", now)
        .ok();
    volume
        .rename_replace_orphan_target(OBJECT_ROOT, "file", OBJECT_ROOT, "dir2", now)
        .ok();
    volume
        .run_batch(
            &[BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "batched",
                content: b"batch",
            }],
            now,
        )
        .ok();
    volume.read_link(symlink, &mut bytes).ok();
    volume.unlink_symlink(OBJECT_ROOT, "link2", now).ok();
    volume.set_object_protection(file, 0o644, now).ok();
    volume.restore_object_metadata(file, metadata).ok();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "windowed",
                content: b"window",
            },
            now,
        )
        .ok();
    volume.window_fsync().ok();
    volume.window_commit(now).ok();
    volume.window_write_file_at(file, 0, b"logged", now).ok();
    volume.window_truncate_file(file, 2, now).ok();
    volume.window_commit(now).ok();
    volume.window_discard();
    volume.snapshot_stat(&handle, file).ok();
    volume.snapshot_allocation_page(&handle, file, 0, 2).ok();
    volume.snapshot_read_link(&handle, symlink, &mut bytes).ok();
    volume
        .snapshot_read_file_at(&handle, file, 0, &mut bytes)
        .ok();
    volume.snapshot_lookup(&handle, OBJECT_ROOT, "file").ok();
    volume
        .snapshot_read_directory_page(&handle, OBJECT_ROOT, None, 2)
        .ok();
    // Attributes, comments, security descriptors, the volume label and the
    // allocation reader that takes a byte offset. Each is read and written on
    // the live volume, then read again through the snapshot view, which is
    // where four of these methods only exist. A refused call still executes
    // under its guard, and executing under a guard is all this test measures:
    // what each one does is stated by its own test elsewhere.
    volume
        .set_attributes(
            file,
            &[("user.one", Some(b"1".as_slice()))],
            AttributeWriteMode::Upsert,
            now,
        )
        .ok();
    volume.attribute(file, "user.one").ok();
    volume.attribute_names(file).ok();
    volume.set_object_owner(file, 501, 20, now).ok();
    volume.set_object_times(file, now, now).ok();
    volume.set_object_comment(file, "a comment", now).ok();
    volume.object_comment(file).ok();
    volume.set_security_projection_policy(SecurityProjectionPolicy::Preserve);
    volume
        .set_security_descriptor(file, 1, 0, b"descriptor", now)
        .ok();
    volume.security_descriptor(file).ok();
    volume.clear_security_descriptor(file, now).ok();
    volume.set_volume_label("Flight").ok();
    volume.file_allocation_from(file, 0, 2).ok();
    volume.snapshot_object_comment(&handle, file).ok();
    volume.snapshot_attribute(&handle, file, "user.one").ok();
    volume.snapshot_attribute_names(&handle, file).ok();
    volume.snapshot_security_descriptor(&handle, file).ok();
    volume.snapshot_maintenance_step(now).ok();
    volume.snapshot_list(0, 4).ok();
    if let Ok(second) = volume.snapshot_create(now) {
        drop(volume.snapshot_open(second));
    }
    drop(handle);
    volume.snapshot_delete(snapshot_id, now).ok();

    let ring = volume.replace_flight_recorder(None).unwrap();
    assert_eq!(ring.dropped(), 0, "the ring must hold every executed call");
    let mut executed = std::collections::BTreeSet::new();
    let mut open = Vec::new();
    for event in ring.events() {
        if event.kind.category() != Category::Api {
            continue;
        }
        let method = event.api.method.expect("an API event names its method");
        if event.kind == EventKind::ApiBegin {
            open.push((event.api.span, method));
            executed.insert(format!("{method:?}"));
        } else {
            assert_ne!(event.kind, EventKind::ApiUnwound, "{method:?} unwound");
            assert_eq!(
                open.pop(),
                Some((event.api.span, method)),
                "outcome of {method:?} must close its own span"
            );
        }
    }
    assert!(open.is_empty(), "every executed call reports an outcome");

    // The registry is the source of truth for what a guard must cover.
    let registry: std::collections::BTreeSet<_> = include_str!("../src/flight.rs")
        .split("pub enum ApiMethod {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name.trim().to_owned())
        .collect();
    // No count is asserted here on purpose. There was one, a literal 66 kept in
    // step by hand, and it went stale while the registry grew to 81. It caught
    // the growth, which is not the question, and it failed FIRST, so the real
    // answer stayed hidden behind "left 81, right 66" until somebody bumped the
    // number: fifteen methods registered and never exercised. A count that
    // fires before the assertion it is standing in for makes a failure less
    // informative, not more. The set comparison below needs no maintenance and
    // names what is wrong.
    assert!(!registry.is_empty(), "the registry must be readable");
    let missing: Vec<_> = registry.difference(&executed).cloned().collect();
    assert!(
        missing.is_empty(),
        "registered but never executed: {missing:?}"
    );
    let unregistered: Vec<_> = executed.difference(&registry).cloned().collect();
    assert!(
        unregistered.is_empty(),
        "executed but not in the registry: {unregistered:?}"
    );
    assert_eq!(ApiMethod::CleanupOrphan as u16, 1);
}

// ---------------------------------------------------------------------------
// Executed publication-family evidence.
// ---------------------------------------------------------------------------

type FamilyVolume = afsplus_core::Volume<SharedDevice>;

fn family_image(snapshots: bool) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 2048);
    afsplus_core::mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [0x63; 16],
            label: "families".into(),
            region_size: 2048,
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

fn family_recorder(capacity: usize) -> FlightRecorder {
    let mut ring = recorder(capacity);
    ring.enable_subsystem_observation();
    ring.enable_object_observation();
    ring.enable_data_observation();
    ring.enable_view_observation();
    ring
}

fn mount_family(device: SharedDevice, pages: usize, snapshots: bool) -> FamilyVolume {
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mount_with_options, mount_with_snapshot_limits, MountOptions};
    let options = MountOptions {
        tree_cache_pages: NonZeroUsize::new(pages),
        ..Default::default()
    };
    if snapshots {
        mount_with_snapshot_limits(
            device,
            options,
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 8,
                reclaim_records: 8,
            },
        )
        .unwrap()
    } else {
        mount_with_options(device, options).unwrap()
    }
}

/// Ordered correlation oracle: API spans nest, every subsystem and commit
/// event names the innermost open call, and one commit attempt keeps one
/// generation and one root operation from its Begin to its outcome.
fn assert_correlation(ring: &FlightRecorder, family: &str) {
    use afsplus_core::flight::{ApiContext, Category};
    use std::collections::BTreeMap;
    let mut stack: Vec<ApiContext> = Vec::new();
    let mut attempts: BTreeMap<u64, (u64, u64, bool)> = BTreeMap::new();
    let mut open_attempt = None;
    for event in ring.events() {
        match event.kind.category() {
            Category::Api => {
                if event.kind == EventKind::ApiBegin {
                    if let Some(parent) = stack.last() {
                        assert_eq!(event.api.parent_span, parent.span, "{family}");
                        assert_eq!(event.api.operation, parent.operation, "{family}");
                    } else {
                        assert_eq!(event.api.parent_span, 0, "{family}");
                        assert_eq!(event.api.operation, event.api.span, "{family}");
                    }
                    stack.push(event.api);
                } else {
                    assert_ne!(event.kind, EventKind::ApiUnwound, "{family} unwound");
                    assert_eq!(stack.pop(), Some(event.api), "{family}");
                }
                assert_eq!(event.attempt, 0, "{family}");
            }
            Category::Transaction | Category::Checkpoint | Category::Io | Category::Error => {
                assert!(event.attempt > 0, "{family}: commit events bind an attempt");
                if event.kind == EventKind::Begin {
                    assert!(open_attempt.is_none(), "{family}: nested commit attempt");
                    open_attempt = Some(event.attempt);
                    attempts.insert(
                        event.attempt,
                        (event.generation, event.api.operation, false),
                    );
                } else if matches!(
                    event.kind,
                    EventKind::IntentDataDurable | EventKind::IntentEmptyFlush
                ) {
                    // Intent barriers belong to the window, not to one attempt.
                    continue;
                } else {
                    let entry = attempts
                        .get_mut(&event.attempt)
                        .unwrap_or_else(|| panic!("{family}: event before its Begin"));
                    assert_eq!(entry.0, event.generation, "{family}");
                    assert_eq!(entry.1, event.api.operation, "{family}");
                    if matches!(event.kind, EventKind::Adopted | EventKind::Failed) {
                        entry.2 = true;
                        open_attempt = None;
                    }
                }
                if !stack.is_empty() {
                    assert_eq!(event.api, *stack.last().unwrap(), "{family}");
                }
            }
            _ => {
                assert_eq!(event.attempt, 0, "{family}: subsystem events use attempt 0");
                if !stack.is_empty() {
                    assert_eq!(event.api, *stack.last().unwrap(), "{family}");
                }
            }
        }
    }
    assert!(stack.is_empty(), "{family}: every call reports an outcome");
    assert!(
        attempts.values().all(|(_, _, closed)| *closed),
        "{family}: every commit attempt reports adoption or failure"
    );
}

/// Compare one publication family with its unobserved execution under four
/// cache profiles, a fault at each of its own barriers and representative
/// write faults, then repeat the normal case with a one-event ring.
fn qualify_family(
    family: &str,
    snapshots: bool,
    refuses: bool,
    prepare: fn(&mut FamilyVolume),
    exercise: fn(&mut FamilyVolume) -> Vec<String>,
) {
    let base = family_image(snapshots);
    // Probe the family's own device operations, after preparation.
    let (writes, flushes) = {
        let device = SharedDevice::new(&base);
        let handle = device.handle();
        let mut volume = mount_family(device, usize::MAX, snapshots);
        prepare(&mut volume);
        let before = handle.trace().len();
        exercise(&mut volume);
        let own = handle.trace()[before..].to_vec();
        (
            own.iter().filter(|(op, _)| *op == 'w').count(),
            own.iter().filter(|(op, _)| *op == 'f').count(),
        )
    };
    assert!(writes > 0, "{family} writes nothing to observe");
    let mut plans: Vec<(Option<usize>, Option<usize>)> = vec![(None, None)];
    for write in [0, writes / 2, writes.saturating_sub(1)] {
        plans.push((Some(write), None));
    }
    for flush in 0..flushes.min(3) {
        plans.push((None, Some(flush)));
    }
    for pages in [2, 4, 8, usize::MAX] {
        for (fail_write, fail_flush) in plans.clone() {
            let plan =
                format!("{family}: {pages} pages, write {fail_write:?} flush {fail_flush:?}");
            let plain_device = SharedDevice::new(&base);
            let observed_device = SharedDevice::new(&base);
            let plain_handle = plain_device.handle();
            let observed_handle = observed_device.handle();
            let mut plain = mount_family(plain_device, pages, snapshots);
            let mut observed = mount_family(observed_device, pages, snapshots);
            prepare(&mut plain);
            prepare(&mut observed);
            plain.device_mut().arm(fail_write, fail_flush);
            observed.device_mut().arm(fail_write, fail_flush);
            observed.replace_flight_recorder(Some(family_recorder(16384)));
            let expected = exercise(&mut plain);
            let actual = exercise(&mut observed);
            let ring = observed.replace_flight_recorder(None).unwrap();
            assert_eq!(actual, expected, "{plan}");
            assert_eq!(observed_handle.trace(), plain_handle.trace(), "{plan}");
            assert_eq!(observed_handle.blocks(), plain_handle.blocks(), "{plan}");
            assert_eq!(ring.dropped(), 0, "{plan}");
            assert_correlation(&ring, &plan);
            assert!(
                ring.events().any(|event| event.kind == EventKind::ApiBegin),
                "{plan}: the family executes at least one guarded call"
            );
            if fail_write.is_none() && fail_flush.is_none() {
                assert!(
                    ring.events()
                        .any(|event| event.kind == EventKind::CheckpointDurable),
                    "{plan}: the family publishes a checkpoint"
                );
                assert_eq!(
                    ring.events()
                        .any(|event| event.kind == EventKind::ApiFailed),
                    refuses,
                    "{plan}: the family's validation refusal reports ApiFailed"
                );
            } else {
                assert!(
                    ring.events().any(|event| matches!(
                        event.kind,
                        EventKind::Failed | EventKind::ApiFailed | EventKind::WindowFailed
                    )),
                    "{plan}: an injected fault reports a failure"
                );
            }
        }
    }
    // A one-event ring loses records without changing the family's result.
    let plain_device = SharedDevice::new(&base);
    let observed_device = SharedDevice::new(&base);
    let mut plain = mount_family(plain_device, usize::MAX, snapshots);
    let mut observed = mount_family(observed_device, usize::MAX, snapshots);
    prepare(&mut plain);
    prepare(&mut observed);
    observed.replace_flight_recorder(Some(family_recorder(1)));
    let expected = exercise(&mut plain);
    assert_eq!(
        exercise(&mut observed),
        expected,
        "{family} with a tiny ring"
    );
    let ring = observed.replace_flight_recorder(None).unwrap();
    assert_eq!(ring.events().len(), 1, "{family}");
    assert_eq!(ring.dropped(), ring.sequence() - 1, "{family}");
    assert_eq!(ring.filtered(), 0, "{family}");
}

fn now() -> Timespec {
    Timespec::default()
}

fn file_in_root(volume: &mut FamilyVolume, name: &str, bytes: usize) -> u64 {
    volume
        .create_file_in_root(name, &vec![0x21u8; bytes], now())
        .unwrap()
}

#[test]
fn family_reclaim_step_publishes_promotions_under_observation() {
    qualify_family(
        "reclaim_step",
        false,
        false,
        |volume| {
            let file = file_in_root(volume, "retired", 12000);
            volume.delete_file_in_root("retired", now()).unwrap();
            let _ = file;
        },
        |volume| {
            vec![
                format!("{:?}", volume.reclaim_step(now())),
                format!("{:?}", volume.reclaim_step(now())),
                format!("{:?}", volume.quarantine_contains(64)),
            ]
        },
    );
}

#[test]
fn family_clone_file_publishes_shared_references_and_refuses_duplicates() {
    qualify_family(
        "clone_file",
        false,
        true,
        |volume| {
            file_in_root(volume, "source", 12000);
        },
        |volume| {
            let source = volume.lookup_root("source").unwrap().unwrap();
            vec![
                format!(
                    "{:?}",
                    volume.clone_file(source, OBJECT_ROOT, "copy", now())
                ),
                format!(
                    "{:?}",
                    volume.clone_file(source, OBJECT_ROOT, "copy", now())
                ),
            ]
        },
    );
}

#[test]
fn family_clone_range_publishes_partial_sharing_and_refuses_directories() {
    qualify_family(
        "clone_range",
        false,
        true,
        |volume| {
            file_in_root(volume, "source", 12000);
            file_in_root(volume, "target", 12000);
            volume.create_directory_in_root("dir", now()).unwrap();
        },
        |volume| {
            let source = volume.lookup_root("source").unwrap().unwrap();
            let target = volume.lookup_root("target").unwrap().unwrap();
            let directory = volume.lookup_root("dir").unwrap().unwrap();
            vec![
                format!(
                    "{:?}",
                    volume.clone_range(source, 0, target, 0, 8192, now())
                ),
                format!(
                    "{:?}",
                    volume.clone_range(source, 0, directory, 0, 4096, now())
                ),
            ]
        },
    );
}

#[test]
fn family_create_leaf_in_directory_publishes_and_refuses_duplicates() {
    qualify_family(
        "create_leaf_in_directory",
        false,
        true,
        |volume| {
            volume.create_directory_in_root("dir", now()).unwrap();
        },
        |volume| {
            let directory = volume.lookup_root("dir").unwrap().unwrap();
            vec![
                format!(
                    "{:?}",
                    volume.create_file_in_directory(directory, "leaf", b"leaf", now())
                ),
                format!(
                    "{:?}",
                    volume.create_file_in_directory(directory, "leaf", b"leaf", now())
                ),
                format!(
                    "{:?}",
                    volume.create_symlink(directory, "link", "leaf", now())
                ),
            ]
        },
    );
}

#[test]
fn family_create_directory_publishes_and_refuses_duplicates() {
    qualify_family(
        "create_directory",
        false,
        true,
        |volume| {
            volume.create_directory_in_root("dir", now()).unwrap();
        },
        |volume| {
            let directory = volume.lookup_root("dir").unwrap().unwrap();
            vec![
                format!("{:?}", volume.create_directory(directory, "child", now())),
                format!("{:?}", volume.create_directory(directory, "child", now())),
                format!("{:?}", volume.create_directory_in_root("second", now())),
            ]
        },
    );
}

#[test]
fn family_cleanup_orphan_data_step_publishes_bounded_progress() {
    qualify_family(
        "cleanup_orphan_data_step",
        false,
        // Cleanup answers an unknown or non-orphan object with zero progress,
        // so this family reports no validation refusal.
        false,
        |volume| {
            file_in_root(volume, "orphaned", 40000);
            volume.orphan_file(OBJECT_ROOT, "orphaned", now()).unwrap();
        },
        |volume| {
            let orphan = volume.first_orphan().ok().flatten().unwrap_or(u64::MAX);
            let mut results = Vec::new();
            for _ in 0..3 {
                results.push(format!("{:?}", volume.cleanup_orphan(orphan, now())));
            }
            results.push(format!("{:?}", volume.cleanup_orphan(OBJECT_ROOT, now())));
            results
        },
    );
}

#[test]
fn family_ensure_orphan_directory_publishes_the_reserved_directory() {
    qualify_family(
        "ensure_orphan_directory",
        false,
        true,
        |volume| {
            file_in_root(volume, "victim", 8000);
        },
        |volume| {
            vec![
                format!("{:?}", volume.orphan_file(OBJECT_ROOT, "victim", now())),
                format!("{:?}", volume.orphan_file(OBJECT_ROOT, "missing", now())),
                format!("{:?}", volume.orphan_count()),
            ]
        },
    );
}

#[test]
fn family_remove_entry_publishes_and_refuses_a_populated_directory() {
    qualify_family(
        "remove_entry",
        false,
        true,
        |volume| {
            file_in_root(volume, "gone", 8000);
            let directory = volume.create_directory_in_root("full", now()).unwrap();
            volume
                .create_file_in_directory(directory, "child", b"child", now())
                .unwrap();
        },
        |volume| {
            vec![
                format!("{:?}", volume.delete_file_in_root("gone", now())),
                format!("{:?}", volume.remove_directory(OBJECT_ROOT, "full", now())),
                format!("{:?}", volume.delete_file_in_root("gone", now())),
            ]
        },
    );
}

#[test]
fn family_link_file_publishes_and_refuses_an_existing_name() {
    qualify_family(
        "link_file",
        false,
        true,
        |volume| {
            file_in_root(volume, "target", 8000);
        },
        |volume| {
            let target = volume.lookup_root("target").unwrap().unwrap();
            vec![
                format!(
                    "{:?}",
                    volume.link_file(target, OBJECT_ROOT, "second", now())
                ),
                format!(
                    "{:?}",
                    volume.link_file(target, OBJECT_ROOT, "second", now())
                ),
            ]
        },
    );
}

#[test]
fn family_rename_internal_publishes_moves_replacements_and_refusals() {
    qualify_family(
        "rename_internal",
        false,
        true,
        |volume| {
            file_in_root(volume, "a", 8000);
            file_in_root(volume, "b", 8000);
        },
        |volume| {
            vec![
                format!(
                    "{:?}",
                    volume.rename(OBJECT_ROOT, "a", OBJECT_ROOT, "moved", now())
                ),
                format!(
                    "{:?}",
                    volume.rename_replace(OBJECT_ROOT, "moved", OBJECT_ROOT, "b", now())
                ),
                format!(
                    "{:?}",
                    volume.rename(OBJECT_ROOT, "missing", OBJECT_ROOT, "c", now())
                ),
                format!(
                    "{:?}",
                    volume.rename_replace_orphan_target(OBJECT_ROOT, "b", OBJECT_ROOT, "c", now())
                ),
            ]
        },
    );
}

#[test]
fn family_commit_staged_file_layout_publishes_writes_truncates_and_reservations() {
    qualify_family(
        "commit_staged_file_layout",
        false,
        true,
        |volume| {
            file_in_root(volume, "file", 12000);
            volume.create_directory_in_root("dir", now()).unwrap();
        },
        |volume| {
            let file = volume.lookup_root("file").unwrap().unwrap();
            let directory = volume.lookup_root("dir").unwrap().unwrap();
            vec![
                format!("{:?}", volume.write_file_at(file, 100, &[3u8; 6000], now())),
                format!("{:?}", volume.truncate_file(file, 5000, now())),
                format!("{:?}", volume.preallocate_file(file, 0, 16384, now())),
                format!("{:?}", volume.write_file_at(directory, 0, b"no", now())),
            ]
        },
    );
}

#[test]
fn family_materialize_batch_publishes_one_checkpoint_per_batch() {
    use afsplus_core::volume::BatchOp;
    qualify_family(
        "materialize_batch",
        false,
        true,
        |volume| {
            file_in_root(volume, "kept", 8000);
        },
        |volume| {
            vec![
                format!(
                    "{:?}",
                    volume.run_batch(
                        &[
                            BatchOp::CreateFile {
                                parent_id: OBJECT_ROOT,
                                name: "one",
                                content: b"one",
                            },
                            BatchOp::CreateFile {
                                parent_id: OBJECT_ROOT,
                                name: "two",
                                content: b"two",
                            },
                        ],
                        now(),
                    )
                ),
                format!(
                    "{:?}",
                    volume.run_batch(
                        &[BatchOp::CreateFile {
                            parent_id: OBJECT_ROOT,
                            name: "kept",
                            content: b"clash",
                        }],
                        now(),
                    )
                ),
            ]
        },
    );
}

#[test]
fn family_commit_object_metadata_publishes_protection_and_restoration() {
    qualify_family(
        "commit_object_metadata",
        false,
        true,
        |volume| {
            file_in_root(volume, "file", 8000);
        },
        |volume| {
            let file = volume.lookup_root("file").unwrap().unwrap();
            let metadata = volume.visible_metadata(file).unwrap().unwrap();
            vec![
                format!("{:?}", volume.set_object_protection(file, 0o600, now())),
                format!(
                    "{:?}",
                    volume.restore_object_metadata(file, metadata.into())
                ),
                format!("{:?}", volume.set_object_protection(u64::MAX, 0o600, now())),
            ]
        },
    );
}

#[test]
fn family_commit_snapshot_change_publishes_registry_edits_with_view_identity() {
    qualify_family(
        "commit_snapshot_change",
        true,
        true,
        |volume| {
            file_in_root(volume, "captured", 8000);
        },
        |volume| {
            let created = volume.snapshot_create(now());
            let id = created.as_ref().copied().unwrap_or(0);
            vec![
                format!("{created:?}"),
                format!("{:?}", volume.snapshot_maintenance_step(now())),
                format!("{:?}", volume.snapshot_delete(id, now())),
                format!("{:?}", volume.snapshot_delete(9999, now())),
            ]
        },
    );
}

#[test]
fn family_deferred_window_publication_joins_groups_and_one_checkpoint() {
    use afsplus_core::volume::BatchOp;
    qualify_family(
        "deferred_window_publication",
        false,
        true,
        |volume| {
            file_in_root(volume, "kept", 8000);
        },
        |volume| {
            let mut results = Vec::new();
            results.push(format!(
                "{:?}",
                volume.window_op(
                    &BatchOp::CreateFile {
                        parent_id: OBJECT_ROOT,
                        name: "staged",
                        content: &[5u8; 9000],
                    },
                    now(),
                )
            ));
            results.push(format!("{:?}", volume.window_fsync()));
            results.push(format!(
                "{:?}",
                volume.window_op(
                    &BatchOp::CreateFile {
                        parent_id: u64::MAX,
                        name: "invalid",
                        content: b"",
                    },
                    now(),
                )
            ));
            results.push(format!("{:?}", volume.window_commit(now())));
            results
        },
    );
}

#[test]
fn family_snapshot_maintenance_failure_names_its_view() {
    use afsplus_core::flight::{Category, ReadPath};
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mount_with_snapshot_limits, MountOptions};
    let base = family_image(true);
    let device = SharedDevice::new(&base);
    let mut volume = mount_with_snapshot_limits(
        device,
        MountOptions::default(),
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 1,
            reclaim_records: 8,
        },
    )
    .unwrap();
    file_in_root(&mut volume, "captured", 8000);
    let id = volume.snapshot_create(now()).unwrap();
    volume.replace_flight_recorder(Some(family_recorder(4096)));
    // A second view exceeds the admission limit, and an unknown view cannot
    // be deleted; both failures name the view they were applied to.
    assert!(volume.snapshot_create(now()).is_err());
    assert!(volume.snapshot_delete(4242, now()).is_err());
    let handle = volume.snapshot_open(id).unwrap();
    assert!(
        volume.snapshot_delete(id, now()).is_err(),
        "a held view is busy"
    );
    drop(handle);
    let ring = volume.replace_flight_recorder(None).unwrap();
    let failures: Vec<_> = ring
        .events()
        .filter(|event| event.kind == EventKind::ViewMaintenanceFailed)
        .map(view_context)
        .collect();
    assert_eq!(failures.len(), 3);
    assert!(failures
        .iter()
        .all(|context| context.path == ReadPath::Maintenance && context.block != 0));
    assert_eq!(
        failures
            .iter()
            .map(|context| context.view_id)
            .collect::<Vec<_>>(),
        [0, 4242, id]
    );
    assert!(ring
        .events()
        .all(|event| event.kind.category() != Category::View || event.attempt == 0));
}

#[test]
fn replacing_a_recorder_mid_window_keeps_staged_write_and_read_observation() {
    use afsplus_core::flight::Category;
    use afsplus_core::volume::BatchOp;
    let mut volume = mount(window_image(false)).unwrap();
    let mut first = recorder(4096);
    first.enable_data_observation();
    first.enable_view_observation();
    volume.replace_flight_recorder(Some(first));
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "first",
                content: &[1u8; 9000],
            },
            now(),
        )
        .unwrap();
    let first = volume.replace_flight_recorder(None).unwrap();
    assert!(first
        .events()
        .any(|event| event.kind == EventKind::DataWriteComplete));
    let window = first
        .events()
        .find(|event| event.window != 0)
        .map(|event| event.window)
        .expect("the open window has an identity");

    let mut second = recorder(4096);
    second.enable_data_observation();
    second.enable_view_observation();
    volume.replace_flight_recorder(Some(second));
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "second",
                content: &[2u8; 9000],
            },
            now(),
        )
        .unwrap();
    volume.window_commit(now()).unwrap();
    volume.lookup_root("second").unwrap().unwrap();
    let second = volume.replace_flight_recorder(None).unwrap();
    let staged: Vec<_> = second
        .events()
        .filter(|event| event.kind.category() == Category::Data)
        .collect();
    assert!(!staged.is_empty(), "the replacement observes staged writes");
    assert_eq!(
        staged[0].window, 1,
        "a replacement starts its own window identity"
    );
    assert_eq!(window, 1);
    assert!(second
        .events()
        .any(|event| event.kind.category() == Category::View));
    assert_eq!(volume.list_root().unwrap().len(), 2);
}
