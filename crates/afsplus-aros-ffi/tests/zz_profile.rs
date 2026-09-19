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
fn now() -> (i64, u32) { unsafe { CLOCK_MS += 2; (CLOCK_MS / 1000, (CLOCK_MS % 1000) as u32 * 1_000_000) } }

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
        if current != 0 { assert_eq!(afsplus_aros_free_lock(fs, current), 0); }
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
        assert_eq!(afsplus_aros_examine_lock(fs, current, &mut info, name.as_mut_ptr(), 108), 0);
        let mut parent = 0;
        assert_eq!(afsplus_aros_parent_lock_with_access(fs, current, 0, &mut parent), 0);
        assert_eq!(afsplus_aros_free_lock(fs, current), 0);
        if parent == 0 { break; }
        current = parent;
    }
}
fn path(drawer: usize) -> Vec<String> { vec!["bench".into(), format!("t{:02}", drawer / 8), format!("d{}", drawer % 8)] }

fn aros_rename(fs: *mut AfsplusAros, drawer: usize, from: &str, to: &str) {
    let dir = path(drawer);
    let mut parts: Vec<&str> = dir.iter().map(|s| s.as_str()).collect();
    parts.push(from);
    let lock = resolve(fs, &parts); name_from_lock(fs, lock); assert_eq!(afsplus_aros_free_lock(fs, lock), 0);
    parts.pop();
    let lock = resolve(fs, &parts); name_from_lock(fs, lock); assert_eq!(afsplus_aros_free_lock(fs, lock), 0);
    let a = resolve(fs, &parts); let b = resolve(fs, &parts);
    let t = now();
    assert_eq!(afsplus_aros_rename(fs, a, from.as_ptr(), from.len() as u32, b, to.as_ptr(), to.len() as u32, t.0, t.1), 0);
    assert_eq!(afsplus_aros_free_lock(fs, a), 0); assert_eq!(afsplus_aros_free_lock(fs, b), 0);
    let mut pending = 0; assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
}
fn aros_create(fs: *mut AfsplusAros, drawer: usize, name: &str) {
    let dir = path(drawer); let parts: Vec<&str> = dir.iter().map(|s| s.as_str()).collect();
    let d = resolve(fs, &parts);
    let t = now();
    let mut file = 0;
    assert_eq!(afsplus_aros_open(fs, d, name.as_ptr(), name.len() as u32, AFSPLUS_AROS_OPEN_NEW_FILE, t.0, t.1, &mut file), 0);
    assert_eq!(afsplus_aros_free_lock(fs, d), 0);
    let mut c = 0;
    assert_eq!(afsplus_aros_write(fs, file, [0x33u8; 1200].as_ptr(), 1200, t.0, t.1, &mut c), 0);
    if std::env::var("FSYNC_ON_CLOSE").is_ok() { assert_eq!(afsplus_aros_fsync(fs, file), 0); }
    assert_eq!(afsplus_aros_close(fs, file), 0);
    let mut pending = 0; assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
}
fn aros_delete(fs: *mut AfsplusAros, drawer: usize, name: &str) {
    let dir = path(drawer); let parts: Vec<&str> = dir.iter().map(|s| s.as_str()).collect();
    let d = resolve(fs, &parts);
    let t = now();
    assert_eq!(afsplus_aros_delete_object(fs, d, name.as_ptr(), name.len() as u32, t.0, t.1), 0);
    assert_eq!(afsplus_aros_free_lock(fs, d), 0);
    let mut pending = 0; assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
}

#[test]
#[ignore = "profiling harness"]
fn profile() {
    let op = std::env::var("PROFILE").unwrap_or_else(|_| "rename".into());
    let seconds: u64 = std::env::var("SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let mut device = formatted(true);
    let fs = mount(&mut device);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    let mk = |fs, base: u64, name: &str| { let mut l = 0; assert_eq!(afsplus_aros_create_directory(fs, base, name.as_ptr(), name.len() as u32, now().0, 0, &mut l), 0); l };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 { let tl = mk(fs, bench, &format!("t{t:02}")); for d in 0..8 { let dl = mk(fs, tl, &format!("d{d}")); assert_eq!(afsplus_aros_free_lock(fs, dl), 0); } assert_eq!(afsplus_aros_free_lock(fs, tl), 0); }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    for drawer in 0..DRAWERS { for f in 0..FILES { aros_create(fs, drawer, &format!("f{f:02}.c")); } }
    assert_eq!(afsplus_aros_flush(fs), 0);
    eprintln!("READY pid {}", std::process::id());
    std::thread::sleep(Duration::from_secs(2));
    let end = Instant::now() + Duration::from_secs(seconds);
    let (mut ops, start) = (0u64, Instant::now());
    let mut pass = 0usize;
    let c0 = counters(fs);
    while Instant::now() < end {
        for drawer in 0..DRAWERS { for f in 0..FILES {
            match op.as_str() {
                "rename" => { let (a, b) = if pass % 2 == 0 { (".c", ".o") } else { (".o", ".c") };
                    aros_rename(fs, drawer, &format!("f{f:02}{a}"), &format!("f{f:02}{b}")); }
                _ => { if pass % 2 == 0 { aros_delete(fs, drawer, &format!("f{f:02}.c")); } else { aros_create(fs, drawer, &format!("f{f:02}.c")); } }
            }
            ops += 1;
        } }
        pass += 1;
    }
    let c1 = counters(fs);
    eprintln!("DONE {op}: {ops} ops, {:?} per op; per op {:.2} flushes, {:.2} writes, {:.1} calls", start.elapsed() / ops as u32,
        (c1.device_flushes - c0.device_flushes) as f64 / ops as f64, (c1.device_writes - c0.device_writes) as f64 / ops as f64, (c1.calls - c0.calls) as f64 / ops as f64);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

fn counters(fs: *mut AfsplusAros) -> AfsplusArosCounters {
    let mut o = AfsplusArosCounters { struct_size: std::mem::size_of::<AfsplusArosCounters>() as u32, ..Default::default() };
    assert_eq!(afsplus_aros_counters(fs, &mut o), 0);
    o
}
fn free_blocks(fs: *mut AfsplusAros) -> u64 {
    let mut h = AfsplusArosHealth { struct_size: std::mem::size_of::<AfsplusArosHealth>() as u32, ..Default::default() };
    assert_eq!(afsplus_aros_health(fs, &mut h), 0);
    h.free_blocks
}
fn orphans(fs: *mut AfsplusAros) -> u64 {
    let mut h = AfsplusArosHealth { struct_size: std::mem::size_of::<AfsplusArosHealth>() as u32, ..Default::default() };
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
    if std::env::var("SYNC").is_err() { assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0); }
    let mk = |fs, base: u64, name: &str| { let mut l = 0; assert_eq!(afsplus_aros_create_directory(fs, base, name.as_ptr(), name.len() as u32, now().0, 0, &mut l), 0); l };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 { let tl = mk(fs, bench, &format!("t{t:02}")); for d in 0..8 { let dl = mk(fs, tl, &format!("d{d}")); assert_eq!(afsplus_aros_free_lock(fs, dl), 0); } assert_eq!(afsplus_aros_free_lock(fs, tl), 0); }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);
    let empty = free_blocks(fs);
    for drawer in 0..DRAWERS { for f in 0..FILES { aros_create(fs, drawer, &format!("f{f:02}.c")); } }
    assert_eq!(afsplus_aros_flush(fs), 0);
    let full = free_blocks(fs);
    if std::env::var("SAMPLE").is_ok() {
        eprintln!("READY pid {}", std::process::id());
        std::thread::sleep(Duration::from_secs(1));
        for _ in 0..6 {
            for drawer in 0..DRAWERS { for f in 0..FILES { aros_delete(fs, drawer, &format!("f{f:02}.c")); } }
            for drawer in 0..DRAWERS { for f in 0..FILES { aros_create(fs, drawer, &format!("f{f:02}.c")); } }
        }
    }
    let c0 = counters(fs);
    let start = Instant::now();
    for drawer in 0..DRAWERS { for f in 0..FILES { aros_delete(fs, drawer, &format!("f{f:02}.c")); } }
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
    if std::env::var("SYNC").is_err() { assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0); }
    let mk = |fs, base: u64, name: &str| { let mut l = 0; assert_eq!(afsplus_aros_create_directory(fs, base, name.as_ptr(), name.len() as u32, now().0, 0, &mut l), 0); l };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 { let tl = mk(fs, bench, &format!("t{t:02}")); for d in 0..8 { let dl = mk(fs, tl, &format!("d{d}")); assert_eq!(afsplus_aros_free_lock(fs, dl), 0); } assert_eq!(afsplus_aros_free_lock(fs, tl), 0); }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);
    let c0 = counters(fs);
    let start = Instant::now();
    for drawer in 0..DRAWERS { for f in 0..FILES { aros_create(fs, drawer, &format!("f{f:02}.c")); } }
    let took = start.elapsed();
    let c1 = counters(fs);
    let n = (DRAWERS * FILES) as u64;
    eprintln!("creates: {n} in {took:?}, {:?} each; per create: {:.2} flushes, {:.2} writes, {:.1} calls, {:.1} cache reads ({:.2} misses)",
        took / n as u32, (c1.device_flushes - c0.device_flushes) as f64 / n as f64, (c1.device_writes - c0.device_writes) as f64 / n as f64,
        (c1.calls - c0.calls) as f64 / n as f64, (c1.cache_hits + c1.cache_misses - c0.cache_hits - c0.cache_misses) as f64 / n as f64,
        (c1.cache_misses - c0.cache_misses) as f64 / n as f64);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}

#[test]
#[ignore = "profiling harness"]
fn create_only() {
    use afsplus_block::MemoryBackend;
    let seconds: u64 = std::env::var("SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let mut device = common::format(MemoryBackend::new(4096, 131072), true);
    let fs = common::mount_sized(&mut device, 131072);
    let mut g = 0;
    assert_eq!(afsplus_aros_set_cache_blocks(fs, 64, &mut g), 0);
    if std::env::var("SYNC").is_err() { assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0); }
    let mk = |fs, base: u64, name: &str| { let mut l = 0; assert_eq!(afsplus_aros_create_directory(fs, base, name.as_ptr(), name.len() as u32, now().0, 0, &mut l), 0); l };
    let bench = mk(fs, 0, "bench");
    for t in 0..DRAWERS / 8 { let tl = mk(fs, bench, &format!("t{t:02}")); for d in 0..8 { let dl = mk(fs, tl, &format!("d{d}")); assert_eq!(afsplus_aros_free_lock(fs, dl), 0); } assert_eq!(afsplus_aros_free_lock(fs, tl), 0); }
    assert_eq!(afsplus_aros_free_lock(fs, bench), 0);
    assert_eq!(afsplus_aros_flush(fs), 0);
    eprintln!("READY pid {}", std::process::id());
    std::thread::sleep(Duration::from_secs(2));
    let c0 = counters(fs);
    let end = Instant::now() + Duration::from_secs(seconds);
    let (mut ops, start) = (0u64, Instant::now());
    let mut pass = 0usize;
    while Instant::now() < end && ops < 15000 {
        for drawer in 0..DRAWERS { for f in 0..FILES { aros_create(fs, drawer, &format!("p{pass}f{f:02}.c")); ops += 1; } }
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
    let rounds: usize = std::env::var("ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let fsync = std::env::var("FSYNC_ON_CLOSE").is_ok();
    let mut d0 = 0;
    assert_eq!(afsplus_aros_create_directory(fs, 0, b"steady".as_ptr(), 6, now().0, 0, &mut d0), 0);
    assert_eq!(afsplus_aros_free_lock(fs, d0), 0);
    let skip = std::env::var("SKIP").unwrap_or_default();
    let round = |fs| {
        let t = now();
        // As the probe: the drawer is looked up per operation and nothing is
        // held between rounds.
        let mut d = 0;
        assert_eq!(afsplus_aros_locate(fs, 0, b"steady".as_ptr(), 6, 0, &mut d), 0);
        let mut file = 0;
        assert_eq!(afsplus_aros_open(fs, d, b"note".as_ptr(), 4, AFSPLUS_AROS_OPEN_NEW_FILE, t.0, t.1, &mut file), 0);
        let mut c = 0;
        assert_eq!(afsplus_aros_write(fs, file, [7u8; 1200].as_ptr(), 1200, t.0, t.1, &mut c), 0);
        if fsync { assert_eq!(afsplus_aros_fsync(fs, file), 0); }
        assert_eq!(afsplus_aros_close(fs, file), 0);
        if !skip.contains("read") {
            assert_eq!(afsplus_aros_open(fs, d, b"note".as_ptr(), 4, AFSPLUS_AROS_OPEN_OLD_FILE, t.0, t.1, &mut file), 0);
            let mut buf = [0u8; 1200]; let mut got = 0;
            assert_eq!(afsplus_aros_read(fs, file, buf.as_mut_ptr(), 1200, &mut got), 0);
            if !skip.contains("record") {
                assert_eq!(afsplus_aros_lock_record(fs, file, 0, 4, 1), 0);
                assert_eq!(afsplus_aros_free_record(fs, file, 0, 4), 0);
            }
            assert_eq!(afsplus_aros_close(fs, file), 0);
        }
        if !skip.contains("examine") {
            let mut l = 0;
            assert_eq!(afsplus_aros_locate(fs, d, b"note".as_ptr(), 4, 0, &mut l), 0);
            let mut info = AfsplusArosFileInfo::default(); let mut name = [0u8; 108];
            assert_eq!(afsplus_aros_examine_lock(fs, l, &mut info, name.as_mut_ptr(), 108), 0);
            assert_eq!(afsplus_aros_free_lock(fs, l), 0);
        }
        if !skip.contains("watch") {
            let mut w = 0;
            assert_eq!(afsplus_aros_watch_add(fs, d, b"note".as_ptr(), 4, &mut w), 0);
            assert_eq!(afsplus_aros_watch_remove(fs, w), 0);
        }
        if !skip.contains("comment") { assert_eq!(afsplus_aros_set_comment(fs, d, b"note".as_ptr(), 4, b"round".as_ptr(), 5, t.0, t.1), 0); }
        if !skip.contains("protect") { assert_eq!(afsplus_aros_set_protection(fs, d, b"note".as_ptr(), 4, 0x10, t.0, t.1), 0); }
        assert_eq!(afsplus_aros_delete_object(fs, d, b"note".as_ptr(), 4, t.0, t.1), 0);
        assert_eq!(afsplus_aros_free_lock(fs, d), 0);
        let mut pending = 0; assert_eq!(afsplus_aros_commit_due(fs, t.0, t.1, &mut pending), 0);
    };
    let settle = std::env::var("SETTLE").is_ok();
    let warmup: usize = std::env::var("WARMUP").ok().and_then(|v| v.parse().ok()).unwrap_or(rounds);
    let reading = |fs| {
        assert_eq!(afsplus_aros_flush(fs), 0);
        if settle { for _ in 0..64 { if orphans(fs) == 0 { break; } assert_eq!(afsplus_aros_flush(fs), 0); } }
        let c = counters(fs);
        (c.heap_bytes, c.heap_peak_bytes, orphans(fs))
    };
    for _ in 0..warmup { round(fs); }
    let warm = reading(fs);
    for _ in 0..rounds { round(fs); }
    let done = reading(fs);
    eprintln!("STEADY fsync={fsync} rounds={rounds}: heap {} -> {} ({:+}), peak {} -> {} ({:+}), orphans {} -> {}",
        warm.0, done.0, done.0 as i64 - warm.0 as i64, warm.1, done.1, done.1 as i64 - warm.1 as i64, warm.2, done.2);
    assert_eq!(afsplus_aros_unmount(fs), 0);
}
