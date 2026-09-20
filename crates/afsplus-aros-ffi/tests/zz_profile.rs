//! Profiling harness, ignored by default: the core calls one AROS
//! Rename(), DeleteFile() and Open(NEWFILE)+Write+Close produce, looped for
//! a sampler. PROFILE=rename|delete|create, SECONDS=n.
mod common;
use afsplus_aros_ffi::*;
use common::{formatted, mount};
use std::time::{Duration, Instant};

const DRAWERS: usize = 80;
const FILES: usize = 32;
static mut CLOCK_MS: i64 = 100_000;
/// Two milliseconds a call, as on the target; (seconds, nanoseconds).
fn now() -> (i64, u32) {
    unsafe {
        CLOCK_MS += 2;
        (CLOCK_MS / 1000, (CLOCK_MS % 1000) as u32 * 1_000_000)
    }
}

fn locate(fs: *mut AfsplusAros, base: u64, name: &str) -> u64 {
    let mut lock = 0;
    let s = afsplus_aros_locate(fs, base, name.as_ptr(), name.len() as u32, 0, &mut lock);
    assert_eq!(s, 0, "locate {name}");
    lock
}
/// resolve_path_lock: one locate per component, the previous lock released,
/// or the whole path in one call with LOCATE_PATH=1.
fn resolve(fs: *mut AfsplusAros, parts: &[&str]) -> u64 {
    if std::env::var("LOCATE_PATH").is_ok() {
        let path = parts.join("/");
        let mut lock = 0;
        let s = afsplus_aros_locate_path(fs, 0, path.as_ptr(), path.len() as u32, 0, &mut lock);
        assert_eq!(s, 0, "locate_path {path}");
        return lock;
    }
    let mut current = 0u64;
    for part in parts {
        let next = locate(fs, current, part);
        if current != 0 {
            assert_eq!(afsplus_aros_free_lock(fs, current), 0);
        }
        current = next;
    }
    current
}
/// NameFromLock: DupLock, then Examine and ParentDir up to the root.
fn name_from_lock(fs: *mut AfsplusAros, lock: u64) {
    let mut current = 0;
    assert_eq!(afsplus_aros_duplicate_lock(fs, lock, &mut current), 0);
    let mut info = AfsplusArosFileInfo::default();
    let mut name = [0u8; 108];
    loop {
        assert_eq!(
            afsplus_aros_examine_lock(fs, current, &mut info, name.as_mut_ptr(), 108),
            0
        );
        let mut parent = 0;
        assert_eq!(
            afsplus_aros_parent_lock_with_access(fs, current, 0, &mut parent),
            0
        );
        assert_eq!(afsplus_aros_free_lock(fs, current), 0);
        if parent == 0 {
            break;
        }
        current = parent;
    }
}
fn path(drawer: usize) -> Vec<String> {
    vec![
        "bench".into(),
        format!("t{:02}", drawer / 8),
        format!("d{}", drawer % 8),
    ]
}

fn aros_rename(fs: *mut AfsplusAros, drawer: usize, from: &str, to: &str) {
    let dir = path(drawer);
    let mut parts: Vec<&str> = dir.iter().map(|s| s.as_str()).collect();
    parts.push(from);
    let lock = resolve(fs, &parts);
    name_from_lock(fs, lock);
    assert_eq!(afsplus_aros_free_lock(fs, lock), 0);
    parts.pop();
    let lock = resolve(fs, &parts);
    name_from_lock(fs, lock);
    assert_eq!(afsplus_aros_free_lock(fs, lock), 0);
    let a = resolve(fs, &parts);
    let b = resolve(fs, &parts);
    let t = now();
    assert_eq!(
        afsplus_aros_rename(
            fs,
            a,
            from.as_ptr(),
            from.len() as u32,
            b,
            to.as_ptr(),
            to.len() as u32,
            t.0,
            t.1
        ),
        0
    );
    assert_eq!(afsplus_aros_free_lock(fs, a), 0);
    assert_eq!(afsplus_aros_free_lock(fs, b), 0);
    let mut pending = 0;
    assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
}
fn aros_create(fs: *mut AfsplusAros, drawer: usize, name: &str) {
    let dir = path(drawer);
    let parts: Vec<&str> = dir.iter().map(|s| s.as_str()).collect();
    let d = resolve(fs, &parts);
    let t = now();
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            d,
            name.as_ptr(),
            name.len() as u32,
            AFSPLUS_AROS_OPEN_NEW_FILE,
            t.0,
            t.1,
            &mut file
        ),
        0
    );
    assert_eq!(afsplus_aros_free_lock(fs, d), 0);
    let mut c = 0;
    assert_eq!(
        afsplus_aros_write(fs, file, [0x33u8; 1200].as_ptr(), 1200, t.0, t.1, &mut c),
        0
    );
    if std::env::var("FSYNC_ON_CLOSE").is_ok() {
        assert_eq!(afsplus_aros_fsync(fs, file), 0);
    }
    assert_eq!(afsplus_aros_close(fs, file), 0);
    let mut pending = 0;
    assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
}
fn aros_delete(fs: *mut AfsplusAros, drawer: usize, name: &str) {
    let dir = path(drawer);
    let parts: Vec<&str> = dir.iter().map(|s| s.as_str()).collect();
    let d = resolve(fs, &parts);
    let t = now();
    assert_eq!(
        afsplus_aros_delete_object(fs, d, name.as_ptr(), name.len() as u32, t.0, t.1),
        0
    );
    assert_eq!(afsplus_aros_free_lock(fs, d), 0);
    let mut pending = 0;
    assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
}

#[test]
#[ignore = "profiling harness"]
fn profile() {
    let op = std::env::var("PROFILE").unwrap_or_else(|_| "rename".into());
    let seconds: u64 = std::env::var("SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let mut device = formatted(true);
    let fs = mount(&mut device);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    let mk = |fs, base: u64, name: &str| {
        let mut l = 0;
        assert_eq!(
            afsplus_aros_create_directory(
                fs,
                base,
                name.as_ptr(),
                name.len() as u32,
                now().0,
                0,
                &mut l
            ),
            0
        );
        l
    };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 {
        let tl = mk(fs, bench, &format!("t{t:02}"));
        for d in 0..8 {
            let dl = mk(fs, tl, &format!("d{d}"));
            assert_eq!(afsplus_aros_free_lock(fs, dl), 0);
        }
        assert_eq!(afsplus_aros_free_lock(fs, tl), 0);
    }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    for drawer in 0..DRAWERS {
        for f in 0..FILES {
            aros_create(fs, drawer, &format!("f{f:02}.c"));
        }
    }
    assert_eq!(afsplus_aros_flush(fs), 0);
    eprintln!("READY pid {}", std::process::id());
    std::thread::sleep(Duration::from_secs(2));
    let end = Instant::now() + Duration::from_secs(seconds);
    let (mut ops, start) = (0u64, Instant::now());
    let mut pass = 0usize;
    let c0 = counters(fs);
    while Instant::now() < end {
        for drawer in 0..DRAWERS {
            for f in 0..FILES {
                match op.as_str() {
                    "rename" => {
                        let (a, b) = if pass % 2 == 0 {
                            (".c", ".o")
                        } else {
                            (".o", ".c")
                        };
                        aros_rename(fs, drawer, &format!("f{f:02}{a}"), &format!("f{f:02}{b}"));
                    }
                    _ => {
                        if pass % 2 == 0 {
                            aros_delete(fs, drawer, &format!("f{f:02}.c"));
                        } else {
                            aros_create(fs, drawer, &format!("f{f:02}.c"));
                        }
                    }
                }
                ops += 1;
            }
        }
        pass += 1;
    }
    let c1 = counters(fs);
    eprintln!(
        "DONE {op}: {ops} ops, {:?} per op; per op {:.2} flushes, {:.2} writes, {:.1} calls",
        start.elapsed() / ops as u32,
        (c1.device_flushes - c0.device_flushes) as f64 / ops as f64,
        (c1.device_writes - c0.device_writes) as f64 / ops as f64,
        (c1.calls - c0.calls) as f64 / ops as f64
    );
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

fn counters(fs: *mut AfsplusAros) -> AfsplusArosCounters {
    let mut o = AfsplusArosCounters {
        struct_size: std::mem::size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(fs, &mut o), 0);
    o
}
fn free_blocks(fs: *mut AfsplusAros) -> u64 {
    let mut h = AfsplusArosHealth {
        struct_size: std::mem::size_of::<AfsplusArosHealth>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_health(fs, &mut h), 0);
    h.free_blocks
}
fn orphans(fs: *mut AfsplusAros) -> u64 {
    let mut h = AfsplusArosHealth {
        struct_size: std::mem::size_of::<AfsplusArosHealth>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_health(fs, &mut h), 0);
    h.pending_orphans
}

#[test]
#[ignore = "profiling harness"]
fn delete_phase() {
    let mut device = formatted(true);
    let fs = mount(&mut device);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    if std::env::var("SYNC").is_err() {
        assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    }
    let mk = |fs, base: u64, name: &str| {
        let mut l = 0;
        assert_eq!(
            afsplus_aros_create_directory(
                fs,
                base,
                name.as_ptr(),
                name.len() as u32,
                now().0,
                0,
                &mut l
            ),
            0
        );
        l
    };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 {
        let tl = mk(fs, bench, &format!("t{t:02}"));
        for d in 0..8 {
            let dl = mk(fs, tl, &format!("d{d}"));
            assert_eq!(afsplus_aros_free_lock(fs, dl), 0);
        }
        assert_eq!(afsplus_aros_free_lock(fs, tl), 0);
    }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);
    let empty = free_blocks(fs);
    for drawer in 0..DRAWERS {
        for f in 0..FILES {
            aros_create(fs, drawer, &format!("f{f:02}.c"));
        }
    }
    assert_eq!(afsplus_aros_flush(fs), 0);
    let full = free_blocks(fs);
    if std::env::var("SAMPLE").is_ok() {
        eprintln!("READY pid {}", std::process::id());
        std::thread::sleep(Duration::from_secs(1));
        for _ in 0..6 {
            for drawer in 0..DRAWERS {
                for f in 0..FILES {
                    aros_delete(fs, drawer, &format!("f{f:02}.c"));
                }
            }
            for drawer in 0..DRAWERS {
                for f in 0..FILES {
                    aros_create(fs, drawer, &format!("f{f:02}.c"));
                }
            }
        }
    }
    let c0 = counters(fs);
    let start = Instant::now();
    for drawer in 0..DRAWERS {
        for f in 0..FILES {
            aros_delete(fs, drawer, &format!("f{f:02}.c"));
        }
    }
    let took = start.elapsed();
    let c1 = counters(fs);
    let after_deletes = free_blocks(fs);
    let orphans_left = orphans(fs);
    let t = Instant::now();
    assert_eq!(afsplus_aros_flush(fs), 0);
    let flush_took = t.elapsed();
    let after_flush = free_blocks(fs);
    let n = (DRAWERS * FILES) as u64;
    eprintln!("deletes: {n} in {took:?}, {:?} each; per delete: {:.2} flushes, {:.1} writes, {:.1} calls, {:.1} cache reads",
        took / n as u32, (c1.device_flushes - c0.device_flushes) as f64 / n as f64, (c1.device_writes - c0.device_writes) as f64 / n as f64,
        (c1.calls - c0.calls) as f64 / n as f64, (c1.cache_hits + c1.cache_misses - c0.cache_hits - c0.cache_misses) as f64 / n as f64);
    eprintln!("free blocks: empty {empty}, with files {full} (files use {}), after deletes {after_deletes} ({} returned, {orphans_left} orphans pending), after flush {after_flush} ({} returned) in {flush_took:?}",
        empty - full, after_deletes - full, after_flush - full);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

#[test]
#[ignore = "profiling harness"]
fn create_phase() {
    let mut device = formatted(true);
    let fs = mount(&mut device);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    if std::env::var("SYNC").is_err() {
        assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    }
    let mk = |fs, base: u64, name: &str| {
        let mut l = 0;
        assert_eq!(
            afsplus_aros_create_directory(
                fs,
                base,
                name.as_ptr(),
                name.len() as u32,
                now().0,
                0,
                &mut l
            ),
            0
        );
        l
    };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 {
        let tl = mk(fs, bench, &format!("t{t:02}"));
        for d in 0..8 {
            let dl = mk(fs, tl, &format!("d{d}"));
            assert_eq!(afsplus_aros_free_lock(fs, dl), 0);
        }
        assert_eq!(afsplus_aros_free_lock(fs, tl), 0);
    }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);
    let c0 = counters(fs);
    let start = Instant::now();
    for drawer in 0..DRAWERS {
        for f in 0..FILES {
            aros_create(fs, drawer, &format!("f{f:02}.c"));
        }
    }
    let took = start.elapsed();
    let c1 = counters(fs);
    let n = (DRAWERS * FILES) as u64;
    eprintln!("creates: {n} in {took:?}, {:?} each; per create: {:.2} flushes, {:.2} writes, {:.1} calls, {:.1} cache reads ({:.2} misses)",
        took / n as u32, (c1.device_flushes - c0.device_flushes) as f64 / n as f64, (c1.device_writes - c0.device_writes) as f64 / n as f64,
        (c1.calls - c0.calls) as f64 / n as f64, (c1.cache_hits + c1.cache_misses - c0.cache_hits - c0.cache_misses) as f64 / n as f64,
        (c1.cache_misses - c0.cache_misses) as f64 / n as f64);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

/// Lot I: what a drawer costs. 90 CreateDirs on a delayed mount, each with
/// 32 files in it, through the C boundary. Before the drawer joined the
/// window a CreateDir committed the window and then a transaction of its
/// own; the numbers to watch are the flushes and writes per drawer, with the
/// files of that drawer subtracted by the second reading.
#[test]
#[ignore = "profiling harness"]
fn mkdir_phase() {
    use afsplus_block::MemoryBackend;
    const DIRS: usize = 90;
    let mut device = common::format(MemoryBackend::new(4096, 131072), true);
    let fs = common::mount_sized(&mut device, 131072);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    if std::env::var("SYNC").is_err() { assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0); }
    let mk = |fs, base: u64, name: &str| { let mut l = 0; assert_eq!(afsplus_aros_create_directory(fs, base, name.as_ptr(), name.len() as u32, now().0, 0, &mut l), 0); l };
    let file = |fs, dir: u64, name: &str| {
        let t = now();
        let mut handle = 0;
        assert_eq!(afsplus_aros_open(fs, dir, name.as_ptr(), name.len() as u32, AFSPLUS_AROS_OPEN_NEW_FILE, t.0, t.1, &mut handle), 0);
        let mut c = 0;
        assert_eq!(afsplus_aros_write(fs, handle, [0x33u8; 1200].as_ptr(), 1200, t.0, t.1, &mut c), 0);
        assert_eq!(afsplus_aros_close(fs, handle), 0);
        let mut pending = 0; assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
    };
    let bench = mk(fs, 0, "bench");
    assert_eq!(afsplus_aros_flush(fs), 0);

    // The drawers alone, so the per-drawer cost is not mixed with the files.
    let c0 = counters(fs);
    let start = Instant::now();
    for d in 0..DIRS { let l = mk(fs, bench, &format!("e{d:02}")); assert_eq!(afsplus_aros_free_lock(fs, l), 0); }
    let bare = start.elapsed();
    let c1 = counters(fs);
    let n = DIRS as u64;
    eprintln!("empty drawers: {DIRS} in {bare:?}, {:?} each; per drawer {:.2} flushes, {:.2} writes, {:.1} calls",
        bare / n as u32, (c1.device_flushes - c0.device_flushes) as f64 / n as f64,
        (c1.device_writes - c0.device_writes) as f64 / n as f64, (c1.calls - c0.calls) as f64 / n as f64);

    // The bench phase itself: a drawer and the 32 files that go in it.
    assert_eq!(afsplus_aros_flush(fs), 0);
    let c2 = counters(fs);
    let start = Instant::now();
    for d in 0..DIRS {
        let l = mk(fs, bench, &format!("d{d:02}"));
        for f in 0..FILES { file(fs, l, &format!("f{f:02}.c")); }
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    }
    let took = start.elapsed();
    let c3 = counters(fs);
    let ops = (DIRS + DIRS * FILES) as u64;
    eprintln!("drawers with files: {DIRS} drawers of {FILES} files ({ops} operations) in {took:?}; \
        per drawer {:.2} flushes, {:.1} writes; per operation {:.3} flushes, {:.2} writes",
        (c3.device_flushes - c2.device_flushes) as f64 / n as f64, (c3.device_writes - c2.device_writes) as f64 / n as f64,
        (c3.device_flushes - c2.device_flushes) as f64 / ops as f64, (c3.device_writes - c2.device_writes) as f64 / ops as f64);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

#[test]
#[ignore = "profiling harness"]
fn create_only() {
    use afsplus_block::MemoryBackend;
    let seconds: u64 = std::env::var("SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let mut device = common::format(MemoryBackend::new(4096, 131072), true);
    let fs = common::mount_sized(&mut device, 131072);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    if std::env::var("SYNC").is_err() {
        assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    }
    let mk = |fs, base: u64, name: &str| {
        let mut l = 0;
        assert_eq!(
            afsplus_aros_create_directory(
                fs,
                base,
                name.as_ptr(),
                name.len() as u32,
                now().0,
                0,
                &mut l
            ),
            0
        );
        l
    };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 {
        let tl = mk(fs, bench, &format!("t{t:02}"));
        for d in 0..8 {
            let dl = mk(fs, tl, &format!("d{d}"));
            assert_eq!(afsplus_aros_free_lock(fs, dl), 0);
        }
        assert_eq!(afsplus_aros_free_lock(fs, tl), 0);
    }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);
    eprintln!("READY pid {}", std::process::id());
    std::thread::sleep(Duration::from_secs(2));
    let c0 = counters(fs);
    let end = Instant::now() + Duration::from_secs(seconds);
    let (mut ops, start) = (0u64, Instant::now());
    let mut pass = 0usize;
    while Instant::now() < end && ops < 15000 {
        for drawer in 0..DRAWERS {
            for f in 0..FILES {
                aros_create(fs, drawer, &format!("p{pass}f{f:02}.c"));
                ops += 1;
            }
        }
        pass += 1;
    }
    let took = start.elapsed();
    let c1 = counters(fs);
    eprintln!("DONE create-only: {ops} ops, {:?} per op; per create {:.2} flushes, {:.2} writes, {:.1} calls, {:.1} cache reads, {:.3} misses",
        took / ops as u32, (c1.device_flushes - c0.device_flushes) as f64 / ops as f64, (c1.device_writes - c0.device_writes) as f64 / ops as f64,
        (c1.calls - c0.calls) as f64 / ops as f64, (c1.cache_hits + c1.cache_misses - c0.cache_hits - c0.cache_misses) as f64 / ops as f64,
        (c1.cache_misses - c0.cache_misses) as f64 / ops as f64);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

#[test]
#[ignore = "profiling harness"]
fn steady_heap() {
    // The STEADY probe of the DOS gate, at the FFI: rounds of create, write,
    // close and delete; the heap read after a flush, twice.
    let mut device = common::materialized(true);
    let fs = mount(&mut device);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    let rounds: usize = std::env::var("ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let fsync = std::env::var("FSYNC_ON_CLOSE").is_ok();
    let mut d0 = 0;
    assert_eq!(
        afsplus_aros_create_directory(fs, 0, b"steady".as_ptr(), 6, now().0, 0, &mut d0),
        0
    );
    assert_eq!(afsplus_aros_free_lock(fs, d0), 0);
    let skip = std::env::var("SKIP").unwrap_or_default();
    let round = |fs| {
        let t = now();
        // As the probe: the drawer is looked up per operation and nothing is
        // held between rounds.
        let mut d = 0;
        assert_eq!(
            afsplus_aros_locate(fs, 0, b"steady".as_ptr(), 6, 0, &mut d),
            0
        );
        let mut file = 0;
        assert_eq!(
            afsplus_aros_open(
                fs,
                d,
                b"note".as_ptr(),
                4,
                AFSPLUS_AROS_OPEN_NEW_FILE,
                t.0,
                t.1,
                &mut file
            ),
            0
        );
        let mut c = 0;
        assert_eq!(
            afsplus_aros_write(fs, file, [7u8; 1200].as_ptr(), 1200, t.0, t.1, &mut c),
            0
        );
        if fsync {
            assert_eq!(afsplus_aros_fsync(fs, file), 0);
        }
        assert_eq!(afsplus_aros_close(fs, file), 0);
        if !skip.contains("read") {
            assert_eq!(
                afsplus_aros_open(
                    fs,
                    d,
                    b"note".as_ptr(),
                    4,
                    AFSPLUS_AROS_OPEN_OLD_FILE,
                    t.0,
                    t.1,
                    &mut file
                ),
                0
            );
            let mut buf = [0u8; 1200];
            let mut got = 0;
            assert_eq!(
                afsplus_aros_read(fs, file, buf.as_mut_ptr(), 1200, &mut got),
                0
            );
            if !skip.contains("record") {
                assert_eq!(afsplus_aros_lock_record(fs, file, 0, 4, 1), 0);
                assert_eq!(afsplus_aros_free_record(fs, file, 0, 4), 0);
            }
            assert_eq!(afsplus_aros_close(fs, file), 0);
        }
        if !skip.contains("examine") {
            let mut l = 0;
            assert_eq!(
                afsplus_aros_locate(fs, d, b"note".as_ptr(), 4, 0, &mut l),
                0
            );
            let mut info = AfsplusArosFileInfo::default();
            let mut name = [0u8; 108];
            assert_eq!(
                afsplus_aros_examine_lock(fs, l, &mut info, name.as_mut_ptr(), 108),
                0
            );
            assert_eq!(afsplus_aros_free_lock(fs, l), 0);
        }
        if !skip.contains("watch") {
            let mut w = 0;
            assert_eq!(
                afsplus_aros_watch_add(fs, d, b"note".as_ptr(), 4, &mut w),
                0
            );
            assert_eq!(afsplus_aros_watch_remove(fs, w), 0);
        }
        if !skip.contains("comment") {
            assert_eq!(
                afsplus_aros_set_comment(
                    fs,
                    d,
                    b"note".as_ptr(),
                    4,
                    b"round".as_ptr(),
                    5,
                    t.0,
                    t.1
                ),
                0
            );
        }
        if !skip.contains("protect") {
            assert_eq!(
                afsplus_aros_set_protection(fs, d, b"note".as_ptr(), 4, 0x10, t.0, t.1),
                0
            );
        }
        assert_eq!(
            afsplus_aros_delete_object(fs, d, b"note".as_ptr(), 4, t.0, t.1),
            0
        );
        assert_eq!(afsplus_aros_free_lock(fs, d), 0);
        let mut pending = 0;
        assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
    };
    let settle = std::env::var("SETTLE").is_ok();
    let warmup: usize = std::env::var("WARMUP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(rounds);
    let reading = |fs| {
        assert_eq!(afsplus_aros_flush(fs), 0);
        if settle {
            for _ in 0..64 {
                if orphans(fs) == 0 {
                    break;
                }
                assert_eq!(afsplus_aros_flush(fs), 0);
            }
        }
        let c = counters(fs);
        (c.heap_bytes, c.heap_peak_bytes, orphans(fs))
    };
    for _ in 0..warmup {
        round(fs);
    }
    let warm = reading(fs);
    for _ in 0..rounds {
        round(fs);
    }
    let done = reading(fs);
    eprintln!("STEADY fsync={fsync} rounds={rounds}: heap {} -> {} ({:+}), peak {} -> {} ({:+}), orphans {} -> {}",
        warm.0, done.0, done.0 as i64 - warm.0 as i64, warm.1, done.1, done.1 as i64 - warm.1 as i64, warm.2, done.2);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

#[test]
#[ignore = "profiling harness"]
fn piecewise_write() {
    // Item 7 of the performance programme: a program that writes a file in
    // pieces, as compilers and editors do, against one that writes it with a
    // single Write. Both go through the C boundary on a delayed mount.
    use afsplus_block::MemoryBackend;
    let files: usize = std::env::var("FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let size: usize = std::env::var("SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(192 * 1024);
    let piece: usize = std::env::var("PIECE").ok().and_then(|v| v.parse().ok()).unwrap_or(8 * 1024);
    let mut device = common::format(MemoryBackend::new(4096, 131072), true);
    let fs = common::mount_sized(&mut device, 131072);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    if std::env::var("SYNC").is_err() { assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0); }
    let mut dir = 0;
    assert_eq!(afsplus_aros_create_directory(fs, 0, b"pieces".as_ptr(), 6, now().0, 0, &mut dir), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);
    let content: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    let one_call = std::env::var("ONE_CALL").is_ok();
    let c0 = counters(fs);
    let start = Instant::now();
    for f in 0..files {
        let name = format!("piece{f:03}.o");
        let t = now();
        let mut file = 0;
        assert_eq!(afsplus_aros_open(fs, dir, name.as_ptr(), name.len() as u32, AFSPLUS_AROS_OPEN_NEW_FILE, t.0, t.1, &mut file), 0);
        let mut at = 0usize;
        while at < size {
            let take = piece.min(size - at);
            let take = if one_call { size } else { take };
            let mut c = 0;
            assert_eq!(afsplus_aros_write(fs, file, content[at..at + take].as_ptr(), take as u32, t.0, t.1, &mut c), 0);
            assert_eq!(c as usize, take);
            at += take;
        }
        assert_eq!(afsplus_aros_close(fs, file), 0);
        let mut pending = 0;
        assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
    }
    let took = start.elapsed();
    assert_eq!(afsplus_aros_flush(fs), 0);
    let c1 = counters(fs);
    // The bytes come back as they were written.
    for f in 0..files {
        let name = format!("piece{f:03}.o");
        let t = now();
        let mut file = 0;
        assert_eq!(afsplus_aros_open(fs, dir, name.as_ptr(), name.len() as u32, AFSPLUS_AROS_OPEN_OLD_FILE, t.0, t.1, &mut file), 0);
        let mut back = vec![0u8; size];
        let mut got = 0;
        assert_eq!(afsplus_aros_read(fs, file, back.as_mut_ptr(), size as u32, &mut got), 0);
        assert_eq!(got as usize, size, "{name} short read");
        assert!(back == content, "{name} came back changed");
        assert_eq!(afsplus_aros_close(fs, file), 0);
    }
    assert_eq!(afsplus_aros_free_lock(fs, dir), 0);
    eprintln!("PIECEWISE {files} files of {size} B in {} B pieces{}: {:?} each, {:.1} writes and {:.2} flushes per file, {:.1} cache reads",
        piece, if one_call { " (ONE_CALL)" } else { "" }, took / files as u32,
        (c1.device_writes - c0.device_writes) as f64 / files as f64,
        (c1.device_flushes - c0.device_flushes) as f64 / files as f64,
        (c1.cache_hits + c1.cache_misses - c0.cache_hits - c0.cache_misses) as f64 / files as f64);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

/// Lot D2: the benchmark's delete phase, on the host. Ten trees of eight
/// drawers of 32 files on a 64 MiB disk, the sizes of `afsplus_bench.c`, then
/// the files, the drawers, the trees and the root drawer removed in that
/// order, `afsplus_aros_commit_due` after every operation as the handler
/// calls it. Every device flush is attributed to the call that caused it, by
/// reading the counters around each call; the checkpoints come from the
/// flight recorder's `CheckpointDurable`, so a line also says how many
/// transactions a call site published.
mod delete_tree {
    pub const TREES: usize = 10;
    pub const DRAWERS_PER_TREE: usize = 8;
    pub const FILES_PER_DRAWER: usize = 32;
    pub const LARGEST: u32 = 16384;

    fn next_random(state: &mut u32) -> u32 {
        let mut value = *state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        *state = value;
        value
    }

    fn file_state(seed: u32, index: u32) -> u32 {
        let state = seed ^ index.wrapping_mul(2_654_435_761) ^ 0x9e37_79b9;
        if state != 0 {
            state
        } else {
            1
        }
    }

    /// `afsplus_bench.c`'s `file_size`: three files in four under 1 KiB, the
    /// fourth up to 16 KiB.
    pub fn file_size(seed: u32, index: u32) -> u32 {
        let mut state = file_state(seed, index);
        let value = next_random(&mut state);
        if (value >> 30) == 0 {
            value % (LARGEST + 1)
        } else {
            value % 1024
        }
    }
}

/// Device flushes, block writes and checkpoints one kind of call spent.
#[derive(Default, Clone, Copy)]
struct Attribution {
    calls: u64,
    flushes: u64,
    writes: u64,
    checkpoints: u64,
}

/// Checkpoints the flight recorder has reported since the last reading. The
/// harness is single-threaded and runs one filesystem.
static CHECKPOINTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `EventKind::CheckpointDurable`.
const CHECKPOINT_DURABLE: u16 = 5;

unsafe extern "C" fn count_checkpoints(_context: *mut std::ffi::c_void, event: *const AfspTraceEvent) {
    // SAFETY: the recorder passes one readable event for the call.
    let event = unsafe { &*event };
    if event.event == CHECKPOINT_DURABLE {
        CHECKPOINTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A reading of everything the table attributes.
fn mark(fs: *mut AfsplusAros) -> (u64, u64, u64) {
    let c = counters(fs);
    (
        c.device_flushes,
        c.device_writes,
        CHECKPOINTS.load(std::sync::atomic::Ordering::Relaxed),
    )
}

fn charge(bucket: &mut Attribution, fs: *mut AfsplusAros, before: (u64, u64, u64)) {
    let after = mark(fs);
    bucket.calls += 1;
    bucket.flushes += after.0 - before.0;
    bucket.writes += after.1 - before.1;
    bucket.checkpoints += after.2 - before.2;
}

#[test]
#[ignore = "profiling harness"]
fn delete_tree_phase() {
    use afsplus_block::MemoryBackend;
    use delete_tree::{DRAWERS_PER_TREE, FILES_PER_DRAWER, LARGEST, TREES};
    // 64 MiB, the benchmark's volume; BLOCKS=n for a smaller one, which is
    // how the post-commit room floor is measured where it is reached.
    let blocks: u64 = std::env::var("BLOCKS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(16_384);
    let seed = 0x4146_5350u32;

    let mut device = common::format(MemoryBackend::new(4096, blocks), true);
    let fs = common::mount_sized(&mut device, blocks);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    if std::env::var("SYNC").is_err() {
        assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    }
    let sink = AfspTraceSink {
        emit: Some(count_checkpoints),
        ctx: std::ptr::null_mut(),
        category_mask: AFSP_TRACE_CHECKPOINT,
    };
    assert_eq!(afsplus_aros_set_trace_sink(fs, &sink), 0);

    let mk = |fs, base: u64, name: &str| {
        let mut l = 0;
        let t = now();
        assert_eq!(
            afsplus_aros_create_directory(fs, base, name.as_ptr(), name.len() as u32, t.0, t.1, &mut l),
            0,
            "create directory {name}"
        );
        l
    };
    let tree_name = |t: usize| format!("t{t:02}");
    let drawer_name = |d: usize| format!("d{d}");
    let file_name = |f: usize| format!("f{f:02}.c");

    // The tree the benchmark builds: one root drawer, ten trees, eight
    // drawers each, 32 files each, the sizes of `afsplus_bench.c`. Only a
    // few locks are held at once, as the benchmark holds none.
    let root = mk(fs, 0, "bench");
    let mut bytes = vec![0x33u8; LARGEST as usize];
    for t in 0..TREES {
        let tl = mk(fs, root, &tree_name(t));
        for d in 0..DRAWERS_PER_TREE {
            let dl = mk(fs, tl, &drawer_name(d));
            for f in 0..FILES_PER_DRAWER {
                let index = ((t * DRAWERS_PER_TREE + d) * FILES_PER_DRAWER + f) as u32;
                let size = delete_tree::file_size(seed, index);
                let name = file_name(f);
                let at = now();
                let mut file = 0;
                assert_eq!(
                    afsplus_aros_open(fs, dl, name.as_ptr(), name.len() as u32, AFSPLUS_AROS_OPEN_NEW_FILE, at.0, at.1, &mut file),
                    0
                );
                if size > 0 {
                    let mut written = 0;
                    assert_eq!(afsplus_aros_write(fs, file, bytes.as_mut_ptr(), size, at.0, at.1, &mut written), 0);
                }
                assert_eq!(afsplus_aros_close(fs, file), 0);
                let mut pending = 0;
                assert_eq!(afsplus_aros_commit_due(fs, at.0, at.1, &mut pending), 0);
            }
            assert_eq!(afsplus_aros_free_lock(fs, dl), 0);
        }
        assert_eq!(afsplus_aros_free_lock(fs, tl), 0);
    }
    assert_eq!(afsplus_aros_flush(fs), 0);
    // Idle maintenance until nothing is left over from the build, so the
    // table measures the delete phase and not the tail of the create phase.
    for _ in 0..8_192 {
        let t = now();
        let mut pending = 0;
        assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
        if pending == 0 {
            break;
        }
    }

    let mut files_bucket = Attribution::default();
    let mut drawers_bucket = Attribution::default();
    let mut trees_bucket = Attribution::default();
    let mut root_bucket = Attribution::default();
    let mut tick_bucket = Attribution::default();
    let start = Instant::now();
    let c0 = counters(fs);
    let checkpoints0 = CHECKPOINTS.load(std::sync::atomic::Ordering::Relaxed);

    let tick = |fs, bucket: &mut Attribution| {
        let t = now();
        let before = mark(fs);
        let mut pending = 0;
        assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
        charge(bucket, fs, before);
    };
    let remove = |fs, parent: u64, name: &str, bucket: &mut Attribution| {
        let t = now();
        let before = mark(fs);
        assert_eq!(
            afsplus_aros_delete_object(fs, parent, name.as_ptr(), name.len() as u32, t.0, t.1),
            0,
            "delete {name}"
        );
        charge(bucket, fs, before);
    };
    for t in 0..TREES {
        let tl = locate(fs, root, &tree_name(t));
        for d in 0..DRAWERS_PER_TREE {
            let dl = locate(fs, tl, &drawer_name(d));
            for f in 0..FILES_PER_DRAWER {
                remove(fs, dl, &file_name(f), &mut files_bucket);
                tick(fs, &mut tick_bucket);
            }
            assert_eq!(afsplus_aros_free_lock(fs, dl), 0);
        }
        assert_eq!(afsplus_aros_free_lock(fs, tl), 0);
    }
    for t in 0..TREES {
        let tl = locate(fs, root, &tree_name(t));
        for d in 0..DRAWERS_PER_TREE {
            remove(fs, tl, &drawer_name(d), &mut drawers_bucket);
            tick(fs, &mut tick_bucket);
        }
        assert_eq!(afsplus_aros_free_lock(fs, tl), 0);
    }
    for t in 0..TREES {
        remove(fs, root, &tree_name(t), &mut trees_bucket);
        tick(fs, &mut tick_bucket);
    }
    assert_eq!(afsplus_aros_free_lock(fs, root), 0);
    remove(fs, 0, "bench", &mut root_bucket);
    tick(fs, &mut tick_bucket);

    let took = start.elapsed();
    let c1 = counters(fs);
    let checkpoints1 = CHECKPOINTS.load(std::sync::atomic::Ordering::Relaxed);
    let drawers = TREES * DRAWERS_PER_TREE;
    let operations = (drawers * FILES_PER_DRAWER + drawers + TREES + 1) as u64;
    eprintln!(
        "DELETE PHASE {operations} operations on {blocks} blocks in {took:?}: {} flushes, {} writes, {} checkpoints",
        c1.device_flushes - c0.device_flushes,
        c1.device_writes - c0.device_writes,
        checkpoints1 - checkpoints0
    );
    eprintln!("{:<16} {:>7} {:>9} {:>9} {:>12} {:>9}", "call site", "calls", "flushes", "writes", "checkpoints", "flush/call");
    for (name, bucket) in [
        ("delete file", files_bucket),
        ("remove drawer", drawers_bucket),
        ("remove tree", trees_bucket),
        ("remove root", root_bucket),
        ("commit_due tick", tick_bucket),
    ] {
        eprintln!(
            "{:<16} {:>7} {:>9} {:>9} {:>12} {:>9.3}",
            name,
            bucket.calls,
            bucket.flushes,
            bucket.writes,
            bucket.checkpoints,
            bucket.flushes as f64 / bucket.calls.max(1) as f64
        );
    }
    assert_eq!(afsplus_aros_set_trace_sink(fs, std::ptr::null()), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

/// Lot D2: what the post-commit room floor costs. A volume filled until the
/// free space is near [`delayed_room_blocks`], then a delete-and-create loop
/// that never goes idle. Every commit of the window then finds the volume
/// below the floor and cleans until it is above it again, one transaction at
/// a time; the numbers to watch are the flushes and checkpoints per
/// operation.
#[test]
#[ignore = "profiling harness"]
fn room_floor_loop() {
    use afsplus_block::MemoryBackend;
    let blocks: u64 = std::env::var("BLOCKS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4_096);
    let rounds: usize = std::env::var("ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    let mut device = common::format(MemoryBackend::new(4096, blocks), true);
    let fs = common::mount_sized(&mut device, blocks);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    let sink = AfspTraceSink {
        emit: Some(count_checkpoints),
        ctx: std::ptr::null_mut(),
        category_mask: AFSP_TRACE_CHECKPOINT,
    };
    assert_eq!(afsplus_aros_set_trace_sink(fs, &sink), 0);

    let mut dir = 0;
    let t = now();
    assert_eq!(afsplus_aros_create_directory(fs, 0, "load".as_ptr(), 4, t.0, t.1, &mut dir), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);

    // Fill until the free space is about the floor the commit keeps: an
    // eighth of the volume before this lot.
    let bytes = [0x33u8; 4096];
    let floor = blocks / 8;
    let mut files = 0usize;
    while free_blocks(fs) > floor + 32 {
        let name = format!("l{files:04}.d");
        let t = now();
        let mut file = 0;
        assert_eq!(afsplus_aros_open(fs, dir, name.as_ptr(), name.len() as u32, AFSPLUS_AROS_OPEN_NEW_FILE, t.0, t.1, &mut file), 0, "fill {name}");
        let mut written = 0;
        assert_eq!(afsplus_aros_write(fs, file, bytes.as_ptr(), 4096, t.0, t.1, &mut written), 0);
        assert_eq!(afsplus_aros_close(fs, file), 0);
        let mut pending = 0;
        assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
        files += 1;
        assert!(files < 100_000, "the volume never filled");
    }
    assert_eq!(afsplus_aros_flush(fs), 0);
    eprintln!("ROOM FLOOR {blocks} blocks, floor {floor}, {files} files, {} free", free_blocks(fs));

    // The loop itself: delete a file and make it again, never idle, so every
    // window commit meets the floor.
    let c0 = counters(fs);
    let checkpoints0 = CHECKPOINTS.load(std::sync::atomic::Ordering::Relaxed);
    let start = Instant::now();
    let mut operations = 0u64;
    for _ in 0..rounds {
        for f in 0..files {
            let name = format!("l{f:04}.d");
            let t = now();
            assert_eq!(afsplus_aros_delete_object(fs, dir, name.as_ptr(), name.len() as u32, t.0, t.1), 0, "delete {name}");
            let mut pending = 0;
            assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
            let t = now();
            let mut file = 0;
            assert_eq!(afsplus_aros_open(fs, dir, name.as_ptr(), name.len() as u32, AFSPLUS_AROS_OPEN_NEW_FILE, t.0, t.1, &mut file), 0, "remake {name}");
            let mut written = 0;
            assert_eq!(afsplus_aros_write(fs, file, bytes.as_ptr(), 4096, t.0, t.1, &mut written), 0);
            assert_eq!(afsplus_aros_close(fs, file), 0);
            let mut pending = 0;
            assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
            operations += 2;
        }
    }
    let took = start.elapsed();
    let c1 = counters(fs);
    let checkpoints1 = CHECKPOINTS.load(std::sync::atomic::Ordering::Relaxed);
    eprintln!(
        "ROOM FLOOR LOOP {operations} operations in {took:?}: {:.3} flushes, {:.2} writes, {:.3} checkpoints per operation",
        (c1.device_flushes - c0.device_flushes) as f64 / operations as f64,
        (c1.device_writes - c0.device_writes) as f64 / operations as f64,
        (checkpoints1 - checkpoints0) as f64 / operations as f64
    );
    assert_eq!(afsplus_aros_set_trace_sink(fs, std::ptr::null()), 0);
    assert_eq!(afsplus_aros_free_lock(fs, dir), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}
