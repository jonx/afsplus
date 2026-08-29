//! `afsplus-check` CLI: verify an AFS+ image file.
//!
//! Usage: `afsplus-check <image> [--json]`
//!
//! Exit codes: 0 clean, 1 findings, 2 usage or I/O failure.

use std::path::PathBuf;
use std::process::ExitCode;

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_check::check_device;
use afsplus_format::ident::Identification;
use afsplus_format::DEFAULT_BLOCK_SIZE;

fn main() -> ExitCode {
    let mut json = false;
    let mut image: Option<PathBuf> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => json = true,
            "--help" | "-h" => {
                eprintln!("usage: afsplus-check <image> [--json]");
                return ExitCode::from(2);
            }
            _ if image.is_none() => image = Some(PathBuf::from(arg)),
            other => {
                eprintln!("unexpected argument: {other}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(image) = image else {
        eprintln!("usage: afsplus-check <image> [--json]");
        return ExitCode::from(2);
    };

    let mut dev = match FileBackend::open_sized_by_file(&image, DEFAULT_BLOCK_SIZE) {
        Ok(dev) => dev,
        Err(e) => {
            eprintln!("cannot open {}: {e}", image.display());
            return ExitCode::from(2);
        }
    };

    // A sparse image file may be shorter than the volume it holds; widen the
    // device to the geometry the identification block declares.
    let mut buf = vec![0u8; DEFAULT_BLOCK_SIZE];
    if dev.read_block(0, &mut buf).is_ok() {
        if let Ok(ident) = Identification::decode(&buf) {
            if ident.total_blocks > dev.total_blocks() {
                dev.set_total_blocks(ident.total_blocks);
            }
        }
    }

    let report = check_device(&mut dev);
    if json {
        println!("{}", report.render_json());
    } else {
        print!("{}", report.render_text());
    }
    if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
