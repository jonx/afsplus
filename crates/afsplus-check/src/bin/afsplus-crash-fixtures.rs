//! Build deterministic intent-log crash images for native mount/replay gates.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use afsplus_block::{crash_states, BlockDevice, MemoryBackend, RecordedOp, RecordingBackend};
use afsplus_check::check_device;
use afsplus_core::volume::BatchOp;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

const TOTAL_BLOCKS: u64 = 16_384;
const OLD_CONTENT: &[u8] = b"old";
const NEW_CONTENT: &[u8] = b"new";

struct Candidate {
    crash_point: usize,
    description: String,
    image: MemoryBackend,
    pending_records: usize,
    warnings: Vec<String>,
    outcome: &'static str,
}

struct Fixture<'a> {
    name: &'static str,
    candidate: &'a Candidate,
}

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn create<'a>(name: &'a str, content: &'a [u8]) -> BatchOp<'a> {
    BatchOp::CreateFile {
        parent_id: OBJECT_ROOT,
        name,
        content,
    }
}

fn publish<'a>(source: &'a str, target: &'a str) -> BatchOp<'a> {
    BatchOp::Rename {
        source_parent_id: OBJECT_ROOT,
        source_name: source,
        target_parent_id: OBJECT_ROOT,
        target_name: target,
        replace: true,
    }
}

fn classify(mut image: MemoryBackend, description: String, crash_point: usize) -> Candidate {
    let report = check_device(&mut image);
    assert!(report.is_clean(), "{description}: {:?}", report.errors);
    let pending_records = report
        .volume
        .as_ref()
        .expect("checked crash image has a volume summary")
        .log_records_pending;
    let warnings = report.warnings;

    let mut recovered = mount(image.clone())
        .unwrap_or_else(|error| panic!("{description}: recovery mount failed: {error}"));
    assert_eq!(
        recovered.lookup_root("HEAD.lock").unwrap(),
        None,
        "{description}: temporary name survived recovery"
    );
    let head = recovered
        .lookup_root("HEAD")
        .unwrap()
        .unwrap_or_else(|| panic!("{description}: HEAD disappeared"));
    let content = recovered.read_file(head).unwrap();
    let outcome = match content.as_slice() {
        OLD_CONTENT => "old",
        NEW_CONTENT => "new",
        other => panic!("{description}: disallowed HEAD content {other:?}"),
    };
    let mut recovered_image = recovered.into_device();
    let recovered_report = check_device(&mut recovered_image);
    assert!(
        recovered_report.is_clean(),
        "{description}: post-replay checker errors {:?}",
        recovered_report.errors
    );

    Candidate {
        crash_point,
        description,
        image,
        pending_records,
        warnings,
        outcome,
    }
}

fn find<'a>(
    candidates: &'a [Candidate],
    label: &str,
    predicate: impl Fn(&Candidate) -> bool,
) -> &'a Candidate {
    candidates
        .iter()
        .find(|candidate| predicate(candidate))
        .unwrap_or_else(|| panic!("crash model did not produce required fixture {label}"))
}

fn write_sparse_image(path: &Path, image: &MemoryBackend) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    let block_size = image.block_size();
    for lba in 0..image.total_blocks() {
        let block = image.peek(lba);
        if block.iter().any(|byte| *byte != 0) {
            file.seek(SeekFrom::Start(lba * block_size as u64))
                .and_then(|_| file.write_all(&block))
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        }
    }
    file.set_len(image.total_blocks() * block_size as u64)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot finish {}: {error}", path.display()))
}

fn run(output: &Path) -> Result<(), String> {
    if output.exists() {
        return Err(format!(
            "refusing to replace existing output {}",
            output.display()
        ));
    }

    let mut formatted = MemoryBackend::new(DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS);
    mkfs(
        &mut formatted,
        &MkfsParams {
            uuid: [0x47; 16],
            label: "AFSPlusCrashReplay".into(),
            region_size: TOTAL_BLOCKS as u32,
            reclaim_caps: Default::default(),
            log_slots: 8,
            timestamp: timestamp(0),
        },
    )
    .map_err(|error| format!("mkfs failed: {error}"))?;
    let mut base_volume =
        mount(formatted).map_err(|error| format!("base mount failed: {error}"))?;
    base_volume
        .create_file_in_root("HEAD", OLD_CONTENT, timestamp(1))
        .map_err(|error| format!("base create failed: {error}"))?;
    let base = base_volume.into_device();

    let mut volume = mount(RecordingBackend::new(base.clone()))
        .map_err(|error| format!("recording mount failed: {error}"))?;
    volume
        .window_op(&create("HEAD.lock", NEW_CONTENT), timestamp(2))
        .map_err(|error| format!("logged create failed: {error}"))?;
    volume
        .window_op(&publish("HEAD.lock", "HEAD"), timestamp(2))
        .map_err(|error| format!("logged publish failed: {error}"))?;
    volume
        .window_fsync()
        .map_err(|error| format!("logged fsync failed: {error}"))?;
    let (_, log) = volume.into_device().into_parts();
    let flush_index = log
        .iter()
        .position(|operation| matches!(operation, RecordedOp::Flush))
        .ok_or_else(|| "recorded fsync has no flush barrier".to_owned())?;
    let log_lba = log[..flush_index]
        .iter()
        .rev()
        .find_map(|operation| match operation {
            RecordedOp::Write { lba, .. } => Some(*lba),
            RecordedOp::Flush => None,
        })
        .ok_or_else(|| "recorded fsync has no intent-log write".to_owned())?;

    let mut candidates = Vec::new();
    for crash_point in 0..=log.len() {
        for state in crash_states(&base, &log, crash_point) {
            candidates.push(classify(state.image, state.description, crash_point));
        }
    }

    let torn_log_marker = format!("(lba {log_lba}) torn");
    let fixtures = [
        Fixture {
            name: "00-before-any-write",
            candidate: find(&candidates, "before any write", |candidate| {
                candidate.crash_point == 0 && candidate.outcome == "old"
            }),
        },
        Fixture {
            name: "01-data-before-log",
            candidate: find(&candidates, "data before log", |candidate| {
                candidate.crash_point == 1
                    && candidate.description.contains("subset 0b1")
                    && candidate.outcome == "old"
            }),
        },
        Fixture {
            name: "02-record-over-missing-data",
            candidate: find(&candidates, "record over missing data", |candidate| {
                candidate.crash_point == flush_index
                    && candidate.pending_records == 0
                    && candidate
                        .warnings
                        .iter()
                        .any(|warning| warning.contains("content CRC mismatch"))
            }),
        },
        Fixture {
            name: "03-torn-log-record",
            candidate: find(&candidates, "torn log record", |candidate| {
                candidate.crash_point == flush_index
                    && candidate.description.contains(&torn_log_marker)
                    && candidate.pending_records == 0
                    && candidate.outcome == "old"
            }),
        },
        Fixture {
            name: "04-valid-record-before-barrier",
            candidate: find(&candidates, "valid record before barrier", |candidate| {
                candidate.crash_point == flush_index
                    && candidate.pending_records == 1
                    && candidate.outcome == "new"
            }),
        },
        Fixture {
            name: "05-valid-record-after-barrier",
            candidate: find(&candidates, "valid record after barrier", |candidate| {
                candidate.crash_point == flush_index + 1
                    && candidate.pending_records == 1
                    && candidate.outcome == "new"
            }),
        },
    ];

    fs::create_dir(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let mut manifest = String::from("fixture\texpected\tpending_before\tdescription\n");
    for fixture in fixtures {
        let filename = format!("{}.img", fixture.name);
        write_sparse_image(&output.join(&filename), &fixture.candidate.image)?;
        let description = fixture.candidate.description.replace(['\t', '\n'], " ");
        manifest.push_str(&format!(
            "{filename}\t{}\t{}\t{description}\n",
            fixture.candidate.outcome, fixture.candidate.pending_records
        ));
    }
    fs::write(output.join("manifest.tsv"), manifest)
        .map_err(|error| format!("cannot write manifest: {error}"))?;
    fs::write(
        output.join("README.txt"),
        "Deterministic AFS+ intent-log crash fixtures.\n\
         Each image is 64 MiB, checker-clean before recovery, and must recover\n\
         HEAD to the manifest's exact old/new content with no HEAD.lock name.\n",
    )
    .map_err(|error| format!("cannot write fixture README: {error}"))?;
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(output) = args.next().map(PathBuf::from) else {
        eprintln!("usage: afsplus-crash-fixtures <output-directory>");
        return ExitCode::from(2);
    };
    if args.next().is_some() {
        eprintln!("usage: afsplus-crash-fixtures <output-directory>");
        return ExitCode::from(2);
    }
    match run(&output) {
        Ok(()) => {
            println!("generated crash fixtures in {}", output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("afsplus-crash-fixtures: {error}");
            ExitCode::FAILURE
        }
    }
}
