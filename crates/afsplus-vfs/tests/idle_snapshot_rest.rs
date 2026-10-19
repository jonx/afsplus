// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! A delayed mount comes to rest: once nothing changes, the idle ticks stop
//! committing, with or without persistent snapshots.
use afsplus_block::MemoryBackend;
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{mkfs_with_options, MkfsOptions, MkfsParams, MountOptions, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{Durability, Vfs};

fn mounted(snapshots: bool) -> Vfs<MemoryBackend> {
    let mut dev = MemoryBackend::new(4096, 8192);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [0x96; 16],
            label: "SYS".into(),
            region_size: 262_144,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec::default(),
        },
        MkfsOptions {
            persistent_snapshots: snapshots,
        },
    )
    .unwrap();
    let mut vfs = if snapshots {
        Vfs::mount_with_snapshot_limits(
            dev,
            MountOptions::default(),
            SnapshotWorkLimits {
                max_edit_records: 4096,
                max_views: 8,
                reclaim_records: 64,
            },
        )
        .unwrap()
    } else {
        Vfs::mount(dev, MountOptions::default()).unwrap()
    };
    vfs.set_durability(Durability::DELAYED).unwrap();
    vfs
}

/// Ticks every 50 ms, as the AROS handler does, for `seconds`; returns the
/// commits made after the first three idle seconds.
fn settle(vfs: &mut Vfs<MemoryBackend>, start: i64, seconds: i64) -> u64 {
    let mut late = 0;
    let mut previous = vfs.generation();
    for tick in 0..seconds * 20 {
        let now = Timespec {
            seconds: start + tick / 20,
            nanoseconds: (tick % 20) as u32 * 50_000_000,
        };
        vfs.commit_if_due(now).unwrap();
        let generation = vfs.generation();
        if tick >= 3 * 20 && generation != previous {
            late += 1;
        }
        previous = generation;
    }
    late
}

fn workload(vfs: &mut Vfs<MemoryBackend>) {
    let now = Timespec {
        seconds: 100,
        nanoseconds: 0,
    };
    let file = vfs.create_file(OBJECT_ROOT, "dhcpd.log", now).unwrap();
    let handle = vfs
        .open_file(file, afsplus_vfs::AccessMode::ReadWrite)
        .unwrap();
    vfs.write(handle, 0, &[7; 9000], now).unwrap();
    vfs.write(handle, 9000, b"lease\n", now).unwrap();
    vfs.close(handle).unwrap();
}

#[test]
fn an_idle_snapshot_volume_stops_committing() {
    for snapshots in [false, true] {
        let mut vfs = mounted(snapshots);
        workload(&mut vfs);
        let late = settle(&mut vfs, 100, 60);
        assert_eq!(
            late, 0,
            "snapshots={snapshots}: {late} commits on an idle volume"
        );
    }
}
