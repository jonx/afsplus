//! `ACTION_SET_COMMENT` and the comment an Examine reports, at the adapter.

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, NameEncoding, OpenMode};
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

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
            uuid: [0xC7; 16],
            label: "Comment".into(),
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

fn adapter(device: MemoryBackend, encoding: NameEncoding) -> ArosAdapter<MemoryBackend> {
    ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig {
            name_encoding: encoding,
            ..ArosConfig::default()
        },
    )
}

fn checked_device(adapter: ArosAdapter<MemoryBackend>) -> MemoryBackend {
    let mut device = adapter.into_vfs().unwrap().into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    device
}

fn create(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8]) {
    let file = adapter
        .open(None, name, OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.close(file).unwrap();
}

#[test]
fn comment_persists_is_replaced_and_is_removed() {
    let mut adapter = adapter(formatted(), NameEncoding::Utf8);
    create(&mut adapter, b"note");
    let drawer = adapter
        .create_directory(None, b"drawer", timestamp(2))
        .unwrap();
    assert_eq!(adapter.comment(None, b"note", 79).unwrap(), b"");

    adapter
        .set_comment(None, b"note", b"first draft", timestamp(10))
        .unwrap();
    // An empty name addresses the base lock's own object.
    adapter
        .set_comment(
            Some(drawer),
            b"",
            "tiroir \u{e9}t\u{e9}".as_bytes(),
            timestamp(11),
        )
        .unwrap();
    adapter.free_lock(drawer).unwrap();

    let mut adapter = self::adapter(checked_device(adapter), NameEncoding::Utf8);
    assert_eq!(adapter.comment(None, b"note", 79).unwrap(), b"first draft");
    assert_eq!(
        adapter.comment(None, b"drawer", 79).unwrap(),
        "tiroir \u{e9}t\u{e9}".as_bytes()
    );

    adapter
        .set_comment(None, b"note", b"second", timestamp(20))
        .unwrap();
    assert_eq!(adapter.comment(None, b"note", 79).unwrap(), b"second");
    adapter
        .set_comment(None, b"note", b"", timestamp(21))
        .unwrap();

    let mut adapter = self::adapter(checked_device(adapter), NameEncoding::Utf8);
    assert_eq!(adapter.comment(None, b"note", 79).unwrap(), b"");
    assert_eq!(
        adapter.comment(None, b"drawer", 79).unwrap(),
        "tiroir \u{e9}t\u{e9}".as_bytes()
    );
}

#[test]
fn refused_comments_leave_the_stored_one() {
    let mut adapter = adapter(formatted(), NameEncoding::Utf8);
    create(&mut adapter, b"note");
    adapter
        .set_comment(None, b"note", b"kept", timestamp(10))
        .unwrap();

    // Control: 255 bytes is the largest stored comment and is accepted on a
    // second object; one byte more is refused.
    create(&mut adapter, b"full");
    adapter
        .set_comment(None, b"full", &[b'x'; 255], timestamp(11))
        .unwrap();
    assert_eq!(adapter.comment(None, b"full", 255).unwrap(), [b'x'; 255]);
    assert_eq!(
        adapter.set_comment(None, b"note", &[b'x'; 256], timestamp(12)),
        Err(ArosError::CommentTooBig)
    );
    assert_eq!(
        adapter.set_comment(None, b"note", b"nul\0inside", timestamp(13)),
        Err(ArosError::InvalidComponentName)
    );
    assert_eq!(
        adapter.set_comment(None, b"note", &[0xff, 0xfe], timestamp(14)),
        Err(ArosError::InvalidComponentName)
    );
    assert_eq!(
        adapter.set_comment(None, b"absent", b"x", timestamp(15)),
        Err(ArosError::ObjectNotFound)
    );
    assert_eq!(adapter.comment(None, b"note", 79).unwrap(), b"kept");
    assert_eq!(
        adapter.comment(None, b"absent", 79),
        Err(ArosError::ObjectNotFound)
    );
}

#[test]
fn reading_cuts_at_a_character_and_never_fails_on_content() {
    // Written through the portable interface: longer than a DOS structure
    // holds, with a character Latin-1 lacks.
    let mut vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter_for_create = ArosAdapter::new(vfs, ArosConfig::default());
    create(&mut adapter_for_create, b"note");
    vfs = adapter_for_create.into_vfs().unwrap();
    let root = vfs.root_object();
    let note = vfs.lookup(root, "note").unwrap();
    let text = format!("{}\u{e9}\u{20ac}tail", "a".repeat(77));
    vfs.set_comment(note, &text, timestamp(5)).unwrap();
    let device = vfs.into_volume().into_device();

    // UTF-8 mount: 77 + 2 bytes fit in 79; the euro sign does not.
    let mut utf8 = adapter(device, NameEncoding::Utf8);
    let expected = format!("{}\u{e9}", "a".repeat(77));
    assert_eq!(
        utf8.comment(None, b"note", 79).unwrap(),
        expected.as_bytes()
    );
    // 78 bytes cannot hold half of the two-byte character.
    assert_eq!(utf8.comment(None, b"note", 78).unwrap(), [b'a'; 77]);
    assert_eq!(utf8.comment(None, b"note", 255).unwrap(), text.as_bytes());

    // Latin-1 mount: one byte per character, `?` for the euro sign.
    let mut latin1 = adapter(checked_device(utf8), NameEncoding::Latin1);
    let mut expected = vec![b'a'; 77];
    expected.extend_from_slice(&[0xe9, b'?']);
    assert_eq!(latin1.comment(None, b"note", 79).unwrap(), expected);

    // A Latin-1 comment is stored as UTF-8 and read back byte for byte.
    latin1
        .set_comment(None, b"note", &[b'd', 0xe9, b'j', 0xe0], timestamp(9))
        .unwrap();
    assert_eq!(
        latin1.comment(None, b"note", 79).unwrap(),
        [b'd', 0xe9, b'j', 0xe0]
    );
    let mut vfs = Vfs::mount(checked_device(latin1), MountOptions::default()).unwrap();
    let root = vfs.root_object();
    let note = vfs.lookup(root, "note").unwrap();
    assert_eq!(vfs.comment(note).unwrap(), "d\u{e9}j\u{e0}");
}
