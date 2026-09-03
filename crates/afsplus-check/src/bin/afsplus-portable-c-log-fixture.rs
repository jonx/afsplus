//! Build one deterministic pending intent-log image for the portable C gate.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::ExitCode;

use afsplus_block::FileBackend;
use afsplus_core::volume::BatchOp;
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
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: timestamp(1),
        },
    )
    .map_err(|error| format!("mkfs failed: {error}"))?;

    let mut volume = mount(device).map_err(|error| format!("mount failed: {error}"))?;
    let mut content = (0..(3 * DEFAULT_BLOCK_SIZE + 50))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let object_id = volume
        .create_file_in_root("durable.bin", &content, timestamp(2))
        .map_err(|error| format!("base create failed: {error}"))?;

    let write_offset = DEFAULT_BLOCK_SIZE + 17;
    let replacement = vec![0xa5; DEFAULT_BLOCK_SIZE + 333];
    volume
        .window_write_file_at(object_id, write_offset as u64, &replacement, timestamp(3))
        .map_err(|error| format!("logged write failed: {error}"))?;
    content[write_offset..write_offset + replacement.len()].copy_from_slice(&replacement);
    volume
        .window_fsync()
        .map_err(|error| format!("write fsync failed: {error}"))?;

    let final_size = DEFAULT_BLOCK_SIZE + 123;
    volume
        .window_truncate_file(object_id, final_size as u64, timestamp(4))
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
                name: "pending.txt",
                content: created_content,
            },
            timestamp(5),
        )
        .map_err(|error| format!("logged create failed: {error}"))?
        .ok_or_else(|| "logged create returned no object ID".to_owned())?;
    volume
        .window_fsync()
        .map_err(|error| format!("create fsync failed: {error}"))?;
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
        "image={} records=3 existing_object={} created_object={} final_size={}",
        image.display(),
        object_id,
        created_id,
        content.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1).map(PathBuf::from);
    let Some(image) = arguments.next() else {
        eprintln!("usage: afsplus-portable-c-log-fixture IMAGE EXPECTED CREATED_EXPECTED");
        return ExitCode::from(2);
    };
    let Some(expected) = arguments.next() else {
        eprintln!("usage: afsplus-portable-c-log-fixture IMAGE EXPECTED CREATED_EXPECTED");
        return ExitCode::from(2);
    };
    let Some(created_expected) = arguments.next() else {
        eprintln!("usage: afsplus-portable-c-log-fixture IMAGE EXPECTED CREATED_EXPECTED");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: afsplus-portable-c-log-fixture IMAGE EXPECTED CREATED_EXPECTED");
        return ExitCode::from(2);
    }
    match run(&image, &expected, &created_expected) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-portable-c-log-fixture: {error}");
            ExitCode::FAILURE
        }
    }
}
