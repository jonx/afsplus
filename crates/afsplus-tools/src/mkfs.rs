use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use afsplus_block::{BlockError, FileBackend};
use afsplus_core::{mkfs, CoreError, MkfsParams, NamePolicy};
use afsplus_format::geometry::MAX_REGION_BLOCKS;
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE};

use crate::common::{json_string, json_string_list, report_failure, uuid_hex, Failure, EXIT_OK};

const TOOL: &str = "mkafsplus";
const USAGE: &str = "usage: mkafsplus [--size-mib N] [--label NAME] [--profile PROFILE] [--case-sensitive|--case-insensitive] [--uuid HEX] [--timestamp-seconds N] [--json] [--force] <image>";
const DEFAULT_SIZE_MIB: u64 = 64;
const BLOCKS_PER_MIB: u64 = 1024 * 1024 / DEFAULT_BLOCK_SIZE as u64;
const LOG_SLOTS: u16 = 8;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum Profile {
    ReaderMinimal,
    ClassicRw,
    BootSafe,
    Workstation,
    Full,
}

impl Profile {
    fn parse(value: &str) -> Result<Self, Failure> {
        match value {
            "reader-minimal" => Ok(Self::ReaderMinimal),
            "classic-rw" => Ok(Self::ClassicRw),
            "boot-safe" => Ok(Self::BootSafe),
            "workstation" => Ok(Self::Workstation),
            "full" => Ok(Self::Full),
            _ => Err(Failure::usage(format!(
                "unknown profile {value:?}; expected reader-minimal, classic-rw, boot-safe, workstation, or full"
            ))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ReaderMinimal => "reader-minimal",
            Self::ClassicRw => "classic-rw",
            Self::BootSafe => "boot-safe",
            Self::Workstation => "workstation",
            Self::Full => "full",
        }
    }

    fn shared_extents(self) -> bool {
        matches!(self, Self::Workstation | Self::Full)
    }

    fn data_policy(self) -> bool {
        matches!(self, Self::Workstation | Self::Full)
    }

    fn orphan_directory(self) -> bool {
        true
    }
}

struct Options {
    image: PathBuf,
    label: String,
    size_mib: u64,
    force: bool,
    json: bool,
    profile: Profile,
    name_policy: NamePolicy,
    uuid: Option<[u8; 16]>,
    timestamp_seconds: Option<i64>,
}

enum ParseResult {
    Options(Options),
    Help,
}

struct FormatSummary {
    uuid: [u8; 16],
    label: String,
    total_blocks: u64,
    profile: Profile,
    name_policy: NamePolicy,
}

pub fn run<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    let options = match parse(args) {
        Ok(ParseResult::Options(options)) => options,
        Ok(ParseResult::Help) => {
            println!("{USAGE}");
            println!(
                "Profiles: reader-minimal, classic-rw, boot-safe, workstation (default), full."
            );
            return EXIT_OK;
        }
        Err(failure) => return report_failure(TOOL, failure),
    };
    match execute(&options) {
        Ok(summary) => {
            print_summary(&summary, options.json);
            EXIT_OK
        }
        Err(failure) => report_failure(TOOL, failure),
    }
}

fn parse<I>(args: I) -> Result<ParseResult, Failure>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let mut image = None;
    let mut label = "AFSPlus".to_owned();
    let mut size_mib = DEFAULT_SIZE_MIB;
    let mut force = false;
    let mut json = false;
    let mut profile = Profile::Workstation;
    let mut name_policy = NamePolicy::Sensitive;
    let mut name_policy_seen = false;
    let mut uuid = None;
    let mut timestamp_seconds = None;
    let mut positional_only = false;

    while let Some(argument) = args.next() {
        if !positional_only {
            match argument.to_str() {
                Some("--help" | "-h") => return Ok(ParseResult::Help),
                Some("--") => {
                    positional_only = true;
                    continue;
                }
                Some("--size-mib") => {
                    let value = utf8_value(&mut args, "--size-mib")?;
                    size_mib = value
                        .parse()
                        .ok()
                        .filter(|value| *value != 0)
                        .ok_or_else(|| {
                            Failure::usage("--size-mib requires a positive base-10 integer")
                        })?;
                    continue;
                }
                Some("--label") => {
                    label = utf8_value(&mut args, "--label")?;
                    continue;
                }
                Some("--profile") => {
                    profile = Profile::parse(&utf8_value(&mut args, "--profile")?)?;
                    continue;
                }
                Some("--uuid") => {
                    if uuid.is_some() {
                        return Err(Failure::usage("--uuid was specified more than once"));
                    }
                    uuid = Some(parse_uuid(&utf8_value(&mut args, "--uuid")?)?);
                    continue;
                }
                Some("--timestamp-seconds") => {
                    if timestamp_seconds.is_some() {
                        return Err(Failure::usage(
                            "--timestamp-seconds was specified more than once",
                        ));
                    }
                    let value = utf8_value(&mut args, "--timestamp-seconds")?;
                    timestamp_seconds = Some(value.parse().map_err(|_| {
                        Failure::usage("--timestamp-seconds requires a signed base-10 integer")
                    })?);
                    continue;
                }
                Some("--force") => {
                    if force {
                        return Err(Failure::usage("--force was specified more than once"));
                    }
                    force = true;
                    continue;
                }
                Some("--json") => {
                    if json {
                        return Err(Failure::usage("--json was specified more than once"));
                    }
                    json = true;
                    continue;
                }
                Some("--case-sensitive" | "--case-insensitive") => {
                    if name_policy_seen {
                        return Err(Failure::usage(
                            "choose exactly one of --case-sensitive and --case-insensitive",
                        ));
                    }
                    name_policy_seen = true;
                    name_policy = if argument == OsStr::new("--case-insensitive") {
                        NamePolicy::Insensitive
                    } else {
                        NamePolicy::Sensitive
                    };
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(Failure::usage(format!("unknown option {value:?}; {USAGE}")));
                }
                _ => {}
            }
        }
        if image.replace(PathBuf::from(&argument)).is_some() {
            return Err(Failure::usage(format!(
                "exactly one image path is required; {USAGE}"
            )));
        }
    }

    if label.len() > afsplus_format::ident::LABEL_MAX_BYTES {
        return Err(Failure::usage(format!(
            "volume label is longer than {} UTF-8 bytes",
            afsplus_format::ident::LABEL_MAX_BYTES
        )));
    }
    let image = image.ok_or_else(|| Failure::usage(format!("missing image path; {USAGE}")))?;
    Ok(ParseResult::Options(Options {
        image,
        label,
        size_mib,
        force,
        json,
        profile,
        name_policy,
        uuid,
        timestamp_seconds,
    }))
}

fn utf8_value<I>(args: &mut I, option: &str) -> Result<String, Failure>
where
    I: Iterator<Item = OsString>,
{
    let value = args
        .next()
        .ok_or_else(|| Failure::usage(format!("{option} requires a value")))?;
    value
        .into_string()
        .map_err(|_| Failure::usage(format!("{option} requires a UTF-8 value")))
}

fn parse_uuid(value: &str) -> Result<[u8; 16], Failure> {
    let compact: String = value
        .chars()
        .filter(|character| *character != '-')
        .collect();
    if compact.len() != 32 || !compact.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Failure::usage(
            "--uuid requires 32 hexadecimal digits (hyphens are optional)",
        ));
    }
    let mut uuid = [0u8; 16];
    for (index, byte) in uuid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16)
            .expect("validated hexadecimal UUID");
    }
    Ok(uuid)
}

fn clock() -> Result<(u128, Timespec), Failure> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            Failure::host_io(format!("system clock is before the Unix epoch: {error}"))
        })?;
    let seconds = i64::try_from(duration.as_secs())
        .map_err(|_| Failure::host_io("system clock does not fit an AFS+ timestamp"))?;
    Ok((
        duration.as_nanos(),
        Timespec {
            seconds,
            nanoseconds: duration.subsec_nanos(),
        },
    ))
}

fn generated_uuid(nanos: u128, path: &Path) -> [u8; 16] {
    let mut uuid = nanos.to_le_bytes();
    for (index, byte) in path.as_os_str().as_encoded_bytes().iter().enumerate() {
        uuid[index % uuid.len()] ^= *byte;
    }
    for (index, byte) in std::process::id().to_le_bytes().iter().enumerate() {
        uuid[index + 8] ^= *byte;
    }
    uuid
}

struct TempImage {
    path: PathBuf,
    published: bool,
}

impl TempImage {
    fn reserve(target: &Path, total_blocks: u64) -> Result<(Self, FileBackend), Failure> {
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = target.file_name().ok_or_else(|| {
            Failure::usage(format!("image path {} has no file name", target.display()))
        })?;
        for _ in 0..64 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{}.mkafsplus-{}-{sequence}.tmp",
                name.to_string_lossy(),
                std::process::id()
            ));
            match FileBackend::create_new(&candidate, DEFAULT_BLOCK_SIZE, total_blocks) {
                Ok(device) => {
                    return Ok((
                        Self {
                            path: candidate,
                            published: false,
                        },
                        device,
                    ));
                }
                Err(BlockError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(error) => {
                    return Err(Failure::host_io(format!(
                        "cannot create temporary image beside {}: {error}",
                        target.display()
                    )));
                }
            }
        }
        Err(Failure::host_io(format!(
            "cannot reserve a unique temporary image beside {}",
            target.display()
        )))
    }

    fn publish(mut self, target: &Path, force: bool) -> Result<(), Failure> {
        if target.exists() && !force {
            return Err(Failure {
                exit: crate::common::EXIT_USAGE_OR_IO,
                id: "E_EXISTS",
                message: format!(
                    "refusing to replace {} (pass --force explicitly)",
                    target.display()
                ),
            });
        }
        if !force {
            fs::hard_link(&self.path, target).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    Failure {
                        exit: crate::common::EXIT_USAGE_OR_IO,
                        id: "E_EXISTS",
                        message: format!(
                            "refusing to replace {} (pass --force explicitly)",
                            target.display()
                        ),
                    }
                } else {
                    Failure::host_io(format!(
                        "cannot publish formatted image as {}: {error}",
                        target.display()
                    ))
                }
            })?;
            fs::remove_file(&self.path).map_err(|error| {
                Failure::host_io(format!(
                    "image was published as {}, but its temporary link could not be removed: {error}",
                    target.display()
                ))
            })?;
            self.published = true;
            return Ok(());
        }
        #[cfg(windows)]
        if target.exists() {
            // Windows rename does not replace an existing destination. The
            // explicit --force authorization permits this platform-specific
            // non-atomic final step; formatting itself still happens off-path.
            fs::remove_file(target).map_err(|error| {
                Failure::host_io(format!("cannot remove {}: {error}", target.display()))
            })?;
        }
        fs::rename(&self.path, target).map_err(|error| {
            Failure::host_io(format!(
                "cannot publish formatted image as {}: {error}",
                target.display()
            ))
        })?;
        self.published = true;
        Ok(())
    }
}

impl Drop for TempImage {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn execute(options: &Options) -> Result<FormatSummary, Failure> {
    if options.image.exists() && !options.force {
        return Err(Failure {
            exit: crate::common::EXIT_USAGE_OR_IO,
            id: "E_EXISTS",
            message: format!(
                "refusing to replace {} (pass --force explicitly)",
                options.image.display()
            ),
        });
    }
    let total_blocks = options
        .size_mib
        .checked_mul(BLOCKS_PER_MIB)
        .ok_or_else(|| Failure::usage("requested size overflows the block count"))?;
    let image_bytes = total_blocks
        .checked_mul(DEFAULT_BLOCK_SIZE as u64)
        .ok_or_else(|| Failure::usage("requested size overflows the host file length"))?;
    let (nanos, clock_now) = clock()?;
    let now = options
        .timestamp_seconds
        .map_or(clock_now, |seconds| Timespec {
            seconds,
            nanoseconds: 0,
        });
    let uuid = options
        .uuid
        .unwrap_or_else(|| generated_uuid(nanos, &options.image));
    let (temporary, mut device) = TempImage::reserve(&options.image, total_blocks)?;
    mkfs(
        &mut device,
        &MkfsParams {
            uuid,
            label: options.label.clone(),
            region_size: MAX_REGION_BLOCKS,
            reclaim_caps: Default::default(),
            log_slots: LOG_SLOTS,
            shared_extents: options.profile.shared_extents(),
            data_policy: options.profile.data_policy(),
            name_policy: options.name_policy,
            timestamp: now,
        },
    )
    .map_err(format_error)?;
    device
        .persist_len(image_bytes)
        .map_err(|error| block_error("cannot persist temporary image", error))?;
    drop(device);
    temporary.publish(&options.image, options.force)?;
    Ok(FormatSummary {
        uuid,
        label: options.label.clone(),
        total_blocks,
        profile: options.profile,
        name_policy: options.name_policy,
    })
}

fn block_error(context: &str, error: BlockError) -> Failure {
    match error {
        BlockError::Io(error) => Failure::host_io(format!("{context}: {error}")),
        other => Failure::media("E_FORMAT", format!("{context}: {other}")),
    }
}

fn format_error(error: CoreError) -> Failure {
    match error {
        CoreError::Block(BlockError::Io(error)) => {
            Failure::host_io(format!("cannot write formatted image: {error}"))
        }
        CoreError::UnsupportedGeometry(message) => Failure::usage(message),
        CoreError::Format(error) => Failure::usage(format!("invalid format parameters: {error}")),
        other => Failure::media("E_FORMAT", format!("formatter failed: {other}")),
    }
}

fn enabled_features(profile: Profile) -> Vec<&'static str> {
    let mut features = vec![
        "org.aros.afsplus:intent-log",
        "org.aros.afsplus:intent-log-data-updates",
    ];
    if profile.orphan_directory() {
        features.push("org.aros.afsplus:orphan-directory");
    }
    if profile.shared_extents() {
        features.push("org.aros.afsplus:shared-extents");
    }
    if profile.data_policy() {
        features.push("org.aros.afsplus:data-policy");
    }
    features.sort_unstable();
    features
}

fn print_summary(summary: &FormatSummary, json: bool) {
    let algorithm = match summary.name_policy {
        NamePolicy::Sensitive => "unicode-nfc",
        NamePolicy::Insensitive => "unicode-nfc-casefold",
    };
    let features = enabled_features(summary.profile);
    if json {
        println!(
            "{{\"schema_version\":1,\"tool\":\"mkafsplus\",\"profile\":{},\"uuid\":{},\"label\":{},\"block_size\":{},\"total_blocks\":{},\"region_blocks\":{},\"name_key_algorithm\":{},\"features\":{}}}",
            json_string(summary.profile.name()),
            json_string(&uuid_hex(&summary.uuid)),
            json_string(&summary.label),
            DEFAULT_BLOCK_SIZE,
            summary.total_blocks,
            MAX_REGION_BLOCKS,
            json_string(algorithm),
            json_string_list(&features),
        );
    } else {
        println!(
            "formatted AFS+ volume {} label {:?}\ngeometry: {} blocks x {} bytes; region {} blocks\nprofile: {}\nnames: {}\nfeatures: {}",
            uuid_hex(&summary.uuid),
            summary.label,
            summary.total_blocks,
            DEFAULT_BLOCK_SIZE,
            MAX_REGION_BLOCKS,
            summary.profile.name(),
            algorithm,
            features.join(", "),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Profile;

    #[test]
    fn compiled_profile_mapping_matches_the_distributed_policy_files() {
        let profiles = [
            (
                Profile::ReaderMinimal,
                include_str!("../../../profiles/reader-minimal.toml"),
            ),
            (
                Profile::ClassicRw,
                include_str!("../../../profiles/classic-rw.toml"),
            ),
            (
                Profile::BootSafe,
                include_str!("../../../profiles/boot-safe.toml"),
            ),
            (
                Profile::Workstation,
                include_str!("../../../profiles/workstation.toml"),
            ),
            (Profile::Full, include_str!("../../../profiles/full.toml")),
        ];
        for (profile, policy) in profiles {
            assert!(
                policy.contains(&format!("name = {:?}", profile.name())),
                "{} profile name drifted",
                profile.name()
            );
            for feature in ["intent_log", "intent_log_data_updates"] {
                assert!(
                    policy.contains(&format!("allow_{feature} = true")),
                    "{} must enable {feature}",
                    profile.name()
                );
            }
            for (feature, enabled) in [
                ("shared_extents", profile.shared_extents()),
                ("data_policy", profile.data_policy()),
                ("orphan_directory", profile.orphan_directory()),
            ] {
                assert!(
                    policy.contains(&format!("allow_{feature} = {enabled}")),
                    "{} {feature} mapping drifted",
                    profile.name()
                );
            }
        }
    }
}
