//! Formatter for the smallest mountable image.
//!
//! mkfs itself is not crash-atomic: until the final flush completes there is
//! simply no valid AFS+ volume on the device, which is the documented and
//! acceptable outcome for an interrupted format.

use afsplus_block::BlockDevice;
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::crc32c::CHECKSUM_CRC32C;
use afsplus_format::dir::DirBlock;
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::omap::ObjectMap;
use afsplus_format::{Timespec, DEFAULT_BLOCK_SHIFT, DEFAULT_BLOCK_SIZE, OBJECT_FIRST_DYNAMIC, OBJECT_ROOT};

use crate::layout;
use crate::CoreError;

pub struct MkfsParams {
    pub uuid: [u8; 16],
    pub label: String,
    pub timestamp: Timespec,
}

/// Formats `dev` with an empty root directory and checkpoint generation 1 in
/// slot A. Slot B is explicitly zeroed so a reused device cannot present a
/// stale-but-valid second checkpoint.
pub fn mkfs<D: BlockDevice>(dev: &mut D, params: &MkfsParams) -> Result<(), CoreError> {
    if dev.block_size() != DEFAULT_BLOCK_SIZE {
        return Err(CoreError::UnsupportedGeometry("prototype supports only 4 KiB blocks"));
    }
    let total_blocks = dev.total_blocks();
    if total_blocks < layout::MIN_TOTAL_BLOCKS {
        return Err(CoreError::UnsupportedGeometry("volume too small"));
    }
    let block_size = dev.block_size();
    let generation = 1u64;

    // Initial COW metadata: root object record, empty root directory block,
    // object map. Placed at the start of the metadata area.
    let root_record_lba = layout::METADATA_START;
    let root_dir_lba = layout::METADATA_START + 1;
    let omap_lba = layout::METADATA_START + 2;
    let next_free = layout::METADATA_START + 3;

    let root_record = ObjectRecord {
        object_id: OBJECT_ROOT,
        object_type: ObjectType::Directory,
        flags: 0,
        link_count: 1,
        size_bytes: 0,
        allocated_bytes: block_size as u64,
        created: params.timestamp,
        modified: params.timestamp,
        changed: params.timestamp,
        protection: 0,
        content_generation: generation,
        data_root: root_dir_lba,
    };

    let root_dir = DirBlock::new(OBJECT_ROOT);

    let mut omap = ObjectMap::default();
    omap.upsert(OBJECT_ROOT, root_record_lba)?;

    dev.write_block(root_record_lba, &root_record.encode(block_size, generation)?)?;
    dev.write_block(root_dir_lba, &root_dir.encode(block_size, generation)?)?;
    dev.write_block(omap_lba, &omap.encode(block_size, generation)?)?;

    let ident = Identification {
        uuid: params.uuid,
        block_shift: DEFAULT_BLOCK_SHIFT,
        checksum_algorithm: CHECKSUM_CRC32C,
        total_blocks,
        checkpoint_slots: [layout::CKPT_SLOT_A, layout::CKPT_SLOT_B],
        metadata_start: layout::METADATA_START,
        label: params.label.clone(),
    };
    dev.write_block(layout::IDENT_LBA, &ident.encode(block_size)?)?;

    // Neutralize any stale content in slot B before the volume can become
    // valid: the identification block is written only after this point in the
    // same pre-checkpoint barrier group... but ordering within a barrier group
    // is not guaranteed, so rely on UUID binding instead: a leftover
    // checkpoint from a previous filesystem cannot match the fresh UUID.
    dev.write_block(layout::CKPT_SLOT_B, &vec![0u8; block_size])?;

    // Barrier: all referenced metadata (and identification) durable before
    // the first checkpoint can exist.
    dev.flush()?;

    let checkpoint = Checkpoint {
        uuid: params.uuid,
        generation,
        root_object_id: OBJECT_ROOT,
        object_map_block: omap_lba,
        next_free_block: next_free,
        next_object_id: OBJECT_FIRST_DYNAMIC,
        committed_tx_id: generation,
        flags: 0,
    };
    dev.write_block(layout::CKPT_SLOT_A, &checkpoint.encode(block_size)?)?;
    dev.flush()?;

    Ok(())
}
