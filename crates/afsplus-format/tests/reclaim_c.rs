//! Cross-read the reclaim queue blocks (ADR-036) with the independent
//! heap-free C decoders: for every image the C verdict must equal the Rust
//! verdict, and both must equal the literal expectation.
#![cfg(unix)]
use afsplus_format::{
    header::{block_type, BlockHeader, HEADER_SIZE},
    reclaim::{
        ReclaimCaps, ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable, SegmentRef, TableRef,
        SEGMENT_ENTRY_CAP, TABLE_REF_CAP,
    },
    DEFAULT_BLOCK_SIZE,
};
use std::{fs, path::PathBuf, process::Command};

const SIZE: usize = DEFAULT_BLOCK_SIZE;

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
        .arg(repo.join("portable/c/tests/reclaim_probe.c"))
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

fn entry(start: u64, blocks: u32, retire_generation: u64) -> ReclaimEntry {
    ReclaimEntry {
        start,
        blocks,
        retire_generation,
    }
}

fn root() -> ReclaimRoot {
    ReclaimRoot {
        pending_blocks: 17,
        appended_blocks_total: 23,
        reclaimed_blocks_total: 6,
        head_segment_offset: 1,
        head_entry_offset: 1,
        head_block_offset: 2,
        caps: ReclaimCaps {
            inline_entries: 3,
            segment_refs: 3,
            table_refs: 2,
        },
        table_refs: vec![TableRef {
            lba: 222,
            ref_count: 3,
        }],
        segment_refs: vec![
            SegmentRef {
                lba: 333,
                entry_count: 2,
            },
            SegmentRef {
                lba: 444,
                entry_count: 202,
            },
        ],
        inline_entries: vec![entry(123, 3, 5), entry(u64::MAX - 7, 7, 11)],
    }
}

/// Reseal with the header fields given; the payload length is kept unless
/// stated.
fn reseal(block: &mut [u8], kind: u32, payload_len: Option<u32>) {
    let header = BlockHeader {
        block_type: kind,
        flags: u16::from_le_bytes(block[6..8].try_into().unwrap()),
        owner: u64::from_le_bytes(block[8..16].try_into().unwrap()),
        generation: u64::from_le_bytes(block[16..24].try_into().unwrap()),
        payload_len: payload_len
            .unwrap_or_else(|| u32::from_le_bytes(block[24..28].try_into().unwrap())),
    };
    header.seal(block);
}

fn refs_listing(refs: impl Iterator<Item = (u64, u32)>, out: &mut Vec<u8>) {
    for (lba, count) in refs {
        out.extend_from_slice(&lba.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
    }
}

fn entries_listing(entries: &[ReclaimEntry], out: &mut Vec<u8>) {
    for entry in entries {
        out.extend_from_slice(&entry.start.to_le_bytes());
        out.extend_from_slice(&entry.blocks.to_le_bytes());
        out.extend_from_slice(&entry.retire_generation.to_le_bytes());
    }
}

#[test]
fn independent_c_decoders_agree_on_the_reclaim_queue_blocks() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-reclaim-c-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();
    let block_path = scratch.0.join("block");
    let listing_path = scratch.0.join("listing");

    // (label, kind, image, accepted)
    let mut images: Vec<(String, &str, Vec<u8>, bool)> = Vec::new();
    let mut push = |label: &str, kind: &'static str, image: Vec<u8>, accepted: bool| {
        images.push((label.to_owned(), kind, image, accepted));
    };

    // Roots.
    let good = root().encode(SIZE, 17).unwrap();
    push("root", "root", good.clone(), true);
    push(
        "empty root, default capacities",
        "root",
        ReclaimRoot::empty(ReclaimCaps::default())
            .encode(SIZE, 1)
            .unwrap(),
        true,
    );
    let mut segments_only = root();
    segments_only.table_refs.clear();
    segments_only.head_segment_offset = 0;
    push(
        "root without tables",
        "root",
        segments_only.encode(SIZE, 17).unwrap(),
        true,
    );
    // Capacities that fill the block exactly: 64 + 12 * (100 + 100) + 20 * 80.
    let mut full = ReclaimRoot::empty(ReclaimCaps {
        inline_entries: 80,
        segment_refs: 100,
        table_refs: 100,
    });
    full.inline_entries = (0..80).map(|i| entry(1000 + i, 1, 9)).collect();
    full.appended_blocks_total = 80;
    full.pending_blocks = 80;
    push(
        "root at capacity",
        "root",
        full.encode(SIZE, 17).unwrap(),
        true,
    );
    let p = HEADER_SIZE;
    let root_cases: [(&str, usize, &[u8]); 16] = [
        ("root version 2", 0, &[2]),
        ("root reserved word", 4, &[1]),
        ("root reserved bytes", 50, &[1]),
        ("root zero inline capacity", 44, &[0, 0]),
        ("root zero segment capacity", 46, &[0, 0]),
        ("root zero table capacity", 48, &[0, 0]),
        ("root capacities beyond the block", 44, &[0xff, 0xff]),
        ("root table count above capacity", 52, &[3]),
        ("root segment count above capacity", 56, &[4]),
        ("root inline count above capacity", 60, &[4]),
        ("root cursor beyond the first table", 32, &[3]),
        ("root totals that do not add up", 8, &[18]),
        ("root reclaimed above appended", 24, &[24]),
        ("root table ref with zero count", 64 + 8, &[0]),
        ("root segment ref above the cap", 64 + 24 + 8, &[203]),
        ("root entry with zero blocks", 64 + 24 + 36 + 8, &[0]),
    ];
    for (label, offset, bytes) in root_cases {
        let mut block = good.clone();
        block[p + offset..p + offset + bytes.len()].copy_from_slice(bytes);
        reseal(&mut block, block_type::RECLAIM_ROOT, None);
        push(label, "root", block, false);
    }
    let mut overflow = good.clone();
    let at = p + 64 + 24 + 36 + 20;
    overflow[at + 8] = 8; // u64::MAX - 7 + 8 overflows
    reseal(&mut overflow, block_type::RECLAIM_ROOT, None);
    push("root entry whose end overflows", "root", overflow, false);
    let mut zero_generation = good.clone();
    zero_generation[p + 64 + 24 + 36 + 12..p + 64 + 24 + 36 + 20].fill(0);
    reseal(&mut zero_generation, block_type::RECLAIM_ROOT, None);
    push(
        "root entry with zero generation",
        "root",
        zero_generation,
        false,
    );
    let mut short = good.clone();
    reseal(
        &mut short,
        block_type::RECLAIM_ROOT,
        Some(64 + 24 + 36 + 59),
    );
    push("root payload shorter than its areas", "root", short, false);
    let mut cursor = segments_only.encode(SIZE, 17).unwrap();
    cursor[p + 36] = 2;
    reseal(&mut cursor, block_type::RECLAIM_ROOT, None);
    push("root cursor beyond the head segment", "root", cursor, false);
    let mut missing = ReclaimRoot::empty(ReclaimCaps::default())
        .encode(SIZE, 1)
        .unwrap();
    missing[p + 40] = 1;
    reseal(&mut missing, block_type::RECLAIM_ROOT, None);
    push("root cursor without a segment", "root", missing, false);

    // Segments and tables.
    let segment = ReclaimSegment {
        entries: vec![entry(123, 3, 5), entry(900, 7, 11)],
    };
    let table = ReclaimTable {
        refs: vec![
            SegmentRef {
                lba: 333,
                entry_count: 2,
            },
            SegmentRef {
                lba: 444,
                entry_count: 202,
            },
        ],
    };
    let good_segment = segment.encode(SIZE, 17).unwrap();
    let good_table = table.encode(SIZE, 17).unwrap();
    push("segment", "segment", good_segment.clone(), true);
    push("table", "table", good_table.clone(), true);
    push(
        "full segment",
        "segment",
        ReclaimSegment {
            entries: (0..SEGMENT_ENTRY_CAP as u64)
                .map(|i| entry(i + 1, 1, 3))
                .collect(),
        }
        .encode(SIZE, 17)
        .unwrap(),
        true,
    );
    push(
        "full table",
        "table",
        ReclaimTable {
            refs: (0..TABLE_REF_CAP as u64)
                .map(|i| SegmentRef {
                    lba: i + 1,
                    entry_count: 1,
                })
                .collect(),
        }
        .encode(SIZE, 17)
        .unwrap(),
        true,
    );
    // A block of one kind is not a block of the other.
    push(
        "table read as a segment",
        "segment",
        good_table.clone(),
        false,
    );
    push(
        "segment read as a table",
        "table",
        good_segment.clone(),
        false,
    );
    for (kind, magic, good, item) in [
        ("segment", block_type::RECLAIM_SEGMENT, &good_segment, 20u32),
        ("table", block_type::RECLAIM_TABLE, &good_table, 12),
    ] {
        let sealed_cases: [(&str, usize, u8, Option<u32>); 6] = [
            ("zero count", 0, 0, None),
            ("count above the payload", 0, 3, None),
            ("reserved word", 4, 1, None),
            (
                "payload longer than the count",
                0,
                2,
                Some(8 + 2 * item + 1),
            ),
            ("payload shorter than the count", 0, 2, Some(8 + item)),
            ("payload below the fixed part", 0, 2, Some(7)),
        ];
        for (label, offset, value, payload) in sealed_cases {
            let mut block = good.clone();
            block[p + offset] = value;
            reseal(&mut block, magic, payload);
            push(&format!("{kind} {label}"), kind, block, false);
        }
        // A count above the cap cannot be built with a matching payload:
        // the caps are what one 4 KiB block holds. It is refused as a count
        // above the payload.
        let mut above = good.clone();
        above[p..p + 4]
            .copy_from_slice(&(if kind == "segment" { 203u32 } else { 339 }).to_le_bytes());
        reseal(&mut above, magic, None);
        push(&format!("{kind} count above the cap"), kind, above, false);
    }
    let mut zero_blocks = good_segment.clone();
    zero_blocks[p + 8 + 8..p + 8 + 12].fill(0);
    reseal(&mut zero_blocks, block_type::RECLAIM_SEGMENT, None);
    push(
        "segment entry with zero blocks",
        "segment",
        zero_blocks,
        false,
    );
    let mut zero_ref = good_table.clone();
    zero_ref[p + 8 + 8..p + 8 + 12].fill(0);
    reseal(&mut zero_ref, block_type::RECLAIM_TABLE, None);
    push("table ref with zero count", "table", zero_ref, false);
    let mut big_ref = good_table.clone();
    big_ref[p + 8 + 8] = 203;
    reseal(&mut big_ref, block_type::RECLAIM_TABLE, None);
    push("table ref above the cap", "table", big_ref, false);
    let mut broken = good_segment.clone();
    broken[p + 9] ^= 1;
    push("segment with a broken checksum", "segment", broken, false);

    // Exact admission (ADR-110): common-header flags, an owner, bytes in the
    // unused slots of a root area, a root payload longer than its areas and
    // bytes after a payload are all refused, by both readers.
    for (kind, magic, good) in [
        ("root", block_type::RECLAIM_ROOT, &good),
        ("segment", block_type::RECLAIM_SEGMENT, &good_segment),
        ("table", block_type::RECLAIM_TABLE, &good_table),
    ] {
        let mut flags = good.clone();
        flags[6] = 1;
        reseal(&mut flags, magic, None);
        push(&format!("{kind} with header flags"), kind, flags, false);
        let mut owner = good.clone();
        owner[8] = 5;
        reseal(&mut owner, magic, None);
        push(&format!("{kind} with an owner"), kind, owner, false);
        let mut tail = good.clone();
        tail[SIZE - 1] = 1;
        reseal(&mut tail, magic, None);
        push(&format!("{kind} with a nonzero tail"), kind, tail, false);
    }
    // One used item less than the capacity in each area: tables at 64,
    // segments at 88, inline entries at 124.
    for (area, offset) in [
        ("table", 64 + 12),
        ("segment", 88 + 24),
        ("inline", 124 + 40),
    ] {
        let mut unused_slot = good.clone();
        unused_slot[p + offset] = 0xee;
        reseal(&mut unused_slot, block_type::RECLAIM_ROOT, None);
        push(
            &format!("root with bytes in an unused {area} slot"),
            "root",
            unused_slot,
            false,
        );
    }
    let mut longer = good.clone();
    reseal(
        &mut longer,
        block_type::RECLAIM_ROOT,
        Some(64 + 24 + 36 + 60 + 9),
    );
    push("root payload longer than its areas", "root", longer, false);

    let mut checked = 0;
    for sanitize in [false, true] {
        let executable = compile(&scratch, sanitize);
        for (label, kind, image, accepted) in &images {
            let mut arguments = vec![(*kind).to_owned(), block_path.to_str().unwrap().to_owned()];
            let mut listing = Vec::new();
            let rust_accepts = match *kind {
                "root" => match ReclaimRoot::decode(image) {
                    Ok((root, generation)) => {
                        arguments.extend(
                            [
                                generation,
                                root.pending_blocks,
                                root.appended_blocks_total,
                                root.reclaimed_blocks_total,
                                root.head_segment_offset.into(),
                                root.head_entry_offset.into(),
                                root.head_block_offset.into(),
                                root.caps.inline_entries.into(),
                                root.caps.segment_refs.into(),
                                root.caps.table_refs.into(),
                                root.table_refs.len() as u64,
                                root.segment_refs.len() as u64,
                                root.inline_entries.len() as u64,
                            ]
                            .map(|value| value.to_string()),
                        );
                        refs_listing(
                            root.table_refs.iter().map(|r| (r.lba, r.ref_count)),
                            &mut listing,
                        );
                        refs_listing(
                            root.segment_refs.iter().map(|r| (r.lba, r.entry_count)),
                            &mut listing,
                        );
                        entries_listing(&root.inline_entries, &mut listing);
                        true
                    }
                    Err(_) => false,
                },
                "segment" => match ReclaimSegment::decode(image) {
                    Ok((segment, generation)) => {
                        arguments
                            .extend([generation.to_string(), segment.entries.len().to_string()]);
                        entries_listing(&segment.entries, &mut listing);
                        true
                    }
                    Err(_) => false,
                },
                _ => match ReclaimTable::decode(image) {
                    Ok((table, generation)) => {
                        arguments.extend([generation.to_string(), table.refs.len().to_string()]);
                        refs_listing(
                            table.refs.iter().map(|r| (r.lba, r.entry_count)),
                            &mut listing,
                        );
                        true
                    }
                    Err(_) => false,
                },
            };
            assert_eq!(rust_accepts, *accepted, "Rust verdict: {label}");
            if rust_accepts {
                fs::write(&listing_path, &listing).unwrap();
                arguments.push(listing_path.to_str().unwrap().to_owned());
            } else {
                arguments.push("reject".into());
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
        // The probe can fail: a wrong expectation is a difference.
        fs::write(&block_path, &images[0].2).unwrap();
        fs::write(&listing_path, b"").unwrap();
        let wrong = Command::new(&executable)
            .args(["segment", block_path.to_str().unwrap(), "17", "2"])
            .arg(&listing_path)
            .status()
            .unwrap();
        assert_eq!(wrong.code(), Some(1));
    }
    eprintln!("reclaim cross-read images checked: {checked}");
}
