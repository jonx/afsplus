//! Residual tail families of the tiny-cache matrix: the mid-run reclaim cursor
//! transition, snapshot-registry reload read failures, and the ambiguous
//! commit tail of the namespace, symlink, deep-tree, orphan-cleanup and
//! reload fixtures the uncertain-tail row names. See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{CoreError, Volume};
use afsplus_format::reclaim::{ReclaimCaps, ReclaimRoot};
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

// ---------------------------------------------------------------------------
// Mid-run reclaim cursor transition
// ---------------------------------------------------------------------------

fn accounting<D: BlockDevice>(volume: &Volume<D>) -> (u64, u64) {
    (volume.free_blocks(), volume.reclaim_pending_blocks())
}

/// Consumed blocks inside the head entry of the selected checkpoint's queue.
fn head_block_offset<D: BlockDevice>(volume: &mut Volume<D>) -> u32 {
    let lba = volume.checkpoint().reclaim_root_block;
    let size = volume.device_mut().block_size();
    let mut block = vec![0u8; size];
    volume.device_mut().read_block(lba, &mut block).unwrap();
    ReclaimRoot::decode(&block).unwrap().0.head_block_offset
}

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

const ANCHOR: &[u8] = b"anchor bytes the cursor transition must keep";
const FILLER: &[u8] = b"filler bytes behind the three-block run";

/// Free and pending blocks of the fixture and of the published state.
const CURSOR_ACCOUNTING: [(u64, u64); 2] = [(208, 18), (209, 20)];
/// Consumed blocks inside the head entry, fixture and published state.
const CURSOR_HEAD_BLOCK_OFFSET: [u32; 2] = [0, 2];

struct CursorState {
    anchor: u64,
    snapshot: u64,
}

struct MidRunCursor;

impl Family for MidRunCursor {
    type State = CursorState;

    fn name(&self) -> &'static str {
        "mid-run reclaim cursor"
    }

    fn format(&self, _variant: Variant) -> Format {
        tiny_format()
    }

    /// The snapshot precedes every block the transition reclaims. A
    /// three-block run and one filler commit behind it put the recorded
    /// step's batch of three blocks inside a run.
    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> CursorState {
        volume.set_reclaim_batch_blocks(1);
        let anchor = volume.create_file_in_root("anchor", ANCHOR, ts(1)).unwrap();
        let snapshot = volume.snapshot_create(ts(2)).unwrap();
        volume
            .create_file_in_root("big", &[0xb7u8; 3 * BS], ts(3))
            .unwrap();
        volume.delete_file_in_root("big", ts(4)).unwrap();
        volume.create_file_in_root("filler", FILLER, ts(5)).unwrap();
        CursorState { anchor, snapshot }
    }

    fn snapshot(&self, state: &CursorState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &CursorState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.anchor, ANCHOR.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &CursorState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(3);
        volume.reclaim_step(ts(6)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CursorState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let mut listed: Vec<String> = volume
            .list_root()
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            vec!["anchor".to_string(), "filler".to_string()],
            "{context}: root entries"
        );
        let mut read = vec![0xa5; ANCHOR.len() + 1];
        assert_eq!(
            volume.read_file_at(state.anchor, 0, &mut read).unwrap(),
            ANCHOR.len(),
            "{context}: anchor length"
        );
        assert_eq!(&read[..ANCHOR.len()], ANCHOR, "{context}: anchor bytes");
        assert_eq!(read[ANCHOR.len()], 0xa5, "{context}: anchor EOF");
        assert_eq!(
            accounting(volume),
            CURSOR_ACCOUNTING[delta as usize],
            "{context}: free/pending"
        );
        assert_eq!(
            head_block_offset(volume),
            CURSOR_HEAD_BLOCK_OFFSET[delta as usize],
            "{context}: consumed blocks inside the head entry"
        );
    }

    /// The recorded step leaves the persistent cursor inside a run.
    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &CursorState,
        _variant: Variant,
    ) {
        assert!(
            head_block_offset(volume) > 0,
            "the recorded step must stop inside a run"
        );
    }
}

crate::profile_tests!(mid_run_cursor, |pages| matrix::plain(
    &MidRunCursor,
    pages,
    12
));
crate::profile_tests!(mid_run_cursor_retained, |pages| matrix::retained(
    &MidRunCursor,
    pages,
    12
));
/// Families of [family_matrix_structure.rs] this row re-runs: the staged
/// directory delete and the three baseline symlink publication paths.
#[allow(dead_code, unused_imports)]
mod structure {
    use crate::common::family_matrix::{
        self as matrix, padded_name, ts, Family, Format, Variant, BS,
    };
    use afsplus_block::{BlockDevice, BlockError, MemoryBackend, TraceBackend};
    use afsplus_core::volume::{BatchOp, ObjectMetadata, PreservedMetadata};
    use afsplus_core::{CoreError, Volume};
    use afsplus_format::{Timespec, OBJECT_ROOT};
    /// Volume geometry of the small fixtures.
    const SMALL: u64 = 512;
    /// Volume geometry of the pre-populated eviction fixtures.
    const WIDE: u64 = 1024;
    /// Long root names used to force staged-node eviction.
    const NAME_LENGTH: usize = 240;

    fn small() -> Format {
        Format::new(SMALL, SMALL as u32)
    }

    fn wide() -> Format {
        Format::new(WIDE, WIDE as u32)
    }

    fn structure_format(variant: Variant) -> Format {
        if variant == Variant::Eviction {
            wide()
        } else {
            small()
        }
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

    /// Reads the opaque symlink target with a sentinel past the end.
    fn link_bytes<D: BlockDevice>(volume: &mut Volume<D>, id: u64, expected: &str, context: &str) {
        let mut read = vec![0xa5; expected.len() + 1];
        assert_eq!(
            volume.read_link(id, &mut read).unwrap(),
            expected.len(),
            "{context}: link length"
        );
        assert_eq!(
            &read[..expected.len()],
            expected.as_bytes(),
            "{context}: link target"
        );
        assert_eq!(read[expected.len()], 0xa5, "{context}: link EOF");
    }

    /// Entries the pre-populated eviction fixtures hold in the root directory.
    const POPULATION: usize = 150;
    // ---------------------------------------------------------------------------
    // Staged directory split and collapse
    // ---------------------------------------------------------------------------

    /// Entries pre-created in the staged directory. Seven 250-byte names fill the
    /// single directory leaf to the point where one more entry splits it and
    /// raises a directory root above it.
    const SPLIT_ENTRIES: usize = 7;
    /// Entries of the pre-populated eviction fixture, whose directory holds
    /// several leaves under a root.
    const SPLIT_POPULATION: usize = 120;
    /// Subjects an eviction fixture spreads across distinct directory leaves.
    const SPREAD: usize = 12;
    /// Length of each subject name. Longer than the populated names, so
    /// inserting the subjects splits leaves of the pre-filled directory.
    const SUBJECT_NAME_LENGTH: usize = 250;
    /// Measured resident staged-node demand of the spread directory batches.
    const DIRECTORY_SPLIT_DEMAND: u64 = 16;
    const DIRECTORY_COLLAPSE_DEMAND: u64 = 16;
    /// Measured demand of the single-entry symlink and rename transactions over a
    /// wide root directory; they evict at two pages only.
    const SYMLINK_RENAME_DEMAND: u64 = 3;
    /// Measured demand of the metadata-only transactions: one object-map node,
    /// which is the limit of these two fixtures.
    const METADATA_DEMAND: u64 = 1;

    const SUBJECT_BYTES: &[u8] = b"split subject";

    fn split_entries(variant: Variant) -> usize {
        if variant == Variant::Eviction {
            SPLIT_POPULATION
        } else {
            SPLIT_ENTRIES
        }
    }

    fn split_subjects(variant: Variant) -> usize {
        if variant == Variant::Eviction {
            SPREAD
        } else {
            1
        }
    }

    pub struct DirectoryState {
        directory: u64,
        /// Names already present in the directory, the subjects included when the
        /// fixture creates them.
        present: Vec<String>,
        /// Names the operation adds or removes.
        subjects: Vec<String>,
    }

    /// Subject names, spread through the key space of the populated entries so
    /// that each lands in a different directory leaf and the transaction stages
    /// one node per leaf above the shared path.
    fn subject_names(entries: usize, count: usize) -> Vec<String> {
        (0..count)
            .map(|slot| {
                let index = entries * slot / count + entries / (2 * count).max(2);
                // 's' sorts after the 'p' padding of the populated names, so the
                // subject lands immediately after entry `index` in the key order.
                let mut name = format!("entry-{index:04}-");
                while name.len() < SUBJECT_NAME_LENGTH {
                    name.push('s');
                }
                name
            })
            .collect()
    }

    /// Creates the staged directory and fills it with long names.
    fn staged_directory(
        volume: &mut Volume<MemoryBackend>,
        entries: usize,
        subjects: Vec<String>,
        include_subjects: bool,
    ) -> DirectoryState {
        let directory = volume.create_directory_in_root("staged", ts(1)).unwrap();
        let mut present: Vec<String> = (0..entries)
            .map(|index| padded_name("entry", index, SUBJECT_NAME_LENGTH))
            .collect();
        for chunk in present.clone().chunks(32) {
            let operations: Vec<BatchOp<'_>> = chunk
                .iter()
                .map(|name| BatchOp::CreateFile {
                    parent_id: directory,
                    name,
                    content: b"",
                })
                .collect();
            volume.run_batch(&operations, ts(2)).unwrap();
        }
        if include_subjects {
            let operations: Vec<BatchOp<'_>> = subjects
                .iter()
                .map(|name| BatchOp::CreateFile {
                    parent_id: directory,
                    name,
                    content: SUBJECT_BYTES,
                })
                .collect();
            volume.run_batch(&operations, ts(3)).unwrap();
            present.extend(subjects.iter().cloned());
        }
        DirectoryState {
            directory,
            present,
            subjects,
        }
    }

    fn verify_directory<D: BlockDevice>(
        volume: &mut Volume<D>,
        state: &DirectoryState,
        holds_subject: bool,
        context: &str,
    ) {
        // One enumeration compares every name, so extra, missing and duplicate
        // entries all fail without a lookup per entry.
        let mut listed: Vec<String> = volume
            .list_directory(state.directory)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        listed.sort();
        let mut expected: Vec<String> = state
            .present
            .iter()
            .filter(|name| !state.subjects.contains(name))
            .cloned()
            .collect();
        if holds_subject {
            expected.extend(state.subjects.iter().cloned());
        }
        expected.sort();
        assert_eq!(listed, expected, "{context}: directory entries");
        for subject in &state.subjects {
            let found = volume
                .lookup_in_directory(state.directory, subject)
                .unwrap();
            if holds_subject {
                file_bytes(
                    volume,
                    found.unwrap_or_else(|| panic!("{context}: {subject} is absent")),
                    SUBJECT_BYTES,
                    context,
                );
            } else {
                assert_eq!(found, None, "{context}: {subject} is present");
            }
        }
        assert_eq!(
            volume.list_root().unwrap(),
            vec![("staged".to_string(), state.directory)],
            "{context}: root"
        );
    }

    pub struct DirectorySplit;

    impl Family for DirectorySplit {
        type State = DirectoryState;

        fn name(&self) -> &'static str {
            "staged directory split"
        }

        fn format(&self, _variant: Variant) -> Format {
            wide()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> DirectoryState {
            let entries = split_entries(variant);
            let subjects = subject_names(entries, split_subjects(variant));
            staged_directory(volume, entries, subjects, false)
        }

        fn captured(&self, _state: &DirectoryState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DirectoryState,
        ) -> Result<(), CoreError> {
            let operations: Vec<BatchOp<'_>> = state
                .subjects
                .iter()
                .map(|name| BatchOp::CreateFile {
                    parent_id: state.directory,
                    name,
                    content: SUBJECT_BYTES,
                })
                .collect();
            volume.run_batch(&operations, ts(30)).map(|_| ())
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DirectoryState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            verify_directory(volume, state, delta == 1, context);
        }

        /// The bounded fixture splits the single directory leaf and raises a
        /// directory root above it. The spread fixture rewrites many leaves of an
        /// already deep directory and records no split.
        fn after_success<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &DirectoryState,
            variant: Variant,
        ) {
            let stats = volume.last_commit_stats().unwrap().tree_mutations;
            let expected = u64::from(variant != Variant::Eviction);
            assert_eq!(stats.splits, expected, "{variant:?}: splits");
            assert_eq!(stats.root_splits, expected, "{variant:?}: root splits");
        }

        fn eviction_demand(&self) -> u64 {
            DIRECTORY_SPLIT_DEMAND
        }
    }

    pub struct DirectoryCollapse;

    impl Family for DirectoryCollapse {
        type State = DirectoryState;

        fn name(&self) -> &'static str {
            "staged directory delete"
        }

        fn format(&self, _variant: Variant) -> Format {
            wide()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> DirectoryState {
            let entries = split_entries(variant);
            let subjects = subject_names(entries, split_subjects(variant));
            staged_directory(volume, entries, subjects, true)
        }

        fn captured(&self, _state: &DirectoryState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DirectoryState,
        ) -> Result<(), CoreError> {
            let operations: Vec<BatchOp<'_>> = state
                .subjects
                .iter()
                .map(|name| BatchOp::DeleteFile {
                    parent_id: state.directory,
                    name,
                })
                .collect();
            volume.run_batch(&operations, ts(30)).map(|_| ())
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DirectoryState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            verify_directory(volume, state, delta == 0, context);
        }

        /// Removing the subject merges the split leaf back and collapses the
        /// directory root in the bounded fixture.
        fn after_success<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &DirectoryState,
            variant: Variant,
        ) {
            let stats = volume.last_commit_stats().unwrap().tree_mutations;
            let expected = u64::from(variant != Variant::Eviction);
            assert_eq!(stats.merges, expected, "{variant:?}: merges");
            assert_eq!(
                stats.root_collapses, expected,
                "{variant:?}: root collapses"
            );
        }

        fn eviction_demand(&self) -> u64 {
            DIRECTORY_COLLAPSE_DEMAND
        }
    }

    // ---------------------------------------------------------------------------
    // Symlink publication paths
    // ---------------------------------------------------------------------------

    const LINK_TARGET: &str = "SYS:Tools/Commodities/Exchange";
    const MOVED_TARGET: &str = "WORK:Projects/afsplus/notes.txt";

    pub struct SymlinkState {
        directory: u64,
        link: u64,
        population: usize,
    }

    fn symlink_fixture(
        volume: &mut Volume<MemoryBackend>,
        variant: Variant,
        create_link: bool,
    ) -> SymlinkState {
        let mut population = 0;
        if variant == Variant::Eviction {
            matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
            population = POPULATION;
        }
        let directory = volume.create_directory_in_root("links", ts(1)).unwrap();
        let link = if create_link {
            volume
                .create_symlink(OBJECT_ROOT, "link", LINK_TARGET, ts(2))
                .unwrap()
        } else {
            0
        };
        SymlinkState {
            directory,
            link,
            population,
        }
    }

    pub struct SymlinkCreate;

    impl Family for SymlinkCreate {
        type State = SymlinkState;

        fn name(&self) -> &'static str {
            "symlink create"
        }

        fn format(&self, variant: Variant) -> Format {
            structure_format(variant)
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SymlinkState {
            symlink_fixture(volume, variant, false)
        }

        fn captured(&self, _state: &SymlinkState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &SymlinkState,
        ) -> Result<(), CoreError> {
            volume
                .create_symlink(OBJECT_ROOT, "link", MOVED_TARGET, ts(30))
                .map(|_| ())
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SymlinkState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let published = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                state.population + 1 + usize::from(published),
                "{context}: root entries"
            );
            match volume.lookup_root("link").unwrap() {
                Some(id) if published => link_bytes(volume, id, MOVED_TARGET, context),
                Some(_) => panic!("{context}: link published early"),
                None => assert!(!published, "{context}: link is absent"),
            }
        }

        /// One symlink object record and its root entry above the wide root leaf;
        /// the fixture evicts at two pages only.
        fn eviction_demand(&self) -> u64 {
            3
        }
    }

    pub struct SymlinkRename;

    impl Family for SymlinkRename {
        type State = SymlinkState;

        fn name(&self) -> &'static str {
            "symlink rename"
        }

        fn format(&self, variant: Variant) -> Format {
            structure_format(variant)
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SymlinkState {
            symlink_fixture(volume, variant, true)
        }

        fn captured(&self, _state: &SymlinkState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SymlinkState,
        ) -> Result<(), CoreError> {
            volume.rename(OBJECT_ROOT, "link", state.directory, "moved", ts(30))
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SymlinkState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let moved = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                state.population + 2 - usize::from(moved),
                "{context}: root entries"
            );
            assert_eq!(
                volume.lookup_root("link").unwrap(),
                (!moved).then_some(state.link),
                "{context}: root link"
            );
            assert_eq!(
                volume
                    .lookup_in_directory(state.directory, "moved")
                    .unwrap(),
                moved.then_some(state.link),
                "{context}: moved link"
            );
            link_bytes(volume, state.link, LINK_TARGET, context);
        }

        fn eviction_demand(&self) -> u64 {
            SYMLINK_RENAME_DEMAND
        }
    }

    pub struct SymlinkUnlink;

    impl Family for SymlinkUnlink {
        type State = SymlinkState;

        fn name(&self) -> &'static str {
            "symlink unlink"
        }

        fn format(&self, variant: Variant) -> Format {
            structure_format(variant)
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SymlinkState {
            symlink_fixture(volume, variant, true)
        }

        fn captured(&self, _state: &SymlinkState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &SymlinkState,
        ) -> Result<(), CoreError> {
            volume.unlink_symlink(OBJECT_ROOT, "link", ts(30))
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SymlinkState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let removed = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                state.population + 2 - usize::from(removed),
                "{context}: root entries"
            );
            if removed {
                assert_eq!(volume.lookup_root("link").unwrap(), None, "{context}");
                assert!(volume.stat(state.link).unwrap().is_none(), "{context}");
            } else {
                assert_eq!(
                    volume.lookup_root("link").unwrap(),
                    Some(state.link),
                    "{context}"
                );
                link_bytes(volume, state.link, LINK_TARGET, context);
            }
        }

        fn eviction_demand(&self) -> u64 {
            3
        }
    }
}

/// Families of [family_matrix_structure_residuals.rs] this row re-runs: the
/// deep split, the deep cross-directory rename, the single create, mkdir and
/// rmdir paths and the three symlink paths at the maximum target length.
#[allow(dead_code, unused_imports)]
mod structure_residuals {

    use crate::common::family_matrix::{
        self as matrix, padded_name, ts, Family, Format, Variant, BS,
    };
    use afsplus_block::{
        BlockDevice, FaultBackend, FaultPlan, MemoryBackend, RecordedOp, TraceBackend,
    };
    use afsplus_core::volume::BatchOp;
    use afsplus_core::{CoreError, Volume};
    use afsplus_format::ident::Identification;
    use afsplus_format::object::SymlinkRecord;
    use afsplus_format::OBJECT_ROOT;

    /// Volume geometry of the bounded fixtures.
    const SMALL: u64 = 512;
    /// Volume geometry of the pre-populated eviction fixtures.
    const WIDE: u64 = 1024;
    /// Volume geometry of the deep-directory fixtures.
    const DEEP: u64 = 8192;
    /// Entries the eviction fixtures hold in the root directory.
    const POPULATION: usize = 150;
    /// Long root names used to force staged-node eviction.
    const NAME_LENGTH: usize = 240;
    /// Subsets drawn per flush segment longer than the exhaustive budget.
    const SAMPLE: usize = 32;

    fn small() -> Format {
        Format::new(SMALL, SMALL as u32)
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

    /// Reads the opaque symlink target with a sentinel past the end.
    fn link_bytes<D: BlockDevice>(volume: &mut Volume<D>, id: u64, expected: &str, context: &str) {
        let mut read = vec![0xa5; expected.len() + 1];
        assert_eq!(
            volume.read_link(id, &mut read).unwrap(),
            expected.len(),
            "{context}: link length"
        );
        assert_eq!(
            &read[..expected.len()],
            expected.as_bytes(),
            "{context}: link target"
        );
        assert_eq!(read[expected.len()], 0xa5, "{context}: link EOF");
    }
    // ---------------------------------------------------------------------------
    // Leaf split inside a directory with more than one interior level
    // ---------------------------------------------------------------------------

    /// Entries of the deep directory. With 250-byte names a leaf holds seven
    /// entries, so this population raises two interior levels above the leaves.
    const DEEP_ENTRIES: usize = 600;
    /// Length of every name in the deep fixtures.
    const DEEP_NAME_LENGTH: usize = 250;
    /// Subjects the split batch inserts. They share one adjacent key range, so
    /// they all land in the same full leaf and split it.
    const DEEP_SUBJECTS: usize = 8;
    /// Entry the subjects sort immediately after.
    const DEEP_ANCHOR_INDEX: usize = 300;
    /// Measured descent depth of the deep directory: a root, two interior levels
    /// and the leaves.
    const DEEP_DEPTH: u8 = 4;
    /// Measured leaf splits of the deep split transaction.
    const DEEP_SPLITS: u64 = 2;

    const DEEP_BYTES: &[u8] = b"deep split subject";

    fn deep_name(index: usize) -> String {
        padded_name("entry", index, DEEP_NAME_LENGTH)
    }

    /// Subject names sorting between entry `DEEP_ANCHOR_INDEX` and its successor.
    fn deep_subjects() -> Vec<String> {
        (0..DEEP_SUBJECTS)
            .map(|slot| {
                let mut name = format!("entry-{DEEP_ANCHOR_INDEX:04}-s{slot:02}-");
                while name.len() < DEEP_NAME_LENGTH {
                    name.push('s');
                }
                name
            })
            .collect()
    }

    fn fill_deep(volume: &mut Volume<MemoryBackend>, directory: u64) -> Vec<String> {
        let names: Vec<String> = (0..DEEP_ENTRIES).map(deep_name).collect();
        for chunk in names.chunks(64) {
            let operations: Vec<BatchOp<'_>> = chunk
                .iter()
                .map(|name| BatchOp::CreateFile {
                    parent_id: directory,
                    name,
                    content: b"",
                })
                .collect();
            volume.run_batch(&operations, ts(2)).unwrap();
        }
        names
    }

    pub struct DeepState {
        directory: u64,
        present: Vec<String>,
        subjects: Vec<String>,
    }

    fn verify_deep<D: BlockDevice>(
        volume: &mut Volume<D>,
        state: &DeepState,
        holds_subjects: bool,
        context: &str,
    ) {
        let mut listed: Vec<String> = volume
            .list_directory(state.directory)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        listed.sort();
        let mut expected = state.present.clone();
        if holds_subjects {
            expected.extend(state.subjects.iter().cloned());
        }
        expected.sort();
        assert_eq!(listed, expected, "{context}: directory entries");
        for subject in &state.subjects {
            let found = volume
                .lookup_in_directory(state.directory, subject)
                .unwrap();
            if holds_subjects {
                file_bytes(
                    volume,
                    found.unwrap_or_else(|| panic!("{context}: {subject} is absent")),
                    DEEP_BYTES,
                    context,
                );
            } else {
                assert_eq!(found, None, "{context}: {subject} is present");
            }
        }
        assert_eq!(
            volume.list_root().unwrap(),
            vec![("deep".to_string(), state.directory)],
            "{context}: root"
        );
    }

    pub struct DeepSplit;

    impl Family for DeepSplit {
        type State = DeepState;

        fn name(&self) -> &'static str {
            "split of a deep directory"
        }

        fn format(&self, _variant: Variant) -> Format {
            Format::new(DEEP, DEEP as u32)
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> DeepState {
            let directory = volume.create_directory_in_root("deep", ts(1)).unwrap();
            let present = fill_deep(volume, directory);
            DeepState {
                directory,
                present,
                subjects: deep_subjects(),
            }
        }

        fn captured(&self, _state: &DeepState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DeepState,
        ) -> Result<(), CoreError> {
            let operations: Vec<BatchOp<'_>> = state
                .subjects
                .iter()
                .map(|name| BatchOp::CreateFile {
                    parent_id: state.directory,
                    name,
                    content: DEEP_BYTES,
                })
                .collect();
            volume.run_batch(&operations, ts(30)).map(|_| ())
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DeepState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            verify_deep(volume, state, delta == 1, context);
        }

        /// The subjects fill one leaf of a directory whose descent passes two
        /// interior levels, so the transaction splits below an interior node and
        /// raises no new directory root.
        fn after_success<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &DeepState,
            _variant: Variant,
        ) {
            let stats = volume.last_commit_stats().unwrap().tree_mutations;
            assert_eq!(stats.max_depth, DEEP_DEPTH, "deep split: descent depth");
            assert_eq!(stats.splits, DEEP_SPLITS, "deep split: splits");
            assert_eq!(stats.root_splits, 0, "deep split: root splits");
        }
    }

    // ---------------------------------------------------------------------------
    // Cross-directory rename between two deep directories
    // ---------------------------------------------------------------------------

    const DEEP_MOVED_BYTES: &[u8] = b"moved between two deep directories";

    pub struct DeepRenameState {
        source: u64,
        target: u64,
        source_present: Vec<String>,
        target_present: Vec<String>,
        moved: String,
        file: u64,
    }

    pub struct DeepCrossRename;

    impl DeepCrossRename {
        /// The moved name sorts into an adjacent key range of both directories.
        fn moved_name() -> String {
            let mut name = format!("entry-{DEEP_ANCHOR_INDEX:04}-m-");
            while name.len() < DEEP_NAME_LENGTH {
                name.push('m');
            }
            name
        }
    }

    impl Family for DeepCrossRename {
        type State = DeepRenameState;

        fn name(&self) -> &'static str {
            "rename between two deep directories"
        }

        fn format(&self, _variant: Variant) -> Format {
            Format::new(DEEP, DEEP as u32)
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> DeepRenameState {
            let source = volume.create_directory_in_root("source", ts(1)).unwrap();
            let target = volume.create_directory_in_root("target", ts(2)).unwrap();
            let mut source_present = fill_deep(volume, source);
            let target_present = fill_deep(volume, target);
            let moved = Self::moved_name();
            let file = volume
                .create_file_in_directory(source, &moved, DEEP_MOVED_BYTES, ts(4))
                .unwrap();
            source_present.push(moved.clone());
            DeepRenameState {
                source,
                target,
                source_present,
                target_present,
                moved,
                file,
            }
        }

        fn captured(&self, state: &DeepRenameState) -> Vec<(u64, Vec<u8>)> {
            vec![(state.file, DEEP_MOVED_BYTES.to_vec())]
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DeepRenameState,
        ) -> Result<(), CoreError> {
            volume.rename(
                state.source,
                &state.moved,
                state.target,
                &state.moved,
                ts(30),
            )
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &DeepRenameState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let moved = delta == 1;
            for (directory, present, holds) in [
                (state.source, &state.source_present, !moved),
                (state.target, &state.target_present, moved),
            ] {
                let mut listed: Vec<String> = volume
                    .list_directory(directory)
                    .unwrap()
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect();
                listed.sort();
                let mut expected: Vec<String> = present
                    .iter()
                    .filter(|name| *name != &state.moved)
                    .cloned()
                    .collect();
                if holds {
                    expected.push(state.moved.clone());
                }
                expected.sort();
                assert_eq!(listed, expected, "{context}: entries of {directory}");
            }
            let (holder, empty) = if moved {
                (state.target, state.source)
            } else {
                (state.source, state.target)
            };
            assert_eq!(
                volume.lookup_in_directory(holder, &state.moved).unwrap(),
                Some(state.file),
                "{context}: renamed entry"
            );
            assert_eq!(
                volume.lookup_in_directory(empty, &state.moved).unwrap(),
                None,
                "{context}: stale entry"
            );
            file_bytes(volume, state.file, DEEP_MOVED_BYTES, context);
            assert_eq!(
                volume.stat(state.file).unwrap().unwrap().link_count,
                1,
                "{context}: link count"
            );
        }

        /// Both directories carry two interior levels above their leaves.
        fn after_success<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &DeepRenameState,
            _variant: Variant,
        ) {
            let stats = volume.last_commit_stats().unwrap().tree_mutations;
            assert_eq!(stats.max_depth, DEEP_DEPTH, "deep rename: descent depth");
        }
    }

    // ---------------------------------------------------------------------------
    // Single create, mkdir, rmdir and hard link
    // ---------------------------------------------------------------------------

    const SINGLE_BYTES: &[u8] = b"single create payload";
    const OCCUPANT: &[u8] = b"occupant bytes the refused path must keep";

    pub struct SingleState {
        /// Object occupying the operation's name in the refusal fixture.
        occupant: Option<u64>,
        /// Subject the operation acts on, when the fixture creates one.
        subject: u64,
        relieved: bool,
    }

    /// Creates the blocking occupant of the refusal fixture.
    fn occupy(volume: &mut Volume<MemoryBackend>, variant: Variant, name: &str) -> Option<u64> {
        (variant == Variant::Refusal)
            .then(|| volume.create_file_in_root(name, OCCUPANT, ts(4)).unwrap())
    }

    /// Removes the occupant so the refused operation can be retried.
    fn free_name<D: BlockDevice>(
        volume: &mut Volume<D>,
        state: &mut SingleState,
        name: &str,
    ) -> u64 {
        volume.delete_file_in_root(name, ts(40)).unwrap();
        state.occupant = None;
        state.relieved = true;
        1
    }

    pub struct SingleCreate;

    impl Family for SingleCreate {
        type State = SingleState;

        fn name(&self) -> &'static str {
            "single file create"
        }

        fn format(&self, _variant: Variant) -> Format {
            small()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
            SingleState {
                occupant: occupy(volume, variant, "subject"),
                subject: 0,
                relieved: false,
            }
        }

        fn captured(&self, _state: &SingleState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &SingleState,
        ) -> Result<(), CoreError> {
            volume
                .create_file_in_root("subject", SINGLE_BYTES, ts(30))
                .map(|_| ())
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SingleState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let published = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                usize::from(published || state.occupant.is_some()),
                "{context}: root entries"
            );
            match volume.lookup_root("subject").unwrap() {
                None => assert!(
                    !published && state.occupant.is_none(),
                    "{context}: subject is absent"
                ),
                Some(id) if published => file_bytes(volume, id, SINGLE_BYTES, context),
                Some(id) => {
                    assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                    file_bytes(volume, id, OCCUPANT, context);
                }
            }
        }

        fn is_refusal(&self, error: &CoreError) -> bool {
            matches!(error, CoreError::AlreadyExists)
        }

        fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
            free_name(volume, state, "subject")
        }
    }

    pub struct Mkdir;

    impl Family for Mkdir {
        type State = SingleState;

        fn name(&self) -> &'static str {
            "mkdir"
        }

        fn format(&self, _variant: Variant) -> Format {
            small()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
            SingleState {
                occupant: occupy(volume, variant, "made"),
                subject: 0,
                relieved: false,
            }
        }

        fn captured(&self, _state: &SingleState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &SingleState,
        ) -> Result<(), CoreError> {
            volume.create_directory_in_root("made", ts(30)).map(|_| ())
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SingleState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let published = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                usize::from(published || state.occupant.is_some()),
                "{context}: root entries"
            );
            match volume.lookup_root("made").unwrap() {
                None => assert!(
                    !published && state.occupant.is_none(),
                    "{context}: directory is absent"
                ),
                Some(id) if published => {
                    assert!(
                        volume.list_directory(id).unwrap().is_empty(),
                        "{context}: the new directory must be empty"
                    );
                }
                Some(id) => {
                    assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                    file_bytes(volume, id, OCCUPANT, context);
                }
            }
        }

        fn is_refusal(&self, error: &CoreError) -> bool {
            matches!(error, CoreError::AlreadyExists)
        }

        fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
            free_name(volume, state, "made")
        }
    }

    pub struct Rmdir;

    impl Family for Rmdir {
        type State = SingleState;

        fn name(&self) -> &'static str {
            "rmdir"
        }

        fn format(&self, _variant: Variant) -> Format {
            small()
        }

        /// The refusal fixture leaves one entry inside the directory, so the
        /// removal is refused until the corrective step deletes it.
        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
            let subject = volume.create_directory_in_root("doomed", ts(2)).unwrap();
            if variant == Variant::Refusal {
                volume
                    .create_file_in_directory(subject, "resident", OCCUPANT, ts(3))
                    .unwrap();
            }
            SingleState {
                occupant: None,
                subject,
                relieved: variant != Variant::Refusal,
            }
        }

        fn captured(&self, _state: &SingleState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &SingleState,
        ) -> Result<(), CoreError> {
            volume.remove_directory(OBJECT_ROOT, "doomed", ts(30))
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SingleState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let published = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                usize::from(!published),
                "{context}: root entries"
            );
            if published {
                assert_eq!(volume.lookup_root("doomed").unwrap(), None, "{context}");
                assert!(
                    volume.stat(state.subject).unwrap().is_none(),
                    "{context}: the directory object survived"
                );
            } else {
                assert_eq!(
                    volume.lookup_root("doomed").unwrap(),
                    Some(state.subject),
                    "{context}"
                );
                let listed = volume.list_directory(state.subject).unwrap();
                let expected = if state.relieved {
                    Vec::new()
                } else {
                    vec!["resident".to_string()]
                };
                assert_eq!(
                    listed.into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
                    expected,
                    "{context}: directory entries"
                );
            }
        }

        fn is_refusal(&self, error: &CoreError) -> bool {
            matches!(error, CoreError::DirectoryNotEmpty)
        }

        fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
            volume
                .delete_file(state.subject, "resident", ts(40))
                .unwrap();
            state.relieved = true;
            1
        }
    }

    pub struct HardLink;

    impl Family for HardLink {
        type State = SingleState;

        fn name(&self) -> &'static str {
            "hard link"
        }

        fn format(&self, _variant: Variant) -> Format {
            small()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> SingleState {
            let subject = volume
                .create_file_in_root("original", SINGLE_BYTES, ts(2))
                .unwrap();
            SingleState {
                occupant: occupy(volume, variant, "alias"),
                subject,
                relieved: false,
            }
        }

        fn captured(&self, state: &SingleState) -> Vec<(u64, Vec<u8>)> {
            vec![(state.subject, SINGLE_BYTES.to_vec())]
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SingleState,
        ) -> Result<(), CoreError> {
            volume.link_file(state.subject, OBJECT_ROOT, "alias", ts(30))
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &SingleState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let published = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                1 + usize::from(published || state.occupant.is_some()),
                "{context}: root entries"
            );
            assert_eq!(
                volume.lookup_root("original").unwrap(),
                Some(state.subject),
                "{context}: original entry"
            );
            file_bytes(volume, state.subject, SINGLE_BYTES, context);
            let alias = volume.lookup_root("alias").unwrap();
            let expected_links = if published { 2 } else { 1 };
            assert_eq!(
                volume.stat(state.subject).unwrap().unwrap().link_count,
                expected_links,
                "{context}: link count"
            );
            match alias {
                None => assert!(
                    !published && state.occupant.is_none(),
                    "{context}: alias is absent"
                ),
                Some(id) if published => {
                    assert_eq!(id, state.subject, "{context}: alias identity");
                    file_bytes(volume, id, SINGLE_BYTES, context);
                }
                Some(id) => {
                    assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                    file_bytes(volume, id, OCCUPANT, context);
                }
            }
        }

        fn is_refusal(&self, error: &CoreError) -> bool {
            matches!(error, CoreError::AlreadyExists)
        }

        fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut SingleState) -> u64 {
            free_name(volume, state, "alias")
        }
    }

    // ---------------------------------------------------------------------------
    // Symlink targets at the maximum representable length
    // ---------------------------------------------------------------------------

    /// Longest target an inline symlink record can represent at this block size.
    fn max_target() -> String {
        let limit = SymlinkRecord::maximum_target_bytes(BS);
        let mut target = "WORK:afsplus/maximum/".to_string();
        while target.len() < limit {
            target.push('t');
        }
        assert_eq!(target.len(), limit);
        target
    }

    /// The second target, also at the maximum length and distinct from the first.
    fn other_max_target() -> String {
        let mut target = max_target();
        target.replace_range(0..1, "S");
        target
    }

    pub struct MaxLinkState {
        directory: u64,
        link: u64,
        occupant: Option<u64>,
    }

    fn max_link_fixture(
        volume: &mut Volume<MemoryBackend>,
        variant: Variant,
        create_link: bool,
    ) -> MaxLinkState {
        let directory = volume.create_directory_in_root("links", ts(1)).unwrap();
        let link = if create_link {
            volume
                .create_symlink(OBJECT_ROOT, "link", &max_target(), ts(2))
                .unwrap()
        } else {
            0
        };
        let occupant = (variant == Variant::Refusal)
            .then(|| volume.create_file_in_root("link", OCCUPANT, ts(3)).unwrap());
        MaxLinkState {
            directory,
            link,
            occupant,
        }
    }

    pub struct MaxSymlinkCreate;

    impl Family for MaxSymlinkCreate {
        type State = MaxLinkState;

        fn name(&self) -> &'static str {
            "symlink create at the maximum target length"
        }

        fn format(&self, _variant: Variant) -> Format {
            small()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> MaxLinkState {
            max_link_fixture(volume, variant, false)
        }

        fn captured(&self, _state: &MaxLinkState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &MaxLinkState,
        ) -> Result<(), CoreError> {
            volume
                .create_symlink(OBJECT_ROOT, "link", &other_max_target(), ts(30))
                .map(|_| ())
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &MaxLinkState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let published = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                1 + usize::from(published || state.occupant.is_some()),
                "{context}: root entries"
            );
            match volume.lookup_root("link").unwrap() {
                None => assert!(
                    !published && state.occupant.is_none(),
                    "{context}: link is absent"
                ),
                Some(id) if published => link_bytes(volume, id, &other_max_target(), context),
                Some(id) => {
                    assert_eq!(Some(id), state.occupant, "{context}: occupant identity");
                    file_bytes(volume, id, OCCUPANT, context);
                }
            }
        }

        fn is_refusal(&self, error: &CoreError) -> bool {
            matches!(error, CoreError::AlreadyExists)
        }

        fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut MaxLinkState) -> u64 {
            volume.delete_file_in_root("link", ts(40)).unwrap();
            state.occupant = None;
            1
        }
    }

    pub struct MaxSymlinkRename;

    impl Family for MaxSymlinkRename {
        type State = MaxLinkState;

        fn name(&self) -> &'static str {
            "symlink rename at the maximum target length"
        }

        fn format(&self, _variant: Variant) -> Format {
            small()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> MaxLinkState {
            max_link_fixture(volume, Variant::Plain, true)
        }

        fn captured(&self, _state: &MaxLinkState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &MaxLinkState,
        ) -> Result<(), CoreError> {
            volume.rename(OBJECT_ROOT, "link", state.directory, "moved", ts(30))
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &MaxLinkState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let moved = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                2 - usize::from(moved),
                "{context}: root entries"
            );
            assert_eq!(
                volume.lookup_root("link").unwrap(),
                (!moved).then_some(state.link),
                "{context}: root link"
            );
            assert_eq!(
                volume
                    .lookup_in_directory(state.directory, "moved")
                    .unwrap(),
                moved.then_some(state.link),
                "{context}: moved link"
            );
            link_bytes(volume, state.link, &max_target(), context);
        }
    }

    pub struct MaxSymlinkUnlink;

    impl Family for MaxSymlinkUnlink {
        type State = MaxLinkState;

        fn name(&self) -> &'static str {
            "symlink unlink at the maximum target length"
        }

        fn format(&self, _variant: Variant) -> Format {
            small()
        }

        fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> MaxLinkState {
            max_link_fixture(volume, Variant::Plain, true)
        }

        fn captured(&self, _state: &MaxLinkState) -> Vec<(u64, Vec<u8>)> {
            Vec::new()
        }

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &MaxLinkState,
        ) -> Result<(), CoreError> {
            volume.unlink_symlink(OBJECT_ROOT, "link", ts(30))
        }

        fn verify<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            state: &MaxLinkState,
            _variant: Variant,
            delta: u64,
            context: &str,
        ) {
            let removed = delta == 1;
            assert_eq!(
                volume.list_root().unwrap().len(),
                2 - usize::from(removed),
                "{context}: root entries"
            );
            if removed {
                assert_eq!(volume.lookup_root("link").unwrap(), None, "{context}");
                assert!(volume.stat(state.link).unwrap().is_none(), "{context}");
            } else {
                assert_eq!(
                    volume.lookup_root("link").unwrap(),
                    Some(state.link),
                    "{context}"
                );
                link_bytes(volume, state.link, &max_target(), context);
            }
        }
    }

    /// A target one byte above the representable maximum must be refused without
    /// any write or flush, and the maximum-length target must then publish.
    fn over_long_symlink_target_refuses(pages: usize) {
        let context = format!("over-long symlink target pages={pages}");
        let mut volume = matrix::open(TraceBackend::new(small().device()), pages);
        let generation = volume.generation();
        volume.device_mut().reset();
        let mut target = max_target();
        target.push('t');
        assert_eq!(target.len(), SymlinkRecord::maximum_target_bytes(BS) + 1);
        assert!(
            matches!(
                volume.create_symlink(OBJECT_ROOT, "link", &target, ts(30)),
                Err(CoreError::InvalidMetadata(_))
            ),
            "{context}: an over-long target must be refused"
        );
        let stats = volume.device_mut().stats();
        assert_eq!(
            (stats.writes, stats.flushes),
            (0, 0),
            "{context}: refusal I/O"
        );
        assert_eq!(volume.generation(), generation, "{context}");
        assert!(volume.list_root().unwrap().is_empty(), "{context}");
        let id = volume
            .create_symlink(OBJECT_ROOT, "link", &max_target(), ts(31))
            .unwrap_or_else(|error| panic!("{context}: retry {error}"));
        link_bytes(&mut volume, id, &max_target(), &context);
        let mut remounted = matrix::open(volume.into_device().into_inner(), pages);
        link_bytes(&mut remounted, id, &max_target(), &context);
        matrix::assert_checker_clean(remounted.device_mut(), &context);
        eprintln!("{context}: refused writes=0 flushes=0 retry=1");
    }
}

/// Families of [family_matrix_reclaim_residuals.rs] this row re-runs: orphan
/// cleanup with ordinary allocation exhausted and the three reload-failure
/// fixtures.
#[allow(dead_code, unused_imports)]
mod reclaim_residuals {
    use crate::common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_core::{object_map, CoreError, Volume};
    use afsplus_format::reclaim::ReclaimCaps;
    use afsplus_format::{OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};

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

    pub struct OpenTargetState {
        target: u64,
        source: u64,
        populated: usize,
    }

    pub struct OpenTargetReplace;

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
            volume.rename_replace_orphan_target(
                OBJECT_ROOT,
                "source",
                OBJECT_ROOT,
                "target",
                ts(30),
            )
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
    // Orphan cleanup with ordinary allocation exhausted
    // ---------------------------------------------------------------------------

    fn fragmented_bytes() -> Vec<u8> {
        let mut bytes = vec![0; 3 * BS];
        bytes[..BS].fill(0x41);
        bytes[2 * BS..].fill(0x42);
        bytes
    }

    pub struct CleanupState {
        object: u64,
        pressure: bool,
    }

    pub struct ExhaustedCleanup;

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
    // Reload read failures outside the directory and batch transactions
    // ---------------------------------------------------------------------------

    /// Logical blocks the fragmented extent-map fixture writes.
    const EXTENT_FIXTURE_BLOCKS: u64 = 160;
    /// Logical blocks the recorded extent-map write covers.
    const EXTENT_WRITE_BLOCKS: u64 = 48;

    pub struct ExtentState {
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

    pub struct ExtentMapWrite;

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
    pub struct AllocationRootPromotion;

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

        fn apply<D: BlockDevice>(
            &self,
            volume: &mut Volume<D>,
            _state: &(),
        ) -> Result<(), CoreError> {
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
}

// ---------------------------------------------------------------------------
// The common uncertain commit tail of the remaining namespace families
//
// Each test runs the driver's ambiguous-publication matrix over one family:
// a completed checkpoint write that reports an error, and reads that fail
// after publication. The operation reports the error, the same operation and
// an independent create return `WindowPoisoned` with no further write or
// flush, the remount exposes exactly one publication and a two-checkpoint
// operation retries to its final state.
// ---------------------------------------------------------------------------

crate::profile_tests!(directory_delete_ambiguous, |pages| matrix::ambiguous(
    &structure::DirectoryCollapse,
    pages,
    Variant::Plain
));
crate::profile_tests!(symlink_create_ambiguous, |pages| matrix::ambiguous(
    &structure::SymlinkCreate,
    pages,
    Variant::Plain
));
crate::profile_tests!(symlink_rename_ambiguous, |pages| matrix::ambiguous(
    &structure::SymlinkRename,
    pages,
    Variant::Plain
));
crate::profile_tests!(symlink_unlink_ambiguous, |pages| matrix::ambiguous(
    &structure::SymlinkUnlink,
    pages,
    Variant::Plain
));
crate::profile_tests!(deep_split_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::DeepSplit,
    pages,
    Variant::Plain
));
crate::profile_tests!(deep_cross_rename_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::DeepCrossRename,
    pages,
    Variant::Plain
));
crate::profile_tests!(single_create_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::SingleCreate,
    pages,
    Variant::Plain
));
crate::profile_tests!(mkdir_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::Mkdir,
    pages,
    Variant::Plain
));
crate::profile_tests!(rmdir_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::Rmdir,
    pages,
    Variant::Plain
));
crate::profile_tests!(max_symlink_create_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::MaxSymlinkCreate,
    pages,
    Variant::Plain
));
crate::profile_tests!(max_symlink_rename_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::MaxSymlinkRename,
    pages,
    Variant::Plain
));
crate::profile_tests!(max_symlink_unlink_ambiguous, |pages| matrix::ambiguous(
    &structure_residuals::MaxSymlinkUnlink,
    pages,
    Variant::Plain
));
crate::profile_tests!(exhausted_cleanup_ambiguous, |pages| matrix::ambiguous(
    &reclaim_residuals::ExhaustedCleanup,
    pages,
    Variant::Exhausted
));
crate::profile_tests!(extent_map_write_ambiguous, |pages| matrix::ambiguous(
    &reclaim_residuals::ExtentMapWrite,
    pages,
    Variant::Plain
));
crate::profile_tests!(object_map_replacement_ambiguous, |pages| matrix::ambiguous(
    &reclaim_residuals::OpenTargetReplace,
    pages,
    Variant::Eviction
));
crate::profile_tests!(allocation_root_promotion_ambiguous, |pages| {
    matrix::ambiguous(
        &reclaim_residuals::AllocationRootPromotion,
        pages,
        Variant::Plain,
    )
});

// ---------------------------------------------------------------------------
// Reload read failures in the snapshot registry
//
// A registry holding many retained views spans several staged nodes, so a
// deletion inside it spills provisional images at the bounded profiles and
// reloads them again. `reload_failures` fails one read per injection point,
// and every refused attempt keeps the exact registry membership, the subject
// bytes and the captured bytes of every retained view, then retries.
// ---------------------------------------------------------------------------

/// Retained views the registry fixture holds before the recorded deletion.
const REGISTRY_VIEWS: u64 = 128;
const REGISTRY_SUBJECT: &[u8] = b"registry subject bytes that every view captures";

struct RegistryState {
    subject: u64,
    /// Every registered identity of the fixture, ascending.
    views: Vec<u64>,
    /// Identity the recorded deletion removes.
    removed: u64,
}

/// Every registered identity of the selected checkpoint, ascending.
fn registry<D: BlockDevice>(volume: &mut Volume<D>) -> Vec<u64> {
    let mut identities = Vec::new();
    let mut low = 0;
    loop {
        let page = volume.snapshot_list(low, 32).unwrap();
        identities.extend(page.entries.into_iter().map(|entry| entry.id));
        match page.next_id {
            Some(next) => low = next,
            None => break,
        }
    }
    identities
}

struct RegistryReload;

impl Family for RegistryReload {
    type State = RegistryState;

    fn name(&self) -> &'static str {
        "snapshot registry reload"
    }

    fn format(&self, _variant: Variant) -> Format {
        Format::new(2048, 256)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> RegistryState {
        let subject = volume
            .create_file_in_root("subject", REGISTRY_SUBJECT, ts(1))
            .unwrap();
        let views: Vec<u64> = (0..REGISTRY_VIEWS)
            .map(|index| volume.snapshot_create(ts(2 + index as i64)).unwrap())
            .collect();
        let removed = views[views.len() / 2];
        RegistryState {
            subject,
            views,
            removed,
        }
    }

    fn snapshot(&self, state: &RegistryState) -> Option<u64> {
        state.views.first().copied()
    }

    fn captured(&self, state: &RegistryState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.subject, REGISTRY_SUBJECT.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RegistryState,
    ) -> Result<(), CoreError> {
        volume.snapshot_delete(state.removed, ts(500))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &RegistryState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let expected: Vec<u64> = state
            .views
            .iter()
            .copied()
            .filter(|id| delta == 0 || *id != state.removed)
            .collect();
        assert_eq!(registry(volume), expected, "{context}: registry membership");
        let mut read = vec![0xa5; REGISTRY_SUBJECT.len() + 1];
        assert_eq!(
            volume.read_file_at(state.subject, 0, &mut read).unwrap(),
            REGISTRY_SUBJECT.len(),
            "{context}: subject length"
        );
        assert_eq!(
            &read[..REGISTRY_SUBJECT.len()],
            REGISTRY_SUBJECT,
            "{context}: subject bytes"
        );
        assert_eq!(read[REGISTRY_SUBJECT.len()], 0xa5, "{context}: subject EOF");
        assert_eq!(
            volume.list_root().unwrap().len(),
            1,
            "{context}: root entries"
        );
        // The removed view is unreachable exactly once the delete publishes.
        assert_eq!(
            matches!(
                volume.snapshot_open(state.removed),
                Err(CoreError::NotFound)
            ),
            delta == 1,
            "{context}: removed view reachability"
        );
    }
}

/// Measured resident staged-node demand of the largest registry mutation the
/// API admits: a deletion inside a registry at the view admission limit.
const REGISTRY_DEMAND: u64 = 3;

/// The registry holds exactly `REGISTRY_VIEWS` identities and refuses one
/// more, so the recorded deletion is the largest registry mutation the API
/// admits, and its literal demand is the limit of this family.
#[test]
fn the_registry_admission_limit_bounds_the_staged_demand() {
    let mut volume = matrix::open(
        RegistryReload.format(Variant::Retained).device(),
        usize::MAX,
    );
    let state = RegistryReload.setup(&mut volume, Variant::Retained);
    assert_eq!(state.views.len() as u64, REGISTRY_VIEWS);
    assert!(
        matches!(
            volume.snapshot_create(ts(400)),
            Err(CoreError::PrototypeLimit(_))
        ),
        "the registry must refuse an identity above the admission limit"
    );
    RegistryReload.apply(&mut volume, &state).unwrap();
    let stats = volume.last_commit_stats().unwrap().tree_mutations;
    assert_eq!(
        (stats.max_resident_staged_nodes, stats.staged_spill_writes),
        (REGISTRY_DEMAND, 0),
        "unlimited demand of the registry deletion"
    );
    for pages in [4usize, 8] {
        let recording = matrix::record(&RegistryReload, pages, Variant::Retained);
        assert_eq!(
            (recording.peak, recording.spills),
            (REGISTRY_DEMAND, 0),
            "pages={pages}: a profile at or above the demand evicts nothing"
        );
    }
    let recording = matrix::record(&RegistryReload, 2, Variant::Retained);
    assert_eq!(
        (recording.peak, recording.spills),
        (2, 1),
        "two pages must spill below the demand"
    );
}

/// Two pages is the one profile below the registry demand. Sixteen reads
/// spread through the spilling deletion fail one at a time; each refused
/// attempt keeps an allowed committed state with the exact registry
/// membership and the captured bytes, passes the checker, and retries to the
/// complete new state. The transaction reads no provisional image back, so
/// this fixture qualifies the read failures over the spill writes it performs.
#[test]
fn registry_deletion_survives_reload_read_failures() {
    let (spills, reloads) = matrix::reload_failures(&RegistryReload, 2, Variant::Retained);
    assert_eq!(
        (spills, reloads),
        (1, 0),
        "two pages spill one provisional registry image and reload none"
    );
}
