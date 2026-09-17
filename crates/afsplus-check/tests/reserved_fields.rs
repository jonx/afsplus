//! Bytes that belong to no field are refused by BOTH readers (ADR-114): the
//! common-header flags of every kind, the owner of the two kinds that give it
//! no meaning, and an identification payload longer than its version's
//! layout. Each probe reseals a valid block with a valid checksum and a zero
//! tail, so [ADR-112](../../../adr/ADR-112-block-zero-tail.md) does not decide
//! the answer.
#![cfg(unix)]
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_check::explain::{BlockRole, Explainer, VolumeTree};
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::header::{BlockHeader, HEADER_SIZE};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BLOCK: usize = 4096;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 0,
    }
}

fn compile(scratch: &Scratch) -> PathBuf {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let probe = scratch.0.join("probe");
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
        .arg(repo.join("portable/c/tests/reserved_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    probe
}

/// A volume that holds one block of every kind this ADR covers.
fn populated() -> (MemoryBackend, u64) {
    let mut dev = MemoryBackend::new(BLOCK, 1024);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x5a; 16],
            label: "Reserved".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    let mut volume = mount(dev).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", &[1u8; 5000], time(2))
        .unwrap();
    // An open window leaves an intent-log record on the device.
    volume
        .window_write_file_at(file, 0, &[2u8; 200], time(3))
        .unwrap();
    volume.window_fsync().unwrap();
    (volume.into_device(), file)
}

/// One block of each kind that the live committed state actually reads:
/// the bitmap page and the region descriptor have three slots each and only
/// the bound one is read, so the role decides, not the magic.
fn one_of_each(dev: &mut MemoryBackend) -> BTreeMap<String, (u64, u32)> {
    let explainer = Explainer::load(dev).unwrap();
    assert_eq!(explainer.problems, Vec::<String>::new());
    let mut found: BTreeMap<String, (u64, u32)> = BTreeMap::new();
    let mut block = vec![0u8; BLOCK];
    for lba in 0..dev.total_blocks() {
        let magic = match explainer.explain_block(dev, lba).unwrap().roles.first() {
            Some(BlockRole::Identification) => "AFSI",
            Some(BlockRole::BitmapSlot { live: true, .. }) => "AFSB",
            Some(BlockRole::RegionDescriptorSlot { live: true, .. }) => "AFSG",
            Some(BlockRole::IntentLogSlot { .. }) => "AFSJ",
            Some(BlockRole::VolumeTreeNode {
                kind: VolumeTree::ObjectMap,
                ..
            }) => "AFST",
            _ => continue,
        };
        dev.read_block(lba, &mut block).unwrap();
        let kind = u32::from_le_bytes(block[0..4].try_into().unwrap());
        // An intent-log slot holds a record only while one is pending.
        if BlockHeader::verify(&block, kind).is_err() {
            continue;
        }
        let payload = u32::from_le_bytes(block[24..28].try_into().unwrap());
        found.entry(magic.to_owned()).or_insert((lba, payload));
    }
    found
}

fn reseal(block: &mut [u8], payload_len: Option<u32>) {
    BlockHeader {
        block_type: u32::from_le_bytes(block[0..4].try_into().unwrap()),
        flags: u16::from_le_bytes(block[6..8].try_into().unwrap()),
        owner: u64::from_le_bytes(block[8..16].try_into().unwrap()),
        generation: u64::from_le_bytes(block[16..24].try_into().unwrap()),
        payload_len: payload_len
            .unwrap_or_else(|| u32::from_le_bytes(block[24..28].try_into().unwrap())),
    }
    .seal(block);
}

#[test]
fn both_readers_refuse_bytes_that_belong_to_no_field() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-reserved-{}", std::process::id())));
    std::fs::create_dir(&scratch.0).unwrap();
    let probe = compile(&scratch);
    let (base, file) = populated();
    let mut clean = base.clone();
    let blocks = one_of_each(&mut clean);
    for magic in ["AFSI", "AFSB", "AFSG", "AFSJ", "AFST"] {
        assert!(
            blocks.contains_key(magic),
            "the volume has no {magic} block"
        );
    }

    // The C entry point that reaches each kind, and the Rust operation that
    // does. The checker reads every structure, so it is the Rust verdict.
    let c_mode = |magic: &str| match magic {
        "AFSI" => vec!["probe".to_owned()],
        "AFST" => vec!["lookup".to_owned(), file.to_string()],
        "AFSJ" => vec!["intent".to_owned()],
        _ => vec!["alloc".to_owned()],
    };
    let image_path = scratch.0.join("image");
    let c_accepts = |dev: &mut MemoryBackend, magic: &str| {
        let mut image = vec![0u8; dev.total_blocks() as usize * BLOCK];
        for lba in 0..dev.total_blocks() {
            dev.read_block(lba, &mut image[lba as usize * BLOCK..][..BLOCK])
                .unwrap();
        }
        std::fs::write(&image_path, &image).unwrap();
        let mut arguments = c_mode(magic);
        arguments.insert(1, image_path.to_str().unwrap().to_owned());
        let code = Command::new(&probe)
            .args(&arguments)
            .status()
            .unwrap()
            .code();
        assert_ne!(code, Some(2), "the probe could not run for {magic}");
        code == Some(0)
    };
    // The Rust verdict per kind, as the C entry point is per kind. A log
    // record the core refuses ends the scan before it, which is how a torn
    // tail is meant to read, so for that kind the verdict is whether the
    // record is still awaiting replay.
    let rust_accepts = |dev: &mut MemoryBackend, magic: &str| {
        let report = check_device(dev);
        if magic == "AFSJ" {
            return report
                .volume
                .as_ref()
                .is_some_and(|volume| volume.log_records_pending > 0);
        }
        report.errors.is_empty()
    };

    // Both readers accept the clean volume through every entry point.
    for magic in ["AFSI", "AFSB", "AFSG", "AFSJ", "AFST"] {
        assert!(
            rust_accepts(&mut clean, magic),
            "the core refuses the clean volume: {magic}"
        );
        assert!(
            c_accepts(&mut clean, magic),
            "C refuses the clean volume: {magic}"
        );
    }

    let mut probes: Vec<(String, &str, MemoryBackend)> = Vec::new();
    let mut block = vec![0u8; BLOCK];
    for (magic, (lba, payload)) in &blocks {
        if !["AFSI", "AFSB", "AFSG", "AFSJ", "AFST"].contains(&magic.as_str()) {
            continue;
        }
        let mut with = |label: String, edit: &dyn Fn(&mut Vec<u8>) -> Option<u32>| {
            let mut dirty = clean.clone();
            dirty.read_block(*lba, &mut block).unwrap();
            let mut copy = block.clone();
            let length = edit(&mut copy);
            reseal(&mut copy, length);
            dirty.write_block(*lba, &copy).unwrap();
            probes.push((label, magic_static(magic), dirty));
        };
        with(format!("{magic}: header flags"), &|b| {
            b[6] = 1;
            None
        });
        // The two kinds that belong to the volume write a zero owner.
        if magic == "AFSI" || magic == "AFSJ" {
            with(format!("{magic}: header owner"), &|b| {
                b[8] = 1;
                None
            });
        }
        // The identification block is the one whose payload may grow.
        if magic == "AFSI" {
            let stated = *payload as usize;
            with(format!("{magic}: payload longer than its version"), &|_b| {
                Some((stated + 8) as u32)
            });
            with(format!("{magic}: bytes past its last field"), &|b| {
                b[HEADER_SIZE + stated..HEADER_SIZE + stated + 8].fill(0xee);
                Some((stated + 8) as u32)
            });
        }
    }
    assert_eq!(probes.len(), 9);
    for (label, magic, mut dirty) in probes {
        assert!(!rust_accepts(&mut dirty, magic), "the core admits {label}");
        assert!(!c_accepts(&mut dirty, magic), "the C reader admits {label}");
    }
}

/// The probe mode table needs a `&'static str`; the magics are known.
fn magic_static(magic: &str) -> &'static str {
    match magic {
        "AFSI" => "AFSI",
        "AFSB" => "AFSB",
        "AFSG" => "AFSG",
        "AFSJ" => "AFSJ",
        _ => "AFST",
    }
}
