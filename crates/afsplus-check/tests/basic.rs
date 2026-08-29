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
    assert_ne!(vol.checkpoint().allocation_root_block, 0);
    assert!(vol.checkpoint().regions.is_empty());
    assert!(vol.list_root().unwrap().is_empty());
    assert_eq!(vol.ident().label, "TestVol");
    let root = vol.stat(afsplus_format::OBJECT_ROOT).unwrap().unwrap();
    assert_eq!(root.object_type, ObjectType::Directory);
    assert_eq!(root.link_count, 1);
    // 64 blocks minus 9 reserved (bootstrap + descriptor + bitmap slots),
    // 3 initial namespace metadata blocks, and the 3-block triple-version
    // allocation-root pool.
    assert_eq!(vol.free_blocks(), 49);
}

#[test]
fn create_with_content_commit_remount_read_back() {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(&mut dev, &params(64)).unwrap();
    let mut vol = mount(dev).unwrap();

    let content = b"bonjour AFS+\n".repeat(400); // ~5 KiB -> 2 data blocks
    let id = vol.create_file_in_root("hello.txt", &content, ts(1_780_000_100)).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_ne!(vol.checkpoint().allocation_root_block, 0);
    assert!(vol.checkpoint().regions.is_empty());
    assert_eq!(vol.lookup_root("hello.txt").unwrap(), Some(id));
    assert_eq!(vol.read_file(id).unwrap(), content);

    // Remount from the same device: the committed state must be identical.
    let dev = vol.into_device();
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 2);
    assert_eq!(vol.lookup_root("hello.txt").unwrap(), Some(id));
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

    assert_eq!(vol.lookup_root("victim.txt").unwrap(), None);
    assert!(vol.stat(id).unwrap().is_none());
    assert!(vol.retired().contains(data_lba), "deleted data must be quarantined, not freed");

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.lookup_root("victim.txt").unwrap(), None);
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
    assert_eq!(vol.list_root().unwrap().len(), 10);

    let mut vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.generation(), 11);
    assert_eq!(vol.list_root().unwrap().len(), 10);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn root_directory_grows_beyond_the_legacy_single_block_limit() {
    let mut dev = MemoryBackend::new(BS, 4096);
    mkfs(&mut dev, &params(4096)).unwrap();
    let mut vol = mount(dev).unwrap();

    for i in 0..300 {
        vol.create_file_in_root(&format!("entry-{i:04}.txt"), b"", ts(i))
            .unwrap();
    }

    assert_eq!(vol.list_root().unwrap().len(), 300);
    assert!(vol.lookup_root("entry-0000.txt").unwrap().is_some());
    assert!(vol.lookup_root("entry-0299.txt").unwrap().is_some());

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);

    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.list_root().unwrap().len(), 300);
    assert!(vol.lookup_root("entry-0173.txt").unwrap().is_some());
}

#[test]
fn nested_directories_are_addressed_by_stable_object_id() {
    let mut dev = MemoryBackend::new(BS, 512);
    mkfs(&mut dev, &params(512)).unwrap();
    let mut vol = mount(dev).unwrap();

    let projects = vol.create_directory_in_root("Projects", ts(1)).unwrap();
    let afsplus = vol.create_directory(projects, "afsplus", ts(2)).unwrap();
    let readme = vol
        .create_file_in_directory(afsplus, "README.md", b"portable core\n", ts(3))
        .unwrap();

    assert_eq!(vol.lookup_root("Projects").unwrap(), Some(projects));
    assert_eq!(
        vol.lookup_in_directory(projects, "afsplus").unwrap(),
        Some(afsplus)
    );
    assert_eq!(
        vol.lookup_in_directory(afsplus, "README.md").unwrap(),
        Some(readme)
    );
    assert_eq!(vol.read_file(readme).unwrap(), b"portable core\n");
    assert!(matches!(
        vol.lookup_in_directory(readme, "not-a-directory"),
        Err(CoreError::NotDirectory)
    ));
    assert!(matches!(
        vol.remove_directory(afsplus_format::OBJECT_ROOT, "Projects", ts(4)),
        Err(CoreError::DirectoryNotEmpty)
    ));
    assert!(matches!(
        vol.remove_directory(afsplus, "README.md", ts(4)),
        Err(CoreError::NotDirectory)
    ));

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.list_directory(projects).unwrap().len(), 1);
    assert_eq!(vol.list_directory(afsplus).unwrap().len(), 1);
    assert_eq!(vol.read_file(readme).unwrap(), b"portable core\n");

    vol.delete_file(afsplus, "README.md", ts(5)).unwrap();
    vol.remove_directory(projects, "afsplus", ts(6)).unwrap();
    vol.remove_directory(afsplus_format::OBJECT_ROOT, "Projects", ts(7))
        .unwrap();
    assert!(vol.list_root().unwrap().is_empty());
    assert!(matches!(vol.list_directory(afsplus), Err(CoreError::NotFound)));

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
}

#[test]
fn rename_and_cross_directory_move_preserve_identity_and_reject_cycles() {
    let mut dev = MemoryBackend::new(BS, 512);
    mkfs(&mut dev, &params(512)).unwrap();
    let mut vol = mount(dev).unwrap();

    let left = vol.create_directory_in_root("left", ts(1)).unwrap();
    let right = vol.create_directory_in_root("right", ts(2)).unwrap();
    let nested = vol.create_directory(left, "nested", ts(3)).unwrap();
    let file = vol
        .create_file_in_directory(left, "note.txt", b"stable identity", ts(4))
        .unwrap();

    let generation = vol.generation();
    assert!(matches!(
        vol.rename(left, "missing", left, "missing", ts(5)),
        Err(CoreError::NotFound)
    ));
    assert_eq!(vol.generation(), generation);

    vol.rename(left, "note.txt", left, "renamed.txt", ts(5))
        .unwrap();
    assert_eq!(vol.lookup_in_directory(left, "note.txt").unwrap(), None);
    assert_eq!(
        vol.lookup_in_directory(left, "renamed.txt").unwrap(),
        Some(file)
    );

    vol.rename(left, "renamed.txt", right, "moved.txt", ts(6))
        .unwrap();
    vol.rename(left, "nested", right, "nested", ts(7))
        .unwrap();
    assert_eq!(vol.lookup_in_directory(right, "moved.txt").unwrap(), Some(file));
    assert_eq!(vol.lookup_in_directory(right, "nested").unwrap(), Some(nested));
    assert_eq!(vol.read_file(file).unwrap(), b"stable identity");

    let generation = vol.generation();
    assert!(matches!(
        vol.rename(
            afsplus_format::OBJECT_ROOT,
            "right",
            nested,
            "cycle",
            ts(8)
        ),
        Err(CoreError::InvalidMove(_))
    ));
    assert_eq!(vol.generation(), generation);
    assert!(matches!(
        vol.rename(
            afsplus_format::OBJECT_ROOT,
            "left",
            afsplus_format::OBJECT_ROOT,
            "right",
            ts(8)
        ),
        Err(CoreError::AlreadyExists)
    ));
    assert_eq!(vol.generation(), generation);

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.lookup_root("right").unwrap(), Some(right));
    assert_eq!(vol.lookup_in_directory(right, "moved.txt").unwrap(), Some(file));
    assert_eq!(vol.lookup_in_directory(right, "nested").unwrap(), Some(nested));
    assert_eq!(vol.read_file(file).unwrap(), b"stable identity");
}

#[test]
fn hard_links_share_one_file_object_until_the_final_unlink() {
    let mut dev = MemoryBackend::new(BS, 512);
    mkfs(&mut dev, &params(512)).unwrap();
    let mut vol = mount(dev).unwrap();

    let first = vol.create_directory_in_root("first", ts(1)).unwrap();
    let second = vol.create_directory_in_root("second", ts(2)).unwrap();
    let file = vol
        .create_file_in_directory(first, "original", b"one object", ts(3))
        .unwrap();
    vol.link_file(file, second, "alias", ts(4)).unwrap();

    assert_eq!(vol.lookup_in_directory(first, "original").unwrap(), Some(file));
    assert_eq!(vol.lookup_in_directory(second, "alias").unwrap(), Some(file));
    assert_eq!(vol.stat(file).unwrap().unwrap().link_count, 2);
    let generation = vol.generation();
    assert!(matches!(
        vol.link_file(first, second, "directory-link", ts(5)),
        Err(CoreError::IsDirectory)
    ));
    assert_eq!(vol.generation(), generation);

    vol.delete_file(first, "original", ts(5)).unwrap();
    assert_eq!(vol.lookup_in_directory(first, "original").unwrap(), None);
    assert_eq!(vol.lookup_in_directory(second, "alias").unwrap(), Some(file));
    assert_eq!(vol.stat(file).unwrap().unwrap().link_count, 1);
    assert_eq!(vol.read_file(file).unwrap(), b"one object");

    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "checker findings: {:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.lookup_in_directory(second, "alias").unwrap(), Some(file));
    assert_eq!(vol.read_file(file).unwrap(), b"one object");

    vol.delete_file(second, "alias", ts(6)).unwrap();
    assert!(vol.stat(file).unwrap().is_none());
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

    // ident + 2 checkpoints + object-map root + root record + directory root +
    // retired list. Object count and allocation-region pages do not add
    // reads to ordinary mount.
    assert_eq!(after_mount.reads, 7);
    assert_eq!(vol.allocator_ram_bytes(), 0);
    assert_eq!(vol.list_root().unwrap().len(), 10);
    assert!(vol.free_blocks() > 0);
    assert_eq!(vol.device_mut().stats().reads, after_mount.reads + 1);

    // One stat descends the one-level object-map tree, then decodes exactly
    // the requested descendant record. Tree height, not object count, bounds
    // the additional reads.
    assert!(vol.stat(first_id).unwrap().is_some());
    assert_eq!(vol.device_mut().stats().reads, after_mount.reads + 3);
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
    // 28 blocks in two tiny regions: the allocation-root pool consumes three
    // permanently reserved blocks. An empty-file transaction needs 5 fresh
    // blocks; quarantine recycling keeps two
    // transactions viable, the third must fail cleanly.
    let mut dev = MemoryBackend::new(BS, 28);
    mkfs(&mut dev, &params(16)).unwrap();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("first.txt", b"", ts(0)).unwrap();
    vol.create_file_in_root("second.txt", b"", ts(1)).unwrap();
    assert!(matches!(
        vol.create_file_in_root("third.txt", b"", ts(2)),
        Err(CoreError::NoSpace)
    ));

    let mut vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.generation(), 3);
    assert_eq!(vol.list_root().unwrap().len(), 2);
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

    // 1 data block, then file record + directory path + root record + object map +
    // allocation root + retired list, 1 bitmap page, 1 region descriptor,
    // 1 checkpoint. Three barriers (data, metadata, commit).
    assert_eq!(commit.data_blocks_written, 1);
    assert_eq!(commit.metadata_blocks_written, 6, "unexpected metadata write amplification");
    assert_eq!(commit.bitmap_pages_written, 1);
    assert_eq!(commit.region_descriptors_written, 1);
    assert_eq!(commit.checkpoint_blocks_written, 1);
    assert_eq!(commit.flushes, 3, "data commit must use exactly three barriers");
    assert_eq!(stats.writes, 10);
    assert_eq!(stats.flushes, 3);
    assert!(stats.reads <= 13, "post-commit re-walk grew unexpectedly: {} reads", stats.reads);

    // Empty-file transaction: no data barrier.
    vol.device_mut().reset();
    vol.create_file_in_root("empty.txt", b"", ts(1)).unwrap();
    let commit = vol.last_commit_stats().unwrap();
    assert_eq!(commit.data_blocks_written, 0);
    assert_eq!(commit.flushes, 2, "empty commit must use exactly two barriers");

    // Mount: ident + 2 checkpoint slots + omap + root record + directory root.
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
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), 1, "mount must fall back to the intact checkpoint");
    assert!(vol.list_root().unwrap().is_empty());
}
