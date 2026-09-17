//! A volume that runs out of space has to stay usable.
//!
//! Filling one through a mount left the write window open: every later
//! operation that needs it closed answered Busy, so the person could not even
//! delete the file that filled the disk. The mount process then sat in
//! uninterruptible wait, `diskutil` blocked on it machine-wide, and only a
//! reboot cleared it.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Vfs, VfsError};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn small() -> Vfs<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 2048);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xF0; 16],
            label: "Full".into(),
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

/// Write until the volume refuses, and say what it refused with.
fn fill(vfs: &mut Vfs<MemoryBackend>, name: &str) -> VfsError {
    let id = vfs.create_file(OBJECT_ROOT, name, at(1)).unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    let chunk = vec![9u8; 64 * 1024];
    let mut offset = 0u64;
    for _ in 0..4096 {
        match vfs.write(handle, offset, &chunk, at(2)) {
            Ok(written) => offset += written as u64,
            Err(error) => {
                let _ = vfs.close(handle);
                return error;
            }
        }
    }
    panic!("the volume accepted more than it holds");
}

#[test]
fn a_full_volume_still_lets_a_person_delete_something() {
    let mut vfs = small();
    let refusal = fill(&mut vfs, "fill.bin");
    eprintln!("the volume refused the write with {refusal:?}");

    // This is the whole point: the disk is full, and the one thing a person
    // does about a full disk must work.
    vfs.unlink_file(OBJECT_ROOT, "fill.bin", at(3))
        .expect("deleting the file that filled the volume must work");
}

#[test]
fn a_full_volume_still_answers_a_listing() {
    let mut vfs = small();
    fill(&mut vfs, "fill.bin");
    let handle = vfs
        .open_directory(OBJECT_ROOT)
        .expect("a full volume must still open its root");
    vfs.read_directory(handle, 0, 16)
        .expect("a full volume must still be readable");
    vfs.close(handle).unwrap();
}

#[test]
fn space_returned_after_a_full_volume_can_be_used_again() {
    let mut vfs = small();
    fill(&mut vfs, "fill.bin");
    vfs.unlink_file(OBJECT_ROOT, "fill.bin", at(3)).unwrap();

    let id = vfs
        .create_file(OBJECT_ROOT, "after.txt", at(4))
        .expect("a file must be creatable again once the disk is emptied");
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, b"recovered", at(5))
        .expect("and writable");
    vfs.close(handle).unwrap();
}

#[test]
fn the_volume_says_what_it_lost_rather_than_answering_busy_for_ever() {
    let mut vfs = small();
    fill(&mut vfs, "fill.bin");

    // The first operation after the failure is the one that discovers it, and
    // it must name the loss. "Busy" on a volume with most of its blocks free
    // tells a person nothing and used to be the answer to everything, for ever.
    match vfs.sync_filesystem() {
        Err(VfsError::WindowLost(lost)) => {
            assert!(lost > 0, "a loss of nothing is not a loss");
        }
        Ok(()) => {}
        Err(other) => panic!("expected the loss to be named, got {other:?}"),
    }

    // And afterwards the volume is ordinary again.
    vfs.unlink_file(OBJECT_ROOT, "fill.bin", at(9))
        .expect("the volume must be usable once the failed window is abandoned");
    assert!(
        vfs.statfs().free_blocks > 0,
        "the volume must report its free space"
    );
}
