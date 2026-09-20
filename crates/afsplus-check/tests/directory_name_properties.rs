//! Finite executable name-policy and typed-directory caller properties.
//! Literal expected keys deliberately do not use normalization/folding helpers.
//! Unicode conformance-table completeness, per-directory policy overrides,
//! epoch-1 wire freeze, host adapters and child-object ownership are excluded.
use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_core::{
    directory, mkfs, mount_with_options, name_key, CoreError, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::dir::DirEntry;
use afsplus_format::geometry::Geometry;
use afsplus_format::ident::{
    FeatureFlags, Identification, NameKeyAlgorithm, UNICODE_VERSION_16_0_0,
};
use afsplus_format::tree::{child_value, ChildRef, TreeItem, TreeKind, TreeNode};
use afsplus_format::{crc32c::CHECKSUM_CRC32C, Timespec};
use std::num::NonZeroUsize;

const GEO: Geometry = Geometry {
    block_size: 4096,
    total_blocks: 256,
    region_size: 256,
};
const ALGORITHMS: [NameKeyAlgorithm; 3] = [
    NameKeyAlgorithm::LegacyIdentity,
    NameKeyAlgorithm::UnicodeNfc,
    NameKeyAlgorithm::UnicodeNfcCasefold,
];
// (original spelling, sensitive NFC key, insensitive NFC casefold key).
const VECTORS: &[(&str, &str, &str)] = &[
    ("Cafe\u{301}", "Café", "café"),
    ("CAFÉ", "CAFÉ", "café"),
    ("Straße", "Straße", "strasse"),
    ("STRASSE", "STRASSE", "strasse"),
    ("\u{212a}", "K", "k"),
    ("\u{212b}", "Å", "å"),
    ("\u{fb03}", "\u{fb03}", "ffi"),
    ("İ", "İ", "i\u{307}"),
    ("I", "I", "i"),
    ("ı", "ı", "ı"),
    ("Σςσ", "Σςσ", "σσσ"),
    ("\u{1100}\u{1161}\u{11a8}", "각", "각"),
    ("a\u{315}\u{300}", "à\u{315}", "à\u{315}"),
    ("\u{344}", "\u{308}\u{301}", "\u{308}\u{301}"),
    ("ẞ", "ẞ", "ss"),
    ("Ａ", "Ａ", "ａ"),
    ("①", "①", "①"),
    ("file:part\\x", "file:part\\x", "file:part\\x"),
];
fn ident(algorithm: NameKeyAlgorithm) -> Identification {
    Identification {
        uuid: [91; 16],
        block_shift: 12,
        checksum_algorithm: CHECKSUM_CRC32C,
        region_size: 256,
        log_slots: 0,
        features: FeatureFlags::default(),
        name_key_algorithm: algorithm,
        unicode_version: if algorithm == NameKeyAlgorithm::LegacyIdentity {
            [0, 0, 0]
        } else {
            UNICODE_VERSION_16_0_0
        },
        total_blocks: 256,
        checkpoint_slots: [1, 2],
        metadata_start: 9,
        label: "name-properties".into(),
    }
}
fn expected<'a>(
    algorithm: NameKeyAlgorithm,
    (original, sensitive, insensitive): (&'a str, &'a str, &'a str),
) -> &'a str {
    match algorithm {
        NameKeyAlgorithm::LegacyIdentity => original,
        NameKeyAlgorithm::UnicodeNfc => sensitive,
        NameKeyAlgorithm::UnicodeNfcCasefold => insensitive,
    }
}
fn value(entry: &DirEntry) -> Vec<u8> {
    let mut value = vec![0; 16 + entry.name.len()];
    value[..2].copy_from_slice(&(entry.name.len() as u16).to_le_bytes());
    value[2] = entry.child_type_hint;
    value[8..16].copy_from_slice(&entry.child_id.to_le_bytes());
    value[16..].copy_from_slice(&entry.name);
    value
}
fn single(key: Vec<u8>, value: Vec<u8>) -> MemoryBackend {
    let mut dev = MemoryBackend::new(4096, 256);
    let mut node = TreeNode::leaf(TreeKind::Directory, 42);
    node.items.push(TreeItem {
        key: key.into(),
        value: value.into(),
    });
    node.subtree_items = 1;
    dev.apply_raw(16, &node.encode(4096, 7).unwrap());
    dev
}

#[test]
fn literal_unicode_vectors_and_composed_contexts_have_independent_keys() {
    for algorithm in ALGORITHMS {
        let ident = ident(algorithm);
        for &vector in VECTORS {
            let wanted = expected(algorithm, vector);
            for (prefix, suffix) in [("", ""), ("00:", ".txt"), ("x-", "-9")] {
                let original = format!("{prefix}{}{suffix}", vector.0);
                let expected = format!("{prefix}{wanted}{suffix}").into_bytes();
                assert_eq!(
                    name_key::comparison_key(&ident, original.as_bytes()).unwrap(),
                    expected,
                    "{algorithm:?}: {original:?}"
                );
                assert!(
                    name_key::validate_entry_key(&ident, &expected, original.as_bytes()).is_ok()
                );
                for byte in 0..expected.len() {
                    let mut wrong = expected.clone();
                    wrong[byte] ^= 1;
                    assert!(matches!(
                        name_key::validate_entry_key(&ident, &wrong, original.as_bytes()),
                        Err(CoreError::Corrupt(_))
                    ));
                }
            }
        }
    }
}

#[test]
fn ascii_admission_and_utf8_byte_boundaries_are_finite_and_locale_free() {
    for algorithm in ALGORITHMS {
        let ident = ident(algorithm);
        for byte in 0u8..=127 {
            let result = name_key::comparison_key(&ident, &[byte]);
            if byte == 0 || byte == b'/' {
                assert!(matches!(result, Err(CoreError::InvalidName(_))));
            } else {
                let wanted = if algorithm == NameKeyAlgorithm::UnicodeNfcCasefold
                    && byte.is_ascii_uppercase()
                {
                    byte + 32
                } else {
                    byte
                };
                assert_eq!(result.unwrap(), [wanted]);
            }
        }
        for length in 0..=260 {
            let original = vec![b'A'; length];
            let result = name_key::comparison_key(&ident, &original);
            if (1..=255).contains(&length) {
                assert_eq!(
                    result.unwrap(),
                    vec![
                        if algorithm == NameKeyAlgorithm::UnicodeNfcCasefold {
                            b'a'
                        } else {
                            b'A'
                        };
                        length
                    ]
                );
            } else {
                assert!(matches!(result, Err(CoreError::InvalidName(_))));
            }
        }
        for (original, sensitive, folded, admit) in [
            (
                "é".repeat(127) + "x",
                "é".repeat(127) + "x",
                "é".repeat(127) + "x",
                true,
            ),
            ("é".repeat(128), "é".repeat(128), "é".repeat(128), false),
            (
                "ß".repeat(127) + "x",
                "ß".repeat(127) + "x",
                "ss".repeat(127) + "x",
                true,
            ),
            (
                "\u{fb03}".repeat(85),
                "\u{fb03}".repeat(85),
                "ffi".repeat(85),
                true,
            ),
            (
                "😀".repeat(63) + "abc",
                "😀".repeat(63) + "abc",
                "😀".repeat(63) + "abc",
                true,
            ),
            ("😀".repeat(64), "😀".repeat(64), "😀".repeat(64), false),
        ] {
            let result = name_key::comparison_key(&ident, original.as_bytes());
            if admit {
                assert_eq!(
                    result.unwrap(),
                    expected(algorithm, (&original, &sensitive, &folded)).as_bytes()
                );
            } else {
                assert!(result.is_err());
            }
        }
        for bad in invalid_names() {
            assert!(matches!(
                name_key::comparison_key(&ident, &bad),
                Err(CoreError::InvalidName(_))
            ));
        }
    }
}
fn invalid_names() -> Vec<Vec<u8>> {
    vec![
        vec![],
        vec![b'x'; 256],
        b"a/b".to_vec(),
        b"a\0b".to_vec(),
        vec![0x80],
        vec![0xc0, 0xaf],
        vec![0xe0, 0x80, 0xaf],
        vec![0xed, 0xa0, 0x80],
        vec![0xf4, 0x90, 0x80, 0x80],
        vec![0xf5, 0x80, 0x80, 0x80],
        vec![0xc2],
        vec![0xe2, 0x82],
        vec![0xf0, 0x9f, 0x98],
    ]
}

#[test]
fn hand_built_directory_values_preserve_spelling_and_binary_page_order() {
    for algorithm in ALGORITHMS {
        let ident = ident(algorithm);
        // Deduplicate independently known aliases, then sort raw key bytes.
        // Unicode spellings themselves determine cross-leaf order; no numeric
        // prefix masks the difference between byte order and locale collation.
        let mut by_key = std::collections::BTreeMap::new();
        for (index, &vector) in VECTORS.iter().enumerate() {
            let key = expected(algorithm, vector).as_bytes().to_vec();
            by_key.entry(key.clone()).or_insert(DirEntry {
                key,
                name: vector.0.as_bytes().to_vec(),
                child_id: 100 + index as u64,
                child_type_hint: 1 + (index % 3) as u8,
            });
        }
        let entries: Vec<_> = by_key.into_values().collect();
        let split = entries.len() / 2;
        let mut dev = MemoryBackend::new(4096, 256);
        for (lba, chunk) in [(17, &entries[..split]), (18, &entries[split..])] {
            let mut node = TreeNode::leaf(TreeKind::Directory, 42);
            for entry in chunk {
                let wire = value(entry);
                assert_eq!(
                    directory::encode_entry(&ident, entry).unwrap(),
                    (entry.key.clone(), wire.clone())
                );
                node.items.push(TreeItem {
                    key: entry.key.clone().into(),
                    value: wire.into(),
                });
            }
            node.subtree_items = chunk.len() as u64;
            dev.apply_raw(lba, &node.encode(4096, 7).unwrap());
        }
        let root = TreeNode {
            kind: TreeKind::Directory,
            owner: 42,
            level: 1,
            subtree_items: entries.len() as u64,
            leftmost_child: 17,
            leftmost_items: split as u64,
            items: vec![TreeItem {
                key: entries[split].key.clone().into(),
                value: child_value(ChildRef {
                    lba: 18,
                    subtree_items: (entries.len() - split) as u64,
                })
                .unwrap()
                .into(),
            }],
        };
        dev.apply_raw(16, &root.encode(4096, 7).unwrap());
        let mut dev = TraceBackend::new(dev);
        directory::validate_root(&mut dev, &GEO, 16, 42, 7).unwrap();
        assert_eq!(dev.stats().reads, 1);
        dev.reset();
        let loaded = directory::load_all(&mut dev, &GEO, 16, 42, 7, &ident).unwrap();
        assert_eq!(loaded.entries, entries);
        assert_eq!(dev.stats().reads, 3);
        for entry in &entries {
            dev.reset();
            assert_eq!(
                directory::lookup_entry(&mut dev, &GEO, 16, 42, 7, &ident, &entry.key).unwrap(),
                Some(entry.clone())
            );
            assert_eq!(dev.stats().reads, 2);
        }
        for limit in 1..=5 {
            let mut collected = Vec::new();
            while collected.len() < entries.len() {
                dev.reset();
                let (page, total) = directory::read_page(
                    &mut dev,
                    &GEO,
                    16,
                    directory::spec(42, 7),
                    &ident,
                    collected.len() as u64,
                    limit,
                )
                .unwrap();
                assert_eq!(total, entries.len() as u64);
                assert!(!page.is_empty());
                assert!(page.len() <= limit);
                assert!(dev.stats().reads <= 3);
                collected.extend(page);
            }
            assert_eq!(collected, entries);
        }
        let mut visited = Vec::new();
        directory::visit_entries(&mut dev, &GEO, 16, 42, 7, &ident, |entry| {
            visited.push(entry.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(visited, entries);
    }
}

#[test]
fn malformed_typed_payloads_are_rejected_after_structural_root_admission() {
    for algorithm in ALGORITHMS {
        let ident = ident(algorithm);
        let entry = DirEntry {
            key: expected(algorithm, ("Café", "Café", "café"))
                .as_bytes()
                .to_vec(),
            name: "Café".as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: 100,
        };
        let good = value(&entry);
        let mut cases = Vec::new();
        for length in 1..good.len() {
            cases.push((entry.key.clone(), good[..length].to_vec()));
        }
        for extra in 1..=4 {
            let mut bytes = good.clone();
            bytes.extend(vec![0; extra]);
            cases.push((entry.key.clone(), bytes));
        }
        for index in 3..8 {
            let mut bytes = good.clone();
            bytes[index] = 1;
            cases.push((entry.key.clone(), bytes));
        }
        for hint in [0, 4, 255] {
            let mut bytes = good.clone();
            bytes[2] = hint;
            cases.push((entry.key.clone(), bytes));
        }
        let mut bytes = good.clone();
        bytes[8..16].fill(0);
        cases.push((entry.key.clone(), bytes));
        for size in [0u16, 1, 4, 6, 255, u16::MAX] {
            let mut bytes = good.clone();
            bytes[..2].copy_from_slice(&size.to_le_bytes());
            cases.push((entry.key.clone(), bytes));
        }
        for name in invalid_names() {
            let mut bad = entry.clone();
            bad.name = name;
            cases.push((entry.key.clone(), value(&bad)));
        }
        for key in [b"wrong".to_vec(), vec![b'k'; 1020], vec![0x80]] {
            cases.push((key, good.clone()));
        }
        for (key, bytes) in cases {
            let mut dev = TraceBackend::new(single(key.clone(), bytes));
            // Bounded mount root admission deliberately does not decode leaf meaning.
            directory::validate_root(&mut dev, &GEO, 16, 42, 7).unwrap();
            dev.reset();
            assert!(directory::lookup_entry(&mut dev, &GEO, 16, 42, 7, &ident, &key).is_err());
            assert_eq!(dev.stats().reads, 1);
            assert!(directory::load_all(&mut dev, &GEO, 16, 42, 7, &ident).is_err());
            assert!(
                directory::read_page(&mut dev, &GEO, 16, directory::spec(42, 7), &ident, 0, 1)
                    .is_err()
            );
            assert!(
                directory::visit_entries(&mut dev, &GEO, 16, 42, 7, &ident, |_| Ok(())).is_err()
            );
        }
        for key in [vec![], vec![b'k'; 1021]] {
            let mut bad = entry.clone();
            bad.key = key;
            assert!(directory::encode_entry(&ident, &bad).is_err());
        }
    }
}

#[test]
fn mounted_policy_collisions_preserve_bytes_and_refuse_without_writes() {
    for policy in [NamePolicy::Sensitive, NamePolicy::Insensitive] {
        for pages in [2, 4, 8, usize::MAX] {
            let mut dev = MemoryBackend::new(4096, 256);
            mkfs(
                &mut dev,
                &MkfsParams {
                    uuid: [92; 16],
                    label: "policy-properties".into(),
                    region_size: 256,
                    reclaim_caps: Default::default(),
                    log_slots: 0,
                    shared_extents: false,
                    data_policy: false,
                    name_policy: policy,
                    timestamp: Timespec::default(),
                },
            )
            .unwrap();
            let options = MountOptions {
                tree_cache_pages: NonZeroUsize::new(pages),
                ..Default::default()
            };
            let mut volume = mount_with_options(TraceBackend::new(dev), options).unwrap();
            let pairs = [
                ("Cafe\u{301}", "Café", true),
                ("Straße", "STRASSE", policy == NamePolicy::Insensitive),
                ("\u{212a}", "K", true),
                ("I", "ı", false),
                ("\u{fb03}", "ffi", policy == NamePolicy::Insensitive),
            ];
            let mut wanted = Vec::new();
            let mut contents = std::collections::BTreeMap::new();
            for (index, (original, alias, collision)) in pairs.into_iter().enumerate() {
                let name = format!("{index}:{original}");
                let alias = format!("{index}:{alias}");
                let id = volume
                    .create_file_in_root(&name, &[index as u8], Timespec::default())
                    .unwrap();
                wanted.push((name.clone(), id));
                contents.insert(id, vec![index as u8]);
                volume.device_mut().reset();
                let generation = volume.generation();
                let result = volume.create_file_in_root(&alias, b"alias", Timespec::default());
                if collision {
                    assert!(matches!(result, Err(CoreError::AlreadyExists)));
                    assert_eq!(volume.generation(), generation);
                    assert_eq!(volume.device_mut().stats().writes, 0);
                    assert_eq!(volume.device_mut().stats().flushes, 0);
                    assert_eq!(volume.lookup_root(&alias).unwrap(), Some(id));
                } else {
                    let alias_id = result.unwrap();
                    wanted.push((alias, alias_id));
                    contents.insert(alias_id, b"alias".to_vec());
                }
                assert_eq!(volume.read_file(id).unwrap(), [index as u8]);
            }
            for name in ["", "bad/name", "bad\0name"] {
                volume.device_mut().reset();
                let generation = volume.generation();
                assert!(matches!(
                    volume.create_file_in_root(name, b"bad", Timespec::default()),
                    Err(CoreError::InvalidName(_))
                ));
                assert_eq!(volume.generation(), generation);
                assert_eq!(volume.device_mut().stats().writes, 0);
                assert_eq!(volume.device_mut().stats().flushes, 0);
            }
            let listed = volume.list_root().unwrap();
            for expected in &wanted {
                assert!(listed.contains(expected));
            }
            assert_eq!(listed.len(), wanted.len());
            let mut dev = volume.into_device().into_inner();
            assert!(afsplus_check::check_device(&mut dev).is_clean());
            let mut volume = mount_with_options(dev, options).unwrap();
            assert_eq!(volume.list_root().unwrap(), listed);
            for (name, id) in wanted {
                assert_eq!(volume.lookup_root(&name).unwrap(), Some(id));
                assert_eq!(volume.read_file(id).unwrap(), contents[&id]);
            }
        }
    }
}

fn codepoints(field: &str) -> String {
    field
        .split_whitespace()
        .map(|hex| char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap())
        .collect()
}

#[test]
fn unicode_16_normalization_corpus_matches_public_comparison_keys() {
    let identity = ident(NameKeyAlgorithm::UnicodeNfc);
    let mut count = 0;
    for (line_number, line) in include_str!("data/unicode-16.0.0/NormalizationTest.txt")
        .lines()
        .enumerate()
    {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() || line.starts_with('@') {
            continue;
        }
        let columns: Vec<_> = line.split(';').take(5).map(codepoints).collect();
        assert_eq!(columns.len(), 5);
        for (input, expected) in [(0, 1), (1, 1), (2, 1), (3, 3), (4, 3)] {
            let result = name_key::comparison_key(&identity, columns[input].as_bytes());
            if columns[input].contains(['/', '\0']) {
                assert!(
                    matches!(result, Err(CoreError::InvalidName(_))),
                    "normalization line {} reserved name",
                    line_number + 1
                );
            } else {
                assert_eq!(
                    result.unwrap(),
                    columns[expected].as_bytes(),
                    "normalization line {} column {}",
                    line_number + 1,
                    input + 1
                );
            }
        }
        count += 1;
    }
    assert_eq!(count, 19965);
}

#[test]
fn unicode_16_full_default_casefold_table_matches_all_valid_scalar_names() {
    let sensitive = ident(NameKeyAlgorithm::UnicodeNfc);
    let folded = ident(NameKeyAlgorithm::UnicodeNfcCasefold);
    let mut mapping = std::collections::BTreeMap::new();
    for line in include_str!("data/unicode-16.0.0/CaseFolding.txt").lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split(';').map(str::trim).collect();
        if matches!(fields[1], "C" | "F") {
            let scalar = u32::from_str_radix(fields[0], 16).unwrap();
            assert!(mapping.insert(scalar, codepoints(fields[2])).is_none());
        }
    }
    assert!(!mapping.is_empty());
    let mut normalization = std::collections::BTreeMap::new();
    for line in include_str!("data/unicode-16.0.0/NormalizationTest.txt").lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() || line.starts_with('@') {
            continue;
        }
        let columns: Vec<_> = line.split(';').take(2).map(codepoints).collect();
        if columns[0].chars().count() == 1 {
            normalization.insert(columns[0].chars().next().unwrap(), columns[1].clone());
        }
    }
    let mut count = 0;
    for scalar in 1..=0x10ffff {
        let Some(character) = char::from_u32(scalar) else {
            continue;
        };
        if character == '/' {
            continue;
        }
        let input = character.to_string();
        // The corpus requires every scalar absent from its single-character
        // normalization inventory to remain unchanged. Include unassigned
        // scalars too; filesystem spelling admission does not reject them.
        let expected_nfc = normalization.get(&character).unwrap_or(&input);
        assert_eq!(
            name_key::comparison_key(&sensitive, input.as_bytes()).unwrap(),
            expected_nfc.as_bytes(),
            "NFC U+{scalar:04X}"
        );
        let expected_fold = mapping.get(&scalar).unwrap_or(&input);
        // NFC is separately qualified against the complete official corpus.
        // This composition tests full/default folding, including absent mappings,
        // without deriving the fold oracle from the production caseless crate.
        let expected = name_key::comparison_key(&sensitive, expected_fold.as_bytes()).unwrap();
        assert_eq!(
            name_key::comparison_key(&folded, input.as_bytes()).unwrap(),
            expected,
            "casefold U+{scalar:04X}"
        );
        count += 1;
    }
    assert_eq!(count, 1112062);
}
