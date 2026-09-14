use super::*;
use std::io::Cursor;
fn limits() -> tar::Limits {
    tar::Limits {
        members: 8,
        member_bytes: 4096,
        trailing_zero_blocks: 0,
    }
}
fn header() -> tar::Header {
    tar::Header {
        path: "files/file".into(),
        link: String::new(),
        kind: tar::Kind::File,
        mode: 0o644,
        uid: 0,
        gid: 0,
        size: 5,
        mtime: 0,
        uname: String::new(),
        gname: String::new(),
    }
}
fn fixture() -> (Vec<u8>, Receipt) {
    let mut writer = Writer::new(Vec::new(), limits()).unwrap();
    writer.start(&header(), None).unwrap();
    writer.write_payload(b"he").unwrap();
    writer.write_payload(b"llo").unwrap();
    writer.finish().unwrap()
}
fn verify(bytes: &[u8]) -> Result<Receipt, Error> {
    let mut reader = Reader::new(Cursor::new(bytes), limits())?;
    assert!(reader.receipt().is_none());
    while let Some(header) = reader.next_header()? {
        reader.begin_payload(None)?;
        let mut left = header.size;
        while left != 0 {
            let mut buf = [0; 3];
            let count = reader.read_payload(&mut buf)?;
            left -= count as u64;
        }
        assert!(reader.receipt().is_none());
    }
    reader
        .receipt()
        .cloned()
        .ok_or(Error::Invalid("no receipt"))
}
#[test]
fn exact_stream_receipt_and_empty_body() {
    let (bytes, receipt) = fixture();
    assert_eq!(verify(&bytes).unwrap(), receipt);
    assert_eq!(receipt.bytes, 2048);
    assert_eq!(receipt.members, 1);
    let (bytes, receipt) = Writer::new(Vec::new(), limits()).unwrap().finish().unwrap();
    assert_eq!(verify(&bytes).unwrap(), receipt);
    assert_eq!(receipt.members, 0);
}
#[test]
fn every_truncated_prefix_and_changed_body_fails_completion() {
    let (bytes, receipt) = fixture();
    for end in 0..bytes.len() {
        assert!(verify(&bytes[..end]).is_err(), "accepted {end}");
    }
    for at in [0, 512, 1024, 1536, receipt.bytes as usize + 512] {
        let mut bad = bytes.clone();
        bad[at] ^= 1;
        assert!(verify(&bad).is_err(), "accepted change at {at}");
    }
    // A syntactically valid ordinary tar prefix with complete end markers is insufficient.
    let mut missing = bytes[..receipt.bytes as usize].to_vec();
    missing.extend([0; 1024]);
    assert!(verify(&missing).is_err());
}
#[test]
fn forged_counts_duplicate_controls_and_trailing_members_fail() {
    let (bytes, receipt) = fixture();
    let end = receipt.bytes as usize;
    for (old, new) in [
        (
            b"AROS.backup.members=1".as_slice(),
            b"AROS.backup.members=2".as_slice(),
        ),
        (b"AROS.backup.bytes=2048", b"AROS.backup.bytes=2049"),
    ] {
        let mut bad = bytes.clone();
        let at = bad[end..]
            .windows(old.len())
            .position(|w| w == old)
            .unwrap()
            + end;
        bad[at..at + old.len()].copy_from_slice(new);
        assert!(verify(&bad).is_err());
    }
    let mut duplicate = bytes[..end + 1024].to_vec();
    duplicate.extend_from_slice(&bytes[end..]);
    assert!(verify(&duplicate).is_err());
    let mut trailing = bytes[..bytes.len() - 1024].to_vec();
    trailing.extend_from_slice(&header().encode().unwrap());
    trailing.extend([0; 1024]);
    assert!(verify(&trailing).is_err());
}
#[test]
fn controls_versions_and_member_limits_are_strict() {
    let (bytes, _) = fixture();
    for (old, new) in [
        (
            b"AROS.backup.envelope=2".as_slice(),
            b"AROS.backup.envelope=3".as_slice(),
        ),
        (b"sha512-256", b"sha256-xxx"),
    ] {
        let mut bad = bytes.clone();
        let at = bad.windows(old.len()).position(|w| w == old).unwrap();
        bad[at..at + old.len()].copy_from_slice(new);
        assert!(verify(&bad).is_err());
    }
    let mut reader = Reader::new(
        Cursor::new(&bytes),
        tar::Limits {
            member_bytes: 4,
            ..limits()
        },
    )
    .unwrap();
    reader.next_header().unwrap();
    assert!(matches!(reader.begin_payload(None), Err(Error::Limit)));
    assert!(matches!(reader.begin_payload(Some(5)), Err(Error::Limit)));
    let mut reader = Reader::new(
        Cursor::new(&bytes),
        tar::Limits {
            members: 0,
            ..limits()
        },
    )
    .unwrap();
    assert!(matches!(reader.next_header(), Err(Error::Limit)));
    assert!(reader.receipt().is_none());
    let mut writer = Writer::new(Vec::new(), limits()).unwrap();
    assert!(writer.start(&control_header(END, 0), None).is_err());
    writer.start(&header(), None).unwrap();
    assert!(writer.finish().is_err());
}
#[test]
fn hash_stream_known_answer_and_length_admission() {
    // SHA-512/256 abc known answer, independently checked with OpenSSL 3.
    let mut io = HashIo::new(Vec::new());
    io.write_all(b"a").unwrap();
    io.write_all(b"bc").unwrap();
    assert_eq!(
        hex(&io.receipt(0).hash),
        "53048e2681941ef99b2e29b76b4c7dabe4c2d0c634fc6d46e0e2f13107e7af23"
    );
    io.bytes = MAX_BYTES;
    assert!(io.write_all(b"x").is_err());
    assert_eq!(io.inner, b"abc");
    for invalid in [
        "",
        "00",
        "+1",
        "-1",
        "1 ",
        "340282366920938463463374607431768211456",
    ] {
        assert!(decimal(invalid).is_err());
    }
}
struct FailFlush(Vec<u8>);
impl Write for FailFlush {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("flush failed"))
    }
}
#[test]
fn output_flush_failure_and_invalid_terminal_never_issue_receipts() {
    let writer = Writer::new(FailFlush(Vec::new()), limits()).unwrap();
    assert!(writer.finish().is_err());
    let (mut bytes, receipt) = fixture();
    bytes[receipt.bytes as usize + 512] ^= 1;
    let mut reader = Reader::new(Cursor::new(bytes), limits()).unwrap();
    reader.next_header().unwrap();
    reader.begin_payload(None).unwrap();
    reader.read_payload(&mut [0; 5]).unwrap();
    assert!(reader.next_header().is_err());
    assert!(reader.receipt().is_none());
    assert!(matches!(reader.next_header(), Err(Error::Poisoned)));
}

#[test]
fn body_namespaces_preserve_control_separation_before_payload_io() {
    let mut writer = Writer::new(Vec::new(), limits()).unwrap();
    let before = writer.tar.stream().inner.len();
    for path in [
        "file",
        "/files/x",
        "files/../_AROS_BACKUP/complete.pax",
        "files//x",
        "files/./x",
        END,
        BEGIN,
        "files/x/",
    ] {
        let mut bad = header();
        bad.path = path.into();
        for kind in [tar::Kind::File, tar::Kind::PaxLocal] {
            bad.kind = kind;
            assert!(
                writer.start(&bad, None).is_err(),
                "accepted {path} as {kind:?}"
            );
            assert_eq!(writer.tar.stream().inner.len(), before);
        }
    }
    let mut link = header();
    link.kind = tar::Kind::HardLink;
    link.size = 0;
    link.link = END.into();
    assert!(writer.start(&link, None).is_err());
    for path in [
        "files/_AROS_BACKUP/complete.pax",
        "_AROS_BACKUP/metadata/object-1",
    ] {
        let mut item = header();
        item.path = path.into();
        writer.start(&item, None).unwrap();
        writer.write_payload(b"hello").unwrap();
    }
    let (bytes, receipt) = writer.finish().unwrap();
    assert_eq!(verify(&bytes).unwrap(), receipt);
    let (mut bytes, _) = fixture();
    let mut bad = header();
    bad.path = "files/../escape".into();
    bytes[1024..1536].copy_from_slice(&bad.encode().unwrap());
    let mut reader = Reader::new(Cursor::new(bytes), limits()).unwrap();
    assert!(matches!(
        reader.next_header(),
        Err(Error::Invalid("invalid body namespace or control"))
    ));
    assert!(reader.receipt().is_none());
}
