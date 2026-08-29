//! Smallest-mountable-image and first-transaction behavior
//! (`implementation/peer-review-prototype-plan.md`, steps 3 and 4), plus I/O
//! accounting from the very first prototype (roadmap Stage A requirement).

use afsplus_block::{FileBackend, MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, CoreError, MkfsParams};
use afsplus_format::object::ObjectType;
use afsplus_format::Timespec;

const BS: usize = 4096;

fn params() -> MkfsParams {
    MkfsParams {
        uuid: [42u8; 16],
        label: "TestVol".into(),
        timestamp: Timespec { seconds: 1_780_000_000, nanoseconds: 0 },
    }
}

fn ts(seconds: i64) -> Timespec {
    Timespec { seconds, nanoseconds: 0 }
}

#[test]
fn mkfs_then_mount_yields_empty_root_at_generation_1() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params()).unwrap();

    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);

    let vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 1);
    assert!(vol.list_root().is_empty());
    assert_eq!(vol.ident().label, "TestVol");
    let root = vol.stat(afsplus_format::OBJECT_ROOT).unwrap();
    assert_eq!(root.object_type, ObjectType::Directory);
    assert_eq!(root.link_count, 1);
}

#[test]
fn create_commit_remount_verify() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params()).unwrap();
    let mut vol = mount(dev).unwrap();

    let id = vol.create_file_in_root("hello.txt", ts(1_780_000_100)).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_eq!(vol.lookup_root("hello.txt"), Some(id));

    // Remount from the same device: the committed state must be identical.
    let dev = vol.into_device();
    let vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_eq!(vol.lookup_root("hello.txt"), Some(id));
    let record = vol.stat(id).unwrap();
    assert_eq!(record.object_type, ObjectType::File);
    assert_eq!(record.link_count, 1);
    assert_eq!(record.size_bytes, 0);
    assert_eq!(record.modified.seconds, 1_780_000_100);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn several_transactions_alternate_checkpoint_slots() {
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(&mut dev, &params()).unwrap();
    let mut vol = mount(dev).unwrap();
    for i in 0..10 {
        vol.create_file_in_root(&format!("file-{i}.txt"), ts(1_780_000_000 + i)).unwrap();
    }
    assert_eq!(vol.generation(), 11);
    assert_eq!(vol.list_root().len(), 10);

    let vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.generation(), 11);
    assert_eq!(vol.list_root().len(), 10);
}

#[test]
fn duplicate_and_invalid_names_are_rejected_without_state_change() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params()).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("hello.txt", ts(0)).unwrap();

    assert!(matches!(vol.create_file_in_root("hello.txt", ts(1)), Err(CoreError::AlreadyExists)));
    assert!(matches!(vol.create_file_in_root("a/b", ts(1)), Err(CoreError::InvalidName(_))));
    assert!(matches!(vol.create_file_in_root("", ts(1)), Err(CoreError::InvalidName(_))));
    assert_eq!(vol.generation(), 2, "failed operations must not advance the committed state");
}

#[test]
fn out_of_space_is_reported_and_state_survives() {
    // 10 blocks: layout overhead 6, one transaction needs 4. The second must fail.
    let mut dev = MemoryBackend::new(BS, 10);
    mkfs(&mut dev, &params()).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("first.txt", ts(0)).unwrap();
    assert!(matches!(vol.create_file_in_root("second.txt", ts(1)), Err(CoreError::NoSpace)));

    let vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_eq!(vol.list_root().len(), 1);
}

#[test]
fn first_transaction_io_accounting() {
    // Write amplification is tracked from the first prototype onward. The
    // exact numbers are prototype characteristics, not format guarantees, but
    // a silent regression here is exactly what the accounting must catch.
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params()).unwrap();
    let mut vol = mount(TraceBackend::new(dev)).unwrap();
    let mount_stats = vol.device_mut().stats();
    vol.device_mut().reset();

    vol.create_file_in_root("hello.txt", ts(0)).unwrap();
    let stats = vol.device_mut().stats();

    // COW transaction: file record + new dir block + new root record + new
    // object map, then the checkpoint. Two barriers.
    assert_eq!(stats.writes, 5, "unexpected metadata write amplification");
    assert_eq!(stats.flushes, 2, "commit must use exactly two barriers");
    assert!(
        stats.reads <= 8,
        "post-commit re-walk grew unexpectedly: {} reads",
        stats.reads
    );
    // Mount reads: ident + 2 slots + walk (omap, root record, root dir).
    assert!(mount_stats.reads <= 8 && mount_stats.writes == 0);
}

#[test]
fn file_image_end_to_end_with_json_report() {
    let dir = std::env::temp_dir().join(format!("afsplus-basic-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("volume.img");

    let mut dev = FileBackend::create(&path, BS, 128).unwrap();
    mkfs(&mut dev, &params()).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("on-disk.txt", ts(7)).unwrap();
    drop(vol);

    let mut dev = FileBackend::open(&path, BS, 128).unwrap();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let json = report.render_json();
    assert!(json.contains("\"schema_version\":1"));
    assert!(json.contains("\"clean\":true"));
    assert!(json.contains("\"generation\":2"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn torn_newest_checkpoint_falls_back_to_previous_generation() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params()).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("hello.txt", ts(0)).unwrap();
    let mut dev = vol.into_device();

    // Generation 2 lives in slot B (LBA 2). Tear it.
    let mut torn = dev.peek(2);
    for byte in &mut torn[1000..] {
        *byte = 0xEE;
    }
    dev.apply_raw(2, &torn);

    let report = check_device(&mut dev);
    assert!(report.is_clean(), "fallback is an allowed state: {:?}", report.errors);
    let vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 1, "mount must fall back to the intact checkpoint");
    assert!(vol.list_root().is_empty());
}
