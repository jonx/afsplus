//! Doing a thing and undoing it must give the volume back.
//!
//! Every operation that allocates has a matching one that releases. If any
//! pair does not balance, a volume shrinks under ordinary use until it is
//! full, and its owner never finds out why: nothing reports it, the checker
//! calls the image clean, and the blocks belong to nobody reachable.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, AttributeWriteMode, MkfsParams, MountOptions};
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
            uuid: [0x1B; 16],
            label: "Balance".into(),
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

/// Free blocks once the volume has been given every chance to settle.
fn settled(vfs: &mut Vfs<MemoryBackend>) -> u64 {
    for round in 0..64 {
        let _ = vfs.cleanup_orphans(8, at(900 + round));
        let before = vfs.statfs().free_blocks;
        let _ = vfs.reclaim_space(8, at(900 + round));
        if vfs.statfs().free_blocks == before && vfs.reclaim_pending_blocks() == 0 {
            break;
        }
    }
    vfs.statfs().free_blocks
}

/// Run `body` between two settled measurements and return what it cost.
fn cost(name: &str, body: impl FnOnce(&mut Vfs<MemoryBackend>)) -> i64 {
    let mut vfs = mounted();
    let before = settled(&mut vfs);
    body(&mut vfs);
    let after = settled(&mut vfs);
    let lost = before as i64 - after as i64;
    eprintln!("{lost:>6}  {name}");
    lost
}

/// One activity a person does, paired with the undo that must give the volume
/// back: the name a reader sees, and the thing to run.
type Pair = (&'static str, Box<dyn Fn(&mut Vfs<MemoryBackend>)>);

fn write_file(vfs: &mut Vfs<MemoryBackend>, name: &str, bytes: usize) {
    let id = vfs.create_file(OBJECT_ROOT, name, at(1)).unwrap();
    let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
    vfs.write(handle, 0, &vec![5u8; bytes], at(2)).unwrap();
    vfs.close(handle).unwrap();
}

#[test]
fn every_pair_of_opposite_operations_gives_the_volume_back() {
    eprintln!(" blocks  operation and its undo");
    let mut worst = 0;
    let mut offenders = Vec::new();
    let rows: Vec<Pair> = vec![
        (
            "make an empty file, delete it",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                vfs.create_file(OBJECT_ROOT, "f", at(1)).unwrap();
                vfs.unlink_file(OBJECT_ROOT, "f", at(3)).unwrap();
            }),
        ),
        (
            "write a 1 MiB file, delete it",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                write_file(vfs, "big", 1 << 20);
                vfs.unlink_file(OBJECT_ROOT, "big", at(3)).unwrap();
            }),
        ),
        (
            "write a 4 MiB file, delete it",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                write_file(vfs, "bigger", 4 << 20);
                vfs.unlink_file(OBJECT_ROOT, "bigger", at(3)).unwrap();
            }),
        ),
        (
            "make a directory, remove it",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                vfs.create_directory(OBJECT_ROOT, "d", at(1)).unwrap();
                vfs.remove_directory(OBJECT_ROOT, "d", at(3)).unwrap();
            }),
        ),
        (
            "write a file, truncate it to nothing, delete it",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                let id = vfs.create_file(OBJECT_ROOT, "t", at(1)).unwrap();
                let handle = vfs.open_file(id, AccessMode::ReadWrite).unwrap();
                vfs.write(handle, 0, &vec![7u8; 1 << 20], at(2)).unwrap();
                vfs.truncate(handle, 0, at(3)).unwrap();
                vfs.close(handle).unwrap();
                vfs.unlink_file(OBJECT_ROOT, "t", at(4)).unwrap();
            }),
        ),
        (
            "overwrite the same file ten times, delete it",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                let id = vfs.create_file(OBJECT_ROOT, "o", at(1)).unwrap();
                let handle = vfs.open_file(id, AccessMode::ReadWrite).unwrap();
                for round in 0..10 {
                    vfs.write(handle, 0, &vec![round as u8; 256 << 10], at(2))
                        .unwrap();
                }
                vfs.close(handle).unwrap();
                vfs.unlink_file(OBJECT_ROOT, "o", at(4)).unwrap();
            }),
        ),
        (
            "make a symlink, delete it",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                vfs.create_symlink(OBJECT_ROOT, "l", "target", at(1))
                    .unwrap();
                vfs.unlink_file(OBJECT_ROOT, "l", at(3)).unwrap();
            }),
        ),
        (
            "hard link a file, unlink both",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                write_file(vfs, "h", 64 << 10);
                let id = vfs.lookup(OBJECT_ROOT, "h").unwrap();
                vfs.link_file(id, OBJECT_ROOT, "h2", at(2)).unwrap();
                vfs.unlink_file(OBJECT_ROOT, "h2", at(3)).unwrap();
                vfs.unlink_file(OBJECT_ROOT, "h", at(4)).unwrap();
            }),
        ),
        (
            "set an extended attribute, remove it, delete the file",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                let id = vfs.create_file(OBJECT_ROOT, "x", at(1)).unwrap();
                vfs.set_attributes(
                    id,
                    &[("user.k", Some(&b"value"[..]))],
                    AttributeWriteMode::Upsert,
                    at(2),
                )
                .unwrap();
                vfs.set_attributes(id, &[("user.k", None)], AttributeWriteMode::Upsert, at(3))
                    .unwrap();
                vfs.unlink_file(OBJECT_ROOT, "x", at(4)).unwrap();
            }),
        ),
        (
            "rename a file over another, delete the survivor",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                write_file(vfs, "a", 64 << 10);
                write_file(vfs, "b", 64 << 10);
                vfs.rename(OBJECT_ROOT, "a", OBJECT_ROOT, "b", true, at(3))
                    .unwrap();
                vfs.unlink_file(OBJECT_ROOT, "b", at(4)).unwrap();
            }),
        ),
        (
            "make two hundred files, delete them all",
            Box::new(|vfs: &mut Vfs<MemoryBackend>| {
                for index in 0..200 {
                    vfs.create_file(OBJECT_ROOT, &format!("m{index}"), at(1))
                        .unwrap();
                }
                for index in 0..200 {
                    vfs.unlink_file(OBJECT_ROOT, &format!("m{index}"), at(3))
                        .unwrap();
                }
            }),
        ),
    ];

    for (name, body) in rows {
        let lost = cost(name, |vfs| body(vfs));
        if lost > 4 {
            offenders.push(format!("{name}: {lost} blocks"));
        }
        worst = worst.max(lost);
    }

    assert!(
        offenders.is_empty(),
        "these pairs do not give the volume back:\n  {}",
        offenders.join("\n  ")
    );
    let _ = worst;
}

#[test]
fn how_long_the_space_of_a_big_delete_takes_to_come_back() {
    // The battery measures free space right after a delete and a sync, and saw
    // 984K still missing after a 20 MiB file. This says whether that is lost or
    // merely not yet drained: the maintenance an unlink drives is bounded on
    // purpose, so a large file may need more than one operation's worth.
    let mut vfs = mounted();
    let empty = settled(&mut vfs);

    write_file(&mut vfs, "twenty", 20 << 20);
    vfs.unlink_file(OBJECT_ROOT, "twenty", at(3)).unwrap();
    let after_delete = vfs.statfs().free_blocks;
    eprintln!("missing right after the delete   {}", empty - after_delete);

    vfs.sync_filesystem().unwrap();
    let after_sync = vfs.statfs().free_blocks;
    eprintln!("missing after one sync           {}", empty - after_sync);

    for round in 0..8 {
        vfs.sync_filesystem().unwrap();
        eprintln!(
            "missing after {} syncs            {}",
            round + 2,
            empty - vfs.statfs().free_blocks
        );
    }
    let settled_again = settled(&mut vfs);
    eprintln!("missing once fully settled       {}", empty - settled_again);
    assert!(
        empty - settled_again <= 4,
        "it must all come back in the end"
    );
}
