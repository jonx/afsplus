//! Owned chains under persistent snapshots (ADR-109): a retained view keeps
//! the attribute set and the security descriptor it captured, whatever the
//! live side does to them, and releases them when it is deleted.
use std::collections::BTreeSet;

use afsplus_block::{for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_check::diff::{diff_devices, DiffOptions};
use afsplus_check::explain::Explainer;
use afsplus_core::mount::select_checkpoint;
use afsplus_core::verify::load_committed_state;
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{
    mkfs_with_snapshots_and_security_descriptors, mount_with_snapshot_limits, AttributeWriteMode,
    MkfsParams, MountOptions, NamePolicy, Volume,
};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, OBJECT_ROOT};

use AttributeWriteMode::Upsert;

const LIMITS: SnapshotWorkLimits = SnapshotWorkLimits {
    max_edit_records: 4096,
    max_views: 8,
    reclaim_records: 8,
};

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 9,
    }
}

fn formatted(blocks: u64) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, blocks);
    mkfs_with_snapshots_and_security_descriptors(
        &mut dev,
        &MkfsParams {
            uuid: [0xc5; 16],
            label: "Chains".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    dev
}

fn open<D: BlockDevice>(dev: D) -> Volume<D> {
    mount_with_snapshot_limits(dev, MountOptions::default(), LIMITS).unwrap()
}

fn checked<D: BlockDevice>(volume: Volume<D>) -> D {
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    dev
}

fn bytes(seed: u8) -> Vec<u8> {
    (0..9000u32).map(|i| seed ^ (i % 253) as u8).collect()
}

/// Blocks that carry a chain segment of `object_id` with this magic.
fn segments<D: BlockDevice>(dev: &mut D, magic: &[u8; 4], object_id: u64) -> BTreeSet<u64> {
    let mut block = vec![0u8; dev.block_size()];
    (0..dev.total_blocks())
        .filter(|lba| {
            dev.read_block(*lba, &mut block).unwrap();
            &block[0..4] == magic && block[8..16] == object_id.to_le_bytes()
        })
        .collect()
}

fn snapshot_owned<D: BlockDevice>(dev: &mut D) -> BTreeSet<u64> {
    let mut block = vec![0u8; dev.block_size()];
    dev.read_block(0, &mut block).unwrap();
    let ident = Identification::decode(&block).unwrap();
    let selection = select_checkpoint(dev, &ident).unwrap();
    let state = load_committed_state(dev, &ident, &selection.chosen).unwrap();
    state.snapshot_owned_blocks.iter().copied().collect()
}

#[test]
fn a_view_keeps_the_chains_it_captured_and_its_deletion_releases_them() {
    let mut volume = open(formatted(1024));
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"content", time(2))
        .unwrap();
    let doomed = volume
        .create_file_in_directory(OBJECT_ROOT, "doomed", b"content", time(2))
        .unwrap();
    for id in [file, doomed] {
        volume
            .set_attributes(
                id,
                &[
                    ("user.big", Some(&bytes(1))),
                    ("user.note", Some(b"captured")),
                ],
                Upsert,
                time(3),
            )
            .unwrap();
        volume
            .set_security_descriptor(id, 0x7fff_0009, 2, &bytes(2), time(3))
            .unwrap();
    }
    let mut dev = checked(volume);
    let captured: BTreeSet<u64> = [b"AFSA", b"AFSX"]
        .into_iter()
        .flat_map(|magic| {
            let mut all = segments(&mut dev, magic, file);
            all.extend(segments(&mut dev, magic, doomed));
            all
        })
        .collect();
    assert_eq!(
        captured.len(),
        12,
        "two objects, two chains, three segments"
    );
    let before = dev.clone();
    let mut volume = open(dev);
    let snapshot = volume.snapshot_create(time(4)).unwrap();

    // The live side replaces, removes and deletes; a clone copies the
    // chains of an object the view captured.
    let clone = volume
        .clone_file(file, OBJECT_ROOT, "clone", time(5))
        .unwrap();
    volume
        .set_attributes(
            file,
            &[("user.big", None), ("user.note", Some(b"live"))],
            Upsert,
            time(5),
        )
        .unwrap();
    volume
        .set_security_descriptor(file, 0x7fff_0009, 3, b"live descriptor", time(5))
        .unwrap();
    volume.delete_file(OBJECT_ROOT, "doomed", time(5)).unwrap();
    // Allocation pressure: every free block is written at least once, so a
    // released captured segment would be overwritten.
    for round in 0..3u8 {
        let mut n = 0;
        while volume
            .create_file_in_directory(
                OBJECT_ROOT,
                &format!("fill-{round}-{n}"),
                &vec![round; 64 * 4096],
                time(6),
            )
            .is_ok()
        {
            n += 1;
        }
        assert!(n > 0 || round > 0);
        for i in 0..n {
            volume
                .delete_file(OBJECT_ROOT, &format!("fill-{round}-{i}"), time(7))
                .unwrap();
        }
    }

    let mut volume = open(checked(volume));
    let view = volume.snapshot_open(snapshot).unwrap();
    for id in [file, doomed] {
        assert_eq!(
            volume.snapshot_attribute_names(&view, id).unwrap().unwrap(),
            ["user.big", "user.note"]
        );
        assert_eq!(
            volume.snapshot_attribute(&view, id, "user.big").unwrap(),
            Some(Some(bytes(1)))
        );
        assert_eq!(
            volume.snapshot_attribute(&view, id, "user.absent").unwrap(),
            Some(None)
        );
        let descriptor = volume
            .snapshot_security_descriptor(&view, id)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!((descriptor.version, descriptor.bytes), (2, bytes(2)));
    }
    // The view does not hold the clone; the live side has moved on.
    assert_eq!(volume.snapshot_attribute_names(&view, clone).unwrap(), None);
    assert_eq!(
        volume.snapshot_security_descriptor(&view, clone).unwrap(),
        None
    );
    drop(view);
    assert_eq!(volume.attribute_names(file).unwrap(), ["user.note"]);
    assert_eq!(volume.attribute(clone, "user.big").unwrap(), Some(bytes(1)));
    assert_eq!(
        volume.security_descriptor(clone).unwrap().unwrap().bytes,
        bytes(2)
    );

    // The captured segments are still the captured bytes, owned by the
    // ledger and by no live object; explain and the image diff agree.
    let mut dev = checked(volume);
    let mut block = vec![0u8; 4096];
    let mut old = vec![0u8; 4096];
    let mut source = before.clone();
    for lba in &captured {
        dev.read_block(*lba, &mut block).unwrap();
        source.read_block(*lba, &mut old).unwrap();
        assert!(block == old, "captured segment {lba} changed");
    }
    let owned = snapshot_owned(&mut dev);
    assert!(captured.is_subset(&owned));
    let explainer = Explainer::load(&mut dev).unwrap();
    assert_eq!(explainer.problems, Vec::<String>::new());
    for lba in &captured {
        assert!(explainer
            .explain_block(&mut dev, *lba)
            .unwrap()
            .is_unowned());
    }
    let diff = diff_devices(&mut source, &mut dev.clone(), DiffOptions::default()).unwrap();
    assert_eq!(diff.problems, Vec::<String>::new());

    // Deleting the view releases what no live object owns: after the
    // reclaim delay the captured blocks are free or reused, and nothing is
    // allocated without an owner.
    let mut volume = open(dev);
    volume.snapshot_delete(snapshot, time(8)).unwrap();
    for n in 0..12 {
        volume
            .set_object_comment(file, &format!("turn {n}"), time(9 + n))
            .unwrap();
    }
    let mut dev = checked(volume);
    // Reclaim runs in bounded steps; whatever it has not reached yet is
    // still ledger-owned, and no captured block is allocated without an
    // owner. Those the live side took back have a live role.
    let owned = snapshot_owned(&mut dev);
    let explainer = Explainer::load(&mut dev).unwrap();
    let mut released = 0;
    for lba in &captured {
        let explanation = explainer.explain_block(&mut dev, *lba).unwrap();
        if explanation.is_unowned() {
            assert!(owned.contains(lba), "block {lba} is a leak");
        } else {
            released += 1;
        }
    }
    assert!(released > 0, "no captured block was released");
    let mut volume = open(dev);
    assert_eq!(volume.attribute(clone, "user.big").unwrap(), Some(bytes(1)));
    checked(volume);
}

#[test]
fn the_checker_proves_the_chains_of_a_retained_view() {
    for magic in [b"AFSA", b"AFSX"] {
        let mut volume = open(formatted(1024));
        let file = volume
            .create_file_in_directory(OBJECT_ROOT, "file", b"content", time(2))
            .unwrap();
        volume
            .set_attributes(file, &[("user.big", Some(&bytes(1)))], Upsert, time(3))
            .unwrap();
        volume
            .set_security_descriptor(file, 0x7fff_0009, 2, &bytes(2), time(3))
            .unwrap();
        let mut dev = checked(volume);
        let captured = segments(&mut dev, magic, file);
        let mut volume = open(dev);
        volume.snapshot_create(time(4)).unwrap();
        volume
            .set_attributes(file, &[("user.big", None)], Upsert, time(5))
            .unwrap();
        volume.clear_security_descriptor(file, time(5)).unwrap();
        let mut dev = checked(volume);
        // Only the view reaches these segments now. Break one.
        let lba = *captured.iter().nth(1).unwrap();
        let mut block = vec![0u8; 4096];
        dev.read_block(lba, &mut block).unwrap();
        block[200] ^= 1;
        dev.write_block(lba, &block).unwrap();
        let report = check_device(&mut dev);
        assert!(
            report.errors.iter().any(|e| e.contains("snapshot")),
            "{:?}",
            report.errors
        );
    }
}

#[test]
fn every_power_cut_of_a_replacement_under_a_view_keeps_the_captured_set() {
    let mut volume = open(formatted(1024));
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"stable", time(2))
        .unwrap();
    volume
        .set_attributes(file, &[("user.big", Some(&bytes(1)))], Upsert, time(3))
        .unwrap();
    volume
        .set_security_descriptor(file, 0x7fff_0009, 2, &bytes(2), time(3))
        .unwrap();
    let snapshot = volume.snapshot_create(time(4)).unwrap();
    let base = volume.into_device();

    let mut recording = open(RecordingBackend::new(base.clone()));
    recording
        .set_attributes(
            file,
            &[("user.big", Some(&bytes(7)[..100]))],
            Upsert,
            time(5),
        )
        .unwrap();
    recording
        .set_security_descriptor(file, 0x7fff_0009, 3, b"live", time(6))
        .unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut live_states = BTreeSet::new();
    let mut states = 0u32;
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = open(state.image);
            let view = volume.snapshot_open(snapshot).unwrap();
            assert_eq!(
                volume.snapshot_attribute(&view, file, "user.big").unwrap(),
                Some(Some(bytes(1)))
            );
            let descriptor = volume
                .snapshot_security_descriptor(&view, file)
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(descriptor.bytes == bytes(2));
            drop(view);
            let live = volume.attribute(file, "user.big").unwrap().unwrap();
            assert!(live == bytes(1) || live == bytes(7)[..100]);
            let version = volume.security_descriptor(file).unwrap().unwrap().version;
            live_states.insert((live.len(), version));
            checked(volume);
            states += 1;
        });
    }
    assert_eq!(
        live_states,
        BTreeSet::from([(9000, 2), (100, 2), (100, 3)]),
        "old, attribute replaced, both replaced"
    );
    eprintln!("chain snapshot crash states: {states}");
}
