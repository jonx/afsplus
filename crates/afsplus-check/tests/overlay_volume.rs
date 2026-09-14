//! Independent filesystem branches must retain exact namespace/content states.
use afsplus_block::{BlockDevice, MemoryBackend, OverlayBackend, OverlayLimits};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};
const BS: usize = 4096;

#[test]
fn filesystem_forks_diverge_without_changing_the_base_or_each_other() {
    let (base, id, now) = populated_base();
    let before: Vec<_> = (0..256).map(|lba| base.peek(lba)).collect();
    let pristine = OverlayBackend::new(
        base,
        OverlayLimits {
            branches: 4,
            entries: 1024,
        },
    )
    .unwrap();
    let mut left = mount(pristine.fork().unwrap()).unwrap();
    left.write_file_at(id, 4096, b"left", now).unwrap();
    left.rename(OBJECT_ROOT, "seed", OBJECT_ROOT, "left", now)
        .unwrap();
    left.sync().unwrap();
    let left = left.into_device();
    let mut right = mount(left.fork().unwrap()).unwrap();
    right.truncate_file(id, 3, now).unwrap();
    right
        .rename(OBJECT_ROOT, "left", OBJECT_ROOT, "right", now)
        .unwrap();
    right.sync().unwrap();
    let mut right_device = right.into_device();
    assert!(check_device(&mut right_device).is_clean());
    let mut right = mount(right_device).unwrap();
    assert_eq!(right.lookup_root("right").unwrap(), Some(id));
    assert_eq!(right.lookup_root("left").unwrap(), None);
    assert_eq!(right.read_file(id).unwrap(), b"ori");
    let mut left = mount(left).unwrap();
    assert_eq!(left.lookup_root("left").unwrap(), Some(id));
    assert_eq!(left.lookup_root("right").unwrap(), None);
    let mut expected = vec![0; 4100];
    expected[..8].copy_from_slice(b"original");
    expected[4096..].copy_from_slice(b"left");
    assert_eq!(left.read_file(id).unwrap(), expected);
    let mut left = left.into_device();
    assert!(check_device(&mut left).is_clean());
    drop(left);
    drop(right);
    assert_eq!((pristine.live_entries(), pristine.live_branches()), (0, 1));
    let mut pristine = pristine;
    for lba in 0..256 {
        let mut out = vec![0; BS];
        pristine.read_block(lba, &mut out).unwrap();
        assert_eq!(out, before[lba as usize], "base block {lba}");
    }
    let mut original = mount(pristine).unwrap();
    assert_eq!(original.lookup_root("seed").unwrap(), Some(id));
    assert_eq!(original.read_file(id).unwrap(), b"original");
}

fn populated_base() -> (MemoryBackend, u64, Timespec) {
    let now = Timespec {
        seconds: 42,
        nanoseconds: 0,
    };
    let mut base = MemoryBackend::new(BS, 256);
    mkfs(
        &mut base,
        &MkfsParams {
            uuid: [4; 16],
            label: "Branches".into(),
            region_size: 64,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: now,
        },
    )
    .unwrap();
    let mut volume = mount(base).unwrap();
    let id = volume
        .create_file_in_root("seed", b"original", now)
        .unwrap();
    let base = volume.into_device();
    (base, id, now)
}

#[test]
fn exhausted_overlay_preserves_the_acknowledged_filesystem_state() {
    for entries in [0, 1, 2, 4] {
        let (base, id, now) = populated_base();
        let branch = OverlayBackend::new(
            base,
            OverlayLimits {
                branches: 1,
                entries,
            },
        )
        .unwrap();
        let mut volume = mount(branch).unwrap();
        assert!(volume.write_file_at(id, 0, b"replacement", now).is_err());
        let mut branch = volume.into_device();
        assert!(branch.live_entries() <= entries);
        let report = check_device(&mut branch);
        assert!(report.is_clean(), "budget {entries}: {:?}", report.errors);
        let mut volume = mount(branch).unwrap();
        assert_eq!(volume.lookup_root("seed").unwrap(), Some(id));
        assert_eq!(volume.read_file(id).unwrap(), b"original");
    }
}

#[test]
fn overlay_publication_cuts_recover_exact_old_or_new_namespace() {
    use afsplus_block::{powercut::for_each_overlay_crash_state, RecordingBackend};
    let (base, _, now) = populated_base();
    let mut volume = mount(RecordingBackend::new(base.clone())).unwrap();
    volume
        .create_file_in_root("published", b"durable payload", now)
        .unwrap();
    let (_, log) = volume.into_device().into_parts();
    let base = OverlayBackend::new(
        base,
        OverlayLimits {
            branches: 3,
            entries: 768,
        },
    )
    .unwrap();
    let mut visits = 0;
    for cut in 0..=log.len() {
        for_each_overlay_crash_state(&base, &log, cut, |mut image, description| {
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{description}: {:?}", report.errors);
            let mut volume = mount(image).unwrap();
            let seed = volume.lookup_root("seed").unwrap().unwrap();
            assert_eq!(volume.read_file(seed).unwrap(), b"original");
            if let Some(id) = volume.lookup_root("published").unwrap() {
                assert_eq!(volume.read_file(id).unwrap(), b"durable payload");
            } else {
                assert!(cut < log.len(), "completed publication was lost");
            }
            visits += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!((base.live_branches(), base.live_entries()), (1, 0));
    }
    assert!(visits > log.len());
}
