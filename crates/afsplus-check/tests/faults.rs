//! Deterministic fault injection (write/flush errors, as opposed to power
//! cuts): a failed transaction must leave the committed state untouched, and
//! a transient fault must be retryable.

use afsplus_block::{FaultBackend, FaultPlan, MemoryBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::Timespec;

const BS: usize = 4096;

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [42u8; 16],
            label: "FaultVol".into(),
            region_size: 64,
            reclaim_caps: Default::default(),
            log_slots: 8,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: Timespec {
                seconds: 1_780_000_000,
                nanoseconds: 0,
            },
        },
    )
    .unwrap();
    dev
}

fn ts() -> Timespec {
    Timespec {
        seconds: 1_780_000_100,
        nanoseconds: 0,
    }
}

#[test]
fn failed_write_at_every_index_leaves_committed_state_untouched() {
    // An empty-file transaction performs 7 writes (5 metadata + 1 bitmap
    // page + 1 checkpoint) and 2 flushes.
    for write_index in 0..7 {
        let plan = FaultPlan {
            fail_write_index: Some(write_index),
            fail_flush_index: None,
            fail_hard: false,
        };
        let mut vol = mount(FaultBackend::new(formatted(), plan)).unwrap();
        let err = vol.create_file_in_root("hello.txt", b"", ts()).unwrap_err();
        assert!(
            err.to_string().contains("injected"),
            "write {write_index}: expected injected fault, got {err}"
        );
        assert_eq!(
            vol.generation(),
            1,
            "write {write_index}: state must not advance"
        );

        let mut dev = vol.into_device().into_inner();
        let report = check_device(&mut dev);
        assert!(
            report.is_clean(),
            "write {write_index}: {:?}",
            report.errors
        );
        let mut vol = mount(dev).unwrap();
        assert_eq!(vol.generation(), 1);
        assert!(vol.list_root().unwrap().is_empty());
    }
}

#[test]
fn failed_flush_at_each_barrier_leaves_committed_state_untouched() {
    for flush_index in 0..2 {
        let plan = FaultPlan {
            fail_write_index: None,
            fail_flush_index: Some(flush_index),
            fail_hard: false,
        };
        let mut vol = mount(FaultBackend::new(formatted(), plan)).unwrap();
        assert!(vol.create_file_in_root("hello.txt", b"", ts()).is_err());
        assert_eq!(
            vol.generation(),
            1,
            "flush {flush_index}: state must not advance"
        );
    }
}

#[test]
fn transient_fault_is_retryable() {
    let plan = FaultPlan {
        fail_write_index: Some(2),
        fail_flush_index: None,
        fail_hard: false,
    };
    let mut vol = mount(FaultBackend::new(formatted(), plan)).unwrap();
    assert!(vol
        .create_file_in_root("hello.txt", b"payload", ts())
        .is_err());

    // Same volume, same operation: must succeed now and be fully consistent.
    let id = vol
        .create_file_in_root("hello.txt", b"payload", ts())
        .unwrap();
    assert_eq!(vol.generation(), 2);

    let mut dev = vol.into_device().into_inner();
    let report = check_device(&mut dev);
    assert!(report.is_clean(), "{:?}", report.errors);
    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.lookup_root("hello.txt").unwrap(), Some(id));
}
