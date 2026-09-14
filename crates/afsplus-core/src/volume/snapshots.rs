//! Persistent snapshot orchestration for the ADR-071 experiment.
use super::*;
use crate::snapshot;
use crate::verify::SnapshotMountState;
use afsplus_format::snapshot::SnapshotRecord;
use afsplus_format::tree::key_u64;

const PAGE_ENTRIES: usize = 64;

/// Runtime experiment budgets, supplied explicitly until qualification selects
/// admission defaults. They do not change the on-disk representation.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotWorkLimits {
    pub max_edit_records: usize,
    pub max_views: usize,
    pub reclaim_records: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotCommitStats {
    pub records_scanned: u64,
    pub blocks_transferred: u64,
    pub scan_wrapped: bool,
    pub ledger_retired_blocks: u64,
    pub registry_nodes_written: u64,
    pub lifetime_nodes_written: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SnapshotMaintenance {
    pub blocked_by_checkpoint: bool,
    pub records_scanned: u64,
    pub blocks_transferred: u64,
    pub blocks_promoted: u64,
    pub scan_wrapped: bool,
    pub ledger_retired_blocks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub id: u64,
    pub generation: u64,
    pub committed_tx_id: u64,
}
impl SnapshotInfo {
    fn from_record(id: u64, record: SnapshotRecord) -> Self {
        Self {
            id,
            generation: record.generation,
            committed_tx_id: record.committed_tx_id,
        }
    }
}

#[derive(Debug)]
pub struct SnapshotListPage {
    pub entries: Vec<SnapshotInfo>,
    pub next_id: Option<u64>,
}

/// A runtime reader lease. Cloning keeps deletion busy; dropping the last
/// clone releases it. A handle from another mount is stale, including remounts.
#[derive(Clone)]
pub struct SnapshotHandle {
    info: SnapshotInfo,
    record: SnapshotRecord,
    lease: Arc<()>,
    mount: Arc<()>,
}
impl SnapshotHandle {
    pub fn info(&self) -> SnapshotInfo {
        self.info
    }
}
impl std::fmt::Debug for SnapshotHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotHandle")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

/// Bound to an immutable view and directory. A matching persisted snapshot
/// can resume this position after remount through a newly opened handle.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotDirectoryCursor {
    uuid: [u8; 16],
    id: u64,
    generation: u64,
    directory: u64,
    ordinal: u64,
}
#[derive(Debug)]
pub struct SnapshotDirectoryPage {
    pub entries: Vec<DirEntry>,
    pub next: SnapshotDirectoryCursor,
    pub eof: bool,
}

pub(super) enum SnapshotRegistryChange {
    Create { id: u64, record: SnapshotRecord },
    Delete { id: u64 },
}

impl<D: BlockDevice> Volume<D> {
    /// Preserve registered snapshots in either checkpoint (ADR-074). An
    /// unreadable older registry cannot authorize optional in-place writes.
    pub(super) fn snapshots_require_cow(&mut self) -> bool {
        if self.state.snapshots.is_none() {
            return false;
        }
        if self.state.snapshots.is_some_and(|state| state.views != 0) {
            return true;
        }
        let Some(older) = self.other_checkpoint.as_ref() else {
            return false;
        };
        let Some(roots) = older.snapshot_roots else {
            // The immutable snapshot feature requires roots in every valid
            // slot. Missing roots are uncertainty, not an empty registry.
            return true;
        };
        snapshot::read_registry_state(
            &mut self.dev,
            &self.ident.geometry(),
            roots.registry,
            older.generation,
        )
        .map_or(true, |(_, count, _)| count != 0)
    }

    fn snapshot_state(&self) -> Result<SnapshotMountState, CoreError> {
        self.state
            .snapshots
            .ok_or(CoreError::FeatureDisabled("persistent snapshots"))
    }
    fn snapshot_limits(&self) -> Result<SnapshotWorkLimits, CoreError> {
        self.snapshot_limits.ok_or(CoreError::PrototypeLimit(
            "configure snapshot work limits before mutation",
        ))
    }
    pub fn set_snapshot_work_limits(
        &mut self,
        limits: SnapshotWorkLimits,
    ) -> Result<(), CoreError> {
        if limits.max_edit_records == 0
            || limits.max_views == 0
            || limits.reclaim_records == 0
            || limits.reclaim_records.checked_add(1).is_none()
            || limits.reclaim_records > limits.max_edit_records
        {
            return Err(CoreError::PrototypeLimit(
                "snapshot work budgets must be positive and fit edit memory",
            ));
        }
        if self
            .state
            .snapshots
            .is_some_and(|s| s.views > limits.max_views as u64)
        {
            return Err(CoreError::PrototypeLimit(
                "snapshot budget is below registered view count",
            ));
        }
        self.snapshot_limits = Some(limits);
        Ok(())
    }

    /// Commit any open intent window, then durably register its consistent
    /// namespace. An uncertain publication poisons the Volume as usual.
    pub fn snapshot_create(&mut self, now: Timespec) -> Result<u64, CoreError> {
        metadata::validate_time(now)?;
        self.snapshot_state()?;
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        let limits = self.snapshot_limits()?;
        if self.snapshot_state()?.views >= limits.max_views as u64 {
            return Err(CoreError::PrototypeLimit(
                "snapshot admission view limit reached",
            ));
        }
        let id = self.snapshot_state()?.registry.allocate_id()?.0;
        self.window_commit(now)?;
        self.ensure_window_closed()?;
        let record = SnapshotRecord {
            generation: self.checkpoint.generation,
            committed_tx_id: self.checkpoint.committed_tx_id,
            object_map_root: self.checkpoint.object_map_block,
        };
        self.commit_snapshot_change(SnapshotRegistryChange::Create { id, record })?;
        Ok(id)
    }

    pub fn snapshot_delete(&mut self, id: u64, now: Timespec) -> Result<(), CoreError> {
        metadata::validate_time(now)?;
        self.snapshot_state()?;
        if !self.mount_mode.allows_user_writes() {
            return Err(CoreError::ReadOnly);
        }
        self.snapshot_limits()?;
        if self
            .snapshot_handles
            .get(&id)
            .is_some_and(|lease| lease.strong_count() != 0)
        {
            return Err(CoreError::Busy);
        }
        self.snapshot_record(id)?;
        self.window_commit(now)?;
        self.ensure_window_closed()?;
        self.commit_snapshot_change(SnapshotRegistryChange::Delete { id })?;
        self.snapshot_handles.remove(&id);
        Ok(())
    }

    fn commit_snapshot_change(&mut self, change: SnapshotRegistryChange) -> Result<(), CoreError> {
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
        if matches!(&change, SnapshotRegistryChange::Create { .. }) {
            self.protect_emergency_headroom(&mut tx);
        }
        self.commit_transaction_inner(
            generation,
            self.checkpoint.next_object_id,
            tx,
            vec![],
            vec![],
            self.checkpoint.object_map_block,
            Some(change),
        )
    }

    pub fn snapshot_list(
        &mut self,
        low_id: u64,
        limit: usize,
    ) -> Result<SnapshotListPage, CoreError> {
        self.snapshot_state()?;
        if limit == 0 || limit > PAGE_ENTRIES {
            return Err(CoreError::PrototypeLimit(
                "snapshot list page limit out of range",
            ));
        }
        let roots = self
            .checkpoint
            .snapshot_roots
            .expect("snapshot mount state has roots");
        let page = snapshot::read_registry_page(
            &mut self.dev,
            &self.ident.geometry(),
            roots.registry,
            self.checkpoint.generation,
            low_id,
            limit,
        )?;
        Ok(SnapshotListPage {
            entries: page
                .records
                .into_iter()
                .map(|(id, record)| SnapshotInfo::from_record(id, record))
                .collect(),
            next_id: page.next_id,
        })
    }

    fn snapshot_record(&mut self, id: u64) -> Result<SnapshotRecord, CoreError> {
        self.snapshot_state()?;
        if id == 0 {
            return Err(CoreError::NotFound);
        }
        let roots = self
            .checkpoint
            .snapshot_roots
            .expect("snapshot state has roots");
        let page = snapshot::read_registry_page(
            &mut self.dev,
            &self.ident.geometry(),
            roots.registry,
            self.checkpoint.generation,
            id,
            1,
        )?;
        page.records
            .into_iter()
            .next()
            .filter(|(found, _)| *found == id)
            .map(|(_, record)| record)
            .ok_or(CoreError::NotFound)
    }

    pub fn snapshot_open(&mut self, id: u64) -> Result<SnapshotHandle, CoreError> {
        let record = self.snapshot_record(id)?;
        let lease = self
            .snapshot_handles
            .get(&id)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| Arc::new(()));
        self.snapshot_handles.insert(id, Arc::downgrade(&lease));
        Ok(SnapshotHandle {
            info: SnapshotInfo::from_record(id, record),
            record,
            lease,
            mount: self.snapshot_mount.clone(),
        })
    }
    fn snapshot_view(&self, handle: &SnapshotHandle) -> Result<SnapshotRecord, CoreError> {
        self.snapshot_state()?;
        if !Arc::ptr_eq(&handle.mount, &self.snapshot_mount)
            || !self
                .snapshot_handles
                .get(&handle.info.id)
                .is_some_and(|lease| lease.ptr_eq(&Arc::downgrade(&handle.lease)))
        {
            return Err(CoreError::Stale);
        }
        Ok(handle.record)
    }

    pub fn snapshot_stat(
        &mut self,
        handle: &SnapshotHandle,
        object_id: u64,
    ) -> Result<Option<ObjectMetadata>, CoreError> {
        let view = self.snapshot_view(handle)?;
        Ok(
            snapshot::view::object(&mut self.dev, &self.ident, view, object_id)?
                .map(ObjectMetadata::from),
        )
    }
    pub fn snapshot_read_file_at(
        &mut self,
        handle: &SnapshotHandle,
        object_id: u64,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, CoreError> {
        let view = self.snapshot_view(handle)?;
        snapshot::view::read_at(
            &mut self.dev,
            &self.ident,
            view,
            object_id,
            offset,
            destination,
        )
    }
    pub fn snapshot_lookup(
        &mut self,
        handle: &SnapshotHandle,
        directory_id: u64,
        name: &str,
    ) -> Result<Option<u64>, CoreError> {
        let view = self.snapshot_view(handle)?;
        validate_name(name.as_bytes()).map_err(CoreError::InvalidName)?;
        let record = snapshot::view::object(&mut self.dev, &self.ident, view, directory_id)?
            .ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let key = self.comparison_key(name.as_bytes())?;
        Ok(directory::lookup_entry(
            &mut self.dev,
            &self.ident.geometry(),
            record.data_root,
            directory_id,
            view.generation,
            &self.ident,
            &key,
        )?
        .map(|entry| entry.child_id))
    }
    pub fn snapshot_read_directory_page(
        &mut self,
        handle: &SnapshotHandle,
        directory_id: u64,
        cursor: Option<SnapshotDirectoryCursor>,
        limit: usize,
    ) -> Result<SnapshotDirectoryPage, CoreError> {
        let view = self.snapshot_view(handle)?;
        if limit == 0 || limit > MAX_DIRECTORY_PAGE_ENTRIES {
            return Err(CoreError::PrototypeLimit(
                "snapshot directory page limit out of range",
            ));
        }
        let mut cursor = cursor.unwrap_or(SnapshotDirectoryCursor {
            uuid: self.ident.uuid,
            id: handle.info.id,
            generation: view.generation,
            directory: directory_id,
            ordinal: 0,
        });
        if cursor.uuid != self.ident.uuid
            || cursor.id != handle.info.id
            || cursor.generation != view.generation
            || cursor.directory != directory_id
        {
            return Err(CoreError::Stale);
        }
        let record = snapshot::view::object(&mut self.dev, &self.ident, view, directory_id)?
            .ok_or(CoreError::NotFound)?;
        if record.object_type != ObjectType::Directory {
            return Err(CoreError::NotDirectory);
        }
        let (entries, total) = directory::read_page(
            &mut self.dev,
            &self.ident.geometry(),
            record.data_root,
            directory::spec(directory_id, view.generation),
            &self.ident,
            cursor.ordinal,
            limit,
        )?;
        cursor.ordinal = cursor
            .ordinal
            .checked_add(entries.len() as u64)
            .ok_or_else(|| CoreError::Corrupt("snapshot directory cursor overflow".into()))?;
        Ok(SnapshotDirectoryPage {
            entries,
            next: cursor,
            eof: cursor.ordinal >= total,
        })
    }

    /// Maintenance reports scan progress separately from physical promotion.
    /// A wrapped pass containing protected runs may reclaim nothing.
    pub fn snapshot_maintenance_step(
        &mut self,
        now: Timespec,
    ) -> Result<SnapshotMaintenance, CoreError> {
        metadata::validate_time(now)?;
        self.snapshot_state()?;
        self.snapshot_limits()?;
        let before = self.generation();
        self.reclaim_step(now)?;
        if self.generation() == before {
            return Ok(SnapshotMaintenance {
                ledger_retired_blocks: self.snapshot_state()?.ledger.retained_blocks,
                ..Default::default()
            });
        }
        let stats = self.last_commit.expect("maintenance committed");
        Ok(SnapshotMaintenance {
            blocked_by_checkpoint: stats.alloc.reclaim.blocked_by_checkpoint,
            records_scanned: stats.snapshots.records_scanned,
            blocks_transferred: stats.snapshots.blocks_transferred,
            blocks_promoted: stats.alloc.blocks_promoted,
            scan_wrapped: stats.snapshots.scan_wrapped,
            ledger_retired_blocks: stats.snapshots.ledger_retired_blocks,
        })
    }

    pub(super) fn prepare_snapshot_registry(
        &mut self,
        tx: &mut TxAllocator,
        generation: u64,
        change: Option<SnapshotRegistryChange>,
        writes: &mut Vec<(u64, Vec<u8>)>,
    ) -> Result<u64, CoreError> {
        let Some(change) = change else {
            return Ok(0);
        };
        let roots = self
            .checkpoint
            .snapshot_roots
            .ok_or(CoreError::FeatureDisabled("persistent snapshots"))?;
        let limits = self.snapshot_limits()?;
        let (state, count, _) = snapshot::read_registry_state(
            &mut self.dev,
            &self.ident.geometry(),
            roots.registry,
            self.checkpoint.generation,
        )?;
        let mut encoded = Vec::new();
        let delete_key;
        let operations = match change {
            SnapshotRegistryChange::Create { id, record } => {
                if count >= limits.max_views as u64 {
                    return Err(CoreError::PrototypeLimit(
                        "snapshot admission view limit reached",
                    ));
                }
                let (allocated, next) = state.allocate_id()?;
                if id != allocated {
                    return Err(CoreError::Corrupt(
                        "snapshot ID changed before publication".into(),
                    ));
                }
                encoded.push((key_u64(0), next.encode()?));
                encoded.push((
                    key_u64(id),
                    record.encode(generation, self.ident.total_blocks)?,
                ));
                encoded
                    .iter()
                    .map(|(key, value)| TreeOperation::Upsert { key, value })
                    .collect::<Vec<_>>()
            }
            SnapshotRegistryChange::Delete { id } => {
                self.snapshot_record(id)?;
                if self
                    .snapshot_handles
                    .get(&id)
                    .is_some_and(|lease| lease.strong_count() != 0)
                {
                    return Err(CoreError::Busy);
                }
                delete_key = key_u64(id);
                vec![TreeOperation::Delete { key: &delete_key }]
            }
        };
        let mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            tx,
            roots.registry,
            snapshot::registry_spec(self.checkpoint.generation),
            generation,
            &operations,
        )?;
        tx.set_snapshot_registry_root(mutation.root_lba)?;
        let count = mutation.writes.len() as u64;
        writes.extend(mutation.writes);
        Ok(count)
    }

    pub(super) fn prepare_snapshot_lifetimes(
        &mut self,
        tx: &mut TxAllocator,
    ) -> Result<SnapshotCommitStats, CoreError> {
        let Some(state) = self.state.snapshots else {
            return Ok(SnapshotCommitStats::default());
        };
        let roots = self
            .checkpoint
            .snapshot_roots
            .expect("snapshot state has roots");
        let limits = self.snapshot_limits()?;
        let mut stats = SnapshotCommitStats::default();
        let mut transfers = Vec::new();
        let mut next_position = None;
        if state.ledger.retained_blocks != 0 {
            let page = snapshot::read_lifetime_page(
                &mut self.dev,
                &self.ident.geometry(),
                roots.lifetimes,
                self.checkpoint.generation,
                limits.reclaim_records,
            )?;
            stats.records_scanned = page.records.len() as u64;
            stats.scan_wrapped = page.next_position == 0;
            next_position = Some(page.next_position);
            transfers = page
                .records
                .into_iter()
                .filter(|run| run.record.retirement != 0)
                .collect();
            if !transfers.is_empty() {
                let mut next = Some(0);
                let mut seen = 0usize;
                while let Some(low) = next {
                    let views = snapshot::read_registry_page(
                        &mut self.dev,
                        &self.ident.geometry(),
                        roots.registry,
                        self.checkpoint.generation,
                        low,
                        PAGE_ENTRIES,
                    )?;
                    seen = seen
                        .checked_add(views.records.len())
                        .ok_or(CoreError::PrototypeLimit("snapshot scan count overflow"))?;
                    if views.advertised_count > limits.max_views as u64 || seen > limits.max_views {
                        return Err(CoreError::PrototypeLimit(
                            "snapshot registry scan budget exceeded",
                        ));
                    }
                    if views.next_id.is_none() && seen as u64 != views.advertised_count {
                        return Err(CoreError::Corrupt(
                            "snapshot registry count mismatch".into(),
                        ));
                    }
                    for (_, view) in views.records {
                        transfers.retain(|run| !run.record.contains(view.generation));
                    }
                    next = views.next_id;
                }
            }
        }
        stats.blocks_transferred = transfers.iter().map(|run| run.record.blocks).sum();
        tx.seal_snapshot_lifetimes(
            &mut self.dev,
            transfers,
            next_position,
            limits.max_edit_records,
            limits.max_views,
        )?;
        Ok(stats)
    }
}

#[cfg(test)]
mod tests;
