use std::time::Instant;

use afsplus_block::{
    crash_states, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend, TraceBackend,
};
use afsplus_check::check_device;
use afsplus_core::{directory, object_map};
use afsplus_core::{mkfs, mount, mount_with_options, MkfsParams, MountOptions, Volume};
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

fn profile_mount<D: BlockDevice>(device: D, pages: usize) -> Volume<D> {
    let volume = mount_with_options(
        device,
        MountOptions {
            tree_cache_pages: std::num::NonZeroUsize::new(pages),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(volume.tree_cache_pages(), pages);
    volume
}

fn record_transaction(
    base: &MemoryBackend,
    pages: usize,
    op: impl FnOnce(&mut Volume<RecordingBackend<MemoryBackend>>),
) -> Vec<RecordedOp> {
    let mut volume = profile_mount(RecordingBackend::new(base.clone()), pages);
    op(&mut volume);
    volume.into_device().into_parts().1
}

fn assert_clean(context: &str, image: &mut MemoryBackend) {
    let report = check_device(image);
    assert!(report.is_clean(), "{context}: {:?}", report.errors);
    // A stopped intent-log tail is an admissible crash artifact. Retained
    // checkpoint findings are warnings too and must fail this oracle.
    assert!(
        report
            .warnings
            .iter()
            .all(|warning| warning.starts_with("intent log tail:")),
        "{context}: {:?}",
        report.warnings
    );
}

/// Whether the selected checkpoint's object map holds internal object 2.
fn orphan_directory_present(volume: &mut Volume<MemoryBackend>) -> bool {
    let checkpoint = volume.checkpoint().clone();
    let geometry = volume.ident().geometry();
    object_map::lookup_lba(
        volume.device_mut(),
        &geometry,
        checkpoint.object_map_block,
        checkpoint.generation,
        OBJECT_ORPHAN_DIRECTORY,
    )
    .unwrap()
    .is_some()
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
    for pages in [2, 4, 8, usize::MAX] {
        fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount_profile(pages);
    }
}

fn fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount_profile(pages: usize) {
    let mut volume = profile_mount(formatted(), pages);
    let object = volume
        .create_file_in_root("fragmented", b"", ts(1))
        .unwrap();
    let mut expected = vec![0; 9 * DEFAULT_BLOCK_SIZE];
    for extent in 0..5u64 {
        let offset = extent as usize * 2 * DEFAULT_BLOCK_SIZE;
        expected[offset..offset + DEFAULT_BLOCK_SIZE].fill(extent as u8 + 1);
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
    assert_eq!(volume.lookup_root("fragmented").unwrap(), None);
    assert!(volume.orphan_object(object).unwrap());
    volume.set_orphan_cleanup_extent_budget(2);

    assert_eq!(volume.read_file(object).unwrap(), expected);
    // ADR-066 removes whole extent records from the logical end and publishes
    // the smaller file. Its size becomes the logical start of the lowest
    // removed extent: extents at blocks 6 and 8 leave a six-block prefix whose
    // block 5 is the original sparse hole, then blocks 2 and 4 leave two.
    const FIRST_PREFIX: usize = 6 * DEFAULT_BLOCK_SIZE;
    const SECOND_PREFIX: usize = 2 * DEFAULT_BLOCK_SIZE;
    let first = volume.cleanup_orphan(object, ts(11)).unwrap();
    assert_eq!(volume.read_file(object).unwrap(), expected[..FIRST_PREFIX]);
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
    assert_clean(&format!("pages={pages} first cleanup step"), &mut dev);

    let mut volume = profile_mount(dev, pages);
    assert!(volume.orphan_object(object).unwrap());
    volume.set_orphan_cleanup_extent_budget(2);
    assert_eq!(volume.read_file(object).unwrap(), expected[..FIRST_PREFIX]);
    let second = volume.cleanup_orphan(object, ts(12)).unwrap();
    assert_eq!(volume.read_file(object).unwrap(), expected[..SECOND_PREFIX]);
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
    assert_clean(&format!("pages={pages} second cleanup step"), &mut dev);

    let mut volume = profile_mount(dev, pages);
    volume.set_orphan_cleanup_extent_budget(2);
    assert_eq!(volume.read_file(object).unwrap(), expected[..SECOND_PREFIX]);
    let final_step = volume.cleanup_orphan(object, ts(13)).unwrap();
    assert_eq!(final_step.extents_removed, 1);
    assert!(final_step.object_removed);
    assert!(!final_step.still_pending);
    assert!(volume.visible_metadata(object).unwrap().is_none());
    let mut dev = volume.into_device();
    assert_clean(&format!("pages={pages} final cleanup step"), &mut dev);
    let mut remounted = profile_mount(dev, pages);
    assert_eq!(remounted.lookup_root("fragmented").unwrap(), None);
    assert_eq!(remounted.orphan_count().unwrap(), 0);
    assert!(remounted.visible_metadata(object).unwrap().is_none());
    let generation = remounted.generation();
    let again = remounted.cleanup_orphan(object, ts(14)).unwrap();
    assert_eq!(again.extents_removed, 0);
    assert!(!again.still_pending && !again.object_removed);
    assert_eq!(remounted.generation(), generation);
    assert_clean(
        &format!("pages={pages} absent retry"),
        remounted.device_mut(),
    );
    eprintln!(
        "fragmented orphan pages={pages} removals=2/2/1 prefixes=6/2/0 blocks absent-retry=no-op"
    );
}

#[test]
fn every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state() {
    for pages in [2, 4, 8, usize::MAX] {
        every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state_profile(pages);
    }
}

fn every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state_profile(pages: usize) {
    let mut setup = profile_mount(formatted(), pages);
    let object = setup
        .create_file_in_root("open", b"old-content", ts(1))
        .unwrap();
    let generation = setup.generation();
    let before_orphan = setup.into_device();
    let insertion_log = record_transaction(&before_orphan, pages, |volume| {
        volume.orphan_file(OBJECT_ROOT, "open", ts(2)).unwrap();
    });
    let mut insertion = [0u64; 3];
    for cut in 0..=insertion_log.len() {
        for mut state in crash_states(&before_orphan, &insertion_log, cut) {
            let context = format!("pages={pages} orphan insertion {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = profile_mount(state.image, pages);
            let delta = recovered.generation().checked_sub(generation).unwrap();
            assert!(delta <= 2, "{context}: unexpected generation delta {delta}");
            insertion[delta as usize] += 1;
            assert_eq!(
                recovered.read_file(object).unwrap(),
                b"old-content",
                "{context}"
            );
            if delta < 2 {
                // Generation +1 is the durable preparatory orphan directory;
                // the application name remains unchanged until the move.
                assert_eq!(
                    orphan_directory_present(&mut recovered),
                    delta == 1,
                    "{context}: preparatory orphan directory"
                );
                assert_eq!(
                    recovered.lookup_root("open").unwrap(),
                    Some(object),
                    "{context}"
                );
                assert_eq!(recovered.orphan_count().unwrap(), 0, "{context}");
                assert_eq!(
                    recovered.orphan_file(OBJECT_ROOT, "open", ts(2)).unwrap(),
                    object
                );
            } else {
                assert_eq!(recovered.lookup_root("open").unwrap(), None, "{context}");
            }
            assert!(recovered.orphan_object(object).unwrap(), "{context}");
            assert_eq!(recovered.orphan_count().unwrap(), 1);
            assert_eq!(recovered.lookup_root("open").unwrap(), None);
            assert_eq!(recovered.read_file(object).unwrap(), b"old-content");
            assert_clean(&context, recovered.device_mut());
        }
    }
    assert!(insertion.iter().all(|count| *count > 0));
    let mut orphaned = profile_mount(before_orphan.clone(), pages);
    orphaned.orphan_file(OBJECT_ROOT, "open", ts(2)).unwrap();
    let orphan_generation = orphaned.generation();
    let orphaned = orphaned.into_device();
    let update_log = record_transaction(&orphaned, pages, |volume| {
        volume
            .write_file_at(object, 0, b"new-content", ts(3))
            .unwrap();
    });
    let mut updates = [0u64; 2];
    for cut in 0..=update_log.len() {
        for mut state in crash_states(&orphaned, &update_log, cut) {
            let context = format!("pages={pages} orphan update {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = profile_mount(state.image, pages);
            let delta = recovered
                .generation()
                .checked_sub(orphan_generation)
                .unwrap();
            assert!(delta <= 1, "{context}: unexpected generation delta {delta}");
            updates[delta as usize] += 1;
            assert_eq!(recovered.lookup_root("open").unwrap(), None);
            assert!(recovered.orphan_object(object).unwrap(), "{context}");
            assert_eq!(
                recovered.read_file(object).unwrap(),
                if delta == 0 {
                    b"old-content"
                } else {
                    b"new-content"
                },
                "{context}"
            );
            if delta == 0 {
                recovered
                    .write_file_at(object, 0, b"new-content", ts(3))
                    .unwrap();
            }
            assert_eq!(recovered.read_file(object).unwrap(), b"new-content");
            assert_eq!(recovered.orphan_count().unwrap(), 1);
            assert_clean(&context, recovered.device_mut());
        }
    }
    assert!(updates.iter().all(|count| *count > 0));
    let cleanup_log = record_transaction(&orphaned, pages, |volume| {
        volume.set_orphan_cleanup_extent_budget(1);
        volume.cleanup_orphan(object, ts(4)).unwrap();
    });
    let mut cleanup = [0u64; 3];
    for cut in 0..=cleanup_log.len() {
        for mut state in crash_states(&orphaned, &cleanup_log, cut) {
            let context = format!("pages={pages} orphan cleanup {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = profile_mount(state.image, pages);
            let delta = recovered
                .generation()
                .checked_sub(orphan_generation)
                .unwrap();
            assert!(delta <= 2, "{context}: unexpected generation delta {delta}");
            cleanup[delta as usize] += 1;
            assert_eq!(recovered.lookup_root("open").unwrap(), None);
            if delta < 2 {
                assert!(recovered.orphan_object(object).unwrap());
                assert_eq!(
                    recovered.read_file(object).unwrap().as_slice(),
                    if delta == 0 {
                        b"old-content".as_slice()
                    } else {
                        b""
                    },
                    "{context}"
                );
                recovered.set_orphan_cleanup_extent_budget(1);
                let progress = recovered.cleanup_orphan(object, ts(4)).unwrap();
                assert!(progress.object_removed && !progress.still_pending);
            }
            assert_eq!(recovered.orphan_count().unwrap(), 0);
            assert!(recovered.visible_metadata(object).unwrap().is_none());
            let final_generation = recovered.generation();
            let again = recovered.cleanup_orphan(object, ts(5)).unwrap();
            assert_eq!(again.extents_removed, 0);
            assert!(!again.object_removed && !again.still_pending);
            assert_eq!(recovered.generation(), final_generation);
            assert_clean(&context, recovered.device_mut());
        }
    }
    assert!(cleanup.iter().all(|count| *count > 0));
    eprintln!("orphan lifecycle pages={pages} insertion={insertion:?} update={updates:?} cleanup={cleanup:?}");
}

#[test]
fn every_open_target_replace_cut_is_old_or_new_namespace() {
    for pages in [2, 4, 8, usize::MAX] {
        every_open_target_replace_cut_is_old_or_new_namespace_profile(pages);
    }
}

fn every_open_target_replace_cut_is_old_or_new_namespace_profile(pages: usize) {
    let mut setup = profile_mount(formatted(), pages);
    let source = setup
        .create_file_in_root("incoming", b"new", ts(1))
        .unwrap();
    let target = setup.create_file_in_root("target", b"old", ts(2)).unwrap();
    let generation = setup.generation();
    let base = setup.into_device();

    let log = record_transaction(&base, pages, |volume| {
        volume
            .rename_replace_orphan_target(OBJECT_ROOT, "incoming", OBJECT_ROOT, "target", ts(3))
            .unwrap();
    });
    assert!(!log.is_empty());

    let mut outcomes = [0u64; 3];
    for cut in 0..=log.len() {
        for mut state in crash_states(&base, &log, cut) {
            let context = format!("pages={pages} open-target replace {}", state.description);
            assert_clean(&context, &mut state.image);
            let mut recovered = profile_mount(state.image, pages);
            let delta = recovered.generation().checked_sub(generation).unwrap();
            assert!(delta <= 2, "{context}: unexpected generation delta {delta}");
            outcomes[delta as usize] += 1;
            assert_eq!(recovered.read_file(source).unwrap(), b"new", "{context}");
            assert_eq!(recovered.read_file(target).unwrap(), b"old", "{context}");
            let source_name = recovered.lookup_root("incoming").unwrap();
            let target_name = recovered.lookup_root("target").unwrap();
            match source_name {
                Some(id) => {
                    assert!(delta < 2, "{context}");
                    // Generation +1 publishes only the preparatory directory.
                    assert_eq!(
                        orphan_directory_present(&mut recovered),
                        delta == 1,
                        "{context}: preparatory orphan directory"
                    );
                    assert_eq!(id, source, "{context}");
                    assert_eq!(target_name, Some(target), "{context}");
                    assert_eq!(recovered.orphan_count().unwrap(), 0, "{context}");
                }
                None => {
                    assert_eq!(delta, 2, "{context}");
                    assert_eq!(target_name, Some(source), "{context}");
                    assert!(recovered.orphan_object(target).unwrap(), "{context}");
                    assert_eq!(recovered.read_file(target).unwrap(), b"old", "{context}");
                }
            }
            if source_name.is_some() {
                recovered
                    .rename_replace_orphan_target(
                        OBJECT_ROOT,
                        "incoming",
                        OBJECT_ROOT,
                        "target",
                        ts(3),
                    )
                    .unwrap();
            }
            assert_eq!(recovered.lookup_root("incoming").unwrap(), None);
            assert_eq!(recovered.lookup_root("target").unwrap(), Some(source));
            assert_eq!(recovered.read_file(source).unwrap(), b"new");
            assert_eq!(recovered.read_file(target).unwrap(), b"old");
            assert!(recovered.orphan_object(target).unwrap());
            assert_eq!(recovered.orphan_count().unwrap(), 1);
            assert_clean(&context, recovered.device_mut());
        }
    }
    assert!(outcomes.iter().all(|count| *count > 0));
    eprintln!("orphan replace pages={pages} outcomes={outcomes:?}");
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
    orphan_node.items[0] = TreeItem {
        key: key.into(),
        value: value.into(),
    };
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
        root.items.push(TreeItem {
            key: key.into(),
            value: value.into(),
        });
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
