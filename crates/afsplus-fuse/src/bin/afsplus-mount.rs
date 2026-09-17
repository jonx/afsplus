use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::{MountMode, MountOptions};
use afsplus_format::ident::Identification;
use afsplus_format::DEFAULT_BLOCK_SIZE;
use afsplus_fuse::diagnostics::{self, Diagnostics};
use afsplus_fuse::fuser_adapter::FuserFilesystem;
use afsplus_fuse::{host_names, FuseConfig};
use afsplus_vfs::Vfs;
use fuser::{Config, MountOption, SessionACL};

/// How often the diagnostics report is rewritten while the volume is mounted.
/// A reader gets a report at most this old, without stopping the filesystem.
const DIAGNOSTICS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

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
    let (mode, image, mountpoint, diagnostics_path) = arguments()?;
    let (uid, gid) = mount_ownership(&image, &mountpoint)?;

    let device = open_image(&image)?;
    let mut vfs = Vfs::mount(
        device,
        MountOptions {
            mode,
            ..Default::default()
        },
    )
    .map_err(|error| format!("cannot mount {}: {error}", image.display()))?;
    // Observation is installed before the volume moves into the filesystem,
    // and only when asked for: an unobserved mount pays nothing.
    let observed = match &diagnostics_path {
        Some(path) => {
            let shared = diagnostics::install(&mut vfs)
                .map_err(|error| format!("cannot install diagnostics: {error}"))?;
            Some((Arc::clone(&shared), path.clone()))
        }
        None => None,
    };
    // Named after the committed label, read from the mounted volume.
    let names = host_names(&vfs);
    let filesystem = FuserFilesystem::new(
        vfs,
        FuseConfig {
            uid,
            gid,
            // macFUSE's FSKit backend can return from host fsync without
            // forwarding FUSE_FSYNC. Complete every data mutation durably so
            // that the missing transport notification cannot lose an
            // acknowledged host durability point.
            durable_data_replies: cfg!(all(target_os = "macos", feature = "macfuse-mount")),
            // The same backend sends removexattr(2) as a SETXATTR without
            // bytes and never sends REMOVEXATTR.
            empty_value_removes: cfg!(all(target_os = "macos", feature = "macfuse-mount")),
            ..FuseConfig::default()
        },
    );
    let mut config = Config::default();
    // FSKit performs mount-time control requests as root. Keep ordinary
    // requests limited to root and the mounting user; DefaultPermissions then
    // enforces the mode bits exported by AFS+.
    config.acl = SessionACL::RootAndOwner;
    config.mount_options = vec![
        MountOption::FSName(names.filesystem),
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

    let reporter = observed.map(|(shared, path)| Reporter::start(shared, path));

    let outcome = mount_session(filesystem, &mountpoint, config, &names.volume, mode);
    if let Some(reporter) = reporter {
        // The last report outlives the mount: what a volume did before it went
        // away is exactly what a person asks about afterwards.
        reporter.stop();
    }
    outcome.map_err(|error| {
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

/// Writes the mount's diagnostics where a person can read them, while the
/// volume is mounted and once more after it is gone.
///
/// The report is a file rather than a signal because `unsafe_code` is
/// forbidden across this workspace, so the driver installs no signal handler.
/// A file costs a reader nothing to find, survives the mount, and needs no
/// terminal: `cat` answers the question at any moment.
struct Reporter {
    shared: Arc<Diagnostics>,
    path: PathBuf,
    stopping: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

impl Reporter {
    fn start(shared: Arc<Diagnostics>, path: PathBuf) -> Self {
        eprintln!(
            "afsplus-mount: diagnostics written to {} every {} seconds; read it with cat",
            path.display(),
            DIAGNOSTICS_INTERVAL.as_secs()
        );
        let stopping = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let shared = Arc::clone(&shared);
            let stopping = Arc::clone(&stopping);
            let path = path.clone();
            move || {
                while !stopping.load(Ordering::Relaxed) {
                    write_report(&path, &shared.report());
                    std::thread::sleep(DIAGNOSTICS_INTERVAL);
                }
            }
        });
        Self {
            shared,
            path,
            stopping,
            thread,
        }
    }

    fn stop(self) {
        self.stopping.store(true, Ordering::Relaxed);
        // A failed join means the reporting thread died; the mount does not
        // depend on it, and the final report is still written here.
        let _ = self.thread.join();
        write_report(&self.path, &self.shared.report());
    }
}

/// Replaces the report in one step, so a reader never sees half of one.
/// A diagnostics failure is reported and never fails the mount: the volume is
/// the thing the user needs, and losing its report must not lose it.
fn write_report(path: &Path, report: &str) {
    let temporary = path.with_extension("writing");
    if let Err(error) =
        std::fs::write(&temporary, report).and_then(|()| std::fs::rename(&temporary, path))
    {
        eprintln!(
            "afsplus-mount: cannot write diagnostics to {}: {error}",
            path.display()
        );
    }
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

fn arguments() -> Result<(MountMode, PathBuf, PathBuf, Option<PathBuf>), String> {
    let mut mode = MountMode::ReadWrite;
    let mut diagnostics = None;
    let mut paths = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--read-only" => mode = MountMode::ReadOnly,
            "--no-changes" => mode = MountMode::NoChanges,
            "--recovery" => mode = MountMode::Recovery,
            "--help" | "-h" => return Err(usage().into()),
            value if value.starts_with("--diagnostics=") => {
                let path = value.trim_start_matches("--diagnostics=");
                if path.is_empty() {
                    return Err(format!("--diagnostics needs a path\n{}", usage()));
                }
                diagnostics = Some(PathBuf::from(path));
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option {value}\n{}", usage()));
            }
            _ => paths.push(PathBuf::from(argument)),
        }
    }
    if paths.len() != 2 {
        return Err(usage().into());
    }
    Ok((mode, paths.remove(0), paths.remove(0), diagnostics))
}

fn usage() -> &'static str {
    "usage: afsplus-mount [--read-only|--no-changes|--recovery] \
     [--diagnostics=<file>] <image> <mountpoint>"
}

fn open_image(path: &Path) -> Result<FileBackend, String> {
    let mut device = FileBackend::open_sized_by_file(path, DEFAULT_BLOCK_SIZE)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut block = vec![0; DEFAULT_BLOCK_SIZE];
    device
        .read_block(0, &mut block)
        .map_err(|error| format!("cannot read identification block: {error}"))?;
    let identification = Identification::decode(&block)
        .map_err(|error| format!("invalid identification block: {error}"))?;
    device.set_total_blocks(identification.total_blocks);
    Ok(device)
}
