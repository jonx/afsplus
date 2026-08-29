//! Smallest-mountable-image and transaction behavior over the region
//! allocator, plus I/O accounting from the very first prototype (roadmap
//! Stage A requirement).

use afsplus_block::{FileBackend, MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, CoreError, MkfsParams};
use afsplus_format::object::ObjectType;
use afsplus_format::Timespec;

const BS: usize = 4096;

fn params(region_size: u32) -> MkfsParams {
    MkfsParams {
        uuid: [42u8; 16],
        label: "TestVol".into(),
        region_size,
        timestamp: Timespec { seconds: 1_780_000_000, nanoseconds: 0 },
    }
}

fn ts(seconds: i64) -> Timespec {
    Timespec { seconds, nanoseconds: 0 }
}

#[test]
fn mkfs_then_mount_yields_empty_root_at_generation_1() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();

    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);

    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 1);
    assert!(vol.list_root().is_empty());
    assert_eq!(vol.ident().label, "TestVol");
    let root = vol.stat(afsplus_format::OBJECT_ROOT).unwrap().unwrap();
    assert_eq!(root.object_type, ObjectType::Directory);
    assert_eq!(root.link_count, 1);
    // 64 blocks minus 9 reserved (bootstrap + descriptor + bitmap slots)
    // minus 3 initial metadata.
    assert_eq!(vol.free_blocks(), 52);
}

#[test]
fn create_with_content_commit_remount_read_back() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();
    let mut vol = mount(dev).unwrap();

    let content = b"bonjour AFS+\n".repeat(400); // ~5 KiB -> 2 data blocks
    let id = vol.create_file_in_root("hello.txt", &content, ts(1_780_000_100)).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_eq!(vol.lookup_root("hello.txt"), Some(id));
    assert_eq!(vol.read_file(id).unwrap(), content);

    // Remount from the same device: the committed state must be identical.
    let dev = vol.into_device();
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_eq!(vol.lookup_root("hello.txt"), Some(id));
    let record = vol.stat(id).unwrap().unwrap();
    assert_eq!(record.object_type, ObjectType::File);
    assert_eq!(record.link_count, 1);
    assert_eq!(record.size_bytes, content.len() as u64);
    assert_eq!(record.data_blocks, 2);
    assert_eq!(record.modified.seconds, 1_780_000_100);
    assert_eq!(vol.read_file(id).unwrap(), content);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn delete_retires_storage_and_checker_stays_clean() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();
    let mut vol = mount(dev).unwrap();

    let id = vol.create_file_in_root("victim.txt", &[0xA5u8; 4000], ts(1)).unwrap();
    let data_lba = vol.stat(id).unwrap().unwrap().data_root;
    vol.delete_file_in_root("victim.txt", ts(2)).unwrap();

    assert_eq!(vol.lookup_root("victim.txt"), None);
    assert!(vol.stat(id).unwrap().is_none());
    assert!(vol.retired().contains(data_lba), "deleted data must be quarantined, not freed");

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let vol = mount(dev).unwrap();
    assert_eq!(vol.lookup_root("victim.txt"), None);
}

#[test]
fn deleting_a_missing_name_fails_cleanly() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();
    let mut vol = mount(dev).unwrap();
    assert!(matches!(vol.delete_file_in_root("ghost", ts(0)), Err(CoreError::NotFound)));
    assert_eq!(vol.generation(), 1);
}

#[test]
fn several_transactions_alternate_checkpoint_slots_and_recycle_space() {
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(&mut dev, &params(256)).unwrap();
    let mut vol = mount(dev).unwrap();
    for i in 0..10 {
        vol.create_file_in_root(&format!("file-{i}.txt"), b"data", ts(1_780_000_000 + i)).unwrap();
    }
    assert_eq!(vol.generation(), 11);
    assert_eq!(vol.list_root().len(), 10);

    let vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.generation(), 11);
    assert_eq!(vol.list_root().len(), 10);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn mount_reads_are_bounded_and_descendants_are_loaded_on_demand() {
    let mut dev = MemoryBackend::new(BS, 256);
    mkfs(&mut dev, &params(256)).unwrap();
    let mut vol = mount(dev).unwrap();
    let mut first_id = 0;
    for i in 0..10 {
        let id = vol
            .create_file_in_root(&format!("file-{i}.txt"), b"data", ts(i))
            .unwrap();
        if i == 0 {
            first_id = id;
        }
    }

    let traced = TraceBackend::new(vol.into_device());
    let mut vol = mount(traced).unwrap();
    let after_mount = vol.device_mut().stats();

    // ident + 2 checkpoints + object map + root record + root directory +
    // retired list. Object count and allocation-region pages do not add
    // reads to ordinary mount.
    assert_eq!(after_mount.reads, 7);
    assert_eq!(vol.allocator_ram_bytes(), 0);
    assert_eq!(vol.list_root().len(), 10);
    assert!(vol.free_blocks() > 0);
    assert_eq!(vol.device_mut().stats().reads, after_mount.reads);

    // One stat descends the one-level object-map tree, then decodes exactly
    // the requested descendant record. Tree height, not object count, bounds
    // the additional reads.
    assert!(vol.stat(first_id).unwrap().is_some());
    assert_eq!(vol.device_mut().stats().reads, after_mount.reads + 2);
}

#[test]
fn one_gib_region_mount_is_bounded_and_small_commit_dirties_one_page() {
    // Sparse backend: exercises the full proposed region geometry without
    // allocating a 1 GiB host buffer.
    let mut dev = MemoryBackend::new(BS, 262_144);
    mkfs(&mut dev, &params(262_144)).unwrap();
    let traced = TraceBackend::new(dev);
    let mut vol = mount(traced).unwrap();
    assert_eq!(vol.ident().geometry().bitmap_page_count(0), 9);
    assert_eq!(vol.device_mut().stats().reads, 6);
    assert_eq!(vol.allocator_ram_bytes(), 0);

    vol.device_mut().reset();
    vol.create_file_in_root("small", b"x", ts(1)).unwrap();
    let stats = vol.last_commit_stats().unwrap();
    assert_eq!(stats.bitmap_pages_written, 1);
    assert_eq!(stats.region_descriptors_written, 1);
    assert!(stats.alloc.allocator_ram_bytes <= 4096);
}

#[test]
fn duplicate_and_invalid_names_are_rejected_without_state_change() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("hello.txt", b"", ts(0)).unwrap();

    assert!(matches!(
        vol.create_file_in_root("hello.txt", b"", ts(1)),
        Err(CoreError::AlreadyExists)
    ));
    assert!(matches!(vol.create_file_in_root("a/b", b"", ts(1)), Err(CoreError::InvalidName(_))));
    assert!(matches!(vol.create_file_in_root("", b"", ts(1)), Err(CoreError::InvalidName(_))));
    assert_eq!(vol.generation(), 2, "failed operations must not advance the committed state");
}

#[test]
fn out_of_space_is_reported_and_state_survives() {
    // 25 blocks in two tiny regions: 15 reserved + 3 mkfs metadata leaves 7 free. An empty-file
    // transaction needs 5 fresh blocks; quarantine recycling keeps two
    // transactions viable, the third must fail cleanly.
    let mut dev = MemoryBackend::new(BS, 25);
    mkfs(&mut dev, &params(16)).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("first.txt", b"", ts(0)).unwrap();
    vol.create_file_in_root("second.txt", b"", ts(1)).unwrap();
    assert!(matches!(
        vol.create_file_in_root("third.txt", b"", ts(2)),
        Err(CoreError::NoSpace)
    ));

    let vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.generation(), 3);
    assert_eq!(vol.list_root().len(), 2);
}

#[test]
fn transaction_io_accounting() {
    // Write amplification is tracked from the first prototype onward. The
    // exact numbers are prototype characteristics, not format guarantees, but
    // a silent regression here is exactly what the accounting must catch.
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();
    let mut vol = mount(TraceBackend::new(dev)).unwrap();
    let mount_stats = vol.device_mut().stats();
    vol.device_mut().reset();

    vol.create_file_in_root("hello.txt", &[7u8; 4000], ts(0)).unwrap();
    let stats = vol.device_mut().stats();
    let commit = vol.last_commit_stats().unwrap();

    // 1 data block, then file record + dir + root record + object map +
    // retired list, 1 bitmap page, 1 region descriptor, 1 checkpoint. Three
    // barriers (data, metadata, commit).
    assert_eq!(commit.data_blocks_written, 1);
    assert_eq!(commit.metadata_blocks_written, 5, "unexpected metadata write amplification");
    assert_eq!(commit.bitmap_pages_written, 1);
    assert_eq!(commit.region_descriptors_written, 1);
    assert_eq!(commit.checkpoint_blocks_written, 1);
    assert_eq!(commit.flushes, 3, "data commit must use exactly three barriers");
    assert_eq!(stats.writes, 9);
    assert_eq!(stats.flushes, 3);
    assert!(stats.reads <= 12, "post-commit re-walk grew unexpectedly: {} reads", stats.reads);

    // Empty-file transaction: no data barrier.
    vol.device_mut().reset();
    vol.create_file_in_root("empty.txt", b"", ts(1)).unwrap();
    let commit = vol.last_commit_stats().unwrap();
    assert_eq!(commit.data_blocks_written, 0);
    assert_eq!(commit.flushes, 2, "empty commit must use exactly two barriers");

    // Mount: ident + 2 checkpoint slots + omap + root record + root dir.
    // Allocation pages are not read until the first mutation.
    assert_eq!(mount_stats.reads, 6);
    assert_eq!(mount_stats.writes, 0);
}

#[test]
fn file_image_end_to_end_with_json_report() {
    let dir = std::env::temp_dir().join(format!("afsplus-basic-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("volume.img");

    let mut dev = FileBackend::create(&path, BS, 128).unwrap();
    mkfs(&mut dev, &params(128)).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("on-disk.txt", b"persisted", ts(7)).unwrap();
    drop(vol);

    let mut dev = FileBackend::open(&path, BS, 128).unwrap();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let json = report.render_json();
    assert!(json.contains("\"schema_version\":2"));
    assert!(json.contains("\"clean\":true"));
    assert!(json.contains("\"generation\":2"));
    assert!(json.contains("\"region_size\":128"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn torn_newest_checkpoint_falls_back_to_previous_generation() {
    // A torn checkpoint is structurally invalid (CRC), so *selection* falls
    // back to the intact slot — this is the designed A/B mechanism, distinct
    // from the forbidden reachable-state fallback (see hardening tests).
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("hello.txt", b"x", ts(0)).unwrap();
    let mut dev = vol.into_device();

    // Generation 2 lives in slot B (LBA 2). Tear it.
    let mut torn = dev.peek(2);
    for byte in &mut torn[1000..] {
        *byte = 0xEE;
    }
    dev.apply_raw(2, &torn);

    let report = check_device(&mut dev);
    assert!(report.is_clean(), "structural fallback is an allowed state: {:?}", report.errors);
    let vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 1, "mount must fall back to the intact checkpoint");
    assert!(vol.list_root().is_empty());
}
