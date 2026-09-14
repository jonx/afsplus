//! ADR-090 allocation records bound to sparse content and scoped readback.
use crate::{
    attachment, envelope, member, pax,
    sparse::{self, consumer, Error},
    stream,
};
use afsplus_format::Timespec;
use afsplus_vfs::{
    backup::{AllocationRange, SnapshotBackend},
    restore::{RestoreBackend, RestoreClient, RestoreObject},
};
use std::io::{Read, Write};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub logical_size: u64,
    pub ranges: Vec<AllocationRange>,
}
#[derive(Clone, Copy)]
pub enum Mode {
    PreserveAllocation,
    RecoverContents,
}
#[derive(Clone, Copy)]
pub struct RestoreOptions {
    pub limits: consumer::Options,
    pub mode: Mode,
    pub reservation_chunk: u64,
    pub reservation_bytes: u128,
    pub readback_entries: u64,
}
pub struct Target<'a, O> {
    pub ordinal: u64,
    pub path: &'a str,
    pub object: &'a RestoreObject<O>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Disposition {
    Preserved,
    Discarded { ranges: u64, bytes: u128 },
}
/// File allocation/content outcome, not an object or whole-job completion receipt.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub logical_bytes: u64,
    pub written_bytes: u64,
    pub next_ordinal: Option<u64>,
    pub allocation: Disposition,
}
#[derive(Debug)]
pub struct ExportReport {
    pub contents: consumer::Report,
    pub next_ordinal: Option<u64>,
}
fn validate(size: u64, ranges: &[AllocationRange], limits: consumer::Options) -> Result<(), Error> {
    if size > limits.map.logical_bytes || ranges.len() > limits.map.entries {
        return Err(Error::Limit);
    }
    let mut end = 0u128;
    for r in ranges {
        if r.length == 0 || (r.offset as u128) < end {
            return Err(Error::Invalid);
        }
        end = r.offset as u128 + r.length as u128;
        if end > (1u128 << 64) {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
const KEYS: [&str; 5] = [
    "AROS.allocation.version",
    "AROS.allocation.path",
    "AROS.allocation.size",
    "AROS.allocation.count",
    "AROS.allocation.ranges",
];
pub fn encode(
    path: &str,
    size: u64,
    ranges: &[AllocationRange],
    limits: consumer::Options,
) -> Result<Vec<u8>, Error> {
    validate(size, ranges, limits)?;
    if !envelope::canonical(path, false, false) {
        return Err(Error::Invalid);
    }
    let length = ranges.iter().try_fold(0usize, |n, r| {
        n.checked_add(r.offset.to_string().len() + r.length.to_string().len() + 4)
            .ok_or(Error::Limit)
    })?;
    if length > limits.records.value_bytes
        || length > limits.records.bytes
        || path.len() > limits.records.value_bytes
    {
        return Err(Error::Limit);
    }
    let mut text = String::new();
    text.try_reserve_exact(length).map_err(|_| Error::Limit)?;
    for r in ranges {
        use std::fmt::Write;
        writeln!(
            text,
            "{},{},{}",
            r.offset,
            r.length,
            if r.unwritten { "u" } else { "w" }
        )
        .map_err(|_| Error::Invalid)?;
    }
    let size = size.to_string();
    let count = ranges.len().to_string();
    let values = ["1", path, &size, &count, &text];
    let records: [pax::Record<'_>; 5] = std::array::from_fn(|i| pax::Record {
        key: KEYS[i],
        value: values[i],
    });
    pax::encode(&records, limits.records).map_err(Error::Pax)
}
pub fn decode(wire: &[u8], limits: consumer::Options) -> Result<(&str, Layout), Error> {
    let records = pax::decode(wire, limits.records).map_err(Error::Pax)?;
    if records.len() != KEYS.len() || records.iter().any(|r| !KEYS.contains(&r.key)) {
        return Err(Error::Invalid);
    }
    let get = |i: usize| {
        records
            .iter()
            .find(|r| r.key == KEYS[i])
            .map(|r| r.value)
            .ok_or(Error::Invalid)
    };
    if get(0)? != "1" || !envelope::canonical(get(1)?, false, false) {
        return Err(Error::Invalid);
    }
    let number = |text| member::unsigned(text).map_err(|_| Error::Invalid);
    let size = number(get(2)?)?;
    let count = usize::try_from(number(get(3)?)?).map_err(|_| Error::Limit)?;
    let text = get(4)?;
    if count > limits.map.entries || count > text.len() / 6 {
        return Err(Error::Limit);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(Error::Invalid);
    }
    let mut ranges = Vec::new();
    ranges.try_reserve_exact(count).map_err(|_| Error::Limit)?;
    for line in text.split_terminator('\n') {
        if ranges.len() == count {
            return Err(Error::Invalid);
        }
        let mut fields = line.split(',');
        let offset = number(fields.next().ok_or(Error::Invalid)?)?;
        let length = number(fields.next().ok_or(Error::Invalid)?)?;
        let unwritten = match fields.next() {
            Some("u") => true,
            Some("w") => false,
            _ => return Err(Error::Invalid),
        };
        if fields.next().is_some() {
            return Err(Error::Invalid);
        }
        ranges.push(AllocationRange {
            offset,
            length,
            unwritten,
        });
    }
    if ranges.len() != count {
        return Err(Error::Invalid);
    }
    validate(size, &ranges, limits)?;
    Ok((
        get(1)?,
        Layout {
            logical_size: size,
            ranges,
        },
    ))
}
fn name(ordinal: u64) -> String {
    format!("_AROS_BACKUP/metadata/allocation-{ordinal}.pax")
}
struct Coverage<I> {
    remaining: I,
    current: Option<AllocationRange>,
    used: u64,
}
impl<I: Iterator<Item = AllocationRange>> Coverage<I> {
    fn new(mut remaining: I) -> Self {
        let current = remaining.next();
        Self {
            remaining,
            current,
            used: 0,
        }
    }
    fn push(&mut self, range: AllocationRange) -> Result<(), Error> {
        if range.length == 0 {
            return Err(Error::Invalid);
        }
        let mut position = range.offset as u128;
        let end = position + range.length as u128;
        if end > (1u128 << 64) {
            return Err(Error::Invalid);
        }
        while position < end {
            let expected = self.current.ok_or(Error::Invalid)?;
            if position != expected.offset as u128 + self.used as u128
                || range.unwritten != expected.unwritten
            {
                return Err(Error::Invalid);
            }
            let take = (end - position).min((expected.length - self.used) as u128) as u64;
            self.used += take;
            position += take as u128;
            if self.used == expected.length {
                self.current = self.remaining.next();
                self.used = 0;
            }
        }
        Ok(())
    }
    fn finish(self) -> Result<(), Error> {
        if self.current.is_none() {
            Ok(())
        } else {
            Err(Error::Invalid)
        }
    }
}
fn match_written(layout: &Layout, map: &sparse::Map) -> Result<(), Error> {
    if layout.logical_size != map.logical_size() {
        return Err(Error::Invalid);
    }
    let expected = layout
        .ranges
        .iter()
        .filter(|r| !r.unwritten && r.offset < layout.logical_size)
        .map(|r| AllocationRange {
            offset: r.offset,
            length: r.length.min(layout.logical_size - r.offset),
            unwritten: false,
        });
    let mut coverage = Coverage::new(expected);
    for r in map.ranges().iter().filter(|r| r.length != 0) {
        coverage.push(AllocationRange {
            offset: r.offset,
            length: r.length,
            unwritten: false,
        })?;
    }
    coverage.finish()
}
pub fn export<P: SnapshotBackend, W: Write>(
    source: &mut attachment::Captured<'_, '_, P>,
    writer: &mut envelope::Writer<W>,
    ordinal: u64,
    path: &str,
    scratch: &mut [u8],
    limits: consumer::Options,
) -> Result<ExportReport, Error> {
    let result = export_inner(source, writer, ordinal, path, scratch, limits);
    if result.is_err() {
        writer.invalidate();
    }
    result
}
fn export_inner<P: SnapshotBackend, W: Write>(
    source: &mut attachment::Captured<'_, '_, P>,
    writer: &mut envelope::Writer<W>,
    ordinal: u64,
    path: &str,
    scratch: &mut [u8],
    limits: consumer::Options,
) -> Result<ExportReport, Error> {
    let sparse_ordinal = ordinal.checked_add(1).ok_or(Error::Limit)?;
    if scratch.is_empty() {
        return Err(Error::Limit);
    }
    let plan = consumer::prepare_source(source, limits, true)?;
    let wire = encode(path, plan.stat.size, &plan.allocations, limits)?;
    writer
        .start(
            &attachment::header(name(ordinal), crate::tar::Kind::File, wire.len() as u64),
            None,
        )
        .map_err(Error::Envelope)?;
    writer.write_payload(&wire).map_err(Error::Envelope)?;
    let contents = consumer::emit_source(
        source,
        writer,
        sparse_ordinal,
        path,
        scratch,
        limits.records,
        &plan,
    )?;
    Ok(ExportReport {
        contents,
        next_ordinal: sparse_ordinal.checked_add(1),
    })
}
fn read_layout<R: Read>(
    reader: &mut stream::Reader<R>,
    ordinal: u64,
    path: &str,
    limits: consumer::Options,
) -> Result<Layout, Error> {
    let expected = name(ordinal);
    let m = reader
        .next_member()
        .map_err(Error::Stream)?
        .ok_or(Error::Invalid)?;
    if !attachment::ordinary(&m, &expected) || m.sparse_size.is_some() {
        return Err(Error::Invalid);
    }
    let size = usize::try_from(m.size).map_err(|_| Error::Limit)?;
    if !reader.raw_path_is(&expected) {
        return Err(Error::Invalid);
    }
    if size > limits.records.bytes {
        return Err(Error::Limit);
    }
    let mut wire = Vec::new();
    wire.try_reserve_exact(size).map_err(|_| Error::Limit)?;
    wire.resize(size, 0);
    if reader.read_payload(&mut wire).map_err(Error::Stream)? != size {
        return Err(Error::Invalid);
    }
    let (actual, layout) = decode(&wire, limits)?;
    if actual != path {
        return Err(Error::Invalid);
    }
    Ok(layout)
}
pub fn restore<R: Read, P: RestoreBackend>(
    reader: &mut stream::Reader<R>,
    client: &mut RestoreClient<'_, P>,
    target: &Target<'_, P::Object>,
    scratch: &mut [u8],
    options: RestoreOptions,
    now: Timespec,
) -> Result<Report, Error> {
    let result = restore_inner(reader, client, target, scratch, options, now);
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
    options: RestoreOptions,
    now: Timespec,
) -> Result<Report, Error> {
    if !reader.can_publish() {
        return Err(Error::NeedsVerifiedReplay);
    }
    let sparse_ordinal = target.ordinal.checked_add(1).ok_or(Error::Limit)?;
    if scratch.is_empty() || options.limits.page_entries == 0 || options.limits.page_entries > 64 {
        return Err(Error::Limit);
    }
    let layout = read_layout(reader, target.ordinal, target.path, options.limits)?;
    let file = consumer::Target {
        path: target.path,
        object: target.object,
    };
    let map = consumer::prepare_restore(reader, client, &file, options.limits.map)?;
    if !reader.raw_path_is(&format!("files/GNUSparseFile.{sparse_ordinal}/payload")) {
        return Err(Error::Invalid);
    }
    match_written(&layout, &map)?;
    let reserved = layout.ranges.iter().filter(|r| r.unwritten);
    let (count, bytes) = reserved.clone().fold((0u64, 0u128), |(n, bytes), r| {
        (n + 1, bytes + r.length as u128)
    });
    if matches!(options.mode, Mode::PreserveAllocation) {
        if bytes > options.reservation_bytes
            || (bytes != 0 && options.reservation_chunk == 0)
            || (!layout.ranges.is_empty() && options.readback_entries == 0)
        {
            return Err(Error::Limit);
        }
        let empty = client
            .allocations(target.object, 0, 1)
            .map_err(Error::Destination)?;
        if !empty.ranges.is_empty() || !empty.eof {
            return Err(Error::Invalid);
        }
        for r in reserved {
            let mut offset = r.offset;
            let mut remaining = r.length;
            while remaining != 0 {
                let take = remaining.min(options.reservation_chunk);
                client
                    .reserve(target.object, offset, take, now)
                    .map_err(Error::Destination)?;
                remaining -= take;
                if remaining != 0 {
                    offset = offset.checked_add(take).ok_or(Error::Invalid)?;
                }
            }
        }
    }
    consumer::write_contents(reader, client, &file, scratch, &map, now)?;
    let stat = client.stat(target.object).map_err(Error::Destination)?;
    if stat.size != layout.logical_size {
        return Err(Error::Invalid);
    }
    let allocation = match options.mode {
        Mode::RecoverContents => Disposition::Discarded {
            ranges: count,
            bytes,
        },
        Mode::PreserveAllocation => {
            let mut coverage = Coverage::new(layout.ranges.iter().copied());
            let mut cursor = 0;
            loop {
                let page = client
                    .allocations(target.object, cursor, options.limits.page_entries)
                    .map_err(Error::Destination)?;
                if page.next > options.readback_entries {
                    return Err(Error::Limit);
                }
                for r in page.ranges {
                    coverage.push(r)?;
                }
                if page.eof {
                    break;
                }
                cursor = page.next;
            }
            coverage.finish()?;
            Disposition::Preserved
        }
    };
    Ok(Report {
        logical_bytes: layout.logical_size,
        written_bytes: map.data_bytes(),
        next_ordinal: sparse_ordinal.checked_add(1),
        allocation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> consumer::Options {
        consumer::Options {
            map: sparse::Limits {
                entries: 64,
                map_bytes: 4096,
                data_bytes: u64::MAX,
                logical_bytes: u64::MAX,
            },
            records: pax::Limits {
                bytes: 4096,
                records: 16,
                key_bytes: 64,
                value_bytes: 2048,
            },
            page_entries: 1,
        }
    }
    fn range(offset: u64, length: u64, unwritten: bool) -> AllocationRange {
        AllocationRange {
            offset,
            length,
            unwritten,
        }
    }
    #[test]
    fn allocation_codec_roundtrips_boundaries_and_refuses_every_truncation() {
        for ranges in [
            vec![],
            vec![range(0, 4096, false), range(u64::MAX - 4095, 4096, true)],
        ] {
            let wire = encode("files/é", u64::MAX, &ranges, options()).unwrap();
            assert_eq!(
                decode(&wire, options()).unwrap(),
                (
                    "files/é",
                    Layout {
                        logical_size: u64::MAX,
                        ranges
                    }
                )
            );
            for end in 0..wire.len() {
                assert!(decode(&wire[..end], options()).is_err(), "prefix {end}");
            }
            let mut limited = options();
            limited.records.bytes = wire.len() - 1;
            assert!(decode(&wire, limited).is_err());
        }
    }
    #[test]
    fn allocation_codec_rejects_schema_numeric_and_range_errors() {
        let valid = ["1", "files/file", "4096", "1", "0,4096,w\n"];
        let wire = |values: &[&str; 5]| {
            pax::encode(
                &std::array::from_fn::<_, 5, _>(|i| pax::Record {
                    key: KEYS[i],
                    value: values[i],
                }),
                options().records,
            )
            .unwrap()
        };
        for (field, bad) in [
            (0, "2"),
            (1, "../escape"),
            (1, "files/../escape"),
            (2, "01"),
            (2, "18446744073709551616"),
            (3, "0"),
            (3, "2"),
            (3, "18446744073709551615"),
            (4, "0,4096,w"),
            (4, "0,0,w\n"),
            (4, "0,4096,x\n"),
            (4, "0,4096,w,extra\n"),
            (4, "00,4096,w\n"),
            (4, "18446744073709551615,2,u\n"),
        ] {
            let mut values = valid;
            values[field] = bad;
            assert!(
                decode(&wire(&values), options()).is_err(),
                "field {field}: {bad}"
            );
        }
        let records: Vec<_> = KEYS
            .iter()
            .zip(valid)
            .map(|(key, value)| pax::Record { key, value })
            .collect();
        for missing in 0..5 {
            let mut records = records.clone();
            records.remove(missing);
            assert!(decode(
                &pax::encode(&records, options().records).unwrap(),
                options()
            )
            .is_err());
        }
        let mut unknown = records.clone();
        unknown[0].key = "AROS.allocation.future";
        assert!(decode(
            &pax::encode(&unknown, options().records).unwrap(),
            options()
        )
        .is_err());
        let mut duplicate = wire(&valid);
        duplicate.extend(pax::encode(&records[..1], options().records).unwrap());
        assert!(decode(&duplicate, options()).is_err());
        for ranges in [
            vec![range(1, 2, false), range(2, 1, true)],
            vec![range(2, 1, false), range(0, 1, true)],
            vec![range(u64::MAX, 2, true)],
        ] {
            assert!(encode("files/file", 0, &ranges, options()).is_err());
        }
        let mut limited = options();
        limited.map.entries = 0;
        assert!(decode(&wire(&valid), limited).is_err());
        limited = options();
        limited.map.logical_bytes = 4095;
        assert!(decode(&wire(&valid), limited).is_err());
    }
    #[test]
    fn allocation_coverage_compares_semantics_across_segmentation_and_u64_boundary() {
        let expected = vec![
            range(0, 4, false),
            range(4, 4, false),
            range(u64::MAX - 3, 4, true),
        ];
        let mut coverage = Coverage::new(expected.clone().into_iter());
        for r in [
            range(0, 8, false),
            range(u64::MAX - 3, 3, true),
            range(u64::MAX, 1, true),
        ] {
            coverage.push(r).unwrap();
        }
        coverage.finish().unwrap();
        for bad in [
            range(0, 9, false),
            range(1, 4, false),
            range(0, 4, true),
            range(0, 0, false),
        ] {
            assert!(Coverage::new(expected.clone().into_iter())
                .push(bad)
                .is_err());
        }
        assert!(Coverage::new(expected.into_iter()).finish().is_err());
        assert!(Coverage::new(std::iter::empty())
            .push(range(0, 1, false))
            .is_err());
    }
    #[test]
    fn allocation_binding_clips_written_tails_and_refuses_size_hole_or_state_changes() {
        let mut layout = Layout {
            logical_size: 4100,
            ranges: vec![range(4096, 4096, false), range(16384, 4096, true)],
        };
        let map = sparse::Map::new(
            4100,
            &[sparse::Range {
                offset: 4096,
                length: 4,
            }],
            options().map,
        )
        .unwrap();
        match_written(&layout, &map).unwrap();
        layout.logical_size += 1;
        assert!(match_written(&layout, &map).is_err());
        layout.logical_size -= 1;
        layout.ranges[0].unwritten = true;
        assert!(match_written(&layout, &map).is_err());
        layout.ranges[0].unwritten = false;
        layout.ranges[0].offset -= 1;
        assert!(match_written(&layout, &map).is_err());
    }
}
