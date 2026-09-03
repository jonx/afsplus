//! Object-record header generation must be bounded by the selected
//! checkpoint, exactly as every tree-node access already is: a valid-CRC
//! record left behind by an uncommitted future transaction (for example on
//! a reused block reachable through a stale map entry after a crash) must
//! be rejected by the read path and by the checker, never decoded as
//! current state.

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, object_map, CoreError, MkfsParams};
use afsplus_format::object::ObjectRecord;
use afsplus_format::Timespec;

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [9u8; 16],
            label: "GenVol".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(1),
        },
    )
    .unwrap();
    dev
}

/// Creates one committed file and returns the device, the file's object ID,
/// its record LBA and the committed checkpoint generation.
fn device_with_victim() -> (MemoryBackend, u64, u64, u64) {
    let dev = formatted();
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("victim", &vec![0x5Au8; BS], ts(2))
        .unwrap();
    let (map_root, committed) = {
        let checkpoint = vol.checkpoint();
        (checkpoint.object_map_block, checkpoint.generation)
    };
    let geo = vol.ident().geometry();
    let lba = object_map::lookup_lba(vol.device_mut(), &geo, map_root, committed, id)
        .unwrap()
        .expect("victim is mapped");
    (vol.into_device(), id, lba, committed)
}

/// Re-seals the record block byte-for-byte with a forged header generation,
/// bypassing every write path. The payload and CRC stay valid.
fn reseal_record_generation(dev: &mut MemoryBackend, lba: u64, generation: u64) {
    let mut block = vec![0u8; BS];
    dev.read_block(lba, &mut block).unwrap();
    let record = ObjectRecord::decode(&block).unwrap();
    let forged = record.encode(BS, generation).unwrap();
    dev.write_block(lba, &forged).unwrap();
}

fn assert_fails_closed(mut dev: MemoryBackend, id: u64) {
    // Read path: the record must be rejected, not served.
    let mut vol = mount(dev).unwrap();
    let error = vol.stat(id).unwrap_err();
    assert!(
        matches!(&error, CoreError::Corrupt(text)
            if text.contains("outside committed range")),
        "{error:?}"
    );

    // Checker: the same bound, reported.
    dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(!report.is_clean());
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("outside committed range")),
        "checker findings: {:?}",
        report.errors
    );
}

#[test]
fn future_generation_object_record_fails_closed() {
    let (mut dev, id, lba, committed) = device_with_victim();
    reseal_record_generation(&mut dev, lba, committed + 1);
    assert_fails_closed(dev, id);
}

#[test]
fn maximum_generation_object_record_fails_closed() {
    let (mut dev, id, lba, _) = device_with_victim();
    reseal_record_generation(&mut dev, lba, u64::MAX);
    assert_fails_closed(dev, id);
}

#[test]
fn zero_generation_object_record_fails_closed() {
    let (mut dev, id, lba, _) = device_with_victim();
    reseal_record_generation(&mut dev, lba, 0);
    assert_fails_closed(dev, id);
}

#[test]
fn committed_generation_object_record_still_reads() {
    // Control: re-sealing with the exact committed generation is a no-op for
    // acceptance — the bound rejects only 0 and the future.
    let (mut dev, id, lba, committed) = device_with_victim();
    reseal_record_generation(&mut dev, lba, committed);
    let mut vol = mount(dev).unwrap();
    assert!(vol.stat(id).unwrap().is_some());
    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}
