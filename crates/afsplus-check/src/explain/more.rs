//! The explain operations beyond block, object and path: one extent, the
//! checkpoint slots, the reclaim queue, the space of one region, and the
//! feature bits. All answer from the walk `Explainer::load` already made.
use afsplus_format::extent::{EXTENT_SHARED, EXTENT_UNWRITTEN};
use afsplus_format::ident::{
    COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG, INCOMPAT_INTENT_LOG_DATA_UPDATES,
    INCOMPAT_PERSISTENT_SNAPSHOTS, INCOMPAT_SECURITY_DESCRIPTORS, RO_COMPAT_ORPHAN_DIRECTORY,
    RO_COMPAT_SHARED_EXTENTS,
};
use afsplus_format::object::ObjectType;

use super::{Allocation, BlockRole, Explainer};

/// What the walk kept of the reclaim queue.
#[derive(Debug, Clone, Default)]
pub(super) struct ReclaimState {
    pub tables: u64,
    pub segments: u64,
    pub inline_entries: u64,
    /// Pending runs in queue order, oldest first: start, blocks, retire
    /// generation. The consumed part of the head run is already removed.
    pub runs: Vec<(u64, u32, u64)>,
}

/// Where one byte offset of a file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtentState {
    /// At or past the end of the file.
    BeyondEnd,
    /// Inside the file, mapped by no extent: reads as zeros.
    Hole,
    Mapped {
        physical_block: u64,
        extent_logical_start: u64,
        extent_physical_start: u64,
        extent_blocks: u64,
        shared: bool,
        /// Allocated and never written: reads as zeros.
        unwritten: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentExplanation {
    pub object_id: u64,
    pub offset: u64,
    pub size_bytes: u64,
    pub logical_block: u64,
    pub offset_in_block: u32,
    pub state: ExtentState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotExplanation {
    pub slot: u8,
    pub block: u64,
    pub selected: bool,
    /// The generation a selectable slot carries, or why it is not selectable.
    pub state: Result<u64, String>,
}

/// The two checkpoint slots and the fields of the selected checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointExplanation {
    pub slots: [SlotExplanation; 2],
    pub generation: u64,
    pub committed_tx_id: u64,
    pub label: String,
    pub root_object_id: u64,
    pub next_object_id: u64,
    pub free_blocks_total: u64,
    pub object_map_block: u64,
    pub allocation_root_block: u64,
    pub reclaim_root_block: u64,
    /// Zero on a volume that has never cloned.
    pub shared_extent_root_block: u64,
    /// Registry root and lifetime-ledger root of a snapshot volume.
    pub snapshot_roots: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimRun {
    pub start: u64,
    pub blocks: u32,
    pub retire_generation: u64,
    /// Position in the queue; zero is the next run to be released.
    pub position: u64,
}

/// The reclaim queue, and the run that holds the block asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimExplanation {
    pub tables: u64,
    pub segments: u64,
    pub inline_entries: u64,
    pub runs: u64,
    pub blocks: u64,
    pub oldest_retire_generation: Option<u64>,
    pub newest_retire_generation: Option<u64>,
    /// The block asked about, when one was.
    pub block: Option<u64>,
    /// The pending run that holds that block; `None` when it is not
    /// quarantined, or when no block was asked about.
    pub run: Option<ReclaimRun>,
}

/// The allocation state of one region, counted from the live bitmap and the
/// roles of the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceExplanation {
    pub region: u32,
    pub first_block: u64,
    pub blocks: u64,
    pub reserved_blocks: u64,
    pub allocated_blocks: u64,
    pub free_blocks: u64,
    /// Allocated blocks waiting in the reclaim queue.
    pub quarantined_blocks: u64,
    /// Allocated blocks with no live role: leaks, or blocks only a retained
    /// snapshot reaches.
    pub unowned_blocks: u64,
    /// First block and length of the longest free run; `None` when full.
    pub largest_free_run: Option<(u64, u64)>,
    /// Descriptor slot the allocation root selects for the region.
    pub live_descriptor_slot: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureClass {
    Compat,
    RoCompat,
    Incompat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureExplanation {
    pub class: FeatureClass,
    pub bit: u8,
    /// Registry identity; `None` for a bit this implementation does not know.
    pub id: Option<&'static str>,
    pub enabled: bool,
}

/// Every feature bit this implementation assigns (`spec/feature-registry.toml`).
pub(super) const KNOWN_FEATURES: [(FeatureClass, u64, &str); 7] = [
    (
        FeatureClass::Compat,
        COMPAT_DATA_POLICY,
        "org.aros.afsplus:data-policy",
    ),
    (
        FeatureClass::RoCompat,
        RO_COMPAT_SHARED_EXTENTS,
        "org.aros.afsplus:shared-extents",
    ),
    (
        FeatureClass::RoCompat,
        RO_COMPAT_ORPHAN_DIRECTORY,
        "org.aros.afsplus:orphan-directory",
    ),
    (
        FeatureClass::Incompat,
        INCOMPAT_INTENT_LOG,
        "org.aros.afsplus:intent-log",
    ),
    (
        FeatureClass::Incompat,
        INCOMPAT_INTENT_LOG_DATA_UPDATES,
        "org.aros.afsplus:intent-log-data-updates",
    ),
    (
        FeatureClass::Incompat,
        INCOMPAT_PERSISTENT_SNAPSHOTS,
        "org.aros.afsplus:persistent-snapshots",
    ),
    (
        FeatureClass::Incompat,
        INCOMPAT_SECURITY_DESCRIPTORS,
        "org.aros.afsplus:security-descriptors",
    ),
];

impl Explainer {
    /// Where byte `offset` of file `object_id` lives.
    pub fn explain_extent(&self, object_id: u64, offset: u64) -> Result<ExtentExplanation, String> {
        let object = self.explain_object(object_id)?;
        if object.object_type != ObjectType::File {
            return Err(format!("object {object_id} is not a file"));
        }
        let block_size = self.ident.geometry().block_size as u64;
        let logical_block = offset / block_size;
        let state = if offset >= object.size_bytes {
            ExtentState::BeyondEnd
        } else {
            self.extents
                .get(&object_id)
                .and_then(|extents| {
                    extents.iter().find(|extent| {
                        (extent.logical_start..extent.logical_start + extent.block_count)
                            .contains(&logical_block)
                    })
                })
                .map_or(ExtentState::Hole, |extent| ExtentState::Mapped {
                    physical_block: extent.physical_start + (logical_block - extent.logical_start),
                    extent_logical_start: extent.logical_start,
                    extent_physical_start: extent.physical_start,
                    extent_blocks: extent.block_count,
                    shared: extent.flags & EXTENT_SHARED != 0,
                    unwritten: extent.flags & EXTENT_UNWRITTEN != 0,
                })
        };
        Ok(ExtentExplanation {
            object_id,
            offset,
            size_bytes: object.size_bytes,
            logical_block,
            offset_in_block: (offset % block_size) as u32,
            state,
        })
    }

    /// The two checkpoint slots and the selected checkpoint.
    pub fn explain_checkpoint(&self) -> CheckpointExplanation {
        let slot = |index: u8| SlotExplanation {
            slot: index,
            block: 1 + u64::from(index),
            selected: index == self.selected_slot,
            state: self.slot_states[usize::from(index)].clone(),
        };
        let checkpoint = &self.checkpoint;
        CheckpointExplanation {
            slots: [slot(0), slot(1)],
            generation: checkpoint.generation,
            committed_tx_id: checkpoint.committed_tx_id,
            label: checkpoint.label.clone(),
            root_object_id: checkpoint.root_object_id,
            next_object_id: checkpoint.next_object_id,
            free_blocks_total: checkpoint.free_blocks_total,
            object_map_block: checkpoint.object_map_block,
            allocation_root_block: checkpoint.allocation_root_block,
            reclaim_root_block: checkpoint.reclaim_root_block,
            shared_extent_root_block: checkpoint.shared_extent_root_block,
            snapshot_roots: checkpoint
                .snapshot_roots
                .map(|roots| (roots.registry, roots.lifetimes)),
        }
    }

    /// The reclaim queue; with a block, the pending run that holds it.
    pub fn explain_reclaim(&self, block: Option<u64>) -> Result<ReclaimExplanation, String> {
        if let Some(lba) = block.filter(|lba| *lba >= self.total_blocks) {
            return Err(format!(
                "block {lba} is outside the volume of {} blocks",
                self.total_blocks
            ));
        }
        let runs = &self.reclaim.runs;
        let run = block.and_then(|lba| {
            runs.iter()
                .enumerate()
                .find(|(_, (start, blocks, _))| {
                    (*start..*start + u64::from(*blocks)).contains(&lba)
                })
                .map(
                    |(position, (start, blocks, retire_generation))| ReclaimRun {
                        start: *start,
                        blocks: *blocks,
                        retire_generation: *retire_generation,
                        position: position as u64,
                    },
                )
        });
        Ok(ReclaimExplanation {
            tables: self.reclaim.tables,
            segments: self.reclaim.segments,
            inline_entries: self.reclaim.inline_entries,
            runs: runs.len() as u64,
            blocks: runs.iter().map(|run| u64::from(run.1)).sum(),
            oldest_retire_generation: runs.iter().map(|run| run.2).min(),
            newest_retire_generation: runs.iter().map(|run| run.2).max(),
            block,
            run,
        })
    }

    /// The allocation state of `region`.
    pub fn explain_space(&self, region: u32) -> Result<SpaceExplanation, String> {
        let geo = self.ident.geometry();
        if region >= geo.region_count() {
            return Err(format!(
                "region {region} is outside the volume of {} regions",
                geo.region_count()
            ));
        }
        let first_block = geo.region_base(region);
        let blocks = u64::from(geo.region_valid_blocks(region));
        let mut space = SpaceExplanation {
            region,
            first_block,
            blocks,
            reserved_blocks: 0,
            allocated_blocks: 0,
            free_blocks: 0,
            quarantined_blocks: 0,
            unowned_blocks: 0,
            largest_free_run: None,
            live_descriptor_slot: None,
        };
        let mut run: Option<(u64, u64)> = None;
        for lba in first_block..first_block + blocks {
            let roles = self.roles.get(&lba).map_or(&[][..], Vec::as_slice);
            let allocation = self.allocation(lba);
            match allocation {
                Allocation::Reserved => space.reserved_blocks += 1,
                Allocation::Free => space.free_blocks += 1,
                Allocation::Allocated => {
                    space.allocated_blocks += 1;
                    if roles.is_empty() {
                        space.unowned_blocks += 1;
                    }
                    if roles
                        .iter()
                        .any(|role| matches!(role, BlockRole::Quarantined { .. }))
                    {
                        space.quarantined_blocks += 1;
                    }
                }
            }
            run = match (allocation, run) {
                (Allocation::Free, Some((start, length))) => Some((start, length + 1)),
                (Allocation::Free, None) => Some((lba, 1)),
                _ => None,
            };
            if let Some(current) = run {
                if space.largest_free_run.is_none_or(|best| current.1 > best.1) {
                    space.largest_free_run = Some(current);
                }
            }
            for role in roles {
                if let BlockRole::RegionDescriptorSlot {
                    region: r,
                    slot,
                    live: true,
                } = role
                {
                    if *r == region {
                        space.live_descriptor_slot = Some(*slot);
                    }
                }
            }
        }
        Ok(space)
    }

    /// Every feature bit the volume sets or this implementation knows, in
    /// class and bit order. A set bit without an identity is unknown here.
    pub fn explain_features(&self) -> Vec<FeatureExplanation> {
        let features = &self.ident.features;
        let mut out = Vec::new();
        for (class, word) in [
            (FeatureClass::Compat, features.compat),
            (FeatureClass::RoCompat, features.ro_compat),
            (FeatureClass::Incompat, features.incompat),
        ] {
            for bit in 0..64u8 {
                let mask = 1u64 << bit;
                let id = KNOWN_FEATURES
                    .iter()
                    .find(|(known, known_mask, _)| *known == class && *known_mask == mask)
                    .map(|(_, _, id)| *id);
                if id.is_some() || word & mask != 0 {
                    out.push(FeatureExplanation {
                        class,
                        bit,
                        id,
                        enabled: word & mask != 0,
                    });
                }
            }
        }
        out
    }

    /// One feature by its registry identity.
    pub fn explain_feature(&self, id: &str) -> Result<FeatureExplanation, String> {
        self.explain_features()
            .into_iter()
            .find(|feature| feature.id == Some(id))
            .ok_or_else(|| format!("feature {id:?} is not known to this implementation"))
    }
}
