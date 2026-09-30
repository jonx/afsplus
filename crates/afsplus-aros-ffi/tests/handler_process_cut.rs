//! Kill a process running the handler's real FFI at disk-write/flush boundaries.
//!
//! Unlike dropping a memory-backend clone, SIGKILL prevents unmount, Rust drops
//! and all further device callbacks. This models handler death with a surviving
//! host kernel, not loss/reordering of the host's unflushed drive cache.
#![cfg(unix)]

use std::ffi::c_void;
use std::fs::{self, File};
use std::io::Write;
use std::mem::size_of;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr;
use std::slice;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, FileBackend};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, NamePolicy};
use afsplus_format::Timespec;

const BLOCK: usize = 4096;
const BLOCKS: u64 = 2048;
const CONTENT: usize = 8192;

struct Device {
    disk: FileBackend,
    event: usize,
    cut: usize,
    armed: bool,
    trace: Option<File>,
    ready: PathBuf,
}

impl Device {
    fn open(path: &Path) -> Self {
        Self {
            disk: FileBackend::open(path, BLOCK, BLOCKS).unwrap(),
            event: 0,
            cut: 0,
            armed: false,
            trace: None,
            ready: PathBuf::new(),
        }
    }

    fn boundary(&mut self, kind: &str) {
        if !self.armed {
            return;
        }
        self.event += 1;
        if let Some(trace) = &mut self.trace {
            writeln!(trace, "{} {kind}", self.event).unwrap();
            trace.flush().unwrap();
        }
        if self.event == self.cut {
            fs::write(&self.ready, format!("{} {kind}\n", self.event)).unwrap();
            // Only the parent may end this process. No unwind, Close or flush.
            loop {
                thread::park();
            }
        }
    }
}

unsafe extern "C" fn read(ctx: *mut c_void, lba: u64, dst: *mut u8, n: u32) -> i32 {
    // SAFETY: mount retains Device and the FFI supplies a writable block.
    let device = unsafe { &mut *ctx.cast::<Device>() };
    let bytes = unsafe { slice::from_raw_parts_mut(dst, n as usize) };
    device.disk.read_block(lba, bytes).map_or(5, |()| 0)
}

unsafe extern "C" fn write(ctx: *mut c_void, lba: u64, src: *const u8, n: u32) -> i32 {
    // SAFETY: mount retains Device and the FFI supplies a readable block.
    let device = unsafe { &mut *ctx.cast::<Device>() };
    let bytes = unsafe { slice::from_raw_parts(src, n as usize) };
    device.boundary("before-write");
    if device.disk.write_block(lba, bytes).is_err() {
        return 5;
    }
    device.boundary("after-write");
    0
}

unsafe extern "C" fn flush(ctx: *mut c_void) -> i32 {
    // SAFETY: the device context lives through every callback.
    let device = unsafe { &mut *ctx.cast::<Device>() };
    device.boundary("before-flush");
    if device.disk.flush().is_err() {
        return 5;
    }
    device.boundary("after-flush");
    0
}

fn mount(device: &mut Device) -> *mut AfsplusAros {
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(device).cast(),
        block_size: BLOCK as u32,
        reserved: 0,
        total_blocks: BLOCKS,
        read_block: Some(read),
        write_block: Some(write),
        flush: Some(flush),
    };
    let config = AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: b"Cut".as_ptr(),
        volume_name_length: 3,
        max_file_handles: 8,
        max_locks: 8,
        max_file_info_name_bytes: 107,
        flags: 0,
    };
    let mut fs = ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut fs), 0);
    fs
}

fn open(fs: *mut AfsplusAros, name: &[u8], mode: u32) -> u64 {
    let mut handle = 0;
    assert_eq!(
        afsplus_aros_open(
            fs,
            0,
            name.as_ptr(),
            name.len() as u32,
            mode,
            1,
            0,
            &mut handle
        ),
        0
    );
    handle
}

fn put(fs: *mut AfsplusAros, file: u64, bytes: &[u8]) {
    let mut count = 0;
    assert_eq!(
        afsplus_aros_write(
            fs,
            file,
            bytes.as_ptr(),
            bytes.len() as u32,
            2,
            0,
            &mut count
        ),
        0
    );
    assert_eq!(count as usize, bytes.len());
}

fn contents(fs: *mut AfsplusAros, name: &[u8]) -> Vec<u8> {
    let file = open(fs, name, AFSPLUS_AROS_OPEN_OLD_FILE);
    let mut bytes = vec![0; CONTENT + 1];
    let mut count = 0;
    assert_eq!(
        afsplus_aros_read(fs, file, bytes.as_mut_ptr(), bytes.len() as u32, &mut count),
        0
    );
    bytes.truncate(count as usize);
    assert_eq!(afsplus_aros_close(fs, file), 0);
    bytes
}

fn allowed(kept: &[u8], target: &[u8], acknowledged: bool) -> bool {
    kept == [b'K'; CONTENT]
        && (target == [b'N'; CONTENT] || (!acknowledged && target == [b'O'; CONTENT]))
}

fn verify(path: &Path, acknowledged: bool) {
    let mut device = Device::open(path);
    let fs = mount(&mut device);
    assert!(
        allowed(
            &contents(fs, b"kept"),
            &contents(fs, b"target"),
            acknowledged
        ),
        "lost durable sentinel, partial overwrite or lost acknowledged write: {}",
        path.display()
    );
    // Recovered volumes must also accept new writes and retain them on remount.
    let file = open(fs, b"after", AFSPLUS_AROS_OPEN_NEW_FILE);
    put(fs, file, b"usable after abrupt termination");
    assert_eq!(afsplus_aros_fsync(fs, file), 0);
    assert_eq!(afsplus_aros_close(fs, file), 0);
    assert_eq!(afsplus_aros_unmount(fs), 0);
    let fs = mount(&mut device);
    assert_eq!(contents(fs, b"after"), b"usable after abrupt termination");
    assert_eq!(afsplus_aros_unmount(fs), 0);
    let report = check_device(&mut device.disk);
    assert!(
        report.errors.is_empty(),
        "checker rejects recovered image {}: {report:?}",
        path.display()
    );
}

// Executed only in a child test process with an explicit private image.
#[test]
#[ignore = "subprocess entry, invoked by killed_handler_recovers_at_every_device_boundary"]
fn child_entry() {
    let image = PathBuf::from(std::env::var_os("AFSPLUS_CUT_IMAGE").unwrap());
    let mut device = Device::open(&image);
    let fs = mount(&mut device);
    let file = open(fs, b"target", AFSPLUS_AROS_OPEN_READ_WRITE);
    assert_eq!(afsplus_aros_set_commit_policy(fs, 5_000, 1_000), 0);
    device.cut = std::env::var("AFSPLUS_CUT_EVENT").unwrap().parse().unwrap();
    device.ready = image.with_extension("ready");
    device.trace = Some(File::create(image.with_extension("trace")).unwrap());
    device.armed = true;
    put(fs, file, &[b'N'; CONTENT]);
    assert_eq!(afsplus_aros_fsync(fs, file), 0);
    device.boundary("acknowledged-fsync");
    // Deliberately exit without calling unmount even for the no-cut control.
}

fn run(image: &Path, cut: usize) {
    let stdout = File::create(image.with_extension("log")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_entry", "--ignored", "--nocapture"])
        .env("AFSPLUS_CUT_IMAGE", image)
        .env("AFSPLUS_CUT_EVENT", cut.to_string())
        .stdout(stdout.try_clone().unwrap())
        .stderr(Stdio::from(stdout))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if cut != 0 && image.with_extension("ready").exists() {
            child.kill().unwrap();
            assert_eq!(
                child.wait().unwrap().signal(),
                Some(9),
                "must be SIGKILL, not graceful exit"
            );
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                cut == 0 && status.success(),
                "child missed cut {cut}: {status}; {}",
                image.display()
            );
            return;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("child timeout: {}", image.display());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn killed_handler_recovers_at_every_device_boundary() {
    let directory = std::env::temp_dir().join(format!(
        "afsplus-process-cut-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    eprintln!("process-cut artifacts: {}", directory.display());
    let baseline = directory.join("baseline.img");
    let mut disk = FileBackend::create_new(&baseline, BLOCK, BLOCKS).unwrap();
    mkfs(
        &mut disk,
        &MkfsParams {
            uuid: [0xCA; 16],
            label: "ProcessCut".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
        },
    )
    .unwrap();
    drop(disk);
    let mut device = Device::open(&baseline);
    let fs = mount(&mut device);
    for (name, byte) in [(b"kept".as_slice(), b'K'), (b"target".as_slice(), b'O')] {
        let file = open(fs, name, AFSPLUS_AROS_OPEN_NEW_FILE);
        put(fs, file, &[byte; CONTENT]);
        assert_eq!(afsplus_aros_fsync(fs, file), 0);
        assert_eq!(afsplus_aros_close(fs, file), 0);
    }
    assert_eq!(afsplus_aros_unmount(fs), 0);
    drop(device);
    let control = directory.join("control.img");
    fs::copy(&baseline, &control).unwrap();
    run(&control, 0);
    let trace = fs::read_to_string(control.with_extension("trace")).unwrap();
    let events: Vec<_> = trace.lines().collect();
    assert!(events.iter().any(|e| e.ends_with("after-write")));
    assert!(events.iter().any(|e| e.ends_with("after-flush")));
    assert!(events.last().unwrap().ends_with("acknowledged-fsync"));
    verify(&control, true);
    for (index, event) in events.iter().enumerate() {
        let image = directory.join(format!("cut-{}.img", index + 1));
        fs::copy(&baseline, &image).unwrap();
        run(&image, index + 1);
        verify(&image, event.ends_with("acknowledged-fsync"));
    }
    // Independent literal oracle controls: damage and loss of an acknowledged
    // write must fail even when the volume would still mount cleanly.
    assert!(!allowed(&[b'X'; CONTENT], &[b'N'; CONTENT], false));
    assert!(!allowed(&[b'K'; CONTENT], &[b'O'; CONTENT], true));
    let mut mixed = [b'O'; CONTENT];
    mixed[..BLOCK].fill(b'N');
    assert!(!allowed(&[b'K'; CONTENT], &mixed, false));
    fs::write(
        directory.join("result.txt"),
        format!(
            "result=PASS cuts={} control=1 semantic_negative_controls=3 model=process-death\n",
            events.len()
        ),
    )
    .unwrap();
    eprintln!(
        "process-cut PASS cuts={} artifacts={}",
        events.len(),
        directory.display()
    );
}
