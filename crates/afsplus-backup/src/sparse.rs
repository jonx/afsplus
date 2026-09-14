//! Explicit GNU sparse PAX 1.0 content transport. Reservations are separate.
use crate::{envelope, member, pax, stream, tar};
use std::io::{Read, Write};

#[derive(Debug)]
pub enum Error {
    Invalid,
    Limit,
    Stream(stream::Error),
    Envelope(envelope::Error),
    Pax(pax::Error),
    #[cfg(feature = "consumer")]
    Source(afsplus_vfs::backup::BackupError),
    #[cfg(feature = "consumer")]
    Destination(afsplus_vfs::restore::RestoreError),
    #[cfg(feature = "consumer")]
    NeedsVerifiedReplay,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub offset: u64,
    pub length: u64,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub entries: usize,
    pub map_bytes: u64,
    pub data_bytes: u64,
    pub logical_bytes: u64,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Map {
    logical_size: u64,
    ranges: Vec<Range>,
    map_bytes: u64,
    data_bytes: u64,
}
impl Map {
    /// Ranges describe stored content only. A final EOF marker is optional.
    pub fn new(logical_size: u64, ranges: &[Range], limits: Limits) -> Result<Self, Error> {
        let (map_bytes, data_bytes) = validate(logical_size, ranges, limits)?;
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(ranges.len())
            .map_err(|_| Error::Limit)?;
        owned.extend_from_slice(ranges);
        Ok(Self {
            logical_size,
            ranges: owned,
            map_bytes,
            data_bytes,
        })
    }
    pub fn logical_size(&self) -> u64 {
        self.logical_size
    }
    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }
    pub fn map_bytes(&self) -> u64 {
        self.map_bytes
    }
    pub fn data_bytes(&self) -> u64 {
        self.data_bytes
    }
    pub fn stored_bytes(&self) -> u64 {
        self.map_bytes + self.data_bytes
    }
    /// Emit only the padded map with one fixed 512-byte buffer.
    pub fn emit(&self, mut output: impl FnMut(&[u8]) -> Result<(), Error>) -> Result<(), Error> {
        let mut block = [0; 512];
        let mut used = 0;
        let mut number = |value: u64| -> Result<(), Error> {
            for byte in value.to_string().bytes().chain(std::iter::once(b'\n')) {
                block[used] = byte;
                used += 1;
                if used == 512 {
                    output(&block)?;
                    block.fill(0);
                    used = 0;
                }
            }
            Ok(())
        };
        number(self.ranges.len() as u64)?;
        for range in &self.ranges {
            number(range.offset)?;
            number(range.length)?;
        }
        if used != 0 {
            output(&block)?;
        }
        Ok(())
    }
}
fn validate(logical_size: u64, ranges: &[Range], limits: Limits) -> Result<(u64, u64), Error> {
    if logical_size > limits.logical_bytes || ranges.len() > limits.entries {
        return Err(Error::Limit);
    }
    let mut end = 0u64;
    let mut bytes = 0u64;
    let mut text = (ranges.len() as u64).to_string().len() as u64 + 1;
    for (i, range) in ranges.iter().enumerate() {
        if range.offset < end
            || (range.length == 0 && (i + 1 != ranges.len() || range.offset != logical_size))
        {
            return Err(Error::Invalid);
        }
        end = range
            .offset
            .checked_add(range.length)
            .ok_or(Error::Invalid)?;
        if end > logical_size {
            return Err(Error::Invalid);
        }
        bytes = bytes.checked_add(range.length).ok_or(Error::Limit)?;
        text = text
            .checked_add(
                range.offset.to_string().len() as u64 + range.length.to_string().len() as u64 + 2,
            )
            .ok_or(Error::Limit)?;
    }
    let padded = text.checked_add(511).ok_or(Error::Limit)? / 512 * 512;
    if padded > limits.map_bytes || bytes > limits.data_bytes || padded.checked_add(bytes).is_none()
    {
        return Err(Error::Limit);
    }
    Ok((padded, bytes))
}
fn admit_count(count: usize, entries: usize, map_bytes: u64) -> Result<(), Error> {
    // Every range requires at least "0\n0\n". Reject impossible counts before
    // reserving vector memory, even if the caller supplies a large entry limit.
    let minimum = (count as u128) * 4 + (count as u64).to_string().len() as u128 + 1;
    if count > entries || minimum.div_ceil(512) * 512 > map_bytes as u128 {
        return Err(Error::Limit);
    }
    Ok(())
}
struct Input<'a, R> {
    reader: &'a mut stream::Reader<R>,
    block: [u8; 512],
    pos: usize,
    consumed: u64,
    maximum: u64,
}
impl<R: Read> Input<'_, R> {
    fn number(&mut self) -> Result<u64, Error> {
        let mut text = [0; 20];
        let mut len = 0;
        loop {
            if self.pos == 512 {
                self.consumed = self.consumed.checked_add(512).ok_or(Error::Limit)?;
                if self.consumed > self.maximum {
                    return Err(Error::Limit);
                }
                if self
                    .reader
                    .read_payload(&mut self.block)
                    .map_err(Error::Stream)?
                    != 512
                {
                    return Err(Error::Invalid);
                }
                self.pos = 0;
            }
            let byte = self.block[self.pos];
            self.pos += 1;
            if byte == b'\n' {
                break;
            }
            if !byte.is_ascii_digit() || len == text.len() {
                return Err(Error::Invalid);
            }
            text[len] = byte;
            len += 1;
        }
        member::unsigned(std::str::from_utf8(&text[..len]).map_err(|_| Error::Invalid)?)
            .map_err(|_| Error::Invalid)
    }
}
/// Consume and validate the map before exposing its data locations.
pub fn read_map<R: Read>(
    reader: &mut stream::Reader<R>,
    logical: u64,
    stored: u64,
    limits: Limits,
) -> Result<Map, Error> {
    let result = read_inner(reader, logical, stored, limits);
    if result.is_err() {
        reader.invalidate();
    }
    result
}
fn read_inner<R: Read>(
    reader: &mut stream::Reader<R>,
    logical: u64,
    stored: u64,
    limits: Limits,
) -> Result<Map, Error> {
    if logical > limits.logical_bytes {
        return Err(Error::Limit);
    }
    let mut input = Input {
        reader,
        block: [0; 512],
        pos: 512,
        consumed: 0,
        maximum: stored.min(limits.map_bytes),
    };
    let count = usize::try_from(input.number()?).map_err(|_| Error::Limit)?;
    admit_count(count, limits.entries, input.maximum)?;
    let mut ranges = Vec::new();
    ranges.try_reserve_exact(count).map_err(|_| Error::Limit)?;
    for _ in 0..count {
        ranges.push(Range {
            offset: input.number()?,
            length: input.number()?,
        });
    }
    if input.block[input.pos..].iter().any(|&b| b != 0) {
        return Err(Error::Invalid);
    }
    let (map_bytes, data_bytes) = validate(logical, &ranges, limits)?;
    if map_bytes != input.consumed || map_bytes + data_bytes != stored {
        return Err(Error::Invalid);
    }
    Ok(Map {
        logical_size: logical,
        ranges,
        map_bytes,
        data_bytes,
    })
}
pub struct Entry<'a> {
    pub ordinal: u64,
    pub path: &'a str,
    pub modified: member::Timestamp,
}
/// Start a sparse entry and emit its map. Caller must stream exactly data_bytes.
pub fn start<W: Write>(
    writer: &mut envelope::Writer<W>,
    entry: &Entry<'_>,
    map: &Map,
    records: pax::Limits,
) -> Result<(), Error> {
    let result = start_inner(writer, entry, map, records);
    if result.is_err() {
        writer.invalidate();
    }
    result
}
fn header(path: String, kind: tar::Kind, size: u64) -> tar::Header {
    tar::Header {
        path,
        kind,
        size,
        mode: 0o600,
        uid: 0,
        gid: 0,
        mtime: 0,
        link: String::new(),
        uname: String::new(),
        gname: String::new(),
    }
}
fn start_inner<W: Write>(
    writer: &mut envelope::Writer<W>,
    entry: &Entry<'_>,
    map: &Map,
    records: pax::Limits,
) -> Result<(), Error> {
    let Entry {
        ordinal,
        path,
        modified,
    } = entry;
    if !envelope::canonical(path, false, false) {
        return Err(Error::Invalid);
    }
    let modified = modified.decimal().map_err(|_| Error::Invalid)?;
    let real = map.logical_size.to_string();
    let wire = pax::encode(
        &[
            pax::Record {
                key: "mtime",
                value: &modified,
            },
            pax::Record {
                key: "GNU.sparse.major",
                value: "1",
            },
            pax::Record {
                key: "GNU.sparse.minor",
                value: "0",
            },
            pax::Record {
                key: "GNU.sparse.name",
                value: path,
            },
            pax::Record {
                key: "GNU.sparse.realsize",
                value: &real,
            },
        ],
        records,
    )
    .map_err(Error::Pax)?;
    writer
        .start(
            &header(
                format!("_AROS_BACKUP/metadata/sparse-{ordinal}.pax"),
                tar::Kind::PaxLocal,
                wire.len() as u64,
            ),
            None,
        )
        .map_err(Error::Envelope)?;
    writer.write_payload(&wire).map_err(Error::Envelope)?;
    writer
        .start(
            &header(
                format!("files/GNUSparseFile.{ordinal}/payload"),
                tar::Kind::File,
                map.stored_bytes(),
            ),
            None,
        )
        .map_err(Error::Envelope)?;
    map.emit(|block| writer.write_payload(block).map_err(Error::Envelope))
}

#[cfg(feature = "consumer")]
pub mod consumer {
    use super::*;
    use crate::attachment::Captured;
    use afsplus_format::Timespec;
    use afsplus_vfs::{
        backup::SnapshotBackend,
        restore::{RestoreBackend, RestoreClient, RestoreObject},
        NodeKind,
    };
    #[derive(Clone, Copy)]
    pub struct Options {
        pub map: Limits,
        pub records: pax::Limits,
        pub page_entries: usize,
    }
    /// Content transport result, never a full-preservation receipt.
    #[derive(Debug, PartialEq, Eq)]
    pub struct Report {
        pub logical_bytes: u64,
        pub written_bytes: u64,
        pub stored_bytes: u64,
        pub omitted_unwritten_ranges: u64,
        pub omitted_unwritten_bytes: u128,
    }
    pub struct Target<'a, O> {
        pub path: &'a str,
        pub object: &'a RestoreObject<O>,
    }
    pub fn export<P: SnapshotBackend, W: Write>(
        source: &mut Captured<'_, '_, P>,
        writer: &mut envelope::Writer<W>,
        ordinal: u64,
        path: &str,
        scratch: &mut [u8],
        options: Options,
    ) -> Result<Report, Error> {
        let result = export_inner(source, writer, ordinal, path, scratch, options);
        if result.is_err() {
            writer.invalidate();
        }
        result
    }
    fn export_inner<P: SnapshotBackend, W: Write>(
        source: &mut Captured<'_, '_, P>,
        writer: &mut envelope::Writer<W>,
        ordinal: u64,
        path: &str,
        scratch: &mut [u8],
        options: Options,
    ) -> Result<Report, Error> {
        if scratch.is_empty() || options.page_entries == 0 || options.page_entries > 64 {
            return Err(Error::Limit);
        }
        let stat = source
            .client
            .stat(source.reader, source.object)
            .map_err(Error::Source)?;
        if stat.kind != NodeKind::File {
            return Err(Error::Invalid);
        }
        if stat.size > options.map.logical_bytes {
            return Err(Error::Limit);
        }
        let mut ranges = Vec::new();
        let mut count = 0usize;
        let mut cursor = 0u64;
        let mut end = 0u128;
        let mut omitted_unwritten_ranges = 0u64;
        let mut omitted_unwritten_bytes = 0u128;
        loop {
            let page = source
                .client
                .allocations(source.reader, source.object, cursor, options.page_entries)
                .map_err(Error::Source)?;
            if page.ranges.len() > options.page_entries || (!page.eof && page.ranges.is_empty()) {
                return Err(Error::Invalid);
            }
            let next = cursor
                .checked_add(page.ranges.len() as u64)
                .ok_or(Error::Limit)?;
            if page.next != next {
                return Err(Error::Invalid);
            }
            for range in page.ranges {
                count = count.checked_add(1).ok_or(Error::Limit)?;
                if count > options.map.entries {
                    return Err(Error::Limit);
                }
                if range.length == 0 || (range.offset as u128) < end || range.offset < cursor {
                    return Err(Error::Invalid);
                }
                end = range.offset as u128 + range.length as u128;
                if end > u64::MAX as u128 + 1 {
                    return Err(Error::Invalid);
                }
                if range.unwritten {
                    omitted_unwritten_ranges += 1;
                    omitted_unwritten_bytes = omitted_unwritten_bytes
                        .checked_add(range.length as u128)
                        .ok_or(Error::Limit)?;
                } else if range.offset < stat.size {
                    admit_count(
                        ranges.len().checked_add(1).ok_or(Error::Limit)?,
                        options.map.entries,
                        options.map.map_bytes,
                    )?;
                    ranges.try_reserve(1).map_err(|_| Error::Limit)?;
                    ranges.push(Range {
                        offset: range.offset,
                        length: range.length.min(stat.size - range.offset),
                    });
                }
            }
            if page.eof {
                break;
            }
            if page.next <= cursor {
                return Err(Error::Invalid);
            }
            cursor = page.next;
        }
        // GNU consumers use this marker to establish trailing logical holes.
        ranges.try_reserve(1).map_err(|_| Error::Limit)?;
        ranges.push(Range {
            offset: stat.size,
            length: 0,
        });
        let (map_bytes, data_bytes) = validate(stat.size, &ranges, options.map)?;
        let map = Map {
            logical_size: stat.size,
            ranges,
            map_bytes,
            data_bytes,
        };
        start(
            writer,
            &Entry {
                ordinal,
                path,
                modified: member::Timestamp {
                    seconds: stat.modified.seconds,
                    nanos: stat.modified.nanoseconds,
                },
            },
            &map,
            options.records,
        )?;
        for range in map.ranges() {
            let mut offset = range.offset;
            let mut left = range.length;
            while left != 0 {
                let ask = left.min(scratch.len() as u64) as usize;
                let got = source
                    .client
                    .read(source.reader, source.object, offset, &mut scratch[..ask])
                    .map_err(Error::Source)?;
                if got == 0 || got > ask {
                    return Err(Error::Invalid);
                }
                writer
                    .write_payload(&scratch[..got])
                    .map_err(Error::Envelope)?;
                offset += got as u64;
                left -= got as u64;
            }
        }
        if source
            .client
            .stat(source.reader, source.object)
            .map_err(Error::Source)?
            != stat
        {
            return Err(Error::Invalid);
        }
        Ok(Report {
            logical_bytes: stat.size,
            written_bytes: data_bytes,
            stored_bytes: map.stored_bytes(),
            omitted_unwritten_ranges,
            omitted_unwritten_bytes,
        })
    }
    /// Restore contents only. The enclosing job owns metadata, loss reporting,
    /// namespace/inventory completeness, final EOF and destination sync.
    pub fn restore<R: Read, P: RestoreBackend>(
        reader: &mut stream::Reader<R>,
        client: &mut RestoreClient<'_, P>,
        target: &Target<'_, P::Object>,
        scratch: &mut [u8],
        limits: Limits,
        now: Timespec,
    ) -> Result<Map, Error> {
        let result = restore_inner(reader, client, target, scratch, limits, now);
        if result.is_err() {
            reader.invalidate();
        }
        result
    }
    fn restore_inner<R: Read, P: RestoreBackend>(
        reader: &mut stream::Reader<R>,
        client: &mut RestoreClient<'_, P>,
        target: &Target<'_, P::Object>,
        scratch: &mut [u8],
        limits: Limits,
        now: Timespec,
    ) -> Result<Map, Error> {
        if !reader.can_publish() {
            return Err(Error::NeedsVerifiedReplay);
        }
        if scratch.is_empty() {
            return Err(Error::Limit);
        }
        let stat = client.stat(target.object).map_err(Error::Destination)?;
        if stat.kind != NodeKind::File
            || stat.size != 0
            || stat.allocated_size != 0
            || stat.links != 1
        {
            return Err(Error::Invalid);
        }
        let m = reader
            .next_member()
            .map_err(Error::Stream)?
            .ok_or(Error::Invalid)?;
        if m.path != target.path || m.kind != tar::Kind::File {
            return Err(Error::Invalid);
        }
        let logical = m.sparse_size.ok_or(Error::Invalid)?;
        let stored = m.size;
        let map = read_map(reader, logical, stored, limits)?;
        for range in map.ranges() {
            let mut offset = range.offset;
            let mut left = range.length;
            while left != 0 {
                let ask = left.min(scratch.len() as u64) as usize;
                let got = reader
                    .read_payload(&mut scratch[..ask])
                    .map_err(Error::Stream)?;
                if got == 0 || got > ask {
                    return Err(Error::Invalid);
                }
                client
                    .write(target.object, offset, &scratch[..got], now)
                    .map_err(Error::Destination)?;
                offset += got as u64;
                left -= got as u64;
            }
        }
        client
            .resize(target.object, logical, now)
            .map_err(Error::Destination)?;
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds() -> Limits {
        Limits {
            entries: 256,
            map_bytes: 8192,
            data_bytes: 65536,
            logical_bytes: u64::MAX,
        }
    }
    fn framing() -> tar::Limits {
        tar::Limits {
            members: 16,
            member_bytes: 65536,
            trailing_zero_blocks: 0,
        }
    }
    fn records() -> pax::Limits {
        pax::Limits {
            bytes: 4096,
            records: 16,
            key_bytes: 64,
            value_bytes: 1024,
        }
    }
    fn custom(text: &[u8], logical: u64, payload: &[u8]) -> Vec<u8> {
        let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
        let real = logical.to_string();
        let wire = pax::encode(
            &[
                pax::Record {
                    key: "GNU.sparse.major",
                    value: "1",
                },
                pax::Record {
                    key: "GNU.sparse.minor",
                    value: "0",
                },
                pax::Record {
                    key: "GNU.sparse.name",
                    value: "files/test",
                },
                pax::Record {
                    key: "GNU.sparse.realsize",
                    value: &real,
                },
            ],
            records(),
        )
        .unwrap();
        writer
            .start(
                &header(
                    "_AROS_BACKUP/metadata/test.pax".into(),
                    tar::Kind::PaxLocal,
                    wire.len() as u64,
                ),
                None,
            )
            .unwrap();
        writer.write_payload(&wire).unwrap();
        let mut map = text.to_vec();
        map.resize(text.len().div_ceil(512) * 512, 0);
        writer
            .start(
                &header(
                    "files/GNUSparseFile.0/test".into(),
                    tar::Kind::File,
                    (map.len() + payload.len()) as u64,
                ),
                None,
            )
            .unwrap();
        writer.write_payload(&map).unwrap();
        writer.write_payload(payload).unwrap();
        writer.finish().unwrap().0
    }
    #[test]
    fn sparse_maps_cross_blocks_and_preserve_empty_and_extreme_lengths() {
        for logical in [0, 1 << 40, u64::MAX] {
            let map = Map::new(
                logical,
                &[Range {
                    offset: logical,
                    length: 0,
                }],
                bounds(),
            )
            .unwrap();
            let mut bytes = Vec::new();
            map.emit(|b| {
                bytes.extend_from_slice(b);
                Ok(())
            })
            .unwrap();
            let wire = custom(&bytes, logical, &[]);
            let mut reader =
                stream::Reader::new_sparse(wire.as_slice(), framing(), records()).unwrap();
            let m = reader.next_member().unwrap().unwrap();
            let stored = m.size;
            assert_eq!(m.sparse_size, Some(logical));
            assert_eq!(
                read_map(&mut reader, logical, stored, bounds()).unwrap(),
                map
            );
            assert!(reader.next_member().unwrap().is_none());
        }
        let ranges: Vec<_> = (0..180)
            .map(|i| Range {
                offset: i * 4096,
                length: 1,
            })
            .collect();
        let map = Map::new(1 << 30, &ranges, bounds()).unwrap();
        assert!(map.map_bytes() > 512);
        let mut bytes = Vec::new();
        map.emit(|b| {
            bytes.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        let wire = custom(&bytes, 1 << 30, &[7; 180]);
        let mut reader = stream::Reader::new_sparse(wire.as_slice(), framing(), records()).unwrap();
        let stored = reader.next_member().unwrap().unwrap().size;
        assert_eq!(
            read_map(&mut reader, 1 << 30, stored, bounds()).unwrap(),
            map
        );
        let mut data = [0; 180];
        assert_eq!(reader.read_payload(&mut data).unwrap(), 180);
        assert_eq!(data, [7; 180]);
        assert!(reader.next_member().unwrap().is_none());
    }
    #[test]
    fn malformed_maps_and_resource_limits_poison_before_data() {
        for text in [
            "01\n0\n1\n",
            "1\n+0\n1\n",
            "1\n0\n-1\n",
            "1\n8\n3\n",
            "1\n0\n0\n",
            "2\n3\n3\n4\n1\n",
            "2\n10\n0\n10\n0\n",
            "1\n18446744073709551615\n1\n",
            "0\nx",
            "99999999999999999999\n",
        ] {
            let wire = custom(text.as_bytes(), 10, &[]);
            let mut reader =
                stream::Reader::new_sparse(wire.as_slice(), framing(), records()).unwrap();
            let stored = reader.next_member().unwrap().unwrap().size;
            assert!(
                read_map(&mut reader, 10, stored, bounds()).is_err(),
                "{text:?}"
            );
            assert!(reader.next_member().is_err());
        }
        for fault in 0..5 {
            let wire = custom(b"1\n0\n1\n", 10, if fault == 4 { &[] } else { &[7] });
            let mut reader =
                stream::Reader::new_sparse(wire.as_slice(), framing(), records()).unwrap();
            let stored = reader.next_member().unwrap().unwrap().size;
            let mut limits = bounds();
            match fault {
                0 => limits.entries = 0,
                1 => limits.map_bytes = 511,
                2 => limits.data_bytes = 0,
                3 => limits.logical_bytes = 9,
                _ => (),
            }
            assert!(read_map(&mut reader, 10, stored, limits).is_err());
            assert!(reader.next_member().is_err());
        }
        assert!(admit_count(usize::MAX, usize::MAX, 512).is_err());
        assert!(Map::new(
            u64::MAX,
            &[Range {
                offset: 0,
                length: u64::MAX
            }],
            Limits {
                data_bytes: u64::MAX,
                ..bounds()
            }
        )
        .is_err());
    }
    #[test]
    fn sparse_header_admission_is_explicit_and_conflicting_sizes_are_refused() {
        let h = header("files/GNUSparseFile.0/test".into(), tar::Kind::File, 512);
        let records = [
            pax::Record {
                key: "GNU.sparse.major",
                value: "1",
            },
            pax::Record {
                key: "GNU.sparse.minor",
                value: "0",
            },
            pax::Record {
                key: "GNU.sparse.name",
                value: "files/test",
            },
            pax::Record {
                key: "GNU.sparse.realsize",
                value: "1024",
            },
        ];
        assert!(member::resolve(&h, &records, self::records()).is_err());
        let m = member::resolve_mode(&h, &records, self::records(), true).unwrap();
        assert_eq!(m.size, 512);
        assert_eq!(m.sparse_size, Some(1024));
        for missing in 0..4 {
            let mut bad = records.to_vec();
            bad.remove(missing);
            assert!(member::resolve_mode(&h, &bad, self::records(), true).is_err());
        }
        for (key, value) in [
            ("size", "512"),
            ("path", "files/test"),
            ("GNU.sparse.extra", "1"),
        ] {
            let mut bad = records.to_vec();
            bad.push(pax::Record { key, value });
            assert!(member::resolve_mode(&h, &bad, self::records(), true).is_err());
        }
        for (i, value) in [
            (0, "2"),
            (1, "1"),
            (2, "../escape"),
            (2, "_AROS_BACKUP/metadata/escape"),
            (3, "-1"),
        ] {
            let mut bad = records;
            bad[i].value = value;
            assert!(member::resolve_mode(&h, &bad, self::records(), true).is_err());
        }
        let wire = custom(b"0\n", 0, &[]);
        let mut reader = stream::Reader::new(wire.as_slice(), framing(), self::records()).unwrap();
        assert!(reader.next_member().is_err());
        assert!(reader.next_member().is_err());
    }
    #[test]
    fn every_sparse_archive_truncation_withholds_completion() {
        let wire = custom(b"1\n1\n3\n", 10, b"abc");
        for end in 0..wire.len() {
            let Ok(mut reader) = stream::Reader::new_sparse(&wire[..end], framing(), records())
            else {
                continue;
            };
            if let Ok(Some(m)) = reader.next_member() {
                let stored = m.size;
                if read_map(&mut reader, 10, stored, bounds()).is_ok()
                    && reader.read_payload(&mut [0; 3]).is_ok()
                {
                    assert!(reader.next_member().is_err());
                }
            }
            assert!(reader.receipt().is_none());
        }
    }
}
