//! Volume-wide shared-extent reference tree (ADR-061).
//!
//! One record per shared physical run, keyed by the run's first physical
//! block. A record exists only while its run has two or more references:
//! the extent flag `EXTENT_SHARED` is a conservative marker obliging an
//! overlap lookup, and this tree is the authority. Records are canonical —
//! ordered, non-overlapping, maximal (adjacent contiguous records with equal
//! counts must have been merged) — so a given sharing state has exactly one
//! representation and the checker compares without guessing.
//!
//! Loading and overlap resolution are deliberately autonomous: the checker
//! validates sharing through these functions without depending on the write
//! path's own readers.

use afsplus_block::BlockDevice;
use afsplus_format::geometry::Geometry;
use afsplus_format::le;
use afsplus_format::tree::{key_u64, TreeItem, TreeKind, TreeNode};

use crate::tree::{visit_tree_nodes, TreeSpec, TreeSummary};
use crate::CoreError;

pub const VALUE_SIZE: usize = 16;

/// A record never stores fewer references than this: a run with a single
/// reference is an ordinary private run and has no record (ADR-061).
pub const MIN_REFERENCE_COUNT: u32 = 2;

/// One shared physical run and its authoritative reference count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedRun {
    pub physical_start: u64,
    pub block_count: u64,
    pub reference_count: u32,
    /// Reserved; must be zero.
    pub flags: u32,
}

impl SharedRun {
    pub fn physical_end(self) -> Result<u64, CoreError> {
        self.physical_start
            .checked_add(self.block_count)
            .ok_or_else(|| CoreError::Corrupt("shared run end overflows".into()))
    }
}

/// One segment of an extent partitioned against the reference tree.
/// `reference_count` is `None` for a private gap (no overlapping record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRun {
    pub physical_start: u64,
    pub block_count: u64,
    pub reference_count: Option<u32>,
}

pub struct LoadedSharedExtents {
    pub records: Vec<SharedRun>,
    pub tree_blocks: Vec<u64>,
    pub summary: TreeSummary,
}

pub fn spec(max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::SharedExtents,
        owner: 0,
        max_generation,
    }
}

pub fn empty_leaf() -> TreeNode {
    TreeNode::leaf(TreeKind::SharedExtents, 0)
}

pub fn encode_run(run: SharedRun) -> Result<([u8; 8], [u8; VALUE_SIZE]), CoreError> {
    validate_run_shape(run)?;
    let mut value = [0u8; VALUE_SIZE];
    le::put_u64(&mut value[0..8], run.block_count);
    le::put_u32(&mut value[8..12], run.reference_count);
    le::put_u32(&mut value[12..16], run.flags);
    Ok((key_u64(run.physical_start), value))
}

pub fn item(run: SharedRun) -> Result<TreeItem, CoreError> {
    let (key, value) = encode_run(run)?;
    Ok(TreeItem {
        key: key.to_vec(),
        value: value.to_vec(),
    })
}

pub fn decode_run(key: &[u8], value: &[u8], geo: &Geometry) -> Result<SharedRun, CoreError> {
    let key: [u8; 8] = key
        .try_into()
        .map_err(|_| CoreError::Corrupt("shared-run key is not eight bytes".into()))?;
    if value.len() != VALUE_SIZE {
        return Err(CoreError::Corrupt(
            "shared-run value is not sixteen bytes".into(),
        ));
    }
    let run = SharedRun {
        physical_start: u64::from_be_bytes(key),
        block_count: le::get_u64(&value[0..8]),
        reference_count: le::get_u32(&value[8..12]),
        flags: le::get_u32(&value[12..16]),
    };
    validate_run_shape(run)?;
    validate_physical_range(run, geo)?;
    Ok(run)
}

fn validate_run_shape(run: SharedRun) -> Result<(), CoreError> {
    if run.block_count == 0 {
        return Err(CoreError::Corrupt("zero-length shared run".into()));
    }
    run.physical_end()?;
    if run.reference_count < MIN_REFERENCE_COUNT {
        return Err(CoreError::Corrupt(
            "shared run with fewer than two references".into(),
        ));
    }
    if run.flags != 0 {
        return Err(CoreError::Corrupt(
            "shared-run reserved flags are nonzero".into(),
        ));
    }
    Ok(())
}

/// Shared runs describe the same physical data runs as extents, so the same
/// bounds rule applies: allocatable, and within one allocation region.
fn validate_physical_range(run: SharedRun, geo: &Geometry) -> Result<(), CoreError> {
    let end = run.physical_end()?;
    if end > geo.total_blocks
        || !geo.is_allocatable(run.physical_start)
        || !geo.is_allocatable(end - 1)
        || geo.region_of(run.physical_start) != geo.region_of(end - 1)
    {
        return Err(CoreError::Corrupt(format!(
            "shared run {}..{end} is not allocatable",
            run.physical_start
        )));
    }
    Ok(())
}

/// Rejects any record sequence that is not the canonical form: strictly
/// ordered, non-overlapping, and maximal (an adjacent contiguous pair with
/// equal counts should have been merged into one record).
pub fn validate_canonical(records: &[SharedRun]) -> Result<(), CoreError> {
    for pair in records.windows(2) {
        let end = pair[0].physical_end()?;
        if end > pair[1].physical_start {
            return Err(CoreError::Corrupt("shared runs overlap".into()));
        }
        if end == pair[1].physical_start && pair[0].reference_count == pair[1].reference_count {
            return Err(CoreError::Corrupt(
                "adjacent shared runs with equal counts are not merged".into(),
            ));
        }
    }
    Ok(())
}

/// Exhaustively loads and validates the reference tree. `root_lba` must be
/// nonzero; a volume that has never cloned has no tree at all.
pub fn load_all<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    max_generation: u64,
) -> Result<LoadedSharedExtents, CoreError> {
    let mut records = Vec::new();
    let mut tree_blocks = Vec::new();
    let summary = visit_tree_nodes(dev, geo, root_lba, spec(max_generation), |lba, node| {
        tree_blocks.push(lba);
        if node.is_leaf() {
            for item in &node.items {
                records.push(decode_run(&item.key, &item.value, geo)?);
            }
        }
        Ok(())
    })?;
    if records.len() as u64 != summary.items {
        return Err(CoreError::Corrupt(
            "shared-run leaf count does not match tree summary".into(),
        ));
    }
    validate_canonical(&records)?;
    Ok(LoadedSharedExtents {
        records,
        tree_blocks,
        summary,
    })
}

/// Partitions one physical run against the canonical record sequence.
///
/// This is the mandatory resolution for any operation on an extent flagged
/// `EXTENT_SHARED`: peers split the underlying runs independently, so one
/// flagged extent may cover shared sub-runs and private gaps. An exact
/// lookup on the extent's own start would find the head and miss the tail
/// (ADR-061). Only the returned private gaps are sole-owned.
pub fn resolve_overlaps(
    physical_start: u64,
    block_count: u64,
    records: &[SharedRun],
) -> Result<Vec<SubRun>, CoreError> {
    if block_count == 0 {
        return Err(CoreError::Corrupt("zero-length overlap query".into()));
    }
    let end = physical_start
        .checked_add(block_count)
        .ok_or_else(|| CoreError::Corrupt("overlap query end overflows".into()))?;

    let mut segments = Vec::new();
    let mut cursor = physical_start;
    // First record whose end lies past the query start; canonical ordering
    // makes every earlier record irrelevant.
    let mut index =
        records.partition_point(|run| run.physical_start.saturating_add(run.block_count) <= cursor);
    while cursor < end && index < records.len() {
        let run = records[index];
        if run.physical_start >= end {
            break;
        }
        if run.physical_start > cursor {
            segments.push(SubRun {
                physical_start: cursor,
                block_count: run.physical_start - cursor,
                reference_count: None,
            });
            cursor = run.physical_start;
        }
        let segment_end = run.physical_end()?.min(end);
        segments.push(SubRun {
            physical_start: cursor,
            block_count: segment_end - cursor,
            reference_count: Some(run.reference_count),
        });
        cursor = segment_end;
        index += 1;
    }
    if cursor < end {
        segments.push(SubRun {
            physical_start: cursor,
            block_count: end - cursor,
            reference_count: None,
        });
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::{resolve_overlaps, validate_canonical, SharedRun, SubRun};

    fn run(start: u64, blocks: u64, count: u32) -> SharedRun {
        SharedRun {
            physical_start: start,
            block_count: blocks,
            reference_count: count,
            flags: 0,
        }
    }

    /// The ADR-061 example: A and B share [0,100), B rewrites [40,60).
    /// A's flagged extent partitions into shared head, private gap, shared
    /// tail — an exact lookup on the start would miss the tail.
    #[test]
    fn overlap_partition_finds_the_tail() {
        let records = [run(0, 40, 2), run(60, 40, 2)];
        let segments = resolve_overlaps(0, 100, &records).unwrap();
        assert_eq!(
            segments,
            vec![
                SubRun {
                    physical_start: 0,
                    block_count: 40,
                    reference_count: Some(2),
                },
                SubRun {
                    physical_start: 40,
                    block_count: 20,
                    reference_count: None,
                },
                SubRun {
                    physical_start: 60,
                    block_count: 40,
                    reference_count: Some(2),
                },
            ]
        );
    }

    #[test]
    fn overlap_partition_clips_to_the_query() {
        let records = [run(10, 80, 3)];
        let segments = resolve_overlaps(30, 20, &records).unwrap();
        assert_eq!(
            segments,
            vec![SubRun {
                physical_start: 30,
                block_count: 20,
                reference_count: Some(3),
            }]
        );
        // A query entirely inside a gap is one private segment.
        let segments = resolve_overlaps(100, 10, &records).unwrap();
        assert_eq!(
            segments,
            vec![SubRun {
                physical_start: 100,
                block_count: 10,
                reference_count: None,
            }]
        );
    }

    #[test]
    fn canonical_form_is_enforced() {
        validate_canonical(&[run(0, 10, 2), run(10, 5, 3), run(20, 5, 3)]).unwrap();
        // Overlap.
        assert!(validate_canonical(&[run(0, 10, 2), run(9, 5, 3)]).is_err());
        // Adjacent contiguous equal counts must have merged.
        assert!(validate_canonical(&[run(0, 10, 2), run(10, 5, 2)]).is_err());
    }
}
