//! `afsplus-disk`: an AFS+ image wrapped in a GPT disk comes back out byte
//! for byte, and a disk whose partition table does not check is refused.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("afsplus-disk-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn formatted(directory: &Path) -> PathBuf {
    let image = directory.join("volume.afsp");
    let status = afsplus_tools::run_mkafsplus(args(&[
        "--size-mib",
        "8",
        "--label",
        "Wrapped",
        image.to_str().unwrap(),
    ]));
    assert_eq!(status, 0);
    image
}

#[test]
fn a_wrapped_image_comes_back_out_unchanged() {
    let directory = scratch("roundtrip");
    let image = formatted(&directory);
    let disk = directory.join("disk.img");
    let back = directory.join("back.afsp");
    assert_eq!(
        afsplus_tools::run_disk(args(&[
            "wrap",
            "--bootpri",
            "5",
            disk.to_str().unwrap(),
            image.to_str().unwrap()
        ])),
        0
    );
    let disk_bytes = fs::read(&disk).unwrap();
    assert_eq!(&disk_bytes[512..520], b"EFI PART");
    assert_eq!(&disk_bytes[disk_bytes.len() - 512..][..8], b"EFI PART");
    assert_eq!(
        afsplus_tools::run_disk(args(&[
            "extract",
            disk.to_str().unwrap(),
            back.to_str().unwrap()
        ])),
        0
    );
    assert!(fs::read(&image).unwrap() == fs::read(&back).unwrap());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_disk_whose_partition_table_does_not_check_is_refused() {
    let directory = scratch("corrupt");
    let image = formatted(&directory);
    let disk = directory.join("disk.img");
    assert_eq!(
        afsplus_tools::run_disk(args(&[
            "wrap",
            disk.to_str().unwrap(),
            image.to_str().unwrap()
        ])),
        0
    );
    // One bit of the first entry's start sector.
    let mut bytes = fs::read(&disk).unwrap();
    bytes[1024 + 32] ^= 1;
    fs::write(&disk, &bytes).unwrap();
    let back = directory.join("back.afsp");
    assert_eq!(
        afsplus_tools::run_disk(args(&[
            "extract",
            disk.to_str().unwrap(),
            back.to_str().unwrap()
        ])),
        1
    );
    assert!(!back.exists());
    fs::remove_dir_all(&directory).unwrap();
}
