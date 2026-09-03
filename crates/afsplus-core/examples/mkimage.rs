//! Builds a small demo AFS+ image with a few files, for inspection with
//! `afsplus-check`.
//!
//! ```text
//! cargo run -p afsplus-core --example mkimage -- demo.img
//! cargo run -p afsplus-check -- demo.img --json
//! ```

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use afsplus_block::FileBackend;
use afsplus_core::{mkfs, mount, MkfsParams};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE};

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("usage: mkimage <image>");
            std::process::exit(2);
        });

    let unix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let now = Timespec {
        seconds: unix.as_secs() as i64,
        nanoseconds: unix.subsec_nanos(),
    };

    // Derive a demo UUID from the clock; real tooling will use proper entropy.
    let mut uuid = [0u8; 16];
    uuid[..8].copy_from_slice(&unix.as_nanos().to_le_bytes()[..8]);
    uuid[8..].copy_from_slice(&std::process::id().to_le_bytes().repeat(2));

    let mut dev = FileBackend::create(&path, DEFAULT_BLOCK_SIZE, 256).unwrap();
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid,
            label: "DemoVol".into(),
            region_size: 64,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: now,
        },
    )
    .unwrap();

    let mut vol = mount(dev).unwrap();
    for name in ["readme.txt", "notes.md", "café.rs"] {
        let content = format!("demo content of {name}\n");
        let id = vol
            .create_file_in_root(name, content.as_bytes(), now)
            .unwrap();
        println!("created {name} as object {id}");
    }
    println!(
        "image {} at generation {} with {} root entries",
        path.display(),
        vol.generation(),
        vol.list_root().expect("list root").len()
    );
}
