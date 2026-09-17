//! The metadata contract of CloneFile and CloneRange: what the destination
//! inherits from the source, what it receives fresh, and what the source
//! keeps. Every expectation is a literal chosen by the test.
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::volume::DataUpdatePolicy;
use afsplus_core::{
    mkfs_with_security_descriptors, mount, MkfsParams, NamePolicy, SecurityProjectionPolicy, Volume,
};
use afsplus_format::{Timespec, OBJECT_ROOT};

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 42,
    }
}

fn volume() -> Volume<MemoryBackend> {
    let mut dev = MemoryBackend::new(4096, 1024);
    mkfs_with_security_descriptors(
        &mut dev,
        &MkfsParams {
            uuid: [0x14; 16],
            label: "Clone".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    mount(dev).unwrap()
}

fn checked<D: BlockDevice>(volume: Volume<D>) -> D {
    let mut dev = volume.into_device();
    let report = check_device(&mut dev);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    dev
}

/// Source: created at 10, content written at 20, protection 0x5a set at 30,
/// in-place policy at 31, descriptor at 32, second link at 33.
fn source(volume: &mut Volume<MemoryBackend>) -> u64 {
    let id = volume
        .create_file_in_directory(OBJECT_ROOT, "source", &[0x61; 9000], time(10))
        .unwrap();
    volume.write_file_at(id, 0, &[0x62; 100], time(20)).unwrap();
    volume.set_object_protection(id, 0x5a, time(30)).unwrap();
    volume
        .set_file_data_policy(id, DataUpdatePolicy::InPlacePrivate, time(31))
        .unwrap();
    volume
        .set_security_descriptor(id, 0x7fff_0001, 1, b"source descriptor", time(32))
        .unwrap();
    volume
        .link_file(id, OBJECT_ROOT, "alias", time(33))
        .unwrap();
    id
}

#[test]
fn clone_file_inherits_content_time_and_protection_and_nothing_else() {
    let mut volume = volume();
    let source = source(&mut volume);
    let before = volume.stat(source).unwrap().unwrap();
    let clone = volume
        .clone_file(source, OBJECT_ROOT, "clone", time(40))
        .unwrap();

    let record = volume.stat(clone).unwrap().unwrap();
    // Inherited: the content's modification time and the classic protection.
    assert_eq!(record.modified, time(20));
    assert_eq!(record.protection, 0x5a);
    assert_eq!(record.size_bytes, 9000);
    // Fresh: identity, creation and change time, one link.
    assert_ne!(clone, source);
    assert_eq!(record.created, time(40));
    assert_eq!(record.changed, time(40));
    assert_eq!(record.link_count, 1);
    // Never inherited: the data-update policy.
    assert_eq!(
        volume.file_data_policy(clone).unwrap(),
        DataUpdatePolicy::FullCow
    );
    // Copied into blocks of its own: the security descriptor.
    let copied = volume.security_descriptor(clone).unwrap().unwrap();
    assert_eq!(
        (copied.format, copied.version, copied.bytes.as_slice()),
        (0x7fff_0001, 1, &b"source descriptor"[..])
    );
    assert!(!copied.projection_diverged);

    // The source keeps every metadata field, its policy and its descriptor.
    let after = volume.stat(source).unwrap().unwrap();
    assert_eq!(
        (after.created, after.modified, after.changed),
        (time(10), time(20), time(33))
    );
    assert_eq!((after.protection, after.link_count), (0x5a, 2));
    assert_eq!(
        (before.created, before.modified, before.changed),
        (after.created, after.modified, after.changed)
    );
    assert_eq!(
        volume.file_data_policy(source).unwrap(),
        DataUpdatePolicy::InPlacePrivate
    );
    assert_eq!(
        volume.security_descriptor(source).unwrap().unwrap().bytes,
        b"source descriptor"
    );
    // The clone carries a descriptor of its own, so the classic edit the
    // source refuses is refused on it too.
    assert!(volume.set_object_protection(source, 0, time(41)).is_err());
    assert!(volume.set_object_protection(clone, 0, time(41)).is_err());
    let mut volume = mount(checked(volume)).unwrap();
    assert_eq!(volume.stat(clone).unwrap().unwrap().modified, time(20));
    checked(volume);
}

#[test]
fn clone_range_changes_destination_content_time_only() {
    let mut volume = volume();
    let source = source(&mut volume);
    let destination = volume
        .create_file_in_directory(OBJECT_ROOT, "destination", &[0x7a; 8192], time(11))
        .unwrap();
    volume
        .set_object_protection(destination, 0x0f, time(12))
        .unwrap();
    volume
        .set_security_descriptor(destination, 0x7fff_0002, 7, b"destination", time(13))
        .unwrap();
    volume
        .clone_range(source, 0, destination, 4096, 4096, time(50))
        .unwrap();

    let record = volume.stat(destination).unwrap().unwrap();
    // A range clone is a content write to the destination.
    assert_eq!(record.modified, time(50));
    assert_eq!(record.changed, time(50));
    // Everything else stays the destination's own.
    assert_eq!(record.created, time(11));
    assert_eq!(record.protection, 0x0f);
    assert_eq!(record.link_count, 1);
    assert_eq!(
        volume.file_data_policy(destination).unwrap(),
        DataUpdatePolicy::FullCow
    );
    let descriptor = volume.security_descriptor(destination).unwrap().unwrap();
    assert_eq!(
        (
            descriptor.format,
            descriptor.version,
            descriptor.bytes.as_slice()
        ),
        (0x7fff_0002, 7, &b"destination"[..])
    );
    assert!(!descriptor.projection_diverged);
    // The source is read, never modified.
    let after = volume.stat(source).unwrap().unwrap();
    assert_eq!(
        (
            after.created,
            after.modified,
            after.changed,
            after.protection
        ),
        (time(10), time(20), time(33), 0x5a)
    );
    let bytes = volume.read_file(destination).unwrap();
    assert_eq!(&bytes[..4096], &[0x7a; 4096][..]);
    assert_eq!(&bytes[4096..4196], &[0x62; 100][..]);
    assert_eq!(&bytes[4196..8192], &[0x61; 3996][..]);
    volume.set_security_projection_policy(SecurityProjectionPolicy::Strict);
    checked(volume);
}
