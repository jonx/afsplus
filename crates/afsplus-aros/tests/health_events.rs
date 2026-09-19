//! The health events no failed call can report: a mount that had to read the
//! older checkpoint of the pair, and a checkpoint whose free-block total
//! disagrees with its own regions. Each has its control: the same volume
//! undamaged records nothing.

use afsplus_aros::health::{HealthEvent, HealthEventKind};
use afsplus_aros::{ArosAdapter, ArosConfig};
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::{AccessMode, Vfs};

const EMPTY: HealthEvent = HealthEvent {
    sequence: 0,
    kind: HealthEventKind::InternalFault,
    dos_error: 0,
};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

/// Two committed generations: the format writes slot A, and each write plus
/// sync publishes the other slot, so both slots of the pair are in use.
fn volume_with_both_slots_used() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x5C; 16],
            label: "Events".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    let mut vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    for (index, name) in ["first", "second"].into_iter().enumerate() {
        let object = vfs
            .create_file(afsplus_format::OBJECT_ROOT, name, at(index as i64 + 1))
            .unwrap();
        let handle = vfs.open_file(object, AccessMode::WriteOnly).unwrap();
        vfs.write(handle, 0, b"payload", at(index as i64 + 1))
            .unwrap();
        vfs.close(handle).unwrap();
        vfs.sync_filesystem().unwrap();
    }
    assert!(vfs.generation() >= 3, "both slots carry a checkpoint");
    vfs.into_volume().into_device()
}

/// The checkpoint slot (LBA 1 or 2) whose header claims the newer generation.
fn newer_slot(device: &mut MemoryBackend) -> u64 {
    let mut block = vec![0u8; 4096];
    let mut newest = (0u64, 0u64);
    for lba in [1u64, 2] {
        device.read_block(lba, &mut block).unwrap();
        let generation = u64::from_le_bytes(block[16..24].try_into().unwrap());
        if generation > newest.1 {
            newest = (lba, generation);
        }
    }
    assert_ne!(newest.0, 0, "a written slot");
    newest.0
}

fn mounted(device: MemoryBackend) -> ArosAdapter<MemoryBackend> {
    let vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    ArosAdapter::new(
        vfs,
        ArosConfig {
            health_event_capacity: 8,
            ..ArosConfig::default()
        },
    )
}

#[test]
fn a_mount_that_reads_the_older_checkpoint_records_one_fallback_event() {
    let mut device = volume_with_both_slots_used();
    let newer = newer_slot(&mut device);
    let mut block = vec![0u8; 4096];
    device.read_block(newer, &mut block).unwrap();
    let published = u64::from_le_bytes(block[16..24].try_into().unwrap());
    // Damage the payload, not the header: the slot still says which
    // generation it was, and its checksum no longer covers what it holds.
    for byte in &mut block[64..96] {
        *byte ^= 0xFF;
    }
    device.write_block(newer, &block).unwrap();
    device.flush().unwrap();

    let mut adapter = mounted(device);
    // The mount is clean and shows the older checkpoint of the pair.
    let health = adapter.health().unwrap();
    assert_eq!(health.generation, published - 1);
    assert_eq!(health.checkpoint_fallbacks, 1);
    assert_eq!(health.events_recorded, 1);
    assert_eq!(health.flags, 0, "a fallback mount is not degraded");
    assert_eq!(health.last_error, 0);
    let lock = adapter
        .locate(None, b"", afsplus_aros::LockAccess::Shared)
        .unwrap();
    adapter.free_lock(lock).unwrap();

    let mut drained = [EMPTY; 4];
    assert_eq!(adapter.health_log().drain(&mut drained), 1);
    assert_eq!(
        drained[0],
        HealthEvent {
            sequence: 1,
            kind: HealthEventKind::CheckpointFallback,
            dos_error: 0,
        }
    );
    // Recorded once, not once per query.
    assert_eq!(adapter.health().unwrap().checkpoint_fallbacks, 1);
}

#[test]
fn an_intact_checkpoint_pair_records_no_fallback() {
    let mut adapter = mounted(volume_with_both_slots_used());
    let health = adapter.health().unwrap();
    assert_eq!(
        (health.checkpoint_fallbacks, health.events_recorded),
        (0, 0)
    );
    let mut drained = [EMPTY; 4];
    assert_eq!(adapter.health_log().drain(&mut drained), 0);
}

/// A slot that was never written claims nothing: the volume formatted and
/// committed once has an empty second slot, which is not a fallback.
#[test]
fn an_unwritten_second_slot_is_not_a_fallback() {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x5D; 16],
            label: "Fresh".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    let mut adapter = mounted(device);
    let health = adapter.health().unwrap();
    assert_eq!((health.checkpoint_fallbacks, health.generation), (0, 1));
}

/// Re-encodes the newest checkpoint with `free_blocks_total` off by one and a
/// checksum that covers the lie: the record is admissible, and only its own
/// regions contradict it.
fn falsify_free_count(device: &mut MemoryBackend) -> u64 {
    const HEADER: usize = 32;
    const FREE_TOTAL: usize = HEADER + 72;
    let slot = newer_slot(device);
    let mut block = vec![0u8; 4096];
    device.read_block(slot, &mut block).unwrap();
    let stated = u64::from_le_bytes(block[FREE_TOTAL..FREE_TOTAL + 8].try_into().unwrap());
    block[FREE_TOTAL..FREE_TOTAL + 8].copy_from_slice(&(stated + 1).to_le_bytes());
    block[28..32].copy_from_slice(&[0; 4]);
    let sum = afsplus_format::crc32c::crc32c(&block);
    block[28..32].copy_from_slice(&sum.to_le_bytes());
    device.write_block(slot, &block).unwrap();
    device.flush().unwrap();
    stated
}

#[test]
fn a_checkpoint_free_count_its_regions_contradict_records_one_event() {
    let mut device = volume_with_both_slots_used();
    let stated = falsify_free_count(&mut device);

    let mut adapter = mounted(device);
    let health = adapter.health().unwrap();
    assert_eq!(health.free_count_mismatches, 1);
    assert_eq!(health.events_recorded, 1);
    assert_eq!(
        health.free_blocks,
        stated + 1,
        "the mount believes the record"
    );

    let mut drained = [EMPTY; 4];
    assert_eq!(adapter.health_log().drain(&mut drained), 1);
    assert_eq!(
        drained[0],
        HealthEvent {
            sequence: 1,
            kind: HealthEventKind::RegionFreeCountMismatch,
            dos_error: 0,
        }
    );
    // Once per mount, not once per query.
    assert_eq!(adapter.health().unwrap().free_count_mismatches, 1);
}

#[test]
fn a_checkpoint_that_agrees_with_its_regions_records_no_mismatch() {
    let mut adapter = mounted(volume_with_both_slots_used());
    let health = adapter.health().unwrap();
    assert_eq!(
        (health.free_count_mismatches, health.events_recorded),
        (0, 0)
    );
}

/// The backlog note of the VFS is an event of the handler's log, with its own
/// counter. The backlog itself is built through the VFS (its own test covers
/// the threshold and the hysteresis); what is proven here is the crossing.
#[test]
fn a_reclaim_backlog_reaches_the_handler_as_one_event() {
    let mut device = MemoryBackend::new(4096, 32_768);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x5E; 16],
            label: "Backlog".into(),
            region_size: 8192,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    let mut volume = afsplus_core::mount_with_options(device, MountOptions::default()).unwrap();
    volume.set_reclaim_batch_blocks(64);
    let mut vfs = Vfs::new(volume);
    vfs.set_durability(afsplus_vfs::Durability::DELAYED)
        .unwrap();
    vfs.set_inline_maintenance(false);
    vfs.set_idle_maintenance(false);

    let chunk = vec![0xA5u8; 1 << 20];
    let object = vfs
        .create_file(afsplus_format::OBJECT_ROOT, "big", at(1))
        .unwrap();
    let handle = vfs.open_file(object, AccessMode::WriteOnly).unwrap();
    for index in 0..24u64 {
        vfs.write(handle, index << 20, &chunk, at(1)).unwrap();
    }
    vfs.close(handle).unwrap();
    vfs.sync_filesystem().unwrap();
    vfs.unlink_file(afsplus_format::OBJECT_ROOT, "big", at(2))
        .unwrap();
    vfs.sync_filesystem().unwrap();
    vfs.cleanup_orphans(4, at(3)).unwrap();
    assert!(
        vfs.reclaim_backlog_high(),
        "the backlog is past the threshold"
    );

    // The adapter takes the mount with the note still on it.
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            health_event_capacity: 8,
            ..ArosConfig::default()
        },
    );
    let health = adapter.health().unwrap();
    assert_eq!(health.reclaim_backlog_highs, 1);
    assert_eq!(health.events_recorded, 1);
    assert_eq!(health.flags, 0, "a backlog is not a degraded state");

    let mut drained = [EMPTY; 4];
    assert_eq!(adapter.health_log().drain(&mut drained), 1);
    assert_eq!(
        drained[0],
        HealthEvent {
            sequence: 1,
            kind: HealthEventKind::ReclaimBacklogHigh,
            dos_error: 0,
        }
    );
}
