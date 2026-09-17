//! Cross-read the snapshot record codecs (ADR-072) with the portable C
//! decoders: for every value and context the C verdict must equal the Rust
//! verdict, and both must equal the literal expectation.
#![cfg(unix)]
use afsplus_format::snapshot::{
    decode_key, LedgerState, LifetimeRecord, RegistryState, SnapshotRecord, VALUE_SIZE,
};
use std::{fs, path::PathBuf, process::Command};

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
        .arg(repo.join("portable/c/tests/snapshot_probe.c"))
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

fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-".into();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn value(words: [u64; 4]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// One probe invocation: kind, bytes, context, and the expected fields, or
/// `None` for a refusal.
struct Case {
    label: String,
    kind: &'static str,
    bytes: Vec<u8>,
    context: Vec<u64>,
    expected: Option<Vec<u64>>,
}

#[test]
fn independent_c_decoders_agree_on_the_snapshot_records() {
    const MAX_GENERATION: u64 = 100;
    const TOTAL: u64 = 1000;
    let mut cases: Vec<Case> = Vec::new();
    let mut push = |label: &str,
                    kind: &'static str,
                    bytes: Vec<u8>,
                    context: &[u64],
                    expected: Option<Vec<u64>>| {
        cases.push(Case {
            label: label.into(),
            kind,
            bytes,
            context: context.to_vec(),
            expected,
        });
    };

    // Keys: eight bytes, big-endian.
    push(
        "key",
        "key",
        0x0102_0304_0506_0708u64.to_be_bytes().to_vec(),
        &[],
        Some(vec![0x0102_0304_0506_0708]),
    );
    push("key zero", "key", vec![0; 8], &[], Some(vec![0]));
    for length in [0usize, 7, 9] {
        push(
            &format!("key of {length} bytes"),
            "key",
            vec![1; length],
            &[],
            None,
        );
    }

    // Registry control record.
    push(
        "registry",
        "registry",
        value([7, 0, 0, 0]),
        &[],
        Some(vec![7]),
    );
    push(
        "registry at the last ID",
        "registry",
        value([u64::MAX, 0, 0, 0]),
        &[],
        Some(vec![u64::MAX]),
    );
    push(
        "registry zero next ID",
        "registry",
        value([0, 0, 0, 0]),
        &[],
        None,
    );
    push(
        "registry reserved",
        "registry",
        value([7, 1, 0, 0]),
        &[],
        None,
    );
    push(
        "registry last reserved byte",
        "registry",
        value([7, 0, 0, 1 << 56]),
        &[],
        None,
    );
    push(
        "registry short",
        "registry",
        value([7, 0, 0, 0])[..31].to_vec(),
        &[],
        None,
    );
    push(
        "registry long",
        "registry",
        [value([7, 0, 0, 0]), vec![0]].concat(),
        &[],
        None,
    );
    push("registry empty", "registry", vec![], &[], None);

    // Snapshot record, judged against the generation and the volume size.
    let context = [MAX_GENERATION, TOTAL];
    push(
        "record",
        "record",
        value([50, 40, 999, 0]),
        &context,
        Some(vec![50, 40, 999]),
    );
    push(
        "record at every bound",
        "record",
        value([100, 100, 1, 0]),
        &context,
        Some(vec![100, 100, 1]),
    );
    for (label, words) in [
        ("record zero generation", [0, 0, 5, 0]),
        ("record generation above the checkpoint", [101, 40, 5, 0]),
        ("record zero transaction", [50, 0, 5, 0]),
        ("record transaction above its generation", [50, 51, 5, 0]),
        ("record zero root", [50, 40, 0, 0]),
        ("record root at the volume end", [50, 40, 1000, 0]),
        ("record reserved", [50, 40, 5, 1]),
    ] {
        push(label, "record", value(words), &context, None);
    }
    push(
        "record short",
        "record",
        value([50, 40, 5, 0])[..24].to_vec(),
        &context,
        None,
    );

    // Lifetime record, keyed by its first block.
    let at = |start: u64| [start, MAX_GENERATION, TOTAL];
    push(
        "lifetime live",
        "lifetime",
        value([10, 5, 0, 0]),
        &at(20),
        Some(vec![10, 5, 0]),
    );
    push(
        "lifetime retired, to the volume end",
        "lifetime",
        value([980, 5, 100, 0]),
        &at(20),
        Some(vec![980, 5, 100]),
    );
    for (label, words, start) in [
        ("lifetime at block zero", [10, 5, 0, 0], 0),
        ("lifetime zero blocks", [0, 5, 0, 0], 20),
        ("lifetime past the volume end", [981, 5, 0, 0], 20),
        ("lifetime whose end overflows", [u64::MAX, 5, 0, 0], 20),
        ("lifetime zero birth", [10, 0, 0, 0], 20),
        ("lifetime birth above the checkpoint", [10, 101, 0, 0], 20),
        ("lifetime retired at its birth", [10, 5, 5, 0], 20),
        ("lifetime retired above the checkpoint", [10, 5, 101, 0], 20),
        ("lifetime reserved", [10, 5, 0, 1], 20),
    ] {
        push(label, "lifetime", value(words), &at(start), None);
    }

    // Ledger control record.
    push(
        "ledger",
        "ledger",
        value([999, 1000, 0, 0]),
        &[TOTAL],
        Some(vec![999, 1000]),
    );
    push(
        "ledger zero",
        "ledger",
        value([0, 0, 0, 0]),
        &[TOTAL],
        Some(vec![0, 0]),
    );
    for (label, words, total) in [
        (
            "ledger scan position at the volume end",
            [1000, 0, 0, 0],
            TOTAL,
        ),
        (
            "ledger retains more than the volume",
            [0, 1001, 0, 0],
            TOTAL,
        ),
        ("ledger of an empty volume", [0, 0, 0, 0], 0),
        ("ledger reserved", [0, 0, 1, 0], TOTAL),
    ] {
        push(label, "ledger", value(words), &[total], None);
    }

    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-snapshot-c-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();
    let mut checked = 0;
    for sanitize in [false, true] {
        let executable = compile(&scratch, sanitize);
        for case in &cases {
            let c = &case.context;
            let rust: Option<Vec<u64>> = match case.kind {
                "key" => decode_key(&case.bytes).ok().map(|id| vec![id]),
                "registry" => RegistryState::decode(&case.bytes)
                    .ok()
                    .map(|s| vec![s.next_id]),
                "record" => SnapshotRecord::decode(&case.bytes, c[0], c[1])
                    .ok()
                    .map(|r| vec![r.generation, r.committed_tx_id, r.object_map_root]),
                "lifetime" => LifetimeRecord::decode(&case.bytes, c[0], c[1], c[2])
                    .ok()
                    .map(|l| vec![l.blocks, l.birth, l.retirement]),
                _ => LedgerState::decode(&case.bytes, c[0])
                    .ok()
                    .map(|l| vec![l.scan_position, l.retained_blocks]),
            };
            assert_eq!(rust, case.expected, "Rust verdict: {}", case.label);
            let mut arguments = vec![case.kind.to_owned(), hex(&case.bytes)];
            arguments.extend(c.iter().map(u64::to_string));
            match &case.expected {
                None => arguments.push("reject".into()),
                Some(fields) => arguments.extend(fields.iter().map(u64::to_string)),
            }
            let status = Command::new(&executable).args(&arguments).status().unwrap();
            assert_eq!(
                status.code(),
                Some(0),
                "C verdict: {} (sanitize={sanitize})",
                case.label
            );
            checked += 1;
        }
        // The probe can fail: a wrong expectation is a difference.
        let wrong = Command::new(&executable)
            .args(["registry", &hex(&value([7, 0, 0, 0])), "8"])
            .status()
            .unwrap();
        assert_eq!(wrong.code(), Some(1));
    }
    assert_eq!(VALUE_SIZE, 32);
    eprintln!("snapshot record cross-read cases checked: {checked}");
}
