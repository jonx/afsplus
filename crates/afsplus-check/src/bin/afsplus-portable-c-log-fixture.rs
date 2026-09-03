//! Build one deterministic pending intent-log image for the portable C gate.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use afsplus_block::FileBackend;
use afsplus_core::volume::BatchOp;
use afsplus_core::volume::DataUpdatePolicy;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

const TOTAL_BLOCKS: u64 = 4096;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn run(image: &PathBuf, expected: &PathBuf, created_expected: &PathBuf) -> Result<(), String> {
    if image.exists() || expected.exists() || created_expected.exists() {
        return Err("refusing to replace an existing fixture output".into());
    }
    let mut device = FileBackend::create(image, DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS)
        .map_err(|error| format!("cannot create fixture image: {error}"))?;
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x5a; 16],
            label: "PortableCLog".into(),
            region_size: TOTAL_BLOCKS as u32,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: false,
            data_policy: true,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: timestamp(1),
        },
    )
    .map_err(|error| format!("mkfs failed: {error}"))?;

    let mut volume = mount(device).map_err(|error| format!("mount failed: {error}"))?;
    let mut content = (0..(3 * DEFAULT_BLOCK_SIZE + 50))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let object_id = volume
        .create_file_in_root("Durable.BIN", &content, timestamp(2))
        .map_err(|error| format!("base create failed: {error}"))?;
    let replaced_id = volume
        .create_file_in_root("Replace.TXT", b"replaced victim", timestamp(2))
        .map_err(|error| format!("replace victim create failed: {error}"))?;
    volume
        .set_file_data_policy(replaced_id, DataUpdatePolicy::InPlacePrivate, timestamp(2))
        .map_err(|error| format!("replace victim policy failed: {error}"))?;
    volume
        .link_file(object_id, OBJECT_ROOT, "Alias.BIN", timestamp(2))
        .map_err(|error| format!("hard link create failed: {error}"))?;
    volume
        .window_op(
            &BatchOp::DeleteFile {
                parent_id: OBJECT_ROOT,
                name: "ALIAS.bin",
            },
            timestamp(3),
        )
        .map_err(|error| format!("logged hard-link delete failed: {error}"))?;
    volume
        .window_fsync()
        .map_err(|error| format!("hard-link delete fsync failed: {error}"))?;

    let write_offset = DEFAULT_BLOCK_SIZE + 17;
    let replacement = vec![0xa5; DEFAULT_BLOCK_SIZE + 333];
    volume
        .window_write_file_at(object_id, write_offset as u64, &replacement, timestamp(4))
        .map_err(|error| format!("logged write failed: {error}"))?;
    content[write_offset..write_offset + replacement.len()].copy_from_slice(&replacement);
    volume
        .window_fsync()
        .map_err(|error| format!("write fsync failed: {error}"))?;

    let final_size = DEFAULT_BLOCK_SIZE + 123;
    volume
        .window_truncate_file(object_id, final_size as u64, timestamp(5))
        .map_err(|error| format!("logged truncate failed: {error}"))?;
    content.truncate(final_size);
    volume
        .window_fsync()
        .map_err(|error| format!("truncate fsync failed: {error}"))?;

    let created_content = b"created through the durable log";
    let created_id = volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "Pending.TXT",
                content: created_content,
            },
            timestamp(6),
        )
        .map_err(|error| format!("logged create failed: {error}"))?
        .ok_or_else(|| "logged create returned no object ID".to_owned())?;
    volume
        .window_fsync()
        .map_err(|error| format!("create fsync failed: {error}"))?;
    volume
        .window_op(
            &BatchOp::Rename {
                source_parent_id: OBJECT_ROOT,
                source_name: "pending.txt",
                target_parent_id: OBJECT_ROOT,
                target_name: "Moved.TXT",
                replace: false,
            },
            timestamp(7),
        )
        .map_err(|error| format!("first logged rename failed: {error}"))?;
    volume
        .window_fsync()
        .map_err(|error| format!("first rename fsync failed: {error}"))?;
    volume
        .window_op(
            &BatchOp::Rename {
                source_parent_id: OBJECT_ROOT,
                source_name: "durable.bin",
                target_parent_id: OBJECT_ROOT,
                target_name: "Final.BIN",
                replace: false,
            },
            timestamp(8),
        )
        .map_err(|error| format!("hard-linked rename failed: {error}"))?;
    volume
        .window_fsync()
        .map_err(|error| format!("hard-linked rename fsync failed: {error}"))?;
    volume
        .window_op(
            &BatchOp::Rename {
                source_parent_id: OBJECT_ROOT,
                source_name: "moved.txt",
                target_parent_id: OBJECT_ROOT,
                target_name: "Replace.TXT",
                replace: true,
            },
            timestamp(9),
        )
        .map_err(|error| format!("logged replacement failed: {error}"))?;
    volume
        .window_fsync()
        .map_err(|error| format!("replacement fsync failed: {error}"))?;
    drop(volume.into_device());
    OpenOptions::new()
        .write(true)
        .open(image)
        .and_then(|file| file.set_len(TOTAL_BLOCKS * DEFAULT_BLOCK_SIZE as u64))
        .map_err(|error| format!("cannot preserve sparse device geometry: {error}"))?;

    fs::write(expected, &content)
        .map_err(|error| format!("cannot write expected file: {error}"))?;
    fs::write(created_expected, created_content)
        .map_err(|error| format!("cannot write created expected file: {error}"))?;
    println!(
        "image={} records=7 existing_object={} replaced_object={} created_object={} final_size={}",
        image.display(),
        object_id,
        replaced_id,
        created_id,
        content.len()
    );
    Ok(())
}

fn verify_c_rename(
    image: &Path,
    expected: &Path,
    created_expected: &Path,
    rename_committed: bool,
) -> Result<(), String> {
    let expected = fs::read(expected)
        .map_err(|error| format!("cannot read expected existing-file bytes: {error}"))?;
    let created_expected = fs::read(created_expected)
        .map_err(|error| format!("cannot read expected created-file bytes: {error}"))?;
    let device = FileBackend::open(image, DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS)
        .map_err(|error| format!("cannot open C-written fixture: {error}"))?;
    let mut volume = mount(device).map_err(|error| format!("C-written replay failed: {error}"))?;

    let absent_name = if rename_committed {
        "Final.BIN"
    } else {
        "C-Written.BIN"
    };
    let expected_name = if rename_committed {
        "c-written.bin"
    } else {
        "Final.BIN"
    };
    if volume
        .lookup_root(absent_name)
        .map_err(|error| format!("absent-name lookup failed: {error}"))?
        .is_some()
    {
        return Err(format!("replay unexpectedly retained {absent_name}"));
    }
    let existing_id = volume
        .lookup_root(expected_name)
        .map_err(|error| format!("expected-name lookup failed: {error}"))?
        .ok_or_else(|| format!("replay did not publish {expected_name}"))?;
    let existing = volume
        .read_file(existing_id)
        .map_err(|error| format!("cannot read C-renamed file: {error}"))?;
    if existing != expected {
        return Err("existing-file content differs after Rust replay".into());
    }

    let created_id = volume
        .lookup_root("Replace.TXT")
        .map_err(|error| format!("created-file lookup failed: {error}"))?
        .ok_or_else(|| "prior logged replacement disappeared".to_owned())?;
    let created = volume
        .read_file(created_id)
        .map_err(|error| format!("cannot read prior logged replacement: {error}"))?;
    if created != created_expected {
        return Err("prior logged replacement content differs after C append".into());
    }
    drop(volume.into_device());
    println!(
        "image={} {}=PASS existing_object={} created_object={}",
        image.display(),
        if rename_committed {
            "c_rename_replay"
        } else {
            "c_torn_replay"
        },
        existing_id,
        created_id
    );
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1).map(PathBuf::from);
    let Some(first) = arguments.next() else {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    };
    let verify_rename = first.as_os_str() == OsStr::new("--verify-c-rename");
    let verify_torn = first.as_os_str() == OsStr::new("--verify-c-torn");
    let image = if verify_rename || verify_torn {
        let Some(image) = arguments.next() else {
            eprintln!(
                "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn] IMAGE EXPECTED CREATED_EXPECTED"
            );
            return ExitCode::from(2);
        };
        image
    } else {
        first
    };
    let Some(expected) = arguments.next() else {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    };
    let Some(created_expected) = arguments.next() else {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    }
    let result = if verify_rename || verify_torn {
        verify_c_rename(&image, &expected, &created_expected, verify_rename)
    } else {
        run(&image, &expected, &created_expected)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-portable-c-log-fixture: {error}");
            ExitCode::FAILURE
        }
    }
}
