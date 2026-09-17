//! The volume label is committed state: a relabel is one commit, every
//! later commit carries it, and a power cut leaves the old label or the new.
use afsplus_block::{for_each_crash_state, BlockDevice, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::{
    mkfs, mount, mount_with_options, CoreError, MkfsParams, MountMode, MountOptions, NamePolicy,
    Volume,
};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, OBJECT_ROOT};

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 0,
    }
}

fn formatted(label: &str) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x1a; 16],
            label: label.into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 4,
            shared_extents: false,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    dev
}

fn checked<D: BlockDevice>(volume: Volume<D>) -> D {
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    dev
}

fn format_time_label(dev: &mut MemoryBackend) -> String {
    let mut block = vec![0u8; 4096];
    dev.read_block(0, &mut block).unwrap();
    Identification::decode(&block).unwrap().label
}

#[test]
fn a_relabel_is_durable_and_every_later_commit_carries_it() {
    let mut volume = mount(formatted("Work")).unwrap();
    assert_eq!(volume.volume_label(), "Work");
    volume.set_volume_label("Système 🜁").unwrap();
    assert_eq!(volume.volume_label(), "Système 🜁");
    // Ordinary commits after the relabel keep the label.
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"payload", time(2))
        .unwrap();
    volume.set_object_protection(file, 5, time(3)).unwrap();
    let mut dev = checked(volume);
    // The identification block is never rewritten.
    assert_eq!(format_time_label(&mut dev), "Work");
    assert_eq!(check_device(&mut dev).volume.unwrap().label, "Système 🜁");

    let mut volume = mount(dev).unwrap();
    assert_eq!(volume.volume_label(), "Système 🜁");
    assert_eq!(volume.read_file(file).unwrap(), b"payload");

    // The unchanged label publishes nothing.
    let generation = volume.checkpoint().generation;
    volume.set_volume_label("Système 🜁").unwrap();
    assert_eq!(volume.checkpoint().generation, generation);
    // The empty label and the 64-byte label are the bounds.
    volume.set_volume_label("").unwrap();
    assert_eq!(volume.volume_label(), "");
    let longest = "é".repeat(32);
    assert_eq!(longest.len(), 64);
    volume.set_volume_label(&longest).unwrap();
    assert_eq!(mount(checked(volume)).unwrap().volume_label(), longest);
}

#[test]
fn an_inadmissible_label_or_a_read_only_mount_changes_nothing() {
    let mut volume = mount(formatted("Work")).unwrap();
    let generation = volume.checkpoint().generation;
    for bad in ["x".repeat(65), "é".repeat(33), "nul\0inside".to_owned()] {
        assert!(matches!(
            volume.set_volume_label(&bad),
            Err(CoreError::InvalidMetadata(_))
        ));
    }
    assert_eq!(volume.volume_label(), "Work");
    assert_eq!(volume.checkpoint().generation, generation);
    let dev = checked(volume);
    let mut read_only = mount_with_options(
        dev,
        MountOptions {
            mode: MountMode::ReadOnly,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(
        read_only.set_volume_label("Other"),
        Err(CoreError::ReadOnly)
    ));
    assert_eq!(read_only.volume_label(), "Work");
}

#[test]
fn every_power_cut_leaves_the_old_label_or_the_new_one() {
    let mut volume = mount(formatted("Before")).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"stable", time(2))
        .unwrap();
    let base = volume.into_device();
    let mut recording = mount(RecordingBackend::new(base.clone())).unwrap();
    recording.set_volume_label("After").unwrap();
    let (_, log) = recording.into_device().into_parts();
    let mut outcomes = [0u32, 0];
    for cut in 0..=log.len() {
        for_each_crash_state(&base, &log, cut, |state| {
            let mut volume = mount(state.image).unwrap();
            match volume.volume_label() {
                "Before" => outcomes[0] += 1,
                "After" => outcomes[1] += 1,
                other => panic!("a third label: {other:?}"),
            }
            assert_eq!(volume.read_file(file).unwrap(), b"stable");
            // The surviving label is writable state, not a stuck one.
            volume.set_volume_label("Again").unwrap();
            assert_eq!(volume.volume_label(), "Again");
            checked(volume);
        });
    }
    assert!(outcomes.iter().all(|count| *count > 0), "{outcomes:?}");
    eprintln!("relabel crash states old/new: {outcomes:?}");
}

#[cfg(unix)]
#[test]
fn the_portable_c_reader_reports_the_same_label_and_the_same_fallback() {
    use afsplus_format::checkpoint::Checkpoint;
    use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
    use std::process::Command;

    let scratch = std::env::temp_dir().join(format!("afsplus-label-c-{}", std::process::id()));
    std::fs::create_dir(&scratch).unwrap();
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let probe = scratch.join("probe");
    let output = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()))
        .args([
            "-std=c99",
            "-pedantic",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsanitize=address,undefined",
        ])
        .arg("-I")
        .arg(repo.join("api"))
        .arg("-I")
        .arg(repo.join("spec"))
        .arg(repo.join("portable/c/reader.c"))
        .arg(repo.join("portable/c/tests/label_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let c_view = |dev: &mut MemoryBackend| -> Option<String> {
        let mut image = vec![0u8; dev.total_blocks() as usize * 4096];
        for lba in 0..dev.total_blocks() {
            dev.read_block(lba, &mut image[lba as usize * 4096..][..4096])
                .unwrap();
        }
        let path = scratch.join("image");
        std::fs::write(&path, image).unwrap();
        let output = Command::new(&probe).arg(&path).output().unwrap();
        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).unwrap())
    };
    let rust_view = |dev: &MemoryBackend| -> Option<String> {
        mount(dev.clone()).ok().map(|volume| {
            format!(
                "{} {}",
                volume.checkpoint().generation,
                volume.volume_label()
            )
        })
    };

    let mut volume = mount(formatted("Before")).unwrap();
    volume.set_volume_label("Après 🜁").unwrap();
    let after_generation = volume.checkpoint().generation;
    let mut dev = volume.into_device();
    let expected = format!("{after_generation} Après 🜁");
    assert_eq!(rust_view(&dev), Some(expected.clone()));
    assert_eq!(c_view(&mut dev), Some(expected));

    // Make the newest checkpoint's label field non-canonical in four ways and
    // reseal it: both readers refuse that slot and select the older
    // checkpoint, which still says "Before".
    let mut block = vec![0u8; 4096];
    let newest = (1..=2u64)
        .find(|lba| {
            dev.read_block(*lba, &mut block).unwrap();
            Checkpoint::decode(&block, &[0x1a; 16])
                .map(|checkpoint| checkpoint.generation == after_generation)
                .unwrap_or(false)
        })
        .unwrap();
    let fallback = format!("{} Before", after_generation - 1);
    for (offset, value, what) in [
        (96usize, 65u8, "length above the bound"),
        (97, 1, "reserved byte"),
        (104 + 60, b'x', "byte in the padding"),
        (104, 0xff, "invalid UTF-8"),
    ] {
        let mut damaged = dev.clone();
        damaged.read_block(newest, &mut block).unwrap();
        let header = BlockHeader::verify(&block, block_type::CHECKPOINT).unwrap();
        block[HEADER_SIZE + offset] = value;
        header.seal(&mut block);
        damaged.write_block(newest, &block).unwrap();
        assert_eq!(rust_view(&damaged), Some(fallback.clone()), "Rust: {what}");
        assert_eq!(c_view(&mut damaged), Some(fallback.clone()), "C: {what}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

/// A device that panics on the next write once armed: the way to unwind out
/// of the middle of a commit.
struct PanickingDevice {
    inner: MemoryBackend,
    armed: std::rc::Rc<std::cell::Cell<bool>>,
}

impl BlockDevice for PanickingDevice {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), afsplus_block::BlockError> {
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), afsplus_block::BlockError> {
        if self.armed.replace(false) {
            panic!("injected unwind inside a commit");
        }
        self.inner.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), afsplus_block::BlockError> {
        self.inner.flush()
    }
}

#[test]
fn an_unwound_relabel_leaves_nothing_for_a_later_commit_to_publish() {
    let armed = std::rc::Rc::new(std::cell::Cell::new(false));
    let mut volume = mount(PanickingDevice {
        inner: formatted("Before"),
        armed: armed.clone(),
    })
    .unwrap();
    armed.set(true);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = volume.set_volume_label("Never");
    }));
    assert!(
        unwound.is_err(),
        "the injected panic fired inside the relabel"
    );
    assert!(!armed.get());
    assert_eq!(volume.volume_label(), "Before");

    // An unrelated commit on the same Volume value. Whether the core accepts
    // it after an unwind or refuses it, the label it can publish is the
    // committed one: the relabel's intent lived in an argument, not a field.
    let created = volume.create_file_in_directory(OBJECT_ROOT, "later", b"x", time(9));
    assert_eq!(volume.volume_label(), "Before");
    let dev = volume.into_device().inner;
    let mut remounted = mount(dev).unwrap();
    assert_eq!(remounted.volume_label(), "Before");
    assert_eq!(
        remounted
            .lookup_in_directory(OBJECT_ROOT, "later")
            .unwrap()
            .is_some(),
        created.is_ok()
    );
    checked(remounted);
}

#[test]
fn a_relabel_survives_logged_writes_a_cut_and_replay() {
    let mut volume = mount(formatted("Before")).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &[1; 4096], time(2))
        .unwrap();
    volume.set_volume_label("After").unwrap();
    // Work made durable by the intent log only, then a power cut.
    volume
        .window_write_file_at(file, 0, &[2; 4096], time(3))
        .unwrap();
    volume.window_fsync().unwrap();
    let cut = volume.device_mut().clone();
    drop(volume);
    // Replay publishes a new checkpoint at mount; it carries the label.
    let mut volume = mount(cut).unwrap();
    assert_eq!(volume.volume_label(), "After");
    assert_eq!(volume.read_file(file).unwrap(), vec![2; 4096]);
    assert_eq!(mount(checked(volume)).unwrap().volume_label(), "After");
}

#[test]
fn a_relabel_on_a_snapshot_volume_keeps_its_roots_and_views() {
    use afsplus_core::volume::SnapshotWorkLimits;
    use afsplus_core::{mkfs_with_options, mount_with_snapshot_limits, MkfsOptions};

    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [0x1b; 16],
            label: "Before".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    let mut volume = mount_with_snapshot_limits(dev, MountOptions::default(), limits).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"captured", time(2))
        .unwrap();
    let snapshot = volume.snapshot_create(time(3)).unwrap();
    volume.set_volume_label("After").unwrap();
    assert!(volume.checkpoint().snapshot_roots.is_some());
    let dev = volume.into_device();
    let mut volume = mount_with_snapshot_limits(dev, MountOptions::default(), limits).unwrap();
    assert_eq!(volume.volume_label(), "After");
    let view = volume.snapshot_open(snapshot).unwrap();
    let mut captured = [0u8; 8];
    assert_eq!(
        volume
            .snapshot_read_file_at(&view, file, 0, &mut captured)
            .unwrap(),
        8
    );
    assert_eq!(&captured, b"captured");
    drop(view);
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.volume.unwrap().label, "After");
}
