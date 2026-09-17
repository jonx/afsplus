//! Wire image of a directory-tree leaf value.
use afsplus_format::dir::{decode_tree_entry_value, encode_tree_entry_value, DirEntry};
use afsplus_format::FormatError;

fn entry(name: &str) -> DirEntry {
    DirEntry {
        key: name.as_bytes().to_vec(),
        name: name.as_bytes().to_vec(),
        child_type_hint: 2,
        child_id: 0x0102_0304_0506_0708,
    }
}

#[test]
fn the_value_has_the_documented_bytes() {
    let value = encode_tree_entry_value(&entry("Dé")).unwrap();
    assert_eq!(
        value,
        [
            3, 0, // name length
            2, // directory
            0, 0, 0, 0, 0, // reserved
            8, 7, 6, 5, 4, 3, 2, 1, // child object ID
            b'D', 0xc3, 0xa9,
        ]
    );
    assert_eq!(
        decode_tree_entry_value("Dé".as_bytes(), &value),
        Ok(entry("Dé"))
    );
    let longest = "x".repeat(255);
    let value = encode_tree_entry_value(&entry(&longest)).unwrap();
    assert_eq!(value.len(), 16 + 255);
    assert_eq!(
        decode_tree_entry_value(b"k", &value).unwrap().name,
        longest.as_bytes()
    );
}

#[test]
fn malformed_values_are_refused() {
    let clean = encode_tree_entry_value(&entry("name")).unwrap();
    let cases: [(&str, Vec<u8>); 9] = [
        ("truncated", clean[..15].to_vec()),
        ("length below the bytes", {
            let mut v = clean.clone();
            v[0] = 3;
            v
        }),
        ("length above the bytes", {
            let mut v = clean.clone();
            v[0] = 5;
            v
        }),
        ("reserved byte", {
            let mut v = clean.clone();
            v[7] = 1;
            v
        }),
        ("hint zero", {
            let mut v = clean.clone();
            v[2] = 0;
            v
        }),
        ("hint four", {
            let mut v = clean.clone();
            v[2] = 4;
            v
        }),
        ("child zero", {
            let mut v = clean.clone();
            v[8..16].fill(0);
            v
        }),
        ("slash in the name", {
            let mut v = clean.clone();
            v[17] = b'/';
            v
        }),
        ("invalid UTF-8", {
            let mut v = clean.clone();
            v[17] = 0xff;
            v
        }),
    ];
    for (what, value) in cases {
        assert!(decode_tree_entry_value(b"name", &value).is_err(), "{what}");
    }
    for bad in [
        DirEntry {
            child_id: 0,
            ..entry("n")
        },
        DirEntry {
            child_type_hint: 0,
            ..entry("n")
        },
        DirEntry {
            name: Vec::new(),
            ..entry("n")
        },
        DirEntry {
            name: vec![b'x'; 256],
            ..entry("n")
        },
    ] {
        assert!(encode_tree_entry_value(&bad).is_err());
    }
    assert_eq!(
        decode_tree_entry_value(b"k", &[0; 15]),
        Err(FormatError::Invalid("directory leaf value is truncated"))
    );
}
