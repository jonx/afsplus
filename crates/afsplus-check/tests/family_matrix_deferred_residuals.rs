//! Residual deferred-window combinations of the tiny-cache matrix: forced
//! eviction of the deferred and replay recovery commits, the resource
//! refusals of the two existing-file window entry points, read failures
//! inside `window_fsync` and `window_commit`, and faults at every write and
//! flush of the intent-record publication. See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend, TraceBackend};
use afsplus_core::shared_extents;
use afsplus_core::volume::BatchOp;
use afsplus_core::{CoreError, MountMode, Volume};
use afsplus_format::OBJECT_ROOT;

use common::family_matrix::{self as matrix, ts, Format, ReplayFamily, Variant, BS};

/// Log slots of every fixture in this file.
const SLOTS: u16 = 8;
/// Long root names of the eviction fixtures.
const POPULATION: usize = 128;
const NAME_LENGTH: usize = 240;

fn image(variant: Variant) -> Format {
    match variant {
        Variant::Eviction => Format {
            log_slots: SLOTS,
            ..Format::new(4096, 16)
        },
        _ => Format {
            log_slots: SLOTS,
            ..Format::new(4096, 4096)
        },
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
        matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
    }
}

/// Reads the whole file with a sentinel past the end.
fn file_bytes<D: BlockDevice>(volume: &mut Volume<D>, id: u64, expected: &[u8], context: &str) {
    let mut read = vec![0xa5; expected.len() + 1];
    assert_eq!(
        volume.read_file_at(id, 0, &mut read).unwrap(),
        expected.len(),
        "{context}: length of {id}"
    );
    assert_eq!(
        &read[..expected.len()],
        expected,
        "{context}: bytes of {id}"
    );
    assert_eq!(read[expected.len()], 0xa5, "{context}: EOF of {id}");
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

/// Three durable namespace groups over a wide root: a create, a rename, and a
/// create paired with a delete inside one group.
struct DeferredNamespace;

struct NamespaceState {
    anchor: u64,
}

const ANCHOR: &[u8] = b"anchor-bytes-kept-through-every-group";
const ALPHA: &[u8] = b"alpha-bytes";
const GAMMA: &[u8] = b"gamma-bytes";
/// Independent spellings the oracle expects, so a changed operation input
/// fails the test.
const EXPECTED_ANCHOR: &[u8] = b"anchor-bytes-kept-through-every-group";
const EXPECTED_ALPHA: &[u8] = b"alpha-bytes";
const EXPECTED_GAMMA: &[u8] = b"gamma-bytes";

impl ReplayFamily for DeferredNamespace {
    type State = NamespaceState;

    fn name(&self) -> &'static str {
        "deferred namespace residual"
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
        assert_bytes(volume, state.anchor, EXPECTED_ANCHOR, context);
        let names: &[&str] = match acknowledged {
            0 => &[],
            1 => &["alpha"],
            2 => &["beta"],
            _ => &["gamma"],
        };
        assert_root(volume, variant, "anchor", names, context);
        for name in names {
            let id = volume.lookup_root(name).unwrap().unwrap();
            let expected = if *name == "gamma" {
                EXPECTED_GAMMA
            } else {
                EXPECTED_ALPHA
            };
            assert_bytes(volume, id, expected, context);
        }
    }

    fn eviction_demand(&self) -> u64 {
        NAMESPACE_DEMAND
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
/// Independent expectation of the three patched windows.
const EXPECTED_PATCHES: [(u64, u8, usize); 3] =
    [(73, 0xc7, 211), (4096 + 10, 0x5a, 300), (2, 0x3e, 4100)];
/// Independent expectation of the untouched byte value.
const EXPECTED_BASE: u8 = 0x18;

fn patched(count: usize) -> Vec<u8> {
    let mut bytes = vec![EXPECTED_BASE; 3 * BS];
    for (offset, value, length) in EXPECTED_PATCHES.iter().take(count) {
        let start = *offset as usize;
        bytes[start..start + length].fill(*value);
    }
    bytes
}

impl ReplayFamily for DeferredWrite {
    type State = WriteState;

    fn name(&self) -> &'static str {
        "deferred write residual"
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

    fn eviction_demand(&self) -> u64 {
        WRITE_DEMAND
    }
}

// ----------------------------------------------------------------- truncate

/// Three durable truncates: a partial shrink, a sparse growth and an aligned
/// shrink.
struct DeferredTruncate;

const SIZES: [u64; 3] = [BS as u64 + 211, 3 * BS as u64 + 50, BS as u64];
/// Independent expectation of the three published sizes.
const EXPECTED_SIZES: [u64; 3] = [4307, 12338, 4096];

fn resized(count: usize) -> Vec<u8> {
    let mut bytes = vec![EXPECTED_BASE; 3 * BS];
    for size in EXPECTED_SIZES.iter().take(count) {
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
        "deferred truncate residual"
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

    fn eviction_demand(&self) -> u64 {
        TRUNCATE_DEMAND
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
/// Independent expectations of both windows and of the published size.
const EXPECTED_MIXED_PATCH: (u64, u8, usize) = (100, 0x9b, 500);
const EXPECTED_MIXED_SECOND: (u64, u8, usize) = (4103, 0x4d, 64);
const EXPECTED_MIXED_SIZE: u64 = 8102;

fn mixed_bytes(acknowledged: usize) -> Vec<u8> {
    let mut bytes = vec![EXPECTED_BASE; 2 * BS];
    if acknowledged >= 1 {
        let (offset, value, length) = EXPECTED_MIXED_PATCH;
        bytes[offset as usize..offset as usize + length].fill(value);
        bytes.truncate(EXPECTED_MIXED_SIZE as usize);
    }
    if acknowledged >= 2 {
        let (offset, value, length) = EXPECTED_MIXED_SECOND;
        bytes[offset as usize..offset as usize + length].fill(value);
    }
    bytes
}

impl ReplayFamily for DeferredMixed {
    type State = MixedState;

    fn name(&self) -> &'static str {
        "deferred mixed window residual"
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
            assert_bytes(volume, id, EXPECTED_GAMMA, context);
        }
    }

    fn eviction_demand(&self) -> u64 {
        MIXED_DEMAND
    }
}

// ------------------------------------------------------------ shared replay

/// Block counts and reference counts of every reference record.
fn run_shape<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<(u64, u32)> {
    let root = volume.checkpoint().shared_extent_root_block;
    if root == 0 {
        return Vec::new();
    }
    let geometry = volume.ident().geometry();
    let generation = volume.generation();
    shared_extents::load_all(volume.device_mut(), &geometry, root, generation)
        .unwrap()
        .records
        .into_iter()
        .map(|run| (run.block_count, run.reference_count))
        .collect()
}

struct SharedState {
    origin: u64,
    peer: u64,
    victim: u64,
    bytes: Vec<u8>,
    background: usize,
}

fn shared_bytes() -> Vec<u8> {
    let mut bytes = vec![0x71; BS];
    bytes.extend_from_slice(&[0x72; BS]);
    bytes
}

/// Independent expectation of the two shared block values.
const EXPECTED_SHARED: [u8; 2] = [0x71, 0x72];

fn expected_shared_bytes() -> Vec<u8> {
    let mut bytes = vec![EXPECTED_SHARED[0]; BS];
    bytes.extend_from_slice(&[EXPECTED_SHARED[1]; BS]);
    bytes
}

/// A durable unlink of one of three owners of a two-block run.
struct SharedUnlinkReplay;

impl ReplayFamily for SharedUnlinkReplay {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "shared replay unlink residual"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        setup_background(volume, variant);
        let bytes = shared_bytes();
        let origin = volume.create_file_in_root("origin", &bytes, ts(1)).unwrap();
        let peer = volume
            .clone_file(origin, OBJECT_ROOT, "peer", ts(2))
            .unwrap();
        let victim = volume
            .clone_file(origin, OBJECT_ROOT, "victim", ts(3))
            .unwrap();
        SharedState {
            origin,
            peer,
            victim,
            bytes,
            background: background(variant),
        }
    }

    fn captured(&self, state: &SharedState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.origin, state.bytes.clone()),
            (state.peer, state.bytes.clone()),
            (state.victim, state.bytes.clone()),
        ]
    }

    fn groups(&self) -> usize {
        1
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &SharedState,
        _index: usize,
    ) -> Result<(), CoreError> {
        volume.window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "victim",
            },
            ts(50),
        )?;
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
        _variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let removed = acknowledged == 1;
        assert_eq!(
            volume.lookup_root("victim").unwrap(),
            (!removed).then_some(state.victim),
            "{context}: victim entry"
        );
        assert_eq!(
            volume.orphan_object(state.victim).unwrap(),
            removed,
            "{context}: orphan flag"
        );
        let expected = expected_shared_bytes();
        file_bytes(volume, state.origin, &expected, context);
        file_bytes(volume, state.peer, &expected, context);
        file_bytes(volume, state.victim, &expected, context);
        assert_eq!(
            run_shape(volume),
            vec![(2, 3)],
            "{context}: reference records"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + if removed { 2 } else { 3 },
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        SHARED_UNLINK_DEMAND
    }
}

/// A durable existing-file write that moves one owner off a shared block.
struct SharedWriteReplay;

const SPLIT: (u64, u8, usize) = (40, 0x8c, 64);
/// Independent expectation of the split window.
const EXPECTED_SPLIT: (u64, u8, usize) = (40, 0x8c, 64);

impl ReplayFamily for SharedWriteReplay {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "shared replay write residual"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        setup_background(volume, variant);
        let bytes = shared_bytes();
        let origin = volume.create_file_in_root("origin", &bytes, ts(1)).unwrap();
        let peer = volume
            .clone_file(origin, OBJECT_ROOT, "peer", ts(2))
            .unwrap();
        SharedState {
            origin,
            peer,
            victim: origin,
            bytes,
            background: background(variant),
        }
    }

    fn captured(&self, state: &SharedState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.origin, state.bytes.clone()),
            (state.peer, state.bytes.clone()),
        ]
    }

    fn groups(&self) -> usize {
        1
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
        _index: usize,
    ) -> Result<(), CoreError> {
        let (offset, value, length) = SPLIT;
        volume.window_write_file_at(state.origin, offset, &vec![value; length], ts(50))?;
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
        _variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let mut expected = expected_shared_bytes();
        if acknowledged == 1 {
            let (offset, value, length) = EXPECTED_SPLIT;
            expected[offset as usize..offset as usize + length].fill(value);
        }
        file_bytes(volume, state.origin, &expected, context);
        file_bytes(volume, state.peer, &expected_shared_bytes(), context);
        assert_eq!(
            run_shape(volume),
            if acknowledged == 1 {
                vec![(1, 2)]
            } else {
                vec![(2, 2)]
            },
            "{context}: reference records"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + 2,
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        SHARED_WRITE_DEMAND
    }
}

// --------------------------------------------------------- replacing rename

/// A durable replacing rename: recovery exposes the incoming object under the
/// target name and orphans the replaced victim with its exact bytes.
struct OrphanReplacingRename;

struct ReplaceState {
    victim: u64,
    victim_bytes: Vec<u8>,
    incoming: u64,
    background: usize,
}

const INCOMING: &[u8] = b"incoming-content";
/// Independent expectation of the incoming payload.
const EXPECTED_INCOMING: &[u8] = b"incoming-content";
/// A fragmented victim: one written block at every second logical block.
const EXTENTS: u64 = 3;

fn fragmented(volume: &mut Volume<MemoryBackend>, name: &str) -> (u64, Vec<u8>) {
    let id = volume.create_file_in_root(name, b"", ts(1)).unwrap();
    for extent in 0..EXTENTS {
        volume
            .write_file_at(
                id,
                extent * 2 * BS as u64,
                &[0xd0 + extent as u8; BS],
                ts(2 + extent as i64),
            )
            .unwrap();
    }
    let bytes = volume.read_file(id).unwrap();
    (id, bytes)
}

/// Independent expectation of the fragmented victim: three written blocks at
/// logical blocks 0, 2 and 4 with one hole between each pair.
fn expected_fragmented() -> Vec<u8> {
    let mut bytes = Vec::new();
    for extent in 0..3u8 {
        bytes.extend_from_slice(&[0xd0 + extent; BS]);
        if extent < 2 {
            bytes.extend_from_slice(&[0u8; BS]);
        }
    }
    bytes
}

impl ReplayFamily for OrphanReplacingRename {
    type State = ReplaceState;

    fn name(&self) -> &'static str {
        "orphan replay replacing rename residual"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> ReplaceState {
        setup_background(volume, variant);
        let (victim, victim_bytes) = fragmented(volume, "target");
        let incoming = volume
            .create_file_in_root("incoming", INCOMING, ts(7))
            .unwrap();
        ReplaceState {
            victim,
            victim_bytes,
            incoming,
            background: background(variant),
        }
    }

    fn captured(&self, state: &ReplaceState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.victim, state.victim_bytes.clone()),
            (state.incoming, INCOMING.to_vec()),
        ]
    }

    fn groups(&self) -> usize {
        1
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ReplaceState,
        _index: usize,
    ) -> Result<(), CoreError> {
        volume.window_op(
            &BatchOp::Rename {
                source_parent_id: OBJECT_ROOT,
                source_name: "incoming",
                target_parent_id: OBJECT_ROOT,
                target_name: "target",
                replace: true,
            },
            ts(50),
        )?;
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ReplaceState,
        _variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let replaced = acknowledged == 1;
        assert_eq!(
            volume.lookup_root("target").unwrap(),
            Some(if replaced {
                state.incoming
            } else {
                state.victim
            }),
            "{context}: target entry"
        );
        assert_eq!(
            volume.lookup_root("incoming").unwrap(),
            (!replaced).then_some(state.incoming),
            "{context}: incoming entry"
        );
        assert_eq!(
            volume.orphan_object(state.victim).unwrap(),
            replaced,
            "{context}: orphan flag"
        );
        file_bytes(volume, state.victim, &expected_fragmented(), context);
        file_bytes(volume, state.incoming, EXPECTED_INCOMING, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + if replaced { 1 } else { 2 },
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        REPLACE_DEMAND
    }
}

// ------------------------------------------------------- orphan final delete

/// One durable group whose delete removes a fragmented file's final link.
struct OrphanFinalDelete;

struct VictimState {
    victim: u64,
    background: usize,
}

impl ReplayFamily for OrphanFinalDelete {
    type State = VictimState;

    fn name(&self) -> &'static str {
        "orphan replay delete residual"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> VictimState {
        setup_background(volume, variant);
        let (victim, _) = fragmented(volume, "victim");
        VictimState {
            victim,
            background: background(variant),
        }
    }

    fn captured(&self, state: &VictimState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.victim, expected_fragmented())]
    }

    fn groups(&self) -> usize {
        1
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &VictimState,
        _index: usize,
    ) -> Result<(), CoreError> {
        volume.window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "victim",
            },
            ts(50),
        )?;
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &VictimState,
        _variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let named = acknowledged == 0;
        assert_eq!(
            volume.lookup_root("victim").unwrap(),
            named.then_some(state.victim),
            "{context}: victim entry"
        );
        assert_eq!(
            volume.orphan_object(state.victim).unwrap(),
            !named,
            "{context}: orphan flag"
        );
        assert_eq!(
            volume.orphan_count().unwrap(),
            u64::from(!named),
            "{context}: orphan count"
        );
        file_bytes(volume, state.victim, &expected_fragmented(), context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + usize::from(named),
            "{context}: root entries"
        );
    }
}

// ------------------------------------- resource refusals of the update paths

/// Ordinary capacity the window leaves for its own later work.
const RESERVE_BLOCKS: u64 = 24;
/// Blocks the refused write asks for.
const REFUSED_BLOCKS: usize = 40;
/// Size the refused truncate asks for; its tail needs one fresh block.
const REFUSED_SIZE: u64 = BS as u64 + 11;
/// Independent expectation of the retried write value and of the size the
/// retried truncate publishes.
const RETRY_VALUE: u8 = 0x2c;
const EXPECTED_RETRY_SIZE: usize = 4107;

/// `window_write_file_at` and `window_truncate_file` refuse for lack of
/// ordinary space with no write and no flush, keep the staged window and the
/// acknowledged group, and admit the same calls after the specified
/// corrective step.
fn window_update_refusals(pages: usize) {
    let format = Format {
        log_slots: SLOTS,
        ..Format::new(256, 256)
    };
    let mut volume = matrix::open(TraceBackend::new(format.device()), pages);
    let anchor = volume
        .create_file_in_root("anchor", &vec![BASE_BYTE; 2 * BS], ts(1))
        .unwrap();
    // Ordinary capacity is consumed before the window opens.
    let filler = volume.create_file_in_root("filler", b"", ts(2)).unwrap();
    let fill = volume.available_blocks().saturating_sub(RESERVE_BLOCKS);
    volume
        .preallocate_file(filler, 0, fill * BS as u64, ts(3))
        .unwrap();
    assert!(volume.available_blocks() <= 32, "the filler left capacity");
    let generation = volume.generation();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "durable",
                content: b"",
            },
            ts(4),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "staged",
                content: b"",
            },
            ts(4),
        )
        .unwrap();
    // One-block creates consume the remainder of the window allocator;
    // `window_op` keeps the window when the last of them is refused.
    let mut pads = 0;
    loop {
        let name = format!("pad-{pads:02}");
        match volume.window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: &name,
                content: &[0x5a; BS],
            },
            ts(4),
        ) {
            Ok(_) => pads += 1,
            Err(CoreError::NoSpace) => break,
            Err(error) => panic!("padding the window: {error:?}"),
        }
        assert!(pads < 64, "the window never runs out of ordinary space");
    }
    let pending = volume.window_unlogged_ops();
    let original = vec![BASE_BYTE; 2 * BS];

    let mut refusals = 0;
    for (label, call) in [("write", 0usize), ("truncate", 1usize)] {
        volume.device_mut().reset();
        let error = match call {
            0 => volume
                .window_write_file_at(anchor, 0, &vec![RETRY_VALUE; REFUSED_BLOCKS * BS], ts(5))
                .expect_err("no ordinary space remains"),
            _ => volume
                .window_truncate_file(anchor, REFUSED_SIZE, ts(5))
                .expect_err("no ordinary space remains"),
        };
        assert!(matches!(error, CoreError::NoSpace), "{label}: {error:?}");
        let stats = volume.device_mut().stats();
        assert_eq!(
            (stats.writes, stats.flushes),
            (0, 0),
            "{label}: refusal issued I/O"
        );
        assert_eq!(
            volume.window_unlogged_ops(),
            pending,
            "{label}: the staged window was lost"
        );
        assert_eq!(
            volume.generation(),
            generation,
            "{label}: refusal published"
        );
        file_bytes(&mut volume, anchor, &original, label);
        refusals += 1;
    }
    eprintln!("window update refusals pages={pages} refusals={refusals} pads={pads}");

    // The window that both refusals preserved holds exactly one acknowledged
    // group, so the specified corrective step is a remount that keeps that
    // group and frees the filler.
    let image = volume.into_device().into_inner();
    let raw = matrix::open_mode(image.clone(), pages, MountMode::NoChanges);
    assert_eq!(raw.generation(), generation, "the refusals published");
    assert_eq!(raw.pending_intent_records(), 1, "acknowledged groups");
    drop(raw);
    let mut volume = matrix::open_mode(image, pages, MountMode::Recovery);
    assert_eq!(
        volume.generation(),
        generation + 1,
        "recovery publishes one checkpoint"
    );
    file_bytes(&mut volume, anchor, &original, "recovered fixture");
    let durable = volume.lookup_root("durable").unwrap().unwrap();
    file_bytes(&mut volume, durable, b"", "recovered fixture");
    assert_eq!(
        volume.lookup_root("staged").unwrap(),
        None,
        "an unlogged operation survived the remount"
    );
    for index in 0..pads {
        assert_eq!(
            volume.lookup_root(&format!("pad-{index:02}")).unwrap(),
            None,
            "an unlogged pad survived the remount"
        );
    }
    assert_eq!(
        volume.list_root().unwrap().len(),
        3,
        "recovered root entries"
    );
    matrix::assert_checker_clean(volume.device_mut(), "recovered fixture");

    // Freeing the filler and reclaiming its blocks admits the same two calls.
    let mut volume = matrix::open(volume.into_device(), pages);
    volume.delete_file_in_root("filler", ts(6)).unwrap();
    volume.set_reclaim_batch_blocks(1024);
    let mut commits = 2;
    while volume.available_blocks() < 128 {
        volume.reclaim_step(ts(7)).unwrap();
        volume.create_file_in_root("scratch", b"", ts(7)).unwrap();
        volume.delete_file_in_root("scratch", ts(7)).unwrap();
        commits += 3;
    }
    volume
        .window_write_file_at(anchor, 0, &vec![RETRY_VALUE; REFUSED_BLOCKS * BS], ts(8))
        .unwrap();
    volume
        .window_truncate_file(anchor, REFUSED_SIZE, ts(8))
        .unwrap();
    volume.window_fsync().unwrap();
    let logged = volume.into_device();
    let raw = matrix::open_mode(logged.clone(), pages, MountMode::NoChanges);
    assert_eq!(raw.pending_intent_records(), 1, "retried group");
    drop(raw);
    let mut recovered = matrix::open_mode(logged, pages, MountMode::Recovery);
    file_bytes(
        &mut recovered,
        anchor,
        &vec![RETRY_VALUE; EXPECTED_RETRY_SIZE],
        "window update retry",
    );
    assert_eq!(
        recovered.stat(anchor).unwrap().unwrap().size_bytes,
        EXPECTED_RETRY_SIZE as u64,
        "retried size"
    );
    let durable = recovered.lookup_root("durable").unwrap().unwrap();
    file_bytes(&mut recovered, durable, b"", "window update retry");
    assert_eq!(
        recovered.list_root().unwrap().len(),
        2,
        "root entries after the corrective step"
    );
    assert_eq!(
        recovered.lookup_root("filler").unwrap(),
        None,
        "the corrective step removed the filler"
    );
    matrix::assert_checker_clean(recovered.device_mut(), "window update retry");
    let mut again = matrix::open(recovered.into_device(), pages);
    file_bytes(
        &mut again,
        anchor,
        &vec![RETRY_VALUE; EXPECTED_RETRY_SIZE],
        "window update remount",
    );
    matrix::assert_checker_clean(again.device_mut(), "window update remount");
    eprintln!("window update refusals pages={pages} corrective_commits={commits}");
}

// --------------------------------------- read failures of fsync and commit

/// Fails the `fail_at`th read issued while the device is armed.
struct FailNthRead {
    inner: MemoryBackend,
    reads: u64,
    fail_at: Option<u64>,
    armed: bool,
    tripped: bool,
}

impl FailNthRead {
    fn new(inner: MemoryBackend, fail_at: Option<u64>) -> Self {
        Self {
            inner,
            reads: 0,
            fail_at,
            armed: false,
            tripped: false,
        }
    }
}

impl BlockDevice for FailNthRead {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if self.armed {
            let index = self.reads;
            self.reads += 1;
            if self.fail_at == Some(index) {
                self.tripped = true;
                return Err(BlockError::Injected("window publication read"));
            }
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

const FIRST: &[u8] = b"first-group-bytes";
const SECOND: &[u8] = b"second-group-bytes";
/// Independent spellings of the two group payloads.
const EXPECTED_FIRST: &[u8] = b"first-group-bytes";
const EXPECTED_SECOND: &[u8] = b"second-group-bytes";
const FSYNC_PATCH: (u64, u8, usize) = (10, 0x5b, 100);
const EXPECTED_FSYNC_PATCH: (u64, u8, usize) = (10, 0x5b, 100);

/// The anchor bytes once the acknowledged write group is replayed.
fn acknowledged_anchor() -> Vec<u8> {
    let mut bytes = vec![EXPECTED_BASE; BS];
    let (offset, value, length) = EXPECTED_FSYNC_PATCH;
    bytes[offset as usize..offset as usize + length].fill(value);
    bytes
}

fn read_fault_base(pages: usize) -> (MemoryBackend, u64, u64) {
    let format = Format {
        log_slots: SLOTS,
        ..Format::new(1024, 256)
    };
    let mut volume = matrix::open(format.device(), pages);
    let anchor = volume
        .create_file_in_root("anchor", &vec![BASE_BYTE; BS], ts(1))
        .unwrap();
    let generation = volume.generation();
    (volume.into_device(), anchor, generation)
}

/// Stages one acknowledged namespace group, one acknowledged write group and
/// one unlogged create.
fn stage_window<D: BlockDevice>(volume: &mut Volume<D>, anchor: u64) {
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "first",
                content: FIRST,
            },
            ts(10),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    let (offset, value, length) = FSYNC_PATCH;
    volume
        .window_write_file_at(anchor, offset, &vec![value; length], ts(11))
        .unwrap();
    volume.window_fsync().unwrap();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "second",
                content: SECOND,
            },
            ts(12),
        )
        .unwrap();
}

/// `window_fsync` issues no read at all, so no read failure can reach it;
/// every read of `window_commit` fails in turn, publishing nothing and
/// requiring a remount that recovers exactly the acknowledged groups.
fn window_publication_read_failures(pages: usize) {
    let (base, anchor, generation) = read_fault_base(pages);

    // The fsync of a namespace group and the fsync of a data group, each with
    // the device armed to fail its very first read.
    let mut volume = matrix::open(FailNthRead::new(base.clone(), Some(0)), pages);
    let mut fsync_reads = Vec::new();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "first",
                content: FIRST,
            },
            ts(10),
        )
        .unwrap();
    volume.device_mut().reads = 0;
    volume.device_mut().armed = true;
    volume.window_fsync().expect("fsync reads nothing");
    volume.device_mut().armed = false;
    fsync_reads.push(volume.device_mut().reads);
    let (offset, value, length) = FSYNC_PATCH;
    volume
        .window_write_file_at(anchor, offset, &vec![value; length], ts(11))
        .unwrap();
    volume.device_mut().reads = 0;
    volume.device_mut().armed = true;
    volume.window_fsync().expect("a data fsync reads nothing");
    volume.device_mut().armed = false;
    fsync_reads.push(volume.device_mut().reads);
    assert_eq!(fsync_reads, vec![0, 0], "window_fsync issued a read");
    assert!(
        !volume.device_mut().tripped,
        "window_fsync tripped the fault"
    );
    drop(volume);

    // Every read of the publication itself.
    let commit_reads = {
        let mut volume = matrix::open(FailNthRead::new(base.clone(), None), pages);
        stage_window(&mut volume, anchor);
        volume.device_mut().reads = 0;
        volume.device_mut().armed = true;
        volume.window_commit(ts(20)).unwrap();
        volume.device_mut().reads
    };
    assert!(commit_reads > 0, "window_commit issued no read");

    let mut outcomes = [0u64; 2];
    for index in 0..commit_reads {
        let context = format!("window_commit read {index} of {commit_reads} pages={pages}");
        let mut volume = matrix::open(FailNthRead::new(base.clone(), Some(index)), pages);
        stage_window(&mut volume, anchor);
        volume.device_mut().reads = 0;
        volume.device_mut().armed = true;
        let error = volume
            .window_commit(ts(20))
            .expect_err("the injected read failure is reported");
        volume.device_mut().armed = false;
        assert!(
            matches!(error, CoreError::Block(BlockError::Injected(_))),
            "{context}: {error:?}"
        );
        assert!(volume.device_mut().tripped, "{context}: no fault injected");
        assert_eq!(volume.generation(), generation, "{context}: published");
        // Remount-required semantics: no further mutation is admitted.
        assert!(
            matches!(
                volume.create_file_in_root("probe", b"", ts(21)),
                Err(CoreError::WindowPoisoned)
            ),
            "{context}: an independent mutation was admitted"
        );
        assert!(
            matches!(volume.window_commit(ts(21)), Err(CoreError::WindowPoisoned)),
            "{context}: the same publication was admitted"
        );
        let image = volume.into_device().inner;
        let mut probe = image.clone();
        matrix::assert_checker_clean(&mut probe, &context);
        let raw = matrix::open_mode(probe, pages, MountMode::NoChanges);
        // A read that fails before the checkpoint slot leaves the acknowledged
        // groups pending; one that fails after it leaves the whole window
        // published, which is the ambiguous publication of the same rule.
        let published = raw
            .generation()
            .checked_sub(generation)
            .filter(|delta| *delta <= 1)
            .unwrap_or_else(|| panic!("{context}: disallowed generation {}", raw.generation()));
        assert_eq!(
            raw.pending_intent_records(),
            if published == 1 { 0 } else { 2 },
            "{context}: acknowledged records"
        );
        drop(raw);
        outcomes[published as usize] += 1;
        let mut recovered = matrix::open_mode(image, pages, MountMode::Recovery);
        assert_eq!(
            recovered.generation(),
            generation + 1,
            "{context}: recovery publishes one checkpoint"
        );
        file_bytes(&mut recovered, anchor, &acknowledged_anchor(), &context);
        let first = recovered.lookup_root("first").unwrap().unwrap();
        file_bytes(&mut recovered, first, EXPECTED_FIRST, &context);
        assert_eq!(
            recovered.lookup_root("second").unwrap().is_some(),
            published == 1,
            "{context}: the unlogged create"
        );
        assert_eq!(
            recovered.lookup_root("probe").unwrap(),
            None,
            "{context}: the refused probe survived"
        );
        assert_eq!(
            recovered.list_root().unwrap().len(),
            2 + published as usize,
            "{context}: root entries"
        );
        matrix::assert_checker_clean(recovered.device_mut(), &context);
        // The retry of the lost operation reaches the complete state.
        let mut recovered = matrix::open(recovered.into_device(), pages);
        if published == 0 {
            recovered
                .window_op(
                    &BatchOp::CreateFile {
                        parent_id: OBJECT_ROOT,
                        name: "second",
                        content: SECOND,
                    },
                    ts(22),
                )
                .unwrap();
            recovered.window_commit(ts(22)).unwrap();
        }
        let second = recovered.lookup_root("second").unwrap().unwrap();
        file_bytes(&mut recovered, second, EXPECTED_SECOND, &context);
        let mut again = matrix::open(recovered.into_device(), pages);
        file_bytes(&mut again, anchor, &acknowledged_anchor(), &context);
        let second = again.lookup_root("second").unwrap().unwrap();
        file_bytes(&mut again, second, EXPECTED_SECOND, &context);
        assert_eq!(
            again.list_root().unwrap().len(),
            3,
            "{context}: root entries"
        );
        matrix::assert_checker_clean(again.device_mut(), &context);
    }
    assert!(
        outcomes[0] > 0,
        "a read failure before publication is required: {outcomes:?}"
    );
    eprintln!(
        "window publication read failures pages={pages} fsync_reads=0 commit_reads={commit_reads} published={outcomes:?}"
    );
}

// -------------------------------- measured staged-node demands of the commits

/// Root directory, object map and allocation root of the replay commit.
const NAMESPACE_DEMAND: u64 = 3;
/// Extent map and allocation root of the replay commit.
const WRITE_DEMAND: u64 = 2;
/// Extent map and allocation root of the replay commit.
const TRUNCATE_DEMAND: u64 = 2;
/// Extent map, directory and allocation root of the replay commit.
const MIXED_DEMAND: u64 = 3;
/// Root directory, orphan directory and allocation root.
const SHARED_UNLINK_DEMAND: u64 = 3;
/// Extent map and allocation root of the replay commit.
const SHARED_WRITE_DEMAND: u64 = 2;
/// Root directory, orphan directory and allocation root.
const REPLACE_DEMAND: u64 = 3;

crate::profile_tests!(namespace_replay_eviction, |pages| matrix::replay_eviction(
    &DeferredNamespace,
    pages
));
crate::profile_tests!(write_replay_eviction, |pages| matrix::replay_eviction(
    &DeferredWrite,
    pages
));
crate::profile_tests!(truncate_replay_eviction, |pages| matrix::replay_eviction(
    &DeferredTruncate,
    pages
));
crate::profile_tests!(mixed_replay_eviction, |pages| matrix::replay_eviction(
    &DeferredMixed,
    pages
));
crate::profile_tests!(shared_unlink_replay_eviction, |pages| {
    matrix::replay_eviction(&SharedUnlinkReplay, pages)
});
crate::profile_tests!(shared_write_replay_eviction, |pages| {
    matrix::replay_eviction(&SharedWriteReplay, pages)
});
crate::profile_tests!(replace_replay_eviction, |pages| matrix::replay_eviction(
    &OrphanReplacingRename,
    pages
));

crate::profile_tests!(update_refusals, |pages| window_update_refusals(pages));
crate::profile_tests!(publication_read_failures, |pages| {
    window_publication_read_failures(pages)
});

crate::profile_tests!(
    shared_unlink_logging_faults,
    |pages| matrix::replay_logging(&SharedUnlinkReplay, pages, Variant::Plain)
);
crate::profile_tests!(shared_write_logging_faults, |pages| matrix::replay_logging(
    &SharedWriteReplay,
    pages,
    Variant::Plain
));
crate::profile_tests!(replace_logging_faults, |pages| matrix::replay_logging(
    &OrphanReplacingRename,
    pages,
    Variant::Plain
));
crate::profile_tests!(
    orphan_delete_logging_faults,
    |pages| matrix::replay_logging(&OrphanFinalDelete, pages, Variant::Plain)
);
