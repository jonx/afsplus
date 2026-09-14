use super::*;
use std::io::Cursor;
fn limits() -> Limits {
    Limits {
        members: 10,
        member_bytes: u64::MAX,
        trailing_zero_blocks: 20,
    }
}
fn header() -> Header {
    Header {
        path: "folder/payload".into(),
        link: String::new(),
        kind: Kind::File,
        mode: 0o640,
        uid: 12,
        gid: 34,
        size: 5,
        mtime: 123,
        uname: "user".into(),
        gname: "group".into(),
    }
}
fn archive() -> Vec<u8> {
    let mut writer = Writer::new(Vec::new(), limits());
    writer.start(&header(), None).unwrap();
    writer.write_payload(b"he").unwrap();
    writer.write_payload(b"llo").unwrap();
    writer.finish().unwrap()
}
fn consume(bytes: &[u8]) -> Result<Vec<(Header, Vec<u8>)>, Error> {
    let mut reader = Reader::new(Cursor::new(bytes), limits());
    let mut result = Vec::new();
    while let Some(header) = reader.next_header()? {
        reader.begin_payload(None)?;
        let mut left = header.size;
        let mut data = Vec::new();
        while left != 0 {
            let mut buf = [0; 7];
            let count = reader.read_payload(&mut buf)?;
            data.extend_from_slice(&buf[..count]);
            left -= count as u64;
        }
        result.push((header, data));
    }
    Ok(result)
}
#[test]
fn independent_python_ustar_fixture_and_streamed_writer() {
    let expected = vec![(header(), b"hello".to_vec())];
    assert_eq!(
        consume(include_bytes!("../../tests/fixtures/python-ustar.bin")).unwrap(),
        expected
    );
    assert_eq!(consume(&archive()).unwrap(), expected);
}
#[test]
fn every_truncated_prefix_and_nonzero_padding_is_rejected() {
    let valid = archive();
    for end in 0..valid.len() {
        assert!(consume(&valid[..end]).is_err(), "accepted prefix {end}");
    }
    for at in [0, 100, 148, 257, 500, 517, 1024, 1536] {
        let mut damaged = valid.clone();
        damaged[at] ^= 1;
        assert!(consume(&damaged).is_err(), "accepted damage {at}");
    }
    let mut extra = valid.clone();
    extra.extend([0; BLOCK]);
    assert!(consume(&extra).is_ok());
    extra.push(0);
    assert!(consume(&extra).is_err());
    let mut extra = valid;
    extra.extend([1; BLOCK]);
    assert!(consume(&extra).is_err());
}
#[test]
fn header_ranges_types_and_long_utf8_names() {
    let mut h = header();
    h.path = format!("{}/{}", "d".repeat(150), "é".repeat(45));
    assert_eq!(Header::decode(&h.encode().unwrap()).unwrap(), h);
    h.path = "x".repeat(101);
    assert!(matches!(h.encode(), Err(Error::Limit)));
    h = header();
    h.size = 1u64 << 33;
    assert!(matches!(h.encode(), Err(Error::Limit)));
    h = header();
    h.kind = Kind::Directory;
    assert!(h.encode().is_err());
    for kind in [
        Kind::Directory,
        Kind::HardLink,
        Kind::Symlink,
        Kind::PaxGlobal,
        Kind::PaxLocal,
    ] {
        h = header();
        h.kind = kind;
        h.size = 0;
        assert_eq!(Header::decode(&h.encode().unwrap()).unwrap(), h);
    }
    // Valid checksum must not conceal unsupported type, binary numbers or text damage.
    for (at, byte) in [(156, b'3'), (124, 0x80), (1, 0xff), (500, 1)] {
        let mut block = header().encode().unwrap();
        block[at] = byte;
        let sum = format!("{:06o}\0 ", checksum(&block));
        block[148..156].copy_from_slice(sum.as_bytes());
        assert!(Header::decode(&block).is_err());
    }
}
#[test]
fn member_and_byte_admission_and_payload_order_are_explicit() {
    let mut writer = Writer::new(
        Vec::new(),
        Limits {
            member_bytes: 4,
            ..limits()
        },
    );
    assert!(matches!(writer.start(&header(), None), Err(Error::Limit)));
    assert!(writer.inner.is_empty());
    let mut writer = Writer::new(Vec::new(), limits());
    writer.start(&header(), None).unwrap();
    assert!(matches!(writer.start(&header(), None), Err(Error::Busy)));
    assert!(matches!(
        writer.write_payload(b"too long"),
        Err(Error::Limit)
    ));
    assert_eq!(writer.inner.len(), BLOCK);
    assert!(matches!(writer.finish(), Err(Error::Busy)));
    let data = archive();
    let mut reader = Reader::new(
        Cursor::new(&data),
        Limits {
            member_bytes: 4,
            ..limits()
        },
    );
    reader.next_header().unwrap();
    assert!(matches!(reader.next_header(), Err(Error::Busy)));
    assert!(matches!(reader.begin_payload(None), Err(Error::Limit)));
    let mut reader = Reader::new(
        Cursor::new(&data),
        Limits {
            members: 0,
            ..limits()
        },
    );
    assert!(matches!(reader.next_header(), Err(Error::Limit)));
    assert!(matches!(reader.next_header(), Err(Error::Poisoned)));
    let mut padded = data;
    padded.extend([0; BLOCK]);
    let mut reader = Reader::new(
        Cursor::new(padded),
        Limits {
            trailing_zero_blocks: 0,
            ..limits()
        },
    );
    reader.next_header().unwrap();
    reader.begin_payload(None).unwrap();
    reader.read_payload(&mut [0; 5]).unwrap();
    assert!(matches!(reader.next_header(), Err(Error::Limit)));
}
#[test]
fn pax_size_override_is_selected_before_data_and_bounded() {
    let mut h = header();
    h.size = 0;
    let mut writer = Writer::new(Vec::new(), limits());
    writer.start(&h, Some(5)).unwrap();
    writer.write_payload(b"hello").unwrap();
    let bytes = writer.finish().unwrap();
    let mut reader = Reader::new(Cursor::new(bytes), limits());
    assert_eq!(reader.next_header().unwrap().unwrap().size, 0);
    reader.begin_payload(Some(5)).unwrap();
    assert!(matches!(reader.begin_payload(Some(5)), Err(Error::Busy)));
    let mut data = [0; 8];
    assert_eq!(reader.read_payload(&mut data).unwrap(), 5);
    assert_eq!(&data[..5], b"hello");
    assert!(reader.next_header().unwrap().is_none());
    h.kind = Kind::Directory;
    let mut writer = Writer::new(Vec::new(), limits());
    assert!(writer.start(&h, Some(0)).is_err());
    let mut reader = Reader::new(Cursor::new(Vec::<u8>::new()), limits());
    reader.state = State::Size(Kind::File, 0);
    reader.begin_payload(Some(u64::MAX)).unwrap();
    assert!(matches!(reader.state, State::Body(u64::MAX, 1)));
}
struct FaultWriter {
    bytes: Vec<u8>,
    fail_after: usize,
    fail_flush: bool,
}
impl Write for FaultWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len() >= self.fail_after {
            return Err(std::io::Error::other("injected write"));
        }
        let count = data.len().min(self.fail_after - self.bytes.len());
        self.bytes.extend_from_slice(&data[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.fail_flush {
            Err(std::io::Error::other("injected flush"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn partial_output_errors_poison_the_stream_and_cannot_report_success() {
    for fail_after in [0, 1, 511, 512, 514, 517, 1023] {
        let mut writer = Writer::new(
            FaultWriter {
                bytes: Vec::new(),
                fail_after,
                fail_flush: false,
            },
            limits(),
        );
        let result = writer
            .start(&header(), None)
            .and_then(|_| writer.write_payload(b"hello"));
        assert!(result.is_err());
        assert!(matches!(
            writer.start(&header(), None),
            Err(Error::Poisoned)
        ));
        assert!(writer.finish().is_err());
    }
    for (fail_after, fail_flush) in [(1024, false), (1500, false), (usize::MAX, true)] {
        let mut writer = Writer::new(
            FaultWriter {
                bytes: Vec::new(),
                fail_after,
                fail_flush,
            },
            limits(),
        );
        writer.start(&header(), None).unwrap();
        writer.write_payload(b"hello").unwrap();
        assert!(writer.finish().is_err());
    }
    let bytes = archive();
    let mut reader = Reader::new(Cursor::new(&bytes[..514]), limits());
    reader.next_header().unwrap();
    reader.begin_payload(None).unwrap();
    assert!(reader.read_payload(&mut [0; 5]).is_err());
    assert!(matches!(reader.next_header(), Err(Error::Poisoned)));
}
