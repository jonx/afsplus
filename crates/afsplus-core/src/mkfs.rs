//! Formatter for the smallest mountable image.
//!
//! Writes the initial metadata (root record, empty root directory, object
//! map), one bitmap page per region into slot 0 at generation 1, the
//! identification block, then checkpoint generation 1 into slot A. Slot B is
//! explicitly zeroed so a reused device cannot present a stale-but-valid
//! second checkpoint (the UUID binding already rejects foreign checkpoints;
//! zeroing also clears leftovers from a previous format of the *same* image).
//!
//! mkfs itself is not crash-atomic: until the final flush completes there is
//! simply no valid AFS+ volume on the device, which is the documented and
//! acceptable outcome for an interrupted format.

use afsplus_block::BlockDevice;
use afsplus_format::bitmap::BitmapPage;
use afsplus_format::checkpoint::{Checkpoint, RegionRecord};
use afsplus_format::crc32c::CHECKSUM_CRC32C;
use afsplus_format::dir::DirBlock;
use afsplus_format::geometry::{Geometry, REGION0_RESERVED};
use afsplus_format::ident::Identification;
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::omap::ObjectMap;
use afsplus_format::{Timespec, DEFAULT_BLOCK_SHIFT, DEFAULT_BLOCK_SIZE, OBJECT_FIRST_DYNAMIC, OBJECT_ROOT};

use crate::layout;
use crate::CoreError;

pub struct MkfsParams {
    pub uuid: [u8; 16],
    pub label: String,
    /// Allocation region size in blocks (power of two; one bitmap page per
    /// region in the prototype).
    pub region_size: u32,
    pub timestamp: Timespec,
}

pub fn mkfs<D: BlockDevice>(dev: &mut D, params: &MkfsParams) -> Result<(), CoreError> {
    if dev.block_size() != DEFAULT_BLOCK_SIZE {
        return Err(CoreError::UnsupportedGeometry("prototype supports only 4 KiB blocks"));
    }
    let geo = Geometry {
        block_size: dev.block_size(),
        total_blocks: dev.total_blocks(),
        region_size: params.region_size,
    };
    geo.validate().map_err(CoreError::Format)?;
    if geo.total_blocks < layout::MIN_TOTAL_BLOCKS {
        return Err(CoreError::UnsupportedGeometry("volume too small"));
    }
    let block_size = geo.block_size;
    let generation = 1u64;

    // Initial COW metadata right after region 0's reserved head.
    let root_record_lba = REGION0_RESERVED;
    let root_dir_lba = REGION0_RESERVED + 1;
    let omap_lba = REGION0_RESERVED + 2;

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
        data_blocks: 0,
    };
    let root_dir = DirBlock::new(OBJECT_ROOT);
    let mut omap = ObjectMap::default();
    omap.upsert(OBJECT_ROOT, root_record_lba)?;

    dev.write_block(root_record_lba, &root_record.encode(block_size, generation)?)?;
    dev.write_block(root_dir_lba, &root_dir.encode(block_size, generation)?)?;
    dev.write_block(omap_lba, &omap.encode(block_size, generation)?)?;

    // One bitmap page per region, slot 0: reserved blocks and the initial
    // metadata are allocated, everything else free.
    let mut regions = Vec::with_capacity(geo.region_count() as usize);
    for r in 0..geo.region_count() {
        let mut page = BitmapPage::all_free(r, geo.region_valid_blocks(r));
        let base = geo.region_base(r);
        for index in 0..page.valid_blocks {
            let lba = base + index as u64;
            let initial_metadata = lba == root_record_lba || lba == root_dir_lba || lba == omap_lba;
            if geo.is_reserved(lba) || initial_metadata {
                page.set_allocated(index, true);
            }
        }
        dev.write_block(geo.bitmap_slot_lba(r, 0), &page.encode(block_size, generation)?)?;
        regions.push(RegionRecord {
            slot: 0,
            free_blocks: page.free_blocks(),
            bitmap_generation: generation,
        });
    }

    let ident = Identification {
        uuid: params.uuid,
        block_shift: DEFAULT_BLOCK_SHIFT,
        checksum_algorithm: CHECKSUM_CRC32C,
        region_size: params.region_size,
        total_blocks: geo.total_blocks,
        checkpoint_slots: [layout::CKPT_SLOT_A, layout::CKPT_SLOT_B],
        metadata_start: REGION0_RESERVED,
        label: params.label.clone(),
    };
    dev.write_block(layout::IDENT_LBA, &ident.encode(block_size)?)?;
    dev.write_block(layout::CKPT_SLOT_B, &vec![0u8; block_size])?;

    // Barrier: all referenced state durable before the checkpoint can exist.
    dev.flush()?;

    let checkpoint = Checkpoint {
        uuid: params.uuid,
        generation,
        root_object_id: OBJECT_ROOT,
        object_map_block: omap_lba,
        retired_list_block: 0,
        next_object_id: OBJECT_FIRST_DYNAMIC,
        committed_tx_id: generation,
        flags: 0,
        regions,
    };
    dev.write_block(layout::CKPT_SLOT_A, &checkpoint.encode(block_size)?)?;
    dev.flush()?;

    Ok(())
}
