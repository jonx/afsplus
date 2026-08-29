//! Mounted volume and the first writable COW transaction.

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::{comparison_key, DirEntry};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::{validate_name, Timespec, OBJECT_ROOT};

use crate::verify::{validate_checkpoint_reachable, ReachableState};
use crate::CoreError;

pub struct Volume<D: BlockDevice> {
    dev: D,
    ident: Identification,
    checkpoint: Checkpoint,
    /// Which checkpoint slot holds the committed state (0 = A, 1 = B).
    current_slot: usize,
    state: ReachableState,
}

impl<D: BlockDevice> Volume<D> {
    pub(crate) fn new(
        dev: D,
        ident: Identification,
        checkpoint: Checkpoint,
        current_slot: usize,
        state: ReachableState,
    ) -> Self {
        Volume { dev, ident, checkpoint, current_slot, state }
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

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    pub fn into_device(self) -> D {
        self.dev
    }

    /// Creates an empty file in the root directory: the first writable
    /// transaction (`implementation/peer-review-prototype-plan.md`, step 4).
    ///
    /// The transaction writes four fresh COW blocks (file record, new root
    /// directory block, new root object record, new object map), a barrier,
    /// the alternate checkpoint at generation + 1, and a final barrier. On
    /// any error before the checkpoint barrier completes, the committed state
    /// is untouched and the in-memory volume still serves the old generation.
    pub fn create_file_in_root(&mut self, name: &str, now: Timespec) -> Result<u64, CoreError> {
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
        let generation = self.checkpoint.generation + 1;
        let object_id = self.checkpoint.next_object_id;

        // Bootstrap bump allocation: four fresh blocks strictly above the
        // committed high-water mark. Nothing reachable from the committed
        // checkpoint is ever written.
        let base = self.checkpoint.next_free_block;
        if base + 4 > self.ident.total_blocks {
            return Err(CoreError::NoSpace);
        }
        let (file_lba, dir_lba, root_lba, omap_lba) = (base, base + 1, base + 2, base + 3);

        let file_record = ObjectRecord {
            object_id,
            object_type: ObjectType::File,
            flags: 0,
            link_count: 1,
            size_bytes: 0,
            allocated_bytes: 0,
            created: now,
            modified: now,
            changed: now,
            protection: 0,
            content_generation: generation,
            data_root: 0,
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
        new_omap.upsert(OBJECT_ROOT, root_lba)?;
        new_omap.upsert(object_id, file_lba)?;

        // 1. COW metadata to fresh blocks.
        self.dev.write_block(file_lba, &file_record.encode(block_size, generation)?)?;
        self.dev.write_block(dir_lba, &new_dir.encode(block_size, generation)?)?;
        self.dev.write_block(root_lba, &new_root.encode(block_size, generation)?)?;
        self.dev.write_block(omap_lba, &new_omap.encode(block_size, generation)?)?;
        // 2. Barrier: metadata durable before any checkpoint can reference it.
        self.dev.flush()?;

        // 3. Alternate checkpoint slot, generation + 1.
        let new_slot = 1 - self.current_slot;
        let new_checkpoint = Checkpoint {
            uuid: self.ident.uuid,
            generation,
            root_object_id: OBJECT_ROOT,
            object_map_block: omap_lba,
            next_free_block: base + 4,
            next_object_id: object_id + 1,
            committed_tx_id: generation,
            flags: 0,
        };
        self.dev
            .write_block(self.ident.checkpoint_slots[new_slot], &new_checkpoint.encode(block_size)?)?;
        // 4. Barrier: only after this may the commit be reported durable.
        self.dev.flush()?;

        // Adopt the committed state. Re-walking from disk (rather than
        // patching caches) keeps the in-memory state provably equal to what a
        // remount would see; acceptable at prototype scale.
        self.checkpoint = new_checkpoint;
        self.current_slot = new_slot;
        self.state = validate_checkpoint_reachable(&mut self.dev, &self.ident, &new_checkpoint)?;

        Ok(object_id)
    }
}
