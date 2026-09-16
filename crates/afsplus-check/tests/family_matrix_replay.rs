//! Shared-ownership and orphan replay through the family-matrix replay
//! runner: durable groups whose recovery publishes orphan state, replaces a
//! shared target or splits a shared run, plus the resource refusal of a
//! deferred window. See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend, TraceBackend};
use afsplus_core::shared_extents;
use afsplus_core::volume::BatchOp;
use afsplus_core::{CoreError, MountMode, Volume};
use afsplus_format::OBJECT_ROOT;

use common::family_matrix::{self as matrix, ts, Format, ReplayFamily, Variant, BS};

const SLOTS: u16 = 8;
/// Long root names of the eviction fixture.
const POPULATION: usize = 300;
const NAME_LENGTH: usize = 240;
/// Seeded full-write subsets drawn per flush segment of a recovery campaign.
const SAMPLE: usize = 128;

fn image(variant: Variant) -> Format {
    match variant {
        Variant::Eviction => Format {
            log_slots: SLOTS,
            ..Format::new(16 * 1024, 16)
        },
        Variant::Exhausted => Format {
            log_slots: SLOTS,
            ..Format::new(4096, 4096)
        },
        _ => Format {
            log_slots: SLOTS,
            ..Format::new(4096, 4096)
        },
    }
}

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

/// Reads the whole file with a sentinel past the end.
fn file_bytes<D: BlockDevice>(volume: &mut Volume<D>, id: u64, expected: &[u8], context: &str) {
    let mut read = vec![0xa5; expected.len() + 1];
    assert_eq!(
        volume.read_file_at(id, 0, &mut read).unwrap(),
        expected.len(),
        "{context}: length of {id}"
    );
    assert_eq!(&read[..expected.len()], expected, "{context}: bytes of {id}");
    assert_eq!(read[expected.len()], 0xa5, "{context}: EOF of {id}");
}

fn populate(volume: &mut Volume<MemoryBackend>, variant: Variant) -> usize {
    if variant == Variant::Eviction {
        matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
        POPULATION
    } else {
        0
    }
}

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

// ------------------------------------------------------- orphaning a victim

/// One durable group whose delete removes a fragmented file's final link, so
/// recovery must publish bounded orphan state instead of retiring the layout.
struct OrphanFinalDelete;

struct VictimState {
    victim: u64,
    bytes: Vec<u8>,
    allocated: u64,
    background: usize,
    filler: Option<u64>,
}

impl ReplayFamily for OrphanFinalDelete {
    type State = VictimState;

    fn name(&self) -> &'static str {
        "orphan replay delete"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> VictimState {
        let background = populate(volume, variant);
        let (victim, bytes) = fragmented(volume, "victim");
        let allocated = volume
            .visible_metadata(victim)
            .unwrap()
            .unwrap()
            .allocated_bytes;
        // Ordinary capacity consumed down to the emergency headroom: replay is
        // privileged maintenance and must still publish the transition.
        let filler = (variant == Variant::Exhausted).then(|| {
            let filler = volume.create_file_in_root("filler", b"", ts(40)).unwrap();
            let fill = volume.available_blocks().saturating_sub(24);
            volume
                .preallocate_file(filler, 0, fill * BS as u64, ts(41))
                .unwrap();
            assert!(volume.available_blocks() <= 32);
            filler
        });
        VictimState {
            victim,
            bytes,
            allocated,
            background,
            filler,
        }
    }

    fn captured(&self, state: &VictimState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.victim, state.bytes.clone())]
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
        file_bytes(volume, state.victim, &state.bytes, context);
        assert_eq!(
            volume
                .visible_metadata(state.victim)
                .unwrap()
                .unwrap()
                .allocated_bytes,
            state.allocated,
            "{context}: allocated bytes"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + usize::from(named) + usize::from(state.filler.is_some()),
            "{context}: root entries"
        );
    }

    /// Root-directory, object-map and orphan-directory paths of the replay
    /// commit.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

// ------------------------------------------- a logged write and a final link

/// A durable existing-file write followed by a durable final delete: the
/// orphan must carry the replayed bytes.
struct OrphanWriteThenDelete;

const PATCH: (u64, u8, usize) = (BS as u64 - 17, 0xa5, BS);

impl ReplayFamily for OrphanWriteThenDelete {
    type State = VictimState;

    fn name(&self) -> &'static str {
        "orphan replay write and delete"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> VictimState {
        let background = populate(volume, variant);
        let victim = volume
            .create_file_in_root("victim", &[0x11; 2 * BS], ts(1))
            .unwrap();
        let bytes = volume.read_file(victim).unwrap();
        let allocated = volume
            .visible_metadata(victim)
            .unwrap()
            .unwrap()
            .allocated_bytes;
        VictimState {
            victim,
            bytes,
            allocated,
            background,
            filler: None,
        }
    }

    fn captured(&self, state: &VictimState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.victim, state.bytes.clone())]
    }

    fn groups(&self) -> usize {
        2
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &VictimState,
        index: usize,
    ) -> Result<(), CoreError> {
        if index == 0 {
            let (offset, value, length) = PATCH;
            volume.window_write_file_at(state.victim, offset, &vec![value; length], ts(50))?;
        } else {
            volume.window_op(
                &BatchOp::DeleteFile {
                    parent_id: OBJECT_ROOT,
                    name: "victim",
                },
                ts(51),
            )?;
        }
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
        let mut expected = state.bytes.clone();
        if acknowledged >= 1 {
            let (offset, value, length) = PATCH;
            expected[offset as usize..offset as usize + length].fill(value);
        }
        let named = acknowledged < 2;
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
        file_bytes(volume, state.victim, &expected, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + usize::from(named),
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        4
    }
}

// ----------------------------------------------------- replacing rename

/// A durable replacing rename: recovery must expose the incoming object under
/// the target name and orphan the replaced victim with its exact bytes.
struct OrphanReplacingRename;

struct ReplaceState {
    victim: u64,
    victim_bytes: Vec<u8>,
    incoming: u64,
    background: usize,
}

const INCOMING: &[u8] = b"incoming-content";

impl ReplayFamily for OrphanReplacingRename {
    type State = ReplaceState;

    fn name(&self) -> &'static str {
        "orphan replay replacing rename"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> ReplaceState {
        let background = populate(volume, variant);
        let (victim, victim_bytes) = fragmented(volume, "target");
        let incoming = volume
            .create_file_in_root("incoming", INCOMING, ts(7))
            .unwrap();
        ReplaceState {
            victim,
            victim_bytes,
            incoming,
            background,
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
            Some(if replaced { state.incoming } else { state.victim }),
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
        file_bytes(volume, state.victim, &state.victim_bytes, context);
        file_bytes(volume, state.incoming, INCOMING, context);
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + if replaced { 1 } else { 2 },
            "{context}: root entries"
        );
    }

    fn eviction_demand(&self) -> u64 {
        4
    }
}

// ------------------------------------------------------- shared ownership

/// A durable unlink of one of three owners of a two-block run.
struct SharedUnlinkReplay;

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

impl ReplayFamily for SharedUnlinkReplay {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "shared replay unlink"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        let background = populate(volume, variant);
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
            background,
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
        // Replay of a final link moves the victim into orphan state, so it
        // keeps its share of the run until orphan cleanup runs.
        assert_eq!(
            volume.orphan_object(state.victim).unwrap(),
            removed,
            "{context}: orphan flag"
        );
        file_bytes(volume, state.origin, &state.bytes, context);
        file_bytes(volume, state.peer, &state.bytes, context);
        file_bytes(volume, state.victim, &state.bytes, context);
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
        4
    }
}

/// A durable existing-file write that moves one owner off a shared block.
struct SharedWriteReplay;

const SPLIT: (u64, u8, usize) = (40, 0x8c, 64);

impl ReplayFamily for SharedWriteReplay {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "shared replay write"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        let background = populate(volume, variant);
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
            background,
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
        let mut expected = state.bytes.clone();
        if acknowledged == 1 {
            let (offset, value, length) = SPLIT;
            expected[offset as usize..offset as usize + length].fill(value);
        }
        file_bytes(volume, state.origin, &expected, context);
        file_bytes(volume, state.peer, &state.bytes, context);
        assert_eq!(
            run_shape(volume),
            if acknowledged == 1 {
                // The rewritten first block leaves the run; the second stays.
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
        4
    }
}

// ------------------------------------------------------------ many orphans

/// One durable group deleting sixteen files: recovery creates every orphan
/// lazily inside one checkpoint.
struct ManyOrphans;

struct ManyState {
    ids: Vec<u64>,
    background: usize,
}

const MANY: usize = 16;

impl ReplayFamily for ManyOrphans {
    type State = ManyState;

    fn name(&self) -> &'static str {
        "orphan replay group"
    }

    fn format(&self, variant: Variant) -> Format {
        image(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> ManyState {
        let background = populate(volume, variant);
        let ids = (0..MANY)
            .map(|index| {
                volume
                    .create_file_in_root(
                        &format!("orphan-{index:02x}"),
                        &[index as u8; 32],
                        ts(index as i64 + 1),
                    )
                    .unwrap()
            })
            .collect();
        ManyState { ids, background }
    }

    fn captured(&self, state: &ManyState) -> Vec<(u64, Vec<u8>)> {
        state
            .ids
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, vec![index as u8; 32]))
            .collect()
    }

    fn groups(&self) -> usize {
        1
    }

    fn log_group<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ManyState,
        _index: usize,
    ) -> Result<(), CoreError> {
        for index in 0..MANY {
            let name = format!("orphan-{index:02x}");
            volume.window_op(
                &BatchOp::DeleteFile {
                    parent_id: OBJECT_ROOT,
                    name: &name,
                },
                ts(50),
            )?;
        }
        volume.window_fsync()
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ManyState,
        _variant: Variant,
        acknowledged: usize,
        context: &str,
    ) {
        let removed = acknowledged == 1;
        for (index, id) in state.ids.iter().enumerate() {
            let name = format!("orphan-{index:02x}");
            assert_eq!(
                volume.lookup_root(&name).unwrap(),
                (!removed).then_some(*id),
                "{context}: {name}"
            );
            assert_eq!(
                volume.orphan_object(*id).unwrap(),
                removed,
                "{context}: orphan flag of {name}"
            );
            file_bytes(volume, *id, &[index as u8; 32], context);
        }
        assert_eq!(
            volume.orphan_count().unwrap(),
            if removed { MANY as u64 } else { 0 },
            "{context}: orphan count"
        );
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.background + if removed { 0 } else { MANY },
            "{context}: root entries"
        );
    }

    /// Root directory, object map, orphan directory and allocation root.
    fn eviction_demand(&self) -> u64 {
        4
    }
}

// -------------------------------------------------------- resource refusal

/// A deferred window that runs out of ordinary space refuses the operation
/// with no flush, keeps the acknowledged and staged work, and succeeds after
/// the specified corrective step.
fn deferred_window_refusal(pages: usize) {
    let format = Format {
        log_slots: SLOTS,
        ..Format::new(256, 256)
    };
    let mut volume = matrix::open(TraceBackend::new(format.device()), pages);
    // Ordinary capacity is consumed before the window opens, so the deferred
    // create of a multi-block payload has nowhere to put its data.
    let filler = volume.create_file_in_root("filler", b"", ts(2)).unwrap();
    let fill = volume.available_blocks().saturating_sub(24);
    volume
        .preallocate_file(filler, 0, fill * BS as u64, ts(3))
        .unwrap();
    assert!(volume.available_blocks() <= 32);
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
            ts(5),
        )
        .unwrap();
    let pending = volume.window_unlogged_ops();
    volume.device_mut().reset();
    let big = vec![0x2c; 40 * BS];
    let error = volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "refused",
                content: &big,
            },
            ts(6),
        )
        .expect_err("no ordinary space remains");
    assert!(matches!(error, CoreError::NoSpace), "{error:?}");
    let stats = volume.device_mut().stats();
    assert_eq!(stats.flushes, 0, "refusal flushed");
    assert_eq!(volume.window_unlogged_ops(), pending, "window lost");
    assert_eq!(volume.generation(), generation, "refusal published");
    assert_eq!(volume.lookup_root("refused").unwrap(), None);
    eprintln!("deferred refusal pages={pages} writes={} flushes=0", stats.writes);

    // The corrective step frees the filler; the same call then succeeds.
    volume.window_commit(ts(7)).unwrap();
    volume.delete_file_in_root("filler", ts(7)).unwrap();
    volume.set_reclaim_batch_blocks(1024);
    let mut commits = 2;
    while volume.available_blocks() < 64 {
        volume.reclaim_step(ts(8)).unwrap();
        volume.create_file_in_root("scratch", b"", ts(8)).unwrap();
        volume.delete_file_in_root("scratch", ts(8)).unwrap();
        commits += 3;
    }
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "refused",
                content: &big,
            },
            ts(9),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    let logged = volume.into_device().into_inner();
    let raw = matrix::open_mode(logged.clone(), pages, MountMode::NoChanges);
    assert_eq!(raw.pending_intent_records(), 1);
    drop(raw);
    let mut recovered = matrix::open_mode(logged, pages, MountMode::Recovery);
    for (name, bytes) in [
        ("durable", b"".as_slice()),
        ("staged", b"".as_slice()),
        ("refused", big.as_slice()),
    ] {
        let id = recovered.lookup_root(name).unwrap().unwrap();
        file_bytes(&mut recovered, id, bytes, "deferred refusal retry");
    }
    matrix::assert_checker_clean(recovered.device_mut(), "deferred refusal retry");
    eprintln!("deferred refusal pages={pages} corrective_commits={commits}");
}

/// Every replayed group is idempotent: recovering the published image again
/// keeps the generation and the state.
fn repeated_recovery_is_stable(pages: usize) {
    let format = Format {
        log_slots: SLOTS,
        ..Format::new(1024, 256)
    };
    let mut volume = matrix::open(format.device(), pages);
    let file = volume
        .create_file_in_root("file", &[0x33; BS], ts(1))
        .unwrap();
    volume
        .window_write_file_at(file, 10, &[0x44; 100], ts(2))
        .unwrap();
    volume.window_fsync().unwrap();
    volume
        .window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "file",
            },
            ts(3),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    let logged = volume.into_device();
    let mut expected = vec![0x33; BS];
    expected[10..110].fill(0x44);
    let mut device = logged;
    let mut generation = None;
    for round in 0..3 {
        let mut recovered = matrix::open_mode(device, pages, MountMode::Recovery);
        let observed = recovered.generation();
        if let Some(first) = generation {
            assert_eq!(observed, first, "round {round}: generation moved");
        }
        generation = Some(observed);
        assert_eq!(recovered.pending_intent_records(), 0);
        assert!(recovered.orphan_object(file).unwrap());
        file_bytes(&mut recovered, file, &expected, "repeated recovery");
        matrix::assert_checker_clean(recovered.device_mut(), "repeated recovery");
        device = recovered.into_device();
    }
    eprintln!("repeated recovery pages={pages} generation={generation:?}");
}

crate::profile_tests!(orphan_delete_replay, |pages| matrix::replay_sampled(
    &OrphanFinalDelete,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de1e_0001
));
crate::profile_tests!(orphan_delete_replay_retained, |pages| matrix::replay_sampled(
    &OrphanFinalDelete,
    pages,
    Variant::Retained,
    12,
    SAMPLE,
    0x5eed_de1e_0002
));
crate::profile_tests!(orphan_delete_replay_exhausted, |pages| {
    matrix::replay_sampled(
        &OrphanFinalDelete,
        pages,
        Variant::Exhausted,
        12,
        SAMPLE,
        0x5eed_de1e_0003,
    )
});
crate::profile_tests!(orphan_delete_replay_eviction, |pages| {
    matrix::replay_sampled(
        &OrphanFinalDelete,
        pages,
        Variant::Eviction,
        12,
        SAMPLE,
        0x5eed_de1e_0004,
    )
});
crate::profile_tests!(orphan_write_delete_replay, |pages| matrix::replay_sampled(
    &OrphanWriteThenDelete,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de1e_0005
));
crate::profile_tests!(orphan_replace_replay, |pages| matrix::replay_sampled(
    &OrphanReplacingRename,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de1e_0006
));
crate::profile_tests!(orphan_replace_replay_retained, |pages| {
    matrix::replay_sampled(
        &OrphanReplacingRename,
        pages,
        Variant::Retained,
        12,
        SAMPLE,
        0x5eed_de1e_0007,
    )
});
crate::profile_tests!(shared_unlink_replay, |pages| matrix::replay_sampled(
    &SharedUnlinkReplay,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de1e_0008
));
crate::profile_tests!(shared_unlink_replay_retained, |pages| {
    matrix::replay_sampled(
        &SharedUnlinkReplay,
        pages,
        Variant::Retained,
        12,
        SAMPLE,
        0x5eed_de1e_0009,
    )
});
crate::profile_tests!(shared_write_replay, |pages| matrix::replay_sampled(
    &SharedWriteReplay,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de1e_000a
));
crate::profile_tests!(shared_write_replay_retained, |pages| {
    matrix::replay_sampled(
        &SharedWriteReplay,
        pages,
        Variant::Retained,
        12,
        SAMPLE,
        0x5eed_de1e_000b,
    )
});
crate::profile_tests!(many_orphans_replay, |pages| matrix::replay_sampled(
    &ManyOrphans,
    pages,
    Variant::Plain,
    12,
    SAMPLE,
    0x5eed_de1e_000c
));
crate::profile_tests!(deferred_refusal, |pages| deferred_window_refusal(pages));
crate::profile_tests!(repeated_recovery, |pages| repeated_recovery_is_stable(pages));
