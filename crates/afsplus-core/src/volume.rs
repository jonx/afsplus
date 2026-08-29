//! Mounted volume: COW transactions over the region allocator.
//!
//! Commit ordering (`docs/08-transactions-and-journal.md` §3):
//!
//! 1. write new user data, barrier (skipped when the transaction has none)
//! 2. write COW metadata and the dirty region bitmap pages, barrier
//! 3. write the alternate checkpoint slot with generation + 1, barrier
//!
//! Every transaction retires the blocks it makes unreachable (old object
//! map, old records, old directory blocks, old retired list, deleted data)
//! and promotes the previous transaction's retirees; see `alloc`.
//!
//! Per-transaction resource accounting is collected from the start
//! ([`CommitStats`]) — metadata bytes, bitmap pages, flushes, retired and
//! promoted blocks, reclaim latency, allocator RAM.

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::{comparison_key, DirEntry};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::retired::RetiredList;
use afsplus_format::{validate_name, Timespec, OBJECT_ROOT};

use crate::alloc::{AllocStats, TxAllocator};
use crate::mount::Selection;
use crate::verify::{load_committed_state, CommittedState};
use crate::CoreError;

/// Measured cost of the last committed transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitStats {
    pub data_blocks_written: u64,
    /// COW metadata blocks (records, directories, object map, retired list).
    pub metadata_blocks_written: u64,
    pub bitmap_pages_written: u64,
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
    state: CommittedState,
    last_commit: Option<CommitStats>,
}

impl<D: BlockDevice> Volume<D> {
    pub(crate) fn new(dev: D, ident: Identification, selection: Selection, state: CommittedState) -> Self {
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
        self.state.bitmaps.free_blocks_total()
    }

    pub fn allocator_ram_bytes(&self) -> usize {
        self.state.bitmaps.ram_bytes()
    }

    pub fn last_commit_stats(&self) -> Option<CommitStats> {
        self.last_commit
    }

    /// Looks a name up in the root directory.
    pub fn lookup_root(&self, name: &str) -> Option<u64> {
        let key = comparison_key(name.as_bytes());
        self.state.directories.get(&OBJECT_ROOT)?.lookup(&key).map(|e| e.child_id)
    }

    /// Lists the root directory as (original name, object ID) pairs.
    pub fn list_root(&self) -> Vec<(String, u64)> {
        match self.state.directories.get(&OBJECT_ROOT) {
            Some(dir) => dir
                .entries
                .iter()
                .map(|e| (String::from_utf8_lossy(&e.name).into_owned(), e.child_id))
                .collect(),
            None => Vec::new(),
        }
    }

    pub fn stat(&self, object_id: u64) -> Option<&ObjectRecord> {
        self.state.objects.get(&object_id)
    }

    /// Reads a file's committed content.
    pub fn read_file(&mut self, object_id: u64) -> Result<Vec<u8>, CoreError> {
        let record = *self
            .state
            .objects
            .get(&object_id)
            .ok_or_else(|| CoreError::Corrupt(format!("no object {object_id}")))?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::Corrupt(format!("object {object_id} is not a file")));
        }
        let block_size = self.dev.block_size();
        let mut content = vec![0u8; (record.data_blocks as usize) * block_size];
        for i in 0..record.data_blocks {
            let offset = i as usize * block_size;
            self.dev
                .read_block(record.data_root + i, &mut content[offset..offset + block_size])?;
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
        let root_dir = self
            .state
            .directories
            .get(&OBJECT_ROOT)
            .ok_or_else(|| CoreError::Corrupt("root directory block missing".into()))?;
        if root_dir.lookup(&key).is_some() {
            return Err(CoreError::AlreadyExists);
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;
        let object_id = self.checkpoint.next_object_id;
        let next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;

        let mut tx = TxAllocator::begin(&self.state.bitmaps, &self.state.retired, generation)?;

        // Data first: allocate one contiguous extent and stage its blocks.
        let data_block_count = (content.len() as u64).div_ceil(block_size as u64);
        let mut data_writes = Vec::new();
        let data_start = if data_block_count > 0 {
            let start = tx.allocate_run(data_block_count)?;
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
        let file_record_lba = tx.allocate()?;
        let dir_lba = tx.allocate()?;
        let root_record_lba = tx.allocate()?;
        let omap_lba = tx.allocate()?;
        let retired_list_lba = tx.allocate()?;

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

        let mut new_dir = root_dir.clone();
        new_dir.insert(DirEntry {
            key,
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        })?;

        let old_root = self.state.objects[&OBJECT_ROOT];
        let new_root = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: dir_lba,
            ..old_root
        };

        let mut new_omap = self.state.object_map.clone();
        new_omap.upsert(OBJECT_ROOT, root_record_lba)?;
        new_omap.upsert(object_id, file_record_lba)?;

        let meta_writes = vec![
            (file_record_lba, file_record.encode(block_size, generation)?),
            (dir_lba, new_dir.encode(block_size, generation)?),
            (root_record_lba, new_root.encode(block_size, generation)?),
            (omap_lba, new_omap.encode(block_size, generation)?),
        ];

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
        let root_dir = self
            .state
            .directories
            .get(&OBJECT_ROOT)
            .ok_or_else(|| CoreError::Corrupt("root directory block missing".into()))?;
        let entry = root_dir.lookup(&key).ok_or(CoreError::NotFound)?.clone();
        let victim = self.state.objects[&entry.child_id];
        if victim.object_type != ObjectType::File {
            return Err(CoreError::PrototypeLimit("only file deletion is implemented"));
        }

        let block_size = self.dev.block_size();
        let generation = self.next_generation()?;

        let mut tx = TxAllocator::begin(&self.state.bitmaps, &self.state.retired, generation)?;

        let dir_lba = tx.allocate()?;
        let root_record_lba = tx.allocate()?;
        let omap_lba = tx.allocate()?;
        let retired_list_lba = tx.allocate()?;

        // Quarantine the object's storage and the COW'd originals.
        let victim_record_lba = self
            .state
            .object_map
            .lookup(entry.child_id)
            .ok_or_else(|| CoreError::Corrupt("victim missing from object map".into()))?;
        tx.retire(victim_record_lba)?;
        for i in 0..victim.data_blocks {
            tx.retire(victim.data_root + i)?;
        }
        self.retire_cow_originals(&mut tx)?;

        let mut new_dir = root_dir.clone();
        new_dir.remove(&key);

        let old_root = self.state.objects[&OBJECT_ROOT];
        let new_root = ObjectRecord {
            modified: now,
            changed: now,
            content_generation: generation,
            data_root: dir_lba,
            ..old_root
        };

        let mut new_omap = self.state.object_map.clone();
        new_omap.remove(entry.child_id);
        new_omap.upsert(OBJECT_ROOT, root_record_lba)?;

        let meta_writes = vec![
            (dir_lba, new_dir.encode(block_size, generation)?),
            (root_record_lba, new_root.encode(block_size, generation)?),
            (omap_lba, new_omap.encode(block_size, generation)?),
        ];

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

    /// Retires the committed blocks every transaction replaces: the object
    /// map, the root object record, the root directory block, and the
    /// previous retired list.
    fn retire_cow_originals(&self, tx: &mut TxAllocator) -> Result<(), CoreError> {
        tx.retire(self.checkpoint.object_map_block)?;
        let old_root_record = self
            .state
            .object_map
            .lookup(OBJECT_ROOT)
            .ok_or_else(|| CoreError::Corrupt("root missing from object map".into()))?;
        tx.retire(old_root_record)?;
        tx.retire(self.state.objects[&OBJECT_ROOT].data_root)?;
        if self.checkpoint.retired_list_block != 0 {
            tx.retire(self.checkpoint.retired_list_block)?;
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
        meta_writes.push((retired_list_lba, finished.retired.encode(block_size, generation)?));

        let mut stats = CommitStats {
            data_blocks_written: data_writes.len() as u64,
            metadata_blocks_written: meta_writes.len() as u64,
            bitmap_pages_written: finished.page_writes.len() as u64,
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

        // 2. COW metadata and dirty bitmap pages, then barrier.
        for (lba, block) in meta_writes.iter().chain(finished.page_writes.iter()) {
            self.dev.write_block(*lba, block)?;
        }
        self.dev.flush()?;
        stats.flushes += 1;

        // 3. Alternate checkpoint slot, then the commit barrier.
        let new_slot = 1 - self.current_slot;
        let new_checkpoint = Checkpoint {
            uuid: self.ident.uuid,
            generation,
            root_object_id: OBJECT_ROOT,
            object_map_block: new_object_map_block,
            retired_list_block: retired_list_lba,
            next_object_id,
            committed_tx_id: generation,
            flags: 0,
            regions: finished.records,
        };
        self.dev
            .write_block(self.ident.checkpoint_slots[new_slot], &new_checkpoint.encode(block_size)?)?;
        self.dev.flush()?;
        stats.flushes += 1;

        stats.bytes_written = (stats.data_blocks_written
            + stats.metadata_blocks_written
            + stats.bitmap_pages_written
            + stats.checkpoint_blocks_written)
            * block_size as u64;

        // Adopt the committed state. Re-loading from disk (rather than
        // patching caches) keeps the in-memory state provably equal to what
        // a remount would see; acceptable at prototype scale.
        self.state = load_committed_state(&mut self.dev, &self.ident, &new_checkpoint)?;
        self.other_checkpoint = Some(std::mem::replace(&mut self.checkpoint, new_checkpoint));
        self.current_slot = new_slot;
        self.last_commit = Some(stats);
        Ok(())
    }
}
