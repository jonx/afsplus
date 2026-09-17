//! Health log: classification, bounded ring with loss accounting, snapshot.

use afsplus_aros::health::{
    HealthEvent, HealthEventKind, HealthLog, HEALTH_CORRUPTION, HEALTH_DEVICE_ERROR,
    HEALTH_INTERNAL_FAULT, HEALTH_REPLAY_PENDING,
};
use afsplus_aros::{ArosAdapter, ArosConfig, ArosError};
use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountMode, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

const EMPTY: HealthEvent = HealthEvent {
    sequence: 0,
    kind: HealthEventKind::InternalFault,
    dos_error: 0,
};

#[test]
fn only_volume_and_device_errors_are_health_events_and_loss_is_counted() {
    let mut log = HealthLog::new(2);
    // Ordinary results describe the request, not the volume.
    for ordinary in [
        ArosError::ObjectNotFound,
        ArosError::ObjectExists,
        ArosError::ObjectInUse,
        ArosError::DiskWriteProtected,
        ArosError::InvalidLock,
    ] {
        log.record(ordinary);
    }
    let mut drained = [EMPTY; 4];
    assert_eq!(log.drain(&mut drained), 0);

    log.record(ArosError::Unknown);
    log.record(ArosError::NotDosDisk);
    log.record(ArosError::DiskFull);
    log.record_internal_fault();
    // Capacity two: sequences 1 and 2 gave way to 3 and 4.
    assert_eq!(log.drain(&mut drained), 2);
    assert_eq!(
        drained[..2],
        [
            HealthEvent {
                sequence: 3,
                kind: HealthEventKind::NoSpace,
                dos_error: 221,
            },
            HealthEvent {
                sequence: 4,
                kind: HealthEventKind::InternalFault,
                dos_error: 100,
            },
        ]
    );
    assert_eq!(log.drain(&mut drained), 0);
}

#[test]
fn snapshot_reports_volume_state_counters_and_degraded_flags() {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC9; 16],
            label: "Health".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
        },
    )
    .unwrap();
    let vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            health_event_capacity: 1,
            ..ArosConfig::default()
        },
    );

    // A fresh volume is healthy: no flags, no counters, no pending work.
    let clean = adapter.health().unwrap();
    assert_eq!(clean.mount_mode, MountMode::ReadWrite);
    assert_eq!(clean.flags, 0);
    assert_eq!(clean.generation, 1);
    assert_eq!(clean.pending_intent_records, 0);
    assert_eq!(clean.pending_orphans, 0);
    assert_eq!(clean.total_blocks, 8192);
    assert!(clean.available_blocks <= clean.free_blocks);
    assert_eq!(
        (
            clean.device_errors,
            clean.corruption_errors,
            clean.no_space_errors,
            clean.internal_faults,
            clean.events_recorded,
            clean.events_dropped,
            clean.last_error,
        ),
        (0, 0, 0, 0, 0, 0, 0)
    );

    adapter.health_log().record(ArosError::Unknown);
    adapter.health_log().record(ArosError::Unknown);
    adapter.health_log().record(ArosError::DiskFull);
    let degraded = adapter.health().unwrap();
    // Disk-full is counted and is not a degraded state.
    assert_eq!(degraded.flags, HEALTH_DEVICE_ERROR);
    assert_eq!(degraded.device_errors, 2);
    assert_eq!(degraded.no_space_errors, 1);
    assert_eq!(degraded.events_recorded, 3);
    assert_eq!(degraded.events_dropped, 2);
    assert_eq!(degraded.last_error, 221);

    adapter.health_log().record(ArosError::NotDosDisk);
    adapter.health_log().record_internal_fault();
    let flags = adapter.health().unwrap().flags;
    assert_eq!(
        flags,
        HEALTH_DEVICE_ERROR | HEALTH_CORRUPTION | HEALTH_INTERNAL_FAULT
    );
    assert_eq!(flags & HEALTH_REPLAY_PENDING, 0);
}

#[test]
fn read_only_view_of_an_unreplayed_log_reports_replay_pending() {
    use afsplus_aros::OpenMode;

    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xCA; 16],
            label: "Pending".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
        },
    )
    .unwrap();
    let now = Timespec {
        seconds: 1,
        nanoseconds: 0,
    };
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    let file = vfs.create_file(vfs.root_object(), "logged", now).unwrap();
    let handle = vfs
        .open_file(file, afsplus_vfs::AccessMode::ReadWrite)
        .unwrap();
    vfs.write(handle, 0, b"durable through the log", now)
        .unwrap();
    // fsync makes the write durable as one intent-log record; the process
    // then stops without publishing a checkpoint.
    vfs.fsync(handle).unwrap();
    let device = vfs.into_volume().into_device();

    let read_only = Vfs::mount(
        device,
        MountOptions {
            mode: MountMode::ReadOnly,
            ..MountOptions::default()
        },
    )
    .unwrap();
    let mut adapter = ArosAdapter::new(read_only, ArosConfig::default());
    let health = adapter.health().unwrap();
    assert_eq!(health.mount_mode, MountMode::ReadOnly);
    assert_eq!(health.pending_intent_records, 1);
    assert_eq!(health.flags, HEALTH_REPLAY_PENDING);
    // The exposed checkpoint predates the logged write.
    let logged = adapter
        .open(None, b"logged", OpenMode::OldFile, now)
        .unwrap();
    assert_eq!(adapter.file_size(logged).unwrap(), 0);
    adapter.close(logged).unwrap();

    // Control: replaying the same image read-write clears the state.
    let device = adapter.into_vfs().unwrap().into_volume().into_device();
    let mut adapter = ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig::default(),
    );
    let health = adapter.health().unwrap();
    assert_eq!(health.pending_intent_records, 0);
    assert_eq!(health.flags, 0);
    let logged = adapter
        .open(None, b"logged", OpenMode::OldFile, now)
        .unwrap();
    assert_eq!(adapter.file_size(logged).unwrap(), 23);
}
