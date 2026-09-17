//! Extended attributes at the AROS adapter: readable in every namespace,
//! writable in the classic system's own two.

use afsplus_aros::{
    ArosAdapter, ArosConfig, ArosError, AttributeWriteMode, NameEncoding, OpenMode,
};
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn mounted(encoding: NameEncoding) -> ArosAdapter<MemoryBackend> {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xA8; 16],
            label: "ArosAttributes".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig {
            name_encoding: encoding,
            ..ArosConfig::default()
        },
    )
}

fn create(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8]) {
    let file = adapter.open(None, name, OpenMode::NewFile, at(1)).unwrap();
    adapter.close(file).unwrap();
}

#[test]
fn classic_namespaces_are_written_and_the_others_only_read() {
    let mut adapter = mounted(NameEncoding::Utf8);
    create(&mut adapter, b"note");
    assert_eq!(adapter.attribute_names(None, b"note").unwrap(), b"");
    assert_eq!(
        adapter.attribute(None, b"note", b"user.kind").unwrap(),
        None
    );

    adapter
        .set_attribute(
            None,
            b"note",
            b"user.kind",
            Some(b"text"),
            AttributeWriteMode::Create,
            at(2),
        )
        .unwrap();
    adapter
        .set_attribute(
            None,
            b"note",
            b"aros.tooltype",
            Some(b"DONOTWAIT"),
            AttributeWriteMode::Upsert,
            at(3),
        )
        .unwrap();
    assert_eq!(
        adapter.set_attribute(
            None,
            b"note",
            b"user.kind",
            Some(b"x"),
            AttributeWriteMode::Create,
            at(4),
        ),
        Err(ArosError::ObjectExists)
    );
    for refused in [&b"security.selinux"[..], b"system.backup"] {
        assert_eq!(
            adapter.set_attribute(
                None,
                b"note",
                refused,
                Some(b"x"),
                AttributeWriteMode::Upsert,
                at(5),
            ),
            Err(ArosError::WriteProtected)
        );
        assert_eq!(
            adapter.set_attribute(
                None,
                b"note",
                refused,
                None,
                AttributeWriteMode::Upsert,
                at(5)
            ),
            Err(ArosError::WriteProtected)
        );
    }
    assert_eq!(
        adapter.set_attribute(
            None,
            b"note",
            b"unregistered.name",
            Some(b"x"),
            AttributeWriteMode::Upsert,
            at(5),
        ),
        Err(ArosError::InvalidComponentName)
    );

    // Another host stored a security attribute: shown, read, not removable.
    let mut vfs = adapter.into_vfs().unwrap();
    let note = vfs.lookup(vfs.root_object(), "note").unwrap();
    vfs.set_attributes(
        note,
        &[("security.selinux", Some(b"ctx".as_slice()))],
        AttributeWriteMode::Upsert,
        at(6),
    )
    .unwrap();
    let mut device = vfs.into_volume().into_device();
    assert!(check_device(&mut device).is_clean());
    let mut adapter = ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig::default(),
    );
    assert_eq!(
        adapter.attribute_names(None, b"note").unwrap(),
        b"aros.tooltype\0security.selinux\0user.kind\0"
    );
    assert_eq!(
        adapter
            .attribute(None, b"note", b"security.selinux")
            .unwrap(),
        Some(b"ctx".to_vec())
    );
    assert_eq!(
        adapter.set_attribute(
            None,
            b"note",
            b"security.selinux",
            None,
            AttributeWriteMode::Upsert,
            at(7),
        ),
        Err(ArosError::WriteProtected)
    );
    adapter
        .set_attribute(
            None,
            b"note",
            b"user.kind",
            None,
            AttributeWriteMode::Upsert,
            at(8),
        )
        .unwrap();
    assert_eq!(
        adapter.set_attribute(
            None,
            b"note",
            b"user.kind",
            None,
            AttributeWriteMode::Upsert,
            at(9)
        ),
        Err(ArosError::ObjectNotFound)
    );
    assert_eq!(
        adapter.attribute(None, b"absent", b"user.kind"),
        Err(ArosError::ObjectNotFound)
    );
}

#[test]
fn a_latin1_mount_spells_what_it_can_and_omits_the_rest() {
    let mut adapter = mounted(NameEncoding::Latin1);
    create(&mut adapter, b"note");
    // "user.caf\u{e9}" arrives as Latin-1 bytes and is stored as UTF-8.
    adapter
        .set_attribute(
            None,
            b"note",
            &[b'u', b's', b'e', b'r', b'.', b'c', b'a', b'f', 0xe9],
            Some(b"1"),
            AttributeWriteMode::Upsert,
            at(2),
        )
        .unwrap();
    let mut vfs = adapter.into_vfs().unwrap();
    let note = vfs.lookup(vfs.root_object(), "note").unwrap();
    assert_eq!(
        vfs.attribute(note, "user.caf\u{e9}").unwrap(),
        Some(b"1".to_vec())
    );
    // A name outside Latin-1, stored by another host.
    vfs.set_attributes(
        note,
        &[("user.\u{20ac}", Some(b"2".as_slice()))],
        AttributeWriteMode::Upsert,
        at(3),
    )
    .unwrap();
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            name_encoding: NameEncoding::Latin1,
            ..ArosConfig::default()
        },
    );
    assert_eq!(
        adapter.attribute_names(None, b"note").unwrap(),
        [b'u', b's', b'e', b'r', b'.', b'c', b'a', b'f', 0xe9, 0]
    );
}
