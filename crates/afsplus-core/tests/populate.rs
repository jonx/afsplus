//! `afsplus-populate` carries a host drawer onto a volume as it is: a file
//! larger than one extent, in chunks; a hole stays a hole; a symlink is a
//! symlink; a hard link is one object with two names.

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("afsplus-populate-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn formatted(path: &Path, blocks: u64) {
    fs::File::create(path).unwrap();
    let mut device = FileBackend::open(path, DEFAULT_BLOCK_SIZE, blocks).unwrap();
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x5A; 16],
            label: "Populate".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: false,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: Timespec::default(),
        },
    )
    .unwrap();
    device.flush().unwrap();
}

#[test]
fn a_sparse_file_a_symlink_and_a_hard_link_come_across_as_they_are() {
    let dir = scratch("drawer");
    let drawer = dir.join("drawer");
    fs::create_dir_all(&drawer).unwrap();
    // 40 MiB: three extents' worth at the 16 MiB cap, and 39 MiB of hole.
    let mut big = fs::File::create(drawer.join("big.bin")).unwrap();
    big.write_all(b"head").unwrap();
    big.seek(SeekFrom::Start(40 * 1024 * 1024)).unwrap();
    big.write_all(b"tail").unwrap();
    drop(big);
    fs::write(drawer.join("target.txt"), b"target\n").unwrap();
    std::os::unix::fs::symlink("target.txt", drawer.join("soft")).unwrap();
    fs::hard_link(drawer.join("target.txt"), drawer.join("hard")).unwrap();

    let image = dir.join("volume.afsp");
    formatted(&image, 64 * 256); // 64 MiB
    let status = Command::new(env!("CARGO_BIN_EXE_afsplus-populate"))
        .arg(&image)
        .arg(&drawer)
        .status()
        .unwrap();
    assert!(status.success(), "populate must accept the drawer");

    let mut device = FileBackend::open_sized_by_file(&image, DEFAULT_BLOCK_SIZE).unwrap();
    device.set_total_blocks(64 * 256);
    let mut volume = mount(device).unwrap();

    // The big file has its length and its two ends, and cost far less than
    // its length: the hole was never written.
    let big = volume.lookup_root("big.bin").unwrap().expect("big.bin");
    let record = volume.stat(big).unwrap().unwrap();
    assert_eq!(record.size_bytes, 40 * 1024 * 1024 + 4);
    let mut head = [0u8; 4];
    volume.read_file_at(big, 0, &mut head).unwrap();
    assert_eq!(&head, b"head");
    let mut tail = [0u8; 4];
    volume
        .read_file_at(big, 40 * 1024 * 1024, &mut tail)
        .unwrap();
    assert_eq!(&tail, b"tail");
    let used = 64 * 256 - volume.free_blocks();
    assert!(
        used < 2048,
        "a 40 MiB file with two written bytes used {used} blocks"
    );

    // The symlink is a symlink with its target.
    let soft = volume.lookup_root("soft").unwrap().expect("soft");
    let mut target = [0u8; 64];
    let n = volume.read_link(soft, &mut target).unwrap();
    assert_eq!(&target[..n], b"target.txt");

    // The hard link is the same object under two names.
    let a = volume
        .lookup_root("target.txt")
        .unwrap()
        .expect("target.txt");
    let b = volume.lookup_root("hard").unwrap().expect("hard");
    assert_eq!(a, b, "a hard link must not become a copy");
    assert_eq!(volume.stat(a).unwrap().unwrap().link_count, 2);
    let _ = OBJECT_ROOT;
    fs::remove_dir_all(&dir).unwrap();
}

/// Control: without the chunked path the same drawer is refused. The old
/// tool read a file whole and asked for one extent; 40 MiB exceeds the cap.
#[test]
fn a_file_over_the_extent_cap_needs_the_chunked_path() {
    let dir = scratch("cap");
    let image = dir.join("volume.afsp");
    formatted(&image, 64 * 256);
    let mut device = FileBackend::open_sized_by_file(&image, DEFAULT_BLOCK_SIZE).unwrap();
    device.set_total_blocks(64 * 256);
    let mut volume = mount(device).unwrap();
    let content = vec![1u8; 17 * 1024 * 1024];
    let refused = volume
        .create_file_in_directory(OBJECT_ROOT, "whole", &content, Timespec::default())
        .is_err();
    assert!(
        refused,
        "one extent of 17 MiB must still be refused by the direct path"
    );
    let id = volume
        .create_file_in_directory(OBJECT_ROOT, "chunked", &[], Timespec::default())
        .unwrap();
    for (i, chunk) in content.chunks(1 << 20).enumerate() {
        volume
            .write_file_at(id, (i as u64) << 20, chunk, Timespec::default())
            .unwrap();
    }
    assert_eq!(
        volume.stat(id).unwrap().unwrap().size_bytes,
        17 * 1024 * 1024
    );
    fs::remove_dir_all(&dir).unwrap();
}
