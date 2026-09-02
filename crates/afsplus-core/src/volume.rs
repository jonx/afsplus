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

use std::collections::{BTreeMap, BTreeSet};

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::crc32c::crc32c;
use afsplus_format::dir::DirEntry;
use afsplus_format::ident::{Identification, RO_COMPAT_SHARED_EXTENTS};
use afsplus_format::intent_log::{LogOp, LogRecord, MAX_LOG_OPS};
use afsplus_format::object::{
    ObjectRecord, ObjectType, MAX_EXTENT_BLOCKS, OBJECT_FLAG_EXTENT_TREE,
};
use afsplus_format::{validate_name, Timespec, OBJECT_ROOT};

use crate::alloc::{AllocStats, TxAllocator};
use crate::allocation_root::{self, ReservedTreePool};
use crate::cow_tree::{mutate_many, TreeMutation, TreeOperation};
use crate::directory;
use crate::extent_map::{self, Extent, EXTENT_SHARED, EXTENT_UNWRITTEN};
use crate::intent_log;
use crate::mount::{MountMode, Selection};
use crate::name_key;
use crate::object_map;
use crate::shared_extents::{self, RefEdit, RefEditStats};
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
    /// Reference-tree nodes written for this transaction (ADR-061).
    pub shared_tree_nodes_written: u64,
    /// Direct-layout files promoted to an extent tree by a clone (ADR-061).
    pub layout_promotions: u64,
    /// Per-block shared-reference accounting (ADR-061).
    pub shared_refs: RefEditStats,
    pub flushes: u64,
    /// Total bytes issued to the device by this transaction.
    pub bytes_written: u64,
    pub alloc: AllocStats,
}

pub const MAX_DIRECTORY_PAGE_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryCursor {
    pub generation: u64,
    pub ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPage {
    pub entries: Vec<DirEntry>,
    pub next: DirectoryCursor,
    pub eof: bool,
}

/// One operation inside a [`Volume::run_batch`] group commit (ADR-026).
#[derive(Debug, Clone)]
pub enum BatchOp<'a> {
    CreateFile {
        parent_id: u64,
        name: &'a str,
        content: &'a [u8],
    },
    DeleteFile {
        parent_id: u64,
        name: &'a str,
    },
    Rename {
        source_parent_id: u64,
        source_name: &'a str,
        target_parent_id: u64,
        target_name: &'a str,
        /// Atomically replace an existing file target in the same batch.
        replace: bool,
    },
}

/// Logical read-your-writes overlay of one in-flight batch.
struct PendingBatch {
    /// Per-directory entry changes: Some = upsert, None = delete.
    dir_changes: BTreeMap<u64, BTreeMap<Vec<u8>, Option<DirEntry>>>,
    /// Timestamp of the last namespace operation touching each directory.
    dir_timestamps: BTreeMap<u64, Timespec>,
    /// Object-record changes: Some = rewrite, None = delete from the map.
    records: BTreeMap<u64, Option<ObjectRecord>>,
    /// Committed record blocks to retire when their object is rewritten.
    committed_record_lbas: BTreeMap<u64, u64>,
    /// Data runs of objects created by this batch (cancellable).
    created_data: BTreeMap<u64, (u64, u64)>,
    data_writes: Vec<(u64, Vec<u8>)>,
    /// Windowed batches write data blocks at operation time (covered by the
    /// fsync or metadata barrier); plain batches stage them for commit.
    write_through: bool,
    /// Window creates already covered by a durable log record: cancelling
    /// one must not release its blocks (an earlier record's content CRC
    /// still covers them); they are sacrificed to quarantine instead.
    logged_created: BTreeSet<u64>,
    /// Data runs of cancelled logged creates, quarantined at materialize.
    sacrificed: Vec<(u64, u64)>,
    next_object_id: u64,
}

/// An open operation window: a live ADR-026 batch whose fsynced prefix is
/// persisted in the intent log (ADR-037).
struct OpenWindow {
    tx: TxAllocator,
    pending: PendingBatch,
    generation: u64,
    unlogged: Vec<LogOp>,
    logged_records: u32,
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
    mount_mode: MountMode,
    pending_intent_records: u32,
    /// Per-transaction reclamation budget in blocks (runtime policy).
    reclaim_batch_blocks: u64,
    /// Allocation-root node sets for (current, other) checkpoints, cached
    /// across commits so the reserved-pool exclusion set needs no tree walk
    /// per transaction. Populated lazily on the first commit (mount stays
    /// bounded) and maintained incrementally afterwards.
    allocation_tree_cache: Option<(Vec<u64>, Vec<u64>)>,
    /// Region where the last allocation succeeded; the next transaction
    /// starts its search there instead of rescanning from region zero.
    alloc_rover_region: u32,
    window: Option<OpenWindow>,
    window_poisoned: bool,
    last_commit: Option<CommitStats>,
    /// The transaction-scoped shared-reference edit (ADR-061), keyed by the
    /// generation it was opened for. `next_generation` clears it so an
    /// aborted transaction can never leak half-applied edits into the next
    /// one, and `commit_transaction` publishes and consumes it.
    shared_refs: Option<(u64, RefEdit)>,
    /// Direct-layout promotions performed by the transaction being built.
    pending_layout_promotions: u64,
}

impl<D: BlockDevice> Volume<D> {
    pub(crate) fn new(
        dev: D,
        ident: Identification,
        selection: Selection,
        state: MountState,
        mount_mode: MountMode,
    ) -> Self {
        Volume {
            dev,
            ident,
            checkpoint: selection.chosen,
            current_slot: selection.chosen_slot,
            other_checkpoint: selection.other,
            state,
            mount_mode,
            pending_intent_records: 0,
            reclaim_batch_blocks: crate::reclaim::DEFAULT_RECLAIM_BATCH_BLOCKS,
            allocation_tree_cache: None,
            alloc_rover_region: 0,
            window: None,
            window_poisoned: false,
            last_commit: None,
            shared_refs: None,
            pending_layout_promotions: 0,
        }
    }

    pub fn generation(&self) -> u64 {
        self.checkpoint.generation
    }

    pub fn ident(&self) -> &Identification {
        &self.ident
    }

    fn comparison_key(&self, name: &[u8]) -> Result<Vec<u8>, CoreError> {
        name_key::comparison_key(&self.ident, name)
    }

    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    pub fn mount_mode(&self) -> MountMode {
        self.mount_mode
    }

    pub fn pending_intent_records(&self) -> u32 {
        self.pending_intent_records
    }

    /// Blocks currently quarantined in the reclaim queue.
    pub fn reclaim_pending_blocks(&self) -> u64 {
        self.state.reclaim_root.pending_blocks
    }

    /// Sets the per-transaction reclamation budget (blocks promoted from the
    /// queue head before each transaction allocates).
    pub fn set_reclaim_batch_blocks(&mut self, blocks: u64) {
        self.reclaim_batch_blocks = blocks.max(1);
    }

    /// Diagnostic: whether `lba` is currently quarantined. Walks the queue;
    /// intended for tests and tooling, not the I/O path.
    pub fn quarantine_contains(&mut self, lba: u64) -> Result<bool, CoreError> {
        crate::reclaim::contains(
            &mut self.dev,
            &self.ident.geometry(),
            self.checkpoint.reclaim_root_block,
            self.checkpoint.generation,
            lba,
        )
    }

    /// Runs one maintenance transaction that only advances reclamation:
    /// promotes up to the configured budget from the queue head and commits.
    /// Returns the number of blocks reclaimed (zero means the queue could
    /// not shrink further and no commit was made).
    pub fn reclaim_step(&mut self, _now: Timespec) -> Result<u64, CoreError> {
        self.ensure_window_closed()?;
        if self.state.reclaim_root.pending_blocks == 0 {
            return Ok(0);
        }
        let before = self.state.reclaim_root.pending_blocks;
        let generation = self.next_generation()?;
        let tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            Vec::new(),
            self.checkpoint.object_map_block,
        )?;
        Ok(before.saturating_sub(self.state.reclaim_root.pending_blocks))
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

    /// Filesystem-wide durability barrier. Successful immediate mutations
    /// are already durable; this also gives adapters an explicit sync hook.
    /// Read-only modes return success without touching the device.
    pub fn sync(&mut self) -> Result<(), CoreError> {
        if !self.mount_mode.allows_user_writes() {
            return Ok(());
        }
        self.ensure_window_closed()?;
        self.dev.flush()?;
        Ok(())
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
        let key = self.comparison_key(name.as_bytes())?;
        Ok(directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            directory_record.data_root,
            directory_id,
            self.checkpoint.generation,
            &self.ident,
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
            &self.ident,
        )?
        .entries
        .iter()
        .map(|e| (String::from_utf8_lossy(&e.name).into_owned(), e.child_id))
        .collect())
    }

    /// Reads a bounded directory page. Cursors bind to the mounted checkpoint
    /// generation; callers must restart after any commit that makes one stale.
    pub fn read_directory_page(
        &mut self,
        directory_id: u64,
        cursor: Option<DirectoryCursor>,
        max_entries: usize,
    ) -> Result<DirectoryPage, CoreError> {
        if max_entries > MAX_DIRECTORY_PAGE_ENTRIES {
            return Err(CoreError::PrototypeLimit(
                "directory page exceeds entry cap",
            ));
        }
        let cursor = cursor.unwrap_or(DirectoryCursor {
            generation: self.checkpoint.generation,
            ordinal: 0,
        });
        if cursor.generation != self.checkpoint.generation {
            return Err(CoreError::Stale);
        }
        let record = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let (entries, total) = directory::read_page(
            &mut self.dev,
            &self.ident.geometry(),
            record.data_root,
            directory::spec(directory_id, self.checkpoint.generation),
            &self.ident,
            cursor.ordinal,
            max_entries,
        )?;
        let next_ordinal = cursor
            .ordinal
            .checked_add(entries.len() as u64)
            .ok_or_else(|| CoreError::Corrupt("directory cursor overflow".into()))?;
        Ok(DirectoryPage {
            entries,
            next: DirectoryCursor {
                generation: cursor.generation,
                ordinal: next_ordinal,
            },
            eof: next_ordinal >= total,
        })
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

    /// Reads committed file bytes at `offset` into a caller-owned buffer.
    /// Sparse holes and unwritten extents are returned as zeros.
    pub fn read_file_at(
        &mut self,
        object_id: u64,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, CoreError> {
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        if destination.is_empty() || offset >= record.size_bytes {
            return Ok(0);
        }
        let count = (record.size_bytes - offset).min(destination.len() as u64);
        let end = offset + count;
        let block_size = self.dev.block_size() as u64;
        let mut block = vec![0u8; block_size as usize];
        for logical_block in offset / block_size..end.div_ceil(block_size) {
            block.fill(0);
            let mapped = if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
                extent_map::lookup_extent(
                    &mut self.dev,
                    &self.ident.geometry(),
                    record.data_root,
                    object_id,
                    self.checkpoint.generation,
                    logical_block,
                )?
                .filter(|extent| extent.flags & EXTENT_UNWRITTEN == 0)
                .map(|extent| extent.physical_start + logical_block - extent.logical_start)
            } else if logical_block < record.data_blocks {
                Some(record.data_root.checked_add(logical_block).ok_or_else(|| {
                    CoreError::Corrupt(format!("object {object_id} extent overflow"))
                })?)
            } else {
                None
            };
            if let Some(lba) = mapped {
                self.dev.read_block(lba, &mut block)?;
            }
            let block_start = logical_block * block_size;
            let copy_start = offset.max(block_start);
            let copy_end = end.min(block_start + block_size);
            let source = (copy_start - block_start) as usize..(copy_end - block_start) as usize;
            let target = (copy_start - offset) as usize..(copy_end - offset) as usize;
            destination[target].copy_from_slice(&block[source]);
        }
        Ok(count as usize)
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
        self.ensure_window_closed()?;
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
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
        self.ensure_window_closed()?;
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
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
        self.ensure_window_closed()?;
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
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

    /// Clones a committed file into a new directory entry that shares every
    /// data run with the source (ADR-027 `CloneFile`, mechanism ADR-061).
    /// Clone is a checkpoint transaction, never an intent-log operation: an
    /// open window is committed first so the clone cannot straddle the two
    /// durability mechanisms. Both sides' maps carry `EXTENT_SHARED`
    /// afterwards, and a direct-layout source is promoted to an extent tree
    /// because the direct representation has no flag word.
    pub fn clone_file(
        &mut self,
        source_id: u64,
        parent_id: u64,
        name: &str,
        now: Timespec,
    ) -> Result<u64, CoreError> {
        if self.window.is_some() {
            self.window_commit(now)?;
        }
        self.ensure_window_closed()?;
        if !self.shared_extents_enabled() {
            return Err(CoreError::FeatureDisabled(
                "shared-extents feature is not enabled on this volume",
            ));
        }
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = self.comparison_key(name.as_bytes())?;
        if self.lookup_in_directory(parent_id, name)?.is_some() {
            return Err(CoreError::AlreadyExists);
        }
        let source = self.read_object(source_id)?.ok_or(CoreError::NotFound)?;
        if source.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        let source_record_lba = self.object_record_lba(source_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("file {source_id} missing from object map"))
        })?;
        let (source_extents, source_tree_blocks) = self.load_file_layout(&source)?;
        let source_was_tree = source.flags & OBJECT_FLAG_EXTENT_TREE != 0;

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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;

        // One more reference per source run. A run shared for the first time
        // gains a record at two references (source + clone); an already
        // shared run is incremented, refusing overflow.
        {
            let edit = self.shared_refs_edit(generation)?;
            for extent in &source_extents {
                edit.acquire(extent.physical_start, extent.block_count)?;
            }
        }
        let shared: Vec<Extent> = source_extents
            .iter()
            .map(|extent| Extent {
                flags: extent.flags | EXTENT_SHARED,
                ..*extent
            })
            .collect();

        let mut meta_writes = Vec::new();

        // Source side: same mapping, now flagged. The direct layout carries
        // no flag word, so a direct source is promoted to an extent tree in
        // this same transaction (and never collapses back, because
        // direct_layout refuses flagged extents).
        let source_new_record_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, source_record_lba)?;
        let (source_flags, source_data_root) = if shared.is_empty() {
            (source.flags, source.data_root)
        } else if source_was_tree {
            let encoded = shared
                .iter()
                .copied()
                .map(extent_map::encode_extent)
                .collect::<Result<Vec<_>, _>>()?;
            let operations: Vec<TreeOperation<'_>> = encoded
                .iter()
                .map(|(key, value)| TreeOperation::Upsert { key, value })
                .collect();
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                source.data_root,
                extent_map::spec(source_id, self.checkpoint.generation),
                generation,
                &operations,
            )?;
            meta_writes.extend(mutation.writes);
            (source.flags, mutation.root_lba)
        } else {
            self.pending_layout_promotions += 1;
            let node_count = extent_map::bulk_node_count(block_size, shared.len())?;
            let mut lbas = Vec::with_capacity(node_count);
            for _ in 0..node_count {
                lbas.push(tx.allocate(&mut self.dev)?);
            }
            let built = extent_map::bulk_build(source_id, block_size, &shared, &lbas)?;
            for (lba, node) in built.nodes {
                meta_writes.push((lba, node.encode(block_size, generation)?));
            }
            (source.flags | OBJECT_FLAG_EXTENT_TREE, built.root_lba)
        };
        let _ = source_tree_blocks; // COW originals are retired by mutate_many
        let source_new_record = ObjectRecord {
            flags: source_flags,
            data_root: source_data_root,
            changed: now,
            ..source
        };

        // Destination: a distinct object with an independent map over the
        // same physical runs. An empty source clones to an empty file; any
        // mapped or tree source clones to a tree destination.
        let dest_record_lba = tx.allocate(&mut self.dev)?;
        let (dest_flags, dest_data_root, dest_data_blocks) =
            if shared.is_empty() && !source_was_tree {
                (0u16, 0u64, 0u64)
            } else {
                let node_count = extent_map::bulk_node_count(block_size, shared.len())?;
                let mut lbas = Vec::with_capacity(node_count);
                for _ in 0..node_count {
                    lbas.push(tx.allocate(&mut self.dev)?);
                }
                let built = extent_map::bulk_build(object_id, block_size, &shared, &lbas)?;
                for (lba, node) in built.nodes {
                    meta_writes.push((lba, node.encode(block_size, generation)?));
                }
                (OBJECT_FLAG_EXTENT_TREE, built.root_lba, source.data_blocks)
            };
        let dest_record = ObjectRecord {
            object_id,
            object_type: ObjectType::File,
            flags: dest_flags,
            link_count: 1,
            size_bytes: source.size_bytes,
            allocated_bytes: source.allocated_bytes,
            created: now,
            modified: source.modified,
            changed: now,
            protection: source.protection,
            content_generation: generation,
            data_root: dest_data_root,
            data_blocks: dest_data_blocks,
        };

        // Namespace: one new directory entry, parent record COW'd.
        let parent_record_new_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, parent_record_lba)?;
        let directory_entry = DirEntry {
            key,
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        };
        let (directory_key, directory_value) =
            directory::encode_entry(&self.ident, &directory_entry)?;
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

        let source_key = object_map::key(source_id);
        let source_value = object_map::value(source_new_record_lba)?;
        let dest_key = object_map::key(object_id);
        let dest_value = object_map::value(dest_record_lba)?;
        let parent_key = object_map::key(parent_id);
        let parent_value = object_map::value(parent_record_new_lba)?;
        let operations = [
            TreeOperation::Upsert {
                key: &source_key,
                value: &source_value,
            },
            TreeOperation::Upsert {
                key: &dest_key,
                value: &dest_value,
            },
            TreeOperation::Upsert {
                key: &parent_key,
                value: &parent_value,
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

        meta_writes.push((
            source_new_record_lba,
            source_new_record.encode(block_size, generation)?,
        ));
        meta_writes.push((dest_record_lba, dest_record.encode(block_size, generation)?));
        meta_writes.push((
            parent_record_new_lba,
            new_parent.encode(block_size, generation)?,
        ));
        meta_writes.extend(directory_mutation.writes);
        meta_writes.extend(omap_mutation.writes);

        self.commit_transaction(
            generation,
            next_object_id,
            tx,
            Vec::new(),
            meta_writes,
            omap_lba,
        )?;
        Ok(object_id)
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
        self.ensure_window_closed()?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = self.comparison_key(name.as_bytes())?;
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
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

        // Everything the new state no longer reaches goes into quarantine.
        tx.retire(&mut self.dev, parent_record_lba)?;

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
        let (directory_key, directory_value) =
            directory::encode_entry(&self.ident, &directory_entry)?;
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
        self.ensure_window_closed()?;
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;
        let directory_root_lba = tx.allocate(&mut self.dev)?;
        let directory_record_lba = tx.allocate(&mut self.dev)?;
        let parent_record_new_lba = tx.allocate(&mut self.dev)?;

        tx.retire(&mut self.dev, parent_record_lba)?;

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
            key: self.comparison_key(name.as_bytes())?,
            name: name.as_bytes().to_vec(),
            child_type_hint: 2,
            child_id: object_id,
        };
        let (directory_key, directory_value) = directory::encode_entry(&self.ident, &entry)?;
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
        self.ensure_window_closed()?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self.read_object(parent_id)?.ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let parent_record_lba = self.object_record_lba(parent_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("directory {parent_id} missing from object map"))
        })?;
        let key = self.comparison_key(name.as_bytes())?;
        let entry = directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            parent.data_root,
            parent_id,
            self.checkpoint.generation,
            &self.ident,
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
                &self.ident,
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;

        let parent_record_new_lba = tx.allocate(&mut self.dev)?;
        let victim_record_new_lba = if keep_file_object {
            Some(tx.allocate(&mut self.dev)?)
        } else {
            None
        };

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
                    self.release_data_run(&mut tx, generation, &extent)?;
                }
            } else if victim.data_blocks > 0 {
                tx.retire_run(&mut self.dev, victim.data_root, victim.data_blocks)?;
            }
            for lba in victim_directory_blocks {
                tx.retire(&mut self.dev, lba)?;
            }
        }
        tx.retire(&mut self.dev, parent_record_lba)?;

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
        self.ensure_window_closed()?;
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;
        let file_new_lba = tx.allocate(&mut self.dev)?;
        let parent_new_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, file_lba)?;
        tx.retire(&mut self.dev, parent_lba)?;

        let entry = DirEntry {
            key: self.comparison_key(name.as_bytes())?,
            name: name.as_bytes().to_vec(),
            child_type_hint: 1,
            child_id: object_id,
        };
        let (entry_key, entry_value) = directory::encode_entry(&self.ident, &entry)?;
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
        self.ensure_window_closed()?;
        validate_name(source_name.as_bytes()).map_err(CoreError::InvalidName)?;
        validate_name(target_name.as_bytes()).map_err(CoreError::InvalidName)?;
        let source_key = self.comparison_key(source_name.as_bytes())?;
        let target_key = self.comparison_key(target_name.as_bytes())?;

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
            &self.ident,
            &source_key,
        )?
        .ok_or(CoreError::NotFound)?;
        let same_key = source_parent_id == target_parent_id && source_key == target_key;
        if same_key && source_entry.name == target_name.as_bytes() {
            return Ok(());
        }
        if !same_key
            && directory::lookup_entry(
                &mut self.dev,
                &self.ident.geometry(),
                target_parent.data_root,
                target_parent_id,
                self.checkpoint.generation,
                &self.ident,
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
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;
        let source_parent_new_lba = tx.allocate(&mut self.dev)?;
        let target_parent_new_lba = if target_parent_id == source_parent_id {
            source_parent_new_lba
        } else {
            tx.allocate(&mut self.dev)?
        };
        let moved_new_lba = tx.allocate(&mut self.dev)?;

        tx.retire(&mut self.dev, source_parent_lba)?;
        if target_parent_id != source_parent_id {
            tx.retire(&mut self.dev, target_parent_lba)?;
        }
        tx.retire(&mut self.dev, moved_lba)?;

        let target_entry = DirEntry {
            key: target_key,
            name: target_name.as_bytes().to_vec(),
            child_type_hint: source_entry.child_type_hint,
            child_id: source_entry.child_id,
        };
        let (target_entry_key, target_entry_value) =
            directory::encode_entry(&self.ident, &target_entry)?;

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
                &self.ident,
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
        tx.retire(&mut self.dev, record_lba)?;
        for extent in removed_extents {
            self.release_data_run(&mut tx, generation, &extent)?;
        }

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
            object_map_mutation.root_lba,
        )
    }

    /// Executes several namespace operations as ONE transaction with ONE
    /// checkpoint publication: the group-commit / bounded atomic-batch
    /// primitive (ADR-026). Either every operation commits or none does.
    ///
    /// Prototype scope: file operations only (create with content, delete,
    /// rename with optional atomic replace). Operations see the effects of
    /// earlier operations in the same batch. Returns one entry per
    /// operation: the created object ID for `CreateFile`, `None` otherwise.
    pub fn run_batch(
        &mut self,
        ops: &[BatchOp<'_>],
        now: Timespec,
    ) -> Result<Vec<Option<u64>>, CoreError> {
        const MAX_BATCH_OPS: usize = 1024;
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        if ops.len() > MAX_BATCH_OPS {
            return Err(CoreError::PrototypeLimit(
                "batch exceeds bounded operation count",
            ));
        }
        self.ensure_window_closed()?;
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;
        let mut pending = PendingBatch {
            dir_changes: BTreeMap::new(),
            dir_timestamps: BTreeMap::new(),
            records: BTreeMap::new(),
            committed_record_lbas: BTreeMap::new(),
            created_data: BTreeMap::new(),
            data_writes: Vec::new(),
            write_through: false,
            logged_created: BTreeSet::new(),
            sacrificed: Vec::new(),
            next_object_id: self.checkpoint.next_object_id,
        };
        let mut results = Vec::with_capacity(ops.len());
        for op in ops {
            results.push(self.apply_batch_op(&mut tx, &mut pending, op, now, generation)?);
        }
        self.materialize_batch(tx, pending, now, generation, false)?;
        Ok(results)
    }

    /// Atomic-replace rename for files: the target, if present, is replaced
    /// in the same transaction (its storage is quarantined or its link count
    /// decremented). One-operation batch.
    pub fn rename_replace(
        &mut self,
        source_parent_id: u64,
        source_name: &str,
        target_parent_id: u64,
        target_name: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.run_batch(
            &[BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace: true,
            }],
            now,
        )?;
        Ok(())
    }

    /// Directory-entry view through the batch overlay, then committed state.
    fn batch_lookup(
        &mut self,
        pending: &PendingBatch,
        directory_id: u64,
        key: &[u8],
    ) -> Result<Option<DirEntry>, CoreError> {
        if let Some(changes) = pending.dir_changes.get(&directory_id) {
            if let Some(change) = changes.get(key) {
                return Ok(change.clone());
            }
        }
        let directory = self
            .batch_record(pending, directory_id)?
            .ok_or(CoreError::NotFound)?;
        if directory.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        // Entries are overlaid separately, so the committed tree root is the
        // right base even when the directory record has pending changes.
        let committed = self.read_object(directory_id)?.ok_or(CoreError::NotFound)?;
        directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            committed.data_root,
            directory_id,
            self.checkpoint.generation,
            &self.ident,
            key,
        )
    }

    /// Object-record view through the batch overlay, then committed state.
    fn batch_record(
        &mut self,
        pending: &PendingBatch,
        object_id: u64,
    ) -> Result<Option<ObjectRecord>, CoreError> {
        if let Some(record) = pending.records.get(&object_id) {
            return Ok(*record);
        }
        self.read_object(object_id)
    }

    /// Remembers the committed record block of an object the batch rewrites
    /// or deletes, so materialization retires exactly one old block per
    /// object.
    fn note_committed_record(
        &mut self,
        pending: &mut PendingBatch,
        object_id: u64,
    ) -> Result<(), CoreError> {
        if pending.committed_record_lbas.contains_key(&object_id)
            || pending.created_data.contains_key(&object_id)
        {
            return Ok(());
        }
        let lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("object {object_id} missing from object map"))
        })?;
        pending.committed_record_lbas.insert(object_id, lba);
        Ok(())
    }

    fn apply_batch_op(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        op: &BatchOp<'_>,
        now: Timespec,
        generation: u64,
    ) -> Result<Option<u64>, CoreError> {
        let block_size = self.dev.block_size();
        match op {
            BatchOp::CreateFile {
                parent_id,
                name,
                content,
            } => {
                validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
                let parent = self
                    .batch_record(pending, *parent_id)?
                    .ok_or(CoreError::NotFound)?;
                if parent.object_type != ObjectType::Directory {
                    return Err(CoreError::NotDirectory);
                }
                let key = self.comparison_key(name.as_bytes())?;
                if self.batch_lookup(pending, *parent_id, &key)?.is_some() {
                    return Err(CoreError::AlreadyExists);
                }
                let object_id = pending.next_object_id;
                pending.next_object_id = object_id
                    .checked_add(1)
                    .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;
                let data_block_count = (content.len() as u64).div_ceil(block_size as u64);
                let data_start = if data_block_count > 0 {
                    let start = tx.allocate_run(&mut self.dev, data_block_count)?;
                    for i in 0..data_block_count as usize {
                        let mut block = vec![0u8; block_size];
                        let from = i * block_size;
                        let to = content.len().min(from + block_size);
                        block[..to - from].copy_from_slice(&content[from..to]);
                        if pending.write_through {
                            self.dev.write_block(start + i as u64, &block)?;
                        } else {
                            pending.data_writes.push((start + i as u64, block));
                        }
                    }
                    start
                } else {
                    0
                };
                pending
                    .created_data
                    .insert(object_id, (data_start, data_block_count));
                pending.records.insert(
                    object_id,
                    Some(ObjectRecord {
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
                    }),
                );
                pending.dir_changes.entry(*parent_id).or_default().insert(
                    key,
                    Some(DirEntry {
                        key: self.comparison_key(name.as_bytes())?,
                        name: name.as_bytes().to_vec(),
                        child_type_hint: 1,
                        child_id: object_id,
                    }),
                );
                pending.dir_timestamps.insert(*parent_id, now);
                Ok(Some(object_id))
            }
            BatchOp::DeleteFile { parent_id, name } => {
                validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
                let key = self.comparison_key(name.as_bytes())?;
                let entry = self
                    .batch_lookup(pending, *parent_id, &key)?
                    .ok_or(CoreError::NotFound)?;
                self.unlink_in_batch(tx, pending, generation, entry.child_id, now)?;
                pending
                    .dir_changes
                    .entry(*parent_id)
                    .or_default()
                    .insert(key, None);
                pending.dir_timestamps.insert(*parent_id, now);
                Ok(None)
            }
            BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace,
            } => {
                validate_name(source_name.as_bytes()).map_err(CoreError::InvalidName)?;
                validate_name(target_name.as_bytes()).map_err(CoreError::InvalidName)?;
                let source_key = self.comparison_key(source_name.as_bytes())?;
                let target_key = self.comparison_key(target_name.as_bytes())?;
                let same_key = source_parent_id == target_parent_id && source_key == target_key;
                let entry = self
                    .batch_lookup(pending, *source_parent_id, &source_key)?
                    .ok_or(CoreError::NotFound)?;
                if same_key && entry.name == target_name.as_bytes() {
                    return Ok(None);
                }
                let moved = self
                    .batch_record(pending, entry.child_id)?
                    .ok_or_else(|| CoreError::Corrupt("moved object missing".into()))?;
                if moved.object_type != ObjectType::File {
                    return Err(CoreError::PrototypeLimit(
                        "batched rename supports files only",
                    ));
                }
                if !same_key {
                    if let Some(existing) =
                        self.batch_lookup(pending, *target_parent_id, &target_key)?
                    {
                        if !replace {
                            return Err(CoreError::AlreadyExists);
                        }
                        let target = self
                            .batch_record(pending, existing.child_id)?
                            .ok_or_else(|| CoreError::Corrupt("replace target missing".into()))?;
                        if target.object_type != ObjectType::File {
                            return Err(CoreError::IsDirectory);
                        }
                        self.unlink_in_batch(tx, pending, generation, existing.child_id, now)?;
                    }
                }
                let target_parent = self
                    .batch_record(pending, *target_parent_id)?
                    .ok_or(CoreError::NotFound)?;
                if target_parent.object_type != ObjectType::Directory {
                    return Err(CoreError::NotDirectory);
                }
                pending
                    .dir_changes
                    .entry(*source_parent_id)
                    .or_default()
                    .insert(source_key, None);
                pending
                    .dir_changes
                    .entry(*target_parent_id)
                    .or_default()
                    .insert(
                        target_key,
                        Some(DirEntry {
                            key: self.comparison_key(target_name.as_bytes())?,
                            name: target_name.as_bytes().to_vec(),
                            child_type_hint: 1,
                            child_id: entry.child_id,
                        }),
                    );
                pending.dir_timestamps.insert(*source_parent_id, now);
                pending.dir_timestamps.insert(*target_parent_id, now);
                if !pending.created_data.contains_key(&entry.child_id) {
                    self.note_committed_record(pending, entry.child_id)?;
                }
                pending.records.insert(
                    entry.child_id,
                    Some(ObjectRecord {
                        changed: now,
                        ..moved
                    }),
                );
                Ok(None)
            }
        }
    }

    /// Drops one link from a file inside a batch: cancels a same-batch
    /// creation entirely (its storage is released, never quarantined),
    /// decrements a multiply-linked committed file, or retires a committed
    /// file's record and storage.
    fn unlink_in_batch(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        generation: u64,
        object_id: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        let victim = self
            .batch_record(pending, object_id)?
            .ok_or_else(|| CoreError::Corrupt("unlink victim missing".into()))?;
        if victim.object_type != ObjectType::File {
            return Err(CoreError::IsDirectory);
        }
        if let Some((data_start, data_blocks)) = pending.created_data.remove(&object_id) {
            pending.records.remove(&object_id);
            if pending.logged_created.contains(&object_id) {
                // A durable log record's content CRC still covers these
                // blocks: never reuse them in this window; quarantine them
                // when the window materializes.
                if data_blocks > 0 {
                    pending.sacrificed.push((data_start, data_blocks));
                }
                return Ok(());
            }
            // Same-batch creation, never logged: nothing on disk or in the
            // log references it. Release the staged data and writes.
            for lba in data_start..data_start + data_blocks {
                tx.release_uncommitted(&mut self.dev, lba)?;
            }
            pending
                .data_writes
                .retain(|(lba, _)| *lba < data_start || *lba >= data_start + data_blocks);
            return Ok(());
        }
        if victim.link_count > 1 {
            self.note_committed_record(pending, object_id)?;
            pending.records.insert(
                object_id,
                Some(ObjectRecord {
                    link_count: victim.link_count - 1,
                    changed: now,
                    ..victim
                }),
            );
            return Ok(());
        }
        self.note_committed_record(pending, object_id)?;
        let committed_lba = pending.committed_record_lbas[&object_id];
        tx.retire(&mut self.dev, committed_lba)?;
        pending.committed_record_lbas.remove(&object_id);
        self.retire_file_storage(tx, generation, &victim)?;
        pending.records.insert(object_id, None);
        Ok(())
    }

    /// True when this volume may hold shared extents (ADR-061).
    fn shared_extents_enabled(&self) -> bool {
        self.ident.features.ro_compat & RO_COMPAT_SHARED_EXTENTS != 0
    }

    /// The transaction-scoped reference edit, loading the committed records
    /// on first use. Keyed by generation; `next_generation` cleared any
    /// leftover from an aborted transaction.
    fn shared_refs_edit(&mut self, generation: u64) -> Result<&mut RefEdit, CoreError> {
        let stale = self
            .shared_refs
            .as_ref()
            .is_none_or(|(opened_for, _)| *opened_for != generation);
        if stale {
            let records = if self.checkpoint.shared_extent_root_block != 0 {
                shared_extents::load_all(
                    &mut self.dev,
                    &self.ident.geometry(),
                    self.checkpoint.shared_extent_root_block,
                    self.checkpoint.generation,
                )?
                .records
            } else {
                Vec::new()
            };
            self.shared_refs = Some((generation, RefEdit::new(records)));
        }
        Ok(&mut self
            .shared_refs
            .as_mut()
            .expect("shared_refs initialized above")
            .1)
    }

    /// Drops this mapping's reference to a data run and retires exactly what
    /// no live mapping still covers (the ADR-061 release table). A private
    /// extent retires whole; a flagged extent is partitioned by overlap, and
    /// a record falling from two references to one is removed WITHOUT
    /// retiring its blocks — the surviving peer still maps them.
    fn release_data_run(
        &mut self,
        tx: &mut TxAllocator,
        generation: u64,
        extent: &Extent,
    ) -> Result<(), CoreError> {
        if extent.flags & EXTENT_SHARED == 0 {
            tx.retire_run(&mut self.dev, extent.physical_start, extent.block_count)?;
            return Ok(());
        }
        let gaps = self
            .shared_refs_edit(generation)?
            .release(extent.physical_start, extent.block_count)?;
        for (start, blocks) in gaps {
            tx.retire_run(&mut self.dev, start, blocks)?;
        }
        Ok(())
    }

    /// Quarantines a committed file's data extents and extent-tree nodes,
    /// honouring shared references (ADR-061).
    fn retire_file_storage(
        &mut self,
        tx: &mut TxAllocator,
        generation: u64,
        victim: &ObjectRecord,
    ) -> Result<(), CoreError> {
        if victim.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            let map = extent_map::load_all(
                &mut self.dev,
                &self.ident.geometry(),
                victim.data_root,
                victim.object_id,
                self.checkpoint.generation,
            )?;
            for lba in map.tree_blocks {
                tx.retire(&mut self.dev, lba)?;
            }
            for extent in map.extents {
                self.release_data_run(tx, generation, &extent)?;
            }
        } else if victim.data_blocks > 0 {
            tx.retire_run(&mut self.dev, victim.data_root, victim.data_blocks)?;
        }
        Ok(())
    }

    fn ensure_window_closed(&self) -> Result<(), CoreError> {
        if !self.mount_mode.allows_user_writes() {
            Err(CoreError::ReadOnly)
        } else if self.window_poisoned {
            Err(CoreError::WindowPoisoned)
        } else if self.window.is_some() {
            Err(CoreError::WindowOpen)
        } else {
            Ok(())
        }
    }

    /// True for errors that reject one operation without having mutated the
    /// window's staged state.
    fn is_validation_error(error: &CoreError) -> bool {
        matches!(
            error,
            CoreError::AlreadyExists
                | CoreError::NotFound
                | CoreError::InvalidName(_)
                | CoreError::NotDirectory
                | CoreError::IsDirectory
                | CoreError::NoSpace
                | CoreError::PrototypeLimit(_)
        )
    }

    /// Pending window operations not yet made durable by an fsync.
    pub fn window_unlogged_ops(&self) -> usize {
        self.window.as_ref().map_or(0, |w| w.unlogged.len())
    }

    /// Applies one operation to the open window (opening it if needed). The
    /// operation is visible to later window operations but not durable until
    /// [`Volume::window_fsync`] and not checkpointed until
    /// [`Volume::window_commit`].
    pub fn window_op(&mut self, op: &BatchOp<'_>, now: Timespec) -> Result<Option<u64>, CoreError> {
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        if self.window_poisoned {
            return Err(CoreError::WindowPoisoned);
        }
        let mut window = match self.window.take() {
            Some(window) => window,
            None => {
                let generation = self.next_generation()?;
                // Windowed transactions never promote quarantined blocks:
                // logged extents must be FREE in the committed bitmaps so
                // replay can claim them deterministically (ADR-037).
                let tx = TxAllocator::begin(
                    &mut self.dev,
                    &self.ident.geometry(),
                    &self.checkpoint,
                    self.other_checkpoint.as_ref(),
                    generation,
                    0,
                    self.alloc_rover_region,
                )?;
                OpenWindow {
                    tx,
                    pending: PendingBatch {
                        dir_changes: BTreeMap::new(),
                        dir_timestamps: BTreeMap::new(),
                        records: BTreeMap::new(),
                        committed_record_lbas: BTreeMap::new(),
                        created_data: BTreeMap::new(),
                        data_writes: Vec::new(),
                        write_through: true,
                        logged_created: BTreeSet::new(),
                        sacrificed: Vec::new(),
                        next_object_id: self.checkpoint.next_object_id,
                    },
                    generation,
                    unlogged: Vec::new(),
                    logged_records: 0,
                }
            }
        };
        let generation = window.generation;
        // A delete (or replacing rename) whose victim is a window create not
        // yet covered by a log record cancels to nothing: the create is
        // scrubbed from the unlogged group so the record never mentions it.
        let cancels_unlogged = self.window_cancel_target(&window.pending, op)?;
        let result = self.apply_batch_op(&mut window.tx, &mut window.pending, op, now, generation);
        match result {
            Ok(created) => {
                if let Some(cancelled) = cancels_unlogged {
                    window.unlogged.retain(|logged| {
                        !matches!(
                            logged,
                            LogOp::Create { expected_object_id, .. }
                                if *expected_object_id == cancelled
                        )
                    });
                    if !matches!(op, BatchOp::DeleteFile { .. }) {
                        window
                            .unlogged
                            .push(Self::log_op_for(op, created, &window.pending, now)?);
                    }
                } else {
                    window
                        .unlogged
                        .push(Self::log_op_for(op, created, &window.pending, now)?);
                }
                self.window = Some(window);
                Ok(created)
            }
            Err(error) if Self::is_validation_error(&error) => {
                self.window = Some(window);
                Err(error)
            }
            Err(error) => {
                self.window_poisoned = true;
                Err(error)
            }
        }
    }

    /// The window-created, not-yet-logged object a delete or replacing
    /// rename would cancel, if any.
    fn window_cancel_target(
        &mut self,
        pending: &PendingBatch,
        op: &BatchOp<'_>,
    ) -> Result<Option<u64>, CoreError> {
        let (parent_id, name) = match op {
            BatchOp::DeleteFile { parent_id, name } => (*parent_id, *name),
            BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace: true,
                ..
            } => {
                if source_parent_id == target_parent_id
                    && self.comparison_key(source_name.as_bytes())?
                        == self.comparison_key(target_name.as_bytes())?
                {
                    return Ok(None);
                }
                (*target_parent_id, *target_name)
            }
            _ => return Ok(None),
        };
        if validate_name(name.as_bytes()).is_err() {
            return Ok(None);
        }
        let key = self.comparison_key(name.as_bytes())?;
        let Some(entry) = self.batch_lookup(pending, parent_id, &key)? else {
            return Ok(None);
        };
        Ok((pending.created_data.contains_key(&entry.child_id)
            && !pending.logged_created.contains(&entry.child_id))
        .then_some(entry.child_id))
    }

    fn log_op_for(
        op: &BatchOp<'_>,
        created: Option<u64>,
        pending: &PendingBatch,
        now: Timespec,
    ) -> Result<LogOp, CoreError> {
        Ok(match op {
            BatchOp::CreateFile {
                parent_id,
                name,
                content,
            } => {
                let object_id = created
                    .ok_or_else(|| CoreError::Corrupt("create produced no object ID".into()))?;
                let (start, blocks) = pending
                    .created_data
                    .get(&object_id)
                    .copied()
                    .ok_or_else(|| CoreError::Corrupt("created data run missing".into()))?;
                let extents = if blocks > 0 {
                    vec![(start, blocks as u32)]
                } else {
                    Vec::new()
                };
                LogOp::Create {
                    parent_id: *parent_id,
                    name: name.as_bytes().to_vec(),
                    expected_object_id: object_id,
                    size_bytes: content.len() as u64,
                    content_crc: crc32c(content),
                    extents,
                    timestamp: now,
                }
            }
            BatchOp::DeleteFile { parent_id, name } => LogOp::Delete {
                parent_id: *parent_id,
                name: name.as_bytes().to_vec(),
                timestamp: now,
            },
            BatchOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace,
            } => LogOp::Rename {
                source_parent_id: *source_parent_id,
                source_name: source_name.as_bytes().to_vec(),
                target_parent_id: *target_parent_id,
                target_name: target_name.as_bytes().to_vec(),
                replace: *replace,
                timestamp: now,
            },
        })
    }

    /// Makes every window operation so far durable: appends ONE log record
    /// covering the unlogged prefix, then one barrier (ADR-037). The group
    /// replays all-or-nothing after a crash.
    pub fn window_fsync(&mut self) -> Result<(), CoreError> {
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        if self.window_poisoned {
            return Err(CoreError::WindowPoisoned);
        }
        let Some(window) = self.window.as_mut() else {
            return Ok(());
        };
        if window.unlogged.is_empty() {
            self.dev.flush()?;
            return Ok(());
        }
        if window.unlogged.len() > MAX_LOG_OPS {
            return Err(CoreError::PrototypeLimit(
                "fsync group exceeds one log record; commit the window",
            ));
        }
        let geo = self.ident.geometry();
        let slots = intent_log::log_slot_lbas(&geo, self.ident.log_slots)?;
        let sequence = window.logged_records + 1;
        if sequence as usize > slots.len() {
            return Err(CoreError::PrototypeLimit(
                "intent log is full; commit the window",
            ));
        }
        let record = LogRecord {
            uuid: self.ident.uuid,
            base_generation: self.checkpoint.generation,
            sequence,
            ops: std::mem::take(&mut window.unlogged),
        };
        let encoded = record.encode(geo.block_size).map_err(CoreError::Format)?;
        let write = self
            .dev
            .write_block(slots[sequence as usize - 1], &encoded)
            .and_then(|()| self.dev.flush());
        match write {
            Ok(()) => {
                let window = self.window.as_mut().expect("window checked above");
                window.logged_records = sequence;
                for op in &record.ops {
                    if let LogOp::Create {
                        expected_object_id, ..
                    } = op
                    {
                        window.pending.logged_created.insert(*expected_object_id);
                    }
                }
                Ok(())
            }
            Err(error) => {
                self.window_poisoned = true;
                Err(error.into())
            }
        }
    }

    /// Materializes the open window as one checkpoint transaction. Always
    /// publishes a checkpoint when any record was logged, so stale records
    /// can never be mistaken for live ones.
    pub fn window_commit(&mut self, now: Timespec) -> Result<(), CoreError> {
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        if self.window_poisoned {
            return Err(CoreError::WindowPoisoned);
        }
        let Some(window) = self.window.take() else {
            return Ok(());
        };
        let force = window.logged_records > 0;
        let result =
            self.materialize_batch(window.tx, window.pending, now, window.generation, force);
        if result.is_err() {
            self.window_poisoned = true;
        }
        result
    }

    /// Replays the intent log after mount (ADR-037): re-runs the valid
    /// record prefix through the batch engine, claiming each logged create's
    /// exact data extents, and publishes one checkpoint. Returns the number
    /// of records replayed.
    pub(crate) fn recover_intent_log(&mut self) -> Result<u32, CoreError> {
        if self.ident.log_slots == 0 {
            return Ok(0);
        }
        let geo = self.ident.geometry();
        let scanned = intent_log::scan(
            &mut self.dev,
            &geo,
            self.ident.log_slots,
            &self.ident.uuid,
            self.checkpoint.generation,
        )?;
        if scanned.records.is_empty() {
            self.pending_intent_records = 0;
            return Ok(0);
        }
        let generation = self.next_generation()?;
        // Same zero-promotion rule as the live window: the recorded extents
        // are FREE in the committed bitmaps and must claim cleanly.
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            0,
            self.alloc_rover_region,
        )?;
        let mut pending = PendingBatch {
            dir_changes: BTreeMap::new(),
            dir_timestamps: BTreeMap::new(),
            records: BTreeMap::new(),
            committed_record_lbas: BTreeMap::new(),
            created_data: BTreeMap::new(),
            data_writes: Vec::new(),
            write_through: true,
            logged_created: BTreeSet::new(),
            sacrificed: Vec::new(),
            next_object_id: self.checkpoint.next_object_id,
        };
        let mut last_timestamp = Timespec::default();
        let replayed = scanned.records.len() as u32;
        for record in scanned.records {
            for op in record.ops {
                last_timestamp = op.timestamp();
                self.apply_log_op(&mut tx, &mut pending, &op, generation)?;
            }
        }
        self.materialize_batch(tx, pending, last_timestamp, generation, true)?;
        self.pending_intent_records = 0;
        Ok(replayed)
    }

    pub(crate) fn inspect_intent_log(&mut self) -> Result<u32, CoreError> {
        if self.ident.log_slots == 0 {
            self.pending_intent_records = 0;
            return Ok(0);
        }
        let scanned = intent_log::scan(
            &mut self.dev,
            &self.ident.geometry(),
            self.ident.log_slots,
            &self.ident.uuid,
            self.checkpoint.generation,
        )?;
        self.pending_intent_records = scanned.records.len() as u32;
        Ok(self.pending_intent_records)
    }

    fn apply_log_op(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        op: &LogOp,
        generation: u64,
    ) -> Result<(), CoreError> {
        let utf8 = |bytes: &[u8]| -> Result<String, CoreError> {
            String::from_utf8(bytes.to_vec())
                .map_err(|_| CoreError::Corrupt("log record name is not UTF-8".into()))
        };
        match op {
            LogOp::Create {
                parent_id,
                name,
                expected_object_id,
                size_bytes,
                extents,
                timestamp,
                ..
            } => self.apply_replay_create(
                tx,
                pending,
                *parent_id,
                &utf8(name)?,
                *expected_object_id,
                *size_bytes,
                extents,
                *timestamp,
                generation,
            ),
            LogOp::Delete {
                parent_id,
                name,
                timestamp,
            } => {
                let name = utf8(name)?;
                self.apply_batch_op(
                    tx,
                    pending,
                    &BatchOp::DeleteFile {
                        parent_id: *parent_id,
                        name: &name,
                    },
                    *timestamp,
                    generation,
                )
                .map(|_| ())
            }
            LogOp::Rename {
                source_parent_id,
                source_name,
                target_parent_id,
                target_name,
                replace,
                timestamp,
            } => {
                let source = utf8(source_name)?;
                let target = utf8(target_name)?;
                self.apply_batch_op(
                    tx,
                    pending,
                    &BatchOp::Rename {
                        source_parent_id: *source_parent_id,
                        source_name: &source,
                        target_parent_id: *target_parent_id,
                        target_name: &target,
                        replace: *replace,
                    },
                    *timestamp,
                    generation,
                )
                .map(|_| ())
            }
        }
    }

    /// The create branch of the batch engine with pre-placed content: the
    /// logged extents are claimed exactly, and the data — already on disk
    /// and CRC-verified by the scan — is never rewritten.
    #[allow(clippy::too_many_arguments)]
    fn apply_replay_create(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        parent_id: u64,
        name: &str,
        expected_object_id: u64,
        size_bytes: u64,
        extents: &[(u64, u32)],
        now: Timespec,
        generation: u64,
    ) -> Result<(), CoreError> {
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let parent = self
            .batch_record(pending, parent_id)?
            .ok_or(CoreError::NotFound)?;
        if parent.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let key = self.comparison_key(name.as_bytes())?;
        if self.batch_lookup(pending, parent_id, &key)?.is_some() {
            return Err(CoreError::Corrupt(
                "log replay found the name already present".into(),
            ));
        }
        // IDs are monotonic and never reused; scrubbed (cancelled-unlogged)
        // creates leave legal gaps that replay skips over.
        if expected_object_id < pending.next_object_id {
            return Err(CoreError::Corrupt(format!(
                "log replay expected object {expected_object_id} below allocator watermark {}",
                pending.next_object_id
            )));
        }
        let object_id = expected_object_id;
        pending.next_object_id = object_id
            .checked_add(1)
            .ok_or(CoreError::PrototypeLimit("object ID space exhausted"))?;
        let (data_start, data_blocks) = match extents {
            [] => (0u64, 0u64),
            [(start, blocks)] => {
                tx.allocate_exact_run(&mut self.dev, *start, *blocks as u64)?;
                (*start, *blocks as u64)
            }
            _ => {
                return Err(CoreError::PrototypeLimit(
                    "multi-extent logged creates are not implemented",
                ))
            }
        };
        let block_size = self.dev.block_size();
        pending
            .created_data
            .insert(object_id, (data_start, data_blocks));
        pending.records.insert(
            object_id,
            Some(ObjectRecord {
                object_id,
                object_type: ObjectType::File,
                flags: 0,
                link_count: 1,
                size_bytes,
                allocated_bytes: data_blocks * block_size as u64,
                created: now,
                modified: now,
                changed: now,
                protection: 0,
                content_generation: generation,
                data_root: data_start,
                data_blocks,
            }),
        );
        pending.dir_changes.entry(parent_id).or_default().insert(
            key,
            Some(DirEntry {
                key: self.comparison_key(name.as_bytes())?,
                name: name.as_bytes().to_vec(),
                child_type_hint: 1,
                child_id: object_id,
            }),
        );
        pending.dir_timestamps.insert(parent_id, now);
        Ok(())
    }

    /// Publishes the batch: one directory-tree mutation per touched
    /// directory, freshly encoded object records, one object-map mutation,
    /// one checkpoint.
    fn materialize_batch(
        &mut self,
        mut tx: TxAllocator,
        mut pending: PendingBatch,
        now: Timespec,
        generation: u64,
        force_commit: bool,
    ) -> Result<(), CoreError> {
        let block_size = self.dev.block_size();
        let mut meta_writes: Vec<(u64, Vec<u8>)> = Vec::new();

        for (start, blocks) in std::mem::take(&mut pending.sacrificed) {
            tx.abandon_uncommitted_run(&mut self.dev, start, blocks)?;
        }

        let dir_ids: Vec<u64> = pending.dir_changes.keys().copied().collect();
        for dir_id in dir_ids {
            let changes = pending
                .dir_changes
                .remove(&dir_id)
                .expect("key listed above");
            let committed = self
                .read_object(dir_id)?
                .ok_or_else(|| CoreError::Corrupt(format!("directory {dir_id} disappeared")))?;
            let mut encoded: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
            for (key, change) in changes {
                match change {
                    Some(entry) => {
                        let (entry_key, entry_value) =
                            directory::encode_entry(&self.ident, &entry)?;
                        debug_assert_eq!(entry_key, key);
                        encoded.push((entry_key, Some(entry_value)));
                    }
                    None => {
                        // A delete of a key with no committed entry is a
                        // same-batch create that was cancelled: skip it.
                        let existed = directory::lookup_entry(
                            &mut self.dev,
                            &self.ident.geometry(),
                            committed.data_root,
                            dir_id,
                            self.checkpoint.generation,
                            &self.ident,
                            &key,
                        )?
                        .is_some();
                        if existed {
                            encoded.push((key, None));
                        }
                    }
                }
            }
            if encoded.is_empty() {
                continue;
            }
            let operations: Vec<TreeOperation<'_>> = encoded
                .iter()
                .map(|(key, value)| match value {
                    Some(value) => TreeOperation::Upsert { key, value },
                    None => TreeOperation::Delete { key },
                })
                .collect();
            let mutation = mutate_many(
                &mut self.dev,
                &self.ident.geometry(),
                &mut tx,
                committed.data_root,
                directory::spec(dir_id, self.checkpoint.generation),
                generation,
                &operations,
            )?;
            meta_writes.extend(mutation.writes);
            self.note_committed_record(&mut pending, dir_id)?;
            let base = self
                .batch_record(&pending, dir_id)?
                .ok_or_else(|| CoreError::Corrupt(format!("directory {dir_id} disappeared")))?;
            let directory_timestamp = pending.dir_timestamps.remove(&dir_id).unwrap_or(now);
            pending.records.insert(
                dir_id,
                Some(ObjectRecord {
                    modified: directory_timestamp,
                    changed: directory_timestamp,
                    content_generation: generation,
                    data_root: mutation.root_lba,
                    ..base
                }),
            );
        }

        // Encode every surviving pending record into a fresh block and build
        // the object-map operation set.
        let mut omap_encoded: Vec<([u8; 8], Option<[u8; 8]>)> = Vec::new();
        for (object_id, record) in &pending.records {
            match record {
                Some(record) => {
                    let lba = tx.allocate(&mut self.dev)?;
                    meta_writes.push((lba, record.encode(block_size, generation)?));
                    if let Some(old) = pending.committed_record_lbas.remove(object_id) {
                        tx.retire(&mut self.dev, old)?;
                    }
                    omap_encoded.push((object_map::key(*object_id), Some(object_map::value(lba)?)));
                }
                None => {
                    omap_encoded.push((object_map::key(*object_id), None));
                }
            }
        }
        if omap_encoded.is_empty() && !force_commit {
            // Every operation cancelled out; there is no state to publish.
            return Ok(());
        }
        let omap_operations: Vec<TreeOperation<'_>> = omap_encoded
            .iter()
            .map(|(key, value)| match value {
                Some(value) => TreeOperation::Upsert { key, value },
                None => TreeOperation::Delete { key },
            })
            .collect();
        let omap_mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &omap_operations,
        )?;
        let omap_root = omap_mutation.root_lba;
        meta_writes.extend(omap_mutation.writes);

        self.commit_transaction(
            generation,
            pending.next_object_id,
            tx,
            std::mem::take(&mut pending.data_writes),
            meta_writes,
            omap_root,
        )
    }

    fn next_generation(&mut self) -> Result<u64, CoreError> {
        // A fresh transaction must never see an aborted one's half-applied
        // reference edits (ADR-061): the generation number alone cannot
        // distinguish them, because an aborted commit does not consume it.
        self.shared_refs = None;
        self.pending_layout_promotions = 0;
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

    /// Returns the mutation plus the new checkpoint's node set for the
    /// cross-commit cache.
    fn mutate_allocation_root(
        &mut self,
        generation: u64,
        dirty_records: &[(u32, afsplus_format::checkpoint::RegionRecord)],
    ) -> Result<(TreeMutation, Vec<u64>), CoreError> {
        if self.checkpoint.allocation_root_block == 0 {
            return Err(CoreError::PrototypeLimit(
                "inline allocation checkpoint cannot be mutated",
            ));
        }
        let geo = self.ident.geometry();
        let (current_blocks, older_blocks) = match self.allocation_tree_cache.take() {
            Some(cached) => cached,
            None => {
                let current = allocation_root::load_tree_blocks(
                    &mut self.dev,
                    &geo,
                    self.checkpoint.allocation_root_block,
                    self.checkpoint.generation,
                )?;
                let older = if let Some(older) = self
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
                (current, older)
            }
        };
        // The take() above cleared the cache; restore it so an aborted
        // commit leaves the committed view intact.
        self.allocation_tree_cache = Some((current_blocks.clone(), older_blocks.clone()));
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
        let mutation = mutate_many(
            &mut self.dev,
            &geo,
            &mut pool,
            self.checkpoint.allocation_root_block,
            allocation_root::spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        // New node set = current − retired paths ∪ freshly written images.
        let retired: std::collections::BTreeSet<u64> = pool.retired_nodes().collect();
        let mut new_blocks: Vec<u64> = current_blocks
            .iter()
            .copied()
            .filter(|lba| !retired.contains(lba))
            .collect();
        new_blocks.extend(mutation.writes.iter().map(|(lba, _)| *lba));
        new_blocks.sort_unstable();
        new_blocks.dedup();
        Ok((mutation, new_blocks))
    }

    /// The common commit tail: durability ordering, checkpoint write, state
    /// adoption, accounting. On any error the committed state is untouched
    /// and the in-memory volume still serves the old generation.
    #[allow(clippy::too_many_arguments)]
    fn commit_transaction(
        &mut self,
        generation: u64,
        next_object_id: u64,
        mut tx: TxAllocator,
        data_writes: Vec<(u64, Vec<u8>)>,
        mut meta_writes: Vec<(u64, Vec<u8>)>,
        new_object_map_block: u64,
    ) -> Result<(), CoreError> {
        let block_size = self.dev.block_size();

        // ADR-061: publish the shared-extent reference edit, if this
        // transaction made one, in the same checkpoint that publishes the
        // extent maps it describes. The first clone allocates the root and
        // it stays allocated afterwards, even once the tree is empty again.
        let mut shared_root = self.checkpoint.shared_extent_root_block;
        let mut shared_stats = RefEditStats::default();
        let mut shared_nodes_written = 0u64;
        if let Some((opened_for, edit)) = self.shared_refs.take() {
            if opened_for == generation {
                let publication = edit.finish()?;
                shared_stats = publication.stats;
                if publication.changed() {
                    if shared_root == 0 {
                        let node = shared_extents::initial_leaf(&publication.records)?;
                        let lba = tx.allocate(&mut self.dev)?;
                        meta_writes.push((lba, node.encode(block_size, generation)?));
                        shared_root = lba;
                        shared_nodes_written = 1;
                    } else {
                        let operations: Vec<TreeOperation<'_>> = publication
                            .deletes
                            .iter()
                            .map(|key| TreeOperation::Delete { key: &key[..] })
                            .chain(publication.upserts.iter().map(|(key, value)| {
                                TreeOperation::Upsert {
                                    key: &key[..],
                                    value: &value[..],
                                }
                            }))
                            .collect();
                        let mutation = mutate_many(
                            &mut self.dev,
                            &self.ident.geometry(),
                            &mut tx,
                            shared_root,
                            shared_extents::spec(self.checkpoint.generation),
                            generation,
                            &operations,
                        )?;
                        shared_nodes_written = mutation.writes.len() as u64;
                        meta_writes.extend(mutation.writes);
                        shared_root = mutation.root_lba;
                    }
                }
            }
        }
        let layout_promotions = std::mem::take(&mut self.pending_layout_promotions);

        let finished = tx.finish(&mut self.dev)?;
        let (allocation_root, new_allocation_tree_blocks) =
            self.mutate_allocation_root(generation, &finished.dirty_records)?;
        let new_allocation_root_block = allocation_root.root_lba;
        meta_writes.extend(allocation_root.writes);
        meta_writes.extend(finished.reclaim_writes);

        let mut stats = CommitStats {
            data_blocks_written: data_writes.len() as u64,
            metadata_blocks_written: meta_writes.len() as u64,
            bitmap_pages_written: finished.bitmap_writes.len() as u64,
            region_descriptors_written: finished.descriptor_writes.len() as u64,
            allocation_records_updated: finished.dirty_records.len() as u64,
            allocation_tree_nodes_written: allocation_root.stats.final_nodes_written,
            checkpoint_blocks_written: 1,
            shared_tree_nodes_written: shared_nodes_written,
            layout_promotions,
            shared_refs: shared_stats,
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
            reclaim_root_block: finished.reclaim_root_lba,
            next_object_id,
            committed_tx_id: generation,
            free_blocks_total,
            flags: 0,
            shared_extent_root_block: shared_root,
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
        // Rotate the allocation-root cache: the previous current tree is now
        // the retained older one.
        let previous_current = self
            .allocation_tree_cache
            .take()
            .map(|(current, _)| current)
            .unwrap_or_default();
        self.allocation_tree_cache = Some((new_allocation_tree_blocks, previous_current));
        self.alloc_rover_region = finished.rover_region;
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
