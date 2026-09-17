//! Classic single-user security preservation: a protection write through the
//! DOS projection never replaces security metadata the projection cannot
//! express, unless the mount requests the downgrade.
//!
//! The executable format stores protection bits only, so the richer metadata
//! is supplied by a probe double; the on-disk query waits on the security
//! preservation container.

use std::collections::BTreeSet;

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, LockAccess, OpenMode, RichSecurityProbe};
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::{ObjectId, Vfs};

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
            uuid: [0xC4; 16],
            label: "Classic".into(),
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
