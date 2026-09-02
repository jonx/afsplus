#![cfg(feature = "fuser-adapter")]

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use afsplus_block::FileBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE};

const TOTAL_BLOCKS: u64 = 8192;

#[test]
#[ignore = "requires a working host FUSE installation and AFSPLUS_FUSE_MOUNT_TEST=1"]
fn real_mount_runs_the_alpha_operation_matrix_and_leaves_a_clean_image() {
    assert_eq!(
        std::env::var("AFSPLUS_FUSE_MOUNT_TEST").as_deref(),
        Ok("1"),
        "set AFSPLUS_FUSE_MOUNT_TEST=1 to acknowledge the host mount"
    );

    let base = unique_test_directory();
    let image = base.join("volume.afsp");
    let mountpoint = unique_mountpoint(&base);
    fs::create_dir_all(&base).unwrap();
    if !cfg!(target_os = "macos") || std::env::var_os("AFSPLUS_FUSE_MOUNT_ROOT").is_some() {
        fs::create_dir_all(&mountpoint).unwrap();
    }
    create_image(&image);

    let child = Command::new(env!("CARGO_BIN_EXE_afsplus-mount"))
        .arg(&image)
        .arg(&mountpoint)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut mounted = MountedChild::new(child, mountpoint.clone());
    wait_until_mounted(&mut mounted, &mountpoint);

    assert_eq!(fs::read(mountpoint.join("seed")).unwrap(), b"mounted");
    fs::create_dir(mountpoint.join("work")).unwrap();
    let draft = mountpoint.join("work/draft");
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&draft)
        .unwrap();
    let before_fsync_generation = {
        let mut device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS).unwrap();
        let report = check_device(&mut device);
        assert!(report.is_clean(), "{:?}", report.errors);
        report.volume.unwrap().generation
    };
    file.write_all(b"hello").unwrap();
    file.seek(SeekFrom::Start(8192)).unwrap();
    file.write_all(b"tail").unwrap();
    file.sync_all().unwrap();
    let mut after_fsync_device =
        FileBackend::open(&image, DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS).unwrap();
    let after_fsync = check_device(&mut after_fsync_device);
    assert!(after_fsync.is_clean(), "{:?}", after_fsync.errors);
    let after_fsync = after_fsync.volume.unwrap();
    assert!(
        after_fsync.log_records_pending > 0 || after_fsync.generation > before_fsync_generation,
        "host fsync returned without a durable intent record or checkpoint"
    );
    file.set_len(5).unwrap();
    drop(file);

    let final_path = mountpoint.join("final");
    fs::rename(&draft, &final_path).unwrap();
    fs::hard_link(&final_path, mountpoint.join("work/linked")).unwrap();
    fs::remove_file(&final_path).unwrap();
    assert_eq!(fs::read(mountpoint.join("work/linked")).unwrap(), b"hello");

    let replacement = mountpoint.join("replacement");
    fs::write(&replacement, b"new").unwrap();
    File::open(&replacement).unwrap().sync_all().unwrap();
    fs::rename(&replacement, mountpoint.join("work/linked")).unwrap();
    assert_eq!(fs::read(mountpoint.join("work/linked")).unwrap(), b"new");
    fs::remove_file(mountpoint.join("work/linked")).unwrap();
    fs::remove_dir(mountpoint.join("work")).unwrap();

    mounted.unmount().unwrap();
    mounted.wait().unwrap();

    let mut device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS).unwrap();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    fs::remove_dir_all(base).unwrap();
}

fn create_image(path: &Path) {
    let timestamp = timestamp();
    let mut device = FileBackend::create(path, DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS).unwrap();
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xA0; 16],
            label: "FuseQualification".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp,
        },
    )
    .unwrap();
    mount(device)
        .unwrap()
        .create_file_in_root("seed", b"mounted", timestamp)
        .unwrap();
}

fn timestamp() -> Timespec {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    Timespec {
        seconds: duration.as_secs() as i64,
        nanoseconds: duration.subsec_nanos(),
    }
}

fn unique_test_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("afsplus-fuse-{}-{nonce}", std::process::id()))
}

fn unique_mountpoint(base: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::var_os("AFSPLUS_FUSE_MOUNT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/Volumes"));
        root.join(format!(
            "AFSPlusQualification-{}-{nonce}",
            std::process::id()
        ))
    } else {
        base.join("mount")
    }
}

fn wait_until_mounted(mounted: &mut MountedChild, mountpoint: &Path) {
    let mut last_error = None;
    for _ in 0..200 {
        match fs::read(mountpoint.join("seed")) {
            Ok(data) if data == b"mounted" => return,
            Ok(data) => last_error = Some(format!("unexpected seed contents: {data:?}")),
            Err(error) => last_error = Some(error.to_string()),
        }
        if let Some(status) = mounted.child.try_wait().unwrap() {
            panic!("afsplus-mount exited before mounting: {status}");
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for the FUSE mount: {last_error:?}");
}

struct MountedChild {
    child: Child,
    mountpoint: PathBuf,
    unmounted: bool,
}

impl MountedChild {
    fn new(child: Child, mountpoint: PathBuf) -> Self {
        MountedChild {
            child,
            mountpoint,
            unmounted: false,
        }
    }

    fn unmount(&mut self) -> std::io::Result<()> {
        if self.unmounted {
            return Ok(());
        }
        let status = if cfg!(target_os = "macos") {
            Command::new("diskutil")
                .arg("unmount")
                .arg(&self.mountpoint)
                .status()?
        } else {
            Command::new("fusermount3")
                .arg("-u")
                .arg(&self.mountpoint)
                .status()
                .or_else(|_| Command::new("umount").arg(&self.mountpoint).status())?
        };
        if !status.success() {
            return Err(std::io::Error::other(format!(
                "unmount command failed with {status}"
            )));
        }
        self.unmounted = true;
        Ok(())
    }

    fn wait(&mut self) -> std::io::Result<()> {
        let status = self.child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "afsplus-mount exited with {status}"
            )))
        }
    }
}

impl Drop for MountedChild {
    fn drop(&mut self) {
        let _ = self.unmount();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
