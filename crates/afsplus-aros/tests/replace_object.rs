//! `ACTION_REPLACE_OBJECT` as the adapter serves it: the filesystem-neutral
//! contract of dos.library `ReplaceObject`, its atomicity under a power cut,
//! and the three-step route a caller takes where the action is absent.

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, LockAccess, OpenMode};
use afsplus_block::{crash_states, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::{Durability, Vfs};

/// `FIBF_DELETE`: set, the object may not be deleted.
const DELETE_FORBIDDEN: u32 = 1;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC6; 16],
            label: "Replace".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: timestamp(0),
        },
    )
    .unwrap();
    device
}

fn adapter<D: BlockDevice>(device: D) -> ArosAdapter<D> {
    ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig::default(),
    )
}

fn device_of<D: BlockDevice>(adapter: ArosAdapter<D>) -> D {
    adapter.into_vfs().unwrap().into_volume().into_device()
}

fn put<D: BlockDevice>(adapter: &mut ArosAdapter<D>, name: &[u8], content: &[u8]) {
    let file = adapter
        .open(None, name, OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.write(file, content, timestamp(2)).unwrap();
    adapter.close(file).unwrap();
}

/// Contents under `name`, `None` when the name is absent.
fn get<D: BlockDevice>(adapter: &mut ArosAdapter<D>, name: &[u8]) -> Option<Vec<u8>> {
    let file = match adapter.open(None, name, OpenMode::OldFile, timestamp(900)) {
        Ok(file) => file,
        Err(ArosError::ObjectNotFound) => return None,
        Err(error) => panic!("{}: {error:?}", String::from_utf8_lossy(name)),
    };
    let size = adapter.file_size(file).unwrap() as usize;
    let mut content = vec![0u8; size];
    assert_eq!(adapter.read_at(file, 0, &mut content).unwrap(), size);
    adapter.close(file).unwrap();
    Some(content)
}

fn names<D: BlockDevice>(adapter: &mut ArosAdapter<D>) -> Vec<String> {
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    let mut names = Vec::new();
    loop {
        match adapter.examine_next(root) {
            Ok(info) => names.push(String::from_utf8(info.name).unwrap()),
            Err(ArosError::NoMoreEntries) => break,
            Err(error) => panic!("{error:?}"),
        }
    }
    adapter.free_lock(root).unwrap();
    names.sort();
    names
}

#[test]
fn every_refusal_leaves_both_names_unchanged() {
    let mut adapter = adapter(formatted());
    put(&mut adapter, b"new", b"NEW");
    put(&mut adapter, b"old", b"OLD");
    let directory = adapter
        .create_directory(None, b"dir", timestamp(3))
        .unwrap();
    adapter.free_lock(directory).unwrap();
    adapter
        .make_soft_link(None, b"link", b"old", timestamp(3))
        .unwrap();
    let before = names(&mut adapter);
    assert_eq!(before, ["dir", "link", "new", "old"]);

    let unchanged = |adapter: &mut ArosAdapter<MemoryBackend>| {
        assert_eq!(names(adapter), ["dir", "link", "new", "old"]);
        assert_eq!(get(adapter, b"new").unwrap(), b"NEW");
        assert_eq!(get(adapter, b"old").unwrap(), b"OLD");
    };

    // The source must be a regular file.
    for source in [&b"dir"[..], b"link"] {
        assert_eq!(
            adapter.replace(None, source, None, b"old", timestamp(4)),
            Err(ArosError::ObjectWrongType)
        );
        // Without a target too: only a file is ever published this way.
        assert_eq!(
            adapter.replace(None, source, None, b"free", timestamp(4)),
            Err(ArosError::ObjectWrongType)
        );
    }
    // The target, when present, must be a regular file.
    for target in [&b"dir"[..], b"link"] {
        assert_eq!(
            adapter.replace(None, b"new", None, target, timestamp(4)),
            Err(ArosError::ObjectWrongType)
        );
    }
    unchanged(&mut adapter);

    // A lock on the target, shared or exclusive, and an open handle.
    for access in [LockAccess::Shared, LockAccess::Exclusive] {
        let held = adapter.locate(None, b"old", access).unwrap();
        assert_eq!(
            adapter.replace(None, b"new", None, b"old", timestamp(5)),
            Err(ArosError::ObjectInUse)
        );
        adapter.free_lock(held).unwrap();
    }
    let open = adapter
        .open(None, b"old", OpenMode::OldFile, timestamp(5))
        .unwrap();
    assert_eq!(
        adapter.replace(None, b"new", None, b"old", timestamp(5)),
        Err(ArosError::ObjectInUse)
    );
    adapter.close(open).unwrap();
    unchanged(&mut adapter);

    // A delete-protected target. Protecting the source refuses nothing: the
    // source is renamed, not deleted.
    adapter
        .set_protection(None, b"old", DELETE_FORBIDDEN, timestamp(6))
        .unwrap();
    assert_eq!(
        adapter.replace(None, b"new", None, b"old", timestamp(6)),
        Err(ArosError::DeleteProtected)
    );
    unchanged(&mut adapter);
    adapter
        .set_protection(None, b"old", 0, timestamp(6))
        .unwrap();

    // A write-protected volume refuses before it looks at either name.
    adapter.set_write_protect(true, 7).unwrap();
    for source in [&b"new"[..], b"absent", b"dir"] {
        assert_eq!(
            adapter.replace(None, source, None, b"old", timestamp(7)),
            Err(ArosError::DiskWriteProtected)
        );
    }
    adapter.set_write_protect(false, 7).unwrap();
    unchanged(&mut adapter);

    // An absent source.
    assert_eq!(
        adapter.replace(None, b"absent", None, b"old", timestamp(8)),
        Err(ArosError::ObjectNotFound)
    );
    unchanged(&mut adapter);

    let mut device = device_of(adapter);
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}

#[test]
fn one_object_under_both_names_succeeds_and_changes_nothing() {
    let mut adapter = adapter(formatted());
    put(&mut adapter, b"file", b"DATA");
    let lock = adapter.locate(None, b"file", LockAccess::Shared).unwrap();
    adapter
        .make_hard_link(None, b"alias", lock, timestamp(3))
        .unwrap();
    let modified = adapter.examine_lock(lock).unwrap().modified;

    // The same entry, in another spelling of an insensitive volume too, and
    // two hard links of one object. The held lock and a delete-protected
    // object do not refuse it: nothing is deleted.
    adapter
        .set_protection(None, b"file", DELETE_FORBIDDEN, timestamp(4))
        .unwrap();
    for (source, target) in [
        (&b"file"[..], &b"file"[..]),
        (b"file", b"FILE"),
        (b"file", b"alias"),
        (b"alias", b"file"),
    ] {
        adapter
            .replace(None, source, None, target, timestamp(50))
            .unwrap();
        assert_eq!(names(&mut adapter), ["alias", "file"]);
    }
    assert_eq!(adapter.examine_lock(lock).unwrap().modified, modified);
    adapter.free_lock(lock).unwrap();
    assert_eq!(get(&mut adapter, b"file").unwrap(), b"DATA");
    assert_eq!(get(&mut adapter, b"alias").unwrap(), b"DATA");
}

#[test]
fn without_a_target_it_is_a_rename() {
    let mut adapter = adapter(formatted());
    put(&mut adapter, b"new", b"NEW");
    let directory = adapter
        .create_directory(None, b"dir", timestamp(3))
        .unwrap();
    adapter
        .replace(None, b"new", Some(directory), b"moved", timestamp(4))
        .unwrap();
    adapter.free_lock(directory).unwrap();
    assert_eq!(names(&mut adapter), ["dir"]);
    let directory = adapter.locate(None, b"dir", LockAccess::Shared).unwrap();
    let moved = adapter
        .locate(Some(directory), b"moved", LockAccess::Shared)
        .unwrap();
    assert_eq!(adapter.examine_lock(moved).unwrap().size, 3);
}

/// What a remount may show after a cut during `ReplaceObject("new", "old")`:
/// the target name always exists, with the old or the new contents, and the
/// source name is gone exactly when the target holds the new contents.
fn replace_state(source: Option<&[u8]>, target: Option<&[u8]>) -> Option<bool> {
    match (source, target) {
        (Some(b"NEW"), Some(b"OLD")) => Some(false),
        (None, Some(b"NEW")) => Some(true),
        _ => None,
    }
}

/// Runs `operation` on a recording mount of `base`, then every modeled power
/// cut of what it wrote, and hands each remounted survivor to `inspect`.
fn for_each_cut(
    base: &MemoryBackend,
    durability: Durability,
    operation: impl FnOnce(&mut ArosAdapter<RecordingBackend<MemoryBackend>>),
    mut inspect: impl FnMut(&str, usize, usize, &mut ArosAdapter<MemoryBackend>),
) {
    let mut recording = adapter(RecordingBackend::new(base.clone()));
    recording.set_durability(durability).unwrap();
    operation(&mut recording);
    let log = device_of(recording).into_parts().1;
    assert!(log.iter().any(|op| matches!(op, RecordedOp::Flush)));
    for cut in 0..=log.len() {
        for mut state in crash_states(base, &log, cut) {
            let context = format!(
                "{durability:?} cut {cut}/{}: {}",
                log.len(),
                state.description
            );
            let report = check_device(&mut state.image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
            assert!(
                report
                    .warnings
                    .iter()
                    .all(|warning| warning.starts_with("intent log tail:")),
                "{context}: {:?}",
                report.warnings
            );
            let mut survivor = adapter(state.image);
            inspect(&context, cut, log.len(), &mut survivor);
            let mut image = device_of(survivor);
            let report = check_device(&mut image);
            assert!(report.is_clean(), "{context}: {:?}", report.errors);
        }
    }
}

fn replace_base() -> MemoryBackend {
    let mut setup = adapter(formatted());
    put(&mut setup, b"new", b"NEW");
    put(&mut setup, b"old", b"OLD");
    put(&mut setup, b"kept", b"KEPT");
    setup.flush().unwrap();
    device_of(setup)
}

#[test]
fn every_cut_of_a_replace_shows_the_old_or_the_new_target() {
    // The oracle refuses what the contract forbids, whatever the volume does.
    assert_eq!(replace_state(None, None), None);
    assert_eq!(replace_state(Some(b"NEW"), None), None);
    assert_eq!(replace_state(None, Some(b"OLD")), None);
    assert_eq!(replace_state(Some(b"NEW"), Some(b"NEW")), None);
    assert_eq!(replace_state(Some(b"OLD"), Some(b"NEW")), None);
    assert_eq!(replace_state(None, Some(b"")), None);

    let base = replace_base();
    for durability in [Durability::Sync, Durability::DELAYED] {
        let mut outcomes = [0u64; 2];
        for_each_cut(
            &base,
            durability,
            |adapter| {
                adapter
                    .replace(None, b"new", None, b"old", timestamp(10))
                    .unwrap();
                adapter.flush().unwrap();
            },
            |context, cut, length, survivor| {
                let source = get(survivor, b"new");
                let target = get(survivor, b"old");
                let replaced = replace_state(source.as_deref(), target.as_deref())
                    .unwrap_or_else(|| panic!("{context}: source {source:?} target {target:?}"));
                assert_eq!(get(survivor, b"kept").unwrap(), b"KEPT", "{context}");
                // Everything issued, flush included: the replacement the
                // caller was told is on disk is there.
                if cut == length {
                    assert!(replaced, "{context}");
                }
                outcomes[usize::from(replaced)] += 1;
                // The survivor finishes or repeats the replacement.
                if !replaced {
                    survivor
                        .replace(None, b"new", None, b"old", timestamp(11))
                        .unwrap();
                }
                assert_eq!(get(survivor, b"old").unwrap(), b"NEW", "{context}");
                assert_eq!(get(survivor, b"new"), None, "{context}");
            },
        );
        assert!(outcomes.iter().all(|count| *count > 0), "{outcomes:?}");
        eprintln!(
            "replace {durability:?}: old={} new={}",
            outcomes[0], outcomes[1]
        );
    }
}

/// The aside name a caller without `ReplaceObject` moves the old file to.
const ASIDE: &[u8] = b".pkgold1a2b3c4d";

/// What a remount may show after a cut during the three-step route: the
/// old contents are under the name or under the aside until the new ones are
/// under the name, and the staged file is gone exactly when it is published.
fn fallback_state(
    staged: Option<&[u8]>,
    target: Option<&[u8]>,
    aside: Option<&[u8]>,
) -> Option<u8> {
    match (staged, target, aside) {
        (Some(b"NEW"), Some(b"OLD"), None) => Some(0),
        (Some(b"NEW"), None, Some(b"OLD")) => Some(1),
        (None, Some(b"NEW"), Some(b"OLD")) => Some(2),
        (None, Some(b"NEW"), None) => Some(3),
        _ => None,
    }
}

#[test]
fn the_three_step_route_keeps_the_old_contents_reachable_at_every_cut() {
    assert_eq!(fallback_state(Some(b"NEW"), None, None), None);
    assert_eq!(fallback_state(None, None, Some(b"OLD")), None);
    assert_eq!(fallback_state(None, Some(b"OLD"), None), None);
    assert_eq!(fallback_state(Some(b"NEW"), Some(b"NEW"), None), None);

    let base = replace_base();
    for durability in [Durability::Sync, Durability::DELAYED] {
        let mut outcomes = [0u64; 4];
        for_each_cut(
            &base,
            durability,
            |adapter| {
                adapter
                    .rename(None, b"old", None, ASIDE, timestamp(10))
                    .unwrap();
                adapter
                    .rename(None, b"new", None, b"old", timestamp(11))
                    .unwrap();
                adapter.delete_object(None, ASIDE, timestamp(12)).unwrap();
                assert_eq!(names(adapter), ["kept", "old"]);
                adapter.flush().unwrap();
            },
            |context, cut, length, survivor| {
                let staged = get(survivor, b"new");
                let target = get(survivor, b"old");
                let aside = get(survivor, ASIDE);
                let step = fallback_state(staged.as_deref(), target.as_deref(), aside.as_deref())
                    .unwrap_or_else(|| {
                        panic!("{context}: staged {staged:?} target {target:?} aside {aside:?}")
                    });
                if cut == length {
                    assert_eq!(step, 3, "{context}");
                }
                outcomes[usize::from(step)] += 1;
            },
        );
        // A delayed window publishes the three steps together, so only the
        // first and the last state exist there.
        match durability {
            Durability::Sync => assert!(outcomes.iter().all(|count| *count > 0), "{outcomes:?}"),
            Durability::Delayed { .. } => {
                assert!(outcomes[0] > 0 && outcomes[3] > 0, "{outcomes:?}");
            }
        }
        eprintln!("fallback {durability:?}: {outcomes:?}");
    }
}

#[test]
fn a_delete_protected_object_is_not_deleted() {
    let mut adapter = adapter(formatted());
    put(&mut adapter, b"file", b"DATA");
    let lock = adapter.locate(None, b"file", LockAccess::Shared).unwrap();
    adapter
        .make_hard_link(None, b"alias", lock, timestamp(3))
        .unwrap();
    adapter.free_lock(lock).unwrap();
    let directory = adapter
        .create_directory(None, b"dir", timestamp(3))
        .unwrap();
    adapter.free_lock(directory).unwrap();
    adapter
        .make_soft_link(None, b"link", b"file", timestamp(3))
        .unwrap();
    // The other three RWED bits forbid nothing here.
    for name in [&b"file"[..], b"dir"] {
        adapter
            .set_protection(None, name, 0b1110, timestamp(4))
            .unwrap();
    }
    adapter.delete_object(None, b"dir", timestamp(5)).unwrap();
    let directory = adapter
        .create_directory(None, b"dir", timestamp(5))
        .unwrap();
    adapter.free_lock(directory).unwrap();

    for name in [&b"file"[..], b"dir"] {
        adapter
            .set_protection(None, name, DELETE_FORBIDDEN, timestamp(6))
            .unwrap();
    }
    // The bit belongs to the object, so both of its names refuse, and it
    // is reported before a holder is.
    let held = adapter.locate(None, b"file", LockAccess::Shared).unwrap();
    for name in [&b"file"[..], b"alias", b"dir"] {
        assert_eq!(
            adapter.delete_object(None, name, timestamp(7)),
            Err(ArosError::DeleteProtected)
        );
    }
    adapter.free_lock(held).unwrap();
    // The soft link is its own object and carries no such bit.
    adapter.delete_object(None, b"link", timestamp(7)).unwrap();
    assert_eq!(names(&mut adapter), ["alias", "dir", "file"]);
    assert_eq!(get(&mut adapter, b"alias").unwrap(), b"DATA");

    for name in [&b"file"[..], b"dir"] {
        adapter.set_protection(None, name, 0, timestamp(8)).unwrap();
    }
    for name in [&b"file"[..], b"alias", b"dir"] {
        adapter.delete_object(None, name, timestamp(9)).unwrap();
    }
    assert!(names(&mut adapter).is_empty());
    let mut device = device_of(adapter);
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
}
