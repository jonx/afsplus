//! Mounted volume: COW transactions over the region allocator.
//!
//! Commit ordering (`docs/08-transactions-and-journal.md` §3):
//!
//! 1. write new user data, barrier (skipped when the transaction has none)
//! 2. write COW metadata and the dirty region bitmap pages, barrier
//! 3. write the alternate checkpoint slot with generation + 1, barrier
//!
//! Every transaction retires the blocks it makes unreachable (replaced
//! object-map paths, old records, old directory paths, old retired list,
//! deleted data) and promotes the previous transaction's retirees; see
//! `alloc`.
//!
//! Per-transaction resource accounting is collected from the start
//! ([`CommitStats`]) — metadata bytes, bitmap pages, region descriptors,
//! flushes, retired and promoted blocks, reclaim latency, allocator RAM.

use std::collections::BTreeSet;

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::{comparison_key, DirEntry};
use afsplus_format::ident::Identification;
use afsplus_format::object::{
    ObjectRecord, ObjectType, MAX_EXTENT_BLOCKS, OBJECT_FLAG_EXTENT_TREE,
};
use afsplus_format::retired::RetiredList;
use afsplus_format::{validate_name, Timespec, OBJECT_ROOT};

use crate::alloc::{AllocStats, TxAllocator};
use crate::allocation_root::{self, ReservedTreePool};
use crate::cow_tree::{mutate_many, TreeMutation, TreeOperation};
use crate::directory;
use crate::extent_map::{self, Extent, EXTENT_UNWRITTEN};
use crate::mount::Selection;
use crate::object_map;
use crate::verify::{load_mount_state, MountState};
use crate::CoreError;

/// Measured cost of the last committed transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitStats {
    pub data_blocks_written: u64,
    /// COW metadata blocks (records, directories, object map, retired list).
    pub metadata_blocks_written: u64,
    pub bitmap_pages_written: u64,
    pub region_descriptors_written: u64,
    /// Allocation-region records changed in the authoritative AFST root.
    pub allocation_records_updated: u64,
    /// COW allocation-root nodes written for those record changes.
    pub allocation_tree_nodes_written: u64,
    pub checkpoint_blocks_written: u64,
    pub flushes: u64,
    /// Total bytes issued to the device by this transaction.
    pub bytes_written: u64,
    pub alloc: AllocStats,
}

pub struct Volume<D: BlockDevice> {
    dev: D,
    ident: Identification,
    checkpoint: Checkpoint,
    /// Which checkpoint slot holds the committed state (0 = A, 1 = B).
    current_slot: usize,
    /// The other slot's structurally valid checkpoint, if any: its bitmap
    /// slots and quarantined blocks must stay untouched.
    other_checkpoint: Option<Checkpoint>,
    state: MountState,
    last_commit: Option<CommitStats>,
}

impl<D: BlockDevice> Volume<D> {
    pub(crate) fn new(
        dev: D,
        ident: Identification,
        selection: Selection,
        state: MountState,
    ) -> Self {
        Volume {
            dev,
            ident,
            checkpoint: selection.chosen,
            current_slot: selection.chosen_slot,
            other_checkpoint: selection.other,
            state,
            last_commit: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.checkpoint.generation
    }

    pub fn ident(&self) -> &Identification {
        &self.ident
    }

    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    pub fn retired(&self) -> &RetiredList {
        &self.state.retired
    }

    pub fn free_blocks(&self) -> u64 {
        self.checkpoint.free_blocks_total
    }

    /// Peak bitmap bytes resident during the most recent transaction (zero
    /// before any mutation on this mount).
    pub fn allocator_ram_bytes(&self) -> usize {
        self.last_commit
            .map(|stats| stats.alloc.allocator_ram_bytes as usize)
            .unwrap_or(0)
    }

    pub fn last_commit_stats(&self) -> Option<CommitStats> {
        self.last_commit
    }

    /// Looks a name up in the root directory.
    pub fn lookup_root(&mut self, name: &str) -> Result<Option<u64>, CoreError> {
        self.lookup_in_directory(OBJECT_ROOT, name)
    }

    /// Looks a name up in a directory identified by its stable object ID.
    pub fn lookup_in_directory(
        &mut self,
        directory_id: u64,
        name: &str,
    ) -> Result<Option<u64>, CoreError> {
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let directory_record = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        if directory_record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let key = comparison_key(name.as_bytes());
        Ok(directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            directory_record.data_root,
            directory_id,
            self.checkpoint.generation,
            &key,
        )?
        .map(|entry| entry.child_id))
    }

    /// Lists the root directory as (original name, object ID) pairs.
    pub fn list_root(&mut self) -> Result<Vec<(String, u64)>, CoreError> {
        self.list_directory(OBJECT_ROOT)
    }

    /// Lists a directory as `(original name, object ID)` pairs.
    pub fn list_directory(&mut self, directory_id: u64) -> Result<Vec<(String, u64)>, CoreError> {
        let directory_record = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        if directory_record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        Ok(directory::load_all(
            &mut self.dev,
            &self.ident.geometry(),
            directory_record.data_root,
            directory_id,
            self.checkpoint.generation,
        )?
        .entries
        .iter()
        .map(|e| (String::from_utf8_lossy(&e.name).into_owned(), e.child_id))
        .collect())
    }

    /// Reads and validates one object record on demand. Returning the record
    /// by value keeps the low-memory path independent of a mandatory object
    /// cache; modern implementations may add a bounded or aggressive cache
    /// above this API.
    pub fn stat(&mut self, object_id: u64) -> Result<Option<ObjectRecord>, CoreError> {
        self.read_object(object_id)
    }

    /// Reads a file's committed content.
    pub fn read_file(&mut self, object_id: u64) -> Result<Vec<u8>, CoreError> {
        let record = self
            .read_object(object_id)?
            .ok_or_else(|| CoreError::Corrupt(format!("no object {object_id}")))?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::Corrupt(format!(
                "object {object_id} is not a file"
            )));
        }
        let block_size = self.dev.block_size();
        let content_len = usize::try_from(record.size_bytes)
            .map_err(|_| CoreError::PrototypeLimit("file is too large to read into one buffer"))?;
        let mut content = vec![0u8; content_len];
        let logical_blocks = record.size_bytes.div_ceil(block_size as u64);
        let mut block = vec![0u8; block_size];
        for logical_block in 0..logical_blocks {
            let lba = if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
                let Some(extent) = extent_map::lookup_extent(
                    &mut self.dev,
                    &self.ident.geometry(),
                    record.data_root,
                    object_id,
                    self.checkpoint.generation,
                    logical_block,
                )?
                else {
                    continue;
                };
                if extent.flags & EXTENT_UNWRITTEN != 0 {
                    continue;
                }
                extent.physical_start + logical_block - extent.logical_start
            } else {
                record.data_root.checked_add(logical_block).ok_or_else(|| {
                    CoreError::Corrupt(format!("object {object_id} extent overflow"))
                })?
            };
            self.dev.read_block(lba, &mut block)?;
            let offset = logical_block as usize * block_size;
            let length = block_size.min(content.len() - offset);
            content[offset..offset + length].copy_from_slice(&block[..length]);
        }
        Ok(content)
    }

    /// Replaces `content.len()` bytes at `offset` using fresh data blocks and
    /// one atomic COW metadata publication. Writing beyond EOF creates a hole;
    /// the file is converted from its cheap direct extent to an AFST extent
    /// map only when the resulting layout is sparse or fragmented.
    pub fn write_file_at(
        &mut self,
        object_id: u64,
        offset: u64,
        content: &[u8],
        now: Timespec,
    ) -> Result<(), CoreError> {
        if content.is_empty() {
            return Ok(());
        }
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {object_id} missing from object map"))
        })?;
        let content_len = u64::try_from(content.len())
            .map_err(|_| CoreError::PrototypeLimit("write buffer is too large"))?;
        let end_offset = offset
            .checked_add(content_len)
            .ok_or(CoreError::PrototypeLimit("file size limit reached"))?;
        let block_size = self.dev.block_size() as u64;
        let first_block = offset / block_size;
        let end_block = end_offset.div_ceil(block_size);
        let write_block_count = end_block - first_block;
        let (old_extents, old_tree_blocks) = self.load_file_layout(&record)?;

        // Partial first/last blocks inherit their committed bytes. Holes and
        // unwritten preallocation read as zeros, so they need no special case.
        let mut blocks = Vec::with_capacity(write_block_count as usize);
        for logical_block in first_block..end_block {
            let mut block = vec![0u8; block_size as usize];
            self.read_layout_block(&old_extents, logical_block, &mut block)?;
            let logical_byte = logical_block.saturating_mul(block_size);
            let copy_start = offset.max(logical_byte);
            let copy_end = end_offset.min(logical_byte.saturating_add(block_size));
            let source_start = usize::try_from(copy_start - offset)
                .map_err(|_| CoreError::PrototypeLimit("write offset is too large"))?;
            let source_end = usize::try_from(copy_end - offset)
                .map_err(|_| CoreError::PrototypeLimit("write offset is too large"))?;
            let target_start = usize::try_from(copy_start - logical_byte)
                .map_err(|_| CoreError::PrototypeLimit("block offset is too large"))?;
            block[target_start..target_start + source_end - source_start]
                .copy_from_slice(&content[source_start..source_end]);
            blocks.push(block);
        }

        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;
        let additions = allocate_extent_runs(
            &mut tx,
            &mut self.dev,
            &self.ident.geometry(),
            first_block,
            write_block_count,
            0,
        )?;
        let mut block_iter = blocks.into_iter();
        let mut data_writes = Vec::with_capacity(write_block_count as usize);
        for extent in &additions {
            for offset in 0..extent.block_count {
                let block = block_iter.next().ok_or_else(|| {
                    CoreError::Corrupt("write extent/data block count mismatch".into())
                })?;
                data_writes.push((extent.physical_start + offset, block));
            }
        }
        if block_iter.next().is_some() {
            return Err(CoreError::Corrupt(
                "write extent/data block count mismatch".into(),
            ));
        }
        let (mut new_extents, removed_extents) =
            replace_logical_range(&old_extents, first_block, end_block, None)?;
        new_extents.extend(additions);
        let new_extents = coalesce_extents(new_extents)?;

        self.commit_file_layout(
            record,
            record_lba,
            old_extents,
            old_tree_blocks,
            new_extents,
            removed_extents,
            record.size_bytes.max(end_offset),
            true,
            now,
            generation,
            tx,
            data_writes,
        )
    }

    /// Changes a file's logical size atomically. Growth creates zero-reading
    /// holes. Shrinking a written partial block COW-rewrites that block with a
    /// zeroed tail so a later extension cannot reveal bytes past the old EOF.
    pub fn truncate_file(
        &mut self,
        object_id: u64,
        new_size: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        if new_size == record.size_bytes {
            return Ok(());
        }
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {object_id} missing from object map"))
        })?;
        let block_size = self.dev.block_size() as u64;
        let (old_extents, old_tree_blocks) = self.load_file_layout(&record)?;
        let mut new_extents = old_extents.clone();
        let mut removed_extents = Vec::new();
        let mut tail_rewrite = None;

        if new_size < record.size_bytes {
            let retained_blocks = new_size.div_ceil(block_size);
            (new_extents, removed_extents) =
                replace_logical_range(&old_extents, retained_blocks, u64::MAX, None)?;
            if !new_size.is_multiple_of(block_size) {
                let logical_block = retained_blocks - 1;
                if extent_at(&new_extents, logical_block)
                    .is_some_and(|extent| extent.flags & EXTENT_UNWRITTEN == 0)
                {
                    let mut block = vec![0u8; block_size as usize];
                    self.read_layout_block(&new_extents, logical_block, &mut block)?;
                    block[(new_size % block_size) as usize..].fill(0);
                    tail_rewrite = Some((logical_block, block));
                }
            }
        }

        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;
        let mut data_writes = Vec::new();
        if let Some((logical_block, block)) = tail_rewrite {
            let physical_start = tx.allocate(&mut self.dev)?;
            let replacement = Extent {
                logical_start: logical_block,
                physical_start,
                block_count: 1,
                flags: 0,
            };
            let (rewritten, mut removed) = replace_logical_range(
                &new_extents,
                logical_block,
                logical_block + 1,
                Some(replacement),
            )?;
            new_extents = rewritten;
            removed_extents.append(&mut removed);
            data_writes.push((physical_start, block));
        }

        self.commit_file_layout(
            record,
            record_lba,
            old_extents,
            old_tree_blocks,
            new_extents,
            removed_extents,
            new_size,
            true,
            now,
            generation,
            tx,
            data_writes,
        )
    }

    /// Reserves physical blocks for a byte range without changing the file's
    /// logical size. New mappings carry the unwritten flag and therefore read
    /// as zeros until replaced by [`Self::write_file_at`].
    pub fn preallocate_file(
        &mut self,
        object_id: u64,
        offset: u64,
        length: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        if length == 0 {
            return Ok(());
        }
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let end_offset = offset
            .checked_add(length)
            .ok_or(CoreError::PrototypeLimit("preallocation range overflows"))?;
        let block_size = self.dev.block_size() as u64;
        let start_block = offset / block_size;
        let end_block = end_offset.div_ceil(block_size);
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {object_id} missing from object map"))
        })?;
        let (old_extents, old_tree_blocks) = self.load_file_layout(&record)?;
        let holes = logical_holes(&old_extents, start_block, end_block)?;
        if holes.is_empty() {
            return Ok(());
        }

        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;
        let mut additions = Vec::new();
        for (logical_start, logical_end) in holes {
            additions.extend(allocate_extent_runs(
                &mut tx,
                &mut self.dev,
                &self.ident.geometry(),
                logical_start,
                logical_end - logical_start,
                EXTENT_UNWRITTEN,
            )?);
        }
        let mut new_extents = old_extents.clone();
        new_extents.extend(additions);
        let new_extents = coalesce_extents(new_extents)?;
        let logical_size = record.size_bytes;

        self.commit_file_layout(
            record,
            record_lba,
            old_extents,
            old_tree_blocks,
            new_extents,
            Vec::new(),
            logical_size,
            false,
            now,
            generation,
            tx,
            Vec::new(),
        )
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    pub fn into_device(self) -> D {
        self.dev
    }

    /// Creates a file with `content` in the root directory.
    pub fn create_file_in_root(
        &mut self,
        name: &str,
        content: &[u8],
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.create_file_in_directory(OBJECT_ROOT, name, content, now)
    }

    /// Creates a file in an arbitrary directory identified by object ID.
    pub fn create_file_in_directory(
        &mut self,
        parent_id: u64,
        name: &str,
        content: &[u8],
        now: Timespec,
    ) -> Result<u64, CoreError> {
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = comparison_key(name.as_bytes());
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let object_id = self.checkpoint.next_object_id;
        let next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;

        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;

        // Data first: allocate one contiguous extent and stage its blocks.
        let data_block_count = (content.len() as u64).div_ceil(block_size as u64);
        let mut data_writes = Vec::new();
        let data_start = if data_block_count > 0 {
            let start = tx.allocate_run(&mut self.dev, data_block_count)?;
            for i in 0..data_block_count as usize {
                let mut block = vec![0u8; block_size];
                let from = i * block_size;
                let to = content.len().min(from + block_size);
                block[..to - from].copy_from_slice(&content[from..to]);
                data_writes.push((start + i as u64, block));
            }
            start
        } else {
            0
        };

        // Fresh blocks for every COW'd structure.
        let file_record_lba = tx.allocate(&mut self.dev)?;
        let parent_record_new_lba = tx.allocate(&mut self.dev)?;
        let retired_list_lba = tx.allocate(&mut self.dev)?;

        // Everything the new state no longer reaches goes into quarantine.
        tx.retire(&mut self.dev, parent_record_lba)?;
        self.retire_previous_list(&mut tx)?;

        let file_record = ObjectRecord {
            object_id,
            object_type: ObjectType::File,
            flags: 0,
            link_count: 1,
            size_bytes: content.len() as u64,
            allocated_bytes: data_block_count * block_size as u64,
            created: now,
            modified: now,
            changed: now,
            protection: 0,
            content_generation: generation,
            data_root: data_start,
            data_blocks: data_block_count,
        };

        let directory_entry = DirEntry {
            key,
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        };
        let (directory_key, directory_value) = directory::encode_entry(&directory_entry)?;
        let directory_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &directory_key,
                value: &directory_value,
            }],
        )?;

        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: directory_mutation.root_lba,
            ..parent
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let file_key = object_map::key(object_id);
        let file_value = object_map::value(file_record_lba)?;
        let operations = [
            TreeOperation::Upsert {
                key: &parent_key,
                value: &parent_value,
            },
            TreeOperation::Upsert {
                key: &file_key,
                value: &file_value,
            },
        ];
        let omap_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        let omap_lba = omap_mutation.root_lba;

        let mut meta_writes = vec![
            (file_record_lba, file_record.encode(block_size, generation)?),
            (
                parent_record_new_lba,
                new_parent.encode(block_size, generation)?,
            ),
        ];
        meta_writes.extend(directory_mutation.writes);
        meta_writes.extend(omap_mutation.writes);

        self.commit_transaction(
            generation,
            next_object_id,
            tx,
            data_writes,
            meta_writes,
            retired_list_lba,
            omap_lba,
        )?;
        Ok(object_id)
    }

    /// Creates a directory in the root directory.
    pub fn create_directory_in_root(
        &mut self,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        self.create_directory(OBJECT_ROOT, name, now)
    }

    /// Creates an empty directory in an arbitrary parent directory.
    pub fn create_directory(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let object_id = self.checkpoint.next_object_id;
        let next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;

        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;
        let directory_root_lba = tx.allocate(&mut self.dev)?;
        let directory_record_lba = tx.allocate(&mut self.dev)?;
        let parent_record_new_lba = tx.allocate(&mut self.dev)?;
        let retired_list_lba = tx.allocate(&mut self.dev)?;

        tx.retire(&mut self.dev, parent_record_lba)?;
        self.retire_previous_list(&mut tx)?;

        let new_directory = ObjectRecord {
            object_id,
            object_type: ObjectType::Directory,
            flags: 0,
            link_count: 1,
            size_bytes: 0,
            allocated_bytes: 0,
            created: now,
            modified: now,
            changed: now,
            protection: 0,
            content_generation: generation,
            data_root: directory_root_lba,
            data_blocks: 0,
        };

        let entry = DirEntry {
            key: comparison_key(name.as_bytes()),
            name: name.as_bytes().to_vec(),
            child_type_hint: 2,
            child_id: object_id,
        };
        let (directory_key, directory_value) = directory::encode_entry(&entry)?;
        let parent_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &directory_key,
                value: &directory_value,
            }],
        )?;
        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: parent_mutation.root_lba,
            ..parent
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let directory_object_key = object_map::key(object_id);
        let directory_object_value = object_map::value(directory_record_lba)?;
        let operations = [
            TreeOperation::Upsert {
                key: &parent_key,
                value: &parent_value,
            },
            TreeOperation::Upsert {
                key: &directory_object_key,
                value: &directory_object_value,
            },
        ];
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;

        let mut metadata_writes = vec![
            (
                directory_root_lba,
                directory::empty_leaf(object_id).encode(block_size, generation)?,
            ),
            (
                directory_record_lba,
                new_directory.encode(block_size, generation)?,
            ),
            (
                parent_record_new_lba,
                new_parent.encode(block_size, generation)?,
            ),
        ];
        metadata_writes.extend(parent_mutation.writes);
        metadata_writes.extend(object_map_mutation.writes);

        self.commit_transaction(
            generation,
            next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            retired_list_lba,
            object_map_mutation.root_lba,
        )?;
        Ok(object_id)
    }

    /// Deletes a file from the root directory. Its record and data blocks are
    /// retired, not freed: they stay quarantined until no still-selectable
    /// checkpoint can reference them.
    pub fn delete_file_in_root(&mut self, name: &str, now: Timespec) -> Result<(), CoreError> {
        self.delete_file(OBJECT_ROOT, name, now)
    }

    /// Deletes a file link from an arbitrary directory.
    pub fn delete_file(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.remove_entry(parent_id, name, now, ObjectType::File)
    }

    /// Removes an empty child directory from an arbitrary parent.
    pub fn remove_directory(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.remove_entry(parent_id, name, now, ObjectType::Directory)
    }

    fn remove_entry(
        &mut self,
        parent_id: u64,
        name: &str,
        now: Timespec,
        expected_type: ObjectType,
    ) -> Result<(), CoreError> {
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = comparison_key(name.as_bytes());
        let entry = directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            parent.data_root,
            parent_id,
            self.checkpoint.generation,
            &key,
        )?
        .ok_or(CoreError::NotFound)?;
        let victim = self
            .read_object(entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("victim missing from object map".into()))?;
        if !matches!(
            (entry.child_type_hint, victim.object_type),
            (1, ObjectType::File) | (2, ObjectType::Directory)
        ) {
            return Err(CoreError::Corrupt(
                "directory entry type hint does not match victim object".into(),
            ));
        }
        if victim.object_type != expected_type {
            return Err(if victim.object_type == ObjectType::Directory {
                CoreError::IsDirectory
            } else {
                CoreError::NotDirectory
            });
        }
        let victim_directory_blocks = if victim.object_type == ObjectType::Directory {
            if victim.link_count != 1 {
                return Err(CoreError::Corrupt(
                    "directory hard links are not supported".into(),
                ));
            }
            let loaded = directory::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                victim.data_root,
                victim.object_id,
                self.checkpoint.generation,
            )?;
            if !loaded.entries.is_empty() {
                return Err(CoreError::DirectoryNotEmpty);
            }
            loaded.tree_blocks
        } else {
            Vec::new()
        };
        let keep_file_object = victim.object_type == ObjectType::File && victim.link_count > 1;
        let victim_extent_map = if victim.object_type == ObjectType::File
            && victim.flags & OBJECT_FLAG_EXTENT_TREE != 0
            && !keep_file_object
        {
            Some(extent_map::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                victim.data_root,
                victim.object_id,
                self.checkpoint.generation,
            )?)
        } else {
            None
        };

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;

        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;

        let parent_record_new_lba = tx.allocate(&mut self.dev)?;
        let victim_record_new_lba = if keep_file_object {
            Some(tx.allocate(&mut self.dev)?)
        } else {
            None
        };
        let retired_list_lba = tx.allocate(&mut self.dev)?;

        // Quarantine the object's storage and the COW'd originals.
        let victim_record_lba = self
            .object_record_lba(entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("victim missing from object map".into()))?;
        tx.retire(&mut self.dev, victim_record_lba)?;
        if !keep_file_object {
            if let Some(map) = victim_extent_map {
                for lba in map.tree_blocks {
                    tx.retire(&mut self.dev, lba)?;
                }
                for extent in map.extents {
                    for lba in extent.physical_start..extent.physical_end()? {
                        tx.retire(&mut self.dev, lba)?;
                    }
                }
            } else {
                for i in 0..victim.data_blocks {
                    tx.retire(&mut self.dev, victim.data_root + i)?;
                }
            }
            for lba in victim_directory_blocks {
                tx.retire(&mut self.dev, lba)?;
            }
        }
        tx.retire(&mut self.dev, parent_record_lba)?;
        self.retire_previous_list(&mut tx)?;

        let directory_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Delete { key: &key }],
        )?;

        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: directory_mutation.root_lba,
            ..parent
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let victim_key = object_map::key(entry.child_id);
        let victim_value = victim_record_new_lba.map(object_map::value).transpose()?;
        let mut operations = vec![TreeOperation::Upsert {
            key: &parent_key,
            value: &parent_value,
        }];
        if let Some(value) = &victim_value {
            operations.push(TreeOperation::Upsert {
                key: &victim_key,
                value,
            });
        } else {
            operations.push(TreeOperation::Delete { key: &victim_key });
        }
        let omap_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        let omap_lba = omap_mutation.root_lba;

        let mut meta_writes = vec![(
            parent_record_new_lba,
            new_parent.encode(block_size, generation)?,
        )];
        if let Some(lba) = victim_record_new_lba {
            let new_victim = ObjectRecord {
                link_count: victim.link_count - 1,
                changed: now,
                ..victim
            };
            meta_writes.push((lba, new_victim.encode(block_size, generation)?));
        }
        meta_writes.extend(directory_mutation.writes);
        meta_writes.extend(omap_mutation.writes);

        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            meta_writes,
            retired_list_lba,
            omap_lba,
        )
    }

    /// Adds another directory link to an existing regular file and updates
    /// its authoritative link count in the same checkpoint transaction.
    pub fn link_file(
        &mut self,
        object_id: u64,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let file = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if file.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let new_link_count = file
            .link_count
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("link count exhausted"))?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }
        let file_lba = self
            .object_record_lba(object_id)?
            .ok_or_else(|| CoreError::Corrupt("linked file missing from object map".into()))?;
        let parent_lba = self
            .object_record_lba(parent_id)?
            .ok_or_else(|| CoreError::Corrupt("link parent missing from object map".into()))?;

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;
        let file_new_lba = tx.allocate(&mut self.dev)?;
        let parent_new_lba = tx.allocate(&mut self.dev)?;
        let retired_list_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, file_lba)?;
        tx.retire(&mut self.dev, parent_lba)?;
        self.retire_previous_list(&mut tx)?;

        let entry = DirEntry {
            key: comparison_key(name.as_bytes()),
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        };
        let (entry_key, entry_value) = directory::encode_entry(&entry)?;
        let directory_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            parent.data_root,
            directory::spec(parent_id, self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &entry_key,
                value: &entry_value,
            }],
        )?;
        let new_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: directory_mutation.root_lba,
            ..parent
        };
        let new_file = ObjectRecord {
            link_count: new_link_count,
            changed: now,
            ..file
        };

        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_new_lba)?;
        let file_key = object_map::key(object_id);
        let file_value = object_map::value(file_new_lba)?;
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[
                TreeOperation::Upsert {
                    key: &parent_key,
                    value: &parent_value,
                },
                TreeOperation::Upsert {
                    key: &file_key,
                    value: &file_value,
                },
            ],
        )?;

        let mut metadata_writes = vec![
            (file_new_lba, new_file.encode(block_size, generation)?),
            (parent_new_lba, new_parent.encode(block_size, generation)?),
        ];
        metadata_writes.extend(directory_mutation.writes);
        metadata_writes.extend(object_map_mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            retired_list_lba,
            object_map_mutation.root_lba,
        )
    }

    /// Atomically renames or moves one namespace entry without changing its
    /// stable object ID. Replacement of an existing destination is a separate
    /// future operation with an explicit contract.
    pub fn rename(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        validate_name(source_name.as_bytes()).map_err(CoreError::InvalidName)?;
        validate_name(target_name.as_bytes()).map_err(CoreError::InvalidName)?;
        let source_key = comparison_key(source_name.as_bytes());
        let target_key = comparison_key(target_name.as_bytes());

        let source_parent = self
            .read_object(source_parent_id)?
            .ok_or(CoreError::NotFound)?;
        if source_parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let target_parent = if target_parent_id == source_parent_id {
            source_parent
        } else {
            self.read_object(target_parent_id)?
                .ok_or(CoreError::NotFound)?
        };
        if target_parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }

        let source_entry = directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            source_parent.data_root,
            source_parent_id,
            self.checkpoint.generation,
            &source_key,
        )?
        .ok_or(CoreError::NotFound)?;
        if source_parent_id == target_parent_id && source_key == target_key {
            return Ok(());
        }
        if directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            target_parent.data_root,
            target_parent_id,
            self.checkpoint.generation,
            &target_key,
        )?
        .is_some()
        {
            return Err(CoreError::AlreadyExists);
        }
        let moved = self
            .read_object(source_entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("moved object missing from object map".into()))?;
        if !matches!(
            (source_entry.child_type_hint, moved.object_type),
            (1, ObjectType::File) | (2, ObjectType::Directory)
        ) {
            return Err(CoreError::Corrupt(
                "source entry type hint does not match moved object".into(),
            ));
        }
        if moved.object_type == ObjectType::Directory
            && self.directory_reaches(moved.object_id, target_parent_id)?
        {
            return Err(CoreError::InvalidMove(
                "a directory cannot be moved into itself or a descendant",
            ));
        }

        let source_parent_lba = self
            .object_record_lba(source_parent_id)?
            .ok_or_else(|| CoreError::Corrupt("source parent missing from object map".into()))?;
        let target_parent_lba = if target_parent_id == source_parent_id {
            source_parent_lba
        } else {
            self.object_record_lba(target_parent_id)?
                .ok_or_else(|| CoreError::Corrupt("target parent missing from object map".into()))?
        };
        let moved_lba = self
            .object_record_lba(moved.object_id)?
            .ok_or_else(|| CoreError::Corrupt("moved object missing from object map".into()))?;

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            &self.state.retired,
            generation,
        )?;
        let source_parent_new_lba = tx.allocate(&mut self.dev)?;
        let target_parent_new_lba = if target_parent_id == source_parent_id {
            source_parent_new_lba
        } else {
            tx.allocate(&mut self.dev)?
        };
        let moved_new_lba = tx.allocate(&mut self.dev)?;
        let retired_list_lba = tx.allocate(&mut self.dev)?;

        tx.retire(&mut self.dev, source_parent_lba)?;
        if target_parent_id != source_parent_id {
            tx.retire(&mut self.dev, target_parent_lba)?;
        }
        tx.retire(&mut self.dev, moved_lba)?;
        self.retire_previous_list(&mut tx)?;

        let target_entry = DirEntry {
            key: target_key,
            name: target_name.as_bytes().to_vec(),
            child_type_hint: source_entry.child_type_hint,
            child_id: source_entry.child_id,
        };
        let (target_entry_key, target_entry_value) = directory::encode_entry(&target_entry)?;

        let (source_mutation, target_mutation) = if source_parent_id == target_parent_id {
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                source_parent.data_root,
                directory::spec(source_parent_id, self.checkpoint.generation),
                generation,
                &[
                    TreeOperation::Delete { key: &source_key },
                    TreeOperation::Upsert {
                        key: &target_entry_key,
                        value: &target_entry_value,
                    },
                ],
            )?;
            (mutation, None)
        } else {
            let source_mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                source_parent.data_root,
                directory::spec(source_parent_id, self.checkpoint.generation),
                generation,
                &[TreeOperation::Delete { key: &source_key }],
            )?;
            let target_mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                target_parent.data_root,
                directory::spec(target_parent_id, self.checkpoint.generation),
                generation,
                &[TreeOperation::Upsert {
                    key: &target_entry_key,
                    value: &target_entry_value,
                }],
            )?;
            (source_mutation, Some(target_mutation))
        };

        let new_source_parent = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: source_mutation.root_lba,
            ..source_parent
        };
        let new_target_parent = target_mutation.as_ref().map(|mutation| ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: mutation.root_lba,
            ..target_parent
        });
        let new_moved = ObjectRecord {
            changed: now,
            ..moved
        };

        let source_parent_map_key = object_map::key(source_parent_id);
        let source_parent_map_value = object_map::value(source_parent_new_lba)?;
        let target_parent_map_key = object_map::key(target_parent_id);
        let target_parent_map_value = object_map::value(target_parent_new_lba)?;
        let moved_map_key = object_map::key(moved.object_id);
        let moved_map_value = object_map::value(moved_new_lba)?;
        let mut map_operations = vec![TreeOperation::Upsert {
            key: &source_parent_map_key,
            value: &source_parent_map_value,
        }];
        if target_parent_id != source_parent_id {
            map_operations.push(TreeOperation::Upsert {
                key: &target_parent_map_key,
                value: &target_parent_map_value,
            });
        }
        map_operations.push(TreeOperation::Upsert {
            key: &moved_map_key,
            value: &moved_map_value,
        });
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &map_operations,
        )?;

        let mut metadata_writes = vec![
            (
                source_parent_new_lba,
                new_source_parent.encode(block_size, generation)?,
            ),
            (moved_new_lba, new_moved.encode(block_size, generation)?),
        ];
        metadata_writes.extend(source_mutation.writes);
        if let (Some(record), Some(mutation)) = (new_target_parent, target_mutation) {
            metadata_writes.push((
                target_parent_new_lba,
                record.encode(block_size, generation)?,
            ));
            metadata_writes.extend(mutation.writes);
        }
        metadata_writes.extend(object_map_mutation.writes);

        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            metadata_writes,
            retired_list_lba,
            object_map_mutation.root_lba,
        )
    }

    /// Returns whether `target_id` is `ancestor_id` or is reachable below it.
    /// This exhaustive guard is used only for directory moves; a future
    /// parent/reverse index may accelerate it without changing semantics.
    fn directory_reaches(&mut self, ancestor_id: u64, target_id: u64) -> Result<bool, CoreError> {
        let mut pending = vec![ancestor_id];
        let mut visited = BTreeSet::new();
        while let Some(directory_id) = pending.pop() {
            if directory_id == target_id {
                return Ok(true);
            }
            if !visited.insert(directory_id) {
                return Err(CoreError::Corrupt(
                    "directory graph contains a cycle or duplicate parent".into(),
                ));
            }
            let record = self.read_object(directory_id)?.ok_or_else(|| {
                CoreError::Corrupt(format!("directory {directory_id} is missing"))
            })?;
            if record.object_type != ObjectType::Directory {
                return Err(CoreError::Corrupt(format!(
                    "directory graph references non-directory object {directory_id}"
                )));
            }
            let loaded = directory::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                directory_id,
                self.checkpoint.generation,
            )?;
            for entry in loaded.entries {
                if entry.child_type_hint == 2 {
                    let child = self.read_object(entry.child_id)?.ok_or_else(|| {
                        CoreError::Corrupt(format!(
                            "directory {directory_id} references missing object {}",
                            entry.child_id
                        ))
                    })?;
                    if child.object_type != ObjectType::Directory {
                        return Err(CoreError::Corrupt(format!(
                            "directory {directory_id} has a bad type hint for object {}",
                            entry.child_id
                        )));
                    }
                    pending.push(entry.child_id);
                }
            }
        }
        Ok(false)
    }

    fn load_file_layout(
        &mut self,
        record: &ObjectRecord,
    ) -> Result<(Vec<Extent>, Vec<u64>), CoreError> {
        if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            let loaded = extent_map::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                record.object_id,
                self.checkpoint.generation,
            )?;
            Ok((loaded.extents, loaded.tree_blocks))
        } else if record.data_blocks == 0 {
            Ok((Vec::new(), Vec::new()))
        } else {
            Ok((
                vec![Extent {
                    logical_start: 0,
                    physical_start: record.data_root,
                    block_count: record.data_blocks,
                    flags: 0,
                }],
                Vec::new(),
            ))
        }
    }

    fn read_layout_block(
        &mut self,
        extents: &[Extent],
        logical_block: u64,
        block: &mut [u8],
    ) -> Result<(), CoreError> {
        let after = extents.partition_point(|extent| extent.logical_start <= logical_block);
        let Some(extent) = after.checked_sub(1).map(|index| extents[index]) else {
            return Ok(());
        };
        if logical_block >= extent.logical_end()? || extent.flags & EXTENT_UNWRITTEN != 0 {
            return Ok(());
        }
        let lba = extent
            .physical_start
            .checked_add(logical_block - extent.logical_start)
            .ok_or_else(|| CoreError::Corrupt("mapped physical block overflows".into()))?;
        self.dev.read_block(lba, block)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_file_layout(
        &mut self,
        record: ObjectRecord,
        record_lba: u64,
        old_extents: Vec<Extent>,
        old_tree_blocks: Vec<u64>,
        new_extents: Vec<Extent>,
        removed_extents: Vec<Extent>,
        new_size: u64,
        content_changed: bool,
        now: Timespec,
        generation: u64,
        mut tx: TxAllocator,
        data_writes: Vec<(u64, Vec<u8>)>,
    ) -> Result<(), CoreError> {
        let block_size = self.dev.block_size();
        let allocated_blocks = new_extents.iter().try_fold(0u64, |total, extent| {
            total
                .checked_add(extent.block_count)
                .ok_or(CoreError::PrototypeLimit("allocated block count overflow"))
        })?;
        let direct = direct_layout(&new_extents, new_size, block_size as u64);
        let was_tree = record.flags & OBJECT_FLAG_EXTENT_TREE != 0;
        let mut extent_writes = Vec::new();
        let (flags, data_root, data_blocks) = if let Some(extent) = direct {
            if was_tree {
                for lba in old_tree_blocks {
                    tx.retire(&mut self.dev, lba)?;
                }
            }
            (
                0,
                extent.map_or(0, |item| item.physical_start),
                allocated_blocks,
            )
        } else if was_tree && old_extents == new_extents {
            (OBJECT_FLAG_EXTENT_TREE, record.data_root, allocated_blocks)
        } else if was_tree {
            let old_encoded = old_extents
                .iter()
                .copied()
                .map(extent_map::encode_extent)
                .collect::<Result<Vec<_>, _>>()?;
            let new_encoded = new_extents
                .iter()
                .copied()
                .map(extent_map::encode_extent)
                .collect::<Result<Vec<_>, _>>()?;
            let mut operations = Vec::with_capacity(old_encoded.len() + new_encoded.len());
            for (key, _) in &old_encoded {
                operations.push(TreeOperation::Delete { key });
            }
            for (key, value) in &new_encoded {
                operations.push(TreeOperation::Upsert { key, value });
            }
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                record.data_root,
                extent_map::spec(record.object_id, self.checkpoint.generation),
                generation,
                &operations,
            )?;
            extent_writes = mutation.writes;
            (OBJECT_FLAG_EXTENT_TREE, mutation.root_lba, allocated_blocks)
        } else {
            let node_count = extent_map::bulk_node_count(block_size, new_extents.len())?;
            let mut lbas = Vec::with_capacity(node_count);
            for _ in 0..node_count {
                lbas.push(tx.allocate(&mut self.dev)?);
            }
            let built = extent_map::bulk_build(record.object_id, block_size, &new_extents, &lbas)?;
            for (lba, node) in built.nodes {
                extent_writes.push((lba, node.encode(block_size, generation)?));
            }
            (OBJECT_FLAG_EXTENT_TREE, built.root_lba, allocated_blocks)
        };

        let new_record_lba = tx.allocate(&mut self.dev)?;
        let retired_list_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, record_lba)?;
        for extent in removed_extents {
            for lba in extent.physical_start..extent.physical_end()? {
                tx.retire(&mut self.dev, lba)?;
            }
        }
        self.retire_previous_list(&mut tx)?;

        let new_record = ObjectRecord {
            flags,
            size_bytes: new_size,
            allocated_bytes: allocated_blocks
                .checked_mul(block_size as u64)
                .ok_or(CoreError::PrototypeLimit("allocated byte count overflow"))?,
            modified: if content_changed {
                now
            } else {
                record.modified
            },
            changed: now,
            content_generation: if content_changed {
                generation
            } else {
                record.content_generation
            },
            data_root,
            data_blocks,
            ..record
        };
        let object_key = object_map::key(record.object_id);
        let object_value = object_map::value(new_record_lba)?;
        let object_map_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &object_key,
                value: &object_value,
            }],
        )?;
        let mut metadata_writes =
            vec![(new_record_lba, new_record.encode(block_size, generation)?)];
        metadata_writes.extend(extent_writes);
        metadata_writes.extend(object_map_mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            data_writes,
            metadata_writes,
            retired_list_lba,
            object_map_mutation.root_lba,
        )
    }

    fn next_generation(&self) -> Result<u64, CoreError> {
        self.checkpoint
            .generation
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("generation counter exhausted"))
    }

    fn read_object(&mut self, object_id: u64) -> Result<Option<ObjectRecord>, CoreError> {
        if object_id == OBJECT_ROOT {
            return Ok(Some(self.state.root_object));
        }
        let Some(lba) = self.lookup_object_lba(object_id)? else {
            return Ok(None);
        };
        if !self.ident.geometry().is_allocatable(lba) {
            return Err(CoreError::Corrupt(format!(
                "object {object_id} record block {lba} outside allocatable bounds"
            )));
        }
        let mut buf = vec![0u8; self.dev.block_size()];
        self.dev.read_block(lba, &mut buf)?;
        let record = ObjectRecord::decode(&buf)
            .map_err(|e| CoreError::Corrupt(format!("object {object_id} record invalid: {e}")))?;
        if record.object_id != object_id {
            return Err(CoreError::Corrupt(format!(
                "object record at block {lba} claims ID {}, map says {object_id}",
                record.object_id
            )));
        }
        if record.object_type == ObjectType::File && record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            extent_map::validate_root(
                &mut self.dev,
                &self.ident.geometry(),
                record.data_root,
                object_id,
                self.checkpoint.generation,
            )?;
        } else if record.object_type == ObjectType::File {
            let data_end = record
                .data_root
                .checked_add(record.data_blocks)
                .ok_or_else(|| CoreError::Corrupt(format!("object {object_id} extent overflow")))?;
            for block in record.data_root..data_end {
                if !self.ident.geometry().is_allocatable(block) {
                    return Err(CoreError::Corrupt(format!(
                        "object {object_id} data block {block} outside allocatable bounds"
                    )));
                }
            }
        }
        Ok(Some(record))
    }

    fn lookup_object_lba(&mut self, object_id: u64) -> Result<Option<u64>, CoreError> {
        object_map::lookup_lba(
            &mut self.dev,
            &self.ident.geometry(),
            self.checkpoint.object_map_block,
            self.checkpoint.generation,
            object_id,
        )
    }

    fn object_record_lba(&mut self, object_id: u64) -> Result<Option<u64>, CoreError> {
        if object_id == OBJECT_ROOT {
            Ok(Some(self.state.root_record_lba))
        } else {
            self.lookup_object_lba(object_id)
        }
    }

    fn mutate_allocation_root(
        &mut self,
        generation: u64,
        dirty_records: &[(u32, afsplus_format::checkpoint::RegionRecord)],
    ) -> Result<TreeMutation, CoreError> {
        if self.checkpoint.allocation_root_block == 0 {
            return Err(CoreError::PrototypeLimit(
                "inline allocation checkpoint cannot be mutated",
            ));
        }
        let geo = self.ident.geometry();
        let current_blocks = allocation_root::load_tree_blocks(
            &mut self.dev,
            &geo,
            self.checkpoint.allocation_root_block,
            self.checkpoint.generation,
        )?;
        let older_blocks = if let Some(older) = self
            .other_checkpoint
            .as_ref()
            .filter(|checkpoint| checkpoint.allocation_root_block != 0)
        {
            allocation_root::load_tree_blocks(
                &mut self.dev,
                &geo,
                older.allocation_root_block,
                older.generation,
            )?
        } else {
            Vec::new()
        };
        let pool_lbas = allocation_root::reserved_pool_lbas(&geo)?;
        let mut pool = ReservedTreePool::new(pool_lbas, &current_blocks, &older_blocks)?;
        let encoded: Vec<_> = dirty_records
            .iter()
            .map(|(region, record)| {
                Ok((
                    allocation_root::key(*region),
                    allocation_root::value(*record)?,
                ))
            })
            .collect::<Result<_, CoreError>>()?;
        let operations: Vec<_> = encoded
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        mutate_many(
            &mut self.dev,
            &geo,
            &mut pool,
            self.checkpoint.allocation_root_block,
            allocation_root::spec(self.checkpoint.generation),
            generation,
            &operations,
        )
    }

    /// Retires the previous generation's bounded retired-list root. Callers
    /// separately retire each object-record block they replace; tree engines
    /// retire exactly the committed COW paths they replace.
    fn retire_previous_list(&mut self, tx: &mut TxAllocator) -> Result<(), CoreError> {
        if self.checkpoint.retired_list_block != 0 {
            tx.retire(&mut self.dev, self.checkpoint.retired_list_block)?;
        }
        Ok(())
    }

    /// The common commit tail: durability ordering, checkpoint write, state
    /// adoption, accounting. On any error the committed state is untouched
    /// and the in-memory volume still serves the old generation.
    #[allow(clippy::too_many_arguments)]
    fn commit_transaction(
        &mut self,
        generation: u64,
        next_object_id: u64,
        tx: TxAllocator,
        data_writes: Vec<(u64, Vec<u8>)>,
        mut meta_writes: Vec<(u64, Vec<u8>)>,
        retired_list_lba: u64,
        new_object_map_block: u64,
    ) -> Result<(), CoreError> {
        let block_size = self.dev.block_size();
        let finished = tx.finish(&self.checkpoint, self.other_checkpoint.as_ref())?;
        debug_assert!(!finished.retired.entries.is_empty());
        let allocation_root = self.mutate_allocation_root(generation, &finished.dirty_records)?;
        let new_allocation_root_block = allocation_root.root_lba;
        meta_writes.extend(allocation_root.writes);
        meta_writes.push((
            retired_list_lba,
            finished.retired.encode(block_size, generation)?,
        ));

        let mut stats = CommitStats {
            data_blocks_written: data_writes.len() as u64,
            metadata_blocks_written: meta_writes.len() as u64,
            bitmap_pages_written: finished.bitmap_writes.len() as u64,
            region_descriptors_written: finished.descriptor_writes.len() as u64,
            allocation_records_updated: finished.dirty_records.len() as u64,
            allocation_tree_nodes_written: allocation_root.stats.final_nodes_written,
            checkpoint_blocks_written: 1,
            alloc: finished.stats,
            ..CommitStats::default()
        };

        // 1. User data, then barrier — only when the transaction has data.
        for (lba, block) in &data_writes {
            self.dev.write_block(*lba, block)?;
        }
        if !data_writes.is_empty() {
            self.dev.flush()?;
            stats.flushes += 1;
        }

        // 2. COW metadata, dirty bitmap pages, then their new region
        // descriptors, all before the publication barrier.
        for (lba, block) in meta_writes
            .iter()
            .chain(finished.bitmap_writes.iter())
            .chain(finished.descriptor_writes.iter())
        {
            self.dev.write_block(*lba, block)?;
        }
        self.dev.flush()?;
        stats.flushes += 1;

        // 3. Alternate checkpoint slot, then the commit barrier.
        let new_slot = 1 - self.current_slot;
        let free_blocks_total = finished.free_blocks_total;
        let new_checkpoint = Checkpoint {
            uuid: self.ident.uuid,
            generation,
            root_object_id: OBJECT_ROOT,
            object_map_block: new_object_map_block,
            allocation_root_block: new_allocation_root_block,
            retired_list_block: retired_list_lba,
            next_object_id,
            committed_tx_id: generation,
            free_blocks_total,
            flags: 0,
            regions: Vec::new(),
        };
        self.dev.write_block(
            self.ident.checkpoint_slots[new_slot],
            &new_checkpoint.encode(block_size)?,
        )?;
        self.dev.flush()?;
        stats.flushes += 1;

        stats.bytes_written = (stats.data_blocks_written
            + stats.metadata_blocks_written
            + stats.bitmap_pages_written
            + stats.region_descriptors_written
            + stats.checkpoint_blocks_written)
            * block_size as u64;

        // Adopt the committed roots through the same bounded loader as a
        // remount. The transaction already owns the exact new bitmap state,
        // so no post-commit full-volume reload is needed.
        self.state = load_mount_state(&mut self.dev, &self.ident, &new_checkpoint)?;
        self.other_checkpoint = Some(std::mem::replace(&mut self.checkpoint, new_checkpoint));
        self.current_slot = new_slot;
        self.last_commit = Some(stats);
        Ok(())
    }
}

fn direct_layout(extents: &[Extent], size_bytes: u64, block_size: u64) -> Option<Option<Extent>> {
    if extents.is_empty() {
        return (size_bytes == 0).then_some(None);
    }
    let [extent] = extents else {
        return None;
    };
    if extent.logical_start != 0 || extent.flags != 0 || extent.block_count > MAX_EXTENT_BLOCKS {
        return None;
    }
    let capacity = extent.block_count.checked_mul(block_size)?;
    let minimum = extent.block_count.checked_sub(1)?.checked_mul(block_size)?;
    (size_bytes > minimum && size_bytes <= capacity).then_some(Some(*extent))
}

fn extent_at(extents: &[Extent], logical_block: u64) -> Option<Extent> {
    let after = extents.partition_point(|extent| extent.logical_start <= logical_block);
    let extent = after.checked_sub(1).map(|index| extents[index])?;
    (logical_block < extent.logical_start.saturating_add(extent.block_count)).then_some(extent)
}

fn logical_holes(extents: &[Extent], start: u64, end: u64) -> Result<Vec<(u64, u64)>, CoreError> {
    let mut holes = Vec::new();
    let mut cursor = start;
    for extent in extents {
        let extent_end = extent.logical_end()?;
        if extent_end <= cursor {
            continue;
        }
        if extent.logical_start >= end {
            break;
        }
        if extent.logical_start > cursor {
            holes.push((cursor, extent.logical_start.min(end)));
        }
        cursor = cursor.max(extent_end.min(end));
        if cursor == end {
            break;
        }
    }
    if cursor < end {
        holes.push((cursor, end));
    }
    Ok(holes)
}

fn coalesce_extents(mut extents: Vec<Extent>) -> Result<Vec<Extent>, CoreError> {
    extents.sort_unstable_by_key(|extent| extent.logical_start);
    let mut coalesced: Vec<Extent> = Vec::with_capacity(extents.len());
    for extent in extents {
        if let Some(previous) = coalesced.last_mut() {
            let previous_end = previous.logical_end()?;
            if previous_end > extent.logical_start {
                return Err(CoreError::Corrupt("logical extents overlap".into()));
            }
            if previous.flags == extent.flags
                && previous_end == extent.logical_start
                && previous.physical_end()? == extent.physical_start
            {
                previous.block_count = previous
                    .block_count
                    .checked_add(extent.block_count)
                    .ok_or_else(|| CoreError::Corrupt("coalesced extent overflows".into()))?;
                continue;
            }
        }
        coalesced.push(extent);
    }
    Ok(coalesced)
}

fn allocate_extent_runs<D: BlockDevice>(
    tx: &mut TxAllocator,
    dev: &mut D,
    geo: &afsplus_format::geometry::Geometry,
    mut logical_start: u64,
    mut block_count: u64,
    flags: u32,
) -> Result<Vec<Extent>, CoreError> {
    let max_run = maximum_allocatable_run(geo)?;
    let mut extents = Vec::new();
    while block_count > 0 {
        let mut candidate = block_count.min(max_run);
        let physical_start = loop {
            match tx.allocate_run(dev, candidate) {
                Ok(start) => break start,
                Err(CoreError::NoSpace) if candidate > 1 => {
                    candidate = candidate.div_ceil(2);
                }
                Err(error) => return Err(error),
            }
        };
        extents.push(Extent {
            logical_start,
            physical_start,
            block_count: candidate,
            flags,
        });
        logical_start = logical_start
            .checked_add(candidate)
            .ok_or(CoreError::PrototypeLimit("logical extent range overflows"))?;
        block_count -= candidate;
    }
    coalesce_extents(extents)
}

fn maximum_allocatable_run(geo: &afsplus_format::geometry::Geometry) -> Result<u64, CoreError> {
    let last = geo.region_count() - 1;
    let candidates = [0, 1.min(last), last];
    let mut maximum = 0u64;
    for region in candidates {
        let bootstrap = if region == 0 {
            afsplus_format::geometry::BOOTSTRAP_BLOCKS
        } else {
            0
        };
        let available =
            geo.region_valid_blocks(region) as u64 - geo.region_reserved_blocks(region) - bootstrap;
        maximum = maximum.max(available);
    }
    if maximum == 0 {
        return Err(CoreError::Corrupt(
            "validated geometry has no allocatable run".into(),
        ));
    }
    Ok(maximum)
}

fn replace_logical_range(
    old_extents: &[Extent],
    start: u64,
    end: u64,
    replacement: Option<Extent>,
) -> Result<(Vec<Extent>, Vec<Extent>), CoreError> {
    if start >= end {
        return Err(CoreError::Corrupt("empty logical replacement range".into()));
    }
    let mut extents = Vec::with_capacity(old_extents.len() + usize::from(replacement.is_some()));
    let mut removed = Vec::new();
    for old in old_extents {
        let old_end = old.logical_end()?;
        if old_end <= start || old.logical_start >= end {
            extents.push(*old);
            continue;
        }
        let overlap_start = old.logical_start.max(start);
        let overlap_end = old_end.min(end);
        if old.logical_start < overlap_start {
            extents.push(Extent {
                block_count: overlap_start - old.logical_start,
                ..*old
            });
        }
        removed.push(Extent {
            logical_start: overlap_start,
            physical_start: old.physical_start + overlap_start - old.logical_start,
            block_count: overlap_end - overlap_start,
            flags: old.flags,
        });
        if overlap_end < old_end {
            extents.push(Extent {
                logical_start: overlap_end,
                physical_start: old.physical_start + overlap_end - old.logical_start,
                block_count: old_end - overlap_end,
                flags: old.flags,
            });
        }
    }
    if let Some(replacement) = replacement {
        if replacement.logical_start != start || replacement.logical_end()? != end {
            return Err(CoreError::Corrupt(
                "replacement extent does not cover requested range".into(),
            ));
        }
        extents.push(replacement);
    }
    Ok((coalesce_extents(extents)?, removed))
}
