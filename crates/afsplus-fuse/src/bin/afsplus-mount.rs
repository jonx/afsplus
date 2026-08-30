use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::{MountMode, MountOptions};
use afsplus_format::ident::Identification;
use afsplus_format::DEFAULT_BLOCK_SIZE;
use afsplus_fuse::fuser_adapter::FuserFilesystem;
use afsplus_fuse::FuseConfig;
use afsplus_vfs::Vfs;
use fuser::{Config, MountOption};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-mount: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let (mode, image, mountpoint) = arguments()?;
    let metadata = std::fs::metadata(&mountpoint)
        .map_err(|error| format!("cannot inspect {}: {error}", mountpoint.display()))?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", mountpoint.display()));
    }

    let (device, identification) = open_image(&image)?;
    let vfs = Vfs::mount(device, MountOptions { mode })
        .map_err(|error| format!("cannot mount {}: {error}", image.display()))?;
    let filesystem = FuserFilesystem::new(
        vfs,
        FuseConfig {
            uid: metadata.uid(),
            gid: metadata.gid(),
            ..FuseConfig::default()
        },
    );
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName(format!("afsplus: {}", identification.label)),
        MountOption::Subtype("afsplus".into()),
        MountOption::DefaultPermissions,
        MountOption::NoDev,
        MountOption::NoSuid,
        MountOption::NoAtime,
        if mode == MountMode::ReadWrite {
            MountOption::RW
        } else {
            MountOption::RO
        },
    ];

    fuser::mount(filesystem, &mountpoint, &config).map_err(|error| {
        if cfg!(target_os = "macos") {
            format!(
                "FUSE mount failed: {error}. This build validates the adapter on macOS but needs the macFUSE/Fuse-T native mount bridge"
            )
        } else {
            format!("FUSE mount failed: {error}")
        }
    })
}

fn arguments() -> Result<(MountMode, PathBuf, PathBuf), String> {
    let mut mode = MountMode::ReadWrite;
    let mut paths = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--read-only" => mode = MountMode::ReadOnly,
            "--no-changes" => mode = MountMode::NoChanges,
            "--recovery" => mode = MountMode::Recovery,
            "--help" | "-h" => return Err(usage().into()),
            value if value.starts_with('-') => {
                return Err(format!("unknown option {value}\n{}", usage()));
            }
            _ => paths.push(PathBuf::from(argument)),
        }
    }
    if paths.len() != 2 {
        return Err(usage().into());
    }
    Ok((mode, paths.remove(0), paths.remove(0)))
}

fn usage() -> &'static str {
    "usage: afsplus-mount [--read-only|--no-changes|--recovery] <image> <mountpoint>"
}

fn open_image(path: &Path) -> Result<(FileBackend, Identification), String> {
    let mut device = FileBackend::open_sized_by_file(path, DEFAULT_BLOCK_SIZE)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut block = vec![0; DEFAULT_BLOCK_SIZE];
    device
        .read_block(0, &mut block)
        .map_err(|error| format!("cannot read identification block: {error}"))?;
    let identification = Identification::decode(&block)
        .map_err(|error| format!("invalid identification block: {error}"))?;
    device.set_total_blocks(identification.total_blocks);
    Ok((device, identification))
}
