use std::io;
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
use nix::unistd::{getegid, geteuid};

/// How often the diagnostics report is rewritten while the volume is mounted.
/// A reader gets a report at most this old, without stopping the filesystem.
const DIAGNOSTICS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Set in the environment of the process that serves the mount, so that it
/// knows it is the supervised one and not the supervisor.
#[cfg(all(target_os = "macos", feature = "macfuse-mount"))]
const SERVE_ENV: &str = "AFSPLUS_MOUNT_SERVE";

fn main() -> ExitCode {
    #[cfg(all(target_os = "macos", feature = "macfuse-mount"))]
    if std::env::var_os(SERVE_ENV).is_none() {
        return supervisor::run();
    }
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
    let (uid, gid) = mount_ownership(&mountpoint)?;

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
    if let Err(error) = native_mount.wait_until_mounted(std::time::Duration::from_secs(30)) {
        drop(native_mount);
        // After a refused mount the request loop can stay inside macFUSE
        // too; the process ends without it.
        if error.kind() != io::ErrorKind::TimedOut {
            let _ = background.join();
        }
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

/// The volume belongs to whoever mounted it.
///
/// It used to take the owner from the mountpoint, or from the image file when
/// the mountpoint did not exist yet. Both read a group from wherever the file
/// or directory happened to sit: an image under a home directory gave the
/// volume `staff`, the same image under `/tmp` gave it `wheel`. The host then
/// asks, after every create, for the group the creating process actually has,
/// and the driver answered that it could not. On a volume whose image sat in
/// `/tmp`, every mkdir and every new file reported "Operation not supported"
/// while quietly succeeding, and a whole battery of ordinary use collapsed for
/// a reason that had nothing to do with the filesystem.
///
/// The mount is already `noowners`, which means exactly this: the volume is
/// the mounting user's. Reading it from the process says so once, and says the
/// same thing wherever the image lives.
fn mount_ownership(mountpoint: &Path) -> Result<(u32, u32), String> {
    match std::fs::metadata(mountpoint) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(format!("{} is not a directory", mountpoint.display())),
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && missing_mountpoint_is_supported(mountpoint) => {}
        Err(error) => {
            return Err(format!("cannot inspect {}: {error}", mountpoint.display()));
        }
    }
    Ok((geteuid().as_raw(), getegid().as_raw()))
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

/// Keeps a dead filesystem process from leaving a dead mount behind.
///
/// macFUSE's FSKit backend relays every request from the kernel to this
/// driver through a helper process, one per mount. If the driver dies while a
/// request is in flight, that helper waits for an answer that never comes:
/// the program that made the request cannot even be killed, `umount` blocks
/// the same way, and the mountpoint is left out of the mount table with every
/// `stat` on it blocking. Nothing cleared it but a reboot. A client killed on
/// its own does no harm; it takes the driver dying mid-request.
///
/// So `afsplus-mount` runs as a supervisor, and the process serving the volume
/// is its child. The supervisor notes which relay appeared when the volume
/// came up, and if the child ends in any way other than a clean unmount it
/// terminates that relay. Everything waiting on the volume is then released
/// with an error, which is what a person expects from a disk that went away.
///
/// It also turns Ctrl-C, `kill` and a closing terminal into a clean unmount.
/// The child is started with those signals blocked, so it can only end through
/// an unmount or something no process can refuse, and never with a request
/// in flight because somebody asked it to stop.
#[cfg(all(target_os = "macos", feature = "macfuse-mount"))]
mod supervisor {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitCode};
    use std::time::{Duration, Instant};

    use nix::sys::signal::{kill, SigSet, Signal};
    use nix::unistd::Pid;

    /// The executable name of macFUSE's per-mount FSKit relay.
    const RELAY: &str = "io.macfuse.app.fsmodule.macfuse";

    pub fn run() -> ExitCode {
        let mut stops = SigSet::empty();
        for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
            stops.add(signal);
        }
        // Before any thread exists, so every later thread and the child inherit it.
        if let Err(error) = stops.thread_block() {
            eprintln!("afsplus-mount: cannot take charge of stop signals: {error}");
            return ExitCode::from(1);
        }
        let mountpoint: PathBuf = match super::arguments() {
            Ok((_, _, mountpoint, _)) => mountpoint,
            Err(error) => {
                eprintln!("afsplus-mount: {error}");
                return ExitCode::from(1);
            }
        };
        let listed = listed_path(&mountpoint);
        let before = relays();
        let executable = match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                eprintln!("afsplus-mount: cannot find this program to start the volume: {error}");
                return ExitCode::from(1);
            }
        };
        let mut child = match Command::new(executable)
            .args(std::env::args_os().skip(1))
            .env(super::SERVE_ENV, "1")
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                eprintln!("afsplus-mount: cannot start the volume: {error}");
                return ExitCode::from(1);
            }
        };

        // A person stopping the mount asks for an unmount, which the child
        // answers like any other and then ends cleanly. The listed spelling,
        // because a path through a symlink stops resolving once a mount dies.
        let target = listed.clone();
        std::thread::spawn(move || loop {
            if stops.wait().is_ok() {
                let _ = Command::new("umount").arg(&target).status();
            }
        });

        let relay = identify_relay(&before, &mut child);
        let status = wait_while_served(&mut child, relay);
        if matches!(&status, Ok(status) if status.success()) {
            return ExitCode::SUCCESS;
        }
        match &status {
            Ok(status) => match status.code() {
                Some(code) => {
                    eprintln!("afsplus-mount: the volume process ended with status {code}")
                }
                None => eprintln!(
                    "afsplus-mount: the volume process was killed while the volume was mounted"
                ),
            },
            Err(error) => eprintln!("afsplus-mount: lost track of the volume process: {error}"),
        }
        unmount_dead(&listed, relay);
        ExitCode::from(1)
    }

    /// Remove a mount whose process is gone, and release whatever waits on it.
    ///
    /// The unmount comes first and the relay goes only if it blocks. An
    /// unmount goes through macOS's file system daemon, which also removes the
    /// mount from the list of mounts it keeps on disk. Terminating the relay
    /// first made the kernel drop the mount without that, and the entry left
    /// behind made macOS refuse every later mount at the same path.
    ///
    /// The unmount blocks when a request was in flight as the process died:
    /// the relay still waits for its answer. Terminating the relay then
    /// releases the waiting programs with an I/O error and lets the unmount
    /// finish. By the listed spelling, because through `/tmp`, which is a
    /// symlink, umount no longer finds a mount whose process is gone.
    fn unmount_dead(listed: &Path, relay: Option<i32>) {
        let Ok(mut unmount) = Command::new("umount")
            .arg(listed)
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            if let Some(pid) = relay {
                release(pid);
            }
            return;
        };
        if !ends_within(&mut unmount, Duration::from_secs(2)) {
            if let Some(pid) = relay {
                release(pid);
            }
            if !ends_within(&mut unmount, Duration::from_secs(15)) {
                eprintln!("afsplus-mount: the volume could not be unmounted");
                let _ = unmount.kill();
            }
        }
        // A relay the unmount did not end is released too, so that nothing
        // outlives the volume.
        if let Some(pid) = relay {
            release(pid);
        }
    }

    fn ends_within(child: &mut Child, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        matches!(child.try_wait(), Ok(Some(_)))
    }

    /// Wait for the volume process, and stop it once nothing can reach it.
    ///
    /// When macOS refuses a mount, it terminates the relay, and macFUSE's
    /// mount call reports the failure and then never returns: the process
    /// would wait for ever with nothing mounted. A relay that is gone while
    /// the process lives means the volume can no longer be served, whatever
    /// the cause. The process gets time to finish on its own first, because a
    /// clean unmount also ends the relay, a moment before the process has
    /// written its last state.
    fn wait_while_served(
        child: &mut Child,
        relay: Option<i32>,
    ) -> std::io::Result<std::process::ExitStatus> {
        let Some(relay) = relay else {
            return child.wait();
        };
        let relay = Pid::from_raw(relay);
        let mut orphaned_since: Option<Instant> = None;
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            if orphaned_since.is_none() && kill(relay, None).is_err() {
                orphaned_since = Some(Instant::now());
            }
            if orphaned_since.is_some_and(|since| since.elapsed() > Duration::from_secs(10)) {
                eprintln!(
                    "afsplus-mount: macOS ended or refused the mount; stopping the volume process"
                );
                let _ = child.kill();
                return child.wait();
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// The path as the mount table spells it: `/tmp` is `/private/tmp`.
    fn listed_path(mountpoint: &Path) -> PathBuf {
        if let Ok(path) = mountpoint.canonicalize() {
            return path;
        }
        match (mountpoint.parent(), mountpoint.file_name()) {
            (Some(parent), Some(name)) => parent
                .canonicalize()
                .map(|parent| parent.join(name))
                .unwrap_or_else(|_| mountpoint.to_path_buf()),
            _ => mountpoint.to_path_buf(),
        }
    }

    /// Processes whose EXECUTABLE is the relay. Matched on the executable
    /// path, never on the command line: a shell that merely mentions the name
    /// matches a command-line search, and this list decides what gets killed.
    fn relays() -> BTreeSet<i32> {
        let Ok(out) = Command::new("ps").args(["-axo", "pid=,comm="]).output() else {
            return BTreeSet::new();
        };
        let suffix = format!("/{RELAY}");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, executable) = line.trim_start().split_once(char::is_whitespace)?;
                let executable = executable.trim();
                (executable == RELAY || executable.ends_with(&suffix))
                    .then(|| pid.parse().ok())
                    .flatten()
            })
            .collect()
    }

    /// The relay that appeared while this volume came up. One per mount, so
    /// the new one is this volume's; if two appeared at once another mount
    /// raced this one and no guess is made.
    ///
    /// It watches the process list and never lists mounts. `mount` asks every
    /// mounted filesystem for its state, so a dead mount anywhere on the
    /// machine blocks it for ever, and this is exactly the code that must
    /// still work when one exists.
    fn identify_relay(before: &BTreeSet<i32>, child: &mut Child) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return None;
            }
            let fresh: Vec<i32> = relays().difference(before).copied().collect();
            match fresh.as_slice() {
                [] => {}
                [pid] => return Some(*pid),
                _ => {
                    eprintln!("afsplus-mount: several volumes came up at once; this one will not be released automatically if its process dies");
                    return None;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    fn release(pid: i32) {
        let pid = Pid::from_raw(pid);
        if kill(pid, None).is_err() {
            return;
        }
        eprintln!("afsplus-mount: releasing the macFUSE relay (pid {pid}) so that programs waiting on the volume are not left blocked");
        let _ = kill(pid, Signal::SIGTERM);
        // Time to finish on its own: a relay that ends normally tells macOS
        // to forget the mount, and one killed before it has done so leaves
        // the mount recorded, which refuses the next mount at that path.
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if kill(pid, None).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = kill(pid, Signal::SIGKILL);
    }
}
