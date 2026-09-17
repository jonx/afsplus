//! Extended attributes (ADR-108): durable, carried by every rewrite of the
//! object's record, copied by a clone, freed with the object, changed by one
//! commit, and either old or new after a power cut.
use afsplus_block::{for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_check::explain::{BlockRole, Explainer};
use afsplus_core::volume::{BatchOp, DataUpdatePolicy, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_options, mkfs_with_security_descriptors, mount, mount_with_snapshot_limits,
    AttributeWriteMode, CoreError, MkfsOptions, MkfsParams, MountOptions, NamePolicy, Volume,
};
use afsplus_format::{Timespec, OBJECT_ROOT};

use AttributeWriteMode::{Create, Replace, Upsert};

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 9,
    }
}

fn params() -> MkfsParams {
    MkfsParams {
        uuid: [0xa7; 16],
        label: "Attributes".into(),
        region_size: 512,
        reclaim_caps: Default::default(),
        log_slots: 4,
        shared_extents: true,
        data_policy: true,
        name_policy: NamePolicy::Sensitive,
        timestamp: time(1),
    }
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_security_descriptors(&mut dev, &params()).unwrap();
    dev
}

fn checked<D: BlockDevice>(volume: Volume<D>) -> D {
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    dev
}

/// Blocks of the device that carry the attribute segment magic.
fn attribute_blocks<D: BlockDevice>(dev: &mut D) -> Vec<u64> {
    let mut block = vec![0u8; dev.block_size()];
    (0..dev.total_blocks())
        .filter(|lba| {
            dev.read_block(*lba, &mut block).unwrap();
            &block[0..4] == b"AFSA"
        })
        .collect()
}

fn big(seed: u8) -> Vec<u8> {
    (0..10_000u32).map(|i| seed ^ (i % 251) as u8).collect()
}

#[test]
fn attributes_survive_every_rewrite_and_a_clone_copies_them() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &[1; 6000], time(2))
        .unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(2))
        .unwrap();
    let link = volume
        .create_symlink(OBJECT_ROOT, "link", "dir/target", time(2))
        .unwrap();
    assert_eq!(volume.attribute_names(file).unwrap(), Vec::<String>::new());
    assert_eq!(volume.attribute(file, "user.none").unwrap(), None);

    // One commit, several attributes, a set that spans three segments.
    let generation = volume.checkpoint().generation;
    volume
        .set_attributes(
            file,
            &[
                ("user.note", Some(b"hello")),
                ("aros.icon", Some(&big(1))),
                ("user.empty", Some(b"")),
            ],
            Upsert,
            time(3),
        )
        .unwrap();
    assert_eq!(volume.checkpoint().generation, generation + 1);
    volume
        .set_attributes(dir, &[("system.kind", Some(b"drawer"))], Create, time(3))
        .unwrap();
    volume
        .set_attributes(link, &[("user.why", Some(b"link"))], Create, time(3))
        .unwrap();
    volume
        .set_attributes(
            OBJECT_ROOT,
            &[("security.tag", Some(b"root"))],
            Create,
            time(3),
        )
        .unwrap();
    let stat = volume.stat(file).unwrap().unwrap();
    assert_eq!((stat.modified, stat.changed), (time(2), time(3)));

    // Every rewrite path of the file's record, with a descriptor and a
    // comment beside the set.
    volume
        .set_security_descriptor(file, 0x7fff_0001, 1, b"descriptor", time(4))
        .unwrap();
    volume
        .set_object_comment(file, "a comment", time(4))
        .unwrap();
    volume
        .write_file_at(file, 40_000, &[2; 5000], time(4))
        .unwrap();
    volume.truncate_file(file, 9000, time(4)).unwrap();
    volume
        .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, time(4))
        .unwrap();
    volume.link_file(file, dir, "alias", time(4)).unwrap();
    volume
        .rename(OBJECT_ROOT, "file", dir, "moved", time(4))
        .unwrap();
    volume
        .run_batch(
            &[BatchOp::DeleteFile {
                parent_id: dir,
                name: "alias",
            }],
            time(4),
        )
        .unwrap();
    volume
        .window_write_file_at(file, 0, &[3; 100], time(5))
        .unwrap();
    volume.window_commit(time(5)).unwrap();
    volume
        .create_file_in_directory(dir, "child", b"", time(5))
        .unwrap();
    volume
        .rename(OBJECT_ROOT, "link", OBJECT_ROOT, "link2", time(5))
        .unwrap();
    let clone = volume
        .clone_file(file, OBJECT_ROOT, "clone", time(6))
        .unwrap();

    let mut volume = mount(checked(volume)).unwrap();
    assert_eq!(
        volume.attribute_names(file).unwrap(),
        ["aros.icon", "user.empty", "user.note"]
    );
    assert_eq!(volume.attribute(file, "aros.icon").unwrap(), Some(big(1)));
    assert_eq!(volume.attribute(file, "user.empty").unwrap(), Some(vec![]));
    assert_eq!(
        volume.attribute(dir, "system.kind").unwrap().as_deref(),
        Some(&b"drawer"[..])
    );
    assert_eq!(
        volume.attribute(link, "user.why").unwrap().as_deref(),
        Some(&b"link"[..])
    );
    assert_eq!(
        volume
            .attribute(OBJECT_ROOT, "security.tag")
            .unwrap()
            .as_deref(),
        Some(&b"root"[..])
    );
    let mut target = [0u8; 32];
    let length = volume.read_link(link, &mut target).unwrap();
    assert_eq!(&target[..length], b"dir/target");
    assert_eq!(volume.object_comment(file).unwrap(), "a comment");
    assert_eq!(
        volume.security_descriptor(file).unwrap().unwrap().bytes,
        b"descriptor"
    );

    // The clone has its own copy; afterwards the two are independent.
    assert_eq!(volume.attribute(clone, "aros.icon").unwrap(), Some(big(1)));
    volume
        .set_attributes(
            clone,
            &[("aros.icon", None), ("user.note", Some(b"clone"))],
            Upsert,
            time(7),
        )
        .unwrap();
    assert_eq!(volume.attribute(file, "aros.icon").unwrap(), Some(big(1)));
    assert_eq!(
        volume.attribute(file, "user.note").unwrap().as_deref(),
        Some(&b"hello"[..])
    );
    assert_eq!(
        volume.attribute_names(clone).unwrap(),
        ["user.empty", "user.note"]
    );
    checked(volume);
}

#[test]
fn deleting_the_object_or_its_last_attribute_frees_the_chain() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    let other = volume
        .create_file_in_directory(OBJECT_ROOT, "other", b"y", time(2))
        .unwrap();
    volume
        .set_attributes(file, &[("user.a", Some(&big(2)))], Create, time(3))
        .unwrap();
    volume
        .set_attributes(other, &[("user.b", Some(&big(3)))], Create, time(3))
        .unwrap();
    // Removing the last attribute removes the reference and the chain.
    volume
        .set_attributes(file, &[("user.a", None)], Upsert, time(4))
        .unwrap();
    assert_eq!(volume.attribute_names(file).unwrap(), Vec::<String>::new());
    volume.delete_file(OBJECT_ROOT, "other", time(5)).unwrap();
    // A few commits let the reclaim queue release the retired blocks; the
    // checker then proves nothing is allocated without an owner.
    for n in 0..6 {
        volume
            .set_object_comment(file, &format!("turn {n}"), time(6 + n))
            .unwrap();
    }
    let mut volume = mount(checked(volume)).unwrap();
    assert_eq!(volume.attribute(file, "user.a").unwrap(), None);
    checked(volume);
}

#[test]
fn modes_bounds_no_ops_and_refusals() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    volume
        .set_attributes(file, &[("user.a", Some(b"1"))], Create, time(3))
        .unwrap();
    let generation = volume.checkpoint().generation;
    let changed = volume.stat(file).unwrap().unwrap().changed;

    type Change<'a> = (&'a str, Option<&'a [u8]>);
    let too_long = format!("user.{}", "n".repeat(251));
    let oversized = vec![0u8; 65_536];
    let half = vec![0u8; 40_000];
    let refused: Vec<(Vec<Change>, AttributeWriteMode, &str)> = vec![
        (vec![("user.a", Some(b"2"))], Create, "exists"),
        (vec![("user.b", Some(b"2"))], Replace, "absent"),
        (vec![("user.b", None)], Upsert, "absent"),
        (vec![("trusted.a", Some(b"2"))], Upsert, "invalid"),
        (vec![("user.", Some(b"2"))], Upsert, "invalid"),
        (vec![("user.nul\0", Some(b"2"))], Upsert, "invalid"),
        (vec![(&too_long, Some(b"2"))], Upsert, "invalid"),
        (vec![("user.b", Some(&oversized))], Upsert, "invalid"),
        (
            vec![("user.b", Some(&half)), ("user.c", Some(&half))],
            Upsert,
            "invalid",
        ),
        // A batch is whole or nothing: the first change is good.
        (
            vec![("user.good", Some(b"1")), ("user.a", Some(b"2"))],
            Create,
            "exists",
        ),
    ];
    for (changes, mode, what) in &refused {
        let result = volume.set_attributes(file, changes, *mode, time(4));
        match *what {
            "exists" => assert!(
                matches!(result, Err(CoreError::AlreadyExists)),
                "{changes:?}"
            ),
            "absent" => assert!(matches!(result, Err(CoreError::NotFound)), "{changes:?}"),
            _ => assert!(
                matches!(result, Err(CoreError::InvalidMetadata(_))),
                "{:?}",
                changes[0].0
            ),
        }
    }
    assert!(matches!(
        volume.set_attributes(999, &[("user.a", Some(b"1"))], Upsert, time(4)),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        volume.attribute(999, "user.a"),
        Err(CoreError::NotFound)
    ));
    // The same value, an empty batch, and set-then-remove are no-ops.
    volume
        .set_attributes(file, &[("user.a", Some(b"1"))], Upsert, time(4))
        .unwrap();
    volume.set_attributes(file, &[], Upsert, time(4)).unwrap();
    volume
        .set_attributes(
            file,
            &[("user.t", Some(b"1")), ("user.t", None)],
            Upsert,
            time(4),
        )
        .unwrap();
    assert_eq!(volume.checkpoint().generation, generation);
    assert_eq!(volume.stat(file).unwrap().unwrap().changed, changed);
    assert_eq!(volume.attribute_names(file).unwrap(), ["user.a"]);

    // Replace and the three modes on the happy side; later entries see
    // earlier ones.
    volume
        .set_attributes(
            file,
            &[("user.a", Some(b"2")), ("user.a", Some(b"3"))],
            Replace,
            time(5),
        )
        .unwrap();
    assert_eq!(
        volume.attribute(file, "user.a").unwrap().as_deref(),
        Some(&b"3"[..])
    );

    // An open window refuses the immediate commit.
    volume.window_write_file_at(file, 0, b"w", time(6)).unwrap();
    assert!(matches!(
        volume.set_attributes(file, &[("user.w", Some(b"1"))], Upsert, time(6)),
        Err(CoreError::WindowOpen)
    ));
    volume.window_commit(time(6)).unwrap();
    checked(volume);
}

#[test]
fn a_volume_with_persistent_snapshots_refuses_attributes() {
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            log_slots: 0,
            ..params()
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let mut volume = mount_with_snapshot_limits(dev, MountOptions::default(), limits).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    assert!(matches!(
        volume.set_attributes(file, &[("user.a", Some(b"1"))], Upsert, time(3)),
        Err(CoreError::FeatureDisabled(_))
    ));
    assert_eq!(volume.attribute_names(file).unwrap(), Vec::<String>::new());
}

#[test]
fn a_damaged_chain_is_reported_and_never_pins_the_object() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    volume
        .set_attributes(file, &[("user.a", Some(&big(4)))], Create, time(3))
        .unwrap();
    let mut dev = checked(volume);
    let blocks = attribute_blocks(&mut dev);
    assert_eq!(blocks.len(), 3);
    // The explain walk, which shares no code with the core, finds the same
    // three blocks, in chain order, and reads the set.
    let explainer = Explainer::load(&mut dev).unwrap();
    for (index, lba) in blocks.iter().enumerate() {
        let explanation = explainer.explain_block(&mut dev, *lba).unwrap();
        assert_eq!(
            explanation.roles,
            [BlockRole::AttributeSegment {
                object_id: file,
                index: index as u16
            }]
        );
        assert!(explanation.identity.unwrap().checksum_valid);
    }
    let summary = explainer
        .explain_object(file)
        .unwrap()
        .attributes
        .clone()
        .unwrap();
    assert_eq!((summary.segments_expected, summary.segments_found), (3, 3));
    assert_eq!(
        summary.attributes,
        Some(vec![("user.a".to_owned(), 10_000)])
    );
    // Flip one content byte of the middle segment: its checksum fails.
    let mut block = vec![0u8; 4096];
    dev.read_block(blocks[1], &mut block).unwrap();
    block[100] ^= 1;
    dev.write_block(blocks[1], &block).unwrap();

    let report = check_device(&mut dev);
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("attribute segment 1")),
        "{:?}",
        report.errors
    );
    let explainer = Explainer::load(&mut dev).unwrap();
    let summary = explainer
        .explain_object(file)
        .unwrap()
        .attributes
        .clone()
        .unwrap();
    assert_eq!((summary.segments_found, summary.attributes), (1, None));
    assert!(explainer
        .explain_block(&mut dev, blocks[2])
        .unwrap()
        .is_unowned());
    let mut volume = mount(dev).unwrap();
    assert!(matches!(
        volume.attribute(file, "user.a"),
        Err(CoreError::Corrupt(_))
    ));
    assert!(matches!(
        volume.set_attributes(file, &[("user.b", Some(b"1"))], Upsert, time(4)),
        Err(CoreError::Corrupt(_))
    ));
    // The object stays deletable, and the volume is consistent afterwards
    // but for the two segments past the damage, leaked on purpose.
    volume.delete_file(OBJECT_ROOT, "file", time(5)).unwrap();
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert_eq!(report.errors.len(), 2, "{:?}", report.errors);
    assert!(
        report
            .errors
            .iter()
            .all(|e| e.contains("owned by nothing (leak)")),
        "{:?}",
        report.errors
    );
}

#[test]
fn every_power_cut_leaves_the_old_set_or_the_new_one() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"stable", time(2))
        .unwrap();
    let bare = volume
        .create_file_in_directory(OBJECT_ROOT, "bare", b"stable", time(2))
        .unwrap();
    volume
        .set_attributes(
            file,
            &[("user.a", Some(&big(5))), ("user.b", Some(b"before"))],
            Create,
            time(3),
        )
        .unwrap();
    let base = volume.into_device();
    type Change<'a> = (&'a str, Option<&'a [u8]>);
    let replacement = big(6);
    let cases: [(u64, Vec<Change>, Vec<&str>); 3] = [
        (
            file,
            vec![
                ("user.a", Some(&replacement[..4500])),
                ("user.b", None),
                ("user.c", Some(b"c")),
            ],
            vec!["user.a", "user.c"],
        ),
        (file, vec![("user.a", None), ("user.b", None)], vec![]),
        (
            bare,
            vec![("user.first", Some(&replacement))],
            vec!["user.first"],
        ),
    ];
    let mut states = 0u32;
    for (object, changes, after) in &cases {
        let before = mount(base.clone())
            .unwrap()
            .attribute_names(*object)
            .unwrap();
        let mut recording = mount(RecordingBackend::new(base.clone())).unwrap();
        recording
            .set_attributes(*object, changes, Upsert, time(4))
            .unwrap();
        let (_, log) = recording.into_device().into_parts();
        let mut outcomes = [0u32, 0];
        for cut in 0..=log.len() {
            for_each_crash_state(&base, &log, cut, |state| {
                let mut volume = mount(state.image).unwrap();
                let names = volume.attribute_names(*object).unwrap();
                if names == before {
                    outcomes[0] += 1;
                    if *object == file {
                        assert_eq!(volume.attribute(file, "user.a").unwrap(), Some(big(5)));
                    }
                } else {
                    assert_eq!(&names, after);
                    for (name, value) in changes {
                        assert_eq!(volume.attribute(*object, name).unwrap().as_deref(), *value);
                    }
                    outcomes[1] += 1;
                }
                assert_eq!(volume.read_file(*object).unwrap(), b"stable");
                checked(volume);
                states += 1;
            });
        }
        assert!(outcomes.iter().all(|count| *count > 0), "{outcomes:?}");
        eprintln!("attributes {after:?} crash states old/new: {outcomes:?}");
    }
    eprintln!("attribute crash states: {states}");
}
