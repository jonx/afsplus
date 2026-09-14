use afsplus_backup::{
    envelope,
    member::Timestamp,
    pax,
    sparse::{self, Entry, Map, Range},
    tar,
};
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--headers") {
        use std::io::Write;
        for size in [(1u64 << 33) - 1, 1u64 << 33, u64::MAX] {
            let header = tar::Header {
                path: "files/wide".into(),
                link: String::new(),
                kind: tar::Kind::File,
                mode: 0o600,
                uid: 0,
                gid: 0,
                size,
                mtime: 0,
                uname: String::new(),
                gname: String::new(),
            };
            std::io::stdout()
                .write_all(&header.encode().unwrap())
                .unwrap();
        }
        return;
    }
    let mut writer = envelope::Writer::new(
        std::io::stdout().lock(),
        tar::Limits {
            members: 32,
            member_bytes: 65536,
            trailing_zero_blocks: 0,
        },
    )
    .unwrap();
    let records = pax::Limits {
        bytes: 4096,
        records: 16,
        key_bytes: 64,
        value_bytes: 1024,
    };
    let limits = sparse::Limits {
        entries: 128,
        map_bytes: 4096,
        data_bytes: 65536,
        logical_bytes: u64::MAX,
    };
    for (ordinal, path, size, ranges, data) in [
        (
            0,
            "files/mixed",
            1048583,
            vec![
                Range {
                    offset: 4096,
                    length: 5,
                },
                Range {
                    offset: 65536,
                    length: 4096,
                },
                Range {
                    offset: 1048583,
                    length: 0,
                },
            ],
            [b"hello".as_slice(), &[0; 4096]].concat(),
        ),
        (
            1,
            "files/holes",
            64 * 1024 * 1024,
            vec![Range {
                offset: 64 * 1024 * 1024,
                length: 0,
            }],
            vec![],
        ),
        (2, "files/empty", 0, vec![], vec![]),
    ] {
        let map = Map::new(size, &ranges, limits).unwrap();
        sparse::start(
            &mut writer,
            &Entry {
                ordinal,
                path,
                modified: Timestamp {
                    seconds: 42,
                    nanos: 125000000,
                },
            },
            &map,
            records,
        )
        .unwrap();
        writer.write_payload(&data).unwrap();
    }
    let _ = writer.finish().unwrap();
}
