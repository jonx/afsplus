//! Cross-read the Rust wire representation with the independent heap-free C codec.
#![cfg(unix)]
use afsplus_format::{
    header::{block_type, BlockHeader, HEADER_SIZE},
    object::{ObjectRecord, ObjectType, SymlinkRecord},
    Timespec,
};
use std::{fs, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn independent_c_symlink_codec_matches_valid_and_malformed_rust_records() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-symlink-codec-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();
    let compiler = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
    let stamp = Timespec {
        seconds: -3,
        nanoseconds: 999_999_999,
    };
    let maximum = "x".repeat(3968);
    for sanitize in [false, true] {
        let executable = scratch
            .0
            .join(if sanitize { "sanitized" } else { "strict" });
        let mut command = Command::new(&compiler);
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
            .arg(repo.join("portable/c/tests/symlink_probe.c"))
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for target in [
            "a",
            "../dir/./file",
            "/absolute//target",
            "SYS:Tools",
            "café/日本語",
            &maximum,
        ] {
            let record = ObjectRecord {
                object_id: 16,
                object_type: ObjectType::Symlink,
                flags: 0,
                link_count: 1,
                size_bytes: target.len() as u64,
                allocated_bytes: 0,
                created: stamp,
                modified: stamp,
                changed: stamp,
                protection: 123,
                content_generation: 5,
                data_root: 0,
                data_blocks: 0,
                security: None,
            };
            let valid = SymlinkRecord { record, target }.encode(4096, 7).unwrap();
            let block_path = scratch.0.join("record.bin");
            let target_path = scratch.0.join("target.bin");
            fs::write(&target_path, target).unwrap();
            fs::write(&block_path, &valid).unwrap();
            assert!(Command::new(&executable)
                .arg(&block_path)
                .arg(&target_path)
                .status()
                .unwrap()
                .success());
            for case in 0..14 {
                let mut block = valid.clone();
                let mut header = BlockHeader::verify(&block, block_type::OBJECT).unwrap();
                match case {
                    0 => block[128] = 0xff,
                    1 => block[128] = 0,
                    2 => block[HEADER_SIZE + 9] = 1,
                    3 => block[HEADER_SIZE + 10] = 1,
                    4 => block[HEADER_SIZE + 24] = 1,
                    5 => block[HEADER_SIZE + 80] = 1,
                    6 => block[HEADER_SIZE + 88] = 1,
                    7 => block[HEADER_SIZE + 16] ^= 1,
                    8 => header.payload_len -= 1,
                    9 => header.flags = 1,
                    10 => header.owner += 1,
                    11 => block[HEADER_SIZE + 12..HEADER_SIZE + 16].fill(0),
                    12 => block[HEADER_SIZE + 40..HEADER_SIZE + 44].fill(0xff),
                    13 => block[HEADER_SIZE + 8] = 1,
                    _ => unreachable!(),
                }
                header.seal(&mut block);
                assert!(SymlinkRecord::decode(&block).is_err(), "Rust case {case}");
                fs::write(&block_path, block).unwrap();
                assert!(
                    Command::new(&executable)
                        .arg(&block_path)
                        .arg("reject")
                        .status()
                        .unwrap()
                        .success(),
                    "C case {case}"
                );
            }
        }
    }
}
