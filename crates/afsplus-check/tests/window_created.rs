//! The open window as a delayed group commit (ADR-121): a lookup sees what
//! the window changed, a file the window created is read, written and
//! truncated before any commit, and every crash mounts at a whole prefix.

use afsplus_block::{crash_states, MemoryBackend, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::volume::{BatchOp, WINDOW_CREATED_REWRITE_MAX};
use afsplus_core::{mkfs, mount, CoreError, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};

const BS: usize = 4096;

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 4096);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [91u8; 16],
            label: "Window".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: NamePolicy::Sensitive,
            timestamp: ts(0),
        },
    )
    .unwrap();
    dev
}

fn create<'a>(name: &'a str, content: &'a [u8]) -> BatchOp<'a> {
    BatchOp::CreateFile {
        parent_id: OBJECT_ROOT,
        name,
        content,
    }
}

fn read_all(vol: &mut afsplus_core::Volume<impl afsplus_block::BlockDevice>, id: u64) -> Vec<u8> {
    let mut buffer = vec![0u8; 1 << 20];
    let count = vol.read_file_at(id, 0, &mut buffer).unwrap();
    buffer.truncate(count);
    buffer
}

#[test]
fn a_lookup_sees_what_the_window_changed() {
    let mut vol = mount(formatted()).unwrap();
    vol.create_file_in_root("kept", b"k", ts(1)).unwrap();
    let id = vol.window_op(&create("new", b"n"), ts(2)).unwrap().unwrap();
    assert_eq!(
        vol.lookup_in_directory(OBJECT_ROOT, "new").unwrap(),
        Some(id)
    );
    vol.window_op(
        &BatchOp::DeleteFile {
            parent_id: OBJECT_ROOT,
            name: "kept",
        },
        ts(3),
    )
    .unwrap();
    assert_eq!(vol.lookup_in_directory(OBJECT_ROOT, "kept").unwrap(), None);
    vol.window_op(
        &BatchOp::Rename {
            source_parent_id: OBJECT_ROOT,
            source_name: "new",
            target_parent_id: OBJECT_ROOT,
            target_name: "renamed",
            replace: false,
        },
        ts(4),
    )
    .unwrap();
    assert_eq!(vol.lookup_in_directory(OBJECT_ROOT, "new").unwrap(), None);
    assert_eq!(
        vol.lookup_in_directory(OBJECT_ROOT, "renamed").unwrap(),
        Some(id)
    );
    assert!(vol.window_changes_directory(OBJECT_ROOT));
    // Nothing of it is committed: a crash here keeps the checkpoint.
    let generation = vol.generation();
    let mut vol = mount(vol.into_device()).unwrap();
    assert_eq!(vol.generation(), generation);
    assert!(vol
        .lookup_in_directory(OBJECT_ROOT, "kept")
        .unwrap()
        .is_some());
    assert_eq!(
        vol.lookup_in_directory(OBJECT_ROOT, "renamed").unwrap(),
        None
    );
}

#[test]
fn a_file_the_window_created_is_written_truncated_and_committed() {
    let mut vol = mount(formatted()).unwrap();
    let id = vol.window_op(&create("f", b""), ts(1)).unwrap().unwrap();
    vol.window_write_file_at(id, 0, b"hello", ts(2)).unwrap();
    assert_eq!(read_all(&mut vol, id), b"hello");
    vol.window_write_file_at(id, 3, b"LOXX", ts(3)).unwrap();
    assert_eq!(read_all(&mut vol, id), b"helLOXX");
    // Past the end leaves a hole of zeros; across a block boundary works.
    vol.window_write_file_at(id, 5000, b"far", ts(4)).unwrap();
    let content = read_all(&mut vol, id);
    assert_eq!(content.len(), 5003);
    assert_eq!(&content[..7], b"helLOXX");
    assert!(content[7..5000].iter().all(|byte| *byte == 0));
    vol.window_truncate_file(id, 4, ts(5)).unwrap();
    assert_eq!(read_all(&mut vol, id), b"helL");
    assert_eq!(vol.visible_metadata(id).unwrap().unwrap().size_bytes, 4);
    // The log cannot describe these writes: fsync refuses and the caller
    // commits the window instead.
    assert!(matches!(
        vol.window_fsync(),
        Err(CoreError::PrototypeLimit(_))
    ));
    vol.window_commit(ts(6)).unwrap();
    let mut dev = vol.into_device();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    let id = vol.lookup_root("f").unwrap().unwrap();
    assert_eq!(vol.read_file(id).unwrap(), b"helL");
}

#[test]
fn a_created_file_past_the_bound_is_refused_and_the_window_kept() {
    let mut vol = mount(formatted()).unwrap();
    let id = vol
        .window_op(&create("big", b"start"), ts(1))
        .unwrap()
        .unwrap();
    let past = vec![7u8; WINDOW_CREATED_REWRITE_MAX as usize];
    assert!(matches!(
        vol.window_write_file_at(id, 1, &past, ts(2)),
        Err(CoreError::PrototypeLimit(_))
    ));
    assert_eq!(read_all(&mut vol, id), b"start");
    assert_eq!(
        vol.lookup_in_directory(OBJECT_ROOT, "big").unwrap(),
        Some(id)
    );
    // Committing and writing again is the caller's way on.
    vol.window_commit(ts(3)).unwrap();
    vol.window_write_file_at(id, 1, &past, ts(4)).unwrap();
    vol.window_commit(ts(5)).unwrap();
    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}

#[test]
fn a_logged_create_rewritten_after_its_fsync_recovers_as_logged() {
    let mut vol = mount(formatted()).unwrap();
    let id = vol.window_op(&create("g", b"v1"), ts(1)).unwrap().unwrap();
    vol.window_fsync().unwrap();
    vol.window_write_file_at(id, 0, b"v2 and more", ts(2))
        .unwrap();
    assert_eq!(read_all(&mut vol, id), b"v2 and more");
    assert!(matches!(
        vol.window_fsync(),
        Err(CoreError::PrototypeLimit(_))
    ));
    // Crash: what was synced is what comes back.
    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
    let mut vol = mount(dev).unwrap();
    let id = vol.lookup_root("g").unwrap().expect("the fsynced create");
    assert_eq!(vol.read_file(id).unwrap(), b"v1");
    let mut dev = vol.into_device();
    assert!(check_device(&mut dev).is_clean());
}

/// Every crash state of committing `ops` mounts clean at the old state or
/// at all of it, which `check_new` verifies.
fn all_or_nothing(
    ops: impl FnOnce(&mut afsplus_core::Volume<RecordingBackend<MemoryBackend>>),
    old_names: &[&str],
    check_new: impl Fn(&mut afsplus_core::Volume<MemoryBackend>, &str),
) {
    let base = {
        let mut vol = mount(formatted()).unwrap();
        vol.create_file_in_root("old", b"old", ts(1)).unwrap();
        vol.into_device()
    };
    let pre_generation = mount(base.clone()).unwrap().generation();
    let mut vol = mount(RecordingBackend::new(base.clone())).unwrap();
    ops(&mut vol);
    vol.window_commit(ts(9)).unwrap();
    let (_, log) = vol.into_device().into_parts();
    let (mut before, mut after) = (0u32, 0u32);
    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            let context = state.description.clone();
            let mut image = state.image;
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            let mut vol = mount(image).unwrap_or_else(|e| panic!("{context}: {e}"));
            if vol.generation() == pre_generation {
                before += 1;
                let names: Vec<String> =
                    vol.list_root().unwrap().into_iter().map(|e| e.0).collect();
                assert_eq!(names, old_names, "{context}");
            } else {
                after += 1;
                check_new(&mut vol, &context);
            }
        }
    }
    assert!(before > 0 && after > 0, "{before} {after}");
}

#[test]
fn every_crash_state_of_committing_created_and_rewritten_files_is_all_or_nothing() {
    all_or_nothing(
        |vol| {
            let a = vol.window_op(&create("a", b""), ts(2)).unwrap().unwrap();
            vol.window_write_file_at(a, 0, &[0xA5; 3000], ts(2))
                .unwrap();
            let b = vol.window_op(&create("b", b"bee"), ts(3)).unwrap().unwrap();
            vol.window_write_file_at(b, 3, b"!", ts(3)).unwrap();
        },
        &["old"],
        |vol, context| {
            let names: Vec<String> = vol.list_root().unwrap().into_iter().map(|e| e.0).collect();
            assert_eq!(names, ["a", "b", "old"], "{context}");
            let a = vol.lookup_root("a").unwrap().unwrap();
            assert_eq!(vol.read_file(a).unwrap(), vec![0xA5; 3000], "{context}");
            let b = vol.lookup_root("b").unwrap().unwrap();
            assert_eq!(vol.read_file(b).unwrap(), b"bee!", "{context}");
        },
    );
}

#[test]
fn every_crash_state_of_committing_a_rename_and_a_delete_is_all_or_nothing() {
    all_or_nothing(
        |vol| {
            vol.window_op(&create("n", b"new"), ts(2)).unwrap();
            vol.window_op(
                &BatchOp::Rename {
                    source_parent_id: OBJECT_ROOT,
                    source_name: "n",
                    target_parent_id: OBJECT_ROOT,
                    target_name: "moved",
                    replace: false,
                },
                ts(3),
            )
            .unwrap();
            vol.window_op(
                &BatchOp::DeleteFile {
                    parent_id: OBJECT_ROOT,
                    name: "old",
                },
                ts(4),
            )
            .unwrap();
        },
        &["old"],
        |vol, context| {
            let names: Vec<String> = vol.list_root().unwrap().into_iter().map(|e| e.0).collect();
            assert_eq!(names, ["moved"], "{context}");
            let moved = vol.lookup_root("moved").unwrap().unwrap();
            assert_eq!(vol.read_file(moved).unwrap(), b"new", "{context}");
        },
    );
}
