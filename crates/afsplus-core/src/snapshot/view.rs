//! Namespace reads bound to a captured object-map root and generation.
//! Live caches, intent overlays and live shared-reference counts are excluded.
use super::namespace_range;
use crate::{directory, extent_map, object_map, CoreError};
use afsplus_block::BlockDevice;
use afsplus_format::ident::{Identification, COMPAT_DATA_POLICY};
use afsplus_format::object::{
    ObjectRecord, ObjectType, OBJECT_FLAG_DATA_IN_PLACE, OBJECT_FLAG_EXTENT_TREE,
};
use afsplus_format::snapshot::SnapshotRecord;
use afsplus_format::OBJECT_ORPHAN_DIRECTORY;

/// Borrowed observation owner for one captured-view operation. No shared global state.
pub(crate) struct Observation<'a> {
    view: SnapshotRecord,
    snapshot_id: u64,
    recorder: Option<&'a mut crate::flight::FlightRecorder>,
    requires_remount: bool,
}
impl<'a> Observation<'a> {
    pub(crate) fn new(
        view: SnapshotRecord,
        snapshot_id: u64,
        recorder: Option<&'a mut crate::flight::FlightRecorder>,
        requires_remount: bool,
    ) -> Self {
        Self {
            view,
            snapshot_id,
            recorder,
            requires_remount,
        }
    }
    fn event(&mut self, id: u64, kind: crate::flight::EventKind, record_block: u64) {
        if let Some(recorder) = &mut self.recorder {
            recorder.object_event(
                self.view.generation,
                kind,
                crate::flight::ObjectContext {
                    object_id: id,
                    record_block,
                    view_id: self.snapshot_id,
                },
                self.requires_remount,
            );
        }
    }
}

pub(crate) fn object<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    observation: &mut Observation<'_>,
    id: u64,
) -> Result<Option<ObjectRecord>, CoreError> {
    if id == OBJECT_ORPHAN_DIRECTORY {
        return Err(CoreError::NotFound);
    }
    let view = observation.view;
    observation.event(id, crate::flight::EventKind::ObjectLookup, 0);
    let geo = ident.geometry();
    let Some(lba) = object_map::lookup_lba(dev, &geo, view.object_map_root, view.generation, id)?
    else {
        observation.event(id, crate::flight::EventKind::ObjectMissing, 0);
        return Ok(None);
    };
    observation.event(id, crate::flight::EventKind::ObjectMapped, lba);
    namespace_range(&geo, lba, 1)?;
    let mut buf = vec![0; geo.block_size];
    dev.read_block(lba, &mut buf)?;
    let (record, generation) = ObjectRecord::decode_metadata_with_generation(&buf)?;
    if generation == 0 || generation > view.generation || record.object_id != id {
        return Err(CoreError::Corrupt(
            "snapshot object identity or generation mismatch".into(),
        ));
    }
    if record.flags & OBJECT_FLAG_DATA_IN_PLACE != 0
        && ident.features.compat & COMPAT_DATA_POLICY == 0
    {
        return Err(CoreError::Corrupt(
            "snapshot object has unnegotiated data policy".into(),
        ));
    }
    match record.object_type {
        ObjectType::File if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 => {
            namespace_range(&geo, record.data_root, 1)?;
            extent_map::validate_root(dev, &geo, record.data_root, id, view.generation)?;
        }
        ObjectType::File if record.data_blocks != 0 => {
            namespace_range(&geo, record.data_root, record.data_blocks)?
        }
        ObjectType::Directory => {
            namespace_range(&geo, record.data_root, 1)?;
            directory::validate_root(dev, &geo, record.data_root, id, view.generation)?;
        }
        _ => (),
    }
    Ok(Some(record))
}

pub(crate) fn read_link<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    observation: &mut Observation<'_>,
    id: u64,
    output: &mut [u8],
) -> Result<usize, CoreError> {
    let view = observation.view;
    let record = object(dev, ident, observation, id)?.ok_or(CoreError::NotFound)?;
    if record.object_type != ObjectType::Symlink {
        return Err(CoreError::InvalidMetadata("object is not a symlink"));
    }
    let geo = ident.geometry();
    let lba = object_map::lookup_lba(dev, &geo, view.object_map_root, view.generation, id)?
        .ok_or(CoreError::NotFound)?;
    namespace_range(&geo, lba, 1)?;
    let mut block = vec![0; geo.block_size];
    dev.read_block(lba, &mut block)?;
    let (link, generation) = afsplus_format::object::SymlinkRecord::decode(&block)?;
    if link.record != record || generation == 0 || generation > view.generation {
        return Err(CoreError::Corrupt(
            "captured symlink identity mismatch".into(),
        ));
    }
    let required = link.target.len();
    if output.len() >= required {
        output[..required].copy_from_slice(link.target.as_bytes());
    }
    Ok(required)
}

pub(crate) fn read_at<D: BlockDevice>(
    dev: &mut D,
    ident: &Identification,
    observation: &mut Observation<'_>,
    id: u64,
    offset: u64,
    destination: &mut [u8],
) -> Result<usize, CoreError> {
    let view = observation.view;
    let record = object(dev, ident, observation, id)?.ok_or(CoreError::NotFound)?;
    if record.object_type != ObjectType::File {
        return Err(CoreError::IsDirectory);
    }
    if offset >= record.size_bytes || destination.is_empty() {
        return Ok(0);
    }
    let count = (record.size_bytes - offset).min(destination.len() as u64);
    let end = offset + count;
    let geo = ident.geometry();
    let block_size = geo.block_size as u64;
    let mut block = vec![0; geo.block_size];
    for logical in offset / block_size..end.div_ceil(block_size) {
        block.fill(0);
        let physical = if record.flags & OBJECT_FLAG_EXTENT_TREE != 0 {
            extent_map::lookup_extent(dev, &geo, record.data_root, id, view.generation, logical)?
                .filter(|extent| extent.flags & extent_map::EXTENT_UNWRITTEN == 0)
                .map(|extent| extent.physical_start + logical - extent.logical_start)
        } else if logical < record.data_blocks {
            Some(record.data_root + logical)
        } else {
            None
        };
        if let Some(lba) = physical {
            namespace_range(&geo, lba, 1)?;
            dev.read_block(lba, &mut block)?;
        }
        let start_byte = logical * block_size;
        let low = offset.max(start_byte);
        let high = end.min(start_byte.saturating_add(block_size));
        destination[(low - offset) as usize..(high - offset) as usize]
            .copy_from_slice(&block[(low - start_byte) as usize..(high - start_byte) as usize]);
    }
    Ok(count as usize)
}
