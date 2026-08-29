//! Shared reachable-state validation (ADR-015: the filesystem and repair
//! tools use the same validation code).
//!
//! `validate_checkpoint_reachable` is what mount uses to decide whether a
//! checkpoint candidate is usable, and what `afsplus-check` builds on for the
//! full invariant sweep. It walks every structure reachable from a checkpoint
//! and validates decoding, bounds, and cross-references.

use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::DirBlock;
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::omap::ObjectMap;
use afsplus_format::{OBJECT_FIRST_DYNAMIC, OBJECT_ROOT};

use crate::CoreError;

/// Everything reachable from one committed checkpoint, fully decoded and
/// cross-validated.
pub struct ReachableState {
    pub object_map: ObjectMap,
    pub objects: BTreeMap<u64, ObjectRecord>,
    /// Directory blocks keyed by owning directory object ID.
    pub directories: BTreeMap<u64, DirBlock>,
    /// LBA of every reachable metadata block (object map, object records,
    /// directory blocks), excluding the checkpoint slot itself.
    pub reachable_blocks: Vec<u64>,
}

/// Walks and validates all state reachable from `checkpoint`.
///
/// This enforces the walk-level subset of `spec/invariants.md`:
/// - all reachable blocks are inside the metadata area and below the
///   checkpoint's allocation high-water mark and the volume bounds
/// - no two reachable structures own the same physical block
/// - the root object exists, is a directory, and object IDs are consistent
/// - every directory entry references an existing object of the hinted type
/// - dynamic object IDs are below the checkpoint's `next_object_id`
pub fn validate_checkpoint_reachable<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
) -> Result<ReachableState, CoreError> {
    let block_size = dev.block_size();
    let mut buf = vec![0u8; block_size];

    if checkpoint.next_free_block > ident.total_blocks {
        return Err(CoreError::Corrupt("allocation high-water mark beyond volume".into()));
    }
    let in_bounds = |lba: u64| -> Result<(), CoreError> {
        if lba < ident.metadata_start || lba >= checkpoint.next_free_block {
            return Err(CoreError::Corrupt(format!(
                "reachable block {lba} outside committed metadata area [{}, {})",
                ident.metadata_start, checkpoint.next_free_block
            )));
        }
        Ok(())
    };

    let mut reachable_blocks = Vec::new();
    let claim = |lba: u64, reachable: &mut Vec<u64>| -> Result<(), CoreError> {
        if reachable.contains(&lba) {
            return Err(CoreError::Corrupt(format!("block {lba} referenced twice")));
        }
        reachable.push(lba);
        Ok(())
    };

    if checkpoint.root_object_id != OBJECT_ROOT {
        return Err(CoreError::Corrupt("checkpoint root object is not the root ID".into()));
    }

    in_bounds(checkpoint.object_map_block)?;
    claim(checkpoint.object_map_block, &mut reachable_blocks)?;
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
        in_bounds(entry.block)?;
        claim(entry.block, &mut reachable_blocks)?;
        dev.read_block(entry.block, &mut buf)?;
        let record = ObjectRecord::decode(&buf)?;
        if record.object_id != entry.object_id {
            return Err(CoreError::Corrupt(format!(
                "object record at block {} claims ID {}, map says {}",
                entry.block, record.object_id, entry.object_id
            )));
        }
        if record.object_type == ObjectType::Directory {
            in_bounds(record.data_root)?;
            claim(record.data_root, &mut reachable_blocks)?;
            dev.read_block(record.data_root, &mut buf)?;
            let dir = DirBlock::decode(&buf)?;
            if dir.owner != record.object_id {
                return Err(CoreError::Corrupt(format!(
                    "directory block at {} owned by {}, expected {}",
                    record.data_root, dir.owner, record.object_id
                )));
            }
            directories.insert(record.object_id, dir);
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

    // Link-count invariant (`spec/invariants.md`, objects): live hard-link
    // count matches reachable directory references; the root carries one
    // implicit reference as the tree anchor.
    let mut ref_counts: BTreeMap<u64, u32> = BTreeMap::new();
    ref_counts.insert(OBJECT_ROOT, 1);
    for dir in directories.values() {
        for entry in &dir.entries {
            *ref_counts.entry(entry.child_id).or_insert(0) += 1;
        }
    }
    for (id, record) in &objects {
        let expected = ref_counts.get(id).copied().unwrap_or(0);
        if record.link_count != expected {
            return Err(CoreError::Corrupt(format!(
                "object {id} link count {} does not match {} reachable references",
                record.link_count, expected
            )));
        }
    }

    Ok(ReachableState { object_map, objects, directories, reachable_blocks })
}
