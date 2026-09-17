//! Space a person frees has to come back.
//!
//! Deleting a file retires its extents into the reclaim ledger; the free pool
//! grows again only when that ledger is drained. Nothing above the core drained
//! it, so a mounted volume never returned a deleted byte: writing and deleting
//! one file repeatedly took a 256 MiB volume down to nothing, and at the end
//! `rm` itself failed for want of space.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Vfs};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn mounted() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 16384);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x9D; 16],
            label: "Space".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    Vfs::mount(device, MountOptions::default()).unwrap()
}

fn write_a_megabyte(vfs: &mut Vfs<MemoryBackend>, name: &str, when: i64) {
    let id = vfs.create_file(OBJECT_ROOT, name, at(when)).unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, &vec![7u8; 1 << 20], at(when)).unwrap();
    vfs.close(handle).unwrap();
}

#[test]
fn deleting_a_file_gives_its_space_back() {
    let mut vfs = mounted();
    let empty = vfs.statfs().free_blocks;

    write_a_megabyte(&mut vfs, "big.bin", 1);
    let written = vfs.statfs().free_blocks;
    assert!(written < empty, "writing a megabyte must consume blocks");

    vfs.unlink_file(OBJECT_ROOT, "big.bin", at(2)).unwrap();
    let after = vfs.statfs().free_blocks;

    assert!(
        after >= written + 256,
        "the megabyte must come back when the file goes: {written} before, {after} after"
    );
    assert!(
        empty - after <= 8,
        "the volume must return to within a few blocks of empty, not {after} of {empty}"
    );
}

#[test]
fn writing_and_deleting_the_same_file_does_not_fill_the_volume() {
    let mut vfs = mounted();
    let empty = vfs.statfs().free_blocks;

    for round in 0..12 {
        write_a_megabyte(&mut vfs, "cycle.bin", 10 + round * 2);
        vfs.unlink_file(OBJECT_ROOT, "cycle.bin", at(11 + round * 2))
            .unwrap();
    }

    let after = vfs.statfs().free_blocks;
    assert!(
        empty - after <= 64,
        "twelve write-and-delete rounds must not eat the volume: {empty} free at the start, {after} at the end"
    );
}

#[test]
fn a_read_only_mount_reclaims_nothing_and_says_so() {
    let mut vfs = mounted();
    let before = vfs.statfs().free_blocks;
    assert_eq!(vfs.reclaim_space(4, at(1)).unwrap(), 0);
    assert_eq!(vfs.statfs().free_blocks, before);
}

#[test]
fn a_deleted_file_someone_still_holds_open_keeps_its_bytes() {
    let mut vfs = mounted();
    let id = vfs.create_file(OBJECT_ROOT, "held.bin", at(1)).unwrap();
    let writer = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.write(writer, 0, &vec![42u8; 1 << 20], at(1)).unwrap();
    vfs.close(writer).unwrap();

    let reader = vfs.open_file(id, AccessMode::ReadOnly).unwrap();
    vfs.unlink_file(OBJECT_ROOT, "held.bin", at(2)).unwrap();

    // The name is gone, the file is not: the reader must still see its bytes.
    let mut seen = vec![0u8; 4096];
    vfs.read(reader, 0, &mut seen).unwrap();
    assert!(
        seen.iter().all(|byte| *byte == 42),
        "cleaning an orphan out from under a live reader would hand it rubbish"
    );
    vfs.close(reader).unwrap();
}
