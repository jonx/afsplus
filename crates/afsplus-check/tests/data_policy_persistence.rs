//! The persistent per-file data-update policy (ADR-065): the opt-in lives in
//! the object record, survives remounts, drives the ADR-062 in-place path
//! exactly where the eligibility rules pass, and fails closed — a planted
//! flag on a volume without the feature is corruption for both the read path
//! and the checker, mirroring the shared-extent marker discipline.

use afsplus_block::{
    for_each_crash_state, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend,
};
use afsplus_check::check_device;
use afsplus_core::volume::DataUpdatePolicy;
use afsplus_core::{
    mkfs, mount_with_options, object_map, CoreError, MkfsParams, MountOptions, Volume,
};
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

fn profile_mount<D: BlockDevice>(device: D, pages: usize) -> Volume<D> {
    let volume = mount_with_options(
        device,
        MountOptions {
            tree_cache_pages: std::num::NonZeroUsize::new(pages),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    volume
}

/// Rejects checker errors and retained-checkpoint warnings. A stopped
/// intent-log tail is the only admissible crash artifact.
fn assert_checker_clean<D: BlockDevice>(device: &mut D, context: &str) {
    let report = check_device(device);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
    assert!(
        report
            .warnings
            .iter()
            .all(|warning| warning.starts_with("intent log tail:")),
        "{context}: {:?}",
        report.warnings
    );
}

#[test]
fn opt_in_persists_across_remount_and_takes_the_in_place_path() {
    for pages in [2, 4, 8, usize::MAX] {
        opt_in_persists_across_remount_and_takes_the_in_place_path_profile(pages);
    }
}

fn opt_in_persists_across_remount_and_takes_the_in_place_path_profile(pages: usize) {
    let dev = formatted(true, false);
    let mut vol = profile_mount(dev, pages);
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
    let mut vol = profile_mount(vol.into_device(), pages);
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
    assert_checker_clean(&mut dev, &format!("pages={pages}"));
}

#[test]
fn clearing_the_flag_restores_full_cow() {
    for pages in [2, 4, 8, usize::MAX] {
        clearing_the_flag_restores_full_cow_profile(pages);
    }
}

fn clearing_the_flag_restores_full_cow_profile(pages: usize) {
    let dev = formatted(true, false);
    let mut vol = profile_mount(dev, pages);
    let id = vol
        .create_file_in_root("tmp", &vec![0x44u8; 2 * BS], ts(2))
        .unwrap();
    vol.set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap();
    vol.set_file_data_policy(id, DataUpdatePolicy::FullCow, ts(4))
        .unwrap();
    assert_eq!(vol.file_data_policy(id).unwrap(), DataUpdatePolicy::FullCow);

    let mut vol = profile_mount(vol.into_device(), pages);
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
    assert_checker_clean(&mut dev, &format!("pages={pages}"));
}

#[test]
fn feature_off_volume_refuses_the_opt_in() {
    for pages in [2, 4, 8, usize::MAX] {
        feature_off_volume_refuses_the_opt_in_profile(pages);
    }
}

fn feature_off_volume_refuses_the_opt_in_profile(pages: usize) {
    let dev = formatted(false, false);
    let mut vol = profile_mount(dev, pages);
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
    for pages in [2, 4, 8, usize::MAX] {
        directories_refuse_the_policy_profile(pages);
    }
}

fn directories_refuse_the_policy_profile(pages: usize) {
    let dev = formatted(true, false);
    let mut vol = profile_mount(dev, pages);
    let error = vol
        .set_file_data_policy(OBJECT_ROOT, DataUpdatePolicy::InPlacePrivate, ts(2))
        .unwrap_err();
    assert!(matches!(error, CoreError::IsDirectory), "{error:?}");
}

#[test]
fn planted_flag_without_the_feature_fails_closed() {
    for pages in [2, 4, 8, usize::MAX] {
        planted_flag_without_the_feature_fails_closed_profile(pages);
    }
}

fn planted_flag_without_the_feature_fails_closed_profile(pages: usize) {
    // Build the flagged record on a feature-off volume byte-for-byte,
    // bypassing the refused API.
    let dev = formatted(false, false);
    let mut vol = profile_mount(dev, pages);
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
    let mut vol = profile_mount(dev, pages);
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
    for pages in [2, 4, 8, usize::MAX] {
        shared_blocks_of_a_flagged_file_still_go_through_cow_profile(pages);
    }
}

fn shared_blocks_of_a_flagged_file_still_go_through_cow_profile(pages: usize) {
    let dev = formatted(true, true);
    let mut vol = profile_mount(dev, pages);
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
    assert_checker_clean(&mut dev, &format!("pages={pages}"));
}

#[test]
fn in_place_write_after_a_crash_keeps_metadata_clean() {
    for pages in [2, 4, 8, usize::MAX] {
        in_place_write_after_a_crash_keeps_metadata_clean_profile(pages);
    }
}

fn in_place_write_after_a_crash_keeps_metadata_clean_profile(pages: usize) {
    let mut volume = profile_mount(formatted(true, false), pages);
    let before = vec![0xaa; 2 * BS];
    let id = volume
        .create_file_in_root("crashy", &before, ts(2))
        .unwrap();
    volume
        .set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
        .unwrap();
    let metadata = volume.stat(id).unwrap().unwrap();
    let generation = volume.generation();
    let base = volume.into_device();
    let offset = 100;
    let length = 3000;
    let mut after = before.clone();
    after[offset..offset + length].fill(0xbb);
    let mut recorded = profile_mount(RecordingBackend::new(base.clone()), pages);
    assert_eq!(
        recorded.file_data_policy(id).unwrap(),
        DataUpdatePolicy::InPlacePrivate
    );
    recorded
        .write_file_at(id, offset as u64, &vec![0xbb; length], ts(4))
        .unwrap();
    let stats = recorded.last_commit_stats().unwrap();
    assert_eq!(stats.data_blocks_overwritten_in_place, 1);
    assert!(stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
    let (_, operations) = recorded.into_device().into_parts();
    assert!(
        matches!(operations.first(), Some(RecordedOp::Write { lba, .. }) if *lba == metadata.data_root)
    );
    let mut outcomes = [0u64; 2];
    let mut torn_old = 0;
    for cut in 0..=operations.len() {
        for_each_crash_state(&base, &operations, cut, |state| {
            let context = format!("persistent pages={pages}: {}", state.description);
            let mut image = state.image;
            assert_checker_clean(&mut image, &context);
            let mut recovered = profile_mount(image, pages);
            let post = match recovered.generation() {
                value if value == generation => false,
                value if value == generation + 1 => true,
                value => panic!("{context}: unexpected generation {value}"),
            };
            assert_eq!(
                recovered.lookup_root("crashy").unwrap(),
                Some(id),
                "{context}"
            );
            assert_eq!(
                recovered.file_data_policy(id).unwrap(),
                DataUpdatePolicy::InPlacePrivate,
                "{context}"
            );
            let actual_metadata = recovered.stat(id).unwrap().unwrap();
            assert_eq!(
                (
                    actual_metadata.data_root,
                    actual_metadata.data_blocks,
                    actual_metadata.size_bytes
                ),
                (
                    metadata.data_root,
                    metadata.data_blocks,
                    metadata.size_bytes
                ),
                "{context}"
            );
            let bytes = recovered.read_file(id).unwrap();
            assert_eq!(bytes.len(), before.len(), "{context}");
            outcomes[usize::from(post)] += 1;
            if post {
                assert_eq!(bytes, after, "{context}");
            } else {
                assert_eq!(&bytes[..offset], &before[..offset], "{context}");
                assert_eq!(
                    &bytes[offset + length..],
                    &before[offset + length..],
                    "{context}"
                );
                assert!(
                    bytes[offset..offset + length]
                        .iter()
                        .all(|byte| *byte == 0xaa || *byte == 0xbb),
                    "{context}"
                );
                if bytes != before && bytes != after {
                    torn_old += 1;
                }
            }
        });
    }
    assert!(outcomes.iter().all(|count| *count > 0));
    assert!(
        torn_old > 0,
        "must exercise the weaker torn-old-data contract"
    );
    eprintln!("persistent private cuts pages={pages} old={} new={} torn_old={torn_old} spills={} peak_staged={}", outcomes[0], outcomes[1], stats.tree_mutations.staged_spill_writes, stats.tree_mutations.max_resident_staged_nodes);
}

#[test]
fn hard_links_share_the_object_policy() {
    for pages in [2, 4, 8, usize::MAX] {
        hard_links_share_the_object_policy_profile(pages);
    }
}

fn hard_links_share_the_object_policy_profile(pages: usize) {
    // The policy is per object, not per name: both links see the same choice.
    let dev = formatted(true, false);
    let mut vol = profile_mount(dev, pages);
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
    for pages in [2, 4, 8, usize::MAX] {
        clone_file_destination_defaults_to_full_cow_profile(pages);
    }
}

fn clone_file_destination_defaults_to_full_cow_profile(pages: usize) {
    // CloneFile creates a new object: the destination starts at the safe
    // default while the source keeps its opt-in.
    let dev = formatted(true, true);
    let mut vol = profile_mount(dev, pages);
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
    for pages in [2, 4, 8, usize::MAX] {
        clone_range_retains_the_destination_policy_profile(pages);
    }
}

fn clone_range_retains_the_destination_policy_profile(pages: usize) {
    // CloneRange rewrites the destination layout; the destination's own
    // persistent choice must travel through that rewrite.
    let dev = formatted(true, true);
    let mut vol = profile_mount(dev, pages);
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
    assert_checker_clean(&mut dev, &format!("pages={pages}"));
}

#[test]
fn policy_flag_publication_cuts_preserve_exact_choice_and_bytes_in_all_profiles() {
    for pages in [2, 4, 8, usize::MAX] {
        for enabling in [true, false] {
            let mut setup = profile_mount(formatted(true, false), pages);
            let content = vec![0x52; BS + 91];
            let id = setup
                .create_file_in_root("policy", &content, ts(2))
                .unwrap();
            if !enabling {
                setup
                    .set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
                    .unwrap();
            }
            let before = setup.stat(id).unwrap().unwrap();
            let generation = setup.generation();
            let desired = if enabling {
                DataUpdatePolicy::InPlacePrivate
            } else {
                DataUpdatePolicy::FullCow
            };
            let base = setup.into_device();
            let mut recorded = profile_mount(RecordingBackend::new(base.clone()), pages);
            recorded.set_file_data_policy(id, desired, ts(4)).unwrap();
            let stats = recorded.last_commit_stats().unwrap();
            assert!(stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
            let (_, operations) = recorded.into_device().into_parts();
            let mut outcomes = [0u64; 2];
            for cut in 0..=operations.len() {
                for_each_crash_state(&base, &operations, cut, |state| {
                    let context = format!(
                        "flag pages={pages} enabling={enabling}: {}",
                        state.description
                    );
                    let mut image = state.image;
                    assert_checker_clean(&mut image, &context);
                    let mut recovered = profile_mount(image, pages);
                    let post = match recovered.generation() {
                        value if value == generation => false,
                        value if value == generation + 1 => true,
                        value => panic!("{context}: unexpected generation {value}"),
                    };
                    let opted_in = if post { enabling } else { !enabling };
                    assert_eq!(
                        recovered.file_data_policy(id).unwrap(),
                        if opted_in {
                            DataUpdatePolicy::InPlacePrivate
                        } else {
                            DataUpdatePolicy::FullCow
                        },
                        "{context}"
                    );
                    assert_eq!(
                        recovered.lookup_root("policy").unwrap(),
                        Some(id),
                        "{context}"
                    );
                    assert_eq!(recovered.list_root().unwrap().len(), 1, "{context}");
                    assert_eq!(recovered.read_file(id).unwrap(), content, "{context}");
                    let actual = recovered.stat(id).unwrap().unwrap();
                    assert_eq!(
                        actual.flags,
                        (before.flags & !OBJECT_FLAG_DATA_IN_PLACE)
                            | if opted_in {
                                OBJECT_FLAG_DATA_IN_PLACE
                            } else {
                                0
                            },
                        "{context}"
                    );
                    assert_eq!(
                        (actual.data_root, actual.data_blocks, actual.size_bytes),
                        (before.data_root, before.data_blocks, before.size_bytes),
                        "{context}"
                    );
                    // Only the in-place flag and change time may differ from
                    // the committed record; every other field stays exact.
                    let mut expected_record = before;
                    expected_record.flags = actual.flags;
                    if post {
                        expected_record.changed = ts(4);
                    }
                    assert_eq!(actual, expected_record, "{context}");
                    outcomes[usize::from(post)] += 1;
                });
            }
            assert!(outcomes.iter().all(|count| *count > 0));
            eprintln!("flag cuts pages={pages} enabling={enabling} old={} new={} spills={} peak_staged={}", outcomes[0], outcomes[1], stats.tree_mutations.staged_spill_writes, stats.tree_mutations.max_resident_staged_nodes);
        }
    }
}

struct FlagFixture {
    id: u64,
    before: ObjectRecord,
    generation: u64,
    enabling: bool,
    content: Vec<u8>,
}

impl FlagFixture {
    fn desired(&self) -> DataUpdatePolicy {
        if self.enabling {
            DataUpdatePolicy::InPlacePrivate
        } else {
            DataUpdatePolicy::FullCow
        }
    }
    fn apply<D: BlockDevice>(&self, volume: &mut Volume<D>) -> Result<(), CoreError> {
        volume.set_file_data_policy(self.id, self.desired(), ts(4))
    }
    fn verify<D: BlockDevice>(&self, volume: &mut Volume<D>, post: bool) {
        let opted_in = if post { self.enabling } else { !self.enabling };
        assert_eq!(volume.generation(), self.generation + u64::from(post));
        assert_eq!(volume.lookup_root("flag").unwrap(), Some(self.id));
        assert_eq!(volume.list_root().unwrap().len(), 1);
        assert_eq!(volume.read_file(self.id).unwrap(), self.content);
        let record = volume.stat(self.id).unwrap().unwrap();
        assert_eq!(
            record.flags,
            (self.before.flags & !OBJECT_FLAG_DATA_IN_PLACE)
                | if opted_in {
                    OBJECT_FLAG_DATA_IN_PLACE
                } else {
                    0
                }
        );
        assert_eq!(
            (record.data_root, record.data_blocks, record.size_bytes),
            (
                self.before.data_root,
                self.before.data_blocks,
                self.before.size_bytes
            )
        );
        let mut expected_record = self.before;
        if post {
            expected_record.changed = ts(4);
            expected_record.flags = if self.enabling {
                self.before.flags | OBJECT_FLAG_DATA_IN_PLACE
            } else {
                self.before.flags & !OBJECT_FLAG_DATA_IN_PLACE
            };
        }
        assert_eq!(record, expected_record);
        assert_eq!(
            volume.file_data_policy(self.id).unwrap(),
            if opted_in {
                DataUpdatePolicy::InPlacePrivate
            } else {
                DataUpdatePolicy::FullCow
            }
        );
    }
}

fn flag_fixture(pages: usize, enabling: bool) -> (FlagFixture, MemoryBackend) {
    let mut setup = profile_mount(formatted(true, false), pages);
    let content = vec![0x62; BS + 51];
    let id = setup.create_file_in_root("flag", &content, ts(2)).unwrap();
    if !enabling {
        setup
            .set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, ts(3))
            .unwrap();
    }
    let fixture = FlagFixture {
        id,
        before: setup.stat(id).unwrap().unwrap(),
        generation: setup.generation(),
        enabling,
        content,
    };
    fixture.verify(&mut setup, false);
    (fixture, setup.into_device())
}

#[test]
fn policy_flag_io_failures_preserve_exact_choice_and_retry_in_all_profiles() {
    use afsplus_block::{FaultBackend, FaultPlan};
    for pages in [2, 4, 8, usize::MAX] {
        for enabling in [true, false] {
            let (fixture, base) = flag_fixture(pages, enabling);
            let mut reference = profile_mount(RecordingBackend::new(base.clone()), pages);
            let checkpoints = reference.ident().checkpoint_slots;
            fixture.apply(&mut reference).unwrap();
            fixture.verify(&mut reference, true);
            let (_, log) = reference.into_device().into_parts();
            let targets: Vec<_> = log
                .iter()
                .filter_map(|op| match op {
                    RecordedOp::Write { lba, .. } => Some(*lba),
                    _ => None,
                })
                .collect();
            assert!(checkpoints.contains(targets.last().unwrap()));
            assert_eq!(
                targets
                    .iter()
                    .filter(|lba| checkpoints.contains(lba))
                    .count(),
                1
            );
            let writes = targets.len() as u64;
            let flushes = log
                .iter()
                .filter(|op| matches!(op, RecordedOp::Flush))
                .count() as u64;
            let plans = (0..writes)
                .map(|i| FaultPlan {
                    fail_write_index: Some(i),
                    ..Default::default()
                })
                .chain((0..flushes).map(|i| FaultPlan {
                    fail_flush_index: Some(i),
                    ..Default::default()
                }));
            let mut retries = 0;
            for plan in plans {
                let mut volume = profile_mount(FaultBackend::new(base.clone(), plan), pages);
                assert!(fixture.apply(&mut volume).is_err());
                assert!(volume.device_mut().tripped());
                let published = plan.fail_flush_index == Some(flushes - 1);
                let uncertain = published || plan.fail_write_index == Some(writes - 1);
                if !published {
                    fixture.verify(&mut volume, false);
                }
                if uncertain {
                    assert!(matches!(
                        fixture.apply(&mut volume),
                        Err(CoreError::WindowPoisoned)
                    ));
                }
                let mut recovered = profile_mount(volume.into_device().into_inner(), pages);
                fixture.verify(&mut recovered, published);
                assert_checker_clean(recovered.device_mut(), &format!("pages={pages} recovered"));
                if !published {
                    fixture.apply(&mut recovered).unwrap();
                }
                fixture.verify(&mut recovered, true);
                let mut again = profile_mount(recovered.into_device(), pages);
                fixture.verify(&mut again, true);
                assert_checker_clean(again.device_mut(), &format!("pages={pages} remounted"));
                if !uncertain {
                    let mut retry = profile_mount(FaultBackend::new(base.clone(), plan), pages);
                    assert!(fixture.apply(&mut retry).is_err());
                    assert!(retry.device_mut().tripped());
                    fixture.verify(&mut retry, false);
                    fixture.apply(&mut retry).unwrap();
                    fixture.verify(&mut retry, true);
                    let mut remounted = profile_mount(retry.into_device().into_inner(), pages);
                    fixture.verify(&mut remounted, true);
                    assert_checker_clean(
                        remounted.device_mut(),
                        &format!("pages={pages} remounted"),
                    );
                    retries += 1;
                }
            }
            assert_eq!(retries, writes + flushes - 2);
            eprintln!("flag faults pages={pages} enabling={enabling} primary={} separate_same_handle={retries}", writes + flushes);
        }
    }
}

// Same completed-write/adoption-read model as faults.rs: errors may be
// reported after the checkpoint is already completely readable on media.
struct PolicyAmbiguousDevice {
    inner: MemoryBackend,
    checkpoints: [u64; 2],
    published: bool,
    fail_write: bool,
    tripped: bool,
    writes: u64,
    flushes: u64,
}
impl BlockDevice for PolicyAmbiguousDevice {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, bytes: &mut [u8]) -> Result<(), afsplus_block::BlockError> {
        if self.published && !self.fail_write {
            self.tripped = true;
            return Err(afsplus_block::BlockError::Injected("post-publication read"));
        }
        self.inner.read_block(lba, bytes)
    }
    fn write_block(&mut self, lba: u64, bytes: &[u8]) -> Result<(), afsplus_block::BlockError> {
        self.writes += 1;
        self.inner.write_block(lba, bytes)?;
        if self.checkpoints.contains(&lba) {
            self.published = true;
            if self.fail_write {
                self.tripped = true;
                return Err(afsplus_block::BlockError::Injected(
                    "completed checkpoint write",
                ));
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), afsplus_block::BlockError> {
        self.flushes += 1;
        self.inner.flush()
    }
}

#[test]
fn policy_flag_ambiguous_publication_blocks_mutations_until_remount_in_all_profiles() {
    for pages in [2, 4, 8, usize::MAX] {
        for enabling in [true, false] {
            for fail_write in [true, false] {
                let (fixture, base) = flag_fixture(pages, enabling);
                let checkpoints = afsplus_format::ident::Identification::decode(&base.peek(0))
                    .unwrap()
                    .checkpoint_slots;
                let mut volume = profile_mount(
                    PolicyAmbiguousDevice {
                        inner: base,
                        checkpoints,
                        published: false,
                        fail_write,
                        tripped: false,
                        writes: 0,
                        flushes: 0,
                    },
                    pages,
                );
                assert!(fixture.apply(&mut volume).is_err());
                assert!(volume.device_mut().published && volume.device_mut().tripped);
                let io = (volume.device_mut().writes, volume.device_mut().flushes);
                assert!(matches!(
                    fixture.apply(&mut volume),
                    Err(CoreError::WindowPoisoned)
                ));
                assert!(matches!(
                    volume.create_file_in_root("blocked", b"never published", ts(5)),
                    Err(CoreError::WindowPoisoned)
                ));
                assert_eq!(
                    (volume.device_mut().writes, volume.device_mut().flushes),
                    io
                );
                let mut recovered = profile_mount(volume.into_device().inner, pages);
                fixture.verify(&mut recovered, true);
                assert_checker_clean(recovered.device_mut(), &format!("pages={pages} recovered"));
                // An idempotent retry must retain the resolved generation.
                fixture.apply(&mut recovered).unwrap();
                fixture.verify(&mut recovered, true);
                let mut again = profile_mount(recovered.into_device(), pages);
                fixture.verify(&mut again, true);
                assert_checker_clean(again.device_mut(), &format!("pages={pages} remounted"));
                eprintln!(
                    "flag ambiguous pages={pages} enabling={enabling} completed_write={fail_write}"
                );
            }
        }
    }
}

fn policy_extended_image(feature: bool, snapshots: bool) -> MemoryBackend {
    let mut device = MemoryBackend::new(BS, 512);
    afsplus_core::mkfs_with_options(
        &mut device,
        &MkfsParams {
            uuid: [0x6a; 16],
            label: "PolicyRetained".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: feature,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(1),
        },
        afsplus_core::MkfsOptions {
            persistent_snapshots: snapshots,
        },
    )
    .unwrap();
    device
}

fn retained_mount<D: BlockDevice>(device: D, pages: usize) -> Volume<D> {
    let volume = afsplus_core::mount_with_snapshot_limits(
        device,
        MountOptions {
            tree_cache_pages: std::num::NonZeroUsize::new(pages),
            ..Default::default()
        },
        afsplus_core::volume::SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 128,
            reclaim_records: 8,
        },
    )
    .unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    volume
}

fn verify_retained_policy<D: BlockDevice>(
    volume: &mut Volume<D>,
    snapshot: u64,
    file: u64,
    expected: &[u8],
    expected_metadata: afsplus_core::volume::ObjectMetadata,
) {
    let view = volume.snapshot_open(snapshot).unwrap();
    let metadata = volume.snapshot_stat(&view, file).unwrap().unwrap();
    assert_eq!(metadata, expected_metadata);
    let mut bytes = vec![0xa5; expected.len() + 1];
    assert_eq!(
        volume
            .snapshot_read_file_at(&view, file, 0, &mut bytes)
            .unwrap(),
        expected.len()
    );
    assert_eq!(&bytes[..expected.len()], expected);
    assert_eq!(bytes[expected.len()], 0xa5);
}

#[test]
fn retained_snapshot_forces_cow_for_flagged_private_writes_in_all_profiles() {
    for pages in [2, 4, 8, usize::MAX] {
        let mut volume = retained_mount(policy_extended_image(true, true), pages);
        let original = vec![0x42; 2 * BS];
        let file = volume
            .create_file_in_root("retained", &original, ts(2))
            .unwrap();
        volume
            .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, ts(3))
            .unwrap();
        volume.set_data_update_policy(DataUpdatePolicy::InPlacePrivate);
        let snapshot = volume.snapshot_create(ts(4)).unwrap();
        let before = volume.stat(file).unwrap().unwrap();
        volume.write_file_at(file, 97, &[0xb3; 307], ts(5)).unwrap();
        assert_eq!(
            volume.file_data_policy(file).unwrap(),
            DataUpdatePolicy::InPlacePrivate
        );
        let stats = volume.last_commit_stats().unwrap();
        assert_eq!(stats.data_blocks_overwritten_in_place, 0);
        assert!(stats.tree_mutations.max_resident_staged_nodes <= pages as u64);
        assert_ne!(
            volume.stat(file).unwrap().unwrap().data_root,
            before.data_root
        );
        let mut expected = original.clone();
        expected[97..404].fill(0xb3);
        assert_eq!(volume.read_file(file).unwrap(), expected);
        verify_retained_policy(&mut volume, snapshot, file, &original, before.into());
        volume
            .set_file_data_policy(file, DataUpdatePolicy::FullCow, ts(6))
            .unwrap();
        assert_eq!(
            volume.file_data_policy(file).unwrap(),
            DataUpdatePolicy::FullCow
        );
        verify_retained_policy(&mut volume, snapshot, file, &original, before.into());
        let mut remounted = retained_mount(volume.into_device(), pages);
        assert_eq!(
            remounted.file_data_policy(file).unwrap(),
            DataUpdatePolicy::FullCow
        );
        assert_eq!(remounted.read_file(file).unwrap(), expected);
        verify_retained_policy(&mut remounted, snapshot, file, &original, before.into());
        let report = check_device(remounted.device_mut());
        assert!(report.is_clean(), "{:?}", report.errors);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        eprintln!(
            "retained policy pages={pages} fallback_in_place=0 spills={} peak_staged={}",
            stats.tree_mutations.staged_spill_writes,
            stats.tree_mutations.max_resident_staged_nodes
        );
    }
}

#[test]
fn policy_flag_applicable_refusals_issue_no_writes_and_preserve_state_in_all_profiles() {
    use afsplus_block::TraceBackend;
    use afsplus_core::MountMode;
    for pages in [2, 4, 8, usize::MAX] {
        for case in [
            "feature",
            "directory",
            "symlink",
            "missing",
            "time",
            "window",
            "readonly",
            "nochanges",
        ] {
            let mut setup = profile_mount(policy_extended_image(case != "feature", false), pages);
            let file = setup.create_file_in_root("file", b"before", ts(2)).unwrap();
            let symlink = setup
                .create_symlink(OBJECT_ROOT, "link", "file", ts(3))
                .unwrap();
            let mode = match case {
                "readonly" => MountMode::ReadOnly,
                "nochanges" => MountMode::NoChanges,
                _ => MountMode::ReadWrite,
            };
            let mut volume = mount_with_options(
                TraceBackend::new(setup.into_device()),
                MountOptions {
                    mode,
                    tree_cache_pages: std::num::NonZeroUsize::new(pages),
                },
            )
            .unwrap();
            assert_eq!(volume.tree_cache_pages(), pages);
            if case == "window" {
                volume
                    .window_write_file_at(file, 0, b"after!", ts(4))
                    .unwrap();
            }
            let generation = volume.generation();
            let unlogged = volume.window_unlogged_ops();
            let record = volume.stat(file).unwrap().unwrap();
            volume.device_mut().reset();
            let target = match case {
                "directory" => OBJECT_ROOT,
                "symlink" => symlink,
                "missing" => 9999,
                _ => file,
            };
            let now = if case == "time" {
                Timespec {
                    seconds: 4,
                    nanoseconds: 1_000_000_000,
                }
            } else {
                ts(4)
            };
            let error = volume
                .set_file_data_policy(target, DataUpdatePolicy::InPlacePrivate, now)
                .unwrap_err();
            assert!(
                match case {
                    "feature" => matches!(error, CoreError::FeatureDisabled(_)),
                    "directory" | "symlink" => matches!(error, CoreError::IsDirectory),
                    "missing" => matches!(error, CoreError::NotFound),
                    "time" => matches!(error, CoreError::InvalidMetadata(_)),
                    "window" => matches!(error, CoreError::WindowOpen),
                    "readonly" | "nochanges" => matches!(error, CoreError::ReadOnly),
                    _ => unreachable!(),
                },
                "pages={pages} case={case}: {error}"
            );
            let io = volume.device_mut().stats();
            assert_eq!((io.writes, io.flushes), (0, 0), "pages={pages} case={case}");
            assert_eq!(volume.generation(), generation);
            assert_eq!(volume.window_unlogged_ops(), unlogged);
            assert_eq!(volume.stat(file).unwrap().unwrap(), record);
            if case == "window" {
                volume.window_commit(ts(5)).unwrap();
            }
            let mut remounted = profile_mount(volume.into_device().into_inner(), pages);
            assert_eq!(
                remounted.file_data_policy(file).unwrap(),
                DataUpdatePolicy::FullCow
            );
            assert_eq!(
                remounted.read_file(file).unwrap(),
                if case == "window" {
                    b"after!"
                } else {
                    b"before"
                }
            );
            if case != "feature" {
                remounted
                    .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, ts(6))
                    .unwrap();
                assert_eq!(
                    remounted.file_data_policy(file).unwrap(),
                    DataUpdatePolicy::InPlacePrivate
                );
            }
            assert_checker_clean(remounted.device_mut(), &format!("pages={pages} remounted"));
            eprintln!("policy refusal pages={pages} case={case}");
        }
    }
}
