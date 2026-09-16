//! Residual orphan, reclaim and staged-tree reload families through the
//! family-matrix driver; see tiny_cache_matrix.md. These families close the
//! open-target replacement and the orphaned-file update, orphan cleanup with
//! ordinary allocation exhausted, the reclaim sealing and segment-consumption
//! transitions, and reload read failures in the extent map, the object map and
//! the allocation root.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{object_map, CoreError, Volume};
use afsplus_format::reclaim::ReclaimCaps;
use afsplus_format::{OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

/// Seeded full-write subsets drawn per oversized flush segment.
const SAMPLE: usize = 32;
/// Long names used to force staged-node eviction.
const NAME_LENGTH: usize = 240;
/// Entries the orphan eviction fixtures hold in the root directory.
const ORPHAN_POPULATION: usize = 400;

fn accounting<D: BlockDevice>(volume: &Volume<D>) -> (u64, u64) {
    (volume.free_blocks(), volume.reclaim_pending_blocks())
}

/// Whether the selected checkpoint's object map holds internal object 2.
fn orphan_directory_present<D: BlockDevice>(volume: &mut Volume<D>) -> bool {
    let checkpoint = volume.checkpoint().clone();
    let geometry = volume.ident().geometry();
    object_map::lookup_lba(
        volume.device_mut(),
        &geometry,
        checkpoint.object_map_block,
        checkpoint.generation,
        OBJECT_ORPHAN_DIRECTORY,
    )
    .unwrap()
    .is_some()
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

fn orphan_format(variant: Variant) -> Format {
    let blocks = if variant == Variant::Eviction {
        4096
    } else {
        512
    };
    Format {
        log_slots: 8,
        ..Format::new(blocks, blocks as u32)
    }
}

fn orphan_population(volume: &mut Volume<MemoryBackend>, variant: Variant) -> usize {
    if variant == Variant::Eviction {
        matrix::populate(volume, "tree", ORPHAN_POPULATION, NAME_LENGTH);
        ORPHAN_POPULATION
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Replacement of an open target
// ---------------------------------------------------------------------------

const TARGET_BYTES: &[u8] = b"open target bytes that survive as an orphan";
const SOURCE_BYTES: &[u8] = b"source bytes that take the target name";

struct OpenTargetState {
    target: u64,
    source: u64,
    populated: usize,
}

struct OpenTargetReplace;

impl Family for OpenTargetReplace {
    type State = OpenTargetState;

    fn name(&self) -> &'static str {
        "open-target replacement"
    }

    fn format(&self, variant: Variant) -> Format {
        orphan_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> OpenTargetState {
        let populated = orphan_population(volume, variant);
        let target = volume
            .create_file_in_root("target", TARGET_BYTES, ts(2))
            .unwrap();
        let source = volume
            .create_file_in_root("source", SOURCE_BYTES, ts(3))
            .unwrap();
        OpenTargetState {
            target,
            source,
            populated,
        }
    }

    fn captured(&self, state: &OpenTargetState) -> Vec<(u64, Vec<u8>)> {
        vec![
            (state.target, TARGET_BYTES.to_vec()),
            (state.source, SOURCE_BYTES.to_vec()),
        ]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &OpenTargetState,
    ) -> Result<(), CoreError> {
        volume.rename_replace_orphan_target(OBJECT_ROOT, "source", OBJECT_ROOT, "target", ts(30))
    }

    /// The lazy orphan directory is a preparatory checkpoint of its own.
    fn publications(&self, _variant: Variant) -> u64 {
        2
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &OpenTargetState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let replaced = delta == 2;
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated + 1 + usize::from(!replaced),
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("source").unwrap(),
            (!replaced).then_some(state.source),
            "{context}: source entry"
        );
        let at_target = volume
            .lookup_root("target")
            .unwrap()
            .unwrap_or_else(|| panic!("{context}: the target name must always resolve"));
        if replaced {
            assert_eq!(at_target, state.source, "{context}: replacement identity");
        } else {
            assert_eq!(at_target, state.target, "{context}: target identity");
        }
        file_bytes(volume, state.source, SOURCE_BYTES, context);
        file_bytes(volume, state.target, TARGET_BYTES, context);
        assert_eq!(
            orphan_directory_present(volume),
            delta >= 1,
            "{context}: preparatory orphan directory"
        );
        assert_eq!(
            volume.orphan_object(state.target).unwrap(),
            replaced,
            "{context}: orphan entry"
        );
        assert_eq!(
            volume.orphan_count().unwrap(),
            u64::from(replaced),
            "{context}: orphan count"
        );
    }

    /// Root-directory, object-map and orphan-directory paths over 400 long
    /// names.
    fn eviction_demand(&self) -> u64 {
        OPEN_TARGET_DEMAND
    }
}

/// Measured resident staged-node demand of the open-target replacement.
const OPEN_TARGET_DEMAND: u64 = 3;

// ---------------------------------------------------------------------------
// Update of an orphaned file
// ---------------------------------------------------------------------------

const ORPHAN_OLD: [u8; BS] = [0x41; BS];

fn orphan_new() -> Vec<u8> {
    let mut bytes = ORPHAN_OLD.to_vec();
    bytes[100..300].fill(0x5e);
    bytes
}

struct OrphanUpdateState {
    object: u64,
    populated: usize,
}

struct OrphanUpdate;

impl Family for OrphanUpdate {
    type State = OrphanUpdateState;

    fn name(&self) -> &'static str {
        "orphaned-file update"
    }

    fn format(&self, variant: Variant) -> Format {
        orphan_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> OrphanUpdateState {
        let populated = orphan_population(volume, variant);
        let object = volume
            .create_file_in_root("open", &ORPHAN_OLD, ts(2))
            .unwrap();
        volume.orphan_file(OBJECT_ROOT, "open", ts(3)).unwrap();
        OrphanUpdateState { object, populated }
    }

    fn captured(&self, state: &OrphanUpdateState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.object, ORPHAN_OLD.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &OrphanUpdateState,
    ) -> Result<(), CoreError> {
        volume.write_file_at(state.object, 100, &[0x5e; 200], ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &OrphanUpdateState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.populated,
            "{context}: root entries"
        );
        assert_eq!(volume.lookup_root("open").unwrap(), None, "{context}");
        assert!(orphan_directory_present(volume), "{context}");
        assert!(
            volume.orphan_object(state.object).unwrap(),
            "{context}: orphan entry"
        );
        assert_eq!(volume.orphan_count().unwrap(), 1, "{context}: orphan count");
        let expected = if delta == 1 {
            orphan_new()
        } else {
            ORPHAN_OLD.to_vec()
        };
        file_bytes(volume, state.object, &expected, context);
    }

    /// Extent-map and object-map paths over 400 long names.
    fn eviction_demand(&self) -> u64 {
        ORPHAN_UPDATE_DEMAND
    }
}

/// Measured resident staged-node demand of the orphaned-file update.
const ORPHAN_UPDATE_DEMAND: u64 = 2;

// ---------------------------------------------------------------------------
// Orphan cleanup with ordinary allocation exhausted
// ---------------------------------------------------------------------------

fn fragmented_bytes() -> Vec<u8> {
    let mut bytes = vec![0; 3 * BS];
    bytes[..BS].fill(0x41);
    bytes[2 * BS..].fill(0x42);
    bytes
}

struct CleanupState {
    object: u64,
    pressure: bool,
}

struct ExhaustedCleanup;

impl Family for ExhaustedCleanup {
    type State = CleanupState;

    fn name(&self) -> &'static str {
        "orphan cleanup with ordinary allocation exhausted"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format {
            log_slots: 8,
            ..Format::new(512, 512)
        }
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> CleanupState {
        volume.set_reclaim_batch_blocks(1);
        let object = volume.create_file_in_root("frag", b"", ts(2)).unwrap();
        volume.write_file_at(object, 0, &[0x41; BS], ts(3)).unwrap();
        volume
            .write_file_at(object, 2 * BS as u64, &[0x42; BS], ts(4))
            .unwrap();
        volume.orphan_file(OBJECT_ROOT, "frag", ts(5)).unwrap();
        let pressure = variant == Variant::Exhausted;
        if pressure {
            let id = volume.create_file_in_root("pressure", b"", ts(6)).unwrap();
            // Reserve the largest range ordinary allocation admits.
            let mut reserve = volume.available_blocks();
            while volume
                .preallocate_file(id, 0, reserve * BS as u64, ts(7))
                .is_err()
            {
                reserve -= 1;
            }
            assert!(
                matches!(
                    volume.create_file_in_root("ordinary-probe", &[1; BS], ts(8)),
                    Err(CoreError::NoSpace)
                ),
                "ordinary allocation must be exhausted"
            );
        }
        CleanupState { object, pressure }
    }

    fn captured(&self, state: &CleanupState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.object, fragmented_bytes())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CleanupState,
    ) -> Result<(), CoreError> {
        volume.set_orphan_cleanup_extent_budget(2);
        volume.cleanup_orphan(state.object, ts(50)).map(|_| ())
    }

    fn publications(&self, _variant: Variant) -> u64 {
        2
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CleanupState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(volume.lookup_root("frag").unwrap(), None, "{context}");
        assert_eq!(
            volume.list_root().unwrap().len(),
            usize::from(state.pressure),
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("pressure").unwrap().is_some(),
            state.pressure,
            "{context}: pressure file"
        );
        assert!(orphan_directory_present(volume), "{context}");
        let pending = delta < 2;
        assert_eq!(
            volume.orphan_object(state.object).unwrap(),
            pending,
            "{context}: orphan entry"
        );
        assert_eq!(
            volume.orphan_count().unwrap(),
            u64::from(pending),
            "{context}: orphan count"
        );
        let metadata = volume.visible_metadata(state.object).unwrap();
        match delta {
            0 => {
                file_bytes(volume, state.object, &fragmented_bytes(), context);
                assert_eq!(
                    metadata.unwrap().allocated_bytes,
                    2 * BS as u64,
                    "{context}: allocated bytes"
                );
            }
            1 => {
                assert!(
                    volume.read_file(state.object).unwrap().is_empty(),
                    "{context}: tail-trimmed file"
                );
                assert_eq!(
                    metadata.unwrap().allocated_bytes,
                    0,
                    "{context}: allocated bytes"
                );
            }
            _ => assert!(metadata.is_none(), "{context}: removed object"),
        }
    }
}

// ---------------------------------------------------------------------------
// Reclaim sealing, segment consumption and cursor transitions
// ---------------------------------------------------------------------------

fn tiny_format() -> Format {
    Format {
        reclaim_caps: ReclaimCaps {
            inline_entries: 4,
            segment_refs: 3,
            table_refs: 8,
        },
        ..Format::new(256, 256)
    }
}

const ANCHOR: &[u8] = b"anchor bytes the reclaim transition must keep";
/// Seed entries each queue-growing fixture creates behind the snapshot.
const RECLAIM_SEEDS: usize = 6;

struct ReclaimState {
    anchor: u64,
    snapshot: u64,
    names: Vec<String>,
}

/// Creates the captured anchor and the snapshot that precedes every block the
/// transition reclaims.
fn reclaim_fixture(volume: &mut Volume<MemoryBackend>, prefix: &str) -> ReclaimState {
    volume.set_reclaim_batch_blocks(1);
    let anchor = volume.create_file_in_root("anchor", ANCHOR, ts(1)).unwrap();
    let snapshot = volume.snapshot_create(ts(2)).unwrap();
    let names: Vec<String> = (0..RECLAIM_SEEDS)
        .map(|index| format!("{prefix}{index}"))
        .collect();
    for name in &names {
        volume.create_file_in_root(name, b"seed", ts(3)).unwrap();
    }
    ReclaimState {
        anchor,
        snapshot,
        names,
    }
}

fn verify_reclaim<D: BlockDevice>(
    volume: &mut Volume<D>,
    state: &ReclaimState,
    extra: Option<&str>,
    expected: (u64, u64),
    context: &str,
) {
    let mut listed: Vec<String> = volume
        .list_root()
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    listed.sort();
    let mut wanted: Vec<String> = state.names.clone();
    wanted.push("anchor".to_string());
    if let Some(extra) = extra {
        wanted.push(extra.to_string());
    }
    wanted.sort();
    assert_eq!(listed, wanted, "{context}: root entries");
    file_bytes(volume, state.anchor, ANCHOR, context);
    assert_eq!(accounting(volume), expected, "{context}: free/pending");
}

/// Free and pending blocks of the fixture and of the published state.
const SEALING_ACCOUNTING: [(u64, u64); 2] = [(189, 23), (181, 30)];
const CONSUMPTION_ACCOUNTING: [(u64, u64); 2] = [(189, 23), (195, 23)];

struct Sealing;

impl Family for Sealing {
    type State = ReclaimState;

    fn name(&self) -> &'static str {
        "reclaim queue sealing"
    }

    fn format(&self, _variant: Variant) -> Format {
        tiny_format()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> ReclaimState {
        reclaim_fixture(volume, "seed")
    }

    fn snapshot(&self, state: &ReclaimState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &ReclaimState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.anchor, ANCHOR.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ReclaimState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(1);
        volume
            .create_file_in_root("sealer", b"payload", ts(50))
            .map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ReclaimState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        verify_reclaim(
            volume,
            state,
            (delta == 1).then_some("sealer"),
            SEALING_ACCOUNTING[delta as usize],
            context,
        );
    }

    /// The tiny caps force the recorded transaction to seal a segment.
    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ReclaimState,
        _variant: Variant,
    ) {
        let reclaim = volume.last_commit_stats().unwrap().alloc.reclaim;
        assert_eq!(
            reclaim.segments_sealed, 1,
            "the recorded transaction must seal one segment"
        );
    }
}

struct Consumption;

impl Family for Consumption {
    type State = ReclaimState;

    fn name(&self) -> &'static str {
        "reclaim segment consumption"
    }

    fn format(&self, _variant: Variant) -> Format {
        tiny_format()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> ReclaimState {
        reclaim_fixture(volume, "g")
    }

    fn snapshot(&self, state: &ReclaimState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &ReclaimState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.anchor, ANCHOR.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ReclaimState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(9);
        volume.reclaim_step(ts(60)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ReclaimState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        verify_reclaim(
            volume,
            state,
            None,
            CONSUMPTION_ACCOUNTING[delta as usize],
            context,
        );
    }

    /// The batch consumes at least one whole segment, which itself retires.
    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &ReclaimState,
        _variant: Variant,
    ) {
        let reclaim = volume.last_commit_stats().unwrap().alloc.reclaim;
        assert!(
            reclaim.structure_blocks_retired > 1,
            "the batch must consume at least one whole segment"
        );
    }
}

// ---------------------------------------------------------------------------
// Reload read failures outside the directory and batch transactions
// ---------------------------------------------------------------------------

/// Logical blocks the fragmented extent-map fixture writes.
const EXTENT_FIXTURE_BLOCKS: u64 = 160;
/// Logical blocks the recorded extent-map write covers.
const EXTENT_WRITE_BLOCKS: u64 = 48;

struct ExtentState {
    file: u64,
}

fn extent_expected(published: bool) -> Vec<u8> {
    let span = (EXTENT_FIXTURE_BLOCKS * 2 - 1) * BS as u64;
    let mut bytes = vec![0u8; span as usize];
    for index in 0..EXTENT_FIXTURE_BLOCKS {
        let start = (index * 2 * BS as u64) as usize;
        bytes[start..start + BS].fill(0x41);
    }
    if published {
        let end = (EXTENT_WRITE_BLOCKS * 2 * BS as u64) as usize;
        bytes[..end].fill(0x5e);
    }
    bytes
}

struct ExtentMapWrite;

impl Family for ExtentMapWrite {
    type State = ExtentState;

    fn name(&self) -> &'static str {
        "extent-map write"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(4096, 4096)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> ExtentState {
        let file = volume.create_file_in_root("frag", b"", ts(1)).unwrap();
        // Every other logical block is written, so the extent map holds one
        // record per block and spans several leaves.
        for index in 0..EXTENT_FIXTURE_BLOCKS {
            volume
                .write_file_at(file, index * 2 * BS as u64, &[0x41; BS], ts(2))
                .unwrap();
        }
        ExtentState { file }
    }

    fn captured(&self, _state: &ExtentState) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ExtentState,
    ) -> Result<(), CoreError> {
        let length = (EXTENT_WRITE_BLOCKS * 2 * BS as u64) as usize;
        volume.write_file_at(state.file, 0, &vec![0x5e; length], ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &ExtentState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.list_root().unwrap().len(),
            1,
            "{context}: root entries"
        );
        file_bytes(volume, state.file, &extent_expected(delta == 1), context);
    }
}

/// Promotion across eight allocation-root leaves, as in the reclaim family
/// matrix, used here for reload read failures in the allocation tree.
struct AllocationRootPromotion;

const PROMOTION_SPANS: u64 = 8;
const PROMOTION_ACCOUNTING: [(u64, u64); 2] = [(969, 600), (1478, 172)];

impl Family for AllocationRootPromotion {
    type State = ();

    fn name(&self) -> &'static str {
        "allocation-root promotion"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(16 * 1024, 16)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) {
        volume.set_reclaim_batch_blocks(1);
        let span = volume.available_blocks() / (PROMOTION_SPANS + 1);
        for index in 0..PROMOTION_SPANS {
            let name = format!("span-{index}");
            let id = volume.create_file_in_root(&name, b"", ts(1)).unwrap();
            volume
                .preallocate_file(id, 0, span * BS as u64, ts(2))
                .unwrap();
        }
        for index in 0..PROMOTION_SPANS {
            volume
                .delete_file_in_root(&format!("span-{index}"), ts(3))
                .unwrap();
        }
    }

    fn captured(&self, _state: &()) -> Vec<(u64, Vec<u8>)> {
        Vec::new()
    }

    fn apply<D: BlockDevice>(&self, volume: &mut Volume<D>, _state: &()) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(1 << 16);
        volume.reclaim_step(ts(30)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &(),
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert!(volume.list_root().unwrap().is_empty(), "{context}: root");
        assert_eq!(
            accounting(volume),
            PROMOTION_ACCOUNTING[delta as usize],
            "{context}: free/pending"
        );
    }
}

// ---------------------------------------------------------------------------
// Generated tests
// ---------------------------------------------------------------------------

const OPEN_TARGET_SEED: u64 = 0x5eed_0f3a_2001;
const ORPHAN_UPDATE_SEED: u64 = 0x5eed_0f3a_2002;

crate::profile_tests!(open_target_replace, |pages| matrix::plain(
    &OpenTargetReplace,
    pages,
    12
));
crate::profile_tests!(open_target_replace_retained, |pages| matrix::retained(
    &OpenTargetReplace,
    pages,
    12
));
crate::profile_tests!(open_target_replace_eviction, |pages| {
    matrix::eviction_sampled(&OpenTargetReplace, pages, SAMPLE, OPEN_TARGET_SEED)
});
crate::profile_tests!(orphan_update, |pages| matrix::plain(
    &OrphanUpdate,
    pages,
    12
));
crate::profile_tests!(orphan_update_retained, |pages| matrix::retained(
    &OrphanUpdate,
    pages,
    12
));
crate::profile_tests!(orphan_update_eviction, |pages| matrix::eviction_sampled(
    &OrphanUpdate,
    pages,
    SAMPLE,
    ORPHAN_UPDATE_SEED
));
crate::profile_tests!(cleanup_with_allocation_exhausted, |pages| {
    let recording = matrix::record(&ExhaustedCleanup, pages, Variant::Exhausted);
    matrix::cuts(&ExhaustedCleanup, &recording, pages, Variant::Exhausted, 12);
    matrix::faults(&ExhaustedCleanup, &recording, pages, Variant::Exhausted);
});
crate::profile_tests!(sealing_retained, |pages| matrix::retained(
    &Sealing, pages, 12
));
crate::profile_tests!(consumption_retained, |pages| matrix::retained(
    &Consumption,
    pages,
    12
));

#[test]
fn extent_map_write_survives_reload_read_failures() {
    // Eight pages hold the whole extent-map window, so only two and four
    // pages spill provisional images.
    let reloads: u64 = [2, 4]
        .into_iter()
        .map(|pages| matrix::reload_failures(&ExtentMapWrite, pages, Variant::Plain).1)
        .sum();
    assert!(reloads > 0, "no profile reloaded a provisional image");
}

#[test]
fn object_map_replacement_survives_reload_read_failures() {
    // The measured demand of three nodes spills at two pages only.
    let (spills, reloads) = matrix::reload_failures(&OpenTargetReplace, 2, Variant::Eviction);
    assert!(spills > 0, "the fixture spilled no provisional image");
    assert!(reloads > 0, "the fixture reloaded no provisional image");
}

/// The promotion spills provisional allocation-root images without reloading
/// any of them inside the same transaction, so this fixture qualifies the read
/// failures over the spill writes it does perform.
#[test]
fn allocation_root_promotion_survives_reload_read_failures() {
    let spills: u64 = [2, 4, 8]
        .into_iter()
        .map(|pages| matrix::reload_failures(&AllocationRootPromotion, pages, Variant::Plain).0)
        .sum();
    assert!(spills > 0, "no profile spilled a provisional image");
}
