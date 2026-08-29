//! Committed-state loading and full invariant verification, shared by the
//! core and the checker (ADR-015) but with distinct roles:
//!
//! - [`load_committed_state`] decodes everything the chosen checkpoint
//!   references, with per-structure validation, bounds checks, and unique
//!   block ownership. Normal mount uses it *after* checkpoint selection; a
//!   failure here is reported as corruption — mount never masks it by
//!   silently falling back to an older checkpoint. (The prototype loads the
//!   whole state eagerly; a real implementation loads on demand. Selection
//!   itself never walks the filesystem.)
//! - [`full_sweep`] is the checker/shadow-verification layer: link counts,
//!   orphaned objects, bitmap-versus-reachability equality, retired-list
//!   quarantine invariants, reserved-bit checks.

use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::{comparison_key, DirBlock};
use afsplus_format::geometry::{Geometry, BITMAP_SLOTS};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::omap::ObjectMap;
use afsplus_format::retired::RetiredList;
use afsplus_format::{OBJECT_FIRST_DYNAMIC, OBJECT_ROOT};

use crate::alloc::Bitmaps;
use crate::CoreError;

/// Everything reachable from one committed checkpoint, fully decoded.
pub struct CommittedState {
    pub object_map: ObjectMap,
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
            return Err(CoreError::Corrupt(format!("block {lba} outside allocatable bounds")));
        }
        if !claimed.insert(lba) {
            return Err(CoreError::Corrupt(format!("block {lba} referenced twice")));
        }
        Ok(())
    };

    claim(checkpoint.object_map_block, &mut claimed)?;
    metadata_blocks.push(checkpoint.object_map_block);
    dev.read_block(checkpoint.object_map_block, &mut buf)?;
    let object_map = ObjectMap::decode(&buf)?;

    let mut objects = BTreeMap::new();
    let mut directories = BTreeMap::new();

    for entry in &object_map.entries {
        if entry.object_id >= checkpoint.next_object_id {
            return Err(CoreError::Corrupt(format!(
                "object {} at or above next_object_id {}",
                entry.object_id, checkpoint.next_object_id
            )));
        }
        if entry.object_id != OBJECT_ROOT && entry.object_id < OBJECT_FIRST_DYNAMIC {
            return Err(CoreError::Corrupt(format!(
                "object {} in reserved internal ID range",
                entry.object_id
            )));
        }
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
                for lba in record.data_root..record.data_root + record.data_blocks {
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
            return Err(CoreError::Corrupt(format!("retired block {} out of bounds", entry.lba)));
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
    }

    // Bitmap versus accounting: allocated ⟺ reserved ∪ reachable ∪ retired.
    for lba in 0..geo.total_blocks {
        let allocated = state.bitmaps.is_allocated(lba);
        let accounted = geo.is_reserved(lba)
            || state.metadata_blocks.contains(&lba)
            || state.data_blocks.contains(&lba)
            || state.retired.contains(lba);
        if allocated && !accounted {
            findings.push(format!("block {lba} is allocated but owned by nothing (leak)"));
        }
        if !allocated && accounted {
            findings.push(format!(
                "block {lba} is marked FREE but reachable from this checkpoint"
            ));
        }
    }

    // Checkpoint free counts must match the pages (already enforced on load;
    // kept here as a cheap cross-check for states built by other writers).
    for (r, record) in checkpoint.regions.iter().enumerate() {
        let counted = state.bitmaps.pages[r].free_blocks();
        if counted != record.free_blocks {
            findings.push(format!(
                "region {r} free count drift: bitmap {counted}, checkpoint {}",
                record.free_blocks
            ));
        }
        if record.slot >= BITMAP_SLOTS {
            findings.push(format!("region {r} references invalid bitmap slot {}", record.slot));
        }
    }

    findings
}
