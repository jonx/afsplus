//! Volume-level cross-read: the portable C reader and the Rust core give the
//! same verdict on objects of real images, with and without the security
//! preservation container.
#![cfg(unix)]
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{
    mkfs, mkfs_with_security_descriptors, mount, CoreError, MkfsParams, NamePolicy,
};
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::object::{ObjectRecord, SecurityRef};
use afsplus_format::{Timespec, OBJECT_ROOT};
use std::{fs, path::PathBuf, process::Command};

const BLOCK: usize = 4096;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 1,
    }
}

fn params() -> MkfsParams {
    MkfsParams {
        uuid: [0xc5; 16],
        label: "SecurityC".into(),
        region_size: 512,
        reclaim_caps: Default::default(),
        log_slots: 0,
        shared_extents: false,
        data_policy: false,
        name_policy: NamePolicy::Sensitive,
        timestamp: time(1),
    }
}

fn image(dev: &mut MemoryBackend) -> Vec<u8> {
    let mut bytes = vec![0u8; dev.total_blocks() as usize * BLOCK];
    for lba in 0..dev.total_blocks() {
        dev.read_block(lba, &mut bytes[lba as usize * BLOCK..][..BLOCK])
            .unwrap();
    }
    bytes
}

/// Offset of the committed record of `object_id`: the object block with the
/// highest generation that names it.
fn record_offset(image: &[u8], object_id: u64) -> usize {
    image
        .chunks(BLOCK)
        .enumerate()
        .filter_map(|(lba, block)| {
            let header = BlockHeader::verify(block, block_type::OBJECT).ok()?;
            (header.owner == object_id).then_some((header.generation, lba * BLOCK))
        })
        .max()
        .expect("object record present")
        .1
}

fn reseal(block: &mut [u8], payload_len: u32) {
    // The block was edited, so read the identity fields directly.
    let owner = u64::from_le_bytes(block[8..16].try_into().unwrap());
    let generation = u64::from_le_bytes(block[16..24].try_into().unwrap());
    BlockHeader {
        block_type: block_type::OBJECT,
        flags: 0,
        owner,
        generation,
        payload_len,
    }
    .seal(block);
}

fn compile(scratch: &Scratch) -> PathBuf {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let executable = scratch.0.join("probe");
    let output = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()))
        .args([
            "-std=c99",
            "-pedantic",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wconversion",
            "-Wshadow",
            "-Wstrict-prototypes",
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
        ])
        .arg("-I")
        .arg(repo.join("api"))
        .arg("-I")
        .arg(repo.join("spec"))
        .arg(repo.join("portable/c/reader.c"))
        .arg(repo.join("portable/c/tests/security_probe.c"))
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    executable
}

/// What both readers must report for one object of one image.
enum Verdict {
    Object {
        kind: u32,
        flags: u16,
        protection: u32,
    },
    Corrupt,
}

fn agree(probe: &PathBuf, scratch: &Scratch, label: &str, image: &[u8], id: u64, want: Verdict) {
    // Rust core.
    let mut dev = MemoryBackend::new(BLOCK, (image.len() / BLOCK) as u64);
    for (lba, block) in image.chunks(BLOCK).enumerate() {
        dev.write_block(lba as u64, block).unwrap();
    }
    let rust = mount(dev).and_then(|mut volume| volume.stat(id));
    match (&want, rust) {
        (
            Verdict::Object {
                kind,
                flags,
                protection,
            },
            Ok(Some(record)),
        ) => {
            let wire = match record.object_type {
                afsplus_format::object::ObjectType::File => 1,
                afsplus_format::object::ObjectType::Directory => 2,
                afsplus_format::object::ObjectType::Symlink => 3,
                afsplus_format::object::ObjectType::Internal => 4,
            };
            assert_eq!(
                (wire, record.flags, record.protection),
                (*kind, *flags, *protection),
                "Rust fields: {label}"
            );
        }
        (Verdict::Corrupt, Err(CoreError::Corrupt(_) | CoreError::Format(_))) => {}
        (_, other) => panic!("Rust verdict differs: {label}: {other:?}"),
    }
    // Portable C reader on the same bytes.
    let path = scratch.0.join("image");
    fs::write(&path, image).unwrap();
    let mut command = Command::new(probe);
    command.arg("lookup").arg(&path).arg(id.to_string());
    match want {
        Verdict::Object {
            kind,
            flags,
            protection,
        } => command.args([kind.to_string(), flags.to_string(), protection.to_string()]),
        Verdict::Corrupt => command.arg("corrupt"),
    };
    assert_eq!(
        command.status().unwrap().code(),
        Some(0),
        "C verdict differs: {label}"
    );
}

#[test]
fn c_and_rust_agree_on_objects_of_real_images() {
    let scratch = Scratch(
        std::env::temp_dir().join(format!("afsplus-security-c-volume-{}", std::process::id())),
    );
    fs::create_dir(&scratch.0).unwrap();
    let probe = compile(&scratch);

    // A volume with the container: descriptor on a file, a directory, a
    // symlink and the root; one plain file beside them.
    let mut dev = MemoryBackend::new(BLOCK, 1024);
    mkfs_with_security_descriptors(&mut dev, &params()).unwrap();
    let mut volume = mount(dev).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"data", time(2))
        .unwrap();
    let dir = volume
        .create_directory(OBJECT_ROOT, "dir", time(2))
        .unwrap();
    let link = volume
        .create_symlink(OBJECT_ROOT, "link", "file", time(2))
        .unwrap();
    let plain = volume
        .create_file_in_directory(OBJECT_ROOT, "plain", b"", time(2))
        .unwrap();
    volume.set_object_protection(file, 0x11, time(3)).unwrap();
    volume.set_object_protection(dir, 0x22, time(3)).unwrap();
    volume.set_object_protection(plain, 0x33, time(3)).unwrap();
    let descriptor: Vec<u8> = (0..9000u32).map(|i| (i * 13) as u8).collect();
    for id in [file, dir, link, OBJECT_ROOT] {
        volume
            .set_security_descriptor(id, 0x7fff_0042, 1, &descriptor, time(4))
            .unwrap();
    }
    let secured = image(&mut volume.into_device());

    for (label, id, kind, flags, protection) in [
        ("secured file", file, 1, 4, 0x11),
        ("secured directory", dir, 2, 4, 0x22),
        ("secured symlink", link, 3, 4, 0),
        ("secured root", OBJECT_ROOT, 2, 4, 0),
        ("plain file beside them", plain, 1, 0, 0x33),
    ] {
        agree(
            &probe,
            &scratch,
            label,
            &secured,
            id,
            Verdict::Object {
                kind,
                flags,
                protection,
            },
        );
    }

    // Resealed corruptions of committed records on that image.
    let at = record_offset(&secured, plain);
    let mut long_payload = secured.clone();
    reseal(&mut long_payload[at..at + BLOCK], 104);
    agree(
        &probe,
        &scratch,
        "payload of 104 bytes",
        &long_payload,
        plain,
        Verdict::Corrupt,
    );
    let mut dirty_tail = secured.clone();
    dirty_tail[at + BLOCK - 1] = 1;
    reseal(&mut dirty_tail[at..at + BLOCK], 96);
    agree(
        &probe,
        &scratch,
        "nonzero tail",
        &dirty_tail,
        plain,
        Verdict::Corrupt,
    );
    let at = record_offset(&secured, file);
    let mut outside = secured.clone();
    outside[at + HEADER_SIZE + 96..at + HEADER_SIZE + 104].copy_from_slice(&u64::MAX.to_le_bytes());
    reseal(&mut outside[at..at + BLOCK], 112);
    // The Rust lookup carries the reference unread, as the C lookup does;
    // only the C volume path bounds the first segment at lookup, so this
    // image is checked against C alone.
    fs::write(scratch.0.join("image"), &outside).unwrap();
    assert_eq!(
        Command::new(&probe)
            .arg("lookup")
            .arg(scratch.0.join("image"))
            .args([file.to_string(), "corrupt".into()])
            .status()
            .unwrap()
            .code(),
        Some(0)
    );

    // A volume without the feature: a record that carries a reference is
    // corruption for both readers.
    let mut dev = MemoryBackend::new(BLOCK, 1024);
    mkfs(&mut dev, &params()).unwrap();
    let mut volume = mount(dev).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"", time(2))
        .unwrap();
    let mut featureless = image(&mut volume.into_device());
    agree(
        &probe,
        &scratch,
        "plain file on a volume without the feature",
        &featureless,
        file,
        Verdict::Object {
            kind: 1,
            flags: 0,
            protection: 0,
        },
    );
    let at = record_offset(&featureless, file);
    let (record, generation) =
        ObjectRecord::decode_metadata_with_generation(&featureless[at..at + BLOCK]).unwrap();
    let forged = record
        .with_security(Some(SecurityRef {
            first_block: 600,
            total_len: 10,
            segment_count: 1,
            flags: 0,
        }))
        .encode(BLOCK, generation)
        .unwrap();
    featureless[at..at + BLOCK].copy_from_slice(&forged);
    agree(
        &probe,
        &scratch,
        "reference without the feature",
        &featureless,
        file,
        Verdict::Corrupt,
    );
}
