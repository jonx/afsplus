use std::time::Instant;

use afsplus_block::{crash_states, MemoryBackend, RecordedOp, RecordingBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{directory, object_map};
use afsplus_core::{mkfs, mount, MkfsParams, Volume};
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::dir::DirEntry;
use afsplus_format::ident::{Identification, RO_COMPAT_ORPHAN_DIRECTORY};
use afsplus_format::object::ObjectRecord;
use afsplus_format::tree::{TreeItem, TreeNode};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ORPHAN_DIRECTORY, OBJECT_ROOT};

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(DEFAULT_BLOCK_SIZE, 16_384);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x66; 16],
            label: "OrphanCleanup".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

fn record_transaction(
    base: &MemoryBackend,
    op: impl FnOnce(&mut Volume<RecordingBackend<MemoryBackend>>),
) -> Vec<RecordedOp> {
    let mut volume = mount(RecordingBackend::new(base.clone())).unwrap();
    op(&mut volume);
    volume.into_device().into_parts().1
}

fn assert_clean(context: &str, image: &mut MemoryBackend) {
    let report = check_device(image);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
}

fn populated_orphan() -> (MemoryBackend, u64, u64) {
    let mut volume = mount(formatted()).unwrap();
    let object = volume.create_file_in_root("open", b"data", ts(1)).unwrap();
    volume.orphan_file(OBJECT_ROOT, "open", ts(2)).unwrap();
    let root_lba = volume.stat(OBJECT_ROOT).unwrap().unwrap().data_root;
    (volume.into_device(), object, root_lba)
}

fn newest_checkpoint(dev: &MemoryBackend, ident: &Identification) -> Checkpoint {
    [1u64, 2]
        .into_iter()
        .filter_map(|lba| Checkpoint::decode(&dev.peek(lba), &ident.uuid).ok())
        .max_by_key(|checkpoint| checkpoint.generation)
        .unwrap()
}

fn orphan_tree_lba(dev: &mut MemoryBackend, ident: &Identification) -> u64 {
    let checkpoint = newest_checkpoint(dev, ident);
    let record_lba = object_map::lookup_lba(
        dev,
        &ident.geometry(),
        checkpoint.object_map_block,
        checkpoint.generation,
        OBJECT_ORPHAN_DIRECTORY,
    )
    .unwrap()
    .unwrap();
    ObjectRecord::decode(&dev.peek(record_lba))
        .unwrap()
        .data_root
}

fn require_checker_error(mut dev: MemoryBackend, needle: &str) {
    let report = check_device(&mut dev);
    assert!(
        report.errors.iter().any(|error| error.contains(needle)),
        "missing checker diagnostic {needle:?}: {:?}",
        report.errors
    );
}

#[test]
fn fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount() {
    let mut volume = mount(formatted()).unwrap();
    let object = volume
        .create_file_in_root("fragmented", b"", ts(1))
        .unwrap();
    for extent in 0..5u64 {
        volume
            .write_file_at(
                object,
                extent * 2 * DEFAULT_BLOCK_SIZE as u64,
                &[extent as u8 + 1; DEFAULT_BLOCK_SIZE],
                ts(2 + extent as i64),
            )
            .unwrap();
    }
    assert_eq!(
        volume
            .visible_metadata(object)
            .unwrap()
            .unwrap()
            .allocated_bytes,
        5 * DEFAULT_BLOCK_SIZE as u64
    );
    assert_eq!(
        volume
            .orphan_file(OBJECT_ROOT, "fragmented", ts(10))
            .unwrap(),
        object
    );
    volume.set_orphan_cleanup_extent_budget(2);

    let first = volume.cleanup_orphan(object, ts(11)).unwrap();
    assert_eq!(first.extents_removed, 2);
    assert!(!first.object_removed);
    assert!(first.still_pending);
    assert_eq!(
        volume
            .visible_metadata(object)
            .unwrap()
            .unwrap()
            .allocated_bytes,
        3 * DEFAULT_BLOCK_SIZE as u64
    );
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    let mut volume = mount(dev).unwrap();
    assert!(volume.orphan_object(object).unwrap());
    volume.set_orphan_cleanup_extent_budget(2);
    let second = volume.cleanup_orphan(object, ts(12)).unwrap();
    assert_eq!(second.extents_removed, 2);
    assert!(second.still_pending);
    assert_eq!(
        volume
            .visible_metadata(object)
            .unwrap()
            .unwrap()
            .allocated_bytes,
        DEFAULT_BLOCK_SIZE as u64
    );
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);

    let mut volume = mount(dev).unwrap();
    volume.set_orphan_cleanup_extent_budget(2);
    let final_step = volume.cleanup_orphan(object, ts(13)).unwrap();
    assert_eq!(final_step.extents_removed, 1);
    assert!(final_step.object_removed);
    assert!(!final_step.still_pending);
    assert!(volume.visible_metadata(object).unwrap().is_none());
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state() {
    let mut setup = mount(formatted()).unwrap();
    let object = setup
        .create_file_in_root("open", b"old-content", ts(1))
        .unwrap();
    let before_orphan = setup.into_device();

    let insertion_log = record_transaction(&before_orphan, |volume| {
        volume.orphan_file(OBJECT_ROOT, "open", ts(2)).unwrap();
    });
    for cut in 0..=insertion_log.len() {
        for mut state in crash_states(&before_orphan, &insertion_log, cut) {
            let context = format!("orphan insertion {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = mount(state.image).unwrap();
            match recovered.lookup_root("open").unwrap() {
                Some(visible) => {
                    assert_eq!(visible, object, "{context}");
                    assert_eq!(recovered.orphan_count().unwrap(), 0, "{context}");
                }
                None => {
                    assert!(recovered.orphan_object(object).unwrap(), "{context}");
                    assert_eq!(recovered.read_file(object).unwrap(), b"old-content");
                }
            }
        }
    }

    let mut orphaned = mount(before_orphan.clone()).unwrap();
    orphaned.orphan_file(OBJECT_ROOT, "open", ts(2)).unwrap();
    let orphaned = orphaned.into_device();
    let update_log = record_transaction(&orphaned, |volume| {
        volume
            .write_file_at(object, 0, b"new-content", ts(3))
            .unwrap();
    });
    for cut in 0..=update_log.len() {
        for mut state in crash_states(&orphaned, &update_log, cut) {
            let context = format!("orphan update {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = mount(state.image).unwrap();
            assert!(recovered.orphan_object(object).unwrap(), "{context}");
            let content = recovered.read_file(object).unwrap();
            assert!(
                content == b"old-content" || content == b"new-content",
                "{context}: recovered mixed content {content:?}"
            );
        }
    }

    let cleanup_log = record_transaction(&orphaned, |volume| {
        volume.set_orphan_cleanup_extent_budget(1);
        volume.cleanup_orphan(object, ts(4)).unwrap();
    });
    for cut in 0..=cleanup_log.len() {
        for mut state in crash_states(&orphaned, &cleanup_log, cut) {
            let context = format!("orphan cleanup {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = mount(state.image).unwrap();
            let pending = recovered.orphan_object(object).unwrap();
            if pending {
                let content = recovered.read_file(object).unwrap();
                assert!(
                    content == b"old-content" || content.is_empty(),
                    "{context}: cleanup recovered an invalid intermediate"
                );
            } else {
                assert!(recovered.visible_metadata(object).unwrap().is_none());
            }
        }
    }
}

#[test]
fn every_open_target_replace_cut_is_old_or_new_namespace() {
    let mut setup = mount(formatted()).unwrap();
    let source = setup
        .create_file_in_root("incoming", b"new", ts(1))
        .unwrap();
    let target = setup.create_file_in_root("target", b"old", ts(2)).unwrap();
    let base = setup.into_device();

    let log = record_transaction(&base, |volume| {
        volume
            .rename_replace_orphan_target(OBJECT_ROOT, "incoming", OBJECT_ROOT, "target", ts(3))
            .unwrap();
    });
    assert!(!log.is_empty());

    for cut in 0..=log.len() {
        for mut state in crash_states(&base, &log, cut) {
            let context = format!("open-target replace {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = mount(state.image).unwrap();
            let source_name = recovered.lookup_root("incoming").unwrap();
            let target_name = recovered.lookup_root("target").unwrap();
            match source_name {
                Some(id) => {
                    assert_eq!(id, source, "{context}");
                    assert_eq!(target_name, Some(target), "{context}");
                    assert_eq!(recovered.orphan_count().unwrap(), 0, "{context}");
                }
                None => {
                    assert_eq!(target_name, Some(source), "{context}");
                    assert!(recovered.orphan_object(target).unwrap(), "{context}");
                    assert_eq!(recovered.read_file(target).unwrap(), b"old", "{context}");
                }
            }
        }
    }
}

#[test]
fn checker_rejects_every_malformed_orphan_authority_surface() {
    let (base, object, root_lba) = populated_orphan();
    let ident = Identification::decode(&base.peek(0)).unwrap();
    let orphan_lba = orphan_tree_lba(&mut base.clone(), &ident);

    let mut missing_feature = base.clone();
    let mut missing_ident = ident.clone();
    missing_ident.features.ro_compat &= !RO_COMPAT_ORPHAN_DIRECTORY;
    missing_feature.apply_raw(0, &missing_ident.encode(DEFAULT_BLOCK_SIZE).unwrap());
    require_checker_error(missing_feature, "reserved internal ID range");

    let mut wrong_name = base.clone();
    let (mut orphan_node, generation) = TreeNode::decode(&wrong_name.peek(orphan_lba)).unwrap();
    let forged = DirEntry {
        key: b"000000000000ffff".to_vec(),
        name: b"000000000000ffff".to_vec(),
        child_type_hint: 1,
        child_id: object,
    };
    let (key, value) = directory::encode_entry(&ident, &forged).unwrap();
    orphan_node.items[0] = TreeItem { key, value };
    wrong_name.apply_raw(
        orphan_lba,
        &orphan_node.encode(DEFAULT_BLOCK_SIZE, generation).unwrap(),
    );
    require_checker_error(wrong_name, "invalid name or type");

    let mut wrong_type = base.clone();
    let (mut orphan_node, generation) = TreeNode::decode(&wrong_type.peek(orphan_lba)).unwrap();
    orphan_node.items[0].value[2] = 2;
    wrong_type.apply_raw(
        orphan_lba,
        &orphan_node.encode(DEFAULT_BLOCK_SIZE, generation).unwrap(),
    );
    require_checker_error(wrong_type, "type hint mismatch");

    for (name, child_id, hint, diagnostic) in [
        ("visible-orphan", object, 1, "link count"),
        (
            "internal-directory",
            OBJECT_ORPHAN_DIRECTORY,
            2,
            "exposes the reserved orphan directory",
        ),
    ] {
        let mut exposed = base.clone();
        let (mut root, generation) = TreeNode::decode(&exposed.peek(root_lba)).unwrap();
        let entry = DirEntry {
            key: name.as_bytes().to_vec(),
            name: name.as_bytes().to_vec(),
            child_type_hint: hint,
            child_id,
        };
        let (key, value) = directory::encode_entry(&ident, &entry).unwrap();
        root.items.push(TreeItem { key, value });
        root.subtree_items = root.items.len() as u64;
        exposed.apply_raw(
            root_lba,
            &root.encode(DEFAULT_BLOCK_SIZE, generation).unwrap(),
        );
        require_checker_error(exposed, diagnostic);
    }
}

#[test]
#[ignore = "optimized ADR-066 orphan cleanup qualification"]
fn orphan_cleanup_qualification_reports_bounded_resource_contract() {
    const EXTENTS: u64 = 33;
    const BUDGET: usize = 8;
    let mut volume = mount(formatted()).unwrap();
    let object = volume.create_file_in_root("many", b"", ts(1)).unwrap();
    for extent in 0..EXTENTS {
        volume
            .write_file_at(
                object,
                extent * 2 * DEFAULT_BLOCK_SIZE as u64,
                &[0x5a; DEFAULT_BLOCK_SIZE],
                ts(2 + extent as i64),
            )
            .unwrap();
    }
    volume.orphan_file(OBJECT_ROOT, "many", ts(40)).unwrap();

    let mut volume = mount(TraceBackend::new(volume.into_device())).unwrap();
    volume.set_orphan_cleanup_extent_budget(BUDGET);
    volume.device_mut().reset();
    let started = Instant::now();
    let progress = volume.cleanup_orphan(object, ts(41)).unwrap();
    let elapsed = started.elapsed();
    let commit = volume.last_commit_stats().unwrap();
    let io = volume.device_mut().stats();
    let remaining = volume
        .visible_metadata(object)
        .unwrap()
        .unwrap()
        .allocated_bytes
        / DEFAULT_BLOCK_SIZE as u64;
    assert_eq!(progress.extents_removed, BUDGET);
    assert!(progress.still_pending);
    assert_eq!(remaining, EXTENTS - BUDGET as u64);
    assert_eq!(io.flushes, 2);
    println!(
        "orphan-cleanup budget_extents={} removed_extents={} remaining_extents={} elapsed_us={} reads={} writes={} bytes_read={} bytes_written={} flushes={} metadata_blocks={} bitmap_pages={} allocator_ram_bytes={}",
        BUDGET,
        progress.extents_removed,
        remaining,
        elapsed.as_micros(),
        io.reads,
        io.writes,
        io.bytes_read,
        io.bytes_written,
        io.flushes,
        commit.metadata_blocks_written,
        commit.bitmap_pages_written,
        commit.alloc.allocator_ram_bytes,
    );

    let traced = volume.into_device();
    let mut dev = traced.into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
}
