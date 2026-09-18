//! Batch and directory-structure families through the family-matrix driver;
//! see tiny_cache_matrix.md. The batch families cover create, delete,
//! cancellation and payload tails; the structure families cover staged
//! directory splits, cross-directory rename, root split and collapse,
//! symlink publication paths and preserved metadata.

mod common;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend, TraceBackend};
use afsplus_core::volume::{BatchOp, ObjectMetadata, PreservedMetadata};
use afsplus_core::{CoreError, Volume};
use afsplus_format::{Timespec, OBJECT_ROOT};
use common::family_matrix::{self as matrix, padded_name, ts, Family, Format, Variant, BS};

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

// ---------------------------------------------------------------------------
// Batch create with payload tails
// ---------------------------------------------------------------------------

/// Payload lengths of the batch entries: empty, one byte, an exact block and
/// a two-block run seven bytes short of the last block boundary. The tails
/// prove zero-padding of borrowed caller payloads.
const TAILS: [usize; 4] = [0, 1, BS, 2 * BS - 7];

/// Distinct byte and a distinct tail length per batch slot.
fn payload(slot: usize) -> Vec<u8> {
    vec![0x40 + slot as u8; TAILS[slot % TAILS.len()]]
}

/// Entries the pre-populated eviction fixtures hold in the root directory.
const POPULATION: usize = 150;
/// Measured resident staged-node demand of the spread batch transactions.
const BATCH_CREATE_DEMAND: u64 = 17;
const BATCH_DELETE_DEMAND: u64 = 16;

/// Batch entry names. A bounded fixture uses five adjacent names; an eviction
/// fixture spreads twelve names through the populated key space so that the
/// transaction stages one root-directory leaf per name.
fn batch_names(variant: Variant, suffix: &str) -> Vec<String> {
    if variant == Variant::Eviction {
        (0..SPREAD)
            .map(|slot| {
                let index = POPULATION * slot / SPREAD;
                format!("{}~{suffix}", padded_name("wide", index, NAME_LENGTH))
            })
            .collect()
    } else {
        (0..TAILS.len())
            .map(|slot| format!("{suffix}-{slot:02}"))
            .collect()
    }
}

/// Bytes the blocker holds in the refusal fixture before the corrective step.
const BLOCKER: &[u8] = b"blocker bytes that the refused batch must not overwrite";

/// Bytes of the anchor file every batch fixture keeps, so a retained snapshot
/// has a literal object to capture.
const ANCHOR: &[u8] = b"anchor bytes the batch must leave alone";

struct BatchCreateState {
    names: Vec<String>,
    anchor: u64,
    /// Entries the fixture itself contributes to the root directory.
    fixture_entries: usize,
    /// Whether the corrective step of the refusal fixture has run.
    relieved: bool,
}

struct BatchCreate;

impl Family for BatchCreate {
    type State = BatchCreateState;

    fn name(&self) -> &'static str {
        "batch create with payload tails"
    }

    fn format(&self, variant: Variant) -> Format {
        structure_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> BatchCreateState {
        let names = batch_names(variant, "batch");
        let mut fixture_entries = 0;
        if variant == Variant::Eviction {
            matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
            fixture_entries += POPULATION;
        }
        let anchor = volume.create_file_in_root("anchor", ANCHOR, ts(5)).unwrap();
        fixture_entries += 1;
        if variant == Variant::Refusal {
            // The last batch entry collides with this name, so the whole batch
            // is cancelled after the earlier entries have been staged. The
            // blocker is counted separately, because the corrective step
            // removes it and the batch then creates it.
            volume
                .create_file_in_root(names.last().unwrap(), BLOCKER, ts(10))
                .unwrap();
        }
        BatchCreateState {
            names,
            anchor,
            fixture_entries,
            relieved: false,
        }
    }

    fn captured(&self, state: &BatchCreateState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.anchor, ANCHOR.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &BatchCreateState,
    ) -> Result<(), CoreError> {
        let payloads: Vec<Vec<u8>> = (0..state.names.len()).map(payload).collect();
        let operations: Vec<BatchOp<'_>> = state
            .names
            .iter()
            .zip(&payloads)
            .map(|(name, content)| BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name,
                content,
            })
            .collect();
        volume.run_batch(&operations, ts(30))?;
        Ok(())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &BatchCreateState,
        variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        let blocked = variant == Variant::Refusal && !state.relieved;
        let expected = if published {
            state.fixture_entries + state.names.len()
        } else {
            state.fixture_entries + usize::from(blocked)
        };
        assert_eq!(
            volume.list_root().unwrap().len(),
            expected,
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("anchor").unwrap(),
            Some(state.anchor),
            "{context}: anchor"
        );
        file_bytes(volume, state.anchor, ANCHOR, context);
        for (index, name) in state.names.iter().enumerate() {
            let id = volume.lookup_root(name).unwrap();
            let last = index + 1 == state.names.len();
            if published {
                let id = id.unwrap_or_else(|| panic!("{context}: {name} is absent"));
                file_bytes(volume, id, &payload(index), context);
            } else if blocked && last {
                let id = id.unwrap_or_else(|| panic!("{context}: blocker is absent"));
                file_bytes(volume, id, BLOCKER, context);
            } else {
                assert!(id.is_none(), "{context}: {name} is present");
            }
        }
    }

    fn eviction_demand(&self) -> u64 {
        BATCH_CREATE_DEMAND
    }

    fn is_refusal(&self, error: &CoreError) -> bool {
        matches!(error, CoreError::AlreadyExists)
    }

    fn relieve<D: BlockDevice>(&self, volume: &mut Volume<D>, state: &mut BatchCreateState) -> u64 {
        volume
            .delete_file_in_root(state.names.last().unwrap(), ts(40))
            .unwrap();
        state.relieved = true;
        1
    }
}

// ---------------------------------------------------------------------------
// Batch delete
// ---------------------------------------------------------------------------

struct BatchDeleteState {
    /// Names the batch removes, with their object identifiers.
    removed: Vec<(String, u64)>,
    /// Names the fixture keeps, with their object identifiers.
    kept: Vec<(String, u64)>,
    /// Pre-populated long names, verified by count only.
    population: usize,
}

struct BatchDelete;

impl Family for BatchDelete {
    type State = BatchDeleteState;

    fn name(&self) -> &'static str {
        "batch delete"
    }

    fn format(&self, variant: Variant) -> Format {
        structure_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> BatchDeleteState {
        let (mut removed, mut kept) = (Vec::new(), Vec::new());
        let mut population = 0;
        if variant == Variant::Eviction {
            matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
            population = POPULATION;
        }
        for (slot, (gone, stays)) in batch_names(variant, "gone")
            .into_iter()
            .zip(batch_names(variant, "stays"))
            .enumerate()
        {
            let id = volume
                .create_file_in_root(&gone, &payload(slot), ts(10 + slot as i64))
                .unwrap();
            removed.push((gone, id));
            let id = volume
                .create_file_in_root(&stays, &payload(slot), ts(60 + slot as i64))
                .unwrap();
            kept.push((stays, id));
        }
        BatchDeleteState {
            removed,
            kept,
            population,
        }
    }

    /// The snapshot captures every object the batch removes, so the retained
    /// view must still read their exact bytes after publication.
    fn captured(&self, state: &BatchDeleteState) -> Vec<(u64, Vec<u8>)> {
        state
            .removed
            .iter()
            .enumerate()
            .map(|(index, (_, id))| (*id, payload(index)))
            .collect()
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &BatchDeleteState,
    ) -> Result<(), CoreError> {
        let operations: Vec<BatchOp<'_>> = state
            .removed
            .iter()
            .map(|(name, _)| BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name,
            })
            .collect();
        volume.run_batch(&operations, ts(30))?;
        Ok(())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &BatchDeleteState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let published = delta == 1;
        let expected =
            state.population + state.kept.len() + if published { 0 } else { state.removed.len() };
        assert_eq!(
            volume.list_root().unwrap().len(),
            expected,
            "{context}: root entries"
        );
        for (index, (name, id)) in state.removed.iter().enumerate() {
            let found = volume.lookup_root(name).unwrap();
            if published {
                assert_eq!(found, None, "{context}: {name} survived");
                assert!(volume.stat(*id).unwrap().is_none(), "{context}: {id}");
            } else {
                assert_eq!(found, Some(*id), "{context}: {name}");
                file_bytes(volume, *id, &payload(index), context);
            }
        }
        for (index, (name, id)) in state.kept.iter().enumerate() {
            assert_eq!(volume.lookup_root(name).unwrap(), Some(*id), "{context}");
            file_bytes(volume, *id, &payload(index), context);
        }
    }

    fn eviction_demand(&self) -> u64 {
        BATCH_DELETE_DEMAND
    }
}

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

struct DirectoryState {
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

struct DirectorySplit;

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

struct DirectoryCollapse;

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
// Cross-directory rename
// ---------------------------------------------------------------------------

struct CrossRenameState {
    moved: String,
    source: u64,
    target: u64,
    file: u64,
    source_present: Vec<String>,
    target_present: Vec<String>,
}

/// Entries per directory of the pre-populated cross-rename fixture.
const CROSS_POPULATION: usize = 150;
const CROSS_BYTES: &[u8] = b"cross directory rename payload";
/// The moved entry carries a long name, so the move splits a leaf of the
/// target directory and collapses the source directory root.
fn cross_name() -> String {
    let mut name = "zzz-moved-".to_string();
    while name.len() < SUBJECT_NAME_LENGTH {
        name.push('z');
    }
    name
}

struct CrossRename;

impl Family for CrossRename {
    type State = CrossRenameState;

    fn name(&self) -> &'static str {
        "cross-directory rename"
    }

    fn format(&self, _variant: Variant) -> Format {
        wide()
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> CrossRenameState {
        let entries = if variant == Variant::Eviction {
            CROSS_POPULATION
        } else {
            SPLIT_ENTRIES
        };
        let moved = cross_name();
        let source = volume.create_directory_in_root("source", ts(1)).unwrap();
        let target = volume.create_directory_in_root("target", ts(2)).unwrap();
        let mut source_present = Vec::new();
        let mut target_present = Vec::new();
        for (directory, names, prefix) in [
            (source, &mut source_present, "src"),
            (target, &mut target_present, "dst"),
        ] {
            let built: Vec<String> = (0..entries)
                .map(|index| padded_name(prefix, index, SUBJECT_NAME_LENGTH))
                .collect();
            for chunk in built.chunks(32) {
                let operations: Vec<BatchOp<'_>> = chunk
                    .iter()
                    .map(|name| BatchOp::CreateFile {
                        parent_id: directory,
                        name,
                        content: b"",
                    })
                    .collect();
                volume.run_batch(&operations, ts(3)).unwrap();
            }
            names.extend(built);
        }
        let file = volume
            .create_file_in_directory(source, &moved, CROSS_BYTES, ts(4))
            .unwrap();
        source_present.push(moved.clone());
        CrossRenameState {
            moved,
            source,
            target,
            file,
            source_present,
            target_present,
        }
    }

    fn captured(&self, state: &CrossRenameState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, CROSS_BYTES.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CrossRenameState,
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
        state: &CrossRenameState,
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
        file_bytes(volume, state.file, CROSS_BYTES, context);
        assert_eq!(
            volume.stat(state.file).unwrap().unwrap().link_count,
            1,
            "{context}: link count"
        );
    }

    /// The bounded fixture moves the entry out of a full source leaf into a
    /// full target leaf, so one transaction splits the target, raises a root
    /// above it, merges the source leaf and collapses the source root.
    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &CrossRenameState,
        variant: Variant,
    ) {
        let stats = volume.last_commit_stats().unwrap().tree_mutations;
        let expected = u64::from(variant != Variant::Eviction);
        assert_eq!(stats.splits, expected, "{variant:?}: splits");
        assert_eq!(stats.root_splits, expected, "{variant:?}: root splits");
        assert_eq!(stats.merges, expected, "{variant:?}: merges");
        assert_eq!(
            stats.root_collapses, expected,
            "{variant:?}: root collapses"
        );
    }

    /// The rename descends both directory trees and the object map; the wide
    /// fixture evicts at two pages only.
    fn eviction_demand(&self) -> u64 {
        3
    }
}

// ---------------------------------------------------------------------------
// Symlink publication paths
// ---------------------------------------------------------------------------

const LINK_TARGET: &str = "SYS:Tools/Commodities/Exchange";
const MOVED_TARGET: &str = "WORK:Projects/afsplus/notes.txt";

struct SymlinkState {
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

struct SymlinkCreate;

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

struct SymlinkRename;

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

struct SymlinkUnlink;

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

// ---------------------------------------------------------------------------
// Protection and preserved metadata
// ---------------------------------------------------------------------------

const META_BYTES: &[u8] = b"metadata subject bytes";
const PROTECTION: u32 = 0x8000_00ff;

fn wanted() -> PreservedMetadata {
    PreservedMetadata {
        protection: PROTECTION,
        owner_uid: 0,
        owner_gid: 0,
        created: Timespec {
            seconds: 11,
            nanoseconds: 12,
        },
        modified: Timespec {
            seconds: 13,
            nanoseconds: 14,
        },
        changed: Timespec {
            seconds: 15,
            nanoseconds: 16,
        },
    }
}

struct MetadataState {
    file: u64,
    before: ObjectMetadata,
    population: usize,
}

fn metadata_fixture(volume: &mut Volume<MemoryBackend>, variant: Variant) -> MetadataState {
    let mut population = 0;
    if variant == Variant::Eviction {
        matrix::populate(volume, "wide", POPULATION, NAME_LENGTH);
        population = POPULATION;
    }
    let file = volume
        .create_file_in_root("subject", META_BYTES, ts(2))
        .unwrap();
    let before = ObjectMetadata::from(volume.stat(file).unwrap().unwrap());
    MetadataState {
        file,
        before,
        population,
    }
}

struct Protection;

impl Family for Protection {
    type State = MetadataState;

    fn name(&self) -> &'static str {
        "object protection"
    }

    fn format(&self, variant: Variant) -> Format {
        structure_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> MetadataState {
        metadata_fixture(volume, variant)
    }

    fn captured(&self, state: &MetadataState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, META_BYTES.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MetadataState,
    ) -> Result<(), CoreError> {
        volume.set_object_protection(state.file, PROTECTION, ts(30))
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MetadataState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 1,
            "{context}: root entries"
        );
        assert_eq!(
            volume.lookup_root("subject").unwrap(),
            Some(state.file),
            "{context}"
        );
        file_bytes(volume, state.file, META_BYTES, context);
        let record = volume.stat(state.file).unwrap().unwrap();
        if delta == 1 {
            assert_eq!(record.protection, PROTECTION, "{context}: protection");
            assert_eq!(record.created, state.before.created, "{context}: created");
            assert_eq!(
                record.modified, state.before.modified,
                "{context}: modified"
            );
            assert_eq!(record.changed, ts(30), "{context}: changed");
        } else {
            assert_eq!(
                ObjectMetadata::from(record),
                state.before,
                "{context}: unchanged metadata"
            );
        }
    }

    fn eviction_demand(&self) -> u64 {
        METADATA_DEMAND
    }
}

struct Restore;

impl Family for Restore {
    type State = MetadataState;

    fn name(&self) -> &'static str {
        "preserved metadata restore"
    }

    fn format(&self, variant: Variant) -> Format {
        structure_format(variant)
    }

    fn setup(&self, volume: &mut Volume<MemoryBackend>, variant: Variant) -> MetadataState {
        metadata_fixture(volume, variant)
    }

    fn captured(&self, state: &MetadataState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.file, META_BYTES.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MetadataState,
    ) -> Result<(), CoreError> {
        volume.restore_object_metadata(state.file, wanted())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &MetadataState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        assert_eq!(
            volume.list_root().unwrap().len(),
            state.population + 1,
            "{context}: root entries"
        );
        file_bytes(volume, state.file, META_BYTES, context);
        let record = volume.stat(state.file).unwrap().unwrap();
        if delta == 1 {
            assert_eq!(
                PreservedMetadata::from(ObjectMetadata::from(record)),
                wanted(),
                "{context}: restored metadata"
            );
        } else {
            assert_eq!(
                ObjectMetadata::from(record),
                state.before,
                "{context}: unchanged metadata"
            );
        }
    }

    fn eviction_demand(&self) -> u64 {
        METADATA_DEMAND
    }
}

/// A plain fixture whose unflushed tail exceeds the exhaustive budget: the
/// recording, the seeded sampled cut campaign and the full fault matrix.
fn plain_sampled<F: Family>(family: &F, pages: usize, sample: usize, seed: u64) {
    let recording = matrix::record(family, pages, Variant::Plain);
    matrix::sampled_cuts(family, &recording, pages, Variant::Plain, sample, seed);
    matrix::faults(family, &recording, pages, Variant::Plain);
}

/// The same for a retained-snapshot fixture, with ambiguous publication.
fn retained_sampled<F: Family>(family: &F, pages: usize, sample: usize, seed: u64) {
    let recording = matrix::record(family, pages, Variant::Retained);
    matrix::sampled_cuts(family, &recording, pages, Variant::Retained, sample, seed);
    matrix::faults(family, &recording, pages, Variant::Retained);
    matrix::ambiguous(family, pages, Variant::Retained);
}

// ---------------------------------------------------------------------------
// Read failures while reloading spilled nodes
// ---------------------------------------------------------------------------

/// Fails one read once the transaction has begun. A bounded cache reloads the
/// provisional images it spilled earlier in the same transaction, so the
/// injected read reaches that reload path as well as ordinary node reads.
struct ReadFault {
    inner: MemoryBackend,
    fail_read: u64,
    reads: u64,
    armed: bool,
    tripped: bool,
}

impl BlockDevice for ReadFault {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if self.armed {
            if self.reads == self.fail_read {
                self.tripped = true;
                self.reads += 1;
                return Err(BlockError::Injected("staged node reload"));
            }
            self.reads += 1;
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

/// Injected read failures per profile.
const RELOAD_INJECTIONS: u64 = 16;

/// Fails reads spread through a spilling transaction and requires an allowed
/// committed state, a clean checker and a successful retry after remount.
/// Returns the reloads of provisional images the successful run performed.
fn reload_read_failures<F: Family>(family: &F, pages: usize) -> u64 {
    let variant = Variant::Eviction;
    let (base, state, generation, _) = matrix::prepare(family, pages, variant);
    let publications = family.publications(variant);

    let mut probe = matrix::open(TraceBackend::new(base.clone()), pages);
    probe.device_mut().reset();
    family.apply(&mut probe, &state).unwrap();
    let stats = probe.last_commit_stats().unwrap().tree_mutations;
    let reads = probe.device_mut().stats().reads;
    assert!(
        stats.staged_spill_writes > 0,
        "{} pages={pages}: the fixture must spill",
        family.name()
    );

    let stride = (reads / RELOAD_INJECTIONS).max(1);
    let (mut injected, mut published_after_fault) = (0u64, 0u64);
    for fail_read in (0..reads).step_by(stride as usize) {
        let context = format!("{} pages={pages} read {fail_read}", family.name());
        let mut volume = matrix::open(
            ReadFault {
                inner: base.clone(),
                fail_read,
                reads: 0,
                armed: false,
                tripped: false,
            },
            pages,
        );
        volume.device_mut().armed = true;
        if family.apply(&mut volume, &state).is_ok() {
            assert!(
                !volume.device_mut().tripped,
                "{context}: a failed read was not reported"
            );
            continue;
        }
        assert!(volume.device_mut().tripped, "{context}");
        volume.device_mut().armed = false;
        let mut recovered = matrix::open(volume.into_device().inner, pages);
        let delta = recovered
            .generation()
            .checked_sub(generation)
            .filter(|delta| *delta <= publications)
            .unwrap_or_else(|| panic!("{context}: disallowed generation"));
        published_after_fault += u64::from(delta == publications);
        family.verify(&mut recovered, &state, variant, delta, &context);
        matrix::assert_checker_clean(recovered.device_mut(), &context);
        if delta < publications {
            family
                .apply(&mut recovered, &state)
                .unwrap_or_else(|error| panic!("{context}: retry {error}"));
        }
        family.verify(&mut recovered, &state, variant, publications, &context);
        let mut again = matrix::open(recovered.into_device(), pages);
        family.verify(&mut again, &state, variant, publications, &context);
        matrix::assert_checker_clean(again.device_mut(), &context);
        injected += 1;
    }
    assert!(
        injected > 0,
        "{} pages={pages}: no read failed",
        family.name()
    );
    eprintln!(
        "{} pages={pages}: reload read failures reads={reads} stride={stride} refused={injected} already_published={published_after_fault} spills={} reloads={}",
        family.name(),
        stats.staged_spill_writes,
        stats.staged_spill_reloads
    );
    stats.staged_spill_reloads
}

#[test]
fn spilled_directory_split_survives_reload_read_failures() {
    let reloads: u64 = [2, 4, 8]
        .into_iter()
        .map(|pages| reload_read_failures(&DirectorySplit, pages))
        .sum();
    assert!(reloads > 0, "no profile reloaded a provisional image");
}

#[test]
fn spilled_directory_collapse_survives_reload_read_failures() {
    let reloads: u64 = [2, 4, 8]
        .into_iter()
        .map(|pages| reload_read_failures(&DirectoryCollapse, pages))
        .sum();
    assert!(reloads > 0, "no profile reloaded a provisional image");
}

#[test]
fn spilled_batch_create_survives_reload_read_failures() {
    let reloads: u64 = [2, 4, 8]
        .into_iter()
        .map(|pages| reload_read_failures(&BatchCreate, pages))
        .sum();
    assert!(reloads > 0, "no profile reloaded a provisional image");
}

// ---------------------------------------------------------------------------
// Generated tests
// ---------------------------------------------------------------------------

/// Seeds of the sampled cut campaigns, one per spilled family part.
const BATCH_CREATE_SEED: u64 = 0x5eed_ba7c_0001;
const BATCH_DELETE_SEED: u64 = 0x5eed_ba7c_0002;
const DIRECTORY_SPLIT_SEED: u64 = 0x5eed_ba7c_0003;
const DIRECTORY_COLLAPSE_SEED: u64 = 0x5eed_ba7c_0004;
const CROSS_RENAME_SEED: u64 = 0x5eed_ba7c_0005;
/// Subsets drawn per flush segment longer than the exhaustive budget.
const SAMPLE: usize = 32;

crate::profile_tests!(batch_create, |pages| matrix::plain(&BatchCreate, pages, 12));
crate::profile_tests!(batch_create_retained, |pages| matrix::retained(
    &BatchCreate,
    pages,
    12
));
crate::profile_tests!(batch_create_refusal, |pages| matrix::refusal(
    &BatchCreate,
    pages
));
crate::profile_tests!(batch_create_eviction, |pages| matrix::eviction_sampled(
    &BatchCreate,
    pages,
    SAMPLE,
    BATCH_CREATE_SEED
));
crate::profile_tests!(batch_create_spilled_ambiguous, |pages| matrix::ambiguous(
    &BatchCreate,
    pages,
    Variant::Eviction
));
crate::profile_tests!(batch_delete, |pages| matrix::plain(&BatchDelete, pages, 12));
crate::profile_tests!(batch_delete_retained, |pages| matrix::retained(
    &BatchDelete,
    pages,
    12
));
crate::profile_tests!(batch_delete_eviction, |pages| matrix::eviction_sampled(
    &BatchDelete,
    pages,
    SAMPLE,
    BATCH_DELETE_SEED
));
crate::profile_tests!(batch_delete_spilled_ambiguous, |pages| matrix::ambiguous(
    &BatchDelete,
    pages,
    Variant::Eviction
));
crate::profile_tests!(directory_split, |pages| matrix::plain(
    &DirectorySplit,
    pages,
    12
));
crate::profile_tests!(directory_split_retained, |pages| matrix::retained(
    &DirectorySplit,
    pages,
    12
));
crate::profile_tests!(directory_split_eviction, |pages| matrix::eviction_sampled(
    &DirectorySplit,
    pages,
    SAMPLE,
    DIRECTORY_SPLIT_SEED
));
crate::profile_tests!(directory_collapse, |pages| matrix::plain(
    &DirectoryCollapse,
    pages,
    12
));
crate::profile_tests!(directory_collapse_eviction, |pages| {
    matrix::eviction_sampled(&DirectoryCollapse, pages, SAMPLE, DIRECTORY_COLLAPSE_SEED)
});
crate::profile_tests!(cross_rename, |pages| plain_sampled(
    &CrossRename,
    pages,
    SAMPLE,
    CROSS_RENAME_SEED
));
crate::profile_tests!(cross_rename_retained, |pages| retained_sampled(
    &CrossRename,
    pages,
    SAMPLE,
    CROSS_RENAME_SEED
));
crate::profile_tests!(cross_rename_eviction, |pages| matrix::eviction_sampled(
    &CrossRename,
    pages,
    SAMPLE,
    CROSS_RENAME_SEED
));
crate::profile_tests!(symlink_create, |pages| matrix::plain(
    &SymlinkCreate,
    pages,
    12
));
crate::profile_tests!(symlink_create_eviction, |pages| matrix::eviction(
    &SymlinkCreate,
    pages,
    None
));
crate::profile_tests!(symlink_rename_eviction, |pages| matrix::eviction(
    &SymlinkRename,
    pages,
    None
));
crate::profile_tests!(symlink_unlink_eviction, |pages| matrix::eviction(
    &SymlinkUnlink,
    pages,
    None
));
crate::profile_tests!(protection_retained, |pages| matrix::retained(
    &Protection,
    pages,
    12
));
crate::profile_tests!(protection_eviction, |pages| matrix::eviction(
    &Protection,
    pages,
    None
));
crate::profile_tests!(restore_retained, |pages| matrix::retained(
    &Restore, pages, 12
));
crate::profile_tests!(restore_eviction, |pages| matrix::eviction(
    &Restore, pages, None
));
