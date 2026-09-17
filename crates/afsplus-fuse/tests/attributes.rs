//! Extended attributes at the kernel-free FUSE adapter: the two host naming
//! conventions and the four operations.

use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{AttributeWriteMode, FuseAdapter, FuseConfig, HostAttributeNames};
use afsplus_vfs::{AccessMode, Vfs, VfsError};

fn at(seconds: i64) -> Timespec {
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
            uuid: [0xFA; 16],
            label: "FuseAttributes".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    device
}

fn adapter(device: MemoryBackend, names: HostAttributeNames) -> FuseAdapter<MemoryBackend> {
    FuseAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        FuseConfig {
            attribute_names: names,
            ..FuseConfig::default()
        },
    )
}

fn file(adapter: &mut FuseAdapter<MemoryBackend>) -> u64 {
    let (attributes, handle) = adapter
        .create_file(OBJECT_ROOT, b"note", AccessMode::WriteOnly, None, at(1))
        .unwrap();
    adapter.close(handle).unwrap();
    attributes.object_id
}

#[test]
fn every_stored_name_has_one_host_spelling_and_back() {
    use HostAttributeNames::{Linux, MacOs};
    // (stored, Linux spelling, macOS spelling)
    let table = [
        ("user.kind", "user.kind", "kind"),
        (
            "user.com.apple.quarantine",
            "user.com.apple.quarantine",
            "com.apple.quarantine",
        ),
        (
            "security.selinux",
            "security.selinux",
            "afsplus.security.selinux",
        ),
        (
            "system.backup",
            "trusted.afsplus.system.backup",
            "afsplus.system.backup",
        ),
        (
            "aros.tooltype",
            "trusted.afsplus.aros.tooltype",
            "afsplus.aros.tooltype",
        ),
        // A stored user name that looks like the macOS escape is escaped.
        (
            "user.afsplus.odd",
            "user.afsplus.odd",
            "afsplus.user.afsplus.odd",
        ),
    ];
    for (stored, linux, macos) in table {
        assert_eq!(Linux.host(stored), linux);
        assert_eq!(Linux.stored(linux.as_bytes()).unwrap(), stored);
        assert_eq!(MacOs.host(stored), macos);
        assert_eq!(MacOs.stored(macos.as_bytes()).unwrap(), stored);
    }
    // Names the conventions do not carry: a second spelling, a kernel-owned
    // namespace, a name without one.
    for refused in [
        "system.posix_acl_access",
        "trusted.other",
        "plain",
        "trusted.afsplus.user.x",
    ] {
        assert_eq!(
            Linux.stored(refused.as_bytes()),
            Err(VfsError::NotSupported),
            "{refused}"
        );
    }
    assert_eq!(
        MacOs.stored(b"afsplus.user.kind"),
        Err(VfsError::NotSupported)
    );
    assert_eq!(MacOs.stored(&[0xff, 0xfe]), Err(VfsError::Invalid));
}

#[test]
fn a_transport_without_removexattr_removes_through_an_empty_value() {
    let device = formatted();
    let mut plain = adapter(device, HostAttributeNames::MacOs);
    let note = file(&mut plain);
    // Control: without the workaround an empty value is a value.
    plain
        .set_attribute(note, b"kind", b"", AttributeWriteMode::Upsert, at(2))
        .unwrap();
    assert_eq!(plain.get_attribute(note, b"kind").unwrap(), b"");

    let mut workaround = FuseAdapter::new(
        plain.into_vfs(),
        FuseConfig {
            attribute_names: HostAttributeNames::MacOs,
            empty_value_removes: true,
            ..FuseConfig::default()
        },
    );
    // The stored empty value stays readable; an empty write removes it.
    assert_eq!(workaround.get_attribute(note, b"kind").unwrap(), b"");
    workaround
        .set_attribute(note, b"kind", b"", AttributeWriteMode::Upsert, at(3))
        .unwrap();
    assert_eq!(
        workaround.get_attribute(note, b"kind"),
        Err(VfsError::NotFound)
    );
    // Already absent is the requested state; a missing object is not.
    workaround
        .set_attribute(note, b"kind", b"", AttributeWriteMode::Upsert, at(4))
        .unwrap();
    assert_eq!(
        workaround.set_attribute(note + 1000, b"kind", b"", AttributeWriteMode::Upsert, at(4)),
        Err(VfsError::NotFound)
    );
    workaround
        .set_attribute(note, b"kind", b"note", AttributeWriteMode::Upsert, at(5))
        .unwrap();
    assert_eq!(workaround.get_attribute(note, b"kind").unwrap(), b"note");
}

#[test]
fn macos_names_round_trip_and_are_seen_from_linux() {
    let mut mac = adapter(formatted(), HostAttributeNames::MacOs);
    let note = file(&mut mac);
    assert_eq!(mac.list_attributes(note).unwrap(), b"");
    assert_eq!(
        mac.get_attribute(note, b"com.apple.quarantine"),
        Err(VfsError::NotFound)
    );

    mac.set_attribute(
        note,
        b"com.apple.quarantine",
        b"0081;00000000;Safari;",
        AttributeWriteMode::Create,
        at(2),
    )
    .unwrap();
    mac.set_attribute(
        note,
        b"afsplus.aros.tooltype",
        b"DONOTWAIT",
        AttributeWriteMode::Upsert,
        at(3),
    )
    .unwrap();
    assert_eq!(
        mac.set_attribute(
            note,
            b"com.apple.quarantine",
            b"x",
            AttributeWriteMode::Create,
            at(4),
        ),
        Err(VfsError::AlreadyExists)
    );
    assert_eq!(
        mac.set_attribute(note, b"absent", b"x", AttributeWriteMode::Replace, at(4)),
        Err(VfsError::NotFound)
    );
    assert_eq!(
        mac.get_attribute(note, b"com.apple.quarantine").unwrap(),
        b"0081;00000000;Safari;"
    );
    // Stored byte order: "aros.tooltype" before "user.com.apple.quarantine".
    assert_eq!(
        mac.list_attributes(note).unwrap(),
        b"afsplus.aros.tooltype\0com.apple.quarantine\0"
    );

    // A value past the format's bound is "too big", never "invalid", and
    // the largest value is stored.
    assert!(matches!(
        mac.set_attribute(
            note,
            b"big",
            &vec![7u8; 65_536],
            AttributeWriteMode::Upsert,
            at(5),
        ),
        Err(VfsError::Limit(_))
    ));
    mac.set_attribute(
        note,
        b"big",
        &vec![7u8; 60_000],
        AttributeWriteMode::Upsert,
        at(5),
    )
    .unwrap();
    assert_eq!(mac.get_attribute(note, b"big").unwrap().len(), 60_000);
    mac.remove_attribute(note, b"big", at(6)).unwrap();
    assert_eq!(
        mac.remove_attribute(note, b"big", at(7)),
        Err(VfsError::NotFound)
    );

    let mut device = mac.into_vfs().into_volume().into_device();
    assert!(check_device(&mut device).is_clean());
    let mut linux = adapter(device, HostAttributeNames::Linux);
    let note = linux.lookup(OBJECT_ROOT, b"note").unwrap().object_id;
    assert_eq!(
        linux.list_attributes(note).unwrap(),
        b"trusted.afsplus.aros.tooltype\0user.com.apple.quarantine\0"
    );
    assert_eq!(
        linux
            .get_attribute(note, b"user.com.apple.quarantine")
            .unwrap(),
        b"0081;00000000;Safari;"
    );
    // A name Linux does not carry reads as absent and cannot be written.
    assert_eq!(
        linux.get_attribute(note, b"system.posix_acl_access"),
        Err(VfsError::NotFound)
    );
    assert_eq!(
        linux.set_attribute(
            note,
            b"system.posix_acl_access",
            b"x",
            AttributeWriteMode::Upsert,
            at(8),
        ),
        Err(VfsError::NotSupported)
    );
    assert_eq!(linux.list_attributes(note + 1000), Err(VfsError::NotFound));
}
