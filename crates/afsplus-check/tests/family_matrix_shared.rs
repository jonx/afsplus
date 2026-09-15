//! Shared-ownership families through the family-matrix driver; see
//! tiny_cache_matrix.md. The families cover a replace rename whose incoming
//! object is unshared, a shared write that splits a run, the unlink of one of
//! three owners and the removal of a final owner.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::shared_extents::{self, SharedRun};
use afsplus_core::volume::{ObjectMetadata, PreservedMetadata};
use afsplus_core::{CoreError, Volume};
use afsplus_format::OBJECT_ROOT;
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

/// Volume geometry of the bounded fixtures.
const SMALL: u64 = 512;
/// Volume geometry of the pre-populated eviction fixtures.
const WIDE: u64 = 1024;
/// Volume geometry of the low-space refusal fixtures.
const TIGHT: u64 = 256;
/// Entries the eviction fixtures hold in the root directory.
const POPULATION: usize = 150;
/// Long root names used to force staged-node eviction.
const NAME_LENGTH: usize = 240;

fn shared_format(variant: Variant) -> Format {
    match variant {
        Variant::Eviction => Format::new(WIDE, WIDE as u32),
        Variant::Refusal => Format::new(TIGHT, TIGHT as u32),
        _ => Format::new(SMALL, SMALL as u32),
    }
}

/// Canonical reference records of the selected checkpoint.
fn shared_runs<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<SharedRun> {
    let root = volume.checkpoint().shared_extent_root_block;
    if root == 0 {
        return Vec::new();
    }
    let geometry = volume.ident().geometry();
    let generation = volume.generation();
    shared_extents::load_all(volume.device_mut(), &geometry, root, generation)
        .unwrap()
        .records
}

/// Block counts and reference counts of every record, in canonical order.
fn run_shape<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<(u64, u32)> {
    shared_runs(volume)
        .into_iter()
        .map(|run| (run.block_count, run.reference_count))
        .collect()
}

/// Reads the whole file with a sentinel past the end, so a longer file fails.
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

fn population(volume: &mut Volume<MemoryBackend>, variant: Variant) -> usize {
    if variant == Variant::Eviction {
        matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
        POPULATION
    } else {
        0
    }
}

/// Fills the volume and drains the reclaim queue until at most one block is
/// available, so the shared write cannot copy the block it must leave. The
/// corrective step deletes the filler and reclaims its space.
fn ballast(volume: &mut Volume<MemoryBackend>) -> u64 {
    let filler = volume.create_file_in_root("ballast", b"", ts(9)).unwrap();
    let mut offset = 0u64;
    for _round in 0..32 {
        let mut misses = 0;
        // A refused block write can leave a block that a differently shaped
        // write still fits, so try further offsets before giving up.
        while misses < 4 {
            match volume.write_file_at(filler, offset, &[0x5b; BS], ts(9)) {
                Ok(()) => misses = 0,
                Err(CoreError::NoSpace) => misses += 1,
                Err(error) => panic!("filling the refusal fixture: {error}"),
            }
            offset += BS as u64;
        }
        drain_reclaim(volume, ts(9));
        if volume.available_blocks() <= 1 {
            break;
        }
    }
    assert!(
        volume.available_blocks() <= 1,
        "refusal fixture keeps {} blocks available",
        volume.available_blocks()
    );
    filler
}

/// Consumes the reclaim queue until it stops returning blocks.
fn drain_reclaim<D: BlockDevice>(volume: &mut Volume<D>, now: afsplus_format::Timespec) {
    volume.set_reclaim_batch_blocks(1024);
    while volume.reclaim_pending_blocks() > 0 && volume.reclaim_step(now).unwrap() > 0 {}
}

/// Deletes the filler and reclaims its blocks. Reclaimed blocks become
/// available only once a later checkpoint replaces the slot that still reaches
/// them, so the step alternates reclaim with a scratch create and delete until
/// the volume has room again.
fn release_ballast<D: BlockDevice>(volume: &mut Volume<D>, nudge: u64) -> u64 {
    let before = volume.generation();
    volume.delete_file_in_root("ballast", ts(40)).unwrap();
    for round in 0..16 {
        drain_reclaim(volume, ts(41));
        if volume.available_blocks() >= RETRY_BLOCKS {
            break;
        }
        // Restoring the object's own metadata publishes a checkpoint without
        // changing any observed state and without needing a free block.
        let record = volume.stat(nudge).unwrap().unwrap();
        let metadata = PreservedMetadata::from(ObjectMetadata::from(record));
        volume.restore_object_metadata(nudge, metadata).unwrap();
        let _ = round;
    }
    drain_reclaim(volume, ts(60));
    assert!(
        volume.available_blocks() >= RETRY_BLOCKS,
        "the corrective step left {} blocks available",
        volume.available_blocks()
    );
    volume.generation() - before
}

/// Blocks the corrective step must make available before the retry.
const RETRY_BLOCKS: u64 = 8;

/// Two blocks with a distinct byte each.
const RUN_BLOCKS: usize = 2;

fn origin_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(RUN_BLOCKS * BS);
    for block in 0..RUN_BLOCKS {
        bytes.extend(std::iter::repeat_n(0x21 + block as u8, BS));
    }
    bytes
}

// ---------------------------------------------------------------------------
// Replace rename of an unshared target
// ---------------------------------------------------------------------------

const INCOMING: &[u8] = b"incoming bytes that replace the victim";
const VICTIM: &[u8] = b"victim bytes that the replacement retires";

struct ReplaceState {
    incoming: Option<u64>,
    victim: u64,
    population: usize,
}

struct ReplaceUnshared;

impl Family for ReplaceUnshared {
    type State = ReplaceState;

    fn name(&self) -> &'static str {
        "replace rename of an unshared target"
    }

    fn format(&self, variant: Variant) -> Format {
        if variant == Variant::Refusal {
            // The refusal is a missing source, so the bounded geometry serves.
            Format::new(SMALL, SMALL as u32)
        } else {
            shared_format(variant)
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> ReplaceState {
        let population = population(volume, variant);
        let victim = volume.create_file_in_root("victim", VICTIM, ts(2)).unwrap();
        let incoming = (variant != Variant::Refusal).then(|| {
            volume
                .create_file_in_root("incoming", INCOMING, ts(3))
                .unwrap()
        });
        ReplaceState {
            incoming,
            victim,
            population,
        }
    }

    fn captured(&self, state: &ReplaceState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.victim, VICTIM.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ReplaceState,
    ) -> Result<(), CoreError> {
        volume.rename_replace(OBJECT_ROOT, "incoming", OBJECT_ROOT, "victim", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ReplaceState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        let incoming = state.incoming;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 1 + usize::from(incoming.is_some() && !published),
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("incoming").unwrap(),
            (!published).then_some(incoming).flatten(),
            "{context}: source entry"
        );
        let at_victim = volume
            .lookup_root("victim")
            .unwrap()
            .unwrap_or_else(|| panic!("{context}: the target name must always resolve"));
        if published {
            assert_eq!(Some(at_victim), incoming, "{context}: replacement identity");
            file_bytes(volume, at_victim, INCOMING, context);
            assert!(
                volume.stat(state.victim).unwrap().is_none(),
                "{context}: the replaced object survived"
            );
        } else {
            assert_eq!(at_victim, state.victim, "{context}: victim identity");
            file_bytes(volume, state.victim, VICTIM, context);
            if let Some(incoming) = incoming {
                file_bytes(volume, incoming, INCOMING, context);
            }
        }
        assert!(run_shape(volume).is_empty(), "{context}: shared runs");
    }

    fn eviction_demand(&self) -> u64 {
        REPLACE_DEMAND
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::NotFound)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut ReplaceState) -> u64 {
        state.incoming = Some(
            volume
                .create_file_in_root("incoming", INCOMING, ts(40))
                .unwrap(),
        );
        1
    }
}

// ---------------------------------------------------------------------------
// Shared write
// ---------------------------------------------------------------------------

const SHARED_WRITE_OFFSET: u64 = 40;
const SHARED_WRITE_LENGTH: usize = 200;

fn origin_after() -> Vec<u8> {
    let mut bytes = origin_bytes();
    let start = SHARED_WRITE_OFFSET as usize;
    bytes[start..start + SHARED_WRITE_LENGTH].fill(0x9d);
    bytes
}

struct SharedState {
    origin: u64,
    peers: Vec<u64>,
    population: usize,
    filled: bool,
}

/// Creates the origin and `peers` clones of it.
fn shared_fixture(
    volume: &mut Volume<MemoryBackend>,
    variant: Variant,
    peers: usize,
) -> SharedState {
    let population = population(volume, variant);
    let origin = volume
        .create_file_in_root("origin", &origin_bytes(), ts(2))
        .unwrap();
    let peers: Vec<u64> = (0..peers)
        .map(|index| {
            volume
                .clone_file(
                    origin,
                    OBJECT_ROOT,
                    &format!("peer-{index}"),
                    ts(3 + index as i64),
                )
                .unwrap()
        })
        .collect();
    let filled = variant == Variant::Refusal;
    if filled {
        ballast(volume);
    }
    SharedState {
        origin,
        peers,
        population,
        filled,
    }
}

fn verify_peers<D: BlockDevice>(volume: &mut Volume<D>, state: &SharedState, context: &str) {
    for (index, peer) in state.peers.iter().enumerate() {
        assert_eq!(
            volume.lookup_root(&format!("peer-{index}")).unwrap(),
            Some(*peer),
            "{context}: peer entry {index}"
        );
        file_bytes(volume, *peer, &origin_bytes(), context);
    }
}

struct SharedWrite;

impl Family for SharedWrite {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "write into a shared run"
    }

    fn format(&self, variant: Variant) -> Format {
        shared_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        shared_fixture(volume, variant, 1)
    }

    fn captured(&self, state: &SharedState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.origin, origin_bytes()),
            (state.peers[0], origin_bytes()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
    ) -> Result<(), CoreError> {
        volume.write_file_at(
            state.origin,
            SHARED_WRITE_OFFSET,
            &[0x9d; SHARED_WRITE_LENGTH],
            ts(30),
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
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 2 + usize::from(state.filled),
            "{context}: root entries"
        );
        let expected = if published {
            origin_after()
        } else {
            origin_bytes()
        };
        file_bytes(volume, state.origin, &expected, context);
        verify_peers(volume, state, context);
        // The write moves the origin off the first block of the run, which
        // leaves the peer as the only owner of the remaining block.
        let expected = if published {
            vec![((RUN_BLOCKS - 1) as u64, 2)]
        } else {
            vec![(RUN_BLOCKS as u64, 2)]
        };
        assert_eq!(run_shape(volume), expected, "{context}: shared runs");
    }

    fn eviction_demand(&self) -> u64 {
        SHARED_WRITE_DEMAND
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::NoSpace)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SharedState) -> u64 {
        state.filled = false;
        release_ballast(volume, state.origin)
    }
}

// ---------------------------------------------------------------------------
// Unlink of one of three owners
// ---------------------------------------------------------------------------

struct SharedUnlink;

impl Family for SharedUnlink {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "unlink one of three owners"
    }

    fn format(&self, variant: Variant) -> Format {
        shared_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        shared_fixture(volume, variant, 2)
    }

    fn captured(&self, state: &SharedState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.origin, origin_bytes()),
            (state.peers[0], origin_bytes()),
            (state.peers[1], origin_bytes()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &SharedState,
    ) -> Result<(), CoreError> {
        volume.delete_file_in_root("peer-1", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 3 - usize::from(published),
            "{context}: root entries"
        );
        file_bytes(volume, state.origin, &origin_bytes(), context);
        file_bytes(volume, state.peers[0], &origin_bytes(), context);
        let gone = state.peers[1];
        if published {
            assert_eq!(volume.lookup_root("peer-1").unwrap(), None, "{context}");
            assert!(volume.stat(gone).unwrap().is_none(), "{context}: {gone}");
        } else {
            assert_eq!(
                volume.lookup_root("peer-1").unwrap(),
                Some(gone),
                "{context}"
            );
            file_bytes(volume, gone, &origin_bytes(), context);
        }
        // Three owners drop to two; the run keeps every block either way.
        let references = if published { 2 } else { 3 };
        assert_eq!(
            run_shape(volume),
            vec![(RUN_BLOCKS as u64, references)],
            "{context}: shared runs"
        );
    }

    fn eviction_demand(&self) -> u64 {
        SHARED_UNLINK_DEMAND
    }
}

// ---------------------------------------------------------------------------
// Removal of the final owner
// ---------------------------------------------------------------------------

struct FinalOwner;

impl Family for FinalOwner {
    type State = SharedState;

    fn name(&self) -> &'static str {
        "unlink the last of two owners"
    }

    fn format(&self, variant: Variant) -> Format {
        shared_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SharedState {
        shared_fixture(volume, variant, 1)
    }

    fn captured(&self, state: &SharedState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.origin, origin_bytes()),
            (state.peers[0], origin_bytes()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &SharedState,
    ) -> Result<(), CoreError> {
        volume.delete_file_in_root("peer-0", ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &SharedState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 2 - usize::from(published),
            "{context}: root entries"
        );
        file_bytes(volume, state.origin, &origin_bytes(), context);
        let gone = state.peers[0];
        if published {
            assert_eq!(volume.lookup_root("peer-0").unwrap(), None, "{context}");
            assert!(volume.stat(gone).unwrap().is_none(), "{context}: {gone}");
            // The survivor is the sole owner, so no reference record remains.
            assert!(run_shape(volume).is_empty(), "{context}: shared runs");
        } else {
            file_bytes(volume, gone, &origin_bytes(), context);
            assert_eq!(
                run_shape(volume),
                vec![(RUN_BLOCKS as u64, 2)],
                "{context}: shared runs"
            );
        }
    }

    fn eviction_demand(&self) -> u64 {
        FINAL_OWNER_DEMAND
    }
}

// ---------------------------------------------------------------------------
// Generated tests
// ---------------------------------------------------------------------------

/// Measured resident staged-node demands at the unlimited profile. Each of
/// these transactions descends one object-map path over the populated root, so
/// the fixtures evict at two pages and the measured demand is their limit.
const REPLACE_DEMAND: u64 = 3;
const SHARED_WRITE_DEMAND: u64 = 1;
const SHARED_UNLINK_DEMAND: u64 = 3;
const FINAL_OWNER_DEMAND: u64 = 3;

crate::profile_tests!(replace_unshared, |pages| matrix::plain(
    &ReplaceUnshared,
    pages,
    12
));
crate::profile_tests!(replace_unshared_retained, |pages| matrix::retained(
    &ReplaceUnshared,
    pages,
    12
));
crate::profile_tests!(replace_unshared_refusal, |pages| matrix::refusal(
    &ReplaceUnshared,
    pages
));
crate::profile_tests!(replace_unshared_eviction, |pages| matrix::eviction(
    &ReplaceUnshared,
    pages,
    None
));
crate::profile_tests!(shared_write_retained, |pages| matrix::retained(
    &SharedWrite,
    pages,
    12
));
crate::profile_tests!(shared_write_refusal, |pages| matrix::refusal(
    &SharedWrite,
    pages
));
crate::profile_tests!(shared_write_eviction, |pages| matrix::eviction(
    &SharedWrite,
    pages,
    None
));
crate::profile_tests!(shared_unlink, |pages| matrix::plain(
    &SharedUnlink,
    pages,
    12
));
crate::profile_tests!(shared_unlink_retained, |pages| matrix::retained(
    &SharedUnlink,
    pages,
    12
));
crate::profile_tests!(shared_unlink_eviction, |pages| matrix::eviction(
    &SharedUnlink,
    pages,
    None
));
crate::profile_tests!(final_owner, |pages| matrix::plain(&FinalOwner, pages, 12));
crate::profile_tests!(final_owner_retained, |pages| matrix::retained(
    &FinalOwner,
    pages,
    12
));
crate::profile_tests!(final_owner_eviction, |pages| matrix::eviction(
    &FinalOwner,
    pages,
    None
));
