//! Exhaustive snapshot ownership verification. Never used by normal mount.
use super::*;
use crate::snapshot::{self, LifetimeRun};
use crate::tree::visit_tree_nodes;
use afsplus_format::snapshot::{decode_key, LedgerState, RegistryState, SnapshotRecord};

pub(super) fn load<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    state: &mut CommittedState,
    namespace_metadata: &[u64],
) -> Result<(), CoreError> {
    let Some(roots) = checkpoint.snapshot_roots else {
        return Ok(());
    };
    let geo = ident.geometry();
    let mut registry = None;
    let mut views = Vec::new();
    let mut housekeeping: BTreeSet<_> = state.metadata_blocks.iter().copied().collect();
    for lba in namespace_metadata {
        housekeeping.remove(lba);
    }
    housekeeping.extend(state.allocation_pool_blocks.iter().copied());
    housekeeping.extend(state.log_area_blocks.iter().copied());
    let mut tree_blocks = Vec::new();
    visit_tree_nodes(
        dev,
        &geo,
        roots.registry,
        snapshot::registry_spec(checkpoint.generation),
        |lba, node| {
            tree_blocks.push(lba);
            if node.is_leaf() {
                for item in &node.items {
                    let id = decode_key(&item.key)?;
                    if id == 0 {
                        registry = Some(RegistryState::decode(&item.value)?);
                    } else {
                        views.push((
                            id,
                            SnapshotRecord::decode(
                                &item.value,
                                checkpoint.generation,
                                geo.total_blocks,
                            )?,
                        ));
                    }
                }
            }
            Ok(())
        },
    )?;
    let registry =
        registry.ok_or_else(|| CoreError::Corrupt("snapshot registry lacks control".into()))?;
    if views.iter().any(|(id, _)| *id >= registry.next_id) {
        return Err(CoreError::Corrupt(
            "snapshot ID outside registry control".into(),
        ));
    }
    let mut ledger = None;
    let mut runs = Vec::new();
    visit_tree_nodes(
        dev,
        &geo,
        roots.lifetimes,
        snapshot::lifetime_spec(checkpoint.generation),
        |lba, node| {
            tree_blocks.push(lba);
            if node.is_leaf() {
                for item in &node.items {
                    if decode_key(&item.key)? == 0 {
                        ledger = Some(LedgerState::decode(&item.value, geo.total_blocks)?);
                    } else {
                        runs.push(snapshot::lifetime(
                            &item.key,
                            &item.value,
                            &geo,
                            checkpoint.generation,
                        )?);
                    }
                }
            }
            Ok(())
        },
    )?;
    let ledger =
        ledger.ok_or_else(|| CoreError::Corrupt("snapshot ledger lacks control".into()))?;
    for pair in runs.windows(2) {
        snapshot::adjacent(pair[0], pair[1])?;
    }
    let retained = runs
        .iter()
        .filter(|run| run.record.retirement != 0)
        .try_fold(0u64, |sum, run| sum.checked_add(run.record.blocks))
        .ok_or_else(|| CoreError::Corrupt("snapshot retained sum overflow".into()))?;
    if retained != ledger.retained_blocks {
        return Err(CoreError::Corrupt(
            "snapshot retained total does not match ledger".into(),
        ));
    }
    let live_metadata: BTreeSet<_> = namespace_metadata.iter().copied().collect();
    let live_data: BTreeSet<_> = state.data_blocks.iter().copied().collect();
    let live: BTreeSet<_> = live_metadata.union(&live_data).copied().collect();
    for lba in &tree_blocks {
        snapshot::namespace_range(&geo, *lba, 1)?;
        if !housekeeping.insert(*lba) || live.contains(lba) {
            return Err(CoreError::Corrupt(format!(
                "snapshot housekeeping block {lba} aliases another owner"
            )));
        }
    }
    for lba in &live {
        require_lifetime(&runs, *lba, checkpoint.generation, true)?;
    }
    let quarantine: BTreeSet<_> = state
        .reclaim_runs
        .iter()
        .flat_map(|run| run.start..run.start + u64::from(run.blocks))
        .collect();
    for run in &runs {
        for lba in run.start..run.start + run.record.blocks {
            if housekeeping.contains(&lba) || quarantine.contains(&lba) {
                return Err(CoreError::Corrupt(format!(
                    "snapshot lifetime block {lba} aliases housekeeping or quarantine"
                )));
            }
            if (run.record.retirement == 0) != live.contains(&lba) {
                return Err(CoreError::Corrupt(format!(
                    "snapshot lifetime block {lba} has inconsistent live ownership"
                )));
            }
            state.snapshot_owned_blocks.push(lba);
        }
    }
    let mut all_metadata = live_metadata;
    let mut all_data = live_data;
    for (id, view) in views {
        let historical = historical_namespace(dev, ident, checkpoint, view)
            .map_err(|error| CoreError::Corrupt(format!("snapshot {id}: {error}")))?;
        for lba in historical.metadata.iter().chain(historical.data.iter()) {
            if housekeeping.contains(lba) || quarantine.contains(lba) {
                return Err(CoreError::Corrupt(format!(
                    "snapshot {id} block {lba} aliases housekeeping or quarantine"
                )));
            }
            require_lifetime(&runs, *lba, view.generation, false)?;
        }
        if let Some(lba) = historical
            .metadata
            .intersection(&all_data)
            .next()
            .or_else(|| historical.data.intersection(&all_metadata).next())
        {
            return Err(CoreError::Corrupt(format!(
                "snapshot {id} block {lba} aliases metadata and data across views"
            )));
        }
        all_metadata.extend(historical.metadata);
        all_data.extend(historical.data);
    }
    // Metadata is immutable COW: its header generation is independently
    // observable allocation birth. Raw data has no such embedded witness.
    let mut buf = vec![0; geo.block_size];
    for &lba in &all_metadata {
        dev.read_block(lba, &mut buf)?;
        let magic = u32::from_le_bytes(buf[..4].try_into().expect("metadata header"));
        let header = afsplus_format::header::BlockHeader::verify(&buf, magic)?;
        let run = lifetime_at(&runs, lba)?;
        if header.generation != run.record.birth {
            return Err(CoreError::Corrupt(format!(
                "snapshot metadata block {lba} birth disagrees with header generation"
            )));
        }
    }
    state.metadata_blocks.extend(all_metadata);
    state.metadata_blocks.extend(tree_blocks);
    state.metadata_blocks.sort_unstable();
    state.metadata_blocks.dedup();
    state.data_blocks = all_data.into_iter().collect();
    Ok(())
}

fn lifetime_at(runs: &[LifetimeRun], lba: u64) -> Result<&LifetimeRun, CoreError> {
    let index = runs.partition_point(|run| run.start <= lba);
    index
        .checked_sub(1)
        .and_then(|index| runs.get(index))
        .filter(|run| lba < run.start + run.record.blocks)
        .ok_or_else(|| {
            CoreError::Corrupt(format!("namespace block {lba} has no snapshot lifetime"))
        })
}
fn require_lifetime(
    runs: &[LifetimeRun],
    lba: u64,
    generation: u64,
    live: bool,
) -> Result<(), CoreError> {
    let run = lifetime_at(runs, lba)?;
    if !run.record.contains(generation) || (live && run.record.retirement != 0) {
        return Err(CoreError::Corrupt(format!(
            "namespace block {lba} is outside its snapshot lifetime at generation {generation}"
        )));
    }
    Ok(())
}

struct HistoricalNamespace {
    metadata: BTreeSet<u64>,
    data: BTreeSet<u64>,
}

fn historical_namespace<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    checkpoint: &Checkpoint,
    view: SnapshotRecord,
) -> Result<HistoricalNamespace, CoreError> {
    let geo = ident.geometry();
    let map = object_map::load_all(dev, &geo, view.object_map_root, view.generation)?;
    let mut metadata: BTreeSet<_> = map.tree_blocks.into_iter().collect();
    let mut data = BTreeMap::<u64, bool>::new();
    let mut objects = BTreeMap::new();
    let mut directories = BTreeMap::new();
    let mut buf = vec![0; geo.block_size];
    let claim_meta = |lba, metadata: &mut BTreeSet<u64>| -> Result<(), CoreError> {
        snapshot::namespace_range(&geo, lba, 1)?;
        if !metadata.insert(lba) {
            return Err(CoreError::Corrupt(format!(
                "historical metadata block {lba} referenced twice"
            )));
        }
        Ok(())
    };
    let claim_data = |start: u64,
                      blocks: u64,
                      shared: bool,
                      data: &mut BTreeMap<u64, bool>|
     -> Result<(), CoreError> {
        if blocks == 0 {
            return Ok(());
        }
        snapshot::namespace_range(&geo, start, blocks)?;
        for lba in start..start + blocks {
            if data
                .insert(lba, shared)
                .is_some_and(|previous| !previous || !shared)
            {
                return Err(CoreError::Corrupt(format!(
                    "historical data block {lba} shared without markers"
                )));
            }
        }
        Ok(())
    };
    for entry in map.entries {
        validate_mapped_object_id(
            entry.object_id,
            checkpoint,
            ident.features.ro_compat & RO_COMPAT_ORPHAN_DIRECTORY != 0,
        )?;
        claim_meta(entry.block, &mut metadata)?;
        dev.read_block(entry.block, &mut buf)?;
        let (record, generation) = ObjectRecord::decode_metadata_with_generation(&buf)?;
        if generation == 0 || generation > view.generation || record.object_id != entry.object_id {
            return Err(CoreError::Corrupt(
                "historical object identity or generation mismatch".into(),
            ));
        }
        if record.flags & OBJECT_FLAG_DATA_IN_PLACE != 0
            && ident.features.compat & COMPAT_DATA_POLICY == 0
        {
            return Err(CoreError::Corrupt(
                "historical object has unnegotiated data policy".into(),
            ));
        }
        if record.object_id == OBJECT_ORPHAN_DIRECTORY
            && (record.object_type != ObjectType::Directory
                || record.flags != 0
                || record.link_count != 1
                || record.size_bytes != 0
                || record.allocated_bytes != 0
                || record.data_blocks != 0)
        {
            return Err(CoreError::Corrupt(
                "historical orphan directory has invalid metadata".into(),
            ));
        }
        // A retained record keeps its owned chains: each must still prove
        // itself at the view's generation, and its segments are historical
        // metadata like the record that names them (ADR-109).
        if let Some(reference) = record.security {
            if ident.features.incompat & INCOMPAT_SECURITY_DESCRIPTORS == 0 {
                return Err(CoreError::Corrupt(
                    "historical object has an unnegotiated security reference".into(),
                ));
            }
            let (segments, _) = crate::volume::load_descriptor_chain(
                dev,
                &geo,
                record.object_id,
                reference,
                view.generation,
            )?;
            for lba in segments {
                claim_meta(lba, &mut metadata)?;
            }
        }
        if let Some(reference) = record.attributes {
            let (segments, _) = crate::volume::load_attribute_chain(
                dev,
                &geo,
                record.object_id,
                reference,
                view.generation,
            )?;
            for lba in segments {
                claim_meta(lba, &mut metadata)?;
            }
        }
        match record.object_type {
            ObjectType::Directory => {
                let dir = directory::load_all(
                    dev,
                    &geo,
                    record.data_root,
                    record.object_id,
                    view.generation,
                    ident,
                )?;
                for lba in &dir.tree_blocks {
                    claim_meta(*lba, &mut metadata)?;
                }
                directories.insert(record.object_id, dir);
            }
            ObjectType::File if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 => {
                let map = extent_map::load_all(
                    dev,
                    &geo,
                    record.data_root,
                    record.object_id,
                    view.generation,
                )?;
                if map.allocated_blocks != record.data_blocks {
                    return Err(CoreError::Corrupt(
                        "historical extent allocation count mismatch".into(),
                    ));
                }
                for lba in map.tree_blocks {
                    claim_meta(lba, &mut metadata)?;
                }
                for extent in map.extents {
                    let shared = extent.flags & extent_map::EXTENT_SHARED != 0;
                    if shared && ident.features.ro_compat & RO_COMPAT_SHARED_EXTENTS == 0 {
                        return Err(CoreError::Corrupt(
                            "historical shared extent without feature".into(),
                        ));
                    }
                    claim_data(extent.physical_start, extent.block_count, shared, &mut data)?;
                }
            }
            ObjectType::File => {
                claim_data(record.data_root, record.data_blocks, false, &mut data)?;
            }
            ObjectType::Symlink => {}
            _ => unreachable!("object decoder rejects unsupported types"),
        }
        objects.insert(record.object_id, record);
    }
    validate_namespace_graph(&objects, &directories)?;
    let mut references = BTreeMap::<u64, u64>::new();
    references.insert(OBJECT_ROOT, 1);
    if objects.contains_key(&OBJECT_ORPHAN_DIRECTORY) {
        references.insert(OBJECT_ORPHAN_DIRECTORY, 1);
    }
    for dir in directories.values() {
        for entry in &dir.entries {
            *references.entry(entry.child_id).or_default() += 1;
        }
    }
    for (id, record) in objects {
        let count = references.get(&id).copied().unwrap_or(0);
        if count == 0 || count != u64::from(record.link_count) {
            return Err(CoreError::Corrupt(format!(
                "historical object {id} link count mismatch"
            )));
        }
    }
    for lba in &metadata {
        snapshot::namespace_range(&geo, *lba, 1)?;
        if data.contains_key(lba) {
            return Err(CoreError::Corrupt(format!(
                "historical block {lba} aliases metadata and data"
            )));
        }
    }
    Ok(HistoricalNamespace {
        metadata,
        data: data.into_keys().collect(),
    })
}
