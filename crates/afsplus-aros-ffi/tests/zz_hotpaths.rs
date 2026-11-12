//! Read-side hot paths as the AROS handler calls them, timed per call:
//! the calls behind Lock, Examine, ExNext, pread and Open on a tree like a
//! Wine prefix (eight path components, 800 files in the last directory).
//! Ignored by default. HOT=name loops one call for a sampler, SECONDS=n.
mod common;
use afsplus_aros_ffi::*;
use afsplus_block::MemoryBackend;
use common::{format, mount_sized};
use std::time::Instant;

const DIRS: [&str; 7] = [
    "linux", "home", "aros", "wine", "drive_c", "windows", "system32",
];
const FILES: usize = 800;

fn bench(name: &str, calls: usize, mut body: impl FnMut(usize)) {
    let only = std::env::var("HOT").ok();
    if let Some(o) = &only {
        if o != name {
            return;
        }
        let seconds: u64 = std::env::var("SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let start = Instant::now();
        let mut i = 0;
        while start.elapsed().as_secs() < seconds {
            body(i);
            i += 1;
        }
    }
    let start = Instant::now();
    for i in 0..calls {
        body(i);
    }
    let ns = start.elapsed().as_nanos() / calls as u128;
    println!("HOT {name:<28} {ns:>8} ns  {calls} calls");
}

/// Block reads the library made, from its cache or the device.
fn reads(fs: *mut AfsplusAros, counters: &mut AfsplusArosCounters) -> u64 {
    assert_eq!(afsplus_aros_counters(fs, counters), 0);
    counters.cache_hits + counters.cache_misses
}

#[test]
#[ignore]
fn hot_paths() {
    // 256 MiB, sparse: room for the filler objects
    const BLOCKS: u64 = 65536;
    let mut device = format(MemoryBackend::new(4096, BLOCKS), false);
    let fs = mount_sized(&mut device, BLOCKS);
    let mut granted = 0;
    let blocks: u32 = std::env::var("CACHE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16384);
    assert_eq!(afsplus_aros_set_cache_blocks(fs, blocks, &mut granted), 0);
    let mut lock = 0u64;
    for d in DIRS {
        let mut next = 0;
        assert_eq!(
            afsplus_aros_create_directory(fs, lock, d.as_ptr(), d.len() as u32, 1, 0, &mut next),
            0
        );
        if lock != 0 {
            assert_eq!(afsplus_aros_free_lock(fs, lock), 0);
        }
        lock = next;
    }
    let dir = lock;
    let data = [0x5au8; 8192];
    for i in 0..FILES {
        let name = format!("module{i:04}.dll");
        let mut file = 0;
        assert_eq!(
            afsplus_aros_open(
                fs,
                dir,
                name.as_ptr(),
                name.len() as u32,
                AFSPLUS_AROS_OPEN_NEW_FILE,
                2,
                0,
                &mut file
            ),
            0
        );
        let mut c = 0;
        assert_eq!(
            afsplus_aros_write(fs, file, data.as_ptr(), 8192, 2, 0, &mut c),
            0
        );
        assert_eq!(afsplus_aros_close(fs, file), 0);
    }
    let mut pending = 0;
    assert_eq!(afsplus_aros_commit_due(fs, 100, 0, &mut pending), 0);
    // A volume holds many more objects than the path walked: they are what
    // makes its trees as deep as a real one's. FILLER empty files, 1000 a drawer.
    let filler: usize = std::env::var("FILLER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20000);
    let mut drawer = 0;
    for i in 0..filler {
        if i % 1000 == 0 {
            if drawer != 0 {
                assert_eq!(afsplus_aros_free_lock(fs, drawer), 0);
            }
            let name = format!("filler{:03}", i / 1000);
            assert_eq!(
                afsplus_aros_create_directory(
                    fs,
                    0,
                    name.as_ptr(),
                    name.len() as u32,
                    4,
                    0,
                    &mut drawer
                ),
                0
            );
        }
        let name = format!("object{i:06}.bin");
        let mut file = 0;
        assert_eq!(
            afsplus_aros_open(
                fs,
                drawer,
                name.as_ptr(),
                name.len() as u32,
                AFSPLUS_AROS_OPEN_NEW_FILE,
                4,
                0,
                &mut file
            ),
            0
        );
        assert_eq!(afsplus_aros_close(fs, file), 0);
        if i % 256 == 255 {
            assert_eq!(
                afsplus_aros_commit_due(fs, 200 + i as i64, 0, &mut pending),
                0
            );
        }
    }
    if drawer != 0 {
        assert_eq!(afsplus_aros_free_lock(fs, drawer), 0);
    }
    assert_eq!(afsplus_aros_commit_due(fs, 100_000, 0, &mut pending), 0);
    let mut counters = AfsplusArosCounters {
        struct_size: std::mem::size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };

    let full = |i: usize| format!("{}/module{:04}.dll", DIRS.join("/"), i % FILES);
    let paths: Vec<String> = (0..FILES).map(full).collect();
    let names: Vec<String> = (0..FILES).map(|i| format!("module{i:04}.dll")).collect();
    let mut info = AfsplusArosFileInfo::default();
    let mut name = [0u8; 108];

    let before = reads(fs, &mut counters);
    bench("locate_path_8_free", 20000, |i| {
        let p = &paths[i % FILES];
        let mut l = 0;
        assert_eq!(
            afsplus_aros_locate_path(fs, 0, p.as_ptr(), p.len() as u32, 0, &mut l),
            0
        );
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    });
    if std::env::var("HOT").is_err() {
        println!(
            "HOT locate_path_8_free block reads per call: {}",
            (reads(fs, &mut counters) - before) / 20000
        );
    }
    bench("locate_1_free", 20000, |i| {
        let p = &names[i % FILES];
        let mut l = 0;
        assert_eq!(
            afsplus_aros_locate(fs, dir, p.as_ptr(), p.len() as u32, 0, &mut l),
            0
        );
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    });
    bench("locate_missing", 20000, |i| {
        let p = format!("absent{:04}.dll", i % FILES);
        let mut l = 0;
        assert_ne!(
            afsplus_aros_locate(fs, dir, p.as_ptr(), p.len() as u32, 0, &mut l),
            0
        );
    });
    let mut one = 0;
    assert_eq!(
        afsplus_aros_locate(
            fs,
            dir,
            names[7].as_ptr(),
            names[7].len() as u32,
            0,
            &mut one
        ),
        0
    );
    let before = reads(fs, &mut counters);
    bench("examine_lock", 20000, |_| {
        assert_eq!(
            afsplus_aros_examine_lock(fs, one, &mut info, name.as_mut_ptr(), 108),
            0
        );
    });
    if std::env::var("HOT").is_err() {
        println!(
            "HOT examine_lock block reads per call: {}",
            (reads(fs, &mut counters) - before) / 20000
        );
    }
    bench("duplicate_free", 20000, |_| {
        let mut l = 0;
        assert_eq!(afsplus_aros_duplicate_lock(fs, one, &mut l), 0);
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    });
    bench("examine_next_per_entry", 20, |_| {
        assert_eq!(
            afsplus_aros_examine_lock(fs, dir, &mut info, name.as_mut_ptr(), 108),
            0
        );
        let mut n = 0;
        while afsplus_aros_examine_next(fs, dir, &mut info, name.as_mut_ptr(), 108) == 0 {
            n += 1;
        }
        assert!(n > 0, "entries {n}");
    });
    bench("open_close", 20000, |i| {
        let p = &names[i % FILES];
        let mut f = 0;
        assert_eq!(
            afsplus_aros_open(
                fs,
                dir,
                p.as_ptr(),
                p.len() as u32,
                AFSPLUS_AROS_OPEN_OLD_FILE,
                3,
                0,
                &mut f
            ),
            0
        );
        assert_eq!(afsplus_aros_close(fs, f), 0);
    });
    let mut file = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            dir,
            names[7].as_ptr(),
            names[7].len() as u32,
            AFSPLUS_AROS_OPEN_OLD_FILE,
            3,
            0,
            &mut file
        ),
        0
    );
    let mut buffer = [0u8; 4096];
    bench("read_at_4k", 20000, |_| {
        let mut c = 0;
        assert_eq!(
            afsplus_aros_read_at(fs, file, 0, buffer.as_mut_ptr(), 4096, &mut c),
            0
        );
        assert_eq!(c, 4096);
    });
    bench("examine_file", 20000, |_| {
        assert_eq!(
            afsplus_aros_examine_file(fs, file, &mut info, name.as_mut_ptr(), 108),
            0
        );
    });
}
