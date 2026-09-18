//! The object's comment and protection word as host attributes (ADR-120):
//! what a host reads and writes, what AROS then sees, and the values that
//! are refused without changing anything.

use afsplus_aros::{ArosAdapter, ArosConfig, LockAccess};
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_fuse::{
    parse_protection, AttributeWriteMode, FuseAdapter, FuseConfig, HostAttributeNames,
};
use afsplus_vfs::{AccessMode, Vfs, VfsError};

const UPSERT: AttributeWriteMode = AttributeWriteMode::Upsert;

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
            uuid: [0xFB; 16],
            label: "ObjectFields".into(),
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

fn adapter(
    vfs: Vfs<MemoryBackend>,
    names: HostAttributeNames,
    empty_value_removes: bool,
) -> FuseAdapter<MemoryBackend> {
    FuseAdapter::new(
        vfs,
        FuseConfig {
            attribute_names: names,
            empty_value_removes,
            ..FuseConfig::default()
        },
    )
}

fn with_file(names: HostAttributeNames) -> (FuseAdapter<MemoryBackend>, u64) {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut host = adapter(vfs, names, false);
    let (attributes, handle) = host
        .create_file(OBJECT_ROOT, b"readme", AccessMode::WriteOnly, None, at(1))
        .unwrap();
    host.close(handle).unwrap();
    (host, attributes.object_id)
}

fn protection(host: &mut FuseAdapter<MemoryBackend>, file: u64, name: &[u8]) -> Vec<u8> {
    host.get_attribute(file, name).unwrap()
}

#[test]
fn a_host_writes_the_comment_and_protection_word_and_aros_sees_them() {
    let (mut mac, file) = with_file(HostAttributeNames::MacOs);
    // A fresh file: the word is always there, the comment is absent.
    assert_eq!(
        mac.list_attributes(file).unwrap(),
        b"afsplus.aros.protection\0"
    );
    assert_eq!(
        mac.get_attribute(file, b"afsplus.aros.comment"),
        Err(VfsError::NotFound)
    );
    let fresh = protection(&mut mac, file, b"afsplus.aros.protection");
    assert!(fresh.starts_with(b"0x") && fresh.len() == 10, "{fresh:?}");

    // Script, pure, archived, and not deletable: bits no POSIX mode carries.
    mac.set_attribute(
        file,
        b"afsplus.aros.protection",
        b"0x00000071",
        UPSERT,
        at(2),
    )
    .unwrap();
    mac.set_attribute(
        file,
        b"afsplus.aros.comment",
        b"Read me first",
        UPSERT,
        at(3),
    )
    .unwrap();
    assert_eq!(
        mac.list_attributes(file).unwrap(),
        b"afsplus.aros.comment\0afsplus.aros.protection\0"
    );
    assert_eq!(
        mac.get_attribute(file, b"afsplus.aros.comment").unwrap(),
        b"Read me first"
    );
    assert_eq!(
        protection(&mut mac, file, b"afsplus.aros.protection"),
        b"0x00000071"
    );

    // What AROS sees after a remount.
    let mut device = mac.into_vfs().into_volume().into_device();
    assert!(check_device(&mut device).is_clean());
    let vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    let mut aros = ArosAdapter::new(vfs, ArosConfig::default());
    let lock = aros.locate(None, b"readme", LockAccess::Shared).unwrap();
    assert_eq!(aros.examine_lock(lock).unwrap().protection, 0x71);
    aros.free_lock(lock).unwrap();
    assert_eq!(aros.comment(None, b"readme", 80).unwrap(), b"Read me first");

    // Linux reads the same fields under its own names, and nothing else.
    let mut device = aros.into_vfs().unwrap().into_volume().into_device();
    assert!(check_device(&mut device).is_clean());
    let vfs = Vfs::mount(device, MountOptions::default()).unwrap();
    let mut linux = adapter(vfs, HostAttributeNames::Linux, false);
    assert_eq!(
        linux.list_attributes(file).unwrap(),
        b"user.afsplus.aros.comment\0user.afsplus.aros.protection\0"
    );
    assert_eq!(
        protection(&mut linux, file, b"user.afsplus.aros.protection"),
        b"0x00000071"
    );
    linux
        .set_attribute(file, b"user.afsplus.aros.comment", b"Lu", UPSERT, at(4))
        .unwrap();
    assert_eq!(
        linux
            .get_attribute(file, b"user.afsplus.aros.comment")
            .unwrap(),
        b"Lu"
    );
    // Removing the comment clears it; the listing then drops it.
    linux
        .remove_attribute(file, b"user.afsplus.aros.comment", at(5))
        .unwrap();
    assert_eq!(
        linux.list_attributes(file).unwrap(),
        b"user.afsplus.aros.protection\0"
    );
}

#[test]
fn refused_field_values_are_invalid_and_change_nothing() {
    let (mut mac, file) = with_file(HostAttributeNames::MacOs);
    mac.set_attribute(
        file,
        b"afsplus.aros.protection",
        b"0x00000071",
        UPSERT,
        at(2),
    )
    .unwrap();
    mac.set_attribute(file, b"afsplus.aros.comment", b"kept", UPSERT, at(3))
        .unwrap();

    for bad in [
        &b""[..],
        b"0x",
        b"zz",
        b"0x1g",
        b"-1",
        b" 0x71",
        b"0x00000071\n",
        b"0x 71",
        // Bits the word does not have.
        b"0x100000000",
        b"0xFFFFFFFFFFFFFFFF",
        // Seventeen digits, even of a value that would fit.
        b"00000000000000071",
    ] {
        assert_eq!(
            mac.set_attribute(file, b"afsplus.aros.protection", bad, UPSERT, at(4)),
            Err(VfsError::Invalid),
            "{bad:?}"
        );
    }
    for bad in [&[0xffu8, 0xfe][..], b"a\0b"] {
        assert_eq!(
            mac.set_attribute(file, b"afsplus.aros.comment", bad, UPSERT, at(4)),
            Err(VfsError::Invalid),
            "{bad:?}"
        );
    }
    assert!(matches!(
        mac.set_attribute(file, b"afsplus.aros.comment", &[b'c'; 4096], UPSERT, at(4)),
        Err(VfsError::Limit(_))
    ));
    // The word always exists: it cannot be removed or created.
    assert_eq!(
        mac.remove_attribute(file, b"afsplus.aros.protection", at(4)),
        Err(VfsError::Invalid)
    );
    assert_eq!(
        mac.set_attribute(
            file,
            b"afsplus.aros.protection",
            b"0x0",
            AttributeWriteMode::Create,
            at(4)
        ),
        Err(VfsError::AlreadyExists)
    );
    // Nothing changed, and no attribute of those names was stored.
    assert_eq!(
        protection(&mut mac, file, b"afsplus.aros.protection"),
        b"0x00000071"
    );
    assert_eq!(
        mac.get_attribute(file, b"afsplus.aros.comment").unwrap(),
        b"kept"
    );
    assert_eq!(
        mac.list_attributes(file).unwrap(),
        b"afsplus.aros.comment\0afsplus.aros.protection\0"
    );

    // The stored names the fields shadow cannot be written from anywhere.
    let mut vfs = mac.into_vfs();
    for reserved in afsplus_vfs::FIELD_ATTRIBUTE_NAMES {
        assert_eq!(
            vfs.set_attributes(file, &[(reserved, Some(&b"x"[..]))], UPSERT, at(5)),
            Err(VfsError::Invalid),
            "{reserved}"
        );
    }
    let mut linux = adapter(vfs, HostAttributeNames::Linux, false);
    assert_eq!(
        linux.set_attribute(file, b"trusted.afsplus.aros.comment", b"x", UPSERT, at(5)),
        Err(VfsError::Invalid)
    );
    let mut aros = ArosAdapter::new(linux.into_vfs(), ArosConfig::default());
    assert!(aros
        .set_attribute(
            None,
            b"readme",
            b"aros.protection",
            Some(b"x"),
            UPSERT,
            at(5)
        )
        .is_err());
}

#[test]
fn an_empty_write_through_macfuse_clears_the_comment_and_not_the_word() {
    let (mac, file) = with_file(HostAttributeNames::MacOs);
    let mut mac = adapter(mac.into_vfs(), HostAttributeNames::MacOs, true);
    mac.set_attribute(file, b"afsplus.aros.comment", b"gone soon", UPSERT, at(2))
        .unwrap();
    mac.set_attribute(file, b"afsplus.aros.comment", b"", UPSERT, at(3))
        .unwrap();
    assert_eq!(
        mac.get_attribute(file, b"afsplus.aros.comment"),
        Err(VfsError::NotFound)
    );
    // Already absent is the requested state.
    mac.set_attribute(file, b"afsplus.aros.comment", b"", UPSERT, at(4))
        .unwrap();
    let before = protection(&mut mac, file, b"afsplus.aros.protection");
    assert_eq!(
        mac.set_attribute(file, b"afsplus.aros.protection", b"", UPSERT, at(5)),
        Err(VfsError::Invalid)
    );
    assert_eq!(
        protection(&mut mac, file, b"afsplus.aros.protection"),
        before
    );
}

#[test]
fn the_protection_text_is_hex_with_an_optional_prefix() {
    assert_eq!(parse_protection(b"0x00000071"), Some(0x71));
    assert_eq!(parse_protection(b"0X71"), Some(0x71));
    assert_eq!(parse_protection(b"71"), Some(0x71));
    assert_eq!(parse_protection(b"ffffffff"), Some(u32::MAX));
    assert_eq!(parse_protection(b"0x0000000000000071"), Some(0x71));
    assert_eq!(parse_protection(b"0x0000000100000000"), None);
    assert_eq!(parse_protection(b"+71"), None);
}
