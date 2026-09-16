//! Residual data-family combinations of the tiny-cache matrix: spilled
//! full-COW, reservation-initialization, preallocation and size-change cuts
//! whose unflushed tails exceed the exhaustive budget, the ambiguous
//! publication of each of those operations, and shared-run truncate. See
//! tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::{FileEditLimits, ObjectMetadata};
use afsplus_core::{CoreError, Volume};
use afsplus_format::object::{ObjectRecord, OBJECT_FLAG_EXTENT_TREE};
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};
use std::ops::Range;

const B: u64 = BS as u64;
/// Seeded sample size of every campaign in this file.
const SAMPLE: usize = 64;

fn lim(max_blocks: u64, max_records: usize) -> FileEditLimits {
    FileEditLimits {
        max_blocks,
        max_records,
    }
}

/// Literal byte value of the written block of record `index`.
fn value(index: u64) -> u8 {
    (index % 200 + 16) as u8
}

type Ranges = Vec<(u64, u64, bool)>;

/// Independent expectation of one object: exact bytes, allocation ranges and
/// the direct/extent-tree layout flag.
#[derive(Clone)]
struct Expect {
    bytes: Vec<u8>,
    ranges: Ranges,
    tree: bool,
}

fn live_ranges<D: BlockDevice>(volume: &mut Volume<D>, id: u64) -> Ranges {
    let mut out = Vec::new();
    let mut cursor = 0;
    loop {
        let page = volume.file_allocation_page(id, cursor, 64).unwrap();
        assert_eq!(page.next, cursor + page.ranges.len() as u64);
        out.extend(
            page.ranges
                .iter()
                .map(|range| (range.offset, range.length, range.unwritten)),
        );
        if page.eof {
            return out;
        }
        assert!(page.next > cursor);
        cursor = page.next;
    }
}

fn clip(window: &Range<usize>, size: usize) -> Range<usize> {
    let start = window.start.min(size.saturating_sub(1));
    start..window.end.min(size)
}

/// Literal bytes over the declared windows, the byte at end of file read with
/// a sentinel past it, the logical size, the layout flag and the complete
/// allocation enumeration. The enumeration is exact, so unchanged extents fix
/// every byte outside the windows; `check_whole_file` reads the complete
/// literal on the recording.
fn check_file<D: BlockDevice>(
    volume: &mut Volume<D>,
    id: u64,
    expect: &Expect,
    windows: &[Range<usize>],
    context: &str,
) -> ObjectRecord {
    let size = expect.bytes.len();
    for window in windows {
        let window = clip(window, size);
        let mut bytes = vec![0xa5; window.len()];
        let read = volume
            .read_file_at(id, window.start as u64, &mut bytes)
            .unwrap();
        assert_eq!(read, window.len(), "{context}: object {id} read length");
        assert!(
            bytes == expect.bytes[window.clone()],
            "{context}: object {id} bytes at {}",
            window.start
        );
    }
    let mut tail = [0xa5u8; 2];
    assert_eq!(
        volume.read_file_at(id, size as u64 - 1, &mut tail).unwrap(),
        1,
        "{context}: object {id} EOF"
    );
    assert_eq!(
        (tail[0], tail[1]),
        (expect.bytes[size - 1], 0xa5),
        "{context}: object {id} EOF sentinel"
    );
    let record = volume.stat(id).unwrap().unwrap();
    assert_eq!(
        record.size_bytes, size as u64,
        "{context}: object {id} size"
    );
    assert_eq!(
        record.flags & OBJECT_FLAG_EXTENT_TREE != 0,
        expect.tree,
        "{context}: object {id} layout"
    );
    assert_eq!(
        live_ranges(volume, id),
        expect.ranges,
        "{context}: object {id} allocation"
    );
    let allocated: u64 = expect.ranges.iter().map(|range| range.1).sum();
    assert_eq!(record.allocated_bytes, allocated, "{context}: object {id}");
    assert_eq!(record.data_blocks * B, allocated, "{context}: object {id}");
    record
}

/// Complete literal bytes of the live object and of its captured view.
fn check_whole_file<D: BlockDevice>(
    volume: &mut Volume<D>,
    snapshot: u64,
    id: u64,
    live: &Expect,
    captured: &Expect,
    context: &str,
) {
    assert!(
        volume.read_file(id).unwrap() == live.bytes,
        "{context}: object {id} complete bytes"
    );
    let view = volume.snapshot_open(snapshot).unwrap();
    let mut bytes = vec![0xa5; captured.bytes.len() + 1];
    assert_eq!(
        volume
            .snapshot_read_file_at(&view, id, 0, &mut bytes)
            .unwrap(),
        captured.bytes.len(),
        "{context}: captured complete length of {id}"
    );
    assert!(
        bytes[..captured.bytes.len()] == captured.bytes[..],
        "{context}: captured complete bytes of {id}"
    );
    assert_eq!(
        bytes[captured.bytes.len()],
        0xa5,
        "{context}: captured complete sentinel of {id}"
    );
}

/// Captured metadata, windowed bytes with an EOF sentinel and captured ranges.
fn check_captured<D: BlockDevice>(
    volume: &mut Volume<D>,
    snapshot: u64,
    id: u64,
    expect: &Expect,
    metadata: ObjectMetadata,
    windows: &[Range<usize>],
    context: &str,
) {
    let view = volume.snapshot_open(snapshot).unwrap();
    assert_eq!(
        volume.snapshot_stat(&view, id).unwrap(),
        Some(metadata),
        "{context}: captured metadata of {id}"
    );
    let size = expect.bytes.len();
    for window in windows {
        let window = clip(window, size);
        let mut bytes = vec![0xa5; window.len()];
        let read = volume
            .snapshot_read_file_at(&view, id, window.start as u64, &mut bytes)
            .unwrap();
        assert_eq!(
            read,
            window.len(),
            "{context}: captured read length of {id}"
        );
        assert!(
            bytes == expect.bytes[window.clone()],
            "{context}: captured bytes of {id} at {}",
            window.start
        );
    }
    let mut tail = [0xa5u8; 2];
    assert_eq!(
        volume
            .snapshot_read_file_at(&view, id, size as u64 - 1, &mut tail)
            .unwrap(),
        1,
        "{context}: captured EOF of {id}"
    );
    assert_eq!(
        (tail[0], tail[1]),
        (expect.bytes[size - 1], 0xa5),
        "{context}: captured EOF sentinel of {id}"
    );
    let mut captured = Vec::new();
    let mut cursor = 0;
    loop {
        let page = volume
            .snapshot_allocation_page(&view, id, cursor, 64)
            .unwrap();
        captured.extend(
            page.ranges
                .iter()
                .map(|range| (range.offset, range.length, range.unwritten)),
        );
        if page.eof {
            break;
        }
        assert!(page.next > cursor);
        cursor = page.next;
    }
    assert_eq!(
        captured, expect.ranges,
        "{context}: captured allocation of {id}"
    );
}

fn check_target_metadata(
    record: &ObjectRecord,
    old: &ObjectRecord,
    content: bool,
    generation: u64,
    context: &str,
) {
    assert_eq!(record.changed, ts(6), "{context}: change time");
    assert_eq!(
        record.modified,
        if content { ts(6) } else { old.modified },
        "{context}: modification time"
    );
    assert_eq!(
        record.content_generation,
        if content {
            generation
        } else {
            old.content_generation
        },
        "{context}: content generation"
    );
    assert_eq!(
        (record.object_type, record.link_count, record.protection),
        (old.object_type, old.link_count, old.protection),
        "{context}: preserved record fields"
    );
    assert_eq!(record.created, old.created, "{context}: creation time");
}

// ---------------------------------------------------------------------------
// Wide fixtures: one written sparse tree and one unwritten reservation tree.
// ---------------------------------------------------------------------------

/// Written sparse tree: one written block at every second logical block.
fn written_tree(volume: &mut Volume<MemoryBackend>, records: u64) -> u64 {
    let file = volume.create_file_in_root("wide", b"", ts(1)).unwrap();
    for index in 0..records {
        volume
            .write_file_at_bounded(file, 2 * index * B, &[value(index); BS], ts(1), lim(1, 8))
            .unwrap();
    }
    file
}

fn written_old(records: u64) -> Expect {
    let count = records as usize;
    let mut bytes = vec![0; (2 * count - 1) * BS];
    for index in 0..count {
        bytes[2 * index * BS..(2 * index + 1) * BS].fill(value(index as u64));
    }
    Expect {
        bytes,
        ranges: (0..records)
            .map(|index| (2 * index * B, B, false))
            .collect(),
        tree: true,
    }
}

/// Reservation tree: a one-block unwritten reservation at every third block.
fn reservation_tree(volume: &mut Volume<MemoryBackend>, records: u64) -> u64 {
    let file = volume.create_file_in_root("resv", b"", ts(2)).unwrap();
    for index in 0..records {
        volume
            .preallocate_file_bounded(file, 3 * index * B, B, ts(2), lim(1, 8))
            .unwrap();
    }
    volume.truncate_file(file, 3 * records * B, ts(3)).unwrap();
    file
}

fn reservation_old(records: u64) -> Expect {
    Expect {
        bytes: vec![0; 3 * records as usize * BS],
        ranges: (0..records).map(|index| (3 * index * B, B, true)).collect(),
        tree: true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Full-COW write in the middle of the written tree.
    CowWrite,
    /// Unbounded reservation-initializing write of the reservation tree.
    ReservationWrite,
    /// Bounded write whose window spans eight extent leaves.
    WindowWrite,
    /// Bounded preallocation of every hole of the written tree.
    ReserveHoles,
    /// Unbounded shrink to half the written tree with a nine-byte tail.
    Shrink,
    /// Bounded shrink to the same size.
    BoundedShrink,
    /// Unbounded sparse growth to four times the size.
    Growth,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::CowWrite => "spilled full-COW write",
            Op::ReservationWrite => "spilled reservation initialization",
            Op::WindowWrite => "bounded write over eight extent leaves",
            Op::ReserveHoles => "spilled hole reservation",
            Op::Shrink => "spilled shrink",
            Op::BoundedShrink => "spilled bounded shrink",
            Op::Growth => "sparse growth",
        }
    }

    /// The reservation tree is the subject only of the reservation write.
    fn reservation(self) -> bool {
        self == Op::ReservationWrite
    }

    /// The multi-leaf window fixture holds the written tree alone.
    fn paired(self) -> bool {
        self != Op::WindowWrite
    }

    /// Preallocation keeps the modification time and content generation.
    fn changes_content(self) -> bool {
        self != Op::ReserveHoles
    }
}

/// Extent records one leaf holds, from the encoded item size.
fn leaf_capacity() -> u64 {
    let (key, value) = afsplus_core::extent_map::encode_extent(afsplus_core::extent_map::Extent {
        logical_start: 0,
        physical_start: 1,
        block_count: 1,
        flags: 0,
    })
    .unwrap();
    afsplus_format::tree::TreeNode::fixed_item_capacity(BS, key.len(), value.len()).unwrap() as u64
}

/// Logical blocks of the bounded window that spans eight extent leaves.
fn window_blocks() -> u64 {
    2 * 8 * leaf_capacity()
}

struct DataState {
    records: u64,
    wide: u64,
    resv: Option<u64>,
    snapshot: u64,
    objects: Vec<ObjectRecord>,
    generation: u64,
}

impl DataState {
    /// Subject and bystander object identities.
    fn ids(&self) -> Vec<u64> {
        std::iter::once(self.wide).chain(self.resv).collect()
    }
}

struct DataFamily {
    op: Op,
    /// Records per file of the forced-eviction fixture, chosen per profile.
    records: u64,
    /// Literal resident staged-node demand of that fixture at unlimited.
    demand: u64,
}

/// Records per file of the plain and ambiguous fixtures.
const SMALL: u64 = 4;

impl DataFamily {
    fn records(&self, variant: Variant) -> u64 {
        if variant == Variant::Eviction {
            self.records
        } else {
            SMALL
        }
    }

    fn target(&self, state: &DataState) -> u64 {
        if self.op.reservation() {
            state.resv.expect("the reservation write needs its tree")
        } else {
            state.wide
        }
    }

    /// Independent expectation of `id` before the operation publishes.
    fn old(&self, state: &DataState, id: u64) -> Expect {
        if id == state.wide {
            written_old(state.records)
        } else {
            reservation_old(state.records)
        }
    }

    /// Independent expectation of the subject after the operation publishes.
    fn published(&self, state: &DataState) -> Expect {
        let records = state.records;
        let (count, middle) = (records as usize, (records / 2) as usize);
        let mut expect = self.old(state, self.target(state));
        match self.op {
            Op::CowWrite => {
                expect.bytes[2 * middle * BS + 7..2 * middle * BS + 107].fill(0xee);
            }
            Op::ReservationWrite => {
                expect.bytes[3 * middle * BS + 7..3 * middle * BS + 107].fill(0x5a);
                expect.ranges[middle].2 = false;
            }
            Op::WindowWrite => {
                let window = window_blocks();
                expect.bytes[..window as usize * BS].fill(0xd7);
                let tail: Ranges = expect
                    .ranges
                    .iter()
                    .copied()
                    .filter(|range| range.0 >= window * B)
                    .collect();
                expect.ranges = std::iter::once((0, window * B, false))
                    .chain(tail)
                    .collect();
            }
            Op::ReserveHoles => {
                expect.ranges = (0..2 * records)
                    .map(|index| (index * B, B, index % 2 == 1))
                    .collect();
            }
            Op::Shrink | Op::BoundedShrink => {
                // Block `records` is written, so its first nine bytes survive.
                expect.bytes.truncate(count * BS + 9);
                expect.ranges.truncate(count / 2 + 1);
            }
            Op::Growth => expect.bytes.resize(4 * count * BS, 0),
        }
        expect
    }
}

/// Byte windows every image checks literally, live and captured: the first
/// and last block of the object and the window the operation changes.
fn windows(family: &DataFamily, state: &DataState, id: u64, size: usize) -> Vec<Range<usize>> {
    let (count, middle) = (state.records as usize, (state.records / 2) as usize);
    let mut windows = vec![0..BS, size.saturating_sub(BS)..size];
    if id != family.target(state) {
        return windows;
    }
    match family.op {
        Op::CowWrite => windows.push(2 * middle * BS..(2 * middle + 1) * BS),
        Op::ReservationWrite => windows.push(3 * middle * BS..(3 * middle + 1) * BS),
        Op::WindowWrite => {
            let window = window_blocks() as usize;
            windows.push(0..2 * BS);
            windows.push((window - 1) * BS..(window + 1) * BS);
        }
        Op::ReserveHoles => windows.push(0..3 * BS),
        Op::Shrink | Op::BoundedShrink => windows.push((count - 1) * BS..(count + 2) * BS),
        Op::Growth => windows.push((2 * count - 2) * BS..(2 * count + 1) * BS),
    }
    windows
}

/// Blocks of the image: both trees, the snapshot's retained copies and the
/// metadata trees fit with headroom.
fn span(records: u64, paired: bool) -> u64 {
    let factor = if paired { 16 } else { 8 };
    (factor * records).next_power_of_two().max(1024)
}

/// Builds the fixture files and returns the subject state.
fn build(volume: &mut Volume<MemoryBackend>, op: Op, records: u64) -> DataState {
    let wide = written_tree(volume, records);
    let resv = op.paired().then(|| reservation_tree(volume, records));
    let snapshot = volume.snapshot_create(ts(4)).unwrap();
    let objects = std::iter::once(wide)
        .chain(resv)
        .map(|id| volume.stat(id).unwrap().unwrap())
        .collect();
    DataState {
        records,
        wide,
        resv,
        snapshot,
        objects,
        generation: volume.generation(),
    }
}

impl Family for DataFamily {
    type State = DataState;

    fn name(&self) -> &'static str {
        self.op.name()
    }

    fn format(&self, variant: Variant) -> Format {
        let blocks = span(self.records(variant), self.op.paired());
        Format::new(blocks, blocks as u32)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> DataState {
        build(volume, self.op, self.records(variant))
    }

    fn snapshot(&self, state: &DataState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &DataState) -> Vec<(u64, Vec<u8>)> {
        state
            .ids()
            .into_iter()
            .map(|id| (id, self.old(state, id).bytes))
            .collect()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DataState,
    ) -> Result<(), CoreError> {
        let (records, middle, now) = (state.records, state.records / 2, ts(6));
        let target = self.target(state);
        match self.op {
            Op::CowWrite => volume.write_file_at(target, 2 * middle * B + 7, &[0xee; 100], now),
            Op::ReservationWrite => {
                volume.write_file_at(target, 3 * middle * B + 7, &[0x5a; 100], now)
            }
            Op::WindowWrite => {
                let window = window_blocks();
                volume.write_file_at_bounded(
                    target,
                    0,
                    &vec![0xd7; window as usize * BS],
                    now,
                    lim(window, window as usize + 4),
                )
            }
            Op::ReserveHoles => volume.preallocate_file_bounded(
                target,
                0,
                2 * records * B,
                now,
                lim(2 * records, 2 * records as usize + 2),
            ),
            Op::Shrink => volume.truncate_file(target, records * B + 9, now),
            Op::BoundedShrink => volume.truncate_file_bounded(
                target,
                records * B + 9,
                now,
                lim(records, records as usize + 2),
            ),
            Op::Growth => volume.truncate_file(target, 4 * records * B, now),
        }
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DataState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let target = self.target(state);
        let names: Vec<&str> = if state.resv.is_some() {
            vec!["wide", "resv"]
        } else {
            vec!["wide"]
        };
        for (name, id) in names.iter().zip(state.ids()) {
            assert_eq!(
                volume.lookup_root(name).unwrap(),
                Some(id),
                "{context}: {name}"
            );
        }
        assert_eq!(
            volume.list_root().unwrap().len(),
            names.len(),
            "{context}: root count"
        );
        for (index, id) in state.ids().into_iter().enumerate() {
            let old = self.old(state, id);
            if delta == 1 && id == target {
                let new = self.published(state);
                let live = windows(self, state, id, new.bytes.len());
                let record = check_file(volume, id, &new, &live, context);
                check_target_metadata(
                    &record,
                    &state.objects[index],
                    self.op.changes_content(),
                    state.generation + 1,
                    context,
                );
            } else {
                let live = windows(self, state, id, old.bytes.len());
                assert_eq!(
                    check_file(volume, id, &old, &live, context),
                    state.objects[index],
                    "{context}: unchanged record of {id}"
                );
            }
            let captured = windows(self, state, id, old.bytes.len());
            check_captured(
                volume,
                state.snapshot,
                id,
                &old,
                ObjectMetadata::from(state.objects[index]),
                &captured,
                context,
            );
        }
    }

    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &DataState,
        _variant: Variant,
    ) {
        assert_eq!(
            volume
                .last_commit_stats()
                .unwrap()
                .data_blocks_overwritten_in_place,
            0,
            "{}: the default policy overwrites no block in place",
            self.op.name()
        );
        let target = self.target(state);
        for id in state.ids() {
            let old = self.old(state, id);
            let live = if id == target {
                self.published(state)
            } else {
                old.clone()
            };
            check_whole_file(volume, state.snapshot, id, &live, &old, self.op.name());
        }
    }

    fn eviction_demand(&self) -> u64 {
        self.demand
    }
}

// ---------------------------------------------------------------------------
// Shared-run truncate: the clone's private block splits one run into a shared
// prefix, a private peer block and a shared suffix.
// ---------------------------------------------------------------------------

const SHARED_BLOCKS: usize = 4;
/// Retired blocks the truncate needs: the removed tail block and the rewrite
/// of the partial block; one block is one too few.
const SHARED_RETIREMENT: u64 = 2;

struct SharedTruncate;

struct SharedState {
    source: u64,
    peer: u64,
    origin: Vec<u8>,
    peer_bytes: Vec<u8>,
    snapshot: u64,
    objects: [ObjectRecord; 2],
    generation: u64,
    budget: u64,
}

impl SharedTruncate {
    fn source_expect(&self, state: &SharedState, delta: u64) -> Expect {
        let full = SHARED_BLOCKS as u64 * B;
        if delta == 0 {
            Expect {
                bytes: state.origin.clone(),
                ranges: vec![(0, full, false)],
                tree: true,
            }
        } else {
            let mut bytes = state.origin.clone();
            bytes.truncate(2 * BS + 9);
            Expect {
                bytes,
                ranges: vec![(0, 2 * B, false), (2 * B, B, false)],
                tree: true,
            }
        }
    }

    fn peer_expect(&self, state: &SharedState) -> Expect {
        Expect {
            bytes: state.peer_bytes.clone(),
            ranges: vec![(0, B, false), (B, B, false), (2 * B, 2 * B, false)],
            tree: true,
        }
    }
}

impl Family for SharedTruncate {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "shared-run truncate"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(512, 512)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        let origin = vec![0x55; SHARED_BLOCKS * BS];
        let source = volume
            .create_file_in_root("source", &origin, ts(2))
            .unwrap();
        let peer = volume
            .clone_file(source, afsplus_format::OBJECT_ROOT, "peer", ts(3))
            .unwrap();
        volume
            .write_file_at(peer, B, &vec![0x99; BS], ts(4))
            .unwrap();
        let mut peer_bytes = origin.clone();
        peer_bytes[BS..2 * BS].fill(0x99);
        let snapshot = volume.snapshot_create(ts(5)).unwrap();
        let budget = if variant == Variant::Refusal {
            SHARED_RETIREMENT - 1
        } else {
            SHARED_RETIREMENT
        };
        SharedState {
            source,
            peer,
            origin,
            peer_bytes,
            snapshot,
            objects: [
                volume.stat(source).unwrap().unwrap(),
                volume.stat(peer).unwrap().unwrap(),
            ],
            generation: volume.generation(),
            budget,
        }
    }

    fn snapshot(&self, state: &SharedState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &SharedState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.source, state.origin.clone()),
            (state.peer, state.peer_bytes.clone()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
    ) -> Result<(), CoreError> {
        volume.truncate_file_bounded(
            state.source,
            2 * B + 9,
            ts(6),
            lim(state.budget, SHARED_BLOCKS + 2),
        )
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.lookup_root("source").unwrap(),
            Some(state.source),
            "{context}: source name"
        );
        assert_eq!(
            volume.lookup_root("peer").unwrap(),
            Some(state.peer),
            "{context}: peer name"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            2,
            "{context}: root count"
        );
        let whole = [Range {
            start: 0,
            end: SHARED_BLOCKS * BS,
        }];
        let record = check_file(
            volume,
            state.source,
            &self.source_expect(state, delta),
            &whole,
            context,
        );
        if delta == 1 {
            check_target_metadata(
                &record,
                &state.objects[0],
                true,
                state.generation + 1,
                context,
            );
        } else {
            assert_eq!(record, state.objects[0], "{context}: unchanged source");
        }
        let peer = self.peer_expect(state);
        assert_eq!(
            check_file(volume, state.peer, &peer, &whole, context),
            state.objects[1],
            "{context}: peer record"
        );
        check_captured(
            volume,
            state.snapshot,
            state.source,
            &self.source_expect(state, 0),
            ObjectMetadata::from(state.objects[0]),
            &whole,
            context,
        );
        check_captured(
            volume,
            state.snapshot,
            state.peer,
            &peer,
            ObjectMetadata::from(state.objects[1]),
            &whole,
            context,
        );
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(
            error,
            CoreError::PrototypeLimit("file edit retirement block budget exhausted")
        )
    }

    /// The specified corrective step raises the retirement budget to the
    /// measured demand; the fixture publishes nothing.
    fn relieve<D: BlockDevice>(&self, _volume: &mut Volume<D>, state: &mut SharedState) -> u64 {
        state.budget = SHARED_RETIREMENT;
        0
    }
}

// ---------------------------------------------------------------------------
// Measured record counts, staged demands and the generated profile tests.
// ---------------------------------------------------------------------------

/// Records per file of the forced-eviction fixture of `op` at `pages`.
fn records_for(op: Op, pages: usize) -> u64 {
    match (op, pages) {
        (_, 2) => 40,
        // Four staged nodes at 120 and at 160 records fit the four-page
        // profile, so the reservation write needs the wider fixture to evict.
        (Op::ReservationWrite, _) => 500,
        (_, 4) => 120,
        _ => 500,
    }
}

/// Measured resident staged-node demand of each fixture at the unlimited
/// profile; the driver asserts it and derives the spill expectation from it.
fn demand(op: Op, records: u64) -> u64 {
    match (op, records) {
        (_, 40) => 3,
        (Op::CowWrite, 120) => 6,
        (Op::ReserveHoles, 120) => 5,
        (Op::Shrink, 120) | (Op::BoundedShrink, 120) => 7,
        (Op::Growth, 120) => 5,
        (Op::CowWrite, 500) | (Op::ReservationWrite, 500) => 10,
        (Op::ReserveHoles, 500) => 12,
        (Op::Shrink, 500) | (Op::BoundedShrink, 500) => 15,
        (Op::Growth, 500) => 4,
        other => panic!("no measured staged demand for {other:?}"),
    }
}

fn evicting(op: Op, pages: usize) -> DataFamily {
    let records = records_for(op, pages);
    DataFamily {
        op,
        records,
        demand: demand(op, records),
    }
}

fn small(op: Op) -> DataFamily {
    DataFamily {
        op,
        records: SMALL,
        demand: 0,
    }
}

/// The bounded window spans eight extent leaves and leaves a sparse tail.
fn window_family() -> DataFamily {
    assert_eq!(leaf_capacity(), 100, "measured extent-leaf capacity");
    DataFamily {
        op: Op::WindowWrite,
        records: 8 * leaf_capacity() + 300,
        demand: 24,
    }
}

crate::profile_tests!(cow_write_eviction, |pages| matrix::eviction_sampled(
    &evicting(Op::CowWrite, pages),
    pages,
    SAMPLE,
    0x00c0_0d17_e001
));
crate::profile_tests!(cow_write_ambiguous, |pages| matrix::ambiguous(
    &small(Op::CowWrite),
    pages,
    Variant::Plain
));
crate::profile_tests!(
    reservation_write_eviction,
    |pages| matrix::eviction_sampled(
        &evicting(Op::ReservationWrite, pages),
        pages,
        SAMPLE,
        0x00c0_0d17_e002
    )
);
crate::profile_tests!(reservation_write_ambiguous, |pages| matrix::ambiguous(
    &small(Op::ReservationWrite),
    pages,
    Variant::Plain
));
crate::profile_tests!(window_write_eviction, |pages| matrix::eviction_recorded(
    &window_family(),
    pages
));
crate::profile_tests!(reserve_holes_eviction, |pages| matrix::eviction_sampled(
    &evicting(Op::ReserveHoles, pages),
    pages,
    SAMPLE,
    0x00c0_0d17_e003
));
crate::profile_tests!(reserve_holes_ambiguous, |pages| matrix::ambiguous(
    &small(Op::ReserveHoles),
    pages,
    Variant::Plain
));
crate::profile_tests!(shrink_eviction, |pages| matrix::eviction_sampled(
    &evicting(Op::Shrink, pages),
    pages,
    SAMPLE,
    0x00c0_0d17_e004
));
crate::profile_tests!(bounded_shrink_eviction, |pages| matrix::eviction_sampled(
    &evicting(Op::BoundedShrink, pages),
    pages,
    SAMPLE,
    0x00c0_0d17_e005
));
crate::profile_tests!(shrink_ambiguous, |pages| matrix::ambiguous(
    &small(Op::Shrink),
    pages,
    Variant::Plain
));
crate::profile_tests!(growth_eviction, |pages| matrix::eviction(
    &evicting(Op::Growth, pages),
    pages,
    Some(12)
));
crate::profile_tests!(growth_ambiguous, |pages| matrix::ambiguous(
    &small(Op::Growth),
    pages,
    Variant::Plain
));
crate::profile_tests!(shared_truncate_cuts_and_faults, |pages| matrix::plain(
    &SharedTruncate,
    pages,
    12
));
crate::profile_tests!(shared_truncate_retained, |pages| matrix::retained(
    &SharedTruncate,
    pages,
    12
));
crate::profile_tests!(shared_truncate_refusal, |pages| matrix::refusal(
    &SharedTruncate,
    pages
));

/// Measured limit: reservation-initializing writes stage four nodes at both
/// 120 and 160 records, so the four-page profile does not evict them there.
#[test]
fn reservation_initialization_stages_four_nodes_at_120_and_160_records() {
    for records in [120, 160] {
        let family = DataFamily {
            op: Op::ReservationWrite,
            records,
            demand: 4,
        };
        let mut volume = matrix::open(family.format(Variant::Eviction).device(), usize::MAX);
        let state = family.setup(&mut volume, Variant::Eviction);
        let base = volume.into_device();
        for pages in [4, usize::MAX] {
            let mut volume = matrix::open(base.clone(), pages);
            family.apply(&mut volume, &state).unwrap();
            let stats = volume.last_commit_stats().unwrap().tree_mutations;
            assert_eq!(
                (stats.max_resident_staged_nodes, stats.staged_spill_writes),
                (4, 0),
                "{records} records at pages={pages}"
            );
            family.verify(&mut volume, &state, Variant::Eviction, 1, "measured limit");
        }
    }
}
