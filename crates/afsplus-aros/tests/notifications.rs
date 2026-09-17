//! DOS notification semantics of the adapter's bounded watch table.

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, LockAccess, OpenMode, WatchId};
use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn adapter(max_watches: usize) -> ArosAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC8; 16],
            label: "Notify".into(),
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
    ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig {
            max_watches,
            ..ArosConfig::default()
        },
    )
}

fn drain(adapter: &mut ArosAdapter<MemoryBackend>) -> Vec<WatchId> {
    let mut ids = [0; 8];
    let count = adapter.drain_watches(&mut ids);
    ids[..count].to_vec()
}

#[test]
fn watches_fire_once_per_drain_for_names_directories_and_future_names() {
    let mut adapter = adapter(8);
    let drawer = adapter
        .create_directory(None, b"Prefs", timestamp(1))
        .unwrap();
    // 1: a file that does not exist yet, spelled in another case.
    let future = adapter.add_watch(Some(drawer), b"SETTINGS").unwrap();
    // 2: the directory itself, through its lock.
    let directory = adapter.add_watch(Some(drawer), b"").unwrap();
    // 3: an unrelated name that must stay silent.
    let silent = adapter.add_watch(None, b"elsewhere").unwrap();
    assert_eq!((future, directory, silent), (1, 2, 3));
    assert_eq!(drain(&mut adapter), Vec::<WatchId>::new());

    // Creation fires the name watch and the containing-directory watch.
    let file = adapter
        .open(Some(drawer), b"settings", OpenMode::NewFile, timestamp(2))
        .unwrap();
    assert_eq!(drain(&mut adapter), vec![1, 2]);
    // Writes coalesce and are reported when the handle closes, as DOS does.
    adapter.write(file, b"a", timestamp(3)).unwrap();
    adapter.write(file, b"b", timestamp(4)).unwrap();
    assert_eq!(drain(&mut adapter), Vec::<WatchId>::new());
    adapter.close(file).unwrap();
    assert_eq!(drain(&mut adapter), vec![1, 2]);
    assert_eq!(drain(&mut adapter), Vec::<WatchId>::new());

    // A read-only open and close changes nothing and fires nothing.
    let reader = adapter
        .open(Some(drawer), b"settings", OpenMode::OldFile, timestamp(5))
        .unwrap();
    adapter.close(reader).unwrap();
    assert_eq!(drain(&mut adapter), Vec::<WatchId>::new());

    // Metadata, rename away and delete each fire; many changes between two
    // drains are one event per watch.
    adapter
        .set_protection(Some(drawer), b"settings", 0x10, timestamp(6))
        .unwrap();
    adapter
        .rename(Some(drawer), b"settings", None, b"moved", timestamp(7))
        .unwrap();
    assert_eq!(drain(&mut adapter), vec![1, 2]);
    adapter.delete_object(None, b"moved", timestamp(8)).unwrap();
    // "moved" lives in the root: neither the old name nor Prefs changed.
    assert_eq!(drain(&mut adapter), Vec::<WatchId>::new());

    // A small drain buffer leaves the rest pending instead of losing it.
    adapter
        .make_soft_link(Some(drawer), b"settings", b"x", timestamp(9))
        .unwrap();
    let mut one = [0; 1];
    assert_eq!(adapter.drain_watches(&mut one), 1);
    assert_eq!(one, [1]);
    assert_eq!(drain(&mut adapter), vec![2]);

    // Removal discards a pending event; an unknown identifier is an error.
    adapter
        .delete_object(Some(drawer), b"settings", timestamp(10))
        .unwrap();
    adapter.remove_watch(future).unwrap();
    assert_eq!(drain(&mut adapter), vec![2]);
    assert_eq!(adapter.remove_watch(future), Err(ArosError::ObjectNotFound));

    // The silent watch never fired: events are matched, not broadcast.
    let file = adapter
        .open(None, b"elsewhere", OpenMode::NewFile, timestamp(11))
        .unwrap();
    adapter.close(file).unwrap();
    assert_eq!(drain(&mut adapter), vec![3]);
    adapter.free_lock(drawer).unwrap();
}

#[test]
fn watch_table_is_bounded() {
    let mut adapter = adapter(2);
    assert_eq!(adapter.add_watch(None, b"a").unwrap(), 1);
    assert_eq!(adapter.add_watch(None, b"b").unwrap(), 2);
    assert_eq!(adapter.add_watch(None, b"c"), Err(ArosError::NoFreeStore));
    adapter.remove_watch(1).unwrap();
    assert_eq!(adapter.add_watch(None, b"c").unwrap(), 3);
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    adapter.remove_watch(2).unwrap();
    assert_eq!(
        adapter.add_watch(None, b"bad/name"),
        Err(ArosError::InvalidComponentName)
    );
    // Root watch: fires for entries created in the root.
    assert_eq!(adapter.add_watch(Some(root), b"").unwrap(), 4);
    adapter
        .create_directory(None, b"new", timestamp(1))
        .map(|lock| adapter.free_lock(lock).unwrap())
        .unwrap();
    let mut ids = [0; 4];
    assert_eq!(adapter.drain_watches(&mut ids), 1);
    assert_eq!(ids[0], 4);
}
