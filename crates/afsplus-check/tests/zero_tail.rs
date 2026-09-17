//! Every metadata block any encoder of this filesystem writes ends where its
//! payload ends: the bytes after it are zero. The survey builds volumes that
//! hold every block kind and looks at each block that carries a known magic
//! and a valid checksum.
use std::collections::BTreeMap;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::SnapshotWorkLimits;
use afsplus_core::{
    mkfs, mkfs_with_snapshots_and_security_descriptors, mount, mount_with_snapshot_limits,
    AttributeWriteMode, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::header::{BlockHeader, HEADER_SIZE};
use afsplus_format::reclaim::ReclaimCaps;
use afsplus_format::{Timespec, OBJECT_ROOT};

const BLOCK: usize = 4096;

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 0,
    }
}

fn params(log_slots: u16, caps: ReclaimCaps) -> MkfsParams {
    MkfsParams {
        uuid: [0x7a; 16],
        label: "ZeroTail".into(),
        region_size: 512,
        reclaim_caps: caps,
        log_slots,
        shared_extents: true,
        data_policy: true,
        name_policy: NamePolicy::Sensitive,
        timestamp: time(1),
    }
}

/// Per magic: blocks seen with a valid header, and those with a nonzero tail.
type Survey = BTreeMap<String, (u64, Vec<u64>)>;

fn survey(dev: &mut MemoryBackend, into: &mut Survey) {
    let mut block = vec![0u8; BLOCK];
    for lba in 0..dev.total_blocks() {
        dev.read_block(lba, &mut block).unwrap();
        if &block[0..3] != b"AFS" {
            continue;
        }
        let magic = u32::from_le_bytes(block[0..4].try_into().unwrap());
        // A valid checksum under its own magic: a block some encoder sealed.
        let Ok(header) = BlockHeader::verify(&block, magic) else {
            continue;
        };
        let entry = into
            .entry(String::from_utf8_lossy(&block[0..4]).into_owned())
            .or_default();
        entry.0 += 1;
        if block[HEADER_SIZE + header.payload_len as usize..]
            .iter()
            .any(|byte| *byte != 0)
        {
            entry.1.push(lba);
        }
    }
}

#[test]
fn every_block_the_encoders_write_has_a_zero_tail() {
    let mut found = Survey::new();

    // A plain volume with tiny reclaim areas, so sealed segments and tables
    // exist, with an intent log holding records, a big directory, a sparse
    // file, a clone, a symlink, comments, attributes and an orphan.
    let tiny = ReclaimCaps {
        inline_entries: 4,
        segment_refs: 3,
        table_refs: 8,
    };
    let mut dev = MemoryBackend::new(BLOCK, 2048);
    mkfs(&mut dev, &params(8, tiny)).unwrap();
    let mut volume = mount(dev).unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(2))
        .unwrap();
    for index in 0..300 {
        volume
            .create_file_in_directory(
                dir,
                &format!("entry-with-a-long-name-{index:04}"),
                b"x",
                time(2),
            )
            .unwrap();
    }
    let sparse = volume
        .create_file_in_directory(OBJECT_ROOT, "sparse", &[1u8; 5000], time(2))
        .unwrap();
    volume
        .write_file_at(sparse, 60 * BLOCK as u64, &[2u8; 9000], time(2))
        .unwrap();
    volume
        .clone_file(sparse, OBJECT_ROOT, "clone", time(3))
        .unwrap();
    volume
        .create_symlink(OBJECT_ROOT, "link", "dir/entry", time(3))
        .unwrap();
    volume
        .set_object_comment(sparse, "a comment", time(3))
        .unwrap();
    volume
        .set_attributes(
            sparse,
            &[("user.big", Some(&[3u8; 9000])), ("user.a", Some(b"1"))],
            AttributeWriteMode::Create,
            time(3),
        )
        .unwrap();
    for index in 0..120 {
        volume
            .delete_file(dir, &format!("entry-with-a-long-name-{index:04}"), time(4))
            .unwrap();
    }
    volume.orphan_file(OBJECT_ROOT, "clone", time(5)).unwrap();
    // Window work leaves intent-log records behind.
    volume
        .window_write_file_at(sparse, 0, &[4u8; 3000], time(6))
        .unwrap();
    volume.window_fsync().unwrap();
    let mut dev = volume.into_device();
    survey(&mut dev, &mut found);

    // A snapshot volume with descriptors: registry and lifetime trees, chain
    // segments, a retained view.
    let mut dev = MemoryBackend::new(BLOCK, 2048);
    mkfs_with_snapshots_and_security_descriptors(&mut dev, &params(0, ReclaimCaps::default()))
        .unwrap();
    let limits = SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 8,
    };
    let mut volume = mount_with_snapshot_limits(dev, MountOptions::default(), limits).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &[5u8; 20_000], time(2))
        .unwrap();
    volume
        .set_security_descriptor(file, 0x7fff_0001, 1, &[6u8; 9000], time(3))
        .unwrap();
    volume.snapshot_create(time(4)).unwrap();
    volume
        .write_file_at(file, 0, &[7u8; 20_000], time(5))
        .unwrap();
    volume.snapshot_create(time(6)).unwrap();
    volume.delete_file(OBJECT_ROOT, "file", time(7)).unwrap();
    let mut dev = volume.into_device();
    survey(&mut dev, &mut found);

    // A small volume with almost no reclamation: the queue grows into
    // sealed tables.
    let mut dev = MemoryBackend::new(BLOCK, 256);
    mkfs(
        &mut dev,
        &MkfsParams {
            region_size: 256,
            ..params(0, tiny)
        },
    )
    .unwrap();
    let mut volume = mount(dev).unwrap();
    volume.set_reclaim_batch_blocks(1);
    for index in 0..24 {
        volume
            .create_file_in_directory(OBJECT_ROOT, &format!("f{index}"), &[9u8; 100], time(2))
            .unwrap();
    }
    for index in 0..24 {
        volume
            .delete_file(OBJECT_ROOT, &format!("f{index}"), time(3))
            .unwrap();
    }
    let mut dev = volume.into_device();
    survey(&mut dev, &mut found);

    for (magic, (blocks, dirty)) in &found {
        eprintln!(
            "{magic}: {blocks} block(s), {} with a nonzero tail",
            dirty.len()
        );
    }
    let expected = [
        "AFSA", "AFSB", "AFSC", "AFSG", "AFSH", "AFSI", "AFSJ", "AFSL", "AFSO", "AFSS", "AFST",
        "AFSX",
    ];
    for magic in expected {
        assert!(found.contains_key(magic), "the survey saw no {magic} block");
    }
    let dirty: Vec<_> = found
        .iter()
        .filter(|(_, (_, dirty))| !dirty.is_empty())
        .collect();
    assert!(dirty.is_empty(), "{dirty:?}");
}

/// The rule lives in the header verification of both readers, so it holds
/// for kinds that never had a tail test of their own. On a real volume, one
/// byte after the payload of each block on the lookup path, resealed with a
/// valid checksum, is refused by the core and by the portable C reader.
#[cfg(unix)]
#[test]
fn both_readers_refuse_a_dirty_tail_in_every_kind_on_the_lookup_path() {
    use afsplus_check::explain::{BlockRole, Explainer, VolumeTree};
    use std::process::Command;

    let scratch = std::env::temp_dir().join(format!("afsplus-zero-tail-{}", std::process::id()));
    std::fs::create_dir(&scratch).unwrap();
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let probe = scratch.join("probe");
    let output = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()))
        .args(["-std=c99", "-pedantic", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(repo.join("api"))
        .arg("-I")
        .arg(repo.join("spec"))
        .arg(repo.join("portable/c/reader.c"))
        .arg(repo.join("portable/c/tests/security_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut dev = MemoryBackend::new(BLOCK, 1024);
    mkfs(&mut dev, &params(0, ReclaimCaps::default())).unwrap();
    let mut volume = mount(dev).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &[1u8; 100], time(2))
        .unwrap();
    let mut dev = volume.into_device();

    // Where the lookup of `file` goes: identification, both checkpoint
    // slots, the object-map node, the object record.
    let explainer = Explainer::load(&mut dev).unwrap();
    let mut targets: Vec<(String, Vec<u64>, &str)> = vec![
        ("identification".into(), vec![0], "unmountable"),
        ("both checkpoint slots".into(), vec![1, 2], "unmountable"),
    ];
    for lba in 0..dev.total_blocks() {
        for role in explainer.explain_block(&mut dev, lba).unwrap().roles {
            match role {
                BlockRole::VolumeTreeNode {
                    kind: VolumeTree::ObjectMap,
                    ..
                } => targets.push(("object-map node".into(), vec![lba], "corrupt")),
                BlockRole::ObjectRecord { object_id } if object_id == file => {
                    targets.push(("object record".into(), vec![lba], "corrupt"))
                }
                _ => {}
            }
        }
    }
    assert_eq!(targets.len(), 4);

    let image_of = |dev: &mut MemoryBackend| {
        let mut image = vec![0u8; dev.total_blocks() as usize * BLOCK];
        for lba in 0..dev.total_blocks() {
            dev.read_block(lba, &mut image[lba as usize * BLOCK..][..BLOCK])
                .unwrap();
        }
        image
    };
    let c_verdict = |image: &[u8], expectation: &str| {
        let path = scratch.join("image");
        std::fs::write(&path, image).unwrap();
        Command::new(&probe)
            .arg("lookup")
            .arg(&path)
            .arg(file.to_string())
            .arg(expectation)
            .status()
            .unwrap()
            .code()
    };
    // The clean image answers in both readers.
    assert!(mount(dev.clone()).unwrap().stat(file).unwrap().is_some());
    assert_eq!(
        c_verdict(&image_of(&mut dev), "corrupt"),
        Some(1),
        "the clean image is not corrupt"
    );

    let mut block = vec![0u8; BLOCK];
    for (what, lbas, expectation) in targets {
        let mut dirty = dev.clone();
        for lba in &lbas {
            dirty.read_block(*lba, &mut block).unwrap();
            let magic = u32::from_le_bytes(block[0..4].try_into().unwrap());
            let header = BlockHeader::verify(&block, magic).unwrap();
            assert!(
                (header.payload_len as usize) < BLOCK - HEADER_SIZE,
                "{what}"
            );
            block[BLOCK - 1] = 1;
            header.seal(&mut block);
            assert!(BlockHeader::checksum_matches(&block));
            dirty.write_block(*lba, &block).unwrap();
        }
        let rust = mount(dirty.clone()).and_then(|mut volume| volume.stat(file));
        assert!(rust.is_err(), "Rust admitted a dirty tail in the {what}");
        // ADR-112 keeps the diagnostic apart from admission: such a block
        // HAS a valid checksum and explain says so, while no reader admits
        // it. A reader that conflated the two would report a torn write.
        // A damaged identification or checkpoint leaves no committed state
        // to walk, so there is no explanation to ask for.
        if let Ok(explainer) = Explainer::load(&mut dirty) {
            for lba in &lbas {
                let identity = explainer
                    .explain_block(&mut dirty, *lba)
                    .unwrap()
                    .identity
                    .unwrap_or_else(|| panic!("{what}: no identity for a resealed block"));
                assert!(
                    identity.checksum_valid,
                    "{what}: explain calls a resealed block's checksum invalid"
                );
            }
        }
        assert_eq!(
            c_verdict(&image_of(&mut dirty), expectation),
            Some(0),
            "C verdict for a dirty tail in the {what}"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
