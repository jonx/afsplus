// Copyright (C) 2026 John KNIPPER <code@jkn.me>.
// SPDX-License-Identifier: APL-1.1
//! Real COW range transactions. Page freezing/coherence and guest msync remain gates.
mod common;
use afsplus_aros_ffi::*;
use afsplus_block::{for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::volume::DataUpdatePolicy;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::Timespec;
const PAGE: usize = 4096;
fn bytes(seed: u8) -> Vec<u8> {
    (0..3 * PAGE)
        .map(|i| seed.wrapping_add((i / PAGE * 29 + i * 13) as u8))
        .collect()
}
fn baseline(inode_policy: bool) -> (MemoryBackend, u64, Vec<u8>) {
    let mut device = MemoryBackend::new(PAGE, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x63; 16],
            label: "MappedWriteback".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: false,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    let mut volume = mount(device).unwrap();
    let old = bytes(7);
    let file = volume
        .create_file_in_root("range", &old, Timespec::default())
        .unwrap();
    if inode_policy {
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, Timespec::default())
            .unwrap();
    }
    let mut device = volume.into_device();
    device.flush().unwrap();
    (device, file, old)
}
#[test]
fn forced_cow_does_not_change_global_or_inode_policy() {
    for (global, inode) in [(true, false), (false, true), (true, true)] {
        let (device, file, _) = baseline(inode);
        let mut volume = mount(device).unwrap();
        let policy = if global {
            DataUpdatePolicy::InPlacePrivate
        } else {
            DataUpdatePolicy::FullCow
        };
        volume.set_data_update_policy(policy);
        let before = volume.file_data_policy(file).unwrap();
        volume
            .write_file_at_atomic_cow(file, 0, &bytes(0x91), Timespec::default())
            .unwrap();
        let stats = volume.last_commit_stats().unwrap();
        assert_eq!(stats.data_blocks_written, 3);
        assert_eq!(stats.data_blocks_overwritten_in_place, 0);
        assert_eq!(volume.data_update_policy(), policy);
        assert_eq!(volume.file_data_policy(file).unwrap(), before);
        let mut read = vec![0; 3 * PAGE];
        volume.read_file_at(file, 0, &mut read).unwrap();
        assert_eq!(read, bytes(0x91));
        let mut device = volume.into_device();
        let report = check_device(&mut device);
        assert!(report.is_clean(), "{:?}", report.errors);
    }
}
#[test]
fn every_range_crash_state_is_wholly_old_or_new_and_control_tears() {
    let (base, file, old) = baseline(true);
    let new = bytes(0xa3);
    let mut volume = mount(RecordingBackend::new(base.clone())).unwrap();
    volume.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
    volume
        .write_file_at_atomic_cow(file, 0, &new, Timespec::default())
        .unwrap();
    let (_, operations) = volume.into_device().into_parts();
    let mut cases = 0;
    let mut old_cases = 0;
    let mut new_cases = 0;
    for cut in 0..=operations.len() {
        for_each_crash_state(&base, &operations, cut, |state| {
            let mut volume = mount(state.image.clone()).unwrap();
            let mut read = vec![0; old.len()];
            volume.read_file_at(file, 0, &mut read).unwrap();
            assert!(read == old || read == new, "torn atomic range at cut {cut}");
            if read == old {
                old_cases += 1;
            } else {
                new_cases += 1;
            }
            let mut device = state.image;
            let report = check_device(&mut device);
            assert!(report.is_clean(), "cut {cut}: {:?}", report.errors);
            cases += 1;
        });
    }
    assert!(cases > operations.len() && old_cases > 0 && new_cases > 0);
    let mut volume = mount(RecordingBackend::new(base.clone())).unwrap();
    volume.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
    volume
        .write_file_at(file, 0, &new, Timespec::default())
        .unwrap();
    assert!(
        volume
            .last_commit_stats()
            .unwrap()
            .data_blocks_overwritten_in_place
            > 0
    );
    let (_, control) = volume.into_device().into_parts();
    let mut torn = 0;
    for cut in 0..=control.len() {
        for_each_crash_state(&base, &control, cut, |state| match mount(state.image) {
            Err(_) => torn += 1,
            Ok(mut volume) => {
                let mut read = vec![0; old.len()];
                if volume.read_file_at(file, 0, &mut read).is_err() || (read != old && read != new)
                {
                    torn += 1;
                }
            }
        });
    }
    assert!(torn > 0, "control did not exercise in-place crash race");
    eprintln!("atomic writeback cases={cases} old={old_cases} new={new_cases} control_torn={torn}");
}
#[test]
fn ffi_atomic_range_is_positioned_durable_and_refuses_extension_or_readonly() {
    let mut device = common::formatted(true);
    let fs = common::mount(&mut device);
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            0,
            b"source".as_ptr(),
            6,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            1,
            0,
            &mut file
        ),
        0
    );
    let original = bytes(9);
    let update = bytes(0x83);
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write_at(
            fs,
            file,
            0,
            original.as_ptr(),
            original.len() as u32,
            2,
            0,
            &mut count
        ),
        0
    );
    let mut old_position = 0;
    assert_eq!(
        afsplus_aros_seek(
            fs,
            file,
            123,
            AFSPLUS_AROS_SEEK_BEGINNING,
            &mut old_position
        ),
        0
    );
    assert_eq!(
        afsplus_aros_writeback_atomic(
            fs,
            file,
            0,
            update.as_ptr(),
            update.len() as u32,
            3,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(count as usize, update.len());
    let mut position = 0;
    assert_eq!(afsplus_aros_file_position(fs, file, &mut position), 0);
    assert_eq!(position, 123);
    count = 0xdead;
    assert_eq!(
        afsplus_aros_writeback_atomic(
            fs,
            file,
            1,
            update.as_ptr(),
            update.len() as u32,
            4,
            0,
            &mut count
        ),
        210
    );
    assert_eq!(count, 0xdead);
    let mut snapshot = 0;
    assert_eq!(
        afsplus_aros_clone_open_snapshot(fs, file, 5, 0, &mut snapshot),
        0
    );
    assert_eq!(
        afsplus_aros_writeback_atomic(
            fs,
            snapshot,
            0,
            update.as_ptr(),
            update.len() as u32,
            6,
            0,
            &mut count
        ),
        214
    );
    assert_eq!(afsplus_aros_close(fs, snapshot), 0);
    assert_eq!(afsplus_aros_close(fs, file), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
    let fs = common::mount(&mut device);
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            0,
            b"source".as_ptr(),
            6,
            AFSPLUS_AROS_OPEN_OLD_FILE,
            7,
            0,
            &mut file
        ),
        0
    );
    let mut read = vec![0; update.len()];
    assert_eq!(
        afsplus_aros_read_at(
            fs,
            file,
            0,
            read.as_mut_ptr(),
            read.len() as u32,
            &mut count
        ),
        0
    );
    assert_eq!(read, update);
    assert_eq!(afsplus_aros_close(fs, file), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}
