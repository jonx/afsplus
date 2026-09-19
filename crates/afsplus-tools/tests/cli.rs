use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::mount;
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "afsplus-tools-{test}-{}-{sequence}",
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

fn binary(name: &str) -> &'static str {
    match name {
        "mkafsplus" => env!("CARGO_BIN_EXE_mkafsplus"),
        "afsplus-info" => env!("CARGO_BIN_EXE_afsplus-info"),
        "afsplus-dump" => env!("CARGO_BIN_EXE_afsplus-dump"),
        "afsplus-extract" => env!("CARGO_BIN_EXE_afsplus-extract"),
        _ => panic!("unknown test binary"),
    }
}

fn run<I, S>(name: &str, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(binary(name)).args(args).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn assert_status(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn format_image(path: &Path, extra: &[&str]) -> Output {
    let mut args = vec![
        "--size-mib",
        "2",
        "--uuid",
        "00112233-4455-6677-8899-aabbccddeeff",
        "--timestamp-seconds",
        "123",
    ];
    args.extend_from_slice(extra);
    let path_string = path.as_os_str().to_owned();
    let mut command = Command::new(binary("mkafsplus"));
    command.args(args).arg(path_string).output().unwrap()
}

#[test]
fn help_and_usage_have_stable_exit_contracts() {
    for name in [
        "mkafsplus",
        "afsplus-info",
        "afsplus-dump",
        "afsplus-extract",
    ] {
        let output = run(name, ["--help"]);
        assert_status(&output, 0);
        assert!(stdout(&output).starts_with("usage:"));
        assert!(stderr(&output).is_empty());

        let output = run(name, ["--definitely-unknown"]);
        assert_status(&output, 2);
        assert!(stderr(&output).starts_with(&format!("{name}: error[E_USAGE]:")));
    }
}

#[test]
fn formatter_json_is_exact_and_profile_features_are_explicit() {
    let temp = TempDir::new("formatter-json");
    let image = temp.join("volume.afsplus");
    let output = format_image(
        &image,
        &["--label", "Test-é", "--profile", "workstation", "--json"],
    );
    assert_status(&output, 0);
    assert_eq!(
        stdout(&output),
        "{\"schema_version\":1,\"tool\":\"mkafsplus\",\"profile\":\"workstation\",\"uuid\":\"00112233445566778899aabbccddeeff\",\"label\":\"Test-é\",\"block_size\":4096,\"total_blocks\":512,\"region_blocks\":262144,\"name_key_algorithm\":\"unicode-nfc\",\"features\":[\"org.aros.afsplus:data-policy\",\"org.aros.afsplus:intent-log\",\"org.aros.afsplus:intent-log-data-updates\",\"org.aros.afsplus:orphan-directory\",\"org.aros.afsplus:shared-extents\"]}\n"
    );
    assert_eq!(fs::metadata(&image).unwrap().len(), 2 * 1024 * 1024);

    let second = format_image(&image, &["--json"]);
    assert_status(&second, 2);
    assert!(stderr(&second).contains("error[E_EXISTS]"));
    assert_eq!(stdout(&second), "");

    let forced = format_image(
        &image,
        &[
            "--profile",
            "classic-rw",
            "--case-insensitive",
            "--force",
            "--json",
        ],
    );
    assert_status(&forced, 0);
    let forced = stdout(&forced);
    assert!(forced.contains("\"profile\":\"classic-rw\""));
    assert!(forced.contains("\"name_key_algorithm\":\"unicode-nfc-casefold\""));
    assert!(!forced.contains("shared-extents"));
    assert!(!forced.contains("data-policy"));
}

#[test]
fn formatter_accepts_non_power_of_two_mib_sizes() {
    let temp = TempDir::new("ordinary-size");
    let image = temp.join("three-mib.afsplus");
    let output = Command::new(binary("mkafsplus"))
        .args([
            "--size-mib",
            "3",
            "--uuid",
            "00112233445566778899aabbccddeeff",
            "--timestamp-seconds",
            "123",
        ])
        .arg(&image)
        .output()
        .unwrap();
    assert_status(&output, 0);
    assert_eq!(fs::metadata(image).unwrap().len(), 3 * 1024 * 1024);
}

#[test]
fn every_declared_profile_formats_and_round_trips_through_info() {
    let temp = TempDir::new("profiles");
    for profile in [
        "reader-minimal",
        "classic-rw",
        "boot-safe",
        "workstation",
        "full",
    ] {
        let image = temp.join(&format!("{profile}.afsplus"));
        let output = format_image(&image, &["--profile", profile, "--json"]);
        assert_status(&output, 0);
        assert!(stdout(&output).contains(&format!("\"profile\":\"{profile}\"")));
        let info = run("afsplus-info", [OsStr::new("--json"), image.as_os_str()]);
        assert_status(&info, 0);
        assert!(stdout(&info).contains("\"schema_version\":1"));
    }
}

#[test]
fn info_and_dump_are_repeatable_and_work_on_a_read_only_populated_image() {
    let temp = TempDir::new("readonly");
    let image = temp.join("populated.afsplus");
    let formatted = format_image(&image, &["--label", "quoted-\"-\\-é", "--json"]);
    assert_status(&formatted, 0);

    let device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut volume = mount(device).unwrap();
    volume
        .create_file_in_root(
            "hello-é.txt",
            &vec![0x5au8; DEFAULT_BLOCK_SIZE + 17],
            Timespec {
                seconds: 456,
                nanoseconds: 789,
            },
        )
        .unwrap();
    drop(volume.into_device());

    let before = fs::read(&image).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        fs::set_permissions(&image, fs::Permissions::from_mode(0o444)).unwrap();
        let mode_before = fs::metadata(&image).unwrap().mode() & 0o777;
        assert_eq!(mode_before, 0o444);
    }

    let info_a = run("afsplus-info", [OsStr::new("--json"), image.as_os_str()]);
    let info_b = run("afsplus-info", [OsStr::new("--json"), image.as_os_str()]);
    assert_status(&info_a, 0);
    assert_status(&info_b, 0);
    assert_eq!(info_a.stdout, info_b.stdout);
    let info = stdout(&info_a);
    assert!(info.starts_with("{\"schema_version\":1,\"tool\":\"afsplus-info\""));
    assert!(info.contains("\"label\":\"quoted-\\\"-\\\\-é\""));
    assert!(info.contains("\"generation\":2"));
    assert!(info.contains("\"emergency_headroom_blocks\":16"));
    assert!(info.contains("\"available_blocks\":"));

    let dump_a = run("afsplus-dump", [OsStr::new("--json"), image.as_os_str()]);
    let dump_b = run("afsplus-dump", [OsStr::new("--json"), image.as_os_str()]);
    assert_status(&dump_a, 0);
    assert_status(&dump_b, 0);
    assert_eq!(dump_a.stdout, dump_b.stdout);
    let dump = stdout(&dump_a);
    assert!(dump.starts_with("{\"schema_version\":1,\"tool\":\"afsplus-dump\""));
    assert!(dump.contains("\"consistent\":true"));
    assert!(dump.contains("\"emergency_headroom_blocks\":16"));
    assert!(dump.contains("\"available_blocks\":"));
    assert!(dump.contains("\"directory_entries\":1"));
    assert!(dump.contains("\"allocation_regions\":1"));
    assert!(dump.contains("\"bitmap_pages\":[{"));
    assert!(dump.contains("\"block_sets\":{\"metadata\":["));
    assert!(dump.contains("\"name\":\"hello-é.txt\""));
    assert!(dump.contains("\"size_bytes\":4113"));
    assert!(dump.contains("\"blocks\":2"));
    assert!(dump.find("\"object_id\":1").unwrap() < dump.find("\"object_id\":16").unwrap());

    assert_eq!(fs::read(&image).unwrap(), before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&image).unwrap().mode() & 0o777, 0o444);
    }
}

#[test]
fn dump_labels_internal_orphan_state_without_exposing_it_as_namespace() {
    let temp = TempDir::new("orphan-dump");
    let image = temp.join("orphan.afsplus");
    assert_status(&format_image(&image, &[]), 0);
    let device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut volume = mount(device).unwrap();
    let object = volume
        .create_file_in_root("open", b"pending", Timespec::default())
        .unwrap();
    volume
        .orphan_file(OBJECT_ROOT, "open", Timespec::default())
        .unwrap();
    assert!(volume.orphan_object(object).unwrap());
    drop(volume.into_device());

    let json = run("afsplus-dump", [OsStr::new("--json"), image.as_os_str()]);
    assert_status(&json, 0);
    let json = stdout(&json);
    assert!(json.contains("\"orphan_entries\":1"));
    assert!(json.contains("\"object_id\":2"));
    assert!(json.contains("\"internal_role\":\"orphan-directory\""));

    let text = run("afsplus-dump", [image.as_os_str()]);
    assert_status(&text, 0);
    assert!(stdout(&text).contains("directory 2 [internal orphan-directory] entries 1"));
}

#[test]
fn media_corruption_and_host_io_are_distinct() {
    let temp = TempDir::new("errors");
    let missing = temp.join("missing.afsplus");
    for name in ["afsplus-info", "afsplus-dump"] {
        let output = run(name, [missing.as_os_str()]);
        assert_status(&output, 2);
        assert!(stderr(&output).starts_with(&format!("{name}: error[E_HOST_IO]:")));
    }

    let image = temp.join("corrupt.afsplus");
    assert_status(&format_image(&image, &[]), 0);
    let mut file = OpenOptions::new().write(true).open(&image).unwrap();
    file.seek(SeekFrom::Start(100)).unwrap();
    file.write_all(&[0xa5]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    for name in ["afsplus-info", "afsplus-dump"] {
        let output = run(name, [image.as_os_str()]);
        assert_status(&output, 1);
        assert!(stderr(&output).starts_with(&format!("{name}: error[E_IDENT]:")));
    }
}

#[test]
fn info_reports_unknown_features_but_dump_refuses_to_guess_their_layout() {
    let temp = TempDir::new("unknown-feature");
    let image = temp.join("unknown.afsplus");
    assert_status(&format_image(&image, &[]), 0);

    let mut device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut block = vec![0u8; DEFAULT_BLOCK_SIZE];
    device.read_block(0, &mut block).unwrap();
    let mut ident = Identification::decode(&block).unwrap();
    ident.features.incompat |= 1 << 63;
    device
        .write_block(0, &ident.encode(DEFAULT_BLOCK_SIZE).unwrap())
        .unwrap();
    device.flush().unwrap();
    drop(device);

    let info = run("afsplus-info", [OsStr::new("--json"), image.as_os_str()]);
    assert_status(&info, 0);
    assert!(stdout(&info).contains("\"incompat\":\"0x8000000000000003\""));

    let dump = run("afsplus-dump", [OsStr::new("--json"), image.as_os_str()]);
    assert_status(&dump, 1);
    assert!(stderr(&dump).starts_with("afsplus-dump: error[E_FEATURE]:"));
    assert!(stdout(&dump).is_empty());
}

#[test]
fn extraction_preserves_bytes_names_and_hard_links_without_source_writes() {
    let temp = TempDir::new("extract-good");
    let image = temp.join("source.afsplus");
    assert_status(&format_image(&image, &[]), 0);
    let mut volume = mount(FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap()).unwrap();
    let now = Timespec {
        seconds: 456,
        nanoseconds: 789,
    };
    let nested = volume.create_directory_in_root("nested", now).unwrap();
    let data: Vec<u8> = (0..DEFAULT_BLOCK_SIZE * 17 + 3)
        .map(|i| (i % 251) as u8)
        .collect();
    let id = volume
        .create_file_in_directory(nested, "quoted-\"-é", &data, now)
        .unwrap();
    volume.link_file(id, OBJECT_ROOT, "alias", now).unwrap();
    let sparse = volume.create_file_in_root("sparse", &[], now).unwrap();
    volume.write_file_at(sparse, 8191, b"end", now).unwrap();
    let mut sparse_expected = vec![0; 8194];
    sparse_expected[8191..].copy_from_slice(b"end");
    drop(volume);
    let before = fs::read(&image).unwrap();
    let modified = fs::metadata(&image).unwrap().modified().unwrap();
    let destination = temp.join("extracted");
    let result = run(
        "afsplus-extract",
        [image.as_os_str(), destination.as_os_str()],
    );
    assert_status(&result, 0);
    assert_eq!(
        fs::read(destination.join(format!("object-{id}.bin"))).unwrap(),
        data
    );
    assert_eq!(
        fs::read(destination.join(format!("object-{sparse}.bin"))).unwrap(),
        sparse_expected
    );
    let manifest = fs::read_to_string(destination.join("manifest.jsonl")).unwrap();
    assert_eq!(
        manifest
            .lines()
            .filter(|line| line.contains("\"record\":\"content\""))
            .count(),
        2
    );
    assert!(manifest.contains("\"link_count\":2"));
    assert!(manifest.contains("71756f7465642d222dc3a9"));
    assert!(manifest.contains("\"created\":[456,789]"));
    assert!(manifest.contains("\"findings\":0,\"complete\":true"));
    assert_eq!(fs::read(&image).unwrap(), before);
    assert_eq!(fs::metadata(&image).unwrap().modified().unwrap(), modified);
    // Refuse both a prior export and the source itself as destinations.
    assert_status(
        &run(
            "afsplus-extract",
            [image.as_os_str(), destination.as_os_str()],
        ),
        2,
    );
    assert_status(
        &run("afsplus-extract", [image.as_os_str(), image.as_os_str()]),
        2,
    );
    assert_eq!(fs::read(&image).unwrap(), before);
}

#[test]
fn extraction_reports_damaged_object_and_recovers_healthy_sibling() {
    let temp = TempDir::new("extract-damage");
    let image = temp.join("source.afsplus");
    assert_status(&format_image(&image, &[]), 0);
    let mut volume = mount(FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap()).unwrap();
    let now = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    let bad = volume.create_file_in_root("bad", b"lost", now).unwrap();
    let good = volume
        .create_file_in_root("good", b"survivor", now)
        .unwrap();
    let geo = volume.ident().geometry();
    let cp = volume.checkpoint().clone();
    let lba = afsplus_core::object_map::lookup_lba(
        volume.device_mut(),
        &geo,
        cp.object_map_block,
        cp.generation,
        bad,
    )
    .unwrap()
    .unwrap();
    let mut block = vec![0; DEFAULT_BLOCK_SIZE];
    volume.device_mut().read_block(lba, &mut block).unwrap();
    block[100] ^= 1;
    volume.device_mut().write_block(lba, &block).unwrap();
    volume.device_mut().flush().unwrap();
    drop(volume);
    let before = fs::read(&image).unwrap();
    let destination = temp.join("extracted");
    assert_status(
        &run(
            "afsplus-extract",
            [image.as_os_str(), destination.as_os_str()],
        ),
        1,
    );
    assert_eq!(
        fs::read(destination.join(format!("object-{good}.bin"))).unwrap(),
        b"survivor"
    );
    assert!(!destination.join(format!("object-{bad}.bin")).exists());
    let manifest = fs::read_to_string(destination.join("manifest.jsonl")).unwrap();
    assert!(manifest.contains(&format!("\"object_id\":{bad},\"id\":\"E_OBJECT\"")));
    assert!(manifest.contains("\"findings\":1,\"complete\":false"));
    assert_eq!(fs::read(&image).unwrap(), before);
}

#[test]
fn extraction_limits_are_explicit_and_unknown_metadata_is_refused() {
    let temp = TempDir::new("extract-limits");
    let image = temp.join("source.afsplus");
    assert_status(&format_image(&image, &[]), 0);
    let mut volume = mount(FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap()).unwrap();
    let now = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    volume.create_file_in_root("a", b"1234", now).unwrap();
    volume.create_file_in_root("b", b"5678", now).unwrap();
    drop(volume);
    for (option, finding) in [
        ("--max-bytes", "E_BYTE_LIMIT"),
        ("--max-entries", "E_ENTRY_LIMIT"),
    ] {
        let destination = temp.join(finding);
        let result = run(
            "afsplus-extract",
            [
                OsStr::new(option),
                OsStr::new("1"),
                image.as_os_str(),
                destination.as_os_str(),
            ],
        );
        assert_status(&result, 1);
        let manifest = fs::read_to_string(destination.join("manifest.jsonl")).unwrap();
        assert!(manifest.contains(finding));
        assert!(manifest.contains("\"complete\":false"));
    }
    let mut device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap();
    let mut block = vec![0; DEFAULT_BLOCK_SIZE];
    device.read_block(0, &mut block).unwrap();
    let mut ident = Identification::decode(&block).unwrap();
    ident.features.ro_compat |= 1 << 63;
    device
        .write_block(0, &ident.encode(DEFAULT_BLOCK_SIZE).unwrap())
        .unwrap();
    device.flush().unwrap();
    drop(device);
    let destination = temp.join("unsupported");
    let before = fs::read(&image).unwrap();
    let result = run(
        "afsplus-extract",
        [image.as_os_str(), destination.as_os_str()],
    );
    assert_status(&result, 1);
    assert!(stderr(&result).contains("E_FEATURE"));
    assert!(!destination.exists());
    assert_eq!(fs::read(&image).unwrap(), before);
}

#[test]
fn extraction_isolates_a_damaged_directory_and_refuses_unreadable_roots() {
    let temp = TempDir::new("extract-directory");
    let image = temp.join("source.afsplus");
    assert_status(&format_image(&image, &[]), 0);
    let mut volume = mount(FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap()).unwrap();
    let now = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    let dir = volume.create_directory_in_root("damaged", now).unwrap();
    let hidden = volume
        .create_file_in_directory(dir, "unreachable", b"lost", now)
        .unwrap();
    let good = volume.create_file_in_root("good", b"kept", now).unwrap();
    let root = volume.checkpoint().object_map_block;
    let dir_lba = volume.stat(dir).unwrap().unwrap().data_root;
    let mut block = vec![0; DEFAULT_BLOCK_SIZE];
    volume.device_mut().read_block(dir_lba, &mut block).unwrap();
    block[100] ^= 1;
    volume.device_mut().write_block(dir_lba, &block).unwrap();
    volume.device_mut().flush().unwrap();
    drop(volume);
    let before = fs::read(&image).unwrap();
    let destination = temp.join("directory-damage");
    assert_status(
        &run(
            "afsplus-extract",
            [image.as_os_str(), destination.as_os_str()],
        ),
        1,
    );
    assert_eq!(
        fs::read(destination.join(format!("object-{good}.bin"))).unwrap(),
        b"kept"
    );
    assert!(!destination.join(format!("object-{hidden}.bin")).exists());
    let manifest = fs::read_to_string(destination.join("manifest.jsonl")).unwrap();
    assert!(manifest.contains(&format!("\"object_id\":{dir},\"id\":\"E_DIRECTORY\"")));
    assert_eq!(fs::read(&image).unwrap(), before);
    let mut device = FileBackend::open(&image, DEFAULT_BLOCK_SIZE, 512).unwrap();
    device.read_block(root, &mut block).unwrap();
    block[100] ^= 1;
    device.write_block(root, &block).unwrap();
    device.flush().unwrap();
    drop(device);
    let before = fs::read(&image).unwrap();
    let destination = temp.join("root-damage");
    let result = run(
        "afsplus-extract",
        [image.as_os_str(), destination.as_os_str()],
    );
    assert_status(&result, 1);
    assert!(stderr(&result).contains("E_MOUNT"));
    assert!(!destination.exists());
    assert_eq!(fs::read(&image).unwrap(), before);
}

/// A profile is a format-time policy; afsplus-info answers the reverse
/// question, which profiles and implementations can take the volume.
#[test]
fn info_reports_which_profiles_can_take_the_volume() {
    let dir = TempDir::new("compat");
    let classic = dir.join("classic.afsp");
    let workstation = dir.join("workstation.afsp");
    assert_status(&format_image(&classic, &["--profile", "classic-rw"]), 0);
    assert_status(&format_image(&workstation, &["--profile", "workstation"]), 0);

    let text = stdout(&run("afsplus-info", [classic.to_str().unwrap()]));
    assert!(text.contains("  classic-rw: full\n"), "{text}");
    assert!(text.contains("  boot-safe: full\n"), "{text}");
    assert!(text.contains("  portable C reader: reads\n"), "{text}");

    let text = stdout(&run("afsplus-info", [workstation.to_str().unwrap()]));
    assert!(
        text.contains("  classic-rw: read-only (beyond it: org.aros.afsplus:data-policy, org.aros.afsplus:shared-extents)\n"),
        "{text}"
    );
    assert!(text.contains("  workstation: full\n"), "{text}");

    let json = stdout(&run("afsplus-info", ["--json", workstation.to_str().unwrap()]));
    assert!(json.contains("\"compatibility\":{\"profiles\":{\"reader-minimal\":{\"verdict\":\"read-only\""), "{json}");
    assert!(json.contains("\"portable_c_reader\":{\"verdict\":\"reads\",\"beyond\":[]}"), "{json}");
}
