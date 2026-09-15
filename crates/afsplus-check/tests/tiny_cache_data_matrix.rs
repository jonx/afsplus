//! Data-write and size-change cache-profile oracles; see tiny_cache_matrix.md.
use afsplus_block::{
    for_each_crash_state_with_budget, BlockDevice, FaultBackend, FaultPlan, MemoryBackend,
    RecordedOp, RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::extent_map::{self, Extent, EXTENT_UNWRITTEN};
use afsplus_core::volume::{DataUpdatePolicy, FileEditLimits, ObjectMetadata, SnapshotWorkLimits};
use afsplus_core::{
    mkfs_with_options, mount_with_snapshot_limits, CoreError, MkfsOptions, MkfsParams, MountMode,
    MountOptions, NamePolicy, Volume,
};
use afsplus_format::object::{ObjectRecord, OBJECT_FLAG_EXTENT_TREE};
use afsplus_format::{Timespec, OBJECT_ROOT};
use std::{collections::BTreeMap, num::NonZeroUsize, time::Instant};

const PROFILES: [usize; 4] = [2, 4, 8, usize::MAX];
const BS: usize = 4096;
const B: u64 = 4096;
/// Explicit full-subset budget: 2^12 subsets at the longest modeled cut.
const CUT_BUDGET: usize = 12;

fn time(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 123,
    }
}
fn lim(max_blocks: u64, max_records: usize) -> FileEditLimits {
    FileEditLimits {
        max_blocks,
        max_records,
    }
}
fn open<D: BlockDevice>(dev: D, pages: usize) -> Volume<D> {
    open_mode(dev, pages, MountMode::ReadWrite)
}
fn open_mode<D: BlockDevice>(dev: D, pages: usize, mode: MountMode) -> Volume<D> {
    let volume = mount_with_snapshot_limits(
        dev,
        MountOptions {
            mode,
            tree_cache_pages: NonZeroUsize::new(pages),
        },
        SnapshotWorkLimits {
            max_edit_records: 4096,
            max_views: 128,
            reclaim_records: 8,
        },
    )
    .unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    volume
}
fn formatted(blocks: u64) -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, blocks);
    mkfs_with_options(
        &mut dev,
        &MkfsParams {
            uuid: [0xc2; 16],
            label: "Data matrix".into(),
            region_size: blocks as u32,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(0),
        },
        MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    dev
}
fn clean<D: BlockDevice>(dev: &mut D) {
    let report = check_device(dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    // Only the stopped intent-log tail is an admissible crash artifact;
    // older selectable checkpoint findings must fail this oracle.
    assert!(
        report
            .warnings
            .iter()
            .all(|w| w.starts_with("intent log tail:")),
        "{:?}",
        report.warnings
    );
}
fn fill(len: usize, runs: &[(usize, usize, u8)]) -> Vec<u8> {
    let mut bytes = vec![0; len];
    for &(start, end, value) in runs {
        bytes[start..end].fill(value);
    }
    bytes
}
fn counts(log: &[RecordedOp]) -> (u64, u64) {
    let writes = log
        .iter()
        .filter(|op| matches!(op, RecordedOp::Write { .. }))
        .count() as u64;
    (writes, log.len() as u64 - writes)
}
fn longest_tail(log: &[RecordedOp]) -> usize {
    log.split(|op| matches!(op, RecordedOp::Flush))
        .map(<[RecordedOp]>::len)
        .max()
        .unwrap_or(0)
}
fn record(
    base: &MemoryBackend,
    pages: usize,
    apply: impl FnOnce(&mut Volume<RecordingBackend<MemoryBackend>>),
) -> (MemoryBackend, Vec<RecordedOp>) {
    let mut volume = open(RecordingBackend::new(base.clone()), pages);
    apply(&mut volume);
    volume.into_device().into_parts()
}

type Ranges = Vec<(u64, u64, bool)>;
#[derive(Clone)]
struct Expect {
    bytes: Vec<u8>,
    ranges: Ranges,
    tree: bool,
}
fn live_ranges<D: BlockDevice>(v: &mut Volume<D>, id: u64) -> Ranges {
    let mut out = Vec::new();
    let mut cursor = 0;
    loop {
        let page = v.file_allocation_page(id, cursor, 64).unwrap();
        assert_eq!(page.next, cursor + page.ranges.len() as u64);
        out.extend(
            page.ranges
                .iter()
                .map(|r| (r.offset, r.length, r.unwritten)),
        );
        if page.eof {
            return out;
        }
        assert!(page.next > cursor);
        cursor = page.next;
    }
}
/// Exact bytes, logical size, direct/tree layout and allocation ranges.
fn check_file<D: BlockDevice>(v: &mut Volume<D>, id: u64, e: &Expect) -> ObjectRecord {
    assert!(v.read_file(id).unwrap() == e.bytes, "object {id} bytes");
    let record = v.stat(id).unwrap().unwrap();
    assert_eq!(record.size_bytes, e.bytes.len() as u64, "object {id} size");
    assert_eq!(
        record.flags & OBJECT_FLAG_EXTENT_TREE != 0,
        e.tree,
        "object {id} layout"
    );
    assert_eq!(live_ranges(v, id), e.ranges, "object {id} allocation");
    let allocated: u64 = e.ranges.iter().map(|r| r.1).sum();
    assert_eq!(record.allocated_bytes, allocated);
    assert_eq!(record.data_blocks * B, allocated);
    record
}
/// Exact captured metadata, bytes (with an untouched sentinel) and layout.
fn check_captured<D: BlockDevice>(
    v: &mut Volume<D>,
    snapshot: u64,
    id: u64,
    e: &Expect,
    metadata: ObjectMetadata,
) {
    let view = v.snapshot_open(snapshot).unwrap();
    assert_eq!(v.snapshot_stat(&view, id).unwrap(), Some(metadata));
    let mut bytes = vec![0xa5; e.bytes.len() + 1];
    assert_eq!(
        v.snapshot_read_file_at(&view, id, 0, &mut bytes).unwrap(),
        e.bytes.len()
    );
    assert!(bytes[..e.bytes.len()] == e.bytes[..], "captured {id} bytes");
    assert_eq!(bytes[e.bytes.len()], 0xa5);
    let mut captured = Vec::new();
    let mut cursor = 0;
    loop {
        let page = v.snapshot_allocation_page(&view, id, cursor, 64).unwrap();
        captured.extend(
            page.ranges
                .iter()
                .map(|r| (r.offset, r.length, r.unwritten)),
        );
        if page.eof {
            break;
        }
        assert!(page.next > cursor);
        cursor = page.next;
    }
    assert_eq!(captured, e.ranges, "captured {id} allocation");
}
fn load_extents<D: BlockDevice>(v: &mut Volume<D>, id: u64) -> Vec<Extent> {
    let record = v.stat(id).unwrap().unwrap();
    assert_ne!(record.flags & OBJECT_FLAG_EXTENT_TREE, 0);
    let geometry = v.ident().geometry();
    let generation = v.generation();
    extent_map::load_all(v.device_mut(), &geometry, record.data_root, id, generation)
        .unwrap()
        .extents
}
fn check_target_metadata(
    record: &ObjectRecord,
    old: &ObjectRecord,
    content: bool,
    generation: u64,
) {
    assert_eq!(record.changed, time(6));
    assert_eq!(
        record.modified,
        if content { time(6) } else { old.modified }
    );
    assert_eq!(
        record.content_generation,
        if content {
            generation
        } else {
            old.content_generation
        }
    );
    assert_eq!(
        (record.object_type, record.link_count, record.protection),
        (old.object_type, old.link_count, old.protection)
    );
    assert_eq!(record.created, old.created);
}

type FaultVolume = Volume<FaultBackend<MemoryBackend>>;

/// Injects a before-write error at every recorded write and an error at every
/// recorded flush. Returns the number of injected failures.
fn fault_matrix(
    base: &MemoryBackend,
    pages: usize,
    log: &[RecordedOp],
    apply: &dyn Fn(&mut FaultVolume) -> Result<(), CoreError>,
    unchanged: &dyn Fn(&mut FaultVolume),
    retried: &dyn Fn(&mut FaultVolume),
    exact: &dyn Fn(&mut FaultVolume, bool),
) -> u64 {
    let (writes, flushes) = counts(log);
    let plans = (0..writes)
        .map(|i| FaultPlan {
            fail_write_index: Some(i),
            ..Default::default()
        })
        .chain((0..flushes).map(|i| FaultPlan {
            fail_flush_index: Some(i),
            ..Default::default()
        }));
    for plan in plans {
        let mut v = open(FaultBackend::new(base.clone(), plan), pages);
        let error = apply(&mut v).expect_err("injected fault must fail the mutation");
        assert!(matches!(error, CoreError::Block(_)), "{plan:?}: {error:?}");
        assert!(v.device_mut().tripped());
        // The last write is the checkpoint and the last flush its barrier.
        let checkpoint_write = plan.fail_write_index == Some(writes - 1);
        let final_barrier = plan.fail_flush_index == Some(flushes - 1);
        if checkpoint_write || final_barrier {
            assert!(
                matches!(apply(&mut v), Err(CoreError::WindowPoisoned)),
                "pages={pages} {plan:?}: publication failure must require remount"
            );
        } else {
            unchanged(&mut v);
            apply(&mut v).unwrap_or_else(|e| panic!("pages={pages} {plan:?} retry: {e:?}"));
            retried(&mut v);
        }
        // FaultBackend drops the failed checkpoint write; after a failed final
        // barrier the complete checkpoint block is on the memory device.
        let committed = !checkpoint_write;
        let image = v.into_device().into_inner();
        let mut recovered = open(FaultBackend::new(image, FaultPlan::default()), pages);
        exact(&mut recovered, committed);
        if !committed {
            apply(&mut recovered).unwrap();
            exact(&mut recovered, true);
        }
    }
    writes + flushes
}

// ---------------------------------------------------------------------------
// Small direct/tree/reservation fixture: every case fits the cut budget.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    DirectPartialOverwrite,
    DirectFullRewrite,
    DirectSparseExtend,
    TreeOverwrite,
    TreeToDirectRewrite,
    ReservationInit,
    ReservationSplit,
    SharedReservationFallback,
    DirectBoundedWrite,
    DirectReserveBeyondEof,
    TreeReserveHole,
    ReservedReserveBeyondEof,
    DirectSparseGrowth,
    TreeSparseGrowth,
    TreeBoundedShrink,
    TreeShrinkToDirect,
    DirectBoundedShrinkSplit,
    DirectBoundedShrinkAligned,
    ReservedBoundedShrink,
}
const CASES: [Case; 19] = [
    Case::DirectPartialOverwrite,
    Case::DirectFullRewrite,
    Case::DirectSparseExtend,
    Case::TreeOverwrite,
    Case::TreeToDirectRewrite,
    Case::ReservationInit,
    Case::ReservationSplit,
    Case::SharedReservationFallback,
    Case::DirectBoundedWrite,
    Case::DirectReserveBeyondEof,
    Case::TreeReserveHole,
    Case::ReservedReserveBeyondEof,
    Case::DirectSparseGrowth,
    Case::TreeSparseGrowth,
    Case::TreeBoundedShrink,
    Case::TreeShrinkToDirect,
    Case::DirectBoundedShrinkSplit,
    Case::DirectBoundedShrinkAligned,
    Case::ReservedBoundedShrink,
];
impl Case {
    fn reserves(self) -> bool {
        matches!(
            self,
            Case::DirectReserveBeyondEof | Case::TreeReserveHole | Case::ReservedReserveBeyondEof
        )
    }
    /// (data blocks written, of which initialized from private reservations).
    fn data_blocks(self) -> (u64, u64) {
        match self {
            Case::DirectPartialOverwrite => (2, 0),
            Case::DirectFullRewrite | Case::TreeToDirectRewrite => (3, 0),
            Case::ReservationInit => (2, 2),
            Case::ReservationSplit => (1, 1),
            Case::DirectSparseExtend
            | Case::TreeOverwrite
            | Case::SharedReservationFallback
            | Case::DirectBoundedWrite
            | Case::TreeBoundedShrink
            | Case::TreeShrinkToDirect
            | Case::DirectBoundedShrinkSplit => (1, 0),
            _ => (0, 0),
        }
    }
}

struct Fixture {
    direct: u64,
    sparse: u64,
    reserved: u64,
    shared: u64,
    peer: u64,
    snapshot: u64,
    generation: u64,
    records: BTreeMap<u64, ObjectRecord>,
    extents: BTreeMap<u64, Vec<Extent>>,
}
impl Fixture {
    fn ids(&self) -> [u64; 5] {
        [
            self.direct,
            self.sparse,
            self.reserved,
            self.shared,
            self.peer,
        ]
    }
    fn old(&self, id: u64) -> Expect {
        if id == self.direct {
            Expect {
                bytes: fill(2 * BS + 100, &[(0, 2 * BS + 100, 0x11)]),
                ranges: vec![(0, 3 * B, false)],
                tree: false,
            }
        } else if id == self.sparse {
            Expect {
                bytes: fill(3 * BS, &[(0, BS, 0x22), (2 * BS, 3 * BS, 0x23)]),
                ranges: vec![(0, B, false), (2 * B, B, false)],
                tree: true,
            }
        } else if id == self.reserved {
            Expect {
                bytes: vec![0; 4 * BS],
                ranges: vec![(0, 2 * B, true), (3 * B, B, true)],
                tree: true,
            }
        } else {
            assert!(id == self.shared || id == self.peer);
            Expect {
                bytes: vec![0; 2 * BS],
                ranges: vec![(0, 2 * B, true)],
                tree: true,
            }
        }
    }
}

fn fixture(pages: usize) -> (MemoryBackend, Fixture) {
    let mut v = open(formatted(512), pages);
    let direct = v
        .create_file_in_root("direct", &[0x11; 2 * BS + 100], time(1))
        .unwrap();
    let sparse = v
        .create_file_in_root("sparse", &[0x22; BS], time(1))
        .unwrap();
    v.write_file_at(sparse, 2 * B, &[0x23; BS], time(2))
        .unwrap();
    let reserved = v.create_file_in_root("reserved", b"", time(1)).unwrap();
    v.preallocate_file(reserved, 0, 2 * B, time(2)).unwrap();
    v.preallocate_file(reserved, 3 * B, B, time(2)).unwrap();
    v.truncate_file(reserved, 4 * B, time(3)).unwrap();
    let shared = v.create_file_in_root("shared", b"", time(1)).unwrap();
    v.preallocate_file(shared, 0, 2 * B, time(2)).unwrap();
    v.truncate_file(shared, 2 * B, time(3)).unwrap();
    let peer = v.clone_file(shared, OBJECT_ROOT, "peer", time(4)).unwrap();
    let snapshot = v.snapshot_create(time(5)).unwrap();
    let mut f = Fixture {
        direct,
        sparse,
        reserved,
        shared,
        peer,
        snapshot,
        generation: v.generation(),
        records: BTreeMap::new(),
        extents: BTreeMap::new(),
    };
    for id in f.ids() {
        let record = check_file(&mut v, id, &f.old(id));
        f.records.insert(id, record);
    }
    for id in [sparse, reserved, shared, peer] {
        f.extents.insert(id, load_extents(&mut v, id));
    }
    clean(v.device_mut());
    (v.into_device(), f)
}

fn apply<D: BlockDevice>(v: &mut Volume<D>, f: &Fixture, case: Case) -> Result<(), CoreError> {
    let now = time(6);
    match case {
        Case::DirectPartialOverwrite => v.write_file_at(f.direct, 7, &[0x31; 5000], now),
        Case::DirectFullRewrite => v.write_file_at(f.direct, 0, &[0x32; 2 * BS + 100], now),
        Case::DirectSparseExtend => v.write_file_at(f.direct, 5 * B + 11, b"tail", now),
        Case::TreeOverwrite => v.write_file_at(f.sparse, 2 * B + 5, &[0x33; 10], now),
        Case::TreeToDirectRewrite => v.write_file_at(f.sparse, 0, &[0x34; 3 * BS], now),
        Case::ReservationInit => {
            v.write_file_at_bounded(f.reserved, 7, &[0x41; 5000], now, lim(2, 8))
        }
        Case::ReservationSplit => {
            v.write_file_at_bounded(f.reserved, B + 3, &[0x42; 10], now, lim(1, 8))
        }
        Case::SharedReservationFallback => {
            v.write_file_at_bounded(f.shared, 3, &[0x43; 10], now, lim(1, 8))
        }
        Case::DirectBoundedWrite => {
            v.write_file_at_bounded(f.direct, 2 * B + 50, &[0x44; 10], now, lim(1, 8))
        }
        Case::DirectReserveBeyondEof => {
            v.preallocate_file_bounded(f.direct, 4 * B, B, now, lim(1, 4))
        }
        Case::TreeReserveHole => v.preallocate_file_bounded(f.sparse, B, B, now, lim(1, 4)),
        Case::ReservedReserveBeyondEof => {
            v.preallocate_file_bounded(f.reserved, 5 * B, 2 * B, now, lim(2, 4))
        }
        Case::DirectSparseGrowth => v.truncate_file(f.direct, 6 * B, now),
        Case::TreeSparseGrowth => v.truncate_file(f.sparse, 8 * B + 1, now),
        Case::TreeBoundedShrink => v.truncate_file_bounded(f.sparse, 2 * B + 7, now, lim(1, 4)),
        Case::TreeShrinkToDirect => v.truncate_file(f.sparse, B - 5, now),
        Case::DirectBoundedShrinkSplit => v.truncate_file_bounded(f.direct, B + 1, now, lim(2, 4)),
        Case::DirectBoundedShrinkAligned => v.truncate_file_bounded(f.direct, B, now, lim(2, 4)),
        Case::ReservedBoundedShrink => v.truncate_file_bounded(f.reserved, B + 3, now, lim(1, 4)),
    }
}

/// Independent literal post-state of the one object each case changes.
fn expected(f: &Fixture, case: Case) -> (u64, Expect) {
    let direct = |bytes, ranges, tree| {
        (
            f.direct,
            Expect {
                bytes,
                ranges,
                tree,
            },
        )
    };
    let sparse = |bytes, ranges, tree| {
        (
            f.sparse,
            Expect {
                bytes,
                ranges,
                tree,
            },
        )
    };
    let reserved = |bytes, ranges| {
        (
            f.reserved,
            Expect {
                bytes,
                ranges,
                tree: true,
            },
        )
    };
    let d = (0, 2 * BS + 100, 0x11);
    let (s0, s2) = ((0, BS, 0x22), (2 * BS, 3 * BS, 0x23));
    match case {
        Case::DirectPartialOverwrite => direct(
            fill(2 * BS + 100, &[d, (7, 5007, 0x31)]),
            vec![(0, 2 * B, false), (2 * B, B, false)],
            true,
        ),
        Case::DirectFullRewrite => direct(
            fill(2 * BS + 100, &[(0, 2 * BS + 100, 0x32)]),
            vec![(0, 3 * B, false)],
            false,
        ),
        Case::DirectSparseExtend => {
            let mut bytes = fill(5 * BS + 15, &[d]);
            bytes[5 * BS + 11..].copy_from_slice(b"tail");
            direct(bytes, vec![(0, 3 * B, false), (5 * B, B, false)], true)
        }
        Case::TreeOverwrite => sparse(
            fill(3 * BS, &[s0, s2, (2 * BS + 5, 2 * BS + 15, 0x33)]),
            vec![(0, B, false), (2 * B, B, false)],
            true,
        ),
        Case::TreeToDirectRewrite => sparse(
            fill(3 * BS, &[(0, 3 * BS, 0x34)]),
            vec![(0, 3 * B, false)],
            false,
        ),
        Case::ReservationInit => reserved(
            fill(4 * BS, &[(7, 5007, 0x41)]),
            vec![(0, 2 * B, false), (3 * B, B, true)],
        ),
        Case::ReservationSplit => reserved(
            fill(4 * BS, &[(BS + 3, BS + 13, 0x42)]),
            vec![(0, B, true), (B, B, false), (3 * B, B, true)],
        ),
        Case::SharedReservationFallback => (
            f.shared,
            Expect {
                bytes: fill(2 * BS, &[(3, 13, 0x43)]),
                ranges: vec![(0, B, false), (B, B, true)],
                tree: true,
            },
        ),
        Case::DirectBoundedWrite => direct(
            fill(2 * BS + 100, &[d, (2 * BS + 50, 2 * BS + 60, 0x44)]),
            vec![(0, 2 * B, false), (2 * B, B, false)],
            true,
        ),
        Case::DirectReserveBeyondEof => direct(
            fill(2 * BS + 100, &[d]),
            vec![(0, 3 * B, false), (4 * B, B, true)],
            true,
        ),
        Case::TreeReserveHole => sparse(
            fill(3 * BS, &[s0, s2]),
            vec![(0, B, false), (B, B, true), (2 * B, B, false)],
            true,
        ),
        Case::ReservedReserveBeyondEof => reserved(
            vec![0; 4 * BS],
            vec![(0, 2 * B, true), (3 * B, B, true), (5 * B, 2 * B, true)],
        ),
        Case::DirectSparseGrowth => direct(fill(6 * BS, &[d]), vec![(0, 3 * B, false)], true),
        Case::TreeSparseGrowth => sparse(
            fill(8 * BS + 1, &[s0, s2]),
            vec![(0, B, false), (2 * B, B, false)],
            true,
        ),
        Case::TreeBoundedShrink => sparse(
            fill(2 * BS + 7, &[s0, (2 * BS, 2 * BS + 7, 0x23)]),
            vec![(0, B, false), (2 * B, B, false)],
            true,
        ),
        Case::TreeShrinkToDirect => sparse(
            fill(BS - 5, &[(0, BS - 5, 0x22)]),
            vec![(0, B, false)],
            false,
        ),
        Case::DirectBoundedShrinkSplit => direct(
            fill(BS + 1, &[(0, BS + 1, 0x11)]),
            vec![(0, B, false), (B, B, false)],
            true,
        ),
        Case::DirectBoundedShrinkAligned => {
            direct(fill(BS, &[(0, BS, 0x11)]), vec![(0, B, false)], false)
        }
        Case::ReservedBoundedShrink => reserved(vec![0; BS + 3], vec![(0, 2 * B, true)]),
    }
}

/// Physical ownership: private reservations are initialized at their reserved
/// addresses, a shared reservation falls back to a fresh block, and every
/// untouched tree-backed file keeps its exact mapping records.
fn check_physical<D: BlockDevice>(
    v: &mut Volume<D>,
    f: &Fixture,
    case: Case,
    committed: bool,
    target: u64,
) {
    let old = &f.extents[&f.reserved];
    match (case, committed) {
        (Case::ReservationInit, true) => assert_eq!(
            load_extents(v, f.reserved),
            vec![Extent { flags: 0, ..old[0] }, old[1]]
        ),
        (Case::ReservationSplit, true) => assert_eq!(
            load_extents(v, f.reserved),
            vec![
                Extent {
                    block_count: 1,
                    ..old[0]
                },
                Extent {
                    logical_start: 1,
                    physical_start: old[0].physical_start + 1,
                    block_count: 1,
                    flags: 0,
                },
                old[1],
            ]
        ),
        (Case::SharedReservationFallback, true) => {
            let old = f.extents[&f.shared][0];
            assert_ne!(old.flags & EXTENT_UNWRITTEN, 0);
            let now = load_extents(v, f.shared);
            assert_eq!(now.len(), 2);
            assert_eq!(
                (now[0].logical_start, now[0].block_count, now[0].flags),
                (0, 1, 0)
            );
            let reserved_run = old.physical_start..old.physical_start + 2;
            assert!(!reserved_run.contains(&now[0].physical_start));
            assert_eq!(
                now[1],
                Extent {
                    logical_start: 1,
                    physical_start: old.physical_start + 1,
                    block_count: 1,
                    ..old
                }
            );
        }
        _ => {}
    }
    for (&id, extents) in &f.extents {
        if !(committed && id == target) {
            assert_eq!(&load_extents(v, id), extents, "object {id} mapping");
        }
    }
}

fn verify<D: BlockDevice>(v: &mut Volume<D>, f: &Fixture, case: Case, committed: bool) {
    assert_eq!(v.generation(), f.generation + u64::from(committed));
    let names: BTreeMap<String, u64> = v.list_root().unwrap().into_iter().collect();
    let wanted: BTreeMap<String, u64> = ["direct", "sparse", "reserved", "shared", "peer"]
        .into_iter()
        .map(String::from)
        .zip(f.ids())
        .collect();
    assert_eq!(names, wanted);
    let (target, new) = expected(f, case);
    for id in f.ids() {
        let old = f.records[&id];
        if committed && id == target {
            let record = check_file(v, id, &new);
            check_target_metadata(&record, &old, !case.reserves(), f.generation + 1);
        } else {
            let record = check_file(v, id, &f.old(id));
            assert_eq!(record, old, "object {id} record");
        }
    }
    check_physical(v, f, case, committed, target);
    for id in f.ids() {
        let metadata = ObjectMetadata::from(f.records[&id]);
        check_captured(v, f.snapshot, id, &f.old(id), metadata);
    }
    clean(v.device_mut());
}

#[test]
fn data_mutations_preserve_exact_live_and_retained_state_in_all_profiles() {
    for pages in PROFILES {
        let (base, f) = fixture(pages);
        for case in CASES {
            let mut v = open(TraceBackend::new(base.clone()), pages);
            assert_eq!(v.data_update_policy(), DataUpdatePolicy::FullCow);
            v.device_mut().reset();
            apply(&mut v, &f, case).unwrap();
            let stats = v.last_commit_stats().unwrap();
            let io = v.device_mut().stats();
            assert_eq!(stats.bytes_written, io.bytes_written, "{case:?}");
            assert_eq!(stats.flushes, io.flushes, "{case:?}");
            let (data, initialized) = case.data_blocks();
            assert_eq!(
                (
                    stats.data_blocks_written,
                    stats.data_blocks_initialized_from_reservation,
                    stats.data_blocks_overwritten_in_place
                ),
                (data, initialized, 0),
                "pages={pages} {case:?}"
            );
            let tree = stats.tree_mutations;
            assert!(tree.max_resident_staged_nodes <= pages as u64);
            assert!(tree.max_staged_nodes_before_eviction <= (pages as u64).saturating_add(1));
            verify(&mut v, &f, case, true);
            let mut remounted = open(v.into_device().into_inner(), pages);
            verify(&mut remounted, &f, case, true);
        }
    }
}

fn data_mutation_cut_matrix(pages: usize, cases: &[Case]) {
    let started = Instant::now();
    let (base, f) = fixture(pages);
    let mut images = 0;
    for &case in cases {
        let (_, log) = record(&base, pages, |v| apply(v, &f, case).unwrap());
        assert!(longest_tail(&log) <= CUT_BUDGET, "{case:?}");
        let mut outcomes = [0usize; 2];
        for cut in 0..=log.len() {
            for_each_crash_state_with_budget(&base, &log, cut, CUT_BUDGET, |state| {
                let mut recovered = open(state.image, pages);
                let committed = recovered.generation() == f.generation + 1;
                verify(&mut recovered, &f, case, committed);
                outcomes[usize::from(committed)] += 1;
            });
        }
        assert!(outcomes.iter().all(|&n| n > 0), "{case:?} {outcomes:?}");
        images += outcomes[0] + outcomes[1];
        eprintln!(
            "pages={pages} case={case:?} ops={} longest_tail={} cuts old/new={outcomes:?}",
            log.len(),
            longest_tail(&log)
        );
    }
    eprintln!(
        "pages={pages} data cut images={images} elapsed={:?}",
        started.elapsed()
    );
}

/// Full-COW, sparse and bounded writes, including reservation initialization.
const WRITE_CASES: &[Case] = CASES.split_at(9).0;
/// Preallocation, sparse growth and (bounded) shrink.
const RESIZE_CASES: &[Case] = CASES.split_at(9).1;

#[test]
fn data_write_cuts_two_pages() {
    data_mutation_cut_matrix(2, WRITE_CASES);
}

#[test]
fn data_write_cuts_four_pages() {
    data_mutation_cut_matrix(4, WRITE_CASES);
}

#[test]
fn data_write_cuts_eight_pages() {
    data_mutation_cut_matrix(8, WRITE_CASES);
}

#[test]
fn data_write_cuts_unlimited() {
    data_mutation_cut_matrix(usize::MAX, WRITE_CASES);
}

#[test]
fn data_reserve_and_resize_cuts_two_pages() {
    data_mutation_cut_matrix(2, RESIZE_CASES);
}

#[test]
fn data_reserve_and_resize_cuts_four_pages() {
    data_mutation_cut_matrix(4, RESIZE_CASES);
}

#[test]
fn data_reserve_and_resize_cuts_eight_pages() {
    data_mutation_cut_matrix(8, RESIZE_CASES);
}

#[test]
fn data_reserve_and_resize_cuts_unlimited() {
    data_mutation_cut_matrix(usize::MAX, RESIZE_CASES);
}

#[test]
fn data_mutation_write_and_barrier_failures_preserve_ownership() {
    for pages in PROFILES {
        let (base, f) = fixture(pages);
        let mut injected = 0;
        for case in CASES {
            let (_, log) = record(&base, pages, |v| apply(v, &f, case).unwrap());
            injected += fault_matrix(
                &base,
                pages,
                &log,
                &|v| apply(v, &f, case),
                &|v| verify(v, &f, case, false),
                &|v| verify(v, &f, case, true),
                &|v, committed| verify(v, &f, case, committed),
            );
        }
        eprintln!("pages={pages} data injected write/flush failures={injected}");
    }
}

fn refusals<D: BlockDevice>(
    v: &mut Volume<D>,
    f: &Fixture,
    case: Case,
) -> Vec<(&'static str, bool)> {
    let now = time(6);
    let bad = Timespec {
        seconds: 6,
        nanoseconds: 1_000_000_000,
    };
    let limit = |r: Result<(), CoreError>| matches!(r, Err(CoreError::PrototypeLimit(_)));
    let invalid_time = |r: Result<(), CoreError>| matches!(r, Err(CoreError::InvalidMetadata(_)));
    let (direct, reserved, sparse) = (f.direct, f.reserved, f.sparse);
    match case {
        Case::DirectPartialOverwrite => vec![
            (
                "directory",
                matches!(
                    v.write_file_at(OBJECT_ROOT, 0, b"x", now),
                    Err(CoreError::IsDirectory)
                ),
            ),
            (
                "missing",
                matches!(
                    v.write_file_at(1 << 40, 0, b"x", now),
                    Err(CoreError::NotFound)
                ),
            ),
            (
                "offset overflow",
                limit(v.write_file_at(direct, u64::MAX - 3, b"overflow", now)),
            ),
            (
                "time",
                invalid_time(v.write_file_at(direct, 7, &[0x31; 5000], bad)),
            ),
            (
                "capacity",
                matches!(
                    v.write_file_at(direct, 0, &vec![0x31; 600 * BS], now),
                    Err(CoreError::NoSpace)
                ),
            ),
            ("empty write", v.write_file_at(direct, 7, b"", now).is_ok()),
        ],
        Case::ReservationInit => {
            let data = [0x41; 5000];
            vec![
                (
                    "blocks",
                    limit(v.write_file_at_bounded(reserved, 7, &data, now, lim(1, 8))),
                ),
                (
                    "zero blocks",
                    limit(v.write_file_at_bounded(reserved, 7, &data, now, lim(0, 8))),
                ),
                (
                    "zero records",
                    limit(v.write_file_at_bounded(reserved, 7, &data, now, lim(2, 0))),
                ),
                (
                    "unbounded records",
                    limit(v.write_file_at_bounded(reserved, 7, &data, now, lim(2, usize::MAX))),
                ),
            ]
        }
        Case::ReservationSplit => vec![(
            "result records",
            limit(v.write_file_at_bounded(reserved, B + 3, &[0x42; 10], now, lim(1, 1))),
        )],
        Case::DirectReserveBeyondEof => vec![
            (
                "blocks",
                limit(v.preallocate_file_bounded(direct, 4 * B, 2 * B, now, lim(1, 4))),
            ),
            (
                "invalid limits",
                limit(v.preallocate_file_bounded(direct, 4 * B, B, now, lim(1, usize::MAX))),
            ),
            (
                "range overflow",
                limit(v.preallocate_file_bounded(direct, u64::MAX - 3, 8, now, lim(1, 4))),
            ),
            (
                "time",
                invalid_time(v.preallocate_file_bounded(direct, 4 * B, B, bad, lim(1, 4))),
            ),
            (
                "capacity",
                matches!(
                    v.preallocate_file(direct, 8 * B, 600 * B, now),
                    Err(CoreError::NoSpace)
                ),
            ),
            (
                "directory",
                matches!(
                    v.preallocate_file(OBJECT_ROOT, 0, B, now),
                    Err(CoreError::IsDirectory)
                ),
            ),
            (
                "empty reservation",
                v.preallocate_file_bounded(reserved, 0, 0, now, lim(1, 1))
                    .is_ok(),
            ),
            (
                "covered reservation",
                v.preallocate_file_bounded(reserved, 0, 2 * B, now, lim(2, 4))
                    .is_ok(),
            ),
        ],
        Case::TreeReserveHole => vec![(
            "result records",
            limit(v.preallocate_file_bounded(sparse, B, B, now, lim(1, 2))),
        )],
        Case::DirectBoundedShrinkSplit => vec![
            (
                "retirement",
                limit(v.truncate_file_bounded(direct, B + 1, now, lim(1, 4))),
            ),
            (
                "zero records",
                limit(v.truncate_file_bounded(direct, B + 1, now, lim(2, 0))),
            ),
            (
                "directory",
                matches!(
                    v.truncate_file(OBJECT_ROOT, 0, now),
                    Err(CoreError::IsDirectory)
                ),
            ),
            ("time", invalid_time(v.truncate_file(direct, B + 1, bad))),
            ("same size", v.truncate_file(sparse, 3 * B, now).is_ok()),
        ],
        Case::ReservedBoundedShrink => vec![(
            "window records",
            limit(v.truncate_file_bounded(reserved, B + 3, now, lim(1, 1))),
        )],
        _ => Vec::new(),
    }
}

#[test]
fn data_mutation_refusals_issue_no_writes_and_retry_in_all_profiles() {
    for pages in PROFILES {
        let (base, f) = fixture(pages);
        let mut refused = 0;
        for case in CASES {
            let mut v = open(TraceBackend::new(base.clone()), pages);
            v.device_mut().reset();
            let results = refusals(&mut v, &f, case);
            if results.is_empty() {
                continue;
            }
            for (label, expected) in &results {
                assert!(expected, "pages={pages} {case:?} {label}");
            }
            assert_eq!(v.device_mut().stats().writes, 0, "{case:?}");
            assert_eq!(v.device_mut().stats().flushes, 0, "{case:?}");
            verify(&mut v, &f, case, false);
            apply(&mut v, &f, case).unwrap();
            verify(&mut v, &f, case, true);
            refused += results.len();
        }
        let mut read_only = open_mode(TraceBackend::new(base.clone()), pages, MountMode::ReadOnly);
        read_only.device_mut().reset();
        for case in CASES {
            assert!(matches!(
                apply(&mut read_only, &f, case),
                Err(CoreError::ReadOnly)
            ));
        }
        assert_eq!(read_only.device_mut().stats().writes, 0);
        assert_eq!(read_only.device_mut().stats().flushes, 0);
        verify(&mut read_only, &f, Case::DirectFullRewrite, false);
        refused += CASES.len();
        eprintln!("pages={pages} no-write refusals and no-ops={refused}");
    }
}

// ---------------------------------------------------------------------------
// Wide fixture: one written sparse tree and one reservation tree, sized per
// profile so the constrained cache really evicts staged extent-map nodes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wide {
    CowWrite,
    BoundedWrite,
    ReservationWrite,
    BoundedReservationWrite,
    ReserveHoles,
    Shrink,
    BoundedShrink,
    Growth,
}
const WIDE: [Wide; 8] = [
    Wide::CowWrite,
    Wide::BoundedWrite,
    Wide::ReservationWrite,
    Wide::BoundedReservationWrite,
    Wide::ReserveHoles,
    Wide::Shrink,
    Wide::BoundedShrink,
    Wide::Growth,
];
struct WideFixture {
    n: u64,
    wide: u64,
    resv: u64,
    snapshot: u64,
    generation: u64,
    records: [ObjectRecord; 2],
}
fn value(i: u64) -> u8 {
    (i % 200 + 16) as u8
}
impl WideFixture {
    fn old(&self, id: u64) -> Expect {
        let n = self.n as usize;
        if id == self.wide {
            let mut bytes = vec![0; (2 * n - 1) * BS];
            for i in 0..n {
                bytes[2 * i * BS..(2 * i + 1) * BS].fill(value(i as u64));
            }
            Expect {
                bytes,
                ranges: (0..self.n).map(|i| (2 * i * B, B, false)).collect(),
                tree: true,
            }
        } else {
            assert_eq!(id, self.resv);
            Expect {
                bytes: vec![0; 3 * n * BS],
                ranges: (0..self.n).map(|i| (3 * i * B, B, true)).collect(),
                tree: true,
            }
        }
    }
}
fn wide_fixture(pages: usize, n: u64) -> (MemoryBackend, WideFixture) {
    let mut v = open(formatted((16 * n).next_power_of_two().max(1024)), pages);
    let wide = v.create_file_in_root("wide", b"", time(1)).unwrap();
    for i in 0..n {
        v.write_file_at_bounded(wide, 2 * i * B, &[value(i); BS], time(1), lim(1, 8))
            .unwrap();
    }
    let resv = v.create_file_in_root("resv", b"", time(2)).unwrap();
    for i in 0..n {
        v.preallocate_file_bounded(resv, 3 * i * B, B, time(2), lim(1, 8))
            .unwrap();
    }
    v.truncate_file(resv, 3 * n * B, time(3)).unwrap();
    let snapshot = v.snapshot_create(time(4)).unwrap();
    let mut f = WideFixture {
        n,
        wide,
        resv,
        snapshot,
        generation: v.generation(),
        records: [
            v.stat(wide).unwrap().unwrap(),
            v.stat(resv).unwrap().unwrap(),
        ],
    };
    f.records = [
        check_file(&mut v, wide, &f.old(wide)),
        check_file(&mut v, resv, &f.old(resv)),
    ];
    clean(v.device_mut());
    (v.into_device(), f)
}
fn wide_apply<D: BlockDevice>(
    v: &mut Volume<D>,
    f: &WideFixture,
    op: Wide,
) -> Result<(), CoreError> {
    let (n, m, now) = (f.n, f.n / 2, time(6));
    match op {
        Wide::CowWrite => v.write_file_at(f.wide, 2 * m * B + 7, &[0xee; 100], now),
        Wide::BoundedWrite => {
            v.write_file_at_bounded(f.wide, 2 * m * B + 7, &[0xeb; 100], now, lim(1, 8))
        }
        Wide::ReservationWrite => v.write_file_at(f.resv, 3 * m * B + 7, &[0x5a; 100], now),
        Wide::BoundedReservationWrite => {
            v.write_file_at_bounded(f.resv, 3 * m * B + 7, &[0x5b; 100], now, lim(1, 8))
        }
        Wide::ReserveHoles => {
            v.preallocate_file_bounded(f.wide, 0, 2 * n * B, now, lim(2 * n, 2 * n as usize + 2))
        }
        Wide::Shrink => v.truncate_file(f.wide, n * B + 9, now),
        Wide::BoundedShrink => {
            v.truncate_file_bounded(f.wide, n * B + 9, now, lim(n, n as usize + 2))
        }
        Wide::Growth => v.truncate_file(f.wide, 4 * n * B, now),
    }
}
fn wide_expected(f: &WideFixture, op: Wide) -> (u64, Expect) {
    let (n, m) = (f.n as usize, (f.n / 2) as usize);
    let target = if matches!(op, Wide::ReservationWrite | Wide::BoundedReservationWrite) {
        f.resv
    } else {
        f.wide
    };
    let mut e = f.old(target);
    match op {
        Wide::CowWrite => e.bytes[2 * m * BS + 7..2 * m * BS + 107].fill(0xee),
        Wide::BoundedWrite => e.bytes[2 * m * BS + 7..2 * m * BS + 107].fill(0xeb),
        Wide::ReservationWrite | Wide::BoundedReservationWrite => {
            let byte = if op == Wide::ReservationWrite {
                0x5a
            } else {
                0x5b
            };
            e.bytes[3 * m * BS + 7..3 * m * BS + 107].fill(byte);
            e.ranges[m].2 = false;
        }
        Wide::ReserveHoles => {
            e.ranges = (0..2 * f.n).map(|i| (i * B, B, i % 2 == 1)).collect();
        }
        Wide::Shrink | Wide::BoundedShrink => {
            // Block n is written (n is even); its first nine bytes survive.
            e.bytes.truncate(n * BS + 9);
            e.ranges.truncate(n / 2 + 1);
        }
        Wide::Growth => e.bytes.resize(4 * n * BS, 0),
    }
    (target, e)
}
fn wide_verify<D: BlockDevice>(v: &mut Volume<D>, f: &WideFixture, op: Wide, committed: bool) {
    assert_eq!(v.generation(), f.generation + u64::from(committed));
    let (target, new) = wide_expected(f, op);
    for (index, id) in [f.wide, f.resv].into_iter().enumerate() {
        let old = f.records[index];
        if committed && id == target {
            let record = check_file(v, id, &new);
            check_target_metadata(&record, &old, op != Wide::ReserveHoles, f.generation + 1);
        } else {
            let record = check_file(v, id, &f.old(id));
            assert_eq!(record, old);
        }
        check_captured(v, f.snapshot, id, &f.old(id), ObjectMetadata::from(old));
    }
    clean(v.device_mut());
}
/// Cheap in-memory identity after a failed attempt; the full exact oracle
/// runs after the in-place retry and after every reconciliation remount.
fn wide_unchanged<D: BlockDevice>(v: &mut Volume<D>, f: &WideFixture) {
    assert_eq!(v.generation(), f.generation);
    assert_eq!(v.stat(f.wide).unwrap(), Some(f.records[0]));
    assert_eq!(v.stat(f.resv).unwrap(), Some(f.records[1]));
}
/// Cheap live identity after an in-place retry; the remount oracle is exact.
fn wide_retried<D: BlockDevice>(v: &mut Volume<D>, f: &WideFixture, op: Wide) {
    assert_eq!(v.generation(), f.generation + 1);
    let (target, new) = wide_expected(f, op);
    let record = v.stat(target).unwrap().unwrap();
    assert_eq!(record.size_bytes, new.bytes.len() as u64);
    assert_eq!(
        record.allocated_bytes,
        new.ranges.iter().map(|r| r.1).sum::<u64>()
    );
    assert_eq!(
        record.content_generation == f.generation + 1,
        op != Wide::ReserveHoles
    );
}

struct EvictionPlan {
    pages: usize,
    records: u64,
    ops: &'static [Wide],
    spilling: &'static [Wide],
    cuts: &'static [Wide],
    faults: &'static [Wide],
}

fn eviction_profile(plan: EvictionPlan) {
    let started = Instant::now();
    let pages = plan.pages;
    let (base, f) = wide_fixture(pages, plan.records);
    eprintln!(
        "pages={pages} records={} setup={:?}",
        plan.records,
        started.elapsed()
    );
    for &op in plan.ops {
        let op_started = Instant::now();
        let mut v = open(RecordingBackend::new(base.clone()), pages);
        wide_apply(&mut v, &f, op).unwrap();
        let stats = v.last_commit_stats().unwrap();
        let tree = stats.tree_mutations;
        let summary = format!(
            "pages={pages} records={} op={op:?} spill={} reload={} resident={} before_eviction={} final={}",
            plan.records,
            tree.staged_spill_writes,
            tree.staged_spill_reloads,
            tree.max_resident_staged_nodes,
            tree.max_staged_nodes_before_eviction,
            tree.final_nodes_written
        );
        assert!(tree.max_resident_staged_nodes <= pages as u64, "{summary}");
        assert!(
            tree.max_staged_nodes_before_eviction <= (pages as u64).saturating_add(1),
            "{summary}"
        );
        if pages == usize::MAX {
            // The unlimited profile never evicts, although the same work
            // stages more nodes than the eight-page profile retains.
            assert_eq!(
                (tree.staged_spill_writes, tree.staged_spill_reloads),
                (0, 0)
            );
            if plan.spilling.contains(&op) {
                assert!(tree.max_resident_staged_nodes > 8, "{summary}");
            }
        } else if plan.spilling.contains(&op) {
            assert!(tree.staged_spill_writes > 0, "{summary}");
        }
        let initialized = u64::from(matches!(
            op,
            Wide::ReservationWrite | Wide::BoundedReservationWrite
        ));
        assert_eq!(stats.data_blocks_initialized_from_reservation, initialized);
        assert_eq!(stats.data_blocks_overwritten_in_place, 0);
        wide_verify(&mut v, &f, op, true);
        let (image, log) = v.into_device().into_parts();
        let mut remounted = open(image, pages);
        wide_verify(&mut remounted, &f, op, true);
        let (writes, flushes) = counts(&log);
        let mut detail = format!(
            "writes={writes} flushes={flushes} longest_tail={}",
            longest_tail(&log)
        );
        if plan.cuts.contains(&op) {
            assert!(longest_tail(&log) <= CUT_BUDGET, "{summary} {detail}");
            let mut outcomes = [0usize; 2];
            for cut in 0..=log.len() {
                for_each_crash_state_with_budget(&base, &log, cut, CUT_BUDGET, |state| {
                    let mut recovered = open(state.image, pages);
                    let committed = recovered.generation() == f.generation + 1;
                    wide_verify(&mut recovered, &f, op, committed);
                    outcomes[usize::from(committed)] += 1;
                });
            }
            assert!(outcomes.iter().all(|&n| n > 0), "{summary} {outcomes:?}");
            detail += &format!(" cuts old/new={outcomes:?}");
        }
        if plan.faults.contains(&op) {
            let injected = fault_matrix(
                &base,
                pages,
                &log,
                &|v| wide_apply(v, &f, op),
                &|v| wide_unchanged(v, &f),
                &|v| wide_retried(v, &f, op),
                &|v, committed| wide_verify(v, &f, op, committed),
            );
            detail += &format!(" injected={injected}");
        }
        eprintln!("{summary} {detail} elapsed={:?}", op_started.elapsed());
    }
    eprintln!("pages={pages} eviction elapsed={:?}", started.elapsed());
}

/// Full rewrites and multi-leaf windows that evict at eight pages with 500
/// records per file. Bounded one-block writes and growth stage at most six
/// extent/object-map nodes here and do not evict at eight pages.
const EIGHT_PAGE_SPILLS: [Wide; 5] = [
    Wide::CowWrite,
    Wide::ReservationWrite,
    Wide::ReserveHoles,
    Wide::Shrink,
    Wide::BoundedShrink,
];

/// Operations that evict at four pages with 120 records per file. The
/// reservation-initializing writes stage exactly four nodes with 120 and 160
/// records and do not evict at four pages.
const FOUR_PAGE_SPILLS: [Wide; 6] = [
    Wide::CowWrite,
    Wide::BoundedWrite,
    Wide::ReserveHoles,
    Wide::Shrink,
    Wide::BoundedShrink,
    Wide::Growth,
];

#[test]
fn data_eviction_two_pages() {
    eviction_profile(EvictionPlan {
        pages: 2,
        records: 40,
        ops: &WIDE,
        spilling: &WIDE,
        cuts: &[],
        faults: &WIDE,
    });
}

#[test]
fn data_eviction_cuts_two_pages_writes() {
    eviction_profile(EvictionPlan {
        pages: 2,
        records: 40,
        ops: WIDE.split_at(4).0,
        spilling: &WIDE,
        cuts: &WIDE,
        faults: &[],
    });
}

#[test]
fn data_eviction_cuts_two_pages_reserve_and_resize() {
    eviction_profile(EvictionPlan {
        pages: 2,
        records: 40,
        ops: WIDE.split_at(4).1,
        spilling: &WIDE,
        cuts: &WIDE,
        faults: &[],
    });
}

#[test]
fn data_eviction_four_pages() {
    eviction_profile(EvictionPlan {
        pages: 4,
        records: 120,
        ops: &WIDE,
        spilling: &FOUR_PAGE_SPILLS,
        cuts: &[],
        faults: &WIDE,
    });
}

#[test]
fn data_eviction_cuts_four_pages_bounded_write() {
    eviction_profile(EvictionPlan {
        pages: 4,
        records: 120,
        ops: &[Wide::BoundedWrite],
        spilling: &FOUR_PAGE_SPILLS,
        cuts: &[Wide::BoundedWrite],
        faults: &[],
    });
}

#[test]
fn data_eviction_cuts_four_pages_bounded_shrink() {
    eviction_profile(EvictionPlan {
        pages: 4,
        records: 120,
        ops: &[Wide::BoundedShrink],
        spilling: &FOUR_PAGE_SPILLS,
        cuts: &[Wide::BoundedShrink],
        faults: &[],
    });
}

#[test]
fn data_eviction_eight_pages_writes_and_reservations() {
    eviction_profile(EvictionPlan {
        pages: 8,
        records: 500,
        ops: &WIDE[..5],
        spilling: &EIGHT_PAGE_SPILLS,
        cuts: &[],
        faults: &[Wide::CowWrite, Wide::ReservationWrite, Wide::ReserveHoles],
    });
}

#[test]
fn data_eviction_eight_pages_size_changes() {
    eviction_profile(EvictionPlan {
        pages: 8,
        records: 500,
        ops: &WIDE[5..],
        spilling: &EIGHT_PAGE_SPILLS,
        cuts: &[],
        faults: &[Wide::Shrink, Wide::BoundedShrink],
    });
}

#[test]
fn data_eviction_unlimited_has_no_spills() {
    eviction_profile(EvictionPlan {
        pages: usize::MAX,
        records: 500,
        ops: &WIDE,
        spilling: &EIGHT_PAGE_SPILLS,
        cuts: &[],
        faults: &[],
    });
}
