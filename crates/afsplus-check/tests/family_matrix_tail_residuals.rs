//! Residual tail families of the tiny-cache matrix: the mid-run reclaim cursor
//! transition, snapshot-registry reload read failures, and the ambiguous
//! commit tail of the namespace, symlink, deep-tree, orphan-cleanup and
//! reload fixtures the uncertain-tail row names. See tiny_cache_matrix.md.

mod common;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::{CoreError, Volume};
use afsplus_format::reclaim::{ReclaimCaps, ReclaimRoot};
use common::family_matrix::{self as matrix, ts, Family, Format, Variant, BS};

// ---------------------------------------------------------------------------
// Mid-run reclaim cursor transition
// ---------------------------------------------------------------------------

fn accounting<D: BlockDevice>(volume: &Volume<D>) -> (u64, u64) {
    (volume.free_blocks(), volume.reclaim_pending_blocks())
}

/// Consumed blocks inside the head entry of the selected checkpoint's queue.
fn head_block_offset<D: BlockDevice>(volume: &mut Volume<D>) -> u32 {
    let lba = volume.checkpoint().reclaim_root_block;
    let size = volume.device_mut().block_size();
    let mut block = vec![0u8; size];
    volume.device_mut().read_block(lba, &mut block).unwrap();
    ReclaimRoot::decode(&block).unwrap().0.head_block_offset
}

fn tiny_format() -> Format {
    Format {
        reclaim_caps: ReclaimCaps {
            inline_entries: 4,
            segment_refs: 3,
            table_refs: 8,
        },
        ..Format::new(256, 256)
    }
}

const ANCHOR: &[u8] = b"anchor bytes the cursor transition must keep";
const FILLER: &[u8] = b"filler bytes behind the three-block run";

/// Free and pending blocks of the fixture and of the published state.
const CURSOR_ACCOUNTING: [(u64, u64); 2] = [(208, 18), (209, 20)];
/// Consumed blocks inside the head entry, fixture and published state.
const CURSOR_HEAD_BLOCK_OFFSET: [u32; 2] = [0, 2];

struct CursorState {
    anchor: u64,
    snapshot: u64,
}

struct MidRunCursor;

impl Family for MidRunCursor {
    type State = CursorState;

    fn name(&self) -> &'static str {
        "mid-run reclaim cursor"
    }

    fn format(&self, _variant: Variant) -> Format {
        tiny_format()
    }

    /// The snapshot precedes every block the transition reclaims. A
    /// three-block run and one filler commit behind it put the recorded
    /// step's batch of three blocks inside a run.
    fn setup(&self, volume: &mut Volume<MemoryBackend>, _variant: Variant) -> CursorState {
        volume.set_reclaim_batch_blocks(1);
        let anchor = volume.create_file_in_root("anchor", ANCHOR, ts(1)).unwrap();
        let snapshot = volume.snapshot_create(ts(2)).unwrap();
        volume
            .create_file_in_root("big", &[0xb7u8; 3 * BS], ts(3))
            .unwrap();
        volume.delete_file_in_root("big", ts(4)).unwrap();
        volume.create_file_in_root("filler", FILLER, ts(5)).unwrap();
        CursorState { anchor, snapshot }
    }

    fn snapshot(&self, state: &CursorState) -> Option<u64> {
        Some(state.snapshot)
    }

    fn captured(&self, state: &CursorState) -> Vec<(u64, Vec<u8>)> {
        vec![(state.anchor, ANCHOR.to_vec())]
    }

    fn apply<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &CursorState,
    ) -> Result<(), CoreError> {
        volume.set_reclaim_batch_blocks(3);
        volume.reclaim_step(ts(6)).map(|_| ())
    }

    fn verify<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        state: &CursorState,
        _variant: Variant,
        delta: u64,
        context: &str,
    ) {
        let mut listed: Vec<String> = volume
            .list_root()
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            vec!["anchor".to_string(), "filler".to_string()],
            "{context}: root entries"
        );
        let mut read = vec![0xa5; ANCHOR.len() + 1];
        assert_eq!(
            volume.read_file_at(state.anchor, 0, &mut read).unwrap(),
            ANCHOR.len(),
            "{context}: anchor length"
        );
        assert_eq!(&read[..ANCHOR.len()], ANCHOR, "{context}: anchor bytes");
        assert_eq!(read[ANCHOR.len()], 0xa5, "{context}: anchor EOF");
        assert_eq!(
            accounting(volume),
            CURSOR_ACCOUNTING[delta as usize],
            "{context}: free/pending"
        );
        assert_eq!(
            head_block_offset(volume),
            CURSOR_HEAD_BLOCK_OFFSET[delta as usize],
            "{context}: consumed blocks inside the head entry"
        );
    }

    /// The recorded step leaves the persistent cursor inside a run.
    fn after_success<D: BlockDevice>(
        &self,
        volume: &mut Volume<D>,
        _state: &CursorState,
        _variant: Variant,
    ) {
        assert!(
            head_block_offset(volume) > 0,
            "the recorded step must stop inside a run"
        );
    }
}

crate::profile_tests!(mid_run_cursor, |pages| matrix::plain(
    &MidRunCursor,
    pages,
    12
));
crate::profile_tests!(mid_run_cursor_retained, |pages| matrix::retained(
    &MidRunCursor,
    pages,
    12
));
