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

fn verify_c_namespace(
    image: &Path,
    expected: &Path,
    created_expected: &Path,
    mode: &str,
) -> Result<(), String> {
    let expected = fs::read(expected)
        .map_err(|error| format!("cannot read expected existing-file bytes: {error}"))?;
    let created_expected = fs::read(created_expected)
        .map_err(|error| format!("cannot read expected created-file bytes: {error}"))?;
    let device = FileBackend::open(image, DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS)
        .map_err(|error| format!("cannot open C-written fixture: {error}"))?;
    let mut volume = mount(device).map_err(|error| format!("C-written replay failed: {error}"))?;

    let (absent_name, expected_name, expected_content) = match mode {
        "rename" => ("Final.BIN", "c-written.bin", expected.as_slice()),
        "torn" => ("C-Written.BIN", "Final.BIN", expected.as_slice()),
        "delete" => ("Final.BIN", "Replace.TXT", created_expected.as_slice()),
        "replace" => ("Replace.TXT", "Final.BIN", created_expected.as_slice()),
        _ => return Err(format!("unknown C verification mode {mode}")),
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
    if existing != expected_content {
        return Err(format!(
            "visible file content differs after C {mode} replay"
        ));
    }

    if !volume
        .orphan_object(17)
        .map_err(|error| format!("prior replacement orphan lookup failed: {error}"))?
    {
        return Err("prior logged replacement victim is not an orphan".into());
    }
    let expected_orphans = if matches!(mode, "delete" | "replace") {
        if !volume
            .orphan_object(16)
            .map_err(|error| format!("C operation orphan lookup failed: {error}"))?
        {
            return Err(format!("C {mode} victim is not an orphan"));
        }
        let orphan = volume
            .read_file(16)
            .map_err(|error| format!("cannot read C {mode} victim: {error}"))?;
        if orphan != expected {
            return Err(format!("C {mode} orphan content differs"));
        }
        2
    } else {
        1
    };
    if volume
        .orphan_count()
        .map_err(|error| format!("orphan count failed: {error}"))?
        != expected_orphans
    {
        return Err(format!("unexpected orphan count after C {mode}"));
    }
    drop(volume.into_device());
    println!(
        "image={} c_{}_replay=PASS visible_object={} orphans={}",
        image.display(),
        mode,
        existing_id,
        expected_orphans
    );
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1).map(PathBuf::from);
    let Some(first) = arguments.next() else {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn|--verify-c-delete|--verify-c-replace] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    };
    let verify_rename = first.as_os_str() == OsStr::new("--verify-c-rename");
    let verify_torn = first.as_os_str() == OsStr::new("--verify-c-torn");
    let verify_delete = first.as_os_str() == OsStr::new("--verify-c-delete");
    let verify_replace = first.as_os_str() == OsStr::new("--verify-c-replace");
    let verifies = verify_rename || verify_torn || verify_delete || verify_replace;
    let image = if verifies {
        let Some(image) = arguments.next() else {
            eprintln!(
                "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn|--verify-c-delete|--verify-c-replace] IMAGE EXPECTED CREATED_EXPECTED"
            );
            return ExitCode::from(2);
        };
        image
    } else {
        first
    };
    let Some(expected) = arguments.next() else {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn|--verify-c-delete|--verify-c-replace] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    };
    let Some(created_expected) = arguments.next() else {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn|--verify-c-delete|--verify-c-replace] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!(
            "usage: afsplus-portable-c-log-fixture [--verify-c-rename|--verify-c-torn|--verify-c-delete|--verify-c-replace] IMAGE EXPECTED CREATED_EXPECTED"
        );
        return ExitCode::from(2);
    }
    let result = if verifies {
        let mode = if verify_rename {
            "rename"
        } else if verify_torn {
            "torn"
        } else if verify_delete {
            "delete"
        } else {
            "replace"
        };
        verify_c_namespace(&image, &expected, &created_expected, mode)
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
