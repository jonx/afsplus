//! One transaction cleans many orphans (performance programme, lot D).
//!
//! What a batch must not do is leave half of itself on the disk. The cut
//! model here is the in-order one: every prefix of the write log the batch
//! issued, which is what a power cut leaves when nothing is reordered, plus,
//! wherever the unflushed tail is small enough to enumerate, every subset of
//! that tail and the representative tears of the shared model. Each state
//! must be a clean image that mounts at the checkpoint before the batch or
//! the one after it, with every orphan either untouched or gone.

use afsplus_block::{
    for_each_crash_state_with_budget, MemoryBackend, RecordedOp, RecordingBackend,
};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;
const PAYLOAD: &[u8] = b"an orphan the batch takes whole or not at all";

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(blocks: u64, region: u32) -> MemoryBackend {
    let mut device = MemoryBackend::new(BS, blocks);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x7D; 16],
            label: "OrphanBatch".into(),
            region_size: region,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    device
}

fn assert_clean(context: &str, image: &mut MemoryBackend) {
    let report = check_device(image);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
    // A stopped intent-log tail is an admissible crash artifact; anything
    // else the checker warns about is not.
    assert!(
        report
            .warnings
            .iter()
            .all(|warning| warning.starts_with("intent log tail:")),
        "{context}: {:?}",
        report.warnings
    );
}

/// The image a power cut leaves when the writes before `cut` all landed in
/// the order they were issued.
fn in_order_cut(base: &MemoryBackend, log: &[RecordedOp], cut: usize) -> MemoryBackend {
    let mut image = base.clone();
    for op in &log[..cut] {
        if let RecordedOp::Write { lba, data } = op {
            image.apply_raw(*lba, data);
        }
    }
    image
}

/// Writes since the last completed flush at `cut`: what the model may still
/// enumerate exhaustively.
fn unflushed_tail(log: &[RecordedOp], cut: usize) -> usize {
    let prefix = &log[..cut];
    let last_flush = prefix
        .iter()
        .rposition(|op| matches!(op, RecordedOp::Flush))
        .map(|index| index + 1)
        .unwrap_or(0);
    prefix[last_flush..]
        .iter()
        .filter(|op| matches!(op, RecordedOp::Write { .. }))
        .count()
}

#[test]
fn every_cut_of_a_batch_leaves_each_orphan_whole_or_gone() {
    const ORPHANS: usize = 6;
    let base = formatted(2048, 2048);
    let mut volume = mount(base.clone()).unwrap();
    let mut objects = Vec::new();
    for index in 0..ORPHANS {
        let name = format!("victim-{index}");
        let object = volume.create_file_in_root(&name, PAYLOAD, ts(1)).unwrap();
        volume.orphan_file(OBJECT_ROOT, &name, ts(2)).unwrap();
        objects.push(object);
    }
    assert_eq!(volume.orphan_count().unwrap(), ORPHANS as u64);
    let before = volume.generation();
    let orphaned = volume.into_device();

    let mut recording = mount(RecordingBackend::new(orphaned.clone())).unwrap();
    let progress = recording
        .cleanup_orphans(ORPHANS, &mut |_| false, ts(3))
        .unwrap();
    assert_eq!(progress.objects_removed, ORPHANS);
    assert!(!progress.still_pending);
    assert_eq!(
        recording.generation(),
        before + 1,
        "the batch is one transaction"
    );
    let log = recording.into_device().into_parts().1;

    let mut outcomes = [0u64; 2];
    let mut states = 0u64;
    for cut in 0..=log.len() {
        let mut images = vec![in_order_cut(&orphaned, &log, cut)];
        if unflushed_tail(&log, cut) <= 8 {
            images.clear();
            for_each_crash_state_with_budget(&orphaned, &log, cut, 8, |state| {
                images.push(state.image)
            });
        }
        for (index, mut image) in images.into_iter().enumerate() {
            let context = format!("cut {cut} state {index}");
            states += 1;
            assert_clean(&context, &mut image);
            let mut recovered = mount(image).unwrap();
            let delta = recovered.generation().checked_sub(before).unwrap();
            assert!(delta <= 1, "{context}: generation delta {delta}");
            outcomes[delta as usize] += 1;
            let pending = recovered.orphan_count().unwrap();
            assert_eq!(
                pending,
                if delta == 0 { ORPHANS as u64 } else { 0 },
                "{context}: the batch is all of its orphans or none of them"
            );
            for object in &objects {
                if delta == 0 {
                    assert!(recovered.orphan_object(*object).unwrap(), "{context}");
                    assert_eq!(recovered.read_file(*object).unwrap(), PAYLOAD, "{context}");
                } else {
                    assert!(
                        recovered.visible_metadata(*object).unwrap().is_none(),
                        "{context}: object {object} is still there"
                    );
                }
            }
            // Whatever the cut left, finishing is one more batch.
            let again = recovered
                .cleanup_orphans(ORPHANS, &mut |_| false, ts(4))
                .unwrap();
            assert_eq!(again.objects_removed, pending as usize, "{context}");
            assert_eq!(recovered.orphan_count().unwrap(), 0, "{context}");
            assert_clean(&context, recovered.device_mut());
        }
    }
    assert!(
        outcomes.iter().all(|count| *count > 0),
        "both outcomes must occur: {outcomes:?} over {states} states"
    );
    eprintln!("orphan batch cuts: {states} states, before/after {outcomes:?}");
}
