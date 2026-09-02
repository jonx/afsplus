//! Formatter for the smallest mountable image.
//!
//! Writes the initial metadata (root record, empty root directory, object
//! map, empty reclaim-queue root), region descriptors and bitmap pages into
//! slot 0 at generation 1,
//! the identification block, then checkpoint generation 1 into slot A. Slot B is
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
use afsplus_format::geometry::Geometry;
use afsplus_format::ident::{
    FeatureFlags, Identification, NameKeyAlgorithm, INCOMPAT_INTENT_LOG,
    INCOMPAT_INTENT_LOG_DATA_UPDATES, RO_COMPAT_SHARED_EXTENTS, UNICODE_VERSION_16_0_0,
};
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::reclaim::{ReclaimCaps, ReclaimRoot};
use afsplus_format::region::{BitmapBinding, RegionDescriptor};
use afsplus_format::{
    Timespec, DEFAULT_BLOCK_SHIFT, DEFAULT_BLOCK_SIZE, OBJECT_FIRST_DYNAMIC, OBJECT_ROOT,
};

use crate::allocation_root;
use crate::directory;
use crate::intent_log;
use crate::layout;
use crate::object_map;
use crate::CoreError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NamePolicy {
    #[default]
    Sensitive,
    Insensitive,
}

pub struct MkfsParams {
    pub uuid: [u8; 16],
    pub label: String,
    /// Allocation region size in blocks (power of two).
    pub region_size: u32,
    /// Reclaim-queue root-area capacities (ADR-036). Tests shrink these to
    /// force sealing and consumption with tiny transactions.
    pub reclaim_caps: ReclaimCaps,
    /// Intent-log slots (ADR-037); 0 disables the log area.
    pub log_slots: u16,
    /// Enables shared data extents (ADR-061, `RO_COMPAT`). Identification is
    /// immutable, so the choice is made here, per compatibility profile:
    /// `workstation` and `full` enable it, the classic and reader profiles do
    /// not. A volume without it rejects clone operations.
    pub shared_extents: bool,
    /// Volume-default directory lookup policy (ADR-008).
    pub name_policy: NamePolicy,
    pub timestamp: Timespec,
}

pub fn mkfs<D: BlockDevice>(dev: &mut D, params: &MkfsParams) -> Result<(), CoreError> {
    if dev.block_size() != DEFAULT_BLOCK_SIZE {
        return Err(CoreError::UnsupportedGeometry(
            "prototype supports only 4 KiB blocks",
        ));
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
    let metadata_start = geo.region0_reserved_blocks();
    let root_record_lba = metadata_start;
    let root_dir_lba = metadata_start + 1;
    let omap_lba = metadata_start + 2;
    let reclaim_root_lba = metadata_start + 3;

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
    let root_dir = directory::empty_leaf(OBJECT_ROOT);
    let omap = object_map::initial_leaf(OBJECT_ROOT, root_record_lba)?;

    dev.write_block(
        root_record_lba,
        &root_record.encode(block_size, generation)?,
    )?;
    dev.write_block(root_dir_lba, &root_dir.encode(block_size, generation)?)?;
    dev.write_block(omap_lba, &omap.encode(block_size, generation)?)?;
    let reclaim_root = ReclaimRoot::empty(params.reclaim_caps);
    dev.write_block(
        reclaim_root_lba,
        &reclaim_root
            .encode(block_size, generation)
            .map_err(CoreError::Format)?,
    )?;

    let provisional_records: Vec<_> = (0..geo.region_count())
        .map(|_| RegionRecord {
            descriptor_slot: 0,
            free_blocks: 0,
            descriptor_generation: generation,
        })
        .collect();
    let provisional_allocation_root = allocation_root::bulk_build(&geo, &provisional_records)?;
    let allocation_pool: std::collections::BTreeSet<_> = provisional_allocation_root
        .pool_lbas
        .iter()
        .copied()
        .collect();
    let log_area = intent_log::log_slot_lbas(&geo, params.log_slots)?;
    let mut initially_allocated = allocation_pool.clone();
    initially_allocated.extend(log_area.iter().copied());
    initially_allocated.insert(root_record_lba);
    initially_allocated.insert(root_dir_lba);
    initially_allocated.insert(omap_lba);
    initially_allocated.insert(reclaim_root_lba);

    // Descriptor slot 0 binds bitmap-page slot 0. Reserved blocks and the
    // initial metadata are allocated; everything else is free.
    let mut regions = Vec::with_capacity(geo.region_count() as usize);
    for r in 0..geo.region_count() {
        let base = geo.region_base(r);
        let mut bindings = Vec::with_capacity(geo.bitmap_page_count(r) as usize);
        for page_index in 0..geo.bitmap_page_count(r) {
            let first_block = page_index * afsplus_format::bitmap::BITMAP_PAGE_BLOCKS;
            let mut page = BitmapPage::all_free(
                r,
                page_index,
                first_block,
                geo.bitmap_page_valid_blocks(r, page_index),
            );
            let page_end = first_block + page.valid_blocks;
            let reserved = geo.region_reserved_blocks(r) as u32
                + if r == 0 {
                    afsplus_format::geometry::BOOTSTRAP_BLOCKS as u32
                } else {
                    0
                };
            for region_index in first_block..page_end.min(reserved) {
                page.set_allocated(region_index - first_block, true);
            }
            let page_lba_start = base + first_block as u64;
            let page_lba_end = base + page_end as u64;
            for lba in initially_allocated.range(page_lba_start..page_lba_end) {
                page.set_allocated((*lba - page_lba_start) as u32, true);
            }
            dev.write_block(
                geo.bitmap_slot_lba(r, page_index, 0),
                &page.encode(block_size, generation)?,
            )?;
            bindings.push(BitmapBinding {
                slot: 0,
                free_blocks: page.free_blocks(),
                generation,
            });
        }
        let free_blocks = bindings.iter().map(|binding| binding.free_blocks).sum();
        let descriptor = RegionDescriptor {
            region: r,
            valid_blocks: geo.region_valid_blocks(r),
            free_blocks,
            pages: bindings,
        };
        dev.write_block(
            geo.descriptor_slot_lba(r, 0),
            &descriptor.encode(block_size, generation)?,
        )?;
        regions.push(RegionRecord {
            descriptor_slot: 0,
            free_blocks,
            descriptor_generation: generation,
        });
    }

    let allocation_root = allocation_root::bulk_build(&geo, &regions)?;
    if allocation_root.pool_lbas != provisional_allocation_root.pool_lbas {
        return Err(CoreError::Corrupt(
            "allocation-root pool changed with record values".into(),
        ));
    }
    for (lba, node) in &allocation_root.nodes {
        dev.write_block(*lba, &node.encode(block_size, generation)?)?;
    }

    let ident = Identification {
        uuid: params.uuid,
        block_shift: DEFAULT_BLOCK_SHIFT,
        checksum_algorithm: CHECKSUM_CRC32C,
        region_size: params.region_size,
        log_slots: params.log_slots,
        features: FeatureFlags {
            incompat: if params.log_slots > 0 {
                INCOMPAT_INTENT_LOG | INCOMPAT_INTENT_LOG_DATA_UPDATES
            } else {
                0
            },
            ro_compat: if params.shared_extents {
                RO_COMPAT_SHARED_EXTENTS
            } else {
                0
            },
            ..FeatureFlags::default()
        },
        name_key_algorithm: match params.name_policy {
            NamePolicy::Sensitive => NameKeyAlgorithm::UnicodeNfc,
            NamePolicy::Insensitive => NameKeyAlgorithm::UnicodeNfcCasefold,
        },
        unicode_version: UNICODE_VERSION_16_0_0,
        total_blocks: geo.total_blocks,
        checkpoint_slots: [layout::CKPT_SLOT_A, layout::CKPT_SLOT_B],
        metadata_start,
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
        allocation_root_block: allocation_root.root_lba,
        reclaim_root_block: reclaim_root_lba,
        next_object_id: OBJECT_FIRST_DYNAMIC,
        committed_tx_id: generation,
        free_blocks_total: regions.iter().map(|record| record.free_blocks as u64).sum(),
        flags: 0,
        shared_extent_root_block: 0,
    };
    dev.write_block(layout::CKPT_SLOT_A, &checkpoint.encode(block_size)?)?;
    dev.flush()?;

    Ok(())
}
