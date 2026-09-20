//! What one operation is allowed to allocate.
//!
//! On AROS every allocation the library makes is an exec `AllocMem` and a
//! `FreeMem`, so the count per operation is a resource the handler spends,
//! not an implementation detail. Lot J took a create from 1,310 allocations
//! to 175, a rename from 1,711 to 212 and a delete from 2,312 to 599; the
//! bounds below are those numbers with a tenth on top, so that a change
//! which puts a per-item allocation back on the descent fails here.
//!
//! The counters live behind the `heap-profile` build:
//!
//! ```text
//! cargo test -q -p afsplus-aros-ffi --features heap-profile --test heap_allocation_bounds
//! ```
#![cfg(feature = "heap-profile")]

mod common;

use afsplus_aros_ffi::heap_profile;
use afsplus_aros_ffi::*;
use afsplus_block::MemoryBackend;

const BLOCK_SIZE: usize = 4096;
const OPERATIONS: u64 = 512;

/// Allocations per operation, at the numbers of 2026-09-20 plus a tenth.
const CREATE_BOUND: f64 = 192.0;
const LOOKUP_BOUND: f64 = 14.0;
const RENAME_BOUND: f64 = 234.0;
const DELETE_BOUND: f64 = 660.0;

/// The same materialized disk the memory harness uses: on a sparse one the
/// meter would count the disk filling up as the library's own memory.
fn disk(total_blocks: u64) -> MemoryBackend {
    let mut device = MemoryBackend::new(BLOCK_SIZE, total_blocks);
    let zeros = [0u8; BLOCK_SIZE];
    for lba in 0..total_blocks {
        device.apply_raw(lba, &zeros);
    }
    common::format(device, true)
}

#[test]
fn an_operation_stays_within_its_allocation_bound() {
    let mut device = disk(32768);
    let filesystem = common::mount_sized(&mut device, 32768);
    let mut granted = 0;
    assert_eq!(
        afsplus_aros_set_cache_blocks(filesystem, 64, &mut granted),
        0
    );
    let (seconds, nanoseconds) = (1_700_000_000i64, 0u32);

    // Each phase is measured between two commits, so the window commit of
    // the phase before it is not charged to this one.
    let phase = |label: &str, bound: f64, body: &mut dyn FnMut()| {
        assert_eq!(afsplus_aros_flush(filesystem), 0);
        heap_profile::reset();
        body();
        assert_eq!(afsplus_aros_flush(filesystem), 0);
        let per = heap_profile::sample().allocations as f64 / OPERATIONS as f64;
        println!("{label}: {per:.1} allocations per operation, bound {bound:.1}");
        assert!(
            per <= bound,
            "a {label} allocates {per:.1} times, above the bound of {bound:.1}"
        );
    };

    phase("create", CREATE_BOUND, &mut || {
        for index in 0..OPERATIONS {
            let name = format!("c{index:04}");
            let mut file = 0;
            assert_eq!(
                afsplus_aros_open(
                    filesystem,
                    0,
                    name.as_ptr(),
                    name.len() as u32,
                    AFSPLUS_AROS_OPEN_NEW_FILE,
                    seconds,
                    nanoseconds,
                    &mut file
                ),
                0
            );
            assert_eq!(afsplus_aros_close(filesystem, file), 0);
        }
    });

    phase("lookup", LOOKUP_BOUND, &mut || {
        for index in 0..OPERATIONS {
            let name = format!("c{index:04}");
            let mut lock = 0;
            assert_eq!(
                afsplus_aros_locate(
                    filesystem,
                    0,
                    name.as_ptr(),
                    name.len() as u32,
                    AFSPLUS_AROS_LOCK_SHARED,
                    &mut lock
                ),
                0
            );
            assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
        }
    });

    phase("rename", RENAME_BOUND, &mut || {
        for index in 0..OPERATIONS {
            let from = format!("c{index:04}");
            let to = format!("r{index:04}");
            assert_eq!(
                afsplus_aros_rename(
                    filesystem,
                    0,
                    from.as_ptr(),
                    from.len() as u32,
                    0,
                    to.as_ptr(),
                    to.len() as u32,
                    seconds,
                    nanoseconds
                ),
                0
            );
        }
    });

    phase("delete", DELETE_BOUND, &mut || {
        for index in 0..OPERATIONS {
            let name = format!("r{index:04}");
            assert_eq!(
                afsplus_aros_delete_object(
                    filesystem,
                    0,
                    name.as_ptr(),
                    name.len() as u32,
                    seconds,
                    nanoseconds
                ),
                0
            );
        }
    });

    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}
