//! `afsplus-image-diff` over two real image files.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use afsplus_block::FileBackend;
use afsplus_core::mount;
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "afsplus-image-diff-{test}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn format_image(path: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_mkafsplus"))
        .args([
            "--size-mib",
            "2",
            "--uuid",
            "00112233-4455-6677-8899-aabbccddeeff",
            "--timestamp-seconds",
            "123",
            "--profile",
            "workstation",
        ])
        .arg(path.as_os_str())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
}

fn diff(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_afsplus-image-diff"))
        .args(args)
        .output()
        .unwrap()
}

fn time(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

#[test]
fn the_command_reports_a_file_creation_in_both_forms() {
    let temp = TempDir::new("create");
    let before = temp.join("before.afsplus");
    let after = temp.join("after.afsplus");
    format_image(&before);
    fs::copy(&before, &after).unwrap();

    let device = FileBackend::open(&after, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut volume = mount(device).unwrap();
    let made = volume
        .create_file_in_directory(OBJECT_ROOT, "made", b"hello", time(456))
        .unwrap();
    volume.sync().unwrap();
    drop(volume.into_device());

    let output = diff(&[before.as_os_str(), after.as_os_str()]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains(&format!("link added 1/\"made\" -> object {made}")),
        "{text}"
    );
    assert!(
        text.contains(&format!("object {made} created (file, 5 bytes, 1 links)")),
        "{text}"
    );

    let output = diff(&[
        std::ffi::OsStr::new("--json"),
        before.as_os_str(),
        after.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let json = String::from_utf8(output.stdout).unwrap();
    assert!(
        json.starts_with(
            "{\"schema_version\":1,\"metadata_only\":false,\"empty\":false,\"partial\":false,"
        ),
        "{json}"
    );
    assert!(
        json.contains(&format!("\"name\":\"made\",\"child\":{made}")),
        "{json}"
    );

    // An image compared with itself is empty, and --metadata says so.
    let output = diff(&[
        std::ffi::OsStr::new("--metadata"),
        before.as_os_str(),
        before.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "no difference"
    );
}

#[test]
fn usage_and_damage_have_their_own_exit_statuses() {
    let temp = TempDir::new("status");
    let image = temp.join("volume.afsplus");
    format_image(&image);

    let output = diff(&[std::ffi::OsStr::new("--help")]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .starts_with("usage:"));

    let output = diff(&[image.as_os_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .starts_with("afsplus-image-diff: error[E_USAGE]:"));

    let output = diff(&[std::ffi::OsStr::new("--definitely-unknown")]);
    assert_eq!(output.status.code(), Some(2));

    // An image whose identification block is gone cannot be compared.
    let broken = temp.join("broken.afsplus");
    fs::write(&broken, vec![0u8; DEFAULT_BLOCK_SIZE * 8]).unwrap();
    let output = diff(&[image.as_os_str(), broken.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("identification"));
}
