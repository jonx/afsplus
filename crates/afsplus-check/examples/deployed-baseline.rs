// SPDX-License-Identifier: MIT
// Copyright (c) 2026 John Knipper

//! Historical fixture producer. Run only to add a baseline, never to replace one.
use std::io::Write;
use std::path::Path;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::BatchOp;
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::{Timespec, OBJECT_ROOT};

fn time(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn save(path: &Path, device: &MemoryBackend) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    for lba in 0..device.total_blocks() {
        file.write_all(&device.peek(lba)).unwrap();
    }
    file.sync_all().unwrap();
}

fn main() {
    let output = std::env::args_os()
        .nth(1)
        .expect("usage: deployed-baseline NEW_DIRECTORY");
    let output = Path::new(&output);
    std::fs::create_dir(output).unwrap();
    let mut device = MemoryBackend::new(4096, 1024);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x30; 16],
            label: "DeployedBaseline".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Insensitive,
            timestamp: time(1_790_769_600),
        },
    )
    .unwrap();
    let mut volume = mount(device).unwrap();
    let directory = volume
        .create_directory_in_root("System", time(1_790_769_601))
        .unwrap();
    let file = volume
        .create_file_in_directory(
            directory,
            "ReadMe",
            b"AFS+ deployed image baseline\n",
            time(1_790_769_602),
        )
        .unwrap();
    volume
        .set_object_comment(file, "Preserve this comment", time(1_790_769_603))
        .unwrap();
    volume
        .set_object_protection(file, 0x40, time(1_790_769_604))
        .unwrap();
    volume
        .create_file_in_root("Payload", &vec![0xa5; 9000], time(1_790_769_605))
        .unwrap();
    let clean = volume.into_device();
    save(&output.join("clean.img"), &clean);
    let mut volume = mount(clean).unwrap();
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "DurablePending",
                content: b"fsync survived without a checkpoint\n",
            },
            time(1_790_769_606),
        )
        .unwrap();
    volume.window_fsync().unwrap();
    save(&output.join("pending.img"), &volume.into_device());
}
