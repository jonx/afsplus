//! Cross-read exact object admission and the security preservation container
//! with the independent heap-free C codec: for every image the C verdict must
//! equal the Rust verdict, and both must equal the literal expectation.
#![cfg(unix)]
use afsplus_format::{
    header::{block_type, BlockHeader, HEADER_SIZE},
    object::{ObjectRecord, ObjectType, SecurityRef, SymlinkRecord},
    security::SecuritySegment,
    Timespec, DEFAULT_BLOCK_SIZE,
};
use std::{fs, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(kind: ObjectType) -> ObjectRecord {
    ObjectRecord {
        object_id: 16,
        object_type: kind,
        flags: 0,
        link_count: 1,
        size_bytes: 0,
        allocated_bytes: 0,
        created: Timespec::default(),
        modified: Timespec::default(),
        changed: Timespec::default(),
        protection: 0x5a,
        content_generation: 1,
        data_root: if kind == ObjectType::Directory {
            123
        } else {
            0
        },
        data_blocks: 0,
        security: None,
    }
}

fn reference() -> SecurityRef {
    SecurityRef {
        first_block: 0x0102_0304_0506_0708,
        total_len: 5000,
        segment_count: 2,
        flags: 1,
    }
}

fn reseal(block: &mut [u8], kind: u32, flags: u16, payload_len: u32) {
    BlockHeader {
        block_type: kind,
        flags,
        owner: 16,
        generation: 7,
        payload_len,
    }
    .seal(block);
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

#[test]
fn independent_c_codec_agrees_on_object_admission_and_the_security_container() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-security-c-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();
    let block_path = scratch.0.join("block");
    let bytes_path = scratch.0.join("bytes");
    let size = DEFAULT_BLOCK_SIZE;

    // (label, image, literal expectation: None = reject, Some(None) = no
    // reference, Some(Some(r)) = that reference).
    let mut references: Vec<(String, Vec<u8>, Option<Option<SecurityRef>>)> = Vec::new();
    for kind in [ObjectType::File, ObjectType::Directory] {
        let plain = record(kind).encode(size, 7).unwrap();
        references.push((format!("{kind:?} plain"), plain.clone(), Some(None)));
        let secured = record(kind)
            .with_security(Some(reference()))
            .encode(size, 7)
            .unwrap();
        references.push((
            format!("{kind:?} secured"),
            secured.clone(),
            Some(Some(reference())),
        ));
        for bit in 0..16 {
            let mut block = plain.clone();
            reseal(&mut block, block_type::OBJECT, 1 << bit, 96);
            references.push((format!("{kind:?} header flag {bit}"), block, None));
        }
        for extra in [1usize, 16, size - HEADER_SIZE - 96] {
            for fill in [0u8, 0xa5] {
                let mut block = plain.clone();
                block[HEADER_SIZE + 96..HEADER_SIZE + 96 + extra].fill(fill);
                reseal(&mut block, block_type::OBJECT, 0, (96 + extra) as u32);
                references.push((format!("{kind:?} payload +{extra} fill {fill}"), block, None));
            }
        }
        for offset in [HEADER_SIZE + 96, size / 2, size - 1] {
            let mut block = plain.clone();
            block[offset] = 1;
            reseal(&mut block, block_type::OBJECT, 0, 96);
            references.push((format!("{kind:?} tail at {offset}"), block, None));
        }
        // The flag without the 16 bytes, and malformed reference fields.
        let mut flag_only = plain.clone();
        flag_only[HEADER_SIZE + 10] = 4;
        reseal(&mut flag_only, block_type::OBJECT, 0, 96);
        references.push((format!("{kind:?} flag without reference"), flag_only, None));
        for (offset, value, what) in [
            (96usize, 0u8, "zero first block"),
            (104, 0x89, "length that needs another count"),
            (108, 3, "wrong segment count"),
            (110, 3, "unassigned reference flag"),
        ] {
            let mut block = secured.clone();
            if offset == 96 {
                block[HEADER_SIZE + 96..HEADER_SIZE + 104].fill(value);
            } else if offset == 104 {
                // 5000 -> 5001 keeps two segments; 0x89 0x3f = 16265 needs five.
                block[HEADER_SIZE + 104] = value;
                block[HEADER_SIZE + 105] = 0x3f;
            } else {
                block[HEADER_SIZE + offset] = value;
            }
            reseal(&mut block, block_type::OBJECT, 0, 112);
            references.push((format!("{kind:?} {what}"), block, None));
        }
    }
    let mut link = record(ObjectType::Symlink);
    link.size_bytes = 6;
    for security in [None, Some(reference())] {
        let block = SymlinkRecord {
            record: link.with_security(security),
            target: "target",
        }
        .encode(size, 7)
        .unwrap();
        references.push((format!("symlink {security:?}"), block, Some(security)));
    }

    let descriptor: Vec<u8> = (0..4041u32).map(|i| (i * 7) as u8).collect();
    let first = SecuritySegment {
        object_id: 16,
        format: 0x7fff_0042,
        version: 3,
        total_len: 4041,
        index: 0,
        count: 2,
        next: 500,
        bytes: &descriptor[..4040],
    };
    let last = SecuritySegment {
        index: 1,
        next: 0,
        bytes: &descriptor[4040..],
        ..first
    };
    let mut segments: Vec<(String, Vec<u8>, Option<SecuritySegment<'_>>)> = vec![
        ("first".into(), first.encode(size, 7).unwrap(), Some(first)),
        ("last".into(), last.encode(size, 7).unwrap(), Some(last)),
    ];
    let clean = last.encode(size, 7).unwrap();
    for (what, offset, value, payload, flags) in [
        ("reserved field", HEADER_SIZE + 6, 1u8, 25u32, 0u16),
        ("zero format", HEADER_SIZE, 0, 25, 0),
        ("index beyond count", HEADER_SIZE + 12, 2, 25, 0),
        ("last segment with a successor", HEADER_SIZE + 16, 9, 25, 0),
        ("nonzero tail", size - 1, 1, 25, 0),
        ("longer payload", size - 2, 0, 26, 0),
        ("header flag", size - 2, 0, 25, 1),
    ] {
        let mut block = clean.clone();
        block[offset] = value;
        if what == "zero format" {
            block[HEADER_SIZE..HEADER_SIZE + 4].fill(0);
        }
        reseal(&mut block, block_type::SECURITY_DESCRIPTOR, flags, payload);
        segments.push((what.into(), block, None));
    }
    // An object block is never a segment, and a segment is never an object.
    segments.push((
        "object block".into(),
        record(ObjectType::File).encode(size, 7).unwrap(),
        None,
    ));
    references.push(("segment block".into(), clean.clone(), None));

    for sanitize in [false, true] {
        let executable = compile(&scratch, sanitize);
        for (label, block, expected) in &references {
            let rust = ObjectRecord::decode_metadata_with_generation(block)
                .ok()
                .map(|(record, _)| record.security);
            assert_eq!(&rust, expected, "Rust verdict: {label}");
            fs::write(&block_path, block).unwrap();
            let mut command = Command::new(&executable);
            command.arg("ref").arg(&block_path);
            match expected {
                None => command.arg("reject"),
                Some(None) => command.arg("none"),
                Some(Some(r)) => command.args([
                    r.first_block.to_string(),
                    r.total_len.to_string(),
                    r.segment_count.to_string(),
                    r.flags.to_string(),
                ]),
            };
            assert_eq!(
                command.status().unwrap().code(),
                Some(0),
                "C verdict differs: {label} (sanitize={sanitize})"
            );
        }
        for (label, block, expected) in &segments {
            let rust = SecuritySegment::decode(block).ok().map(|(s, _)| s);
            assert_eq!(&rust, expected, "Rust verdict: {label}");
            fs::write(&block_path, block).unwrap();
            let mut command = Command::new(&executable);
            command.arg("seg").arg(&block_path);
            match expected {
                None => {
                    command.arg("reject");
                }
                Some(s) => {
                    fs::write(&bytes_path, s.bytes).unwrap();
                    command
                        .args([
                            s.object_id.to_string(),
                            s.format.to_string(),
                            s.version.to_string(),
                            s.total_len.to_string(),
                            s.index.to_string(),
                            s.count.to_string(),
                            s.next.to_string(),
                            "7".into(),
                        ])
                        .arg(&bytes_path);
                }
            };
            assert_eq!(
                command.status().unwrap().code(),
                Some(0),
                "C verdict differs: {label} (sanitize={sanitize})"
            );
        }
        // Negative control of the probe itself: a wrong expectation is a
        // mismatch, so exit code 0 above is evidence.
        fs::write(&block_path, &references[1].1).unwrap();
        let wrong = Command::new(&executable)
            .arg("ref")
            .arg(&block_path)
            .args(["1", "5000", "2", "1"])
            .status()
            .unwrap();
        assert_eq!(wrong.code(), Some(1));
    }
    assert_eq!(references.len(), 2 * (2 + 16 + 6 + 3 + 1 + 4) + 2 + 1);
    assert_eq!(segments.len(), 10);
}
