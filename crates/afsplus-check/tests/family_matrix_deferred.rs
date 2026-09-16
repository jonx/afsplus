//! Deferred intent-log windows through the family-matrix driver: namespace,
//! existing-file write and truncate groups qualified with the replay runner,
//! and the refusal and read-failure paths of every window entry point.
//! See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend, TraceBackend};
use afsplus_core::volume::BatchOp;
use afsplus_core::{CoreError, MountMode, Volume};
use afsplus_format::{Timespec, OBJECT_ROOT};

use common::family_matrix::{self as matrix, ts, Format, ReplayFamily, Variant, BS};

/// Log slots of every deferred fixture.
const SLOTS: u16 = 8;
/// Long root names of the eviction fixtures.
const POPULATION: usize = 300;
const NAME_LENGTH: usize = 240;
/// Seeded full-write subsets drawn per flush segment of a recovery campaign.
const SAMPLE: usize = 128;

fn image(variant: Variant) -> Format {
    Format {
        log_slots: SLOTS,
        ..Format::new(
            if variant == Variant::Eviction {
                16 * 1024
            } else {
                1024
            },
            if variant == Variant::Eviction {
                16
            } else {
                256
            },
        )
    }
}

/// Root entries beyond the family's own names.
fn background(variant: Variant) -> usize {
    if variant == Variant::Eviction {
        POPULATION
    } else {
        0
    }
}

fn setup_background(volume: &mut Volume<MemoryBackend>, variant: Variant) {
    if variant == Variant::Eviction {
        matrix::populate(volume, "tree", POPULATION, NAME_LENGTH);
    }
}

/// Exact root listing: the anchor, the background names and `names`.
fn assert_root<D: BlockDevice>(
    volume: &mut Volume<D>,
    variant: Variant,
    anchor: &str,
    names: &[&str],
    context: &str,
) {
    let entries = volume.list_root().unwrap();
    assert_eq!(
        entries.len(),
        background(variant) + 1 + names.len(),
        "{context}: root entry count"
    );
    let present: std::collections::BTreeSet<String> =
        entries.into_iter().map(|(name, _)| name).collect();
    assert!(present.contains(anchor), "{context}: anchor entry");
    for name in names {
        assert!(present.contains(*name), "{context}: entry {name}");
    }
    for name in ["alpha", "beta", "gamma", "note", "renamed"] {
        if !names.contains(&name) {
            assert!(
                !present.contains(name),
                "{context}: unexpected entry {name}"
            );
        }
    }
}

fn assert_bytes<D: BlockDevice>(volume: &mut Volume<D>, id: u64, expected: &[u8], context: &str) {
    assert_eq!(volume.read_file(id).unwrap(), expected, "{context}: bytes");
}

// ---------------------------------------------------------------- namespace

/// Three durable namespace groups: a create, a rename, and a create paired
/// with a delete inside one group.
struct DeferredNamespace;

struct NamespaceState {
    anchor: u64,
}

const ANCHOR: &[u8] = b"anchor-bytes-kept-through-every-group";
const ALPHA: &[u8] = b"alpha-bytes";
const GAMMA: &[u8] = b"gamma-bytes";

impl ReplayFamily for DeferredNamespace {
    type State = NamespaceState;

    fn name(&self) -> &'static str {
        "deferred namespace"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> NamespaceState {
        setup_background(volume, variant);
        let anchor = volume.create_file_in_root("anchor", ANCHOR, ts(1)).unwrap();
        NamespaceState { anchor }
    }

    fn captured(&self, state: &NamespaceState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.anchor, ANCHOR.to_vec())]
    }

    fn groups(&self) -> usize {
        3
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &NamespaceState,
        index: usize,
    ) -> Result<(), CoreError> {
        match index {
            0 => {
                volume.window_op(
                    &BatchOp::CreateFile {
                        parent_id: OBJECT_ROOT,
                        name: "alpha",
                        content: ALPHA,
                    },
                    ts(10),
                )?;
            }
            1 => {
                volume.window_op(
                    &BatchOp::Rename {
                        source_parent_id: OBJECT_ROOT,
                        source_name: "alpha",
                        target_parent_id: OBJECT_ROOT,
                        target_name: "beta",
                        replace: false,
                    },
                    ts(11),
                )?;
            }
            _ => {
                volume.window_op(
                    &BatchOp::CreateFile {
                        parent_id: OBJECT_ROOT,
                        name: "gamma",
                        content: GAMMA,
                    },
                    ts(12),
                )?;
                volume.window_op(
                    &BatchOp::DeleteFile {
                        parent_id: OBJECT_ROOT,
                        name: "beta",
                    },
                    ts(13),
                )?;
            }
        }
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &NamespaceState,
        variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        assert_bytes(volume, state.anchor, ANCHOR, context);
        let names: &[&str] = match acknowledged {
            0 => &[],
            1 => &["alpha"],
            2 => &["beta"],
            _ => &["gamma"],
        };
        assert_root(volume, variant, "anchor", names, context);
        for name in names {
            let id = volume.lookup_root(name).unwrap().unwrap();
            let expected = if *name == "gamma" { GAMMA } else { ALPHA };
            assert_bytes(volume, id, expected, context);
        }
    }

    /// Root directory, object map and allocation root of the replay commit.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

// -------------------------------------------------------------------- write

/// Three durable existing-file writes at distinct offsets.
struct DeferredWrite;

struct WriteState {
    file: u64,
}

const BASE_BYTE: u8 = 0x18;
const PATCHES: [(u64, u8, usize); 3] = [(73, 0xc7, 211), (4096 + 10, 0x5a, 300), (2, 0x3e, 4100)];

fn patched(count: usize) -> Vec<u8> {
    let mut bytes = vec![BASE_BYTE; 3 * BS];
    for (offset, value, length) in PATCHES.iter().take(count) {
        let start = *offset as usize;
        bytes[start..start + length].fill(*value);
    }
    bytes
}

impl ReplayFamily for DeferredWrite {
    type State = WriteState;

    fn name(&self) -> &'static str {
        "deferred write"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> WriteState {
        setup_background(volume, variant);
        let file = volume
            .create_file_in_root("anchor", &vec![BASE_BYTE; 3 * BS], ts(1))
            .unwrap();
        WriteState { file }
    }

    fn captured(&self, state: &WriteState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, vec![BASE_BYTE; 3 * BS])]
    }

    fn groups(&self) -> usize {
        3
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &WriteState,
        index: usize,
    ) -> Result<(), CoreError> {
        let (offset, value, length) = PATCHES[index];
        volume.window_write_file_at(
            state.file,
            offset,
            &vec![value; length],
            ts(20 + index as i64),
        )?;
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &WriteState,
        variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        assert_bytes(volume, state.file, &patched(acknowledged), context);
        assert_root(volume, variant, "anchor", &[], context);
    }

    /// Extent map, object map and allocation root of the replay commit.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

// ----------------------------------------------------------------- truncate

/// Three durable truncates: a partial shrink, a sparse growth and an aligned
/// shrink.
struct DeferredTruncate;

const SIZES: [u64; 3] = [BS as u64 + 211, 3 * BS as u64 + 50, BS as u64];

fn resized(count: usize) -> Vec<u8> {
    let mut bytes = vec![BASE_BYTE; 3 * BS];
    for size in SIZES.iter().take(count) {
        let size = *size as usize;
        if size <= bytes.len() {
            bytes.truncate(size);
        } else {
            bytes.resize(size, 0);
        }
    }
    bytes
}

impl ReplayFamily for DeferredTruncate {
    type State = WriteState;

    fn name(&self) -> &'static str {
        "deferred truncate"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> WriteState {
        setup_background(volume, variant);
        let file = volume
            .create_file_in_root("anchor", &vec![BASE_BYTE; 3 * BS], ts(1))
            .unwrap();
        WriteState { file }
    }

    fn captured(&self, state: &WriteState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, vec![BASE_BYTE; 3 * BS])]
    }

    fn groups(&self) -> usize {
        3
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &WriteState,
        index: usize,
    ) -> Result<(), CoreError> {
        volume.window_truncate_file(state.file, SIZES[index], ts(30 + index as i64))?;
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &WriteState,
        variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let expected = resized(acknowledged);
        assert_bytes(volume, state.file, &expected, context);
        assert_eq!(
            volume.stat(state.file).unwrap().unwrap().size_bytes,
            expected.len() as u64,
            "{context}: size"
        );
        assert_root(volume, variant, "anchor", &[], context);
    }

    /// Extent map, object map and allocation root of the replay commit.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

// -------------------------------------------------------------------- mixed

/// Windows that combine namespace, write and truncate work in one group.
struct DeferredMixed;

struct MixedState {
    file: u64,
}

const MIXED_PATCH: (u64, u8, usize) = (100, 0x9b, 500);
const MIXED_SECOND: (u64, u8, usize) = (BS as u64 + 7, 0x4d, 64);
const MIXED_SIZE: u64 = 2 * BS as u64 - 90;

fn mixed_bytes(acknowledged: usize) -> Vec<u8> {
    let mut bytes = vec![BASE_BYTE; 2 * BS];
    if acknowledged >= 1 {
        let (offset, value, length) = MIXED_PATCH;
        bytes[offset as usize..offset as usize + length].fill(value);
        bytes.truncate(MIXED_SIZE as usize);
    }
    if acknowledged >= 2 {
        let (offset, value, length) = MIXED_SECOND;
        bytes[offset as usize..offset as usize + length].fill(value);
    }
    bytes
}

impl ReplayFamily for DeferredMixed {
    type State = MixedState;

    fn name(&self) -> &'static str {
        "deferred mixed window"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> MixedState {
        setup_background(volume, variant);
        let file = volume
            .create_file_in_root("anchor", &vec![BASE_BYTE; 2 * BS], ts(1))
            .unwrap();
        MixedState { file }
    }

    fn captured(&self, state: &MixedState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, vec![BASE_BYTE; 2 * BS])]
    }

    fn groups(&self) -> usize {
        2
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MixedState,
        index: usize,
    ) -> Result<(), CoreError> {
        if index == 0 {
            volume.window_op(
                &BatchOp::CreateFile {
                    parent_id: OBJECT_ROOT,
                    name: "note",
                    content: GAMMA,
                },
                ts(40),
            )?;
            let (offset, value, length) = MIXED_PATCH;
            volume.window_write_file_at(state.file, offset, &vec![value; length], ts(41))?;
            volume.window_truncate_file(state.file, MIXED_SIZE, ts(42))?;
        } else {
            volume.window_op(
                &BatchOp::Rename {
                    source_parent_id: OBJECT_ROOT,
                    source_name: "note",
                    target_parent_id: OBJECT_ROOT,
                    target_name: "renamed",
                    replace: false,
                },
                ts(43),
            )?;
            let (offset, value, length) = MIXED_SECOND;
            volume.window_write_file_at(state.file, offset, &vec![value; length], ts(44))?;
        }
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MixedState,
        variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let expected = mixed_bytes(acknowledged);
        assert_bytes(volume, state.file, &expected, context);
        assert_eq!(
            volume.stat(state.file).unwrap().unwrap().size_bytes,
            expected.len() as u64,
            "{context}: size"
        );
        let names: &[&str] = match acknowledged {
            0 => &[],
            1 => &["note"],
            _ => &["renamed"],
        };
        assert_root(volume, variant, "anchor", names, context);
        for name in names {
            let id = volume.lookup_root(name).unwrap().unwrap();
            assert_bytes(volume, id, GAMMA, context);
        }
    }

    /// Extent map, object map, directory and allocation root.
    fn eviction_demand(&self) -> u64 {
        4
    }
}

// ------------------------------------------------- window entry-point paths

/// Fails the next read once, so a window entry point's preflight read fails.
struct FailOneRead {
    inner: TraceBackend<MemoryBackend>,
    armed: bool,
}

impl BlockDevice for FailOneRead {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if std::mem::take(&mut self.armed) {
            return Err(BlockError::Injected("window preflight read"));
        }
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

const BAD_TIME: Timespec = Timespec {
    seconds: 5,
    nanoseconds: 1_000_000_000,
};

/// Every refusal and read-failure path of `window_op`, `window_write_file_at`,
/// `window_truncate_file`, `window_fsync` and `window_commit` keeps the staged
/// window intact, issues no write and no flush, and leaves the acknowledged
/// and pending work recoverable.
fn window_entry_refusals(pages: usize) {
    let format = Format {
        log_slots: SLOTS,
        ..Format::new(1024, 256)
    };
    let base = {
        let mut volume = matrix::open(format.device(), pages);
        volume
            .create_file_in_root("anchor", &vec![BASE_BYTE; BS], ts(1))
            .unwrap();
        volume.create_directory_in_root("dir", ts(1)).unwrap();
        volume.into_device()
    };
    let mut volume = matrix::open(
        FailOneRead {
            inner: TraceBackend::new(base.clone()),
            armed: false,
        },
        pages,
    );
    let anchor = volume.lookup_root("anchor").unwrap().unwrap();
    let directory = volume.lookup_root("dir").unwrap().unwrap();
    let generation = volume.generation();

    // One acknowledged group, then one staged group that no refusal may lose.
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "durable",
                content: b"durable bytes",
            },
            ts(2),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "staged",
                content: b"staged bytes",
            },
            ts(3),
        )
        .unwrap();
    let pending = volume.window_unlogged_ops();
    assert_eq!(pending, 1);
    let before = volume.device_mut().inner.stats();

    type Call = Box<dyn FnOnce(&mut Volume<FailOneRead>) -> Result<(), CoreError>>;
    let calls: Vec<(&str, &str, bool, Call)> = vec![
        (
            "op invalid time",
            "InvalidMetadata(\"timestamp nanoseconds out of range\")",
            false,
            Box::new(|volume| {
                volume
                    .window_op(
                        &BatchOp::CreateFile {
                            parent_id: OBJECT_ROOT,
                            name: "late",
                            content: b"",
                        },
                        BAD_TIME,
                    )
                    .map(|_| ())
            }),
        ),
        (
            "op duplicate name",
            "AlreadyExists",
            false,
            Box::new(|volume| {
                volume
                    .window_op(
                        &BatchOp::CreateFile {
                            parent_id: OBJECT_ROOT,
                            name: "anchor",
                            content: b"",
                        },
                        ts(4),
                    )
                    .map(|_| ())
            }),
        ),
        (
            "op missing parent",
            "NotFound",
            false,
            Box::new(|volume| {
                volume
                    .window_op(
                        &BatchOp::DeleteFile {
                            parent_id: u64::MAX,
                            name: "child",
                        },
                        ts(4),
                    )
                    .map(|_| ())
            }),
        ),
        (
            "op parent is a file",
            "NotDirectory",
            false,
            Box::new(move |volume| {
                volume
                    .window_op(
                        &BatchOp::DeleteFile {
                            parent_id: anchor,
                            name: "child",
                        },
                        ts(4),
                    )
                    .map(|_| ())
            }),
        ),
        (
            "op invalid name",
            "InvalidName(Invalid(\"name length out of range\"))",
            false,
            Box::new(|volume| {
                volume
                    .window_op(
                        &BatchOp::Rename {
                            source_parent_id: OBJECT_ROOT,
                            source_name: "",
                            target_parent_id: OBJECT_ROOT,
                            target_name: "anchor",
                            replace: true,
                        },
                        ts(4),
                    )
                    .map(|_| ())
            }),
        ),
        (
            "op preflight read failure",
            "Block(Injected(\"window preflight read\"))",
            true,
            Box::new(|volume| {
                volume
                    .window_op(
                        &BatchOp::DeleteFile {
                            parent_id: OBJECT_ROOT,
                            name: "absent",
                        },
                        ts(4),
                    )
                    .map(|_| ())
            }),
        ),
        (
            "write invalid time",
            "InvalidMetadata(\"timestamp nanoseconds out of range\")",
            false,
            Box::new(move |volume| volume.window_write_file_at(anchor, 0, b"x", BAD_TIME)),
        ),
        (
            "write offset overflow",
            "PrototypeLimit(\"file size limit reached\")",
            false,
            Box::new(move |volume| volume.window_write_file_at(anchor, u64::MAX, b"x", ts(4))),
        ),
        (
            "write missing object",
            "NotFound",
            false,
            Box::new(|volume| volume.window_write_file_at(u64::MAX, 0, b"x", ts(4))),
        ),
        (
            "write to a directory",
            "IsDirectory",
            false,
            Box::new(move |volume| volume.window_write_file_at(directory, 0, b"x", ts(4))),
        ),
        (
            "write layout read failure",
            "Block(Injected(\"window preflight read\"))",
            true,
            Box::new(move |volume| volume.window_write_file_at(anchor, 0, b"x", ts(4))),
        ),
        (
            "truncate invalid time",
            "InvalidMetadata(\"timestamp nanoseconds out of range\")",
            false,
            Box::new(move |volume| volume.window_truncate_file(anchor, 10, BAD_TIME)),
        ),
        (
            "truncate missing object",
            "NotFound",
            false,
            Box::new(|volume| volume.window_truncate_file(u64::MAX, 10, ts(4))),
        ),
        (
            "truncate a directory",
            "IsDirectory",
            false,
            Box::new(move |volume| volume.window_truncate_file(directory, 10, ts(4))),
        ),
        (
            "truncate record read failure",
            "Block(Injected(\"window preflight read\"))",
            true,
            Box::new(move |volume| volume.window_truncate_file(anchor, 10, ts(4))),
        ),
        (
            "commit invalid time",
            "InvalidMetadata(\"timestamp nanoseconds out of range\")",
            false,
            Box::new(|volume| volume.window_commit(BAD_TIME)),
        ),
    ];
    let mut refusals = 0;
    for (label, expected, arm, call) in calls {
        volume.device_mut().armed = arm;
        let error = call(&mut volume).expect_err(label);
        assert_eq!(format!("{error:?}"), expected, "{label}");
        assert!(!volume.device_mut().armed, "{label}: no preflight read");
        assert_eq!(
            volume.window_unlogged_ops(),
            pending,
            "{label}: window lost"
        );
        let stats = volume.device_mut().inner.stats();
        assert_eq!(
            (stats.writes, stats.flushes),
            (before.writes, before.flushes),
            "{label}: refusal issued I/O"
        );
        assert_eq!(volume.generation(), generation, "{label}: generation");
        refusals += 1;
    }

    // An empty write and an unchanged truncate are admitted no-ops.
    volume.window_write_file_at(anchor, 0, b"", ts(4)).unwrap();
    volume
        .window_truncate_file(anchor, BS as u64, ts(4))
        .unwrap();
    assert_eq!(volume.window_unlogged_ops(), pending);
    let stats = volume.device_mut().inner.stats();
    assert_eq!(
        (stats.writes, stats.flushes),
        (before.writes, before.flushes),
        "no-op issued I/O"
    );

    // The staged group survives every refusal and becomes durable.
    volume.window_fsync().unwrap();
    let logged = volume.into_device().inner.into_inner();
    let raw = matrix::open_mode(logged.clone(), pages, MountMode::NoChanges);
    assert_eq!(raw.generation(), generation);
    assert_eq!(raw.pending_intent_records(), 2);
    drop(raw);
    let mut recovered = matrix::open_mode(logged, pages, MountMode::Recovery);
    assert_eq!(recovered.generation(), generation + 1);
    for (name, bytes) in [
        ("durable", b"durable bytes".as_slice()),
        ("staged", b"staged bytes".as_slice()),
    ] {
        let id = recovered.lookup_root(name).unwrap().unwrap();
        assert_eq!(recovered.read_file(id).unwrap(), bytes, "{name}");
    }
    assert_eq!(recovered.read_file(anchor).unwrap(), vec![BASE_BYTE; BS]);
    assert_eq!(recovered.list_root().unwrap().len(), 4);
    matrix::assert_checker_clean(recovered.device_mut(), "window refusals");

    // A no-changes mount refuses every entry point without I/O.
    let mut read_only =
        matrix::open_mode(TraceBackend::new(base.clone()), pages, MountMode::NoChanges);
    read_only.device_mut().reset();
    type Denied =
        Box<dyn FnOnce(&mut Volume<TraceBackend<MemoryBackend>>) -> Result<(), CoreError>>;
    let denied: Vec<(&str, Denied)> = vec![
        (
            "read-only op",
            Box::new(|volume| {
                volume
                    .window_op(
                        &BatchOp::CreateFile {
                            parent_id: OBJECT_ROOT,
                            name: "denied",
                            content: b"",
                        },
                        ts(5),
                    )
                    .map(|_| ())
            }),
        ),
        (
            "read-only write",
            Box::new(move |volume| volume.window_write_file_at(anchor, 0, b"x", ts(5))),
        ),
        (
            "read-only truncate",
            Box::new(move |volume| volume.window_truncate_file(anchor, 0, ts(5))),
        ),
        ("read-only fsync", Box::new(|volume| volume.window_fsync())),
        (
            "read-only commit",
            Box::new(|volume| volume.window_commit(ts(5))),
        ),
    ];
    for (label, call) in denied {
        let result = call(&mut read_only);
        assert!(
            matches!(result, Err(CoreError::ReadOnly)),
            "{label}: {result:?}"
        );
        refusals += 1;
    }
    let stats = read_only.device_mut().stats();
    assert_eq!((stats.writes, stats.flushes), (0, 0), "read-only I/O");
    eprintln!("window entry refusals pages={pages} refusals={refusals}");
}

/// A `PrototypeLimit` refusal of an oversized fsync group keeps the window.
fn window_group_limit_refusal(pages: usize) {
    let format = Format {
        log_slots: 1,
        ..Format::new(1024, 256)
    };
    let mut volume = matrix::open(TraceBackend::new(format.device()), pages);
    let generation = volume.generation();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "first",
                content: b"first bytes",
            },
            ts(1),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "second",
                content: b"second bytes",
            },
            ts(2),
        )
        .unwrap();
    let pending = volume.window_unlogged_ops();
    let before = volume.device_mut().stats();
    let error = volume.window_fsync().expect_err("one slot is full");
    assert_eq!(
        format!("{error:?}"),
        "PrototypeLimit(\"intent log is full; commit the window\")"
    );
    assert_eq!(volume.window_unlogged_ops(), pending);
    let after = volume.device_mut().stats();
    assert_eq!(
        (after.writes, after.flushes),
        (before.writes, before.flushes),
        "refused fsync issued I/O"
    );
    assert_eq!(volume.generation(), generation);
    // The window still commits, publishing both groups as one checkpoint.
    volume.window_commit(ts(3)).unwrap();
    assert_eq!(volume.generation(), generation + 1);
    let mut recovered = matrix::open(volume.into_device().into_inner(), pages);
    for (name, bytes) in [
        ("first", b"first bytes".as_slice()),
        ("second", b"second bytes".as_slice()),
    ] {
        let id = recovered.lookup_root(name).unwrap().unwrap();
        assert_eq!(recovered.read_file(id).unwrap(), bytes, "{name}");
    }
    matrix::assert_checker_clean(recovered.device_mut(), "group limit refusal");
    eprintln!("window group-limit refusal pages={pages}");
}

crate::profile_tests!(namespace_replay, |pages| matrix::replay_with_faults(
    &DeferredNamespace,
    pages,
    Variant::Plain,
    12
));
crate::profile_tests!(namespace_replay_retained, |pages| matrix::replay(
    &DeferredNamespace,
    pages,
    Variant::Retained,
    12
));
crate::profile_tests!(write_replay, |pages| matrix::replay_sampled(
    &DeferredWrite,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de11_0001
));
crate::profile_tests!(write_replay_retained, |pages| matrix::replay_sampled(
    &DeferredWrite,
    pages,
    Variant::Retained,
    12,
    SAMPLE,
    0x5eed_de11_0002
));
crate::profile_tests!(truncate_replay, |pages| matrix::replay_sampled(
    &DeferredTruncate,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de11_0003
));
crate::profile_tests!(truncate_replay_retained, |pages| matrix::replay_sampled(
    &DeferredTruncate,
    pages,
    Variant::Retained,
    12,
    SAMPLE,
    0x5eed_de11_0004
));
crate::profile_tests!(mixed_replay, |pages| matrix::replay_sampled(
    &DeferredMixed,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de11_0005
));
crate::profile_tests!(mixed_replay_retained, |pages| matrix::replay_sampled(
    &DeferredMixed,
    pages,
    Variant::Retained,
    12,
    SAMPLE,
    0x5eed_de11_0006
));
crate::profile_tests!(window_refusals, |pages| window_entry_refusals(pages));
crate::profile_tests!(window_group_limit, |pages| window_group_limit_refusal(
    pages
));
