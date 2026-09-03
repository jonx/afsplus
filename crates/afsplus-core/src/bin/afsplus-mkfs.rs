//! Create a bounded AFS+ image file for mounting or conformance tests.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use afsplus_block::FileBackend;
use afsplus_core::{mkfs, MkfsParams, NamePolicy};
use afsplus_format::{geometry::MAX_REGION_BLOCKS, Timespec, DEFAULT_BLOCK_SIZE};

const DEFAULT_SIZE_MIB: u64 = 64;
const BLOCKS_PER_MIB: u64 = 1024 * 1024 / DEFAULT_BLOCK_SIZE as u64;

struct Options {
    path: PathBuf,
    label: String,
    size_mib: u64,
    force: bool,
    name_policy: NamePolicy,
}

fn usage() -> ! {
    eprintln!(
        "usage: afsplus-mkfs [--size-mib N] [--label NAME] \
         [--case-sensitive|--case-insensitive] [--force] <image>"
    );
    std::process::exit(2);
}

fn parse_options() -> Options {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut label = "AFSPlus".to_owned();
    let mut size_mib = DEFAULT_SIZE_MIB;
    let mut force = false;
    let mut name_policy = NamePolicy::Sensitive;
    let mut case_option_seen = false;

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--size-mib" => {
                size_mib = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| *value != 0)
                    .unwrap_or_else(|| usage());
            }
            "--label" => label = args.next().unwrap_or_else(|| usage()),
            "--force" => force = true,
            "--case-sensitive" | "--case-insensitive" => {
                if case_option_seen {
                    usage();
                }
                case_option_seen = true;
                name_policy = if argument == "--case-insensitive" {
                    NamePolicy::Insensitive
                } else {
                    NamePolicy::Sensitive
                };
            }
            "-h" | "--help" => usage(),
            _ if argument.starts_with('-') || path.is_some() => usage(),
            _ => path = Some(PathBuf::from(argument)),
        }
    }
    Options {
        path: path.unwrap_or_else(|| usage()),
        label,
        size_mib,
        force,
        name_policy,
    }
}

fn timestamp() -> Result<(u128, Timespec), String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    let seconds = i64::try_from(duration.as_secs())
        .map_err(|_| "system clock does not fit the AFS+ timestamp".to_owned())?;
    Ok((
        duration.as_nanos(),
        Timespec {
            seconds,
            nanoseconds: duration.subsec_nanos(),
        },
    ))
}

fn volume_uuid(nanos: u128, path: &Path) -> [u8; 16] {
    let mut uuid = nanos.to_le_bytes();
    for (index, byte) in path.as_os_str().as_encoded_bytes().iter().enumerate() {
        uuid[index % uuid.len()] ^= *byte;
    }
    let process = std::process::id().to_le_bytes();
    for (index, byte) in process.iter().enumerate() {
        uuid[index + 8] ^= *byte;
    }
    uuid
}

fn run(options: Options) -> Result<(), String> {
    if options.path.exists() && !options.force {
        return Err(format!(
            "refusing to replace existing image {} (pass --force explicitly)",
            options.path.display()
        ));
    }
    let total_blocks = options
        .size_mib
        .checked_mul(BLOCKS_PER_MIB)
        .ok_or_else(|| "requested image size overflows the block count".to_owned())?;
    let image_bytes = total_blocks
        .checked_mul(DEFAULT_BLOCK_SIZE as u64)
        .ok_or_else(|| "requested image size overflows the host file length".to_owned())?;
    let (nanos, now) = timestamp()?;
    let mut device = FileBackend::create(&options.path, DEFAULT_BLOCK_SIZE, total_blocks)
        .map_err(|error| format!("cannot create {}: {error}", options.path.display()))?;
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: volume_uuid(nanos, &options.path),
            label: options.label.clone(),
            region_size: u32::try_from(total_blocks)
                .unwrap_or(MAX_REGION_BLOCKS)
                .min(MAX_REGION_BLOCKS),
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: options.name_policy,
            timestamp: now,
        },
    )
    .map_err(|error| format!("cannot format {}: {error}", options.path.display()))?;
    drop(device);

    // FileBackend deliberately permits a short sparse tail. A portable image
    // container must nevertheless advertise its complete device geometry to
    // host file-backed drivers such as MacAROS fdsk.device.
    let image = OpenOptions::new()
        .write(true)
        .open(&options.path)
        .map_err(|error| format!("cannot reopen {}: {error}", options.path.display()))?;
    image
        .set_len(image_bytes)
        .and_then(|()| image.sync_all())
        .map_err(|error| {
            format!(
                "cannot persist the container length for {}: {error}",
                options.path.display()
            )
        })?;
    println!(
        "formatted {} as {:?}: {} MiB, {} blocks of {} bytes, {:?} names",
        options.path.display(),
        options.label,
        options.size_mib,
        total_blocks,
        DEFAULT_BLOCK_SIZE,
        options.name_policy
    );
    Ok(())
}

fn main() -> ExitCode {
    match run(parse_options()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-mkfs: {error}");
            ExitCode::FAILURE
        }
    }
}
