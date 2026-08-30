use std::io;
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
use fuser::{Config, MountOption, SessionACL};

fn main() -> ExitCode {
    init_logging();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-mount: {error}");
            ExitCode::from(1)
        }
    }
}

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{} {}: {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

fn init_logging() {
    let level = match std::env::var("AFSPLUS_LOG").as_deref() {
        Ok("trace") => log::LevelFilter::Trace,
        Ok("debug") => log::LevelFilter::Debug,
        Ok("info") => log::LevelFilter::Info,
        Ok("warn") => log::LevelFilter::Warn,
        _ => log::LevelFilter::Error,
    };
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

fn run() -> Result<(), String> {
    let (mode, image, mountpoint) = arguments()?;
    let (uid, gid) = mount_ownership(&image, &mountpoint)?;

    let (device, identification) = open_image(&image)?;
    let vfs = Vfs::mount(device, MountOptions { mode })
        .map_err(|error| format!("cannot mount {}: {error}", image.display()))?;
    let filesystem = FuserFilesystem::new(
        vfs,
        FuseConfig {
            uid,
            gid,
            ..FuseConfig::default()
        },
    );
    let mut config = Config::default();
    // FSKit performs mount-time control requests as root. Keep ordinary
    // requests limited to root and the mounting user; DefaultPermissions then
    // enforces the mode bits exported by AFS+.
    config.acl = SessionACL::RootAndOwner;
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

    mount_session(filesystem, &mountpoint, config, &identification.label, mode).map_err(|error| {
        #[cfg(all(target_os = "macos", not(feature = "macfuse-mount")))]
        return format!(
            "FUSE mount failed: {error}. Install macFUSE and rebuild with --features macfuse-mount"
        );
        #[cfg(not(all(target_os = "macos", not(feature = "macfuse-mount"))))]
        format!("FUSE mount failed: {error}")
    })
}

#[cfg(all(target_os = "macos", feature = "macfuse-mount"))]
fn mount_session(
    filesystem: FuserFilesystem<FileBackend>,
    mountpoint: &Path,
    config: Config,
    label: &str,
    mode: MountMode,
) -> io::Result<()> {
    let options = vec![
        "backend=fskit".to_string(),
        "fsname=afsplus".to_string(),
        format!("volname={label}"),
        if mode == MountMode::ReadWrite {
            "rw".to_string()
        } else {
            "ro".to_string()
        },
        "nodev".to_string(),
        "nosuid".to_string(),
        "default_permissions".to_string(),
    ];
    let mut native_mount = afsplus_macfuse_sys::MacFuseMount::new(mountpoint, &options)?;
    let transport = native_mount.transport();
    let session = match fuser::Session::from_transport(filesystem, transport, config.acl, config) {
        Ok(session) => session,
        Err(error) => {
            drop(native_mount);
            return Err(error);
        }
    };
    // MFMount completes only after FSKit has queried the freshly initialized
    // filesystem (notably with STATFS). Start the request loop before waiting
    // for the mount operation, otherwise both sides wait for each other.
    let background = session.spawn()?;
    if let Err(error) = native_mount.wait_until_mounted() {
        drop(native_mount);
        let _ = background.join();
        return Err(error);
    }
    let result = background.join();
    drop(native_mount);
    result
}

fn mount_ownership(image: &Path, mountpoint: &Path) -> Result<(u32, u32), String> {
    match std::fs::metadata(mountpoint) {
        Ok(metadata) if metadata.is_dir() => Ok((metadata.uid(), metadata.gid())),
        Ok(_) => Err(format!("{} is not a directory", mountpoint.display())),
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && missing_mountpoint_is_supported(mountpoint) =>
        {
            let metadata = std::fs::metadata(image)
                .map_err(|error| format!("cannot inspect {}: {error}", image.display()))?;
            Ok((metadata.uid(), metadata.gid()))
        }
        Err(error) => Err(format!("cannot inspect {}: {error}", mountpoint.display())),
    }
}

#[cfg(all(target_os = "macos", feature = "macfuse-mount"))]
fn missing_mountpoint_is_supported(mountpoint: &Path) -> bool {
    mountpoint.parent() == Some(Path::new("/Volumes")) && mountpoint.file_name().is_some()
}

#[cfg(not(all(target_os = "macos", feature = "macfuse-mount")))]
fn missing_mountpoint_is_supported(_mountpoint: &Path) -> bool {
    false
}

#[cfg(not(all(target_os = "macos", feature = "macfuse-mount")))]
fn mount_session(
    filesystem: FuserFilesystem<FileBackend>,
    mountpoint: &Path,
    config: Config,
    _label: &str,
    _mode: MountMode,
) -> io::Result<()> {
    fuser::mount(filesystem, mountpoint, &config)
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
