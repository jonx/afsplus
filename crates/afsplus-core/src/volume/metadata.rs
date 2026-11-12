//! Trusted metadata mutation and exact restoration using the common COW tail.
use super::*;

/// Existing protection, owner and timestamp fields that can be preserved on
/// restore. The comment is restored on its own, through
/// `set_object_comment`, before these fields, so that the change time it
/// advances is then restored too. Object identity, data layout, link count and
/// content generation are managed by destination operations and cannot be
/// supplied through this structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreservedMetadata {
    pub protection: u32,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
}
impl From<ObjectMetadata> for PreservedMetadata {
    fn from(record: ObjectMetadata) -> Self {
        Self {
            protection: record.protection,
            owner_uid: record.owner_uid,
            owner_gid: record.owner_gid,
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
        self.trace_api(crate::flight::ApiMethod::SetObjectProtection, |volume| {
            volume.set_object_protection_untraced(object_id, protection, now)
        })
    }

    fn set_object_protection_untraced(
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
        self.project_protection(&mut record, protection)?;
        record.changed = now;
        self.commit_object_metadata(record)
    }

    /// Change the POSIX owner, recording the supplied change time. The core
    /// stores the identities and evaluates nothing: who may set them is the
    /// trusted host's question, as it is for the protection word. Zero is
    /// root and a legitimate value, so there is no "unset" to special-case.
    /// An unchanged pair is a no-op.
    pub fn set_object_owner(
        &mut self,
        object_id: u64,
        owner_uid: u32,
        owner_gid: u32,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetObjectOwner, |volume| {
            volume.set_object_owner_untraced(object_id, owner_uid, owner_gid, now)
        })
    }

    fn set_object_owner_untraced(
        &mut self,
        object_id: u64,
        owner_uid: u32,
        owner_gid: u32,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        self.ensure_public_object_id(object_id)?;
        validate_time(now)?;
        let mut record = self.metadata_target(object_id)?;
        if record.owner_uid == owner_uid && record.owner_gid == owner_gid {
            return Ok(());
        }
        record.owner_uid = owner_uid;
        record.owner_gid = owner_gid;
        record.changed = now;
        self.commit_object_metadata(record)
    }

    /// Set the modification time a caller names, as `utimes` does. The
    /// metadata-change time becomes `now`, because changing the modification
    /// time IS a metadata change; POSIX does not let a caller choose it.
    ///
    /// There is no access time in this format and none is invented here: the
    /// adapters report the modification time for it, which is what a volume
    /// that does not track access time has to say.
    pub fn set_object_times(
        &mut self,
        object_id: u64,
        modified: Timespec,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetObjectTimes, |volume| {
            volume.set_object_times_untraced(object_id, modified, now)
        })
    }

    fn set_object_times_untraced(
        &mut self,
        object_id: u64,
        modified: Timespec,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        self.ensure_public_object_id(object_id)?;
        validate_time(modified)?;
        validate_time(now)?;
        let mut record = self.metadata_target(object_id)?;
        if record.modified == modified {
            return Ok(());
        }
        record.modified = modified;
        record.changed = now;
        self.commit_object_metadata(record)
    }

    /// Exact restoration of the existing protection, owner and timestamp fields.
    /// Requires independently authorized host restore access before invocation.
    /// Unlike ordinary metadata edits, this preserves the archived change time.
    pub fn restore_object_metadata(
        &mut self,
        object_id: u64,
        metadata: PreservedMetadata,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::RestoreObjectMetadata, |volume| {
            volume.restore_object_metadata_untraced(object_id, metadata)
        })
    }

    fn restore_object_metadata_untraced(
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
        self.project_protection(&mut record, metadata.protection)?;
        record.owner_uid = metadata.owner_uid;
        record.owner_gid = metadata.owner_gid;
        record.created = metadata.created;
        record.modified = metadata.modified;
        record.changed = metadata.changed;
        self.commit_object_metadata(record)
    }

    /// The comment of `object_id`; empty when it has none (ADR-106).
    pub fn object_comment(&mut self, object_id: u64) -> Result<String, CoreError> {
        self.trace_api(crate::flight::ApiMethod::ObjectComment, |volume| {
            volume.ensure_public_object_id(object_id)?;
            Ok(volume
                .metadata_target(object_id)?
                .comment
                .as_str()
                .to_owned())
        })
    }

    /// Set or, with an empty string, remove the comment of `object_id`: one
    /// metadata commit that rewrites the object record, so a power cut
    /// leaves the old comment or the new one. At most 255 bytes of UTF-8
    /// without NUL. An unchanged comment is a no-op. The change time
    /// advances; the modification time stays, the content did not change.
    pub fn set_object_comment(
        &mut self,
        object_id: u64,
        comment: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetObjectComment, |volume| {
            volume.ensure_window_closed()?;
            volume.ensure_public_object_id(object_id)?;
            validate_time(now)?;
            let comment = afsplus_format::object::Comment::new(comment)
                .map_err(|_| CoreError::InvalidMetadata("object comment out of range"))?;
            let record = volume.metadata_target(object_id)?;
            if record.comment == comment {
                return Ok(());
            }
            let mut record = record.with_comment(comment);
            record.changed = now;
            volume.commit_object_metadata(record)
        })
    }

    /// The committed volume label. The identification block keeps the label
    /// given at format time; this is the current one (ADR-104).
    pub fn volume_label(&self) -> &str {
        &self.checkpoint.label
    }

    /// Relabel the volume: one commit whose checkpoint carries the new
    /// label, so a power cut leaves the old label or the new one. The rule
    /// is the formatter's: at most 64 bytes of UTF-8 without NUL. An
    /// unchanged label is a no-op.
    pub fn set_volume_label(&mut self, label: &str) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetVolumeLabel, |volume| {
            volume.set_volume_label_untraced(label)
        })
    }

    fn set_volume_label_untraced(&mut self, label: &str) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        afsplus_format::ident::validate_label(label)
            .map_err(|_| CoreError::InvalidMetadata("volume label out of range"))?;
        if label == self.checkpoint.label {
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
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);
        self.commit_transaction_inner(
            generation,
            self.checkpoint.next_object_id,
            tx,
            CommitData::Prepared(Vec::new()),
            Vec::new(),
            self.checkpoint.object_map_block,
            None,
            Some(label),
        )
    }

    /// [`Self::set_object_protection`] staged in the open window (ADR-121).
    /// See [`Self::window_edit_record`].
    pub fn window_set_object_protection(
        &mut self,
        object_id: u64,
        protection: u32,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(
            crate::flight::ApiMethod::WindowSetObjectProtection,
            |volume| {
                volume.window_edit_record(object_id, now, false, |volume, record| {
                    if record.protection == protection {
                        return Ok(false);
                    }
                    volume.project_protection(record, protection)?;
                    Ok(true)
                })
            },
        )
    }

    /// [`Self::set_object_owner`] staged in the open window.
    pub fn window_set_object_owner(
        &mut self,
        object_id: u64,
        owner_uid: u32,
        owner_gid: u32,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowSetObjectOwner, |volume| {
            volume.window_edit_record(object_id, now, false, |_, record| {
                if record.owner_uid == owner_uid && record.owner_gid == owner_gid {
                    return Ok(false);
                }
                record.owner_uid = owner_uid;
                record.owner_gid = owner_gid;
                Ok(true)
            })
        })
    }

    /// [`Self::set_object_times`] staged in the open window.
    pub fn window_set_object_times(
        &mut self,
        object_id: u64,
        modified: Timespec,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowSetObjectTimes, |volume| {
            validate_time(modified)?;
            volume.window_edit_record(object_id, now, true, |_, record| {
                if record.modified == modified {
                    return Ok(false);
                }
                record.modified = modified;
                Ok(true)
            })
        })
    }

    /// [`Self::set_object_comment`] staged in the open window.
    pub fn window_set_object_comment(
        &mut self,
        object_id: u64,
        comment: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowSetObjectComment, |volume| {
            let comment = afsplus_format::object::Comment::new(comment)
                .map_err(|_| CoreError::InvalidMetadata("object comment out of range"))?;
            volume.window_edit_record(object_id, now, false, |_, record| {
                if record.comment == comment {
                    return Ok(false);
                }
                *record = record.with_comment(comment);
                Ok(true)
            })
        })
    }

    /// Stages one edit of an object record in the open window, opening it
    /// if needed (ADR-121 decision 8). The record is rewritten in memory
    /// only; the window's commit publishes it with everything else the
    /// window holds, so a power cut leaves every staged change of the window
    /// or none, and a later operation never survives an earlier one. No log
    /// record describes the edit, so an fsync commits the window instead of
    /// logging it. `edit` answers whether it changed anything; an unchanged
    /// record stages nothing. A symlink, whose record block also holds its
    /// target, and the root are [`CoreError::PrototypeLimit`]: the caller
    /// commits the window and edits immediately. So is a time edit of a
    /// directory with no settable record yet.
    fn window_edit_record(
        &mut self,
        object_id: u64,
        now: Timespec,
        sets_time: bool,
        edit: impl FnOnce(&Self, &mut ObjectRecord) -> Result<bool, CoreError>,
    ) -> Result<(), CoreError> {
        validate_time(now)?;
        self.ensure_public_object_id(object_id)?;
        let mut window = self.take_or_open_window()?;
        // Nothing below writes to the device or the allocator, so any
        // refusal leaves the window exactly as it was.
        let result = (|| {
            let mut record = self.window_editable_record(&window, object_id)?;
            if !edit(self, &mut record)? {
                return Ok(());
            }
            record.changed = now;
            if sets_time && record.object_type == ObjectType::Directory {
                // The staged time must outlast the entry changes before it,
                // which materialization would otherwise stamp over it.
                window.pending.dir_timestamps.remove(&object_id);
                window.pending.dir_times_set.insert(object_id);
            }
            self.window_stage_record(&mut window, record)
        })();
        self.window = Some(window);
        result
    }

    /// The record of `object_id` as the window leaves it, if the window may
    /// stage an edit of it.
    pub(super) fn window_editable_record(
        &mut self,
        window: &OpenWindow,
        object_id: u64,
    ) -> Result<ObjectRecord, CoreError> {
        let record = self
            .batch_record(&window.pending, object_id)?
            .ok_or(CoreError::NotFound)?;
        let record = Self::as_window_leaves_it(window, object_id, record);
        match record.object_type {
            ObjectType::Internal => Err(CoreError::NotFound),
            ObjectType::Symlink => Err(CoreError::PrototypeLimit(
                "a symlink record is rewritten with its target, outside the window",
            )),
            _ if object_id == OBJECT_ROOT => Err(CoreError::PrototypeLimit(
                "the root record is edited outside the window",
            )),
            _ => Ok(record),
        }
    }

    /// Puts an edited record into the window, remembering the committed
    /// block it replaces so that the commit retires exactly that one.
    pub(super) fn window_stage_record(
        &mut self,
        window: &mut OpenWindow,
        record: ObjectRecord,
    ) -> Result<(), CoreError> {
        PreservedMetadata::from(ObjectMetadata::from(record)).validate()?;
        let object_id = record.object_id;
        if let std::collections::btree_map::Entry::Vacant(slot) =
            window.pending.committed_record_lbas.entry(object_id)
        {
            // An object the window created has no committed block.
            if let Some(lba) = self.object_record_lba(object_id)? {
                slot.insert(lba);
            }
        }
        window.pending.records.insert(object_id, Some(record));
        window.unloggable = true;
        Ok(())
    }

    fn metadata_target(&mut self, object_id: u64) -> Result<ObjectRecord, CoreError> {
        let record = self.visible_record(object_id)?.ok_or(CoreError::NotFound)?;
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
        let encoded = self.encode_preserving_target(record, generation)?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
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
