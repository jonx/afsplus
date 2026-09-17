//! Classic single-user security preservation: a protection write through the
//! DOS projection never replaces security metadata the projection cannot
//! express, unless the mount requests the downgrade.
//!
//! Metadata the container does not hold is supplied by a probe double, where
//! refusal is the only non-destructive answer. An on-disk descriptor is the
//! other case: the handler preserves its bytes, lets the classic write land
//! and records the divergence, unless the mount asks for the strict refusal.

use std::collections::BTreeSet;

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, LockAccess, OpenMode, RichSecurityProbe};
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{
    mkfs, mkfs_with_security_descriptors, mount, MkfsParams, MountOptions, NamePolicy,
};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{ObjectId, Vfs};

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn params() -> MkfsParams {
    MkfsParams {
        uuid: [0xC4; 16],
        label: "Classic".into(),
        region_size: 4096,
        reclaim_caps: Default::default(),
        log_slots: 8,
        shared_extents: true,
        data_policy: false,
        name_policy: NamePolicy::Insensitive,
        timestamp: timestamp(0),
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(&mut device, &params()).unwrap();
    device
}

/// A format identity no implementation in this repository evaluates.
const UNKNOWN_FORMAT: u32 = 0x7fff_0055;
const DESCRIPTOR: &[u8] = b"opaque security descriptor";

/// One protection write through the DOS projection on an object that carries
/// an on-disk descriptor. Returns the result, and the stored protection bits
/// and the divergence mark as a fresh mount reads them.
fn descriptor_scenario(strict_security_projection: bool) -> (Result<(), ArosError>, u32, bool) {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs_with_security_descriptors(&mut device, &params()).unwrap();
    let mut volume = mount(device).unwrap();
    let file = volume
        .create_file_in_directory(OBJECT_ROOT, "guarded", b"x", timestamp(10))
        .unwrap();
    volume
        .set_object_protection(file, 0x0F, timestamp(11))
        .unwrap();
    volume
        .set_security_descriptor(file, UNKNOWN_FORMAT, 3, DESCRIPTOR, timestamp(12))
        .unwrap();
    let device = volume.into_device();

    let mut adapter = ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig {
            strict_security_projection,
            ..ArosConfig::default()
        },
    );
    let write = adapter.set_protection(None, b"guarded", 0x33, timestamp(20));

    let mut device = adapter.into_vfs().unwrap().into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut volume = mount(device).unwrap();
    let descriptor = volume.security_descriptor(file).unwrap().unwrap();
    // No path weakens the bytes, whichever policy is in force.
    assert_eq!(descriptor.bytes, DESCRIPTOR);
    assert_eq!((descriptor.format, descriptor.version), (UNKNOWN_FORMAT, 3));
    let protection = volume.stat(file).unwrap().unwrap().protection;
    (write, protection, descriptor.projection_diverged)
}

#[test]
fn the_handler_preserves_a_descriptor_by_default_and_refuses_the_edit_when_strict() {
    // Default: the classic write lands, every descriptor byte stays, and the
    // divergence mark survives the remount.
    let (applied, protection, diverged) = descriptor_scenario(false);
    assert_eq!(applied, Ok(()));
    assert_eq!(protection, 0x33);
    assert!(diverged);

    // The strict mount option: the core refuses the projection and nothing
    // changes, mapped as before through VfsError::NotSupported.
    let (refused, protection, diverged) = descriptor_scenario(true);
    assert_eq!(refused, Err(ArosError::ActionNotKnown));
    assert_eq!(protection, 0x0F);
    assert!(!diverged);
}

struct Guarded(BTreeSet<ObjectId>);

impl RichSecurityProbe for Guarded {
    fn carries_rich_security(&mut self, object_id: ObjectId) -> Result<bool, ArosError> {
        Ok(self.0.contains(&object_id))
    }
}

fn create(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8]) -> ObjectId {
    let file = adapter
        .open(None, name, OpenMode::NewFile, timestamp(10))
        .unwrap();
    let object = adapter.examine_file(file).unwrap().object_id;
    adapter.close(file).unwrap();
    object
}

fn protection(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8]) -> u32 {
    let lock = adapter.locate(None, name, LockAccess::Shared).unwrap();
    let value = adapter.examine_lock(lock).unwrap().protection;
    adapter.free_lock(lock).unwrap();
    value
}

fn scenario(allow_security_downgrade: bool) -> (Result<(), ArosError>, u32, u32) {
    let vfs = Vfs::mount(formatted(), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            allow_security_downgrade,
            ..ArosConfig::default()
        },
    );
    let guarded = create(&mut adapter, b"guarded");
    create(&mut adapter, b"plain");
    adapter.set_security_probe(Box::new(Guarded(BTreeSet::from([guarded]))));

    let guarded_write = adapter.set_protection(None, b"guarded", 0x0F, timestamp(20));
    adapter
        .set_protection(None, b"plain", 0x0F, timestamp(21))
        .unwrap();
    // Rename keeps the object, so the guard follows the new name.
    adapter
        .rename(None, b"guarded", None, b"moved", timestamp(22))
        .unwrap();
    let moved_write = adapter.set_protection(None, b"moved", 0x33, timestamp(23));
    assert_eq!(moved_write.is_ok(), allow_security_downgrade);

    let mut device = adapter.into_vfs().unwrap().into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut adapter = ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig::default(),
    );
    (
        guarded_write,
        protection(&mut adapter, b"moved"),
        protection(&mut adapter, b"plain"),
    )
}

#[test]
fn classic_protection_write_never_replaces_richer_security_silently() {
    // Preservation: refused with ERROR_WRITE_PROTECTED, stored bits untouched,
    // while the unguarded sibling takes the same write.
    let (refused, guarded_bits, plain_bits) = scenario(false);
    assert_eq!(refused, Err(ArosError::WriteProtected));
    assert_eq!(ArosError::WriteProtected.io_error(), 223);
    assert_eq!(guarded_bits, 0);
    assert_eq!(plain_bits, 0x0F);

    // Control: the explicit downgrade mount performs the identical writes, so
    // the refusal above is the policy and not an unrelated failure.
    let (accepted, guarded_bits, plain_bits) = scenario(true);
    assert_eq!(accepted, Ok(()));
    assert_eq!(guarded_bits, 0x33);
    assert_eq!(plain_bits, 0x0F);
}
