//! Mount-policy and feature-negotiation qualification for Mountable Alpha-0.

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{
    layout, mkfs, mount, mount_with_options, CoreError, MkfsParams, MountMode, MountOptions,
};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64, nanoseconds: u32) -> Timespec {
    Timespec {
        seconds,
        nanoseconds,
    }
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 4096);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [91u8; 16],
            label: "MountModes".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(1, 0),
        },
    )
    .unwrap();
    dev
}

fn rewrite_ident(dev: &mut MemoryBackend, edit: impl FnOnce(&mut Identification)) {
    let mut ident = Identification::decode(&dev.peek(layout::IDENT_LBA)).unwrap();
    edit(&mut ident);
    dev.apply_raw(layout::IDENT_LBA, &ident.encode(BS).unwrap());
}

#[test]
fn no_changes_observes_pending_log_with_zero_writes() {
    let operation_time = ts(42, 123_456_789);
    let dev = {
        let mut vol = mount(formatted()).unwrap();
        vol.window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "durable",
                content: b"payload",
            },
            operation_time,
        )
        .unwrap();
        vol.window_fsync().unwrap();
        vol.into_device()
    };

    let traced = TraceBackend::new(dev);
    let mut vol = mount_with_options(
        traced,
        MountOptions {
            mode: MountMode::NoChanges,
        },
    )
    .unwrap();
    assert_eq!(vol.generation(), 1);
    assert_eq!(vol.pending_intent_records(), 1);
    assert_eq!(vol.lookup_root("durable").unwrap(), None, "pre-replay view");
    assert!(matches!(
        vol.create_file_in_root("forbidden", b"x", operation_time),
        Err(CoreError::ReadOnly)
    ));
    let traced = vol.into_device();
    assert_eq!(traced.stats().writes, 0);
    assert_eq!(traced.stats().flushes, 0);

    // Recovery is the explicit write-on-open mode. It replays first and then
    // exposes a read-only recovered view with the original operation time.
    let mut recovered = mount_with_options(
        traced.into_inner(),
        MountOptions {
            mode: MountMode::Recovery,
        },
    )
    .unwrap();
    assert_eq!(recovered.generation(), 2);
    assert_eq!(recovered.pending_intent_records(), 0);
    let object = recovered
        .lookup_root("durable")
        .unwrap()
        .expect("replayed create");
    let stat = recovered.stat(object).unwrap().unwrap();
    assert_eq!(stat.created, operation_time);
    assert_eq!(stat.modified, operation_time);
    assert_eq!(stat.changed, operation_time);
    let root = recovered.stat(OBJECT_ROOT).unwrap().unwrap();
    assert_eq!(root.modified, operation_time);
    assert!(matches!(
        recovered.create_file_in_root("still-forbidden", b"x", operation_time),
        Err(CoreError::ReadOnly)
    ));
}

#[test]
fn compatibility_classes_control_mount_policy() {
    const UNKNOWN: u64 = 1 << 63;

    let mut ro_compat = formatted();
    rewrite_ident(&mut ro_compat, |ident| ident.features.ro_compat |= UNKNOWN);
    let report = check_device(&mut ro_compat.clone());
    assert!(report.is_clean());
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("read-only-compatible")));
    assert!(matches!(
        mount(ro_compat.clone()),
        Err(CoreError::ReadOnlyRequiredFeatures(UNKNOWN))
    ));
    mount_with_options(
        ro_compat,
        MountOptions {
            mode: MountMode::NoChanges,
        },
    )
    .expect("unknown RO_COMPAT is safe in NO_CHANGES");

    let mut incompat = formatted();
    rewrite_ident(&mut incompat, |ident| ident.features.incompat |= UNKNOWN);
    let report = check_device(&mut incompat.clone());
    assert!(!report.is_clean());
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("incompatible")));
    assert!(matches!(
        mount_with_options(
            incompat,
            MountOptions {
                mode: MountMode::NoChanges
            },
        ),
        Err(CoreError::UnsupportedIncompatFeatures(UNKNOWN))
    ));

    let mut compat = formatted();
    rewrite_ident(&mut compat, |ident| ident.features.compat |= UNKNOWN);
    mount(compat).expect("unknown COMPAT is safe to ignore");
}

#[test]
fn snapshot_record_codecs_do_not_implicitly_enable_snapshot_mounts() {
    use afsplus_format::ident::INCOMPAT_PERSISTENT_SNAPSHOTS;
    let mut image = formatted();
    rewrite_ident(&mut image, |ident| {
        ident.features.incompat |= INCOMPAT_PERSISTENT_SNAPSHOTS
    });
    for mode in [
        MountMode::ReadWrite,
        MountMode::ReadOnly,
        MountMode::NoChanges,
        MountMode::Recovery,
    ] {
        assert!(matches!(
            mount_with_options(image.clone(), MountOptions { mode }),
            Err(CoreError::UnsupportedIncompatFeatures(
                INCOMPAT_PERSISTENT_SNAPSHOTS
            ))
        ));
    }
    let report = check_device(&mut image);
    assert!(!report.is_clean());
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("incompatible")));
}

#[test]
fn selected_snapshot_extension_mismatch_never_falls_back() {
    use afsplus_format::checkpoint::{Checkpoint, SnapshotRoots};
    use afsplus_format::ident::INCOMPAT_PERSISTENT_SNAPSHOTS;
    for extended in [false, true] {
        let mut image = formatted();
        let mut ident = Identification::decode(&image.peek(layout::IDENT_LBA)).unwrap();
        if !extended {
            ident.features.incompat |= INCOMPAT_PERSISTENT_SNAPSHOTS;
        }
        image.apply_raw(layout::IDENT_LBA, &ident.encode(BS).unwrap());
        let mut newest = Checkpoint::decode(&image.peek(layout::CKPT_SLOT_A), &ident.uuid).unwrap();
        newest.generation += 1;
        newest.committed_tx_id = newest.generation;
        if extended {
            newest.snapshot_roots = Some(SnapshotRoots {
                registry: 100,
                lifetimes: 101,
            });
        }
        image.apply_raw(layout::CKPT_SLOT_B, &newest.encode(BS).unwrap());
        let error = match afsplus_core::mount::select_checkpoint(&mut image, &ident) {
            Ok(_) => panic!("mismatched selected snapshot checkpoint was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("snapshot extension disagrees"));
        // The same extended checkpoint selects normally when its feature agrees.
        if extended {
            ident.features.incompat |= INCOMPAT_PERSISTENT_SNAPSHOTS;
            image.apply_raw(layout::IDENT_LBA, &ident.encode(BS).unwrap());
            let chosen = afsplus_core::mount::select_checkpoint(&mut image, &ident).unwrap();
            assert_eq!(chosen.chosen.generation, newest.generation);
            assert!(matches!(
                mount(image),
                Err(CoreError::UnsupportedIncompatFeatures(
                    INCOMPAT_PERSISTENT_SNAPSHOTS
                ))
            ));
        }
    }
}
