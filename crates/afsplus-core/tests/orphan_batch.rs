//! What one batched orphan cleanup does and does not do (performance
//! programme, lot D): it stops at its extent budget, and it passes over an
//! orphan the caller still holds open instead of stopping there. The cut
//! campaign for the same batch is `afsplus-check`'s `orphan_batch_cuts`.

use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy, Volume};
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

#[test]
fn a_batch_stops_at_its_extent_budget_and_leaves_the_rest_pending() {
    const REGIONS: u64 = 160;
    const REGION_BLOCKS: u32 = 16;
    let mut volume: Volume<MemoryBackend> =
        mount(formatted(REGIONS * u64::from(REGION_BLOCKS), REGION_BLOCKS)).unwrap();
    volume.set_reclaim_batch_blocks(1);
    // Hold the namespace down so the fragmented file cannot take one run.
    for index in 0..120 {
        volume
            .create_file_in_root(&format!("n{index:03}"), b"", ts(1))
            .unwrap();
    }
    let fragmented = volume
        .create_file_in_root("fragmented", b"", ts(1))
        .unwrap();
    let available = volume.available_blocks();
    volume
        .preallocate_file(fragmented, 0, (available - 16) * BS as u64, ts(2))
        .unwrap();
    volume
        .orphan_file(OBJECT_ROOT, "fragmented", ts(3))
        .unwrap();
    volume.set_orphan_cleanup_extent_budget(2);

    let before = volume.generation();
    let progress = volume.cleanup_orphans(4, &mut |_| false, ts(4)).unwrap();
    assert_eq!(volume.generation(), before + 1, "one transaction");
    assert_eq!(progress.objects_removed, 0);
    assert_eq!(progress.objects_advanced, 1);
    assert_eq!(
        progress.extents_removed, 8,
        "four slots of the budget of two, and not one extent more"
    );
    assert!(progress.still_pending);
    assert_eq!(volume.orphan_count().unwrap(), 1);
    assert_eq!(volume.first_orphan().unwrap(), Some(fragmented));

    let mut rounds = 0;
    while volume.orphan_count().unwrap() != 0 {
        volume.cleanup_orphans(4, &mut |_| false, ts(5)).unwrap();
        rounds += 1;
        assert!(rounds < 256, "batched cleanup did not converge");
    }
    assert!(rounds > 1, "one fragmented file was not bounded at all");
    assert_eq!(volume.first_orphan().unwrap(), None);
}

#[test]
fn a_batch_passes_over_what_the_caller_holds_open() {
    let mut volume = mount(formatted(2048, 2048)).unwrap();
    let mut objects = Vec::new();
    for index in 0..4 {
        let name = format!("victim-{index}");
        let object = volume.create_file_in_root(&name, PAYLOAD, ts(1)).unwrap();
        volume.orphan_file(OBJECT_ROOT, &name, ts(2)).unwrap();
        objects.push(object);
    }
    // The first orphan of the directory is the one held open: it used to stop
    // the cleanup of everything behind it.
    let held = objects[0];
    let before = volume.generation();
    let progress = volume
        .cleanup_orphans(4, &mut |object| object == held, ts(3))
        .unwrap();
    assert_eq!(volume.generation(), before + 1, "one transaction");
    assert_eq!(progress.objects_removed, 3);
    assert!(progress.still_pending);
    assert_eq!(volume.orphan_count().unwrap(), 1);
    assert_eq!(volume.first_orphan().unwrap(), Some(held));
    assert_eq!(volume.read_file(held).unwrap(), PAYLOAD);

    // Released, it is cleaned like any other.
    let progress = volume.cleanup_orphans(4, &mut |_| false, ts(4)).unwrap();
    assert_eq!(progress.objects_removed, 1);
    assert!(!progress.still_pending);
    assert_eq!(volume.orphan_count().unwrap(), 0);
}
