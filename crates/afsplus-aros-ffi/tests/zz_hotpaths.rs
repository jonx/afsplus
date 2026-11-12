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

/// Runs `body` `calls` times and prints what one call costs: time, and the
/// blocks the library read, from its cache or the device. `per` divides
/// both when one call serves several entries. HOT=name loops that one
/// body first, for a sampler.
fn bench(
    fs: *mut AfsplusAros,
    name: &str,
    calls: usize,
    per: usize,
    mut body: impl FnMut(usize),
) {
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
    // Once unmeasured: the first pass decodes what later passes find kept.
    body(0);
    let before = reads(fs);
    let start = Instant::now();
    for i in 0..calls {
        body(i);
    }
    let units = (calls * per) as f64;
    let ns = start.elapsed().as_nanos() as f64 / units;
    let after = reads(fs);
    println!(
        "HOT {name:<26} {ns:>9.0} ns  {:>7.2} block reads  {:>7.2} cache accesses  ({calls} x {per})",
        (after.0 - before.0) as f64 / units,
        (after.1 - before.1) as f64 / units,
    );
}

/// Blocks whose bytes the library read, from its cache or the device, and
/// every access to a cached block, read or found decoded.
fn reads(fs: *mut AfsplusAros) -> (u64, u64) {
    let mut counters = AfsplusArosCounters {
        struct_size: std::mem::size_of::<AfsplusArosCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_counters(fs, &mut counters), 0);
    let copied = counters.cache_hits + counters.cache_misses;
    (copied, copied)
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
    // The mount the handler offers with delayed commit (ADR-121): nothing
    // waits during the loops below, which only read.
    assert_eq!(afsplus_aros_set_commit_policy(fs, 5000, 1000), 0);
    // Idle ticks until the space the setup released is returned.
    for tick in 0..100_000 {
        assert_eq!(
            afsplus_aros_commit_due(fs, 110_000 + tick, 0, &mut pending),
            0
        );
        if pending == 0 {
            break;
        }
    }
    assert_eq!(pending, 0, "idle maintenance never rests");
    let full = |i: usize| format!("{}/module{:04}.dll", DIRS.join("/"), i % FILES);
    let paths: Vec<String> = (0..FILES).map(full).collect();
    let names: Vec<String> = (0..FILES).map(|i| format!("module{i:04}.dll")).collect();
    let mut info = AfsplusArosFileInfo::default();
    let mut name = [0u8; 108];
    let mut comment = [0u8; 80];

    bench(fs, "locate_path_8_free", 20000, 1, |i| {
        let p = &paths[i % FILES];
        let mut l = 0;
        assert_eq!(
            afsplus_aros_locate_path(fs, 0, p.as_ptr(), p.len() as u32, 0, &mut l),
            0
        );
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    });
    // AFSPLUS_ACTION_EXAMINE_NAME64: resolve, examine, free.
    bench(fs, "stat_by_name_8", 20000, 1, |i| {
        let p = &paths[i % FILES];
        let mut l = 0;
        assert_eq!(
            afsplus_aros_locate_path(fs, 0, p.as_ptr(), p.len() as u32, 0, &mut l),
            0
        );
        assert_eq!(
            afsplus_aros_examine_lock(fs, l, &mut info, name.as_mut_ptr(), 108),
            0
        );
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    });
    bench(fs, "locate_1_free", 20000, 1, |i| {
        let p = &names[i % FILES];
        let mut l = 0;
        assert_eq!(
            afsplus_aros_locate(fs, dir, p.as_ptr(), p.len() as u32, 0, &mut l),
            0
        );
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    });
    let absent: Vec<String> = (0..FILES).map(|i| format!("absent{i:04}.dll")).collect();
    bench(fs, "locate_missing", 20000, 1, |i| {
        let p = &absent[i % FILES];
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
    bench(fs, "examine_lock", 20000, 1, |_| {
        assert_eq!(
            afsplus_aros_examine_lock(fs, one, &mut info, name.as_mut_ptr(), 108),
            0
        );
    });
    // ACTION_EXAMINE_OBJECT asks for the comment in a second call.
    bench(fs, "comment_of_lock", 20000, 1, |_| {
        let mut length = 0;
        assert_eq!(
            afsplus_aros_comment(
                fs,
                one,
                std::ptr::null(),
                0,
                comment.as_mut_ptr(),
                80,
                &mut length
            ),
            0
        );
    });
    bench(fs, "comment_by_name", 20000, 1, |i| {
        let p = &names[i % FILES];
        let mut length = 0;
        assert_eq!(
            afsplus_aros_comment(
                fs,
                dir,
                p.as_ptr(),
                p.len() as u32,
                comment.as_mut_ptr(),
                80,
                &mut length
            ),
            0
        );
    });
    bench(fs, "duplicate_free", 20000, 1, |_| {
        let mut l = 0;
        assert_eq!(afsplus_aros_duplicate_lock(fs, one, &mut l), 0);
        assert_eq!(afsplus_aros_free_lock(fs, l), 0);
    });
    // ACTION_EXAMINE_OBJECT rewinds the lock's cursor, ACTION_EXAMINE_NEXT
    // then takes one entry a packet, and its comment by name.
    bench(fs, "examine_next_per_entry", 20, FILES, |_| {
        assert_eq!(afsplus_aros_rewind_directory(fs, dir), 0);
        assert_eq!(
            afsplus_aros_examine_lock(fs, dir, &mut info, name.as_mut_ptr(), 108),
            0
        );
        let mut n = 0;
        while afsplus_aros_examine_next(fs, dir, &mut info, name.as_mut_ptr(), 108) == 0 {
            let mut length = 0;
            assert_eq!(
                afsplus_aros_comment(
                    fs,
                    dir,
                    name.as_ptr(),
                    info.name_length,
                    comment.as_mut_ptr(),
                    80,
                    &mut length
                ),
                0
            );
            n += 1;
        }
        assert_eq!(n, FILES);
    });
    // ACTION_EXAMINE_ALL below ED_COMMENT: a rewind, then examine_next for
    // every entry the caller's buffer takes, no comment.
    bench(fs, "exall_per_entry", 20, FILES, |_| {
        assert_eq!(afsplus_aros_rewind_directory(fs, dir), 0);
        let mut n = 0;
        while afsplus_aros_examine_next(fs, dir, &mut info, name.as_mut_ptr(), 108) == 0 {
            n += 1;
        }
        assert_eq!(n, FILES);
    });
    let mut page = vec![0u8; 64 * AFSPLUS_AROS_DIR_RECORD_MAX as usize];
    bench(fs, "dir_read_per_entry", 20, FILES, |_| {
        let mut walk = 0;
        assert_eq!(afsplus_aros_dir_open(fs, dir, &mut walk), 0);
        let (mut n, mut eof) = (0, 0);
        while eof == 0 {
            let mut count = 0;
            assert_eq!(
                afsplus_aros_dir_read(
                    fs,
                    walk,
                    page.as_mut_ptr(),
                    page.len() as u32,
                    64,
                    &mut count,
                    &mut eof
                ),
                0
            );
            n += count as usize;
        }
        assert_eq!(n, FILES);
        assert_eq!(afsplus_aros_dir_close(fs, walk), 0);
    });
    bench(fs, "open_close", 20000, 1, |i| {
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
    bench(fs, "read_at_4k", 20000, 1, |i| {
        let mut c = 0;
        assert_eq!(
            afsplus_aros_read_at(
                fs,
                file,
                (i as u64 % 2) * 4096,
                buffer.as_mut_ptr(),
                4096,
                &mut c
            ),
            0
        );
        assert_eq!(c, 4096);
    });
    bench(fs, "read_at_4k_unaligned", 20000, 1, |_| {
        let mut c = 0;
        assert_eq!(
            afsplus_aros_read_at(fs, file, 1000, buffer.as_mut_ptr(), 4096, &mut c),
            0
        );
        assert_eq!(c, 4096);
    });
    bench(fs, "examine_file", 20000, 1, |_| {
        assert_eq!(
            afsplus_aros_examine_file(fs, file, &mut info, name.as_mut_ptr(), 108),
            0
        );
    });
    bench(fs, "file_comment", 20000, 1, |_| {
        let mut length = 0;
        assert_eq!(
            afsplus_aros_file_comment(fs, file, comment.as_mut_ptr(), 80, &mut length),
            0
        );
    });
    // What the handler asks after every batch of packets on a delayed
    // mount with nothing waiting.
    bench(fs, "commit_due_idle", 20000, 1, |i| {
        let mut pending = 0;
        assert_eq!(
            afsplus_aros_commit_due(fs, 300_000 + i as i64, 0, &mut pending),
            0
        );
        assert_eq!(pending, 0);
    });
}
