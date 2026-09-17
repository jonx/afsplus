//! Cross-read extended attributes (ADR-108) with the independent heap-free C
//! codec: for every image the C verdict must equal the Rust verdict, and both
//! must equal the literal expectation.
#![cfg(unix)]
use afsplus_format::{
    attrs::{decode_attribute_set, encode_attribute_set, ATTRIBUTE_CHAIN},
    chain::ChainSegment,
    header::{block_type, BlockHeader, HEADER_SIZE},
    object::{AttributeRef, Comment, ObjectRecord, ObjectType, SecurityRef, SymlinkRecord},
    security::SECURITY_CHAIN,
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
        attributes: None,
        comment: Comment::EMPTY,
    }
}

fn set_reference() -> AttributeRef {
    AttributeRef {
        first_block: 0x0102_0304_0506_0708,
        total_len: 9000,
        segment_count: 3,
    }
}

fn security_reference() -> SecurityRef {
    SecurityRef {
        first_block: 77,
        total_len: 5000,
        segment_count: 2,
        flags: 1,
    }
}

fn reseal(block: &mut [u8], kind: u32, payload_len: u32) {
    BlockHeader {
        block_type: kind,
        flags: 0,
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
        .arg(repo.join("portable/c/tests/attribute_probe.c"))
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

/// Image of a set written by hand, so the refused ones can be built.
fn set_image(entries: &[(&[u8], &[u8])], count: u16) -> Vec<u8> {
    let mut out = count.to_le_bytes().to_vec();
    out.extend_from_slice(&[0, 0]);
    for (name, value) in entries {
        out.push(name.len() as u8);
        out.push(0);
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(value);
    }
    out
}

#[test]
fn independent_c_codec_agrees_on_extended_attributes() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-attributes-c-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();
    let block_path = scratch.0.join("block");
    let bytes_path = scratch.0.join("bytes");
    let size = DEFAULT_BLOCK_SIZE;

    // References: None = reject, Some(None) = no reference.
    let mut references: Vec<(String, Vec<u8>, Option<Option<AttributeRef>>)> = Vec::new();
    // Records with every combination of the three optional fields:
    // (image, security first block, attribute first block, comment size).
    let mut fields: Vec<(String, Vec<u8>, u64, u64, usize)> = Vec::new();
    for kind in [ObjectType::File, ObjectType::Directory] {
        let plain = record(kind).encode(size, 7).unwrap();
        references.push((format!("{kind:?} plain"), plain.clone(), Some(None)));
        let with = record(kind).with_attributes(Some(set_reference()));
        let good = with.encode(size, 7).unwrap();
        references.push((
            format!("{kind:?} attributes"),
            good.clone(),
            Some(Some(set_reference())),
        ));
        for mask in 0..8u8 {
            let mut r = record(kind);
            if mask & 1 != 0 {
                r = r.with_security(Some(security_reference()));
            }
            if mask & 2 != 0 {
                r = r.with_attributes(Some(set_reference()));
            }
            if mask & 4 != 0 {
                r = r.with_comment(Comment::new("Dé note").unwrap());
            }
            fields.push((
                format!("{kind:?} fields {mask}"),
                r.encode(size, 7).unwrap(),
                if mask & 1 != 0 { 77 } else { 0 },
                if mask & 2 != 0 {
                    set_reference().first_block
                } else {
                    0
                },
                if mask & 4 != 0 { 8 } else { 0 },
            ));
        }
        let mut flag_only = plain.clone();
        flag_only[HEADER_SIZE + 10] = 0x10;
        reseal(&mut flag_only, block_type::OBJECT, 96);
        references.push((format!("{kind:?} flag without reference"), flag_only, None));
        let mut unflagged = good.clone();
        unflagged[HEADER_SIZE + 10] = 0;
        reseal(&mut unflagged, block_type::OBJECT, 112);
        references.push((format!("{kind:?} reference without flag"), unflagged, None));
        let mut next_bit = plain.clone();
        next_bit[HEADER_SIZE + 10] = 0x20;
        reseal(&mut next_bit, block_type::OBJECT, 96);
        references.push((format!("{kind:?} unassigned flag bit 5"), next_bit, None));
        for (what, first, total, count, reserved) in [
            ("zero first block", 0u64, 9000u32, 3u16, 0u16),
            ("zero length", 9, 0, 0, 0),
            ("length above the bound", 9, 65_537, 17, 0),
            ("wrong segment count", 9, 9000, 2, 0),
            ("nonzero reserved", 9, 9000, 3, 1),
        ] {
            let mut block = good.clone();
            let at = HEADER_SIZE + 96;
            block[at..at + 8].copy_from_slice(&first.to_le_bytes());
            block[at + 8..at + 12].copy_from_slice(&total.to_le_bytes());
            block[at + 12..at + 14].copy_from_slice(&count.to_le_bytes());
            block[at + 14..at + 16].copy_from_slice(&reserved.to_le_bytes());
            reseal(&mut block, block_type::OBJECT, 112);
            references.push((format!("{kind:?} {what}"), block, None));
        }
        // The largest legal set: 65,536 bytes in 17 segments.
        let largest = AttributeRef {
            first_block: 9,
            total_len: 65_536,
            segment_count: 17,
        };
        references.push((
            format!("{kind:?} largest set"),
            record(kind)
                .with_attributes(Some(largest))
                .encode(size, 7)
                .unwrap(),
            Some(Some(largest)),
        ));
    }
    let mut link = record(ObjectType::Symlink).with_attributes(Some(set_reference()));
    link.size_bytes = 6;
    let symlink = SymlinkRecord {
        record: link,
        target: "target",
    }
    .encode(size, 7)
    .unwrap();
    references.push((
        "symlink attributes".into(),
        symlink,
        Some(Some(set_reference())),
    ));

    // Segments: None = reject.
    let content: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();
    let capacity = size - HEADER_SIZE - 24;
    let mut segments: Vec<(String, Vec<u8>, Option<ChainSegment>)> = Vec::new();
    for (index, chunk) in content.chunks(capacity).enumerate() {
        let segment = ChainSegment {
            object_id: 16,
            format: 1,
            version: 0,
            total_len: 9000,
            index: index as u16,
            count: 3,
            next: if index == 2 { 0 } else { 500 + index as u64 },
            bytes: chunk,
        };
        let block = segment.encode(&ATTRIBUTE_CHAIN, size, 7).unwrap();
        if index == 2 {
            let payload = (24 + chunk.len()) as u32;
            let mut reserved = block.clone();
            reserved[HEADER_SIZE + 6] = 1;
            reseal(&mut reserved, block_type::ATTRIBUTE_SET, payload);
            segments.push(("segment reserved".into(), reserved, None));
            let mut tail = block.clone();
            tail[size - 1] = 1;
            reseal(&mut tail, block_type::ATTRIBUTE_SET, payload);
            segments.push(("segment tail".into(), tail, None));
            let mut longer = block.clone();
            reseal(&mut longer, block_type::ATTRIBUTE_SET, payload + 1);
            segments.push(("segment length not exact".into(), longer, None));
            let mut zero_format = block.clone();
            zero_format[HEADER_SIZE..HEADER_SIZE + 4].fill(0);
            reseal(&mut zero_format, block_type::ATTRIBUTE_SET, payload);
            segments.push(("segment format zero".into(), zero_format, None));
            let mut linked = block.clone();
            linked[HEADER_SIZE + 16] = 9;
            reseal(&mut linked, block_type::ATTRIBUTE_SET, payload);
            segments.push(("last segment with a next".into(), linked, None));
            // The same segment under the security magic is not an attribute
            // segment.
            let foreign = segment.encode(&SECURITY_CHAIN, size, 7).unwrap();
            segments.push(("security segment".into(), foreign, None));
        }
        segments.push((format!("segment {index}"), block, Some(segment)));
    }

    // Sets: None = reject.
    let small: [(&str, &[u8]); 3] = [
        ("aros.icon", b"\x01\x00\x02"),
        ("user.empty", b""),
        ("user.note", b"hello"),
    ];
    let big_value = vec![0x5au8; 65_000];
    let big: [(&str, &[u8]); 2] = [("system.big", &big_value), ("user.z", b"z")];
    let long_name = format!("user.{}", "n".repeat(250));
    let prefix_order: [(&str, &[u8]); 2] = [("user.a", b"1"), ("user.ab", b"2")];
    let mut sets: Vec<(String, Vec<u8>, bool)> = vec![
        ("small".into(), encode_attribute_set(&small).unwrap(), true),
        ("big".into(), encode_attribute_set(&big).unwrap(), true),
        (
            "longest name".into(),
            encode_attribute_set(&[(&long_name, b"v")]).unwrap(),
            true,
        ),
        (
            "prefix sorts first".into(),
            encode_attribute_set(&prefix_order).unwrap(),
            true,
        ),
    ];
    let refused: [(&str, Vec<u8>); 12] = [
        (
            "unsorted",
            set_image(&[(b"user.b", b"1"), (b"user.a", b"2")], 2),
        ),
        (
            "duplicate",
            set_image(&[(b"user.a", b"1"), (b"user.a", b"2")], 2),
        ),
        (
            "prefix after",
            set_image(&[(b"user.ab", b"1"), (b"user.a", b"2")], 2),
        ),
        ("empty", set_image(&[], 0)),
        ("count above", set_image(&[(b"user.a", b"1")], 2)),
        (
            "count below",
            set_image(&[(b"user.a", b"1"), (b"user.b", b"1")], 1),
        ),
        ("unknown namespace", set_image(&[(b"other.a", b"1")], 1)),
        ("namespace alone", set_image(&[(b"user.", b"1")], 1)),
        ("NUL in name", set_image(&[(b"user.a\0", b"1")], 1)),
        ("name not UTF-8", set_image(&[(b"user.\xff", b"1")], 1)),
        ("empty name", set_image(&[(b"", b"1")], 1)),
        ("too short", vec![1, 0, 0]),
    ];
    for (what, image) in refused {
        sets.push((what.into(), image, false));
    }
    let mut reserved = set_image(&[(b"user.a", b"1")], 1);
    reserved[3] = 1;
    sets.push(("set reserved".into(), reserved, false));
    let mut entry_reserved = set_image(&[(b"user.a", b"1")], 1);
    entry_reserved[5] = 1;
    sets.push(("entry reserved".into(), entry_reserved, false));
    let mut trailing = set_image(&[(b"user.a", b"1")], 1);
    trailing.push(0);
    sets.push(("trailing byte".into(), trailing, false));
    let mut above = set_image(&[(b"user.a", &big_value), (b"user.b", &[0u8; 600])], 2);
    assert!(above.len() > 65_536);
    sets.push(("above the bound".into(), std::mem::take(&mut above), false));

    let mut checked = 0;
    for sanitize in [false, true] {
        let executable = compile(&scratch, sanitize);
        let run = |arguments: Vec<String>, label: &str| {
            let status = Command::new(&executable).args(&arguments).status().unwrap();
            assert_eq!(status.code(), Some(0), "{label} (sanitize={sanitize})");
        };
        let path = |p: &PathBuf| p.to_str().unwrap().to_owned();
        for (label, block, expected) in &references {
            let rust = if block[HEADER_SIZE + 8] == 3 {
                SymlinkRecord::decode(block).map(|(s, _)| s.record.attributes)
            } else {
                ObjectRecord::decode(block).map(|r| r.attributes)
            };
            assert_eq!(rust.ok(), *expected, "{label}");
            fs::write(&block_path, block).unwrap();
            let mut arguments = vec!["ref".to_owned(), path(&block_path)];
            match expected {
                None => arguments.push("reject".into()),
                Some(None) => arguments.push("none".into()),
                Some(Some(r)) => arguments.extend([
                    r.first_block.to_string(),
                    r.total_len.to_string(),
                    r.segment_count.to_string(),
                ]),
            }
            run(arguments, label);
            checked += 1;
        }
        for (label, block, security, attributes, comment) in &fields {
            let rust = ObjectRecord::decode(block).unwrap();
            assert_eq!(rust.security.map_or(0, |r| r.first_block), *security);
            assert_eq!(rust.attributes.map_or(0, |r| r.first_block), *attributes);
            assert_eq!(rust.comment.as_str().len(), *comment);
            fs::write(&block_path, block).unwrap();
            run(
                vec![
                    "fields".into(),
                    path(&block_path),
                    security.to_string(),
                    attributes.to_string(),
                    comment.to_string(),
                ],
                label,
            );
            checked += 1;
        }
        for (label, block, expected) in &segments {
            let rust = ChainSegment::decode(&ATTRIBUTE_CHAIN, block).ok();
            assert_eq!(rust.map(|(s, _)| s), *expected, "{label}");
            fs::write(&block_path, block).unwrap();
            let mut arguments = vec!["seg".to_owned(), path(&block_path)];
            match expected {
                None => arguments.push("reject".into()),
                Some(s) => {
                    fs::write(&bytes_path, s.bytes).unwrap();
                    arguments.extend([
                        s.object_id.to_string(),
                        s.format.to_string(),
                        s.version.to_string(),
                        s.total_len.to_string(),
                        s.index.to_string(),
                        s.count.to_string(),
                        s.next.to_string(),
                        "7".into(),
                        path(&bytes_path),
                    ]);
                }
            }
            run(arguments, label);
            checked += 1;
        }
        for (label, image, accepted) in &sets {
            let rust = decode_attribute_set(image);
            assert_eq!(rust.is_ok(), *accepted, "{label}");
            fs::write(&block_path, image).unwrap();
            let mut arguments = vec!["set".to_owned(), path(&block_path)];
            match rust {
                Err(_) => arguments.push("reject".into()),
                Ok(entries) => {
                    let mut listing = Vec::new();
                    for (name, value) in &entries {
                        listing.extend_from_slice(&(name.len() as u16).to_le_bytes());
                        listing.extend_from_slice(&(value.len() as u16).to_le_bytes());
                        listing.extend_from_slice(name.as_bytes());
                        listing.extend_from_slice(value);
                    }
                    fs::write(&bytes_path, listing).unwrap();
                    arguments.extend([entries.len().to_string(), path(&bytes_path)]);
                }
            }
            run(arguments, label);
            checked += 1;
        }
        // The probe can fail: a wrong expectation is a difference.
        fs::write(&block_path, &references[1].1).unwrap();
        let wrong = Command::new(&executable)
            .args(["ref", &path(&block_path), "1", "9000", "3"])
            .status()
            .unwrap();
        assert_eq!(wrong.code(), Some(1));
    }
    eprintln!("attribute cross-read images checked: {checked}");
}
