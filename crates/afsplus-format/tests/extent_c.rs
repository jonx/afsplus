//! The extent-map item has one wire image, and the Rust codec, the portable C
//! decoder and a literal expectation agree on every image below.
#![cfg(unix)]
use afsplus_format::extent::{ExtentItem, EXTENT_SHARED, EXTENT_UNWRITTEN};
use std::{fs, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Label, key, value and the literal expectation (`None` is a refusal).
type Case = (&'static str, Vec<u8>, Vec<u8>, Option<ExtentItem>);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn extent_item_wire_image_is_literal_and_c_agrees() {
    // The documented image, byte for byte.
    let item = ExtentItem {
        logical_start: 0x0102_0304_0506_0708,
        physical_start: 0x1112_1314_1516_1718,
        block_count: 3,
        flags: EXTENT_SHARED,
    };
    let (key, value) = item.encode().unwrap();
    assert_eq!(key, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        value,
        [
            0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, // physical start
            3, 0, 0, 0, 0, 0, 0, 0, // block count
            2, 0, 0, 0, // flags: shared is bit 1
            0, 0, 0, 0, // reserved
        ]
    );
    assert_eq!((EXTENT_UNWRITTEN, EXTENT_SHARED), (1, 2));
    assert_eq!(ExtentItem::decode(&key, &value), Ok(item));

    // (label, key, value, expectation).
    let mut cases: Vec<Case> = Vec::new();
    for flags in [
        0,
        EXTENT_UNWRITTEN,
        EXTENT_SHARED,
        EXTENT_UNWRITTEN | EXTENT_SHARED,
    ] {
        let item = ExtentItem { flags, ..item };
        let (key, value) = item.encode().unwrap();
        cases.push(("valid", key.to_vec(), value.to_vec(), Some(item)));
    }
    let edge = ExtentItem {
        logical_start: u64::MAX - 1,
        physical_start: u64::MAX - 1,
        block_count: 1,
        flags: 0,
    };
    let (edge_key, edge_value) = edge.encode().unwrap();
    cases.push((
        "last representable block",
        edge_key.to_vec(),
        edge_value.to_vec(),
        Some(edge),
    ));
    let mutate = |at: usize, byte: u8| {
        let mut bad = value;
        bad[at] = byte;
        bad.to_vec()
    };
    cases.push(("zero count", key.to_vec(), mutate(8, 0), None));
    cases.push(("unknown flag bit 2", key.to_vec(), mutate(16, 4), None));
    cases.push(("unknown high flag", key.to_vec(), mutate(19, 0x80), None));
    for at in 20..24 {
        cases.push(("reserved byte", key.to_vec(), mutate(at, 1), None));
    }
    cases.push(("short value", key.to_vec(), value[..23].to_vec(), None));
    cases.push((
        "long value",
        key.to_vec(),
        [&value[..], &[0]].concat(),
        None,
    ));
    cases.push(("short key", key[..7].to_vec(), value.to_vec(), None));
    let mut overflow = value;
    overflow[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    cases.push((
        "physical and logical overflow",
        key.to_vec(),
        overflow.to_vec(),
        None,
    ));
    assert!(ExtentItem {
        block_count: 0,
        ..item
    }
    .encode()
    .is_err());
    assert!(ExtentItem { flags: 4, ..item }.encode().is_err());

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-extent-c-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();
    let probe = scratch.0.join("probe");
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
        ])
        .arg("-I")
        .arg(repo.join("api"))
        .arg("-I")
        .arg(repo.join("spec"))
        .arg(repo.join("portable/c/reader.c"))
        .arg(repo.join("portable/c/tests/extent_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (label, key, value, expected) in &cases {
        assert_eq!(
            &ExtentItem::decode(key, value).ok(),
            expected,
            "Rust: {label}"
        );
        let mut command = Command::new(&probe);
        command.arg(hex(key)).arg(hex(value));
        match expected {
            None => command.arg("reject"),
            Some(e) => command.args([
                e.logical_start.to_string(),
                e.physical_start.to_string(),
                e.block_count.to_string(),
                e.flags.to_string(),
            ]),
        };
        assert_eq!(command.status().unwrap().code(), Some(0), "C: {label}");
    }
    // A wrong expectation is a mismatch, so exit code 0 above is evidence.
    let wrong = Command::new(&probe)
        .arg(hex(&key))
        .arg(hex(&value))
        .args(["1", "2", "3", "1"])
        .status()
        .unwrap();
    assert_eq!(wrong.code(), Some(1));
    assert_eq!(cases.len(), 16);
}
