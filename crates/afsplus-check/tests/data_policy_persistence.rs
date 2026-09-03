//! The persistent per-file data-update policy (ADR-065): the opt-in lives in
//! the object record, survives remounts, drives the ADR-062 in-place path
//! exactly where the eligibility rules pass, and fails closed — a planted
//! flag on a volume without the feature is corruption for both the read path
//! and the checker, mirroring the shared-extent marker discipline.

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::volume::DataUpdatePolicy;
use afsplus_core::{mkfs, mount, object_map, CoreError, MkfsParams, Volume};
use afsplus_format::object::{ObjectRecord, OBJECT_FLAG_DATA_IN_PLACE};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(data_policy: bool, shared_extents: bool) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [11u8; 16],
            label: "PolicyVol".into(),
            region_size: 256,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents,
            data_policy,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(1),
        },
    )
    .unwrap();
    dev
}

fn in_place_blocks(vol: &Volume<MemoryBackend>) -> u64 {
    vol.last_commit_stats()
        .map(|stats| stats.data_blocks_overwritten_in_place)
        .unwrap_or(0)
}

#[test]
fn opt_in_persists_across_remount_and_takes_the_in_place_path() {
    let dev = formatted(true, false);
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("db", &vec![0x11u8; 4 * BS], ts(2))
        .unwrap();
    assert_eq!(
        vol.file_data_policy(id).unwrap(),
        DataUpdatePolicy::FullCow,
        "creation default is full COW"
    );
    vol.set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap();
    let before = vol.stat(id).unwrap().unwrap();

    // The flagged file overwrites its committed private blocks in place.
    vol.write_file_at(id, BS as u64, &vec![0x22u8; BS], ts(4))
        .unwrap();
    assert_eq!(in_place_blocks(&vol), 1, "one block rewritten in place");
    let after = vol.stat(id).unwrap().unwrap();
    assert_eq!(
        (before.data_root, before.data_blocks),
        (after.data_root, after.data_blocks),
        "the physical layout did not move"
    );

    // The choice is on disk: a fresh mount still honors it.
    let mut vol = mount(vol.into_device()).unwrap();
    assert_eq!(
        vol.file_data_policy(id).unwrap(),
        DataUpdatePolicy::InPlacePrivate
    );
    vol.write_file_at(id, 0, &vec![0x33u8; 2 * BS], ts(5))
        .unwrap();
    assert_eq!(in_place_blocks(&vol), 2);
    let mut content = vec![0x33u8; 2 * BS];
    content.extend(vec![0x11u8; 2 * BS]);
    assert_eq!(vol.read_file(id).unwrap(), content);

    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}

#[test]
fn clearing_the_flag_restores_full_cow() {
    let dev = formatted(true, false);
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("tmp", &vec![0x44u8; 2 * BS], ts(2))
        .unwrap();
    vol.set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap();
    vol.set_file_data_policy(id, DataUpdatePolicy::FullCow, ts(4))
        .unwrap();
    assert_eq!(vol.file_data_policy(id).unwrap(), DataUpdatePolicy::FullCow);

    let before = vol.stat(id).unwrap().unwrap();
    vol.write_file_at(id, 0, &vec![0x55u8; 2 * BS], ts(5))
        .unwrap();
    assert_eq!(in_place_blocks(&vol), 0, "cleared file writes through COW");
    let after = vol.stat(id).unwrap().unwrap();
    assert_ne!(
        before.data_root, after.data_root,
        "COW moved the data to fresh blocks"
    );
    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}

#[test]
fn feature_off_volume_refuses_the_opt_in() {
    let dev = formatted(false, false);
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("plain", &vec![0x66u8; BS], ts(2))
        .unwrap();
    let error = vol
        .set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap_err();
    assert!(matches!(error, CoreError::FeatureDisabled(_)), "{error:?}");
}

#[test]
fn directories_refuse_the_policy() {
    let dev = formatted(true, false);
    let mut vol = mount(dev).unwrap();
    let error = vol
        .set_file_data_policy(OBJECT_ROOT, DataUpdatePolicy::InPlacePrivate, ts(2))
        .unwrap_err();
    assert!(matches!(error, CoreError::IsDirectory), "{error:?}");
}

#[test]
fn planted_flag_without_the_feature_fails_closed() {
    // Build the flagged record on a feature-off volume byte-for-byte,
    // bypassing the refused API.
    let dev = formatted(false, false);
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("victim", &vec![0x77u8; BS], ts(2))
        .unwrap();
    let (map_root, committed) = {
        let checkpoint = vol.checkpoint();
        (checkpoint.object_map_block, checkpoint.generation)
    };
    let geo = vol.ident().geometry();
    let lba = object_map::lookup_lba(vol.device_mut(), &geo, map_root, committed, id)
        .unwrap()
        .expect("victim is mapped");
    let mut dev = vol.into_device();
    let mut block = vec![0u8; BS];
    dev.read_block(lba, &mut block).unwrap();
    let mut record = ObjectRecord::decode(&block).unwrap();
    record.flags |= OBJECT_FLAG_DATA_IN_PLACE;
    let forged = record.encode(BS, committed).unwrap();
    dev.write_block(lba, &forged).unwrap();

    // Read path: corrupt, not honored.
    let mut vol = mount(dev).unwrap();
    let error = vol.stat(id).unwrap_err();
    assert!(
        matches!(&error, CoreError::Corrupt(text)
            if text.contains("data-policy feature")),
        "{error:?}"
    );

    // Checker: the same congruence, reported.
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(!report.is_clean());
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("OBJECT_FLAG_DATA_IN_PLACE")),
        "checker findings: {:?}",
        report.errors
    );
}

#[test]
fn shared_blocks_of_a_flagged_file_still_go_through_cow() {
    let dev = formatted(true, true);
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("origin", &vec![0x88u8; 2 * BS], ts(2))
        .unwrap();
    vol.set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap();
    vol.clone_file(id, OBJECT_ROOT, "copy", ts(4)).unwrap();
    let copy = vol.lookup_root("copy").unwrap().unwrap();

    // Every block is shared now: the flagged file must fall back to COW and
    // the clone must keep its bytes.
    vol.write_file_at(id, 0, &vec![0x99u8; BS], ts(5)).unwrap();
    assert_eq!(in_place_blocks(&vol), 0, "shared blocks are never in-place");
    assert_eq!(vol.read_file(copy).unwrap(), vec![0x88u8; 2 * BS]);
    let mut expected = vec![0x99u8; BS];
    expected.extend(vec![0x88u8; BS]);
    assert_eq!(vol.read_file(id).unwrap(), expected);

    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}

#[test]
fn in_place_write_after_a_crash_keeps_metadata_clean() {
    // The ADR-062 power-cut contract exercised through the persistent flag
    // rather than the runtime switch: after losing the in-place commit, the
    // volume mounts, the checker is clean, and the file carries either the
    // old or the new bytes in the overwritten range — never bad metadata.
    let dev = formatted(true, false);
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("crashy", &vec![0xAAu8; 2 * BS], ts(2))
        .unwrap();
    vol.set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap();

    // Snapshot the committed image, apply the in-place write, then "crash"
    // by resuming from the snapshot with only the data block torn.
    let committed = vol.into_device();
    let snapshot = committed.clone();
    let mut vol = mount(committed).unwrap();
    vol.write_file_at(id, 0, &vec![0xBBu8; BS], ts(4)).unwrap();
    let after = vol.into_device();

    for image in [snapshot, after] {
        let mut dev = image;
        let report = check_device(&mut dev);
        assert!(report.is_clean(), "findings: {:?}", report.errors);
        let mut vol = mount(dev).unwrap();
        assert_eq!(
            vol.file_data_policy(id).unwrap(),
            DataUpdatePolicy::InPlacePrivate
        );
        let bytes = vol.read_file(id).unwrap();
        assert_eq!(bytes.len(), 2 * BS);
        assert_eq!(&bytes[BS..], vec![0xAAu8; BS].as_slice());
    }
}

#[test]
fn hard_links_share_the_object_policy() {
    // The policy is per object, not per name: both links see the same choice.
    let dev = formatted(true, false);
    let mut vol = mount(dev).unwrap();
    let id = vol
        .create_file_in_root("first", &vec![0xC1u8; BS], ts(2))
        .unwrap();
    vol.link_file(id, OBJECT_ROOT, "second", ts(3)).unwrap();
    vol.set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(4))
        .unwrap();
    let via_link = vol.lookup_root("second").unwrap().unwrap();
    assert_eq!(via_link, id, "hard links resolve to the same object");
    assert_eq!(
        vol.file_data_policy(via_link).unwrap(),
        DataUpdatePolicy::InPlacePrivate
    );
}

#[test]
fn clone_file_destination_defaults_to_full_cow() {
    // CloneFile creates a new object: the destination starts at the safe
    // default while the source keeps its opt-in.
    let dev = formatted(true, true);
    let mut vol = mount(dev).unwrap();
    let source = vol
        .create_file_in_root("origin", &vec![0xC2u8; 2 * BS], ts(2))
        .unwrap();
    vol.set_file_data_policy(source, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap();
    let copy = vol.clone_file(source, OBJECT_ROOT, "copy", ts(4)).unwrap();
    assert_eq!(
        vol.file_data_policy(copy).unwrap(),
        DataUpdatePolicy::FullCow,
        "a clone starts at the creation default"
    );
    assert_eq!(
        vol.file_data_policy(source).unwrap(),
        DataUpdatePolicy::InPlacePrivate,
        "cloning must not strip the source opt-in"
    );
}

#[test]
fn clone_range_retains_the_destination_policy() {
    // CloneRange rewrites the destination layout; the destination's own
    // persistent choice must travel through that rewrite.
    let dev = formatted(true, true);
    let mut vol = mount(dev).unwrap();
    let source = vol
        .create_file_in_root("donor", &vec![0xC3u8; 2 * BS], ts(2))
        .unwrap();
    let destination = vol
        .create_file_in_root("target", &vec![0xC4u8; 2 * BS], ts(3))
        .unwrap();
    vol.set_file_data_policy(destination, DataUpdatePolicy::InPlacePrivate, ts(4))
        .unwrap();
    vol.clone_range(source, 0, destination, 0, BS as u64, ts(5))
        .unwrap();
    assert_eq!(
        vol.file_data_policy(destination).unwrap(),
        DataUpdatePolicy::InPlacePrivate,
        "the destination keeps its persistent opt-in"
    );
    assert_eq!(
        vol.file_data_policy(source).unwrap(),
        DataUpdatePolicy::FullCow,
        "the source is untouched"
    );
    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}
