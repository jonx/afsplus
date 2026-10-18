//! ADR-133: bounded reclamation while a restore object remains private.
use super::*;

impl<D: BlockDevice> Volume<D> {
    /// Trusted maintenance only. Each step releases at most `max_extents`
    /// logical extent records, preserving snapshot references and the active
    /// marker. The namespace entry disappears only after its data is empty.
    pub fn discard_restore_file_step(
        &mut self,
        parent: u64,
        name: &str,
        expected: u64,
        root: u64,
        max_extents: usize,
        now: Timespec,
    ) -> Result<OrphanCleanupProgress, CoreError> {
        self.trace_api(crate::flight::ApiMethod::DiscardRestoreFileStep, |volume| {
            volume.discard_restore_file_step_untraced(
                parent,
                name,
                expected,
                root,
                max_extents,
                now,
            )
        })
    }

    fn discard_restore_file_step_untraced(
        &mut self,
        parent: u64,
        name: &str,
        expected: u64,
        root: u64,
        max_extents: usize,
        now: Timespec,
    ) -> Result<OrphanCleanupProgress, CoreError> {
        metadata::validate_time(now)?;
        self.ensure_window_closed()?;
        if self.ident.features.incompat & afsplus_format::ident::INCOMPAT_PERSISTENT_SNAPSHOTS == 0
        {
            return Err(CoreError::FeatureDisabled(
                "restore recovery requires persistent snapshots",
            ));
        }
        if max_extents == 0 || max_extents > 64 {
            return Err(CoreError::PrototypeLimit(
                "restore discard extent budget invalid",
            ));
        }
        if self.lookup_in_directory(parent, name)? != Some(expected)
            || self.attribute(expected, "system.restore.stage")?
                != Some(root.to_le_bytes().to_vec())
            || self.attribute(root, "system.restore.stage")? != Some(root.to_le_bytes().to_vec())
        {
            return Err(CoreError::InvalidMetadata(
                "restore discard identity or marker mismatch",
            ));
        }
        let step = self.cleanup_orphan_data_step(expected, max_extents, now)?;
        if !step.data_empty {
            return Ok(OrphanCleanupProgress {
                extents_removed: step.extents_removed,
                object_removed: false,
                still_pending: true,
            });
        }
        self.delete_file(parent, name, now)?;
        Ok(OrphanCleanupProgress {
            extents_removed: step.extents_removed,
            object_removed: true,
            still_pending: false,
        })
    }
}
