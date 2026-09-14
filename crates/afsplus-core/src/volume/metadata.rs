//! Trusted metadata mutation and exact restoration using the common COW tail.
use super::*;

/// Existing protection and timestamp fields that can be preserved on restore.
/// Object identity, data layout, link count and content generation are managed
/// by destination operations and cannot be supplied through this structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreservedMetadata {
    pub protection: u32,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
}
impl From<ObjectMetadata> for PreservedMetadata {
    fn from(record: ObjectMetadata) -> Self {
        Self {
            protection: record.protection,
            created: record.created,
            modified: record.modified,
            changed: record.changed,
        }
    }
}
impl PreservedMetadata {
    fn validate(&self) -> Result<(), CoreError> {
        for time in [self.created, self.modified, self.changed] {
            validate_time(time)?;
        }
        Ok(())
    }
}
pub(super) fn validate_time(time: Timespec) -> Result<(), CoreError> {
    time.validate()
        .map_err(|_| CoreError::InvalidMetadata("timestamp nanoseconds out of range"))
}

impl<D: BlockDevice> Volume<D> {
    /// Change existing protection bits, recording the supplied change time.
    /// The trusted host evaluates permission; the core does not reinterpret
    /// these bits as a POSIX mode or a rich ACL. An unchanged value is a no-op.
    pub fn set_object_protection(
        &mut self,
        object_id: u64,
        protection: u32,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        self.ensure_public_object_id(object_id)?;
        validate_time(now)?;
        let mut record = self.metadata_target(object_id)?;
        if record.protection == protection {
            return Ok(());
        }
        record.protection = protection;
        record.changed = now;
        self.commit_object_metadata(record)
    }

    /// Exact restoration of the existing protection and timestamp fields.
    /// Requires independently authorized host restore access before invocation.
    /// Unlike ordinary metadata edits, this preserves the archived change time.
    pub fn restore_object_metadata(
        &mut self,
        object_id: u64,
        metadata: PreservedMetadata,
    ) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        self.ensure_public_object_id(object_id)?;
        metadata.validate()?;
        let mut record = self.metadata_target(object_id)?;
        if PreservedMetadata::from(ObjectMetadata::from(record)) == metadata {
            return Ok(());
        }
        record.protection = metadata.protection;
        record.created = metadata.created;
        record.modified = metadata.modified;
        record.changed = metadata.changed;
        self.commit_object_metadata(record)
    }

    fn metadata_target(&mut self, object_id: u64) -> Result<ObjectRecord, CoreError> {
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type == ObjectType::Internal {
            return Err(CoreError::NotFound);
        }
        Ok(record)
    }

    pub(super) fn commit_object_metadata(&mut self, record: ObjectRecord) -> Result<(), CoreError> {
        // Validate before allocation, retirement or publication. This also
        // protects the existing per-file data-policy mutation path.
        PreservedMetadata::from(ObjectMetadata::from(record)).validate()?;
        let object_id = record.object_id;
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("object {object_id} missing from object map"))
        })?;
        let generation = self.next_generation()?;
        let encoded = record.encode(self.dev.block_size(), generation)?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?;
        self.protect_emergency_headroom(&mut tx);
        let new_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, record_lba)?;
        let key = object_map::key(object_id);
        let value = object_map::value(new_lba)?;
        let mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &key,
                value: &value,
            }],
        )?;
        let mut writes = vec![(new_lba, encoded)];
        writes.extend(mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            writes,
            mutation.root_lba,
        )
    }
}
