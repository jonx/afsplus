//! End-to-end proof of the security preservation container: opaque
//! descriptors survive every rewrite of their object, a classic protection
//! edit cannot destroy or silently contradict them, and their blocks have one
//! owner from attachment to the death of the object.
use afsplus_block::{for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::volume::{BatchOp, DataUpdatePolicy, PreservedMetadata};
use afsplus_core::{
    mkfs, mkfs_with_security_descriptors, mount, CoreError, MkfsParams, NamePolicy,
    SecurityDescriptor, SecurityProjectionPolicy, Volume,
};
use afsplus_format::header::{block_type, BlockHeader};
use afsplus_format::ident::{Identification, INCOMPAT_PERSISTENT_SNAPSHOTS};
use afsplus_format::{Timespec, OBJECT_ROOT};

/// A format identity no implementation in this repository evaluates.
const UNKNOWN_FORMAT: u32 = 0x7fff_0042;

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 7,
    }
}

fn params() -> MkfsParams {
    MkfsParams {
        uuid: [0xb5; 16],
        label: "Security".into(),
        region_size: 512,
        reclaim_caps: Default::default(),
        log_slots: 0,
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

/// Deterministic opaque bytes, including NUL and 0xff, that no codec in the
/// repository produces.
fn blob(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn expect(format: u32, version: u16, diverged: bool, bytes: &[u8]) -> Option<SecurityDescriptor> {
    Some(SecurityDescriptor {
        format,
        version,
        projection_diverged: diverged,
        bytes: bytes.to_vec(),
    })
}

fn checked<D: BlockDevice>(volume: Volume<D>) -> D {
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    dev
}

#[test]
fn descriptors_of_every_size_class_round_trip_across_remount() {
    let mut volume = mount(formatted()).unwrap();
    let mut objects = Vec::new();
    // One byte, one full segment, one byte over, and the 64 KiB bound.
    for (index, len) in [1usize, 4040, 4041, 65_536].into_iter().enumerate() {
        let name = format!("f{index}");
        let id = volume
            .create_file_in_directory(OBJECT_ROOT, &name, b"data", time(2))
            .unwrap();
        assert_eq!(volume.security_descriptor(id).unwrap(), None);
        let bytes = blob(len, index as u8);
        volume
            .set_security_descriptor(id, UNKNOWN_FORMAT, 9, &bytes, time(3))
            .unwrap();
        assert_eq!(
            volume.security_descriptor(id).unwrap(),
            expect(UNKNOWN_FORMAT, 9, false, &bytes)
        );
        assert_eq!(volume.stat(id).unwrap().unwrap().changed, time(3));
        objects.push((id, bytes));
    }
    let mut volume = mount(checked(volume)).unwrap();
    for (id, bytes) in &objects {
        assert_eq!(
            volume.security_descriptor(*id).unwrap(),
            expect(UNKNOWN_FORMAT, 9, false, bytes)
        );
        assert_eq!(volume.read_file(*id).unwrap(), b"data");
    }
    // Out-of-range input is refused before any allocation.
    let (id, _) = objects[0];
    for (format, bytes) in [
        (0u32, blob(4, 0)),
        (UNKNOWN_FORMAT, Vec::new()),
        (UNKNOWN_FORMAT, blob(65_537, 0)),
    ] {
        assert!(matches!(
            volume.set_security_descriptor(id, format, 0, &bytes, time(4)),
            Err(CoreError::InvalidMetadata(_))
        ));
    }
    checked(volume);
}

#[test]
fn an_unknown_descriptor_survives_every_rewrite_of_its_object() {
    let mut volume = mount(formatted()).unwrap();
    let file_bytes = blob(5000, 1);
    let dir_bytes = blob(300, 2);
    let link_bytes = blob(17, 3);
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &[0x11; 6000], time(2))
        .unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(2))
        .unwrap();
    let link = volume
        .create_symlink(OBJECT_ROOT, "link", "dir/target", time(2))
        .unwrap();
    volume
        .set_security_descriptor(file, UNKNOWN_FORMAT, 1, &file_bytes, time(3))
        .unwrap();
    volume
        .set_security_descriptor(dir, UNKNOWN_FORMAT, 2, &dir_bytes, time(3))
        .unwrap();
    volume
        .set_security_descriptor(link, UNKNOWN_FORMAT, 3, &link_bytes, time(3))
        .unwrap();
    // The root directory is an object like any other.
    volume
        .set_security_descriptor(OBJECT_ROOT, UNKNOWN_FORMAT, 4, b"root", time(3))
        .unwrap();

    // File: data write, extension into an extent tree, truncation, policy
    // flag, hard link, rename, timestamp restoration, clone source.
    volume
        .write_file_at(file, 100, &[0x22; 50], time(4))
        .unwrap();
    volume
        .write_file_at(file, 40_000, &[0x33; 5000], time(4))
        .unwrap();
    volume.truncate_file(file, 9000, time(4)).unwrap();
    volume
        .set_file_data_policy(file, DataUpdatePolicy::InPlacePrivate, time(4))
        .unwrap();
    volume.link_file(file, dir, "alias", time(4)).unwrap();
    volume
        .rename(OBJECT_ROOT, "file", dir, "moved", time(4))
        .unwrap();
    let stat = volume.stat(file).unwrap().unwrap();
    volume
        .restore_object_metadata(
            file,
            PreservedMetadata {
                protection: stat.protection,
                created: time(-5),
                modified: time(-4),
                changed: time(-3),
            },
        )
        .unwrap();
    let clone = volume
        .clone_file(file, OBJECT_ROOT, "clone", time(5))
        .unwrap();
    // A batch that rewrites the file's link count through the window engine.
    volume
        .run_batch(
            &[BatchOp::DeleteFile {
                parent_id: dir,
                name: "alias",
            }],
            time(5),
        )
        .unwrap();
    // Directory: child creation and removal rewrite its record.
    volume
        .create_file_in_directory(dir, "child", b"x", time(5))
        .unwrap();
    volume.delete_file(dir, "child", time(5)).unwrap();
    volume
        .rename(OBJECT_ROOT, "dir", OBJECT_ROOT, "dir2", time(5))
        .unwrap();
    // Symlink: rename rewrites the record and keeps the inline target.
    volume
        .rename(OBJECT_ROOT, "link", OBJECT_ROOT, "link2", time(5))
        .unwrap();

    let mut volume = mount(checked(volume)).unwrap();
    assert_eq!(
        volume.security_descriptor(file).unwrap(),
        expect(UNKNOWN_FORMAT, 1, false, &file_bytes)
    );
    assert_eq!(
        volume.security_descriptor(dir).unwrap(),
        expect(UNKNOWN_FORMAT, 2, false, &dir_bytes)
    );
    assert_eq!(
        volume.security_descriptor(link).unwrap(),
        expect(UNKNOWN_FORMAT, 3, false, &link_bytes)
    );
    assert_eq!(
        volume.security_descriptor(OBJECT_ROOT).unwrap(),
        expect(UNKNOWN_FORMAT, 4, false, b"root")
    );
    let mut target = [0u8; 32];
    let length = volume.read_link(link, &mut target).unwrap();
    assert_eq!(&target[..length], b"dir/target");
    assert_eq!(volume.stat(file).unwrap().unwrap().created, time(-5));
    // The clone is a new object: it shares data and carries no descriptor.
    assert_eq!(volume.security_descriptor(clone).unwrap(), None);
    assert_eq!(volume.read_file(clone).unwrap().len(), 9000);
    checked(volume);
}

#[test]
fn a_protection_edit_cannot_destroy_or_silently_contradict_a_descriptor() {
    let mut volume = mount(formatted()).unwrap();
    let bytes = blob(4500, 9);
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"secret", time(2))
        .unwrap();
    volume.set_object_protection(file, 0x0f, time(2)).unwrap();
    volume
        .set_security_descriptor(file, UNKNOWN_FORMAT, 1, &bytes, time(3))
        .unwrap();

    // Strict is the default: the edit is refused and nothing changes.
    assert!(matches!(
        volume.set_object_protection(file, 0, time(4)),
        Err(CoreError::SecurityProjectionRefused)
    ));
    let stat = volume.stat(file).unwrap().unwrap();
    assert!(matches!(
        volume.restore_object_metadata(
            file,
            PreservedMetadata {
                protection: 0,
                created: stat.created,
                modified: stat.modified,
                changed: stat.changed,
            },
        ),
        Err(CoreError::SecurityProjectionRefused)
    ));
    let stat = volume.stat(file).unwrap().unwrap();
    assert_eq!((stat.protection, stat.changed), (0x0f, time(3)));
    assert_eq!(
        volume.security_descriptor(file).unwrap(),
        expect(UNKNOWN_FORMAT, 1, false, &bytes)
    );
    // Writing the value already present is the documented no-op.
    volume.set_object_protection(file, 0x0f, time(4)).unwrap();
    // An object without a descriptor is edited as before.
    let plain = volume
        .create_file_in_directory(OBJECT_ROOT, "plain", b"", time(4))
        .unwrap();
    volume.set_object_protection(plain, 0xf0, time(4)).unwrap();
    assert_eq!(volume.stat(plain).unwrap().unwrap().protection, 0xf0);

    // Preserve: the edit lands, every descriptor byte stays, and the
    // divergence is durable.
    volume.set_security_projection_policy(SecurityProjectionPolicy::Preserve);
    volume.set_object_protection(file, 0, time(5)).unwrap();
    let mut volume = mount(checked(volume)).unwrap();
    assert_eq!(volume.stat(file).unwrap().unwrap().protection, 0);
    assert_eq!(
        volume.security_descriptor(file).unwrap(),
        expect(UNKNOWN_FORMAT, 1, true, &bytes)
    );
    // The policy is host runtime state: a fresh mount is strict again.
    assert!(matches!(
        volume.set_object_protection(file, 0x0f, time(6)),
        Err(CoreError::SecurityProjectionRefused)
    ));
    // Supplying a descriptor again reconciles the object.
    let reconciled = blob(10, 4);
    volume
        .set_security_descriptor(file, UNKNOWN_FORMAT, 2, &reconciled, time(7))
        .unwrap();
    assert_eq!(
        volume.security_descriptor(file).unwrap(),
        expect(UNKNOWN_FORMAT, 2, false, &reconciled)
    );
    // The explicit downgrade is the one operation that discards the bytes;
    // after it the classic edit is allowed.
    volume.clear_security_descriptor(file, time(8)).unwrap();
    assert_eq!(volume.security_descriptor(file).unwrap(), None);
    volume.clear_security_descriptor(file, time(8)).unwrap();
    volume.set_object_protection(file, 0x0f, time(9)).unwrap();
    checked(volume);
}

#[test]
fn descriptor_blocks_die_with_their_object_on_every_removal_path() {
    let mut volume = mount(formatted()).unwrap();
    let bytes = blob(9000, 5);
    let direct = volume
        .create_file_in_directory(OBJECT_ROOT, "direct", b"a", time(2))
        .unwrap();
    let batched = volume
        .create_file_in_directory(OBJECT_ROOT, "batched", b"b", time(2))
        .unwrap();
    let replaced = volume
        .create_file_in_directory(OBJECT_ROOT, "replaced", b"c", time(2))
        .unwrap();
    volume
        .create_file_in_directory(OBJECT_ROOT, "incoming", b"d", time(2))
        .unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(2))
        .unwrap();
    let link = volume
        .create_symlink(OBJECT_ROOT, "link", "direct", time(2))
        .unwrap();
    for id in [direct, batched, replaced, dir, link] {
        volume
            .set_security_descriptor(id, UNKNOWN_FORMAT, 1, &bytes, time(3))
            .unwrap();
    }
    // Replacing a descriptor retires the old chain.
    volume
        .set_security_descriptor(direct, UNKNOWN_FORMAT, 2, &blob(3, 6), time(3))
        .unwrap();
    let mut volume = mount(checked(volume)).unwrap();

    volume.delete_file(OBJECT_ROOT, "direct", time(4)).unwrap();
    volume
        .remove_directory(OBJECT_ROOT, "dir", time(4))
        .unwrap();
    volume.unlink_symlink(OBJECT_ROOT, "link", time(4)).unwrap();
    volume
        .run_batch(
            &[
                BatchOp::DeleteFile {
                    parent_id: OBJECT_ROOT,
                    name: "batched",
                },
                BatchOp::Rename {
                    source_parent_id: OBJECT_ROOT,
                    source_name: "incoming",
                    target_parent_id: OBJECT_ROOT,
                    target_name: "replaced",
                    replace: true,
                },
            ],
            time(4),
        )
        .unwrap();
    for id in [direct, batched, replaced, dir, link] {
        assert!(matches!(
            volume.security_descriptor(id),
            Err(CoreError::NotFound)
        ));
    }
    // The checker owns the verdict: a leaked or doubly owned segment is a
    // finding, and there is none.
    checked(volume);
}

#[test]
fn the_checker_rejects_a_damaged_or_foreign_descriptor_chain() {
    let mut volume = mount(formatted()).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    volume
        .set_security_descriptor(file, UNKNOWN_FORMAT, 1, &blob(5000, 8), time(3))
        .unwrap();
    let mut clean = checked(volume);

    let block_size = clean.block_size();
    let mut block = vec![0u8; block_size];
    let mut segments = Vec::new();
    for lba in 0..clean.total_blocks() {
        clean.read_block(lba, &mut block).unwrap();
        if BlockHeader::verify(&block, block_type::SECURITY_DESCRIPTOR).is_ok() {
            segments.push(lba);
        }
    }
    assert_eq!(segments.len(), 2);

    // Negative controls: each resealed corruption of a segment is a checker
    // error, so the clean verdicts above are evidence.
    for (offset, value) in [(8usize, 0xeeu8), (44, 1), (48, 1)] {
        let mut damaged = clean.clone();
        damaged.read_block(segments[0], &mut block).unwrap();
        let header = BlockHeader::verify(&block, block_type::SECURITY_DESCRIPTOR).unwrap();
        let mut owner = header.owner;
        if offset == 8 {
            owner ^= u64::from(value);
        } else {
            block[offset] ^= value;
        }
        BlockHeader { owner, ..header }.seal(&mut block);
        damaged.write_block(segments[0], &block).unwrap();
        let report = check_device(&mut damaged);
        assert!(!report.errors.is_empty(), "offset {offset} went unnoticed");
    }
}

#[test]
fn volumes_without_the_feature_refuse_descriptors_and_unqualified_combinations_do_not_mount() {
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs(&mut dev, &params()).unwrap();
    let mut volume = mount(dev).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"x", time(2))
        .unwrap();
    assert!(matches!(
        volume.set_security_descriptor(file, UNKNOWN_FORMAT, 1, b"abc", time(3)),
        Err(CoreError::FeatureDisabled(_))
    ));
    assert_eq!(volume.security_descriptor(file).unwrap(), None);
    volume.set_object_protection(file, 1, time(3)).unwrap();
    checked(volume);

    // Descriptors together with persistent snapshots are unqualified.
    let mut dev = formatted();
    let mut block = vec![0u8; 4096];
    dev.read_block(0, &mut block).unwrap();
    let mut ident = Identification::decode(&block).unwrap();
    ident.features.incompat |= INCOMPAT_PERSISTENT_SNAPSHOTS;
    dev.write_block(0, &ident.encode(4096).unwrap()).unwrap();
    assert!(matches!(
        mount(dev),
        Err(CoreError::UnsupportedIncompatFeatures(_))
    ));
}

#[test]
fn orphan_cleanup_and_intent_log_replay_retire_the_descriptor() {
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_security_descriptors(
        &mut dev,
        &MkfsParams {
            log_slots: 8,
            ..params()
        },
    )
    .unwrap();
    let mut volume = mount(dev).unwrap();
    let bytes = blob(8100, 11);
    let orphaned = volume
        .create_file_in_directory(OBJECT_ROOT, "orphaned", &[7; 5000], time(2))
        .unwrap();
    let logged = volume
        .create_file_in_directory(OBJECT_ROOT, "logged", b"y", time(2))
        .unwrap();
    let kept = volume
        .create_file_in_directory(OBJECT_ROOT, "kept", b"z", time(2))
        .unwrap();
    for id in [orphaned, logged, kept] {
        volume
            .set_security_descriptor(id, UNKNOWN_FORMAT, 1, &bytes, time(3))
            .unwrap();
    }

    // The orphan keeps its descriptor while it waits for cleanup.
    assert_eq!(
        volume
            .orphan_file(OBJECT_ROOT, "orphaned", time(4))
            .unwrap(),
        orphaned
    );
    let mut volume = mount(checked(volume)).unwrap();
    while volume
        .cleanup_orphan(orphaned, time(5))
        .unwrap()
        .still_pending
    {}
    assert_eq!(volume.orphan_count().unwrap(), 0);

    // A delete made durable by the intent log only, then a power cut.
    volume
        .window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "logged",
            },
            time(6),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    let cut = volume.device_mut().clone();
    drop(volume);
    let mut volume = mount(cut).unwrap();
    // Replay moved the final link into the orphan directory; the descriptor
    // stays with the object until cleanup removes both.
    assert_eq!(volume.orphan_count().unwrap(), 1);
    assert_eq!(
        volume.security_descriptor(logged).unwrap(),
        expect(UNKNOWN_FORMAT, 1, false, &bytes)
    );
    while volume
        .cleanup_orphan(logged, time(7))
        .unwrap()
        .still_pending
    {}
    assert!(matches!(
        volume.security_descriptor(logged),
        Err(CoreError::NotFound)
    ));
    assert_eq!(
        volume.security_descriptor(kept).unwrap(),
        expect(UNKNOWN_FORMAT, 1, false, &bytes)
    );
    checked(volume);
}

#[test]
fn every_power_cut_leaves_the_old_or_the_new_descriptor_state() {
    // Four transitions: attach, replace, preserve-mode protection edit, and
    // deletion. Each modeled cut mounts to exactly the state before or after
    // the transition, with a clean checker verdict.
    let old_bytes = blob(4100, 1);
    let new_bytes = blob(8200, 2);
    for transition in 0..4 {
        let mut volume = mount(formatted()).unwrap();
        let file = volume
            .create_file_in_directory(OBJECT_ROOT, "file", b"payload", time(2))
            .unwrap();
        if transition > 0 {
            volume
                .set_security_descriptor(file, UNKNOWN_FORMAT, 1, &old_bytes, time(3))
                .unwrap();
        }
        let base = volume.into_device();
        let mut recording = mount(RecordingBackend::new(base.clone())).unwrap();
        match transition {
            0 => recording
                .set_security_descriptor(file, UNKNOWN_FORMAT, 1, &old_bytes, time(4))
                .unwrap(),
            1 => recording
                .set_security_descriptor(file, UNKNOWN_FORMAT, 2, &new_bytes, time(4))
                .unwrap(),
            2 => {
                recording.set_security_projection_policy(SecurityProjectionPolicy::Preserve);
                recording
                    .set_object_protection(file, 0x55, time(4))
                    .unwrap();
            }
            _ => recording.delete_file(OBJECT_ROOT, "file", time(4)).unwrap(),
        }
        let (_, log) = recording.into_device().into_parts();
        let before = if transition == 0 {
            None
        } else {
            expect(UNKNOWN_FORMAT, 1, false, &old_bytes)
        };
        let after = match transition {
            0 => expect(UNKNOWN_FORMAT, 1, false, &old_bytes),
            1 => expect(UNKNOWN_FORMAT, 2, false, &new_bytes),
            2 => expect(UNKNOWN_FORMAT, 1, true, &old_bytes),
            _ => None,
        };
        let mut outcomes = [0u32, 0];
        for cut in 0..=log.len() {
            for_each_crash_state(&base, &log, cut, |state| {
                let mut volume = mount(state.image).unwrap();
                let present = volume.lookup_in_directory(OBJECT_ROOT, "file").unwrap();
                let current = match present {
                    Some(_) => {
                        let protection = volume.stat(file).unwrap().unwrap().protection;
                        let descriptor = volume.security_descriptor(file).unwrap();
                        // The protection value and the divergence mark move
                        // together or not at all.
                        if transition == 2 {
                            let diverged = descriptor.as_ref().unwrap().projection_diverged;
                            assert_eq!(protection == 0x55, diverged);
                        }
                        descriptor
                    }
                    None => {
                        assert_eq!(transition, 3);
                        None
                    }
                };
                let is_after = if transition == 3 {
                    present.is_none()
                } else {
                    current == after
                };
                if is_after {
                    outcomes[1] += 1;
                } else {
                    assert_eq!(current, before);
                    outcomes[0] += 1;
                }
                checked(volume);
            });
        }
        assert!(
            outcomes.iter().all(|&n| n > 0),
            "{transition}: {outcomes:?}"
        );
        eprintln!("security transition {transition} crash states old/new: {outcomes:?}");
    }
}
