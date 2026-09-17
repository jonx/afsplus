//! Object-ID operations of the v2 group: lookup by ID, stat by ID and a paged
//! directory enumerator that survives namespace changes between pages.

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, LockAccess, OpenMode, V2Kind};
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

fn adapter(max_enumerators: usize) -> ArosAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xD1; 16],
            label: "ObjectIds".into(),
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
            max_enumerators,
            ..ArosConfig::default()
        },
    )
}

fn create(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8]) {
    let file = adapter
        .open(None, name, OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.close(file).unwrap();
}

fn page_names(
    adapter: &mut ArosAdapter<MemoryBackend>,
    enumerator: u64,
    limit: usize,
) -> (Vec<String>, bool) {
    let page = adapter.read_enumerator(enumerator, limit).unwrap();
    (
        page.entries
            .iter()
            .map(|entry| String::from_utf8(entry.name.clone()).unwrap())
            .collect(),
        page.eof,
    )
}

#[test]
fn enumerator_pages_survive_namespace_changes_between_pages() {
    let mut adapter = adapter(4);
    for name in ["b", "d", "f", "h", "j", "l", "n"] {
        create(&mut adapter, name.as_bytes());
    }
    let root = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    let enumerator = adapter.open_enumerator(Some(root)).unwrap();

    assert_eq!(
        page_names(&mut adapter, enumerator, 3),
        (vec!["b".into(), "d".into(), "f".into()], false)
    );
    // Between pages: delete a returned entry and an upcoming one, create one
    // before the position and one after it, rename the last returned entry.
    adapter.delete_object(None, b"b", timestamp(2)).unwrap();
    adapter.delete_object(None, b"j", timestamp(2)).unwrap();
    create(&mut adapter, b"a");
    create(&mut adapter, b"i");
    adapter
        .rename(None, b"f", None, b"c", timestamp(3))
        .unwrap();
    // Everything ordered after "f" exactly once, nothing before it again.
    assert_eq!(
        page_names(&mut adapter, enumerator, 3),
        (vec!["h".into(), "i".into(), "l".into()], false)
    );
    assert_eq!(
        page_names(&mut adapter, enumerator, 3),
        (vec!["n".into()], true)
    );
    assert_eq!(page_names(&mut adapter, enumerator, 3), (vec![], true));

    // Control: a second enumerator opened now sees the final namespace from
    // the start, so the first one's view above was a continuation.
    let fresh = adapter.open_enumerator(Some(root)).unwrap();
    assert_eq!(
        page_names(&mut adapter, fresh, 64),
        (
            ["a", "c", "d", "h", "i", "l", "n"]
                .iter()
                .map(|name| name.to_string())
                .collect(),
            true
        )
    );

    // The directory itself disappearing fails cleanly.
    let drawer = adapter.create_directory(None, b"zz", timestamp(4)).unwrap();
    let doomed = adapter.open_enumerator(Some(drawer)).unwrap();
    adapter.free_lock(drawer).unwrap();
    adapter.delete_object(None, b"zz", timestamp(5)).unwrap();
    assert_eq!(
        adapter.read_enumerator(doomed, 4).map(|_| ()),
        Err(ArosError::ObjectNotFound)
    );

    // Bounded table, explicit close, dead identifiers.
    let fourth = adapter.open_enumerator(None).unwrap();
    assert_eq!(adapter.open_enumerator(None), Err(ArosError::NoFreeStore));
    adapter.close_enumerator(fourth).unwrap();
    assert_eq!(
        adapter.close_enumerator(fourth),
        Err(ArosError::InvalidLock)
    );
    assert_eq!(
        adapter.read_enumerator(fourth, 1).map(|_| ()),
        Err(ArosError::InvalidLock)
    );
    assert_eq!(
        adapter.read_enumerator(fresh, 0).map(|_| ()),
        Err(ArosError::BadNumber)
    );
    adapter.free_lock(root).unwrap();
}

#[test]
fn lookup_and_stat_by_id_are_stable_across_rename_and_do_not_follow_links() {
    let mut adapter = adapter(4);
    create(&mut adapter, b"note");
    let drawer = adapter
        .create_directory(None, b"drawer", timestamp(2))
        .unwrap();
    adapter
        .make_soft_link(None, b"alias", b"note", timestamp(3))
        .unwrap();

    let note = adapter.lookup_id(None, "NOTE").unwrap();
    let stat = adapter.stat_id(note).unwrap();
    assert_eq!(stat.object_id, note);
    assert_eq!(stat.kind, V2Kind::File);
    assert_eq!((stat.size, stat.links), (0, 1));
    assert_eq!(
        adapter.stat_id(adapter.root_object()).unwrap().kind,
        V2Kind::Directory
    );

    // The ID names the object, not the path.
    adapter
        .rename(None, b"note", Some(drawer), b"moved", timestamp(4))
        .unwrap();
    assert_eq!(adapter.lookup_id(Some(drawer), "moved").unwrap(), note);
    assert_eq!(
        adapter.lookup_id(None, "note"),
        Err(ArosError::ObjectNotFound)
    );
    assert_eq!(adapter.stat_id(note).unwrap().object_id, note);

    // A link is an object of its own; v2 never resolves it.
    let alias = adapter.lookup_id(None, "alias").unwrap();
    assert_ne!(alias, note);
    assert_eq!(adapter.stat_id(alias).unwrap().kind, V2Kind::Symlink);

    // Deleted objects and guessed identifiers are not found.
    adapter
        .delete_object(Some(drawer), b"moved", timestamp(5))
        .unwrap();
    assert_eq!(
        adapter.stat_id(note).map(|_| ()),
        Err(ArosError::ObjectNotFound)
    );
    assert_eq!(
        adapter.stat_id(0xDEAD_BEEF).map(|_| ()),
        Err(ArosError::ObjectNotFound)
    );
    assert_eq!(
        adapter.lookup_id(None, "bad/name"),
        Err(ArosError::InvalidComponentName)
    );
    adapter.free_lock(drawer).unwrap();
}
