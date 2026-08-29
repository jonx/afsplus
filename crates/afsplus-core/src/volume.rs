//! Mounted volume: COW transactions over the region allocator.
//!
//! Commit ordering (`docs/08-transactions-and-journal.md` §3):
//!
//! 1. write new user data, barrier (skipped when the transaction has none)
//! 2. write COW metadata and the dirty region bitmap pages, barrier
//! 3. write the alternate checkpoint slot with generation + 1, barrier
//!
//! Every transaction retires the blocks it makes unreachable (replaced
//! object-map paths, old records, old directory blocks, old retired list,
//! deleted data) and promotes the previous transaction's retirees; see
//! `alloc`.
//!
//! Per-transaction resource accounting is collected from the start
//! ([`CommitStats`]) — metadata bytes, bitmap pages, region descriptors,
//! flushes, retired and promoted blocks, reclaim latency, allocator RAM.

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::{comparison_key, DirEntry};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::retired::RetiredList;
use afsplus_format::{validate_name, Timespec, OBJECT_ROOT};

use crate::alloc::{AllocStats, TxAllocator};
use crate::allocation_root::{self, ReservedTreePool};
use crate::cow_tree::{mutate_many, TreeMutation, TreeOperation};
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
    pub fn lookup_root(&self, name: &str) -> Option<u64> {
        let key = comparison_key(name.as_bytes());
        self.state.root_directory.lookup(&key).map(|e| e.child_id)
    }

    /// Lists the root directory as (original name, object ID) pairs.
    pub fn list_root(&self) -> Vec<(String, u64)> {
        self.state
            .root_directory
            .entries
            .iter()
            .map(|e| (String::from_utf8_lossy(&e.name).into_owned(), e.child_id))
            .collect()
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
        let mut content = vec![0u8; (record.data_blocks as usize) * block_size];
        for i in 0..record.data_blocks {
            let offset = i as usize * block_size;
            let lba = record
                .data_root
                .checked_add(i)
                .ok_or_else(|| CoreError::Corrupt(format!("object {object_id} extent overflow")))?;
            self.dev
                .read_block(lba, &mut content[offset..offset + block_size])?;
        }
        content.truncate(record.size_bytes as usize);
        Ok(content)
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
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let key = comparison_key(name.as_bytes());
        let root_dir = self.state.root_directory.clone();
        if root_dir.lookup(&key).is_some() {
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
        let dir_lba = tx.allocate(&mut self.dev)?;
        let root_record_lba = tx.allocate(&mut self.dev)?;
        let retired_list_lba = tx.allocate(&mut self.dev)?;

        // Everything the new state no longer reaches goes into quarantine.
        self.retire_cow_originals(&mut tx)?;

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

        let mut new_dir = root_dir;
        new_dir.insert(DirEntry {
            key,
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        })?;

        let old_root = self.state.root_object;
        let new_root = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: dir_lba,
            ..old_root
        };

        let root_key = object_map::key(OBJECT_ROOT);
        let root_value = object_map::value(root_record_lba)?;
        let file_key = object_map::key(object_id);
        let file_value = object_map::value(file_record_lba)?;
        let operations = [
            TreeOperation::Upsert {
                key: &root_key,
                value: &root_value,
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
            (dir_lba, new_dir.encode(block_size, generation)?),
            (root_record_lba, new_root.encode(block_size, generation)?),
        ];
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

    /// Deletes a file from the root directory. Its record and data blocks are
    /// retired, not freed: they stay quarantined until no still-selectable
    /// checkpoint can reference them.
    pub fn delete_file_in_root(&mut self, name: &str, now: Timespec) -> Result<(), CoreError> {
        let key = comparison_key(name.as_bytes());
        let root_dir = self.state.root_directory.clone();
        let entry = root_dir.lookup(&key).ok_or(CoreError::NotFound)?.clone();
        let victim = self
            .read_object(entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("victim missing from object map".into()))?;
        if victim.object_type != ObjectType::File {
            return Err(CoreError::PrototypeLimit(
                "only file deletion is implemented",
            ));
        }

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

        let dir_lba = tx.allocate(&mut self.dev)?;
        let root_record_lba = tx.allocate(&mut self.dev)?;
        let retired_list_lba = tx.allocate(&mut self.dev)?;

        // Quarantine the object's storage and the COW'd originals.
        let victim_record_lba = self
            .lookup_object_lba(entry.child_id)?
            .ok_or_else(|| CoreError::Corrupt("victim missing from object map".into()))?;
        tx.retire(&mut self.dev, victim_record_lba)?;
        for i in 0..victim.data_blocks {
            tx.retire(&mut self.dev, victim.data_root + i)?;
        }
        self.retire_cow_originals(&mut tx)?;

        let mut new_dir = root_dir;
        new_dir.remove(&key);

        let old_root = self.state.root_object;
        let new_root = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: dir_lba,
            ..old_root
        };

        let root_key = object_map::key(OBJECT_ROOT);
        let root_value = object_map::value(root_record_lba)?;
        let victim_key = object_map::key(entry.child_id);
        let operations = [
            TreeOperation::Upsert {
                key: &root_key,
                value: &root_value,
            },
            TreeOperation::Delete { key: &victim_key },
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
            (dir_lba, new_dir.encode(block_size, generation)?),
            (root_record_lba, new_root.encode(block_size, generation)?),
        ];
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
        if record.object_type == ObjectType::File {
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

    fn mutate_allocation_root(
        &mut self,
        generation: u64,
        records: &[afsplus_format::checkpoint::RegionRecord],
    ) -> Result<TreeMutation, CoreError> {
        if self.checkpoint.allocation_root_block == 0 {
            return Err(CoreError::PrototypeLimit(
                "inline allocation checkpoint cannot be mutated",
            ));
        }
        let geo = self.ident.geometry();
        let current = allocation_root::load_all(
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
            allocation_root::load_all(
                &mut self.dev,
                &geo,
                older.allocation_root_block,
                older.generation,
            )?
            .tree_blocks
        } else {
            Vec::new()
        };
        let layout = allocation_root::bulk_build(&geo, records)?;
        let mut pool = ReservedTreePool::new(
            layout.pool_lbas,
            &current.tree_blocks,
            &older_blocks,
        )?;
        let encoded: Vec<_> = records
            .iter()
            .enumerate()
            .map(|(region, record)| {
                Ok((
                    allocation_root::key(region as u32),
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

    /// Retires the committed blocks every transaction replaces: the object
    /// root object record, the root directory block, and the previous retired
    /// list. The object-map engine retires exactly the COW paths it replaces.
    fn retire_cow_originals(&mut self, tx: &mut TxAllocator) -> Result<(), CoreError> {
        tx.retire(&mut self.dev, self.state.root_record_lba)?;
        tx.retire(&mut self.dev, self.state.root_object.data_root)?;
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
        let allocation_root = self.mutate_allocation_root(generation, &finished.records)?;
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
        let free_blocks_total = finished
            .records
            .iter()
            .map(|record| record.free_blocks as u64)
            .sum();
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
