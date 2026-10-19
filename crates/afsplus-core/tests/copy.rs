//! `afsplus-copy` moves a volume's live tree into a freshly formatted one,
//! here to gain persistent snapshots, with bytes, holes, links and metadata.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::volume::{AttributeWriteMode, PreservedMetadata, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_options, mount, mount_with_snapshot_limits, MkfsOptions, MkfsParams, MountOptions,
    NamePolicy, Volume,
};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

const BLOCKS: u64 = 64 * 256;
const LIMITS: SnapshotWorkLimits = SnapshotWorkLimits {
    max_edit_records: 4096,
    max_views: 8,
    reclaim_records: 64,
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("afsplus-copy-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn format(path: &Path, snapshots: bool) {
    fs::File::create(path).unwrap();
    let mut device = FileBackend::open(path, DEFAULT_BLOCK_SIZE, BLOCKS).unwrap();
    mkfs_with_options(
        &mut device,
        &MkfsParams {
            uuid: [if snapshots { 0x5D } else { 0x5C }; 16],
            label: "SYS".into(),
            region_size: 4096,
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
    device.flush().unwrap();
}

fn device(path: &Path) -> FileBackend {
    let mut device = FileBackend::open_sized_by_file(path, DEFAULT_BLOCK_SIZE).unwrap();
    device.set_total_blocks(BLOCKS);
    device
}

fn copy(source: &Path, destination: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_afsplus-copy"))
        .arg(source)
        .arg(destination)
        .output()
        .unwrap()
}

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 7,
    }
}

/// A tree with each kind of object and each kind of metadata.
fn populate(volume: &mut Volume<FileBackend>) -> (u64, u64, u64) {
    let now = at(1_800_000_000);
    let prefs = volume.create_directory(OBJECT_ROOT, "Prefs", now).unwrap();
    let env = volume.create_directory(prefs, "Env-Archive", now).unwrap();
    let script = volume
        .create_file_in_directory(env, "Startup", b"Echo hello\n", now)
        .unwrap();
    volume
        .set_object_comment(script, "boot script", now)
        .unwrap();
    volume
        .set_attributes(
            script,
            &[("aros.icon", Some(b"icon".as_slice()))],
            AttributeWriteMode::Create,
            now,
        )
        .unwrap();
    // Data, a 3 MiB hole, data, then a zero tail kept by the size alone.
    let big = volume.create_file_in_root("big", b"head", now).unwrap();
    volume.write_file_at(big, 3 << 20, b"middle", now).unwrap();
    volume.truncate_file(big, 5 << 20, now).unwrap();
    volume.link_file(big, prefs, "big-link", now).unwrap();
    volume.create_symlink(prefs, "soft", "big", now).unwrap();
    volume.set_object_comment(prefs, "drawer", now).unwrap();
    volume
        .restore_object_metadata(
            script,
            PreservedMetadata {
                protection: 0x40,
                owner_uid: 501,
                owner_gid: 20,
                created: at(1_700_000_001),
                modified: at(1_700_000_002),
                changed: at(1_700_000_003),
            },
        )
        .unwrap();
    volume
        .restore_object_metadata(
            prefs,
            PreservedMetadata {
                protection: 0,
                owner_uid: 0,
                owner_gid: 0,
                created: at(1_600_000_001),
                modified: at(1_600_000_002),
                changed: at(1_600_000_003),
            },
        )
        .unwrap();
    volume.sync().unwrap();
    (prefs, script, big)
}

#[test]
fn a_volume_gains_snapshots_with_its_tree_and_metadata_intact() {
    let dir = scratch("gain");
    let (source, destination) = (dir.join("old.afsp"), dir.join("new.afsp"));
    format(&source, false);
    let mut volume = mount(device(&source)).unwrap();
    let (prefs, script, _big) = populate(&mut volume);
    let expected = (
        volume.visible_metadata(prefs).unwrap().unwrap(),
        volume.visible_metadata(script).unwrap().unwrap(),
    );
    drop(volume);
    let before = fs::read(&source).unwrap();
    format(&destination, true);

    let output = copy(&source, &destination);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("1 hard links"), "{report}");
    assert!(report.contains("objects compared identical"), "{report}");
    assert_eq!(
        fs::read(&source).unwrap(),
        before,
        "the source is never written"
    );

    // The copy needs a snapshot mount, and takes a snapshot.
    assert!(mount(device(&destination)).is_err());
    let mut copied =
        mount_with_snapshot_limits(device(&destination), MountOptions::default(), LIMITS).unwrap();
    let prefs2 = copied.lookup_root("Prefs").unwrap().unwrap();
    let env2 = copied
        .lookup_in_directory(prefs2, "Env-Archive")
        .unwrap()
        .unwrap();
    let script2 = copied
        .lookup_in_directory(env2, "Startup")
        .unwrap()
        .unwrap();
    let big2 = copied.lookup_root("big").unwrap().unwrap();
    assert_eq!(
        copied.lookup_in_directory(prefs2, "big-link").unwrap(),
        Some(big2)
    );
    for (from, to) in [(expected.0, prefs2), (expected.1, script2)] {
        let got = copied.visible_metadata(to).unwrap().unwrap();
        assert_eq!(PreservedMetadata::from(got), PreservedMetadata::from(from));
    }
    assert_eq!(copied.object_comment(script2).unwrap(), "boot script");
    assert_eq!(
        copied.attribute(script2, "aros.icon").unwrap(),
        Some(b"icon".to_vec())
    );
    let meta = copied.visible_metadata(big2).unwrap().unwrap();
    assert_eq!((meta.size_bytes, meta.link_count), (5 << 20, 2));
    assert!(meta.allocated_bytes < 1 << 20, "holes stay holes");
    let mut middle = [0u8; 6];
    copied.read_file_at(big2, 3 << 20, &mut middle).unwrap();
    assert_eq!(&middle, b"middle");
    copied.snapshot_create(Timespec::default()).unwrap();
    assert_eq!(copied.snapshot_list(0, 4).unwrap().entries.len(), 1);
}

#[test]
fn a_non_empty_destination_and_a_source_with_snapshots_are_refused() {
    let dir = scratch("refuse");
    let (source, destination) = (dir.join("old.afsp"), dir.join("new.afsp"));
    format(&source, true);
    format(&destination, true);
    let mut volume =
        mount_with_snapshot_limits(device(&destination), MountOptions::default(), LIMITS).unwrap();
    volume
        .create_file_in_root("taken", b"x", Timespec::default())
        .unwrap();
    volume.sync().unwrap();
    drop(volume);
    let output = copy(&source, &destination);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-empty"));

    let fresh = dir.join("fresh.afsp");
    format(&fresh, true);
    let mut volume =
        mount_with_snapshot_limits(device(&source), MountOptions::default(), LIMITS).unwrap();
    volume
        .create_file_in_root("data", b"x", Timespec::default())
        .unwrap();
    volume.snapshot_create(Timespec::default()).unwrap();
    drop(volume);
    let output = copy(&source, &fresh);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("holds snapshots"));
}
