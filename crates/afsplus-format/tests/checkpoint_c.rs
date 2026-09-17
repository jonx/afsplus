//! Cross-read one checkpoint block, with and without the snapshot roots of
//! ADR-073, with the portable C decoder: for every image the C verdict must
//! equal the Rust verdict, and both must equal the literal expectation.
#![cfg(unix)]
use afsplus_format::{
    checkpoint::{Checkpoint, SnapshotRoots},
    header::{block_type, BlockHeader, HEADER_SIZE},
    DEFAULT_BLOCK_SIZE, OBJECT_ROOT,
};
use std::{fs, path::PathBuf, process::Command};

const SIZE: usize = DEFAULT_BLOCK_SIZE;
const UUID: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn compile(scratch: &Scratch, sanitize: bool) -> PathBuf {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let compiler = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
    let executable = scratch
        .0
        .join(if sanitize { "sanitized" } else { "strict" });
    let mut command = Command::new(compiler);
    command.args([
        "-std=c99",
        "-pedantic",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Wconversion",
        "-Wshadow",
        "-Wstrict-prototypes",
    ]);
    if sanitize {
        command.args(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]);
    }
    let output = command
        .arg("-I")
        .arg(repo.join("api"))
        .arg("-I")
        .arg(repo.join("spec"))
        .arg(repo.join("portable/c/reader.c"))
        .arg(repo.join("portable/c/tests/checkpoint_probe.c"))
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

fn checkpoint(label: &str, roots: Option<SnapshotRoots>) -> Checkpoint {
    Checkpoint {
        uuid: UUID,
        generation: 0x0102_0304_0506,
        root_object_id: OBJECT_ROOT,
        object_map_block: 1001,
        allocation_root_block: 1002,
        reclaim_root_block: 1003,
        next_object_id: 1004,
        committed_tx_id: 1005,
        free_blocks_total: 1006,
        flags: 0,
        shared_extent_root_block: 1007,
        label: label.into(),
        snapshot_roots: roots,
    }
}

fn reseal(block: &mut [u8], payload_len: Option<u32>) {
    BlockHeader {
        block_type: block_type::CHECKPOINT,
        flags: u16::from_le_bytes(block[6..8].try_into().unwrap()),
        owner: u64::from_le_bytes(block[8..16].try_into().unwrap()),
        generation: u64::from_le_bytes(block[16..24].try_into().unwrap()),
        payload_len: payload_len
            .unwrap_or_else(|| u32::from_le_bytes(block[24..28].try_into().unwrap())),
    }
    .seal(block);
}

#[test]
fn independent_c_decoder_agrees_on_checkpoint_blocks() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-checkpoint-c-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();
    let block_path = scratch.0.join("block");
    let label_path = scratch.0.join("label");
    let roots = SnapshotRoots {
        registry: 2001,
        lifetimes: 2002,
    };
    let p = HEADER_SIZE;

    let mut images: Vec<(String, Vec<u8>, bool)> = Vec::new();
    let plain = checkpoint("Work", None).encode(SIZE).unwrap();
    let snapshot = checkpoint("Wörk 🜁", Some(roots)).encode(SIZE).unwrap();
    assert_eq!(&plain[24..28], &168u32.to_le_bytes());
    assert_eq!(&snapshot[24..28], &184u32.to_le_bytes());
    assert_eq!(&snapshot[p + 168..p + 176], &2001u64.to_le_bytes());
    assert_eq!(&snapshot[p + 176..p + 184], &2002u64.to_le_bytes());
    images.push(("plain".into(), plain.clone(), true));
    images.push(("with snapshot roots".into(), snapshot.clone(), true));
    images.push((
        "empty label".into(),
        checkpoint("", Some(roots)).encode(SIZE).unwrap(),
        true,
    ));
    images.push((
        "longest label".into(),
        checkpoint(&"é".repeat(32), None).encode(SIZE).unwrap(),
        true,
    ));
    for (what, good) in [("plain", &plain), ("snapshot", &snapshot)] {
        let cases: [(&str, usize, &[u8]); 9] = [
            ("another volume's UUID", 0, &[0xfe]),
            ("zero generation", 16, &[0; 8]),
            ("generation unlike the header's", 16, &[7]),
            ("wrong root object", 24, &[9]),
            ("nonzero flags word", 80, &[1]),
            ("label length above 64", 96, &[65]),
            ("label reserved byte", 97, &[1]),
            ("label byte after its length", 96 + 8 + 20, b"x"),
            ("NUL inside the label", 96 + 8 + 1, &[0]),
        ];
        for (label, offset, bytes) in cases {
            let mut block = good.clone();
            block[p + offset..p + offset + bytes.len()].copy_from_slice(bytes);
            reseal(&mut block, None);
            images.push((format!("{what}: {label}"), block, false));
        }
        let mut utf8 = good.clone();
        utf8[p + 96 + 8] = 0xff;
        reseal(&mut utf8, None);
        images.push((format!("{what}: label not UTF-8"), utf8, false));
        let mut flags = good.clone();
        flags[6] = 1;
        reseal(&mut flags, None);
        images.push((format!("{what}: header flags"), flags, false));
        let mut owner = good.clone();
        owner[8] = 1;
        reseal(&mut owner, None);
        images.push((format!("{what}: header owner"), owner, false));
        for length in [0u32, 96, 167, 169, 176, 183, 185, 200] {
            let mut block = good.clone();
            reseal(&mut block, Some(length));
            images.push((format!("{what}: payload of {length} bytes"), block, false));
        }
        let mut broken = good.clone();
        broken[p + 40] ^= 1;
        images.push((format!("{what}: broken checksum"), broken, false));
    }
    // The long payload needs its roots; the short one must not be read as
    // long.
    let mut grown = plain.clone();
    reseal(&mut grown, Some(184));
    images.push((
        "plain resealed as 184 bytes: zero roots".into(),
        grown,
        false,
    ));
    let mut shrunk = snapshot.clone();
    reseal(&mut shrunk, Some(168));
    images.push((
        "snapshot resealed as 168 bytes: its roots are a nonzero tail".into(),
        shrunk,
        false,
    ));
    for (label, registry, lifetimes) in [
        ("zero registry root", 0u64, 2002u64),
        ("zero lifetime root", 2001, 0),
        ("equal roots", 2001, 2001),
    ] {
        let mut block = snapshot.clone();
        block[p + 168..p + 176].copy_from_slice(&registry.to_le_bytes());
        block[p + 176..p + 184].copy_from_slice(&lifetimes.to_le_bytes());
        reseal(&mut block, None);
        images.push((label.into(), block, false));
    }
    // Nothing follows the payload (ADR-111).
    let mut tail = plain.clone();
    tail[SIZE - 1] = 1;
    reseal(&mut tail, None);
    images.push(("nonzero tail".into(), tail, false));

    let uuid_hex: String = UUID.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut checked = 0;
    for sanitize in [false, true] {
        let executable = compile(&scratch, sanitize);
        for (label, image, accepted) in &images {
            let rust = Checkpoint::decode(image, &UUID);
            assert_eq!(rust.is_ok(), *accepted, "Rust verdict: {label}");
            let mut arguments = vec![block_path.to_str().unwrap().to_owned(), uuid_hex.clone()];
            match rust {
                Err(_) => arguments.push("reject".into()),
                Ok(c) => {
                    let roots = c.snapshot_roots;
                    arguments.extend(
                        [
                            c.generation,
                            c.object_map_block,
                            c.allocation_root_block,
                            c.reclaim_root_block,
                            c.next_object_id,
                            c.committed_tx_id,
                            c.free_blocks_total,
                            c.shared_extent_root_block,
                            u64::from(roots.is_some()),
                            roots.map_or(0, |r| r.registry),
                            roots.map_or(0, |r| r.lifetimes),
                        ]
                        .map(|value| value.to_string()),
                    );
                    fs::write(&label_path, c.label.as_bytes()).unwrap();
                    arguments.push(label_path.to_str().unwrap().to_owned());
                }
            }
            fs::write(&block_path, image).unwrap();
            let status = Command::new(&executable).args(&arguments).status().unwrap();
            assert_eq!(
                status.code(),
                Some(0),
                "C verdict: {label} (sanitize={sanitize})"
            );
            checked += 1;
        }
        // Another volume's UUID refuses a good block, and the probe can fail.
        fs::write(&block_path, &plain).unwrap();
        let other = Command::new(&executable)
            .args([
                block_path.to_str().unwrap(),
                "ffffffffffffffffffffffffffffffff",
                "reject",
            ])
            .status()
            .unwrap();
        assert_eq!(other.code(), Some(0));
        let wrong = Command::new(&executable)
            .args([block_path.to_str().unwrap(), &uuid_hex, "reject"])
            .status()
            .unwrap();
        assert_eq!(wrong.code(), Some(1));
    }
    eprintln!("checkpoint cross-read images checked: {checked}");
}
