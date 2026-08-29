//! Committed-state loading and full invariant verification, shared by the
//! core and the checker (ADR-015) but with distinct roles:
//!
//! - [`load_mount_state`] reads only the object-map root, root object/root
//!   directory, and the bounded retired-list root. Normal mount uses this
//!   path and never walks every object or allocation bitmap.
//! - [`load_committed_state`] decodes everything the chosen checkpoint
//!   references, with per-structure validation, bounds checks, and unique
//!   block ownership. The checker and explicit shadow verification use it.
//! - [`full_sweep`] is the checker/shadow-verification layer: link counts,
//!   orphaned objects, bitmap-versus-reachability equality, retired-list
//!   quarantine invariants, reserved-bit checks.

use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::BlockDevice;
use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::{comparison_key, DirBlock};
use afsplus_format::geometry::{Geometry, DESCRIPTOR_SLOTS};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::retired::RetiredList;
use afsplus_format::{OBJECT_FIRST_DYNAMIC, OBJECT_ROOT};

use crate::alloc::Bitmaps;
use crate::allocation_root;
use crate::object_map::{self, LoadedObjectMap};
use crate::CoreError;

/// Everything reachable from one committed checkpoint, fully decoded.
pub struct CommittedState {
    pub object_map: LoadedObjectMap,
    pub allocation_records: Vec<afsplus_format::checkpoint::RegionRecord>,
    pub allocation_pool_blocks: Vec<u64>,
    pub objects: BTreeMap<u64, ObjectRecord>,
    /// Directory blocks keyed by owning directory object ID.
    pub directories: BTreeMap<u64, DirBlock>,
    pub retired: RetiredList,
    pub bitmaps: Bitmaps,
    /// Every reachable metadata block (object map, records, directory
    /// blocks, retired list) — excludes reserved blocks and file data.
    pub metadata_blocks: Vec<u64>,
    /// Every reachable file-data block.
    pub data_blocks: Vec<u64>,
}

/// Bounded state needed to expose a mounted root namespace. The object map is
/// already a tree and is descended on demand; the directory remains one
/// prototype page until its own migration.
pub struct MountState {
    pub root_record_lba: u64,
    pub root_object: ObjectRecord,
    pub root_directory: DirBlock,
    pub retired: RetiredList,
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

    claim_root(checkpoint.object_map_block, &mut roots)?;
    let root_record_lba = object_map::lookup_lba(
        dev,
        &geo,
        checkpoint.object_map_block,
        checkpoint.generation,
        OBJECT_ROOT,
    )?
    .ok_or_else(|| CoreError::Corrupt("root object missing from object map".into()))?;
    claim_root(root_record_lba, &mut roots)?;
    dev.read_block(root_record_lba, &mut buf)?;
    let root_object = ObjectRecord::decode(&buf)?;
    if root_object.object_id != OBJECT_ROOT || root_object.object_type != ObjectType::Directory {
        return Err(CoreError::Corrupt(
            "root object is not the root directory".into(),
        ));
    }

    claim_root(root_object.data_root, &mut roots)?;
    dev.read_block(root_object.data_root, &mut buf)?;
    let root_directory = DirBlock::decode(&buf)?;
    if root_directory.owner != OBJECT_ROOT {
        return Err(CoreError::Corrupt(format!(
            "root directory block owned by {}, expected {OBJECT_ROOT}",
            root_directory.owner
        )));
    }
    for entry in &root_directory.entries {
        if entry.key != comparison_key(&entry.name) {
            return Err(CoreError::Corrupt(
                "root directory entry key does not match its name".into(),
            ));
        }
        if !matches!(entry.child_type_hint, 1 | 2) {
            return Err(CoreError::Corrupt(format!(
                "root directory has invalid type hint for object {}",
                entry.child_id
            )));
        }
    }

    let retired = if checkpoint.retired_list_block != 0 {
        claim_root(checkpoint.retired_list_block, &mut roots)?;
        dev.read_block(checkpoint.retired_list_block, &mut buf)?;
        RetiredList::decode(&buf)?
    } else {
        RetiredList::default()
    };
    for entry in &retired.entries {
        if !geo.is_allocatable(entry.lba) {
            return Err(CoreError::Corrupt(format!(
                "retired block {} out of bounds",
                entry.lba
            )));
        }
        if entry.retire_generation > checkpoint.generation {
            return Err(CoreError::Corrupt(format!(
                "retired block {} from future generation {}",
                entry.lba, entry.retire_generation
            )));
        }
        if roots.contains(&entry.lba) {
            return Err(CoreError::Corrupt(format!(
                "retired block {} is still a mounted root",
                entry.lba
            )));
        }
    }

    Ok(MountState {
        root_record_lba,
        root_object,
        root_directory,
        retired,
    })
}

fn validate_mapped_object_id(object_id: u64, checkpoint: &Checkpoint) -> Result<(), CoreError> {
    if object_id >= checkpoint.next_object_id {
        return Err(CoreError::Corrupt(format!(
            "object {object_id} at or above next_object_id {}",
            checkpoint.next_object_id
        )));
    }
    if object_id != OBJECT_ROOT && object_id < OBJECT_FIRST_DYNAMIC {
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

    for entry in &object_map.entries {
        validate_mapped_object_id(entry.object_id, checkpoint)?;
        claim(entry.block, &mut claimed)?;
        metadata_blocks.push(entry.block);
        dev.read_block(entry.block, &mut buf)?;
        let record = ObjectRecord::decode(&buf)?;
        if record.object_id != entry.object_id {
            return Err(CoreError::Corrupt(format!(
                "object record at block {} claims ID {}, map says {}",
                entry.block, record.object_id, entry.object_id
            )));
        }
        match record.object_type {
            ObjectType::Directory => {
                claim(record.data_root, &mut claimed)?;
                metadata_blocks.push(record.data_root);
                dev.read_block(record.data_root, &mut buf)?;
                let dir = DirBlock::decode(&buf)?;
                if dir.owner != record.object_id {
                    return Err(CoreError::Corrupt(format!(
                        "directory block at {} owned by {}, expected {}",
                        record.data_root, dir.owner, record.object_id
                    )));
                }
                for dir_entry in &dir.entries {
                    // Hardening: the stored comparison key must be exactly
                    // what the key encoder derives from the original name.
                    if dir_entry.key != comparison_key(&dir_entry.name) {
                        return Err(CoreError::Corrupt(format!(
                            "directory {} entry key does not match its name",
                            record.object_id
                        )));
                    }
                }
                directories.insert(record.object_id, dir);
            }
            ObjectType::File => {
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
            _ => unreachable!("rejected by ObjectRecord::decode"),
        }
        objects.insert(record.object_id, record);
    }

    let root = objects
        .get(&OBJECT_ROOT)
        .ok_or_else(|| CoreError::Corrupt("root object missing from object map".into()))?;
    if root.object_type != ObjectType::Directory {
        return Err(CoreError::Corrupt("root object is not a directory".into()));
    }

    for (dir_id, dir) in &directories {
        for entry in &dir.entries {
            let child = objects.get(&entry.child_id).ok_or_else(|| {
                CoreError::Corrupt(format!(
                    "directory {dir_id} references missing object {}",
                    entry.child_id
                ))
            })?;
            let hint_matches = matches!(
                (entry.child_type_hint, child.object_type),
                (1, ObjectType::File) | (2, ObjectType::Directory)
            );
            if !hint_matches {
                return Err(CoreError::Corrupt(format!(
                    "directory {dir_id} type hint mismatch for object {}",
                    entry.child_id
                )));
            }
        }
    }

    let retired = if checkpoint.retired_list_block != 0 {
        claim(checkpoint.retired_list_block, &mut claimed)?;
        metadata_blocks.push(checkpoint.retired_list_block);
        dev.read_block(checkpoint.retired_list_block, &mut buf)?;
        RetiredList::decode(&buf)?
    } else {
        RetiredList::default()
    };
    for entry in &retired.entries {
        if claimed.contains(&entry.lba) {
            return Err(CoreError::Corrupt(format!(
                "retired block {} is still reachable",
                entry.lba
            )));
        }
    }
    for entry in &retired.entries {
        if !geo.is_allocatable(entry.lba) {
            return Err(CoreError::Corrupt(format!(
                "retired block {} out of bounds",
                entry.lba
            )));
        }
        if entry.retire_generation > checkpoint.generation {
            return Err(CoreError::Corrupt(format!(
                "retired block {} from future generation {}",
                entry.lba, entry.retire_generation
            )));
        }
    }

    let bitmaps = Bitmaps::load(dev, &geo, checkpoint)?;

    Ok(CommittedState {
        object_map,
        allocation_records: allocation.records,
        allocation_pool_blocks: allocation_layout.pool_lbas,
        objects,
        directories,
        retired,
        bitmaps,
        metadata_blocks,
        data_blocks,
    })
}

/// Full invariant sweep over a loaded state (`spec/invariants.md`).
/// Returns findings instead of failing fast so the checker can report all of
/// them. Normal mount does not run this.
pub fn full_sweep(state: &CommittedState, geo: &Geometry, checkpoint: &Checkpoint) -> Vec<String> {
    let mut findings = Vec::new();

    // Link counts: live references match, and nothing dangles unreferenced.
    let mut ref_counts: BTreeMap<u64, u32> = BTreeMap::new();
    ref_counts.insert(OBJECT_ROOT, 1);
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
        }
        if expected == 0 && *id != OBJECT_ROOT {
            findings.push(format!(
                "object {id} remains in the object map with no directory reference"
            ));
        }
    }

    // Retired entries must be quarantined: allocated bit set, unreachable.
    for entry in &state.retired.entries {
        if !state.bitmaps.is_allocated(entry.lba) {
            findings.push(format!("retired block {} is marked free", entry.lba));
        }
        if state.metadata_blocks.contains(&entry.lba) || state.data_blocks.contains(&entry.lba) {
            findings.push(format!("retired block {} is still reachable", entry.lba));
        }
        if state.allocation_pool_blocks.contains(&entry.lba) {
            findings.push(format!(
                "retired block {} belongs to the permanent allocation-root pool",
                entry.lba
            ));
        }
    }

    // Bitmap versus accounting: allocated ⟺ reserved ∪ reachable ∪ retired.
    for lba in 0..geo.total_blocks {
        let allocated = state.bitmaps.is_allocated(lba);
        let accounted = geo.is_reserved(lba)
            || state.metadata_blocks.contains(&lba)
            || state.data_blocks.contains(&lba)
            || state.allocation_pool_blocks.contains(&lba)
            || state.retired.contains(lba);
        if allocated && !accounted {
            findings.push(format!(
                "block {lba} is allocated but owned by nothing (leak)"
            ));
        }
        if !allocated && accounted {
            findings.push(format!(
                "block {lba} is marked FREE but reachable from this checkpoint"
            ));
        }
    }

    // Checkpoint free counts must match the pages (already enforced on load;
    // kept here as a cheap cross-check for states built by other writers).
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
        }
        if record.descriptor_slot >= DESCRIPTOR_SLOTS {
            findings.push(format!(
                "region {r} references invalid descriptor slot {}",
                record.descriptor_slot
            ));
        }
        free_total += record.free_blocks as u64;
    }
    if free_total != checkpoint.free_blocks_total {
        findings.push(format!(
            "checkpoint free total {} does not match allocation root {free_total}",
            checkpoint.free_blocks_total
        ));
    }

    findings
}
