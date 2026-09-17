//! Committed-state loading and full invariant verification, shared by the
//! core and the checker (ADR-015) but with distinct roles:
//!
//! - [`load_mount_state`] reads only the object-map root, root object/root
//!   directory, and the bounded reclaim-queue root. Normal mount uses this
//!   path and never walks every object, allocation bitmap, or the sealed
//!   reclaim segments.
//! - [`load_committed_state`] decodes everything the chosen checkpoint
//!   references, with per-structure validation, bounds checks, and unique
//!   block ownership. The checker and explicit shadow verification use it.
//! - [`full_sweep`] is the checker/shadow-verification layer: link counts,
//!   orphaned objects, bitmap-versus-reachability equality, retired-list
//!   quarantine invariants, reserved-bit checks.

mod snapshots;

use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::BlockDevice;
use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::geometry::{Geometry, DESCRIPTOR_SLOTS};
use afsplus_format::ident::{
    Identification, COMPAT_DATA_POLICY, INCOMPAT_SECURITY_DESCRIPTORS, RO_COMPAT_ORPHAN_DIRECTORY,
    RO_COMPAT_SHARED_EXTENTS,
};
use afsplus_format::object::{
    ObjectRecord, ObjectType, OBJECT_FLAG_DATA_IN_PLACE, OBJECT_FLAG_EXTENT_TREE,
};
use afsplus_format::reclaim::{ReclaimEntry, ReclaimRoot};
use afsplus_format::{OBJECT_FIRST_DYNAMIC, OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};

use crate::alloc::Bitmaps;
use crate::allocation_root;
use crate::directory::{self, LoadedDirectory};
use crate::extent_map;
use crate::flight::{
    EventKind, FindingKind, FlightRecorder, LifecycleContext, VerifyContext, VerifyPhase,
    VerifyScope,
};
use crate::object_map::{self, LoadedObjectMap};
use crate::shared_extents::{self, SharedRun};
use crate::CoreError;

/// Verification progress and finding locations for an attached recorder.
/// A verification entry point without a recorder builds an empty observer,
/// which emits nothing and holds no state a caller can see. Emission performs
/// no device I/O and no allocation, so an observed verification reads exactly
/// the blocks and reports exactly the findings of an unobserved one.
struct VerifyObserver<'a> {
    recorder: Option<&'a mut FlightRecorder>,
    scope: VerifyScope,
    generation: u64,
    /// The phase owning the next observation, reported with a failure.
    phase: Option<VerifyPhase>,
}

impl<'a> VerifyObserver<'a> {
    fn new(recorder: Option<&'a mut FlightRecorder>, scope: VerifyScope, generation: u64) -> Self {
        Self {
            recorder,
            scope,
            generation,
            phase: None,
        }
    }

    fn emit(&mut self, kind: EventKind, context: VerifyContext) {
        let generation = self.generation;
        if let Some(recorder) = &mut self.recorder {
            recorder.lifecycle_event(
                generation,
                kind,
                false,
                0,
                LifecycleContext::Verify(context),
            );
        }
    }

    fn context(&self) -> VerifyContext {
        VerifyContext {
            scope: self.scope,
            phase: self.phase,
            finding: None,
            region: 0,
            ordinal: 0,
            object_id: 0,
            block: 0,
        }
    }

    fn begin(&mut self) {
        self.phase = None;
        let context = self.context();
        self.emit(EventKind::VerifyBegin, context);
    }

    fn phase(&mut self, phase: VerifyPhase, block: u64) {
        self.phase = Some(phase);
        let context = VerifyContext {
            block,
            ..self.context()
        };
        self.emit(EventKind::VerifyPhase, context);
    }

    /// Report the finding just appended, at its ordinal in the returned list.
    fn finding(
        &mut self,
        kind: FindingKind,
        ordinal: usize,
        region: u32,
        object_id: u64,
        block: u64,
    ) {
        let context = VerifyContext {
            finding: Some(kind),
            ordinal: ordinal as u64,
            region,
            object_id,
            block,
            ..self.context()
        };
        self.emit(EventKind::VerifyFinding, context);
    }

    fn complete(&mut self, ordinal: u64) {
        self.phase = None;
        let context = VerifyContext {
            ordinal,
            ..self.context()
        };
        self.emit(EventKind::VerifyComplete, context);
    }

    fn failed(&mut self) {
        let context = self.context();
        self.emit(EventKind::VerifyFailed, context);
    }
}

/// Everything reachable from one committed checkpoint, fully decoded.
pub struct CommittedState {
    /// Physical lifetime ownership, including retired runs awaiting transfer.
    pub snapshot_owned_blocks: Vec<u64>,
    pub object_map: LoadedObjectMap,
    pub allocation_records: Vec<afsplus_format::checkpoint::RegionRecord>,
    pub allocation_pool_blocks: Vec<u64>,
    /// Permanently allocated intent-log slots (ADR-037); empty when disabled.
    pub log_area_blocks: Vec<u64>,
    pub objects: BTreeMap<u64, ObjectRecord>,
    /// Directory trees keyed by owning directory object ID.
    pub directories: BTreeMap<u64, LoadedDirectory>,
    /// Every unconsumed quarantined run, cursor-adjusted, FIFO order.
    pub reclaim_runs: Vec<ReclaimEntry>,
    pub reclaim_pending_blocks: u64,
    pub bitmaps: Bitmaps,
    /// Every reachable metadata block (object map, records, directory trees,
    /// reclaim-queue structure and registered views) — excludes reserved blocks
    /// and file data. Physical sharing across views is counted once.
    pub metadata_blocks: Vec<u64>,
    /// Every distinct file-data block reachable from live or registered views.
    pub data_blocks: Vec<u64>,
    /// The canonical shared-run records (ADR-061), already cross-checked:
    /// every record's reference count equals the live mappings found over
    /// its run during this load.
    pub shared_records: Vec<SharedRun>,
    /// The reference tree's own node blocks.
    pub shared_tree_blocks: Vec<u64>,
}

/// Snapshot control records and advertised view count, read along bounded
/// root-to-control paths. Exhaustive ownership checking is a separate gate.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotMountState {
    pub registry: afsplus_format::snapshot::RegistryState,
    pub views: u64,
    pub ledger: afsplus_format::snapshot::LedgerState,
}

/// Bounded state needed to expose a mounted root namespace. The object map and
/// directory are trees: mount validates their roots and descends on demand.
pub struct MountState {
    pub snapshots: Option<SnapshotMountState>,
    pub root_record_lba: u64,
    pub root_object: ObjectRecord,
    pub root_directory_root_lba: u64,
    pub reclaim_root: ReclaimRoot,
}

/// Loads only bounded roots for normal operation. This intentionally does
/// not prove whole-volume reachability, link counts, bitmap equality, or the
/// integrity of every descendant record; those checks belong to
/// [`load_committed_state`] plus [`full_sweep`]. Descendants are decoded when
/// accessed.
pub fn load_mount_state<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
) -> Result<MountState, CoreError> {
    load_mount_state_inner(
        dev,
        ident,
        checkpoint,
        &mut VerifyObserver::new(None, VerifyScope::MountState, checkpoint.generation),
    )
}

/// Load the bounded roots with a caller-owned recorder attached. Standalone
/// verification owns this observation; normal mount keeps the unobserved
/// bounded path, so exhaustive verification stays outside mount.
pub fn load_mount_state_observed<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    recorder: &mut FlightRecorder,
) -> Result<MountState, CoreError> {
    let mut observer = VerifyObserver::new(
        Some(recorder),
        VerifyScope::MountState,
        checkpoint.generation,
    );
    observer.begin();
    let result = load_mount_state_inner(dev, ident, checkpoint, &mut observer);
    match result {
        Ok(state) => {
            observer.complete(0);
            Ok(state)
        }
        Err(error) => {
            observer.failed();
            Err(error)
        }
    }
}

fn load_mount_state_inner<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    observer: &mut VerifyObserver<'_>,
) -> Result<MountState, CoreError> {
    let _allocation_scope =
        crate::allocation_trace::enter(crate::allocation_trace::Domain::Verifier);
    let geo = ident.geometry();
    let mut buf = vec![0u8; geo.block_size];
    let mut roots = BTreeSet::new();
    let claim_root = |lba: u64, roots: &mut BTreeSet<u64>| -> Result<(), CoreError> {
        if !geo.is_allocatable(lba) {
            return Err(CoreError::Corrupt(format!(
                "mount root block {lba} outside allocatable bounds"
            )));
        }
        if !roots.insert(lba) {
            return Err(CoreError::Corrupt(format!(
                "mount root block {lba} referenced twice"
            )));
        }
        Ok(())
    };

    observer.phase(VerifyPhase::ObjectMap, checkpoint.object_map_block);
    claim_root(checkpoint.object_map_block, &mut roots)?;
    if checkpoint.shared_extent_root_block != 0 {
        // ADR-061: bounded mount claims the root; kind, owner and generation
        // are checked whenever the tree is actually loaded.
        observer.phase(
            VerifyPhase::SharedExtents,
            checkpoint.shared_extent_root_block,
        );
        claim_root(checkpoint.shared_extent_root_block, &mut roots)?;
        observer.phase(VerifyPhase::ObjectMap, checkpoint.object_map_block);
    }
    let root_record_lba = object_map::lookup_lba(
        dev,
        &geo,
        checkpoint.object_map_block,
        checkpoint.generation,
        OBJECT_ROOT,
    )?
    .ok_or_else(|| CoreError::Corrupt("root object missing from object map".into()))?;
    observer.phase(VerifyPhase::RootObject, root_record_lba);
    claim_root(root_record_lba, &mut roots)?;
    dev.read_block(root_record_lba, &mut buf)?;
    let (root_object, root_generation) = ObjectRecord::decode_metadata_with_generation(&buf)?;
    if root_generation == 0 || root_generation > checkpoint.generation {
        return Err(CoreError::Corrupt(format!(
            "root object record block {root_record_lba} generation {root_generation} outside committed range"
        )));
    }
    if root_object.object_id != OBJECT_ROOT || root_object.object_type != ObjectType::Directory {
        return Err(CoreError::Corrupt(
            "root object is not the root directory".into(),
        ));
    }

    observer.phase(VerifyPhase::RootDirectory, root_object.data_root);
    claim_root(root_object.data_root, &mut roots)?;
    directory::validate_root(
        dev,
        &geo,
        root_object.data_root,
        OBJECT_ROOT,
        checkpoint.generation,
    )?;

    // Bounded reclaim view: decode and validate only the root block. The
    // sealed segments/tables behind it are batch and checker territory.
    observer.phase(VerifyPhase::ReclaimQueue, checkpoint.reclaim_root_block);
    claim_root(checkpoint.reclaim_root_block, &mut roots)?;
    dev.read_block(checkpoint.reclaim_root_block, &mut buf)?;
    let (reclaim_root, reclaim_generation) =
        ReclaimRoot::decode(&buf).map_err(|e| CoreError::Corrupt(format!("reclaim root: {e}")))?;
    if reclaim_generation > checkpoint.generation {
        return Err(CoreError::Corrupt(
            "reclaim root generation is from the future".into(),
        ));
    }
    for entry in &reclaim_root.inline_entries {
        crate::reclaim::validate_run(&geo, entry)?;
        if entry.retire_generation > checkpoint.generation {
            return Err(CoreError::Corrupt(
                "reclaim entry generation is from the future".into(),
            ));
        }
        if roots.contains(&entry.start) {
            return Err(CoreError::Corrupt(format!(
                "quarantined block {} is still a mounted root",
                entry.start
            )));
        }
    }

    let snapshots = if let Some(snapshot_roots) = checkpoint.snapshot_roots {
        observer.phase(VerifyPhase::Snapshots, snapshot_roots.registry);
        claim_root(snapshot_roots.registry, &mut roots)?;
        claim_root(snapshot_roots.lifetimes, &mut roots)?;
        let (registry, views, _) = crate::snapshot::read_registry_state(
            dev,
            &geo,
            snapshot_roots.registry,
            checkpoint.generation,
        )?;
        let (ledger, _, _) = crate::snapshot::read_ledger_state(
            dev,
            &geo,
            snapshot_roots.lifetimes,
            checkpoint.generation,
        )?;
        Some(SnapshotMountState {
            registry,
            views,
            ledger,
        })
    } else {
        None
    };
    Ok(MountState {
        snapshots,
        root_record_lba,
        root_object,
        root_directory_root_lba: root_object.data_root,
        reclaim_root,
    })
}

fn validate_mapped_object_id(
    object_id: u64,
    checkpoint: &Checkpoint,
    orphan_directory_enabled: bool,
) -> Result<(), CoreError> {
    if object_id >= checkpoint.next_object_id {
        return Err(CoreError::Corrupt(format!(
            "object {object_id} at or above next_object_id {}",
            checkpoint.next_object_id
        )));
    }
    if object_id != OBJECT_ROOT
        && !(orphan_directory_enabled && object_id == OBJECT_ORPHAN_DIRECTORY)
        && object_id < OBJECT_FIRST_DYNAMIC
    {
        return Err(CoreError::Corrupt(format!(
            "object {object_id} in reserved internal ID range"
        )));
    }
    Ok(())
}

/// Decodes and cross-validates the state referenced by `checkpoint`.
pub fn load_committed_state<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
) -> Result<CommittedState, CoreError> {
    load_committed_state_inner(
        dev,
        ident,
        checkpoint,
        &mut VerifyObserver::new(None, VerifyScope::CommittedState, checkpoint.generation),
    )
}

/// Decode the committed state with a caller-owned recorder attached. Each
/// phase event names the structure being decoded and its root block where the
/// checkpoint holds one; a failure reports the phase that raised it.
pub fn load_committed_state_observed<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    recorder: &mut FlightRecorder,
) -> Result<CommittedState, CoreError> {
    let mut observer = VerifyObserver::new(
        Some(recorder),
        VerifyScope::CommittedState,
        checkpoint.generation,
    );
    observer.begin();
    let result = load_committed_state_inner(dev, ident, checkpoint, &mut observer);
    match result {
        Ok(state) => {
            observer.complete(0);
            Ok(state)
        }
        Err(error) => {
            observer.failed();
            Err(error)
        }
    }
}

fn load_committed_state_inner<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    observer: &mut VerifyObserver<'_>,
) -> Result<CommittedState, CoreError> {
    let geo = ident.geometry();
    let block_size = geo.block_size;
    let mut buf = vec![0u8; block_size];

    // One shared ownership set: any block referenced twice — metadata/
    // metadata, metadata/data, or data/data — is corruption.
    let mut claimed: BTreeSet<u64> = BTreeSet::new();
    let mut metadata_blocks: Vec<u64> = Vec::new();
    let mut data_blocks: Vec<u64> = Vec::new();
    let claim = |lba: u64, claimed: &mut BTreeSet<u64>| -> Result<(), CoreError> {
        if !geo.is_allocatable(lba) {
            return Err(CoreError::Corrupt(format!(
                "block {lba} outside allocatable bounds"
            )));
        }
        if !claimed.insert(lba) {
            return Err(CoreError::Corrupt(format!("block {lba} referenced twice")));
        }
        Ok(())
    };

    observer.phase(
        VerifyPhase::AllocationRoot,
        checkpoint.allocation_root_block,
    );
    let allocation = if checkpoint.allocation_root_block != 0 {
        allocation_root::load_all(
            dev,
            &geo,
            checkpoint.allocation_root_block,
            checkpoint.generation,
        )?
    } else {
        return Err(CoreError::PrototypeLimit(
            "checker requires the AFST allocation root",
        ));
    };
    for lba in &allocation.tree_blocks {
        claim(*lba, &mut claimed)?;
        metadata_blocks.push(*lba);
    }
    let allocation_layout = allocation_root::bulk_build(&geo, &allocation.records)?;

    // ADR-061: the reference tree is the authority for multi-owner runs.
    // Its nodes are ordinary reachable metadata; each record's run is
    // claimed once here, then counted per live mapping below.
    let (shared_records, shared_tree_blocks) = if checkpoint.shared_extent_root_block != 0 {
        observer.phase(
            VerifyPhase::SharedExtents,
            checkpoint.shared_extent_root_block,
        );
        let loaded = shared_extents::load_all(
            dev,
            &geo,
            checkpoint.shared_extent_root_block,
            checkpoint.generation,
        )?;
        for lba in &loaded.tree_blocks {
            claim(*lba, &mut claimed)?;
            metadata_blocks.push(*lba);
        }
        for run in &loaded.records {
            for lba in run.physical_start..run.physical_end()? {
                claim(lba, &mut claimed)?;
                data_blocks.push(lba);
            }
        }
        (loaded.records, loaded.tree_blocks)
    } else {
        (Vec::new(), Vec::new())
    };
    // Physical intervals of live mappings, split by whether they carry the
    // conservative marker. The shared cross-check below is an endpoint sweep
    // over these intervals: memory and work stay proportional to the number
    // of mapping boundaries, never to the numeric size of the runs.
    let mut flagged_intervals: Vec<(u64, u64)> = Vec::new();
    let mut unflagged_intervals: Vec<(u64, u64)> = Vec::new();
    let shared_enabled = ident.features.ro_compat & RO_COMPAT_SHARED_EXTENTS != 0;
    let orphan_directory_enabled = ident.features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0;

    let namespace_metadata_start = metadata_blocks.len();
    observer.phase(VerifyPhase::ObjectMap, checkpoint.object_map_block);
    let object_map = object_map::load_all(
        dev,
        &geo,
        checkpoint.object_map_block,
        checkpoint.generation,
    )?;
    for lba in &object_map.tree_blocks {
        claim(*lba, &mut claimed)?;
        metadata_blocks.push(*lba);
    }

    let mut objects = BTreeMap::new();
    let mut directories = BTreeMap::new();

    observer.phase(VerifyPhase::ObjectRecords, 0);
    for entry in &object_map.entries {
        validate_mapped_object_id(entry.object_id, checkpoint, orphan_directory_enabled)?;
        claim(entry.block, &mut claimed)?;
        metadata_blocks.push(entry.block);
        dev.read_block(entry.block, &mut buf)?;
        let (record, record_generation) = ObjectRecord::decode_metadata_with_generation(&buf)?;
        if record_generation == 0 || record_generation > checkpoint.generation {
            return Err(CoreError::Corrupt(format!(
                "object {} record block {} generation {record_generation} outside committed range",
                entry.object_id, entry.block
            )));
        }
        if record.flags & OBJECT_FLAG_DATA_IN_PLACE != 0
            && ident.features.compat & COMPAT_DATA_POLICY == 0
        {
            return Err(CoreError::Corrupt(format!(
                "object {} carries OBJECT_FLAG_DATA_IN_PLACE without the data-policy feature",
                entry.object_id
            )));
        }
        if let Some(reference) = record.security {
            if ident.features.incompat & INCOMPAT_SECURITY_DESCRIPTORS == 0 {
                return Err(CoreError::Corrupt(format!(
                    "object {} carries a security reference without the security-descriptors feature",
                    entry.object_id
                )));
            }
            let (segments, _) = crate::volume::load_descriptor_chain(
                dev,
                &geo,
                entry.object_id,
                reference,
                checkpoint.generation,
            )?;
            for lba in segments {
                claim(lba, &mut claimed)?;
                metadata_blocks.push(lba);
            }
        }
        if let Some(reference) = record.attributes {
            let (segments, _) = crate::volume::load_attribute_chain(
                dev,
                &geo,
                entry.object_id,
                reference,
                checkpoint.generation,
            )?;
            for lba in segments {
                claim(lba, &mut claimed)?;
                metadata_blocks.push(lba);
            }
        }
        if record.object_id != entry.object_id {
            return Err(CoreError::Corrupt(format!(
                "object record at block {} claims ID {}, map says {}",
                entry.block, record.object_id, entry.object_id
            )));
        }
        if record.object_id == OBJECT_ORPHAN_DIRECTORY
            && (record.object_type != ObjectType::Directory
                || record.flags != 0
                || record.link_count != 1
                || record.size_bytes != 0
                || record.allocated_bytes != 0
                || record.data_blocks != 0)
        {
            return Err(CoreError::Corrupt(
                "reserved orphan-directory object has invalid metadata".into(),
            ));
        }
        match record.object_type {
            ObjectType::Directory => {
                let dir = directory::load_all(
                    dev,
                    &geo,
                    record.data_root,
                    record.object_id,
                    checkpoint.generation,
                    ident,
                )?;
                for lba in &dir.tree_blocks {
                    claim(*lba, &mut claimed)?;
                    metadata_blocks.push(*lba);
                }
                directories.insert(record.object_id, dir);
            }
            ObjectType::File => {
                if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
                    let map = extent_map::load_all(
                        dev,
                        &geo,
                        record.data_root,
                        record.object_id,
                        checkpoint.generation,
                    )?;
                    if map.allocated_blocks != record.data_blocks {
                        return Err(CoreError::Corrupt(format!(
                            "object {} records {} allocated blocks, extent map has {}",
                            record.object_id, record.data_blocks, map.allocated_blocks
                        )));
                    }
                    for lba in map.tree_blocks {
                        claim(lba, &mut claimed)?;
                        metadata_blocks.push(lba);
                    }
                    for extent in map.extents {
                        if extent.flags & crate::extent_map::EXTENT_SHARED != 0 {
                            // Fail closed (ADR-061): a flagged extent has no
                            // legal history on a volume without the feature
                            // or without a reference tree.
                            if !shared_enabled || checkpoint.shared_extent_root_block == 0 {
                                return Err(CoreError::Corrupt(format!(
                                    "object {} carries EXTENT_SHARED {}",
                                    record.object_id,
                                    if shared_enabled {
                                        "but the volume has no reference tree"
                                    } else {
                                        "without the shared-extents feature"
                                    }
                                )));
                            }
                            flagged_intervals.push((extent.physical_start, extent.physical_end()?));
                        } else {
                            unflagged_intervals
                                .push((extent.physical_start, extent.physical_end()?));
                            for lba in extent.physical_start..extent.physical_end()? {
                                claim(lba, &mut claimed)?;
                                data_blocks.push(lba);
                            }
                        }
                    }
                } else {
                    let data_end = record
                        .data_root
                        .checked_add(record.data_blocks)
                        .ok_or_else(|| {
                            CoreError::Corrupt(format!(
                                "object {} extent end overflows block address",
                                record.object_id
                            ))
                        })?;
                    for lba in record.data_root..data_end {
                        claim(lba, &mut claimed)?;
                        data_blocks.push(lba);
                    }
                }
            }
            ObjectType::Symlink => {}
            _ => unreachable!("rejected by ObjectRecord::decode"),
        }
        objects.insert(record.object_id, record);
    }

    // Bidirectional (ADR-061), by endpoint sweep: the canonical runs
    // reconstructed from the flagged mappings must equal the stored records
    // exactly, so a missing record is as detectable as a wrong count, and
    // work is proportional to the number of boundaries.
    {
        observer.phase(VerifyPhase::SharedMappings, 0);
        let mut events: BTreeMap<u64, i64> = BTreeMap::new();
        for (start, end) in &flagged_intervals {
            *events.entry(*start).or_insert(0) += 1;
            *events.entry(*end).or_insert(0) -= 1;
        }
        let mut expected: Vec<SharedRun> = Vec::new();
        let mut count: i64 = 0;
        let mut previous: Option<u64> = None;
        for (position, delta) in &events {
            if *delta == 0 {
                continue;
            }
            if let Some(start) = previous {
                if count >= 2 {
                    expected.push(SharedRun {
                        physical_start: start,
                        block_count: position - start,
                        reference_count: u32::try_from(count).map_err(|_| {
                            CoreError::Corrupt(format!(
                                "shared run at {start} has more references than fit a count"
                            ))
                        })?,
                        flags: 0,
                    });
                } else if count == 1 {
                    // The private part of a flagged mapping: exactly one
                    // owner, claimed exclusively like any other data.
                    for lba in start..*position {
                        claim(lba, &mut claimed)?;
                        data_blocks.push(lba);
                    }
                }
            }
            count += delta;
            previous = Some(*position);
        }
        if count != 0 {
            return Err(CoreError::Corrupt(
                "shared mapping sweep does not balance".into(),
            ));
        }
        if expected != shared_records {
            return Err(CoreError::Corrupt(format!(
                "live shared mappings reconstruct {} canonical runs {:?}, tree stores {} {:?}",
                expected.len(),
                expected
                    .iter()
                    .map(|run| (run.physical_start, run.block_count, run.reference_count))
                    .collect::<Vec<_>>(),
                shared_records.len(),
                shared_records
                    .iter()
                    .map(|run| (run.physical_start, run.block_count, run.reference_count))
                    .collect::<Vec<_>>(),
            )));
        }

        // A mapping over a record without the marker is the dangerous false
        // negative. Both lists are sorted intervals: one linear pass.
        let mut unflagged = unflagged_intervals;
        unflagged.sort_unstable();
        let mut record_index = 0usize;
        for (start, end) in unflagged {
            while record_index < shared_records.len()
                && shared_records[record_index].physical_end()? <= start
            {
                record_index += 1;
            }
            if let Some(run) = shared_records.get(record_index) {
                if run.physical_start < end {
                    return Err(CoreError::Corrupt(format!(
                        "blocks {}..{} map a shared run without EXTENT_SHARED",
                        start.max(run.physical_start),
                        end.min(run.physical_end()?)
                    )));
                }
            }
        }
    }

    observer.phase(VerifyPhase::Namespace, 0);
    validate_namespace_graph(&objects, &directories)?;
    let namespace_metadata = metadata_blocks[namespace_metadata_start..].to_vec();

    // Exhaustive reclaim-queue walk: every structure block is reachable
    // metadata; every unconsumed run must be disjoint from reachable state.
    observer.phase(VerifyPhase::ReclaimQueue, checkpoint.reclaim_root_block);
    let reclaim = crate::reclaim::load_all(
        dev,
        &geo,
        checkpoint.reclaim_root_block,
        checkpoint.generation,
    )?;
    for lba in &reclaim.structure_blocks {
        claim(*lba, &mut claimed)?;
        metadata_blocks.push(*lba);
    }
    for run in &reclaim.runs {
        for lba in run.start..run.end().map_err(CoreError::Format)? {
            if claimed.contains(&lba) {
                return Err(CoreError::Corrupt(format!(
                    "quarantined block {lba} is still reachable"
                )));
            }
        }
        if run.retire_generation > checkpoint.generation {
            return Err(CoreError::Corrupt(format!(
                "quarantined run {} from future generation {}",
                run.start, run.retire_generation
            )));
        }
    }

    observer.phase(VerifyPhase::AllocationBitmaps, 0);
    let bitmaps = Bitmaps::load(dev, &geo, checkpoint)?;
    observer.phase(VerifyPhase::IntentLogArea, 0);
    let log_area_blocks = crate::intent_log::log_slot_lbas(&geo, ident.log_slots)?;

    let mut state = CommittedState {
        snapshot_owned_blocks: Vec::new(),
        object_map,
        allocation_records: allocation.records,
        allocation_pool_blocks: allocation_layout.pool_lbas,
        log_area_blocks,
        objects,
        directories,
        reclaim_runs: reclaim.runs,
        reclaim_pending_blocks: reclaim.pending_blocks,
        bitmaps,
        metadata_blocks,
        data_blocks,
        shared_records,
        shared_tree_blocks,
    };
    observer.phase(
        VerifyPhase::Snapshots,
        checkpoint.snapshot_roots.map_or(0, |roots| roots.registry),
    );
    snapshots::load(dev, ident, checkpoint, &mut state, &namespace_metadata)?;
    Ok(state)
}

fn validate_namespace_graph(
    objects: &BTreeMap<u64, ObjectRecord>,
    directories: &BTreeMap<u64, LoadedDirectory>,
) -> Result<(), CoreError> {
    let root = objects
        .get(&OBJECT_ROOT)
        .ok_or_else(|| CoreError::Corrupt("root object missing from object map".into()))?;
    if root.object_type != ObjectType::Directory {
        return Err(CoreError::Corrupt("root object is not a directory".into()));
    }

    for (dir_id, dir) in directories {
        for entry in &dir.entries {
            let child = objects.get(&entry.child_id).ok_or_else(|| {
                CoreError::Corrupt(format!(
                    "directory {dir_id} references missing object {}",
                    entry.child_id
                ))
            })?;
            let hint_matches = matches!(
                (entry.child_type_hint, child.object_type),
                (1, ObjectType::File) | (2, ObjectType::Directory) | (3, ObjectType::Symlink)
            );
            if !hint_matches {
                return Err(CoreError::Corrupt(format!(
                    "directory {dir_id} type hint mismatch for object {}",
                    entry.child_id
                )));
            }
            if *dir_id == OBJECT_ORPHAN_DIRECTORY {
                let expected_name = format!("{:016x}", entry.child_id);
                if entry.name.as_slice() != expected_name.as_bytes()
                    || entry.child_type_hint != 1
                    || child.object_type != ObjectType::File
                {
                    return Err(CoreError::Corrupt(format!(
                        "orphan directory entry for object {} has invalid name or type",
                        entry.child_id
                    )));
                }
            } else if entry.child_id == OBJECT_ORPHAN_DIRECTORY {
                return Err(CoreError::Corrupt(format!(
                    "directory {dir_id} exposes the reserved orphan directory"
                )));
            }
        }
    }

    // Reference counts alone cannot detect a disconnected directory cycle:
    // every object in that cycle can have exactly one incoming reference.
    let mut pending = vec![OBJECT_ROOT];
    if objects.contains_key(&OBJECT_ORPHAN_DIRECTORY) {
        pending.push(OBJECT_ORPHAN_DIRECTORY);
    }
    let mut reached = BTreeSet::new();
    let mut visited_directories = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited_directories.insert(id) {
            return Err(CoreError::Corrupt(format!(
                "directory {id} has a cycle or multiple parents"
            )));
        }
        reached.insert(id);
        if let Some(directory) = directories.get(&id) {
            for entry in &directory.entries {
                reached.insert(entry.child_id);
                if objects[&entry.child_id].object_type == ObjectType::Directory {
                    pending.push(entry.child_id);
                }
            }
        }
    }
    if let Some(id) = objects.keys().find(|id| !reached.contains(id)) {
        return Err(CoreError::Corrupt(format!(
            "object {id} is unreachable from namespace roots"
        )));
    }
    Ok(())
}

/// Full invariant sweep over a loaded state (`spec/invariants.md`).
/// Returns findings instead of failing fast so the checker can report all of
/// them. Normal mount does not run this.
pub fn full_sweep(state: &CommittedState, geo: &Geometry, checkpoint: &Checkpoint) -> Vec<String> {
    full_sweep_inner(
        state,
        geo,
        checkpoint,
        &mut VerifyObserver::new(None, VerifyScope::FullSweep, checkpoint.generation),
    )
}

/// Sweep the invariants with a caller-owned recorder attached. Every finding
/// receives one event at its ordinal in the returned list, carrying the
/// finding class and the object, block or region it names; the completion
/// event carries the finding count. The returned findings are those of
/// `full_sweep` over the same state, in the same order.
pub fn full_sweep_observed(
    state: &CommittedState,
    geo: &Geometry,
    checkpoint: &Checkpoint,
    recorder: &mut FlightRecorder,
) -> Vec<String> {
    let mut observer = VerifyObserver::new(
        Some(recorder),
        VerifyScope::FullSweep,
        checkpoint.generation,
    );
    observer.begin();
    let findings = full_sweep_inner(state, geo, checkpoint, &mut observer);
    observer.complete(findings.len() as u64);
    findings
}

fn full_sweep_inner(
    state: &CommittedState,
    geo: &Geometry,
    checkpoint: &Checkpoint,
    observer: &mut VerifyObserver<'_>,
) -> Vec<String> {
    let mut findings = Vec::new();

    // Link counts: live references match, and nothing dangles unreferenced.
    observer.phase(VerifyPhase::LinkCounts, 0);
    let mut ref_counts: BTreeMap<u64, u32> = BTreeMap::new();
    ref_counts.insert(OBJECT_ROOT, 1);
    if state.objects.contains_key(&OBJECT_ORPHAN_DIRECTORY) {
        ref_counts.insert(OBJECT_ORPHAN_DIRECTORY, 1);
    }
    for dir in state.directories.values() {
        for entry in &dir.entries {
            *ref_counts.entry(entry.child_id).or_insert(0) += 1;
        }
    }
    for (id, record) in &state.objects {
        let expected = ref_counts.get(id).copied().unwrap_or(0);
        if record.link_count != expected {
            findings.push(format!(
                "object {id} link count {} does not match {} reachable references",
                record.link_count, expected
            ));
            observer.finding(FindingKind::LinkCount, findings.len() - 1, 0, *id, 0);
        }
        if expected == 0 && *id != OBJECT_ROOT {
            findings.push(format!(
                "object {id} remains in the object map with no directory reference"
            ));
            observer.finding(
                FindingKind::UnreferencedObject,
                findings.len() - 1,
                0,
                *id,
                0,
            );
        }
    }

    // Quarantined runs: every block allocated, unreachable, and outside the
    // permanent allocation-root pool.
    observer.phase(VerifyPhase::QuarantinedRuns, 0);
    for run in &state.reclaim_runs {
        for lba in run.start..run.start + run.blocks as u64 {
            if !state.bitmaps.is_allocated(lba) {
                findings.push(format!("quarantined block {lba} is marked free"));
                observer.finding(FindingKind::QuarantinedFree, findings.len() - 1, 0, 0, lba);
            }
            if state.metadata_blocks.contains(&lba)
                || state.data_blocks.contains(&lba)
                || state.snapshot_owned_blocks.contains(&lba)
            {
                findings.push(format!("quarantined block {lba} is still reachable"));
                observer.finding(
                    FindingKind::QuarantinedReachable,
                    findings.len() - 1,
                    0,
                    0,
                    lba,
                );
            }
            if state.allocation_pool_blocks.contains(&lba) {
                findings.push(format!(
                    "quarantined block {lba} belongs to the permanent allocation-root pool"
                ));
                observer.finding(FindingKind::QuarantinedPool, findings.len() - 1, 0, 0, lba);
            }
            if state.log_area_blocks.contains(&lba) {
                findings.push(format!(
                    "quarantined block {lba} belongs to the intent-log area"
                ));
                observer.finding(
                    FindingKind::QuarantinedLogArea,
                    findings.len() - 1,
                    0,
                    0,
                    lba,
                );
            }
        }
    }

    // Bitmap versus accounting: allocated ⟺ reserved ∪ reachable ∪ retired.
    // Build the sparse expected set, verify every expected block directly,
    // then scan set bits byte-wise. This remains exhaustive without one loop
    // iteration per logical LBA on mostly-free multi-terabyte volumes.
    observer.phase(VerifyPhase::BitmapAccounting, 0);
    let mut accounted = BTreeSet::new();
    for region in 0..geo.region_count() {
        let reserved = geo.region_reserved_blocks(region)
            + if region == 0 {
                afsplus_format::geometry::BOOTSTRAP_BLOCKS
            } else {
                0
            };
        let base = geo.region_base(region);
        accounted.extend((0..reserved).map(|offset| base + offset));
    }
    accounted.extend(state.metadata_blocks.iter().copied());
    accounted.extend(state.data_blocks.iter().copied());
    accounted.extend(state.snapshot_owned_blocks.iter().copied());
    accounted.extend(state.allocation_pool_blocks.iter().copied());
    accounted.extend(state.log_area_blocks.iter().copied());
    for run in &state.reclaim_runs {
        accounted.extend(run.start..run.start + run.blocks as u64);
    }
    for lba in &accounted {
        if !state.bitmaps.is_allocated(*lba) {
            findings.push(format!(
                "block {lba} is marked FREE but reachable from this checkpoint"
            ));
            observer.finding(FindingKind::ReachableFree, findings.len() - 1, 0, 0, *lba);
        }
    }
    for (region, pages) in state.bitmaps.pages.iter().enumerate() {
        let base = geo.region_base(region as u32);
        for page in pages {
            for (byte_index, raw) in page.bits.iter().copied().enumerate() {
                let mut set_bits = raw;
                while set_bits != 0 {
                    let bit = set_bits.trailing_zeros();
                    let local_index = byte_index as u32 * 8 + bit;
                    if local_index < page.valid_blocks {
                        let lba = base + page.first_block as u64 + local_index as u64;
                        if !accounted.contains(&lba) {
                            findings.push(format!(
                                "block {lba} is allocated but owned by nothing (leak)"
                            ));
                            observer.finding(
                                FindingKind::Leak,
                                findings.len() - 1,
                                region as u32,
                                0,
                                lba,
                            );
                        }
                    }
                    set_bits &= set_bits - 1;
                }
            }
        }
    }

    // Checkpoint free counts must match the pages (already enforced on load;
    // kept here as a cheap cross-check for states built by other writers).
    observer.phase(VerifyPhase::FreeCounts, 0);
    let mut free_total = 0u64;
    for (r, record) in state.allocation_records.iter().enumerate() {
        let counted: u32 = state.bitmaps.pages[r]
            .iter()
            .map(BitmapPage::free_blocks)
            .sum();
        if counted != record.free_blocks {
            findings.push(format!(
                "region {r} free count drift: bitmap {counted}, checkpoint {}",
                record.free_blocks
            ));
            observer.finding(
                FindingKind::RegionFreeCount,
                findings.len() - 1,
                r as u32,
                0,
                0,
            );
        }
        if record.descriptor_slot >= DESCRIPTOR_SLOTS {
            findings.push(format!(
                "region {r} references invalid descriptor slot {}",
                record.descriptor_slot
            ));
            observer.finding(
                FindingKind::DescriptorSlot,
                findings.len() - 1,
                r as u32,
                0,
                0,
            );
        }
        free_total += record.free_blocks as u64;
    }
    if free_total != checkpoint.free_blocks_total {
        findings.push(format!(
            "checkpoint free total {} does not match allocation root {free_total}",
            checkpoint.free_blocks_total
        ));
        observer.finding(FindingKind::FreeTotal, findings.len() - 1, 0, 0, 0);
    }

    findings
}
