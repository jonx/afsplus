//! Exercise the cross-host half of the same-image Alpha-0 qualification.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const AROS_FILE: &str = "alpha0.from-aros";
const HOST_FILE: &str = "alpha0.from-host";
const WORK_DIRECTORY: &str = "alpha0.host-work";

fn path(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

fn run(root: &Path) -> Result<(), String> {
    let from_aros = fs::read(path(root, AROS_FILE))
        .map_err(|error| format!("cannot read {AROS_FILE}: {error}"))?;
    if from_aros != b"hello" {
        return Err(format!(
            "{AROS_FILE} has unexpected contents: {from_aros:?}"
        ));
    }

    let from_host = path(root, HOST_FILE);
    let work = path(root, WORK_DIRECTORY);
    if from_host.exists() || work.exists() {
        return Err("refusing a reused host Alpha-0 fixture".to_owned());
    }

    fs::create_dir(&work).map_err(|error| format!("cannot create work directory: {error}"))?;
    let draft = work.join("draft");
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&draft)
        .map_err(|error| format!("cannot create draft: {error}"))?;
    file.write_all(b"host")
        .map_err(|error| format!("cannot write prefix: {error}"))?;
    file.seek(SeekFrom::Start(8192))
        .and_then(|_| file.write_all(b"tail"))
        .map_err(|error| format!("cannot make sparse write: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("cannot fsync sparse write: {error}"))?;
    file.set_len(4)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot truncate and fsync: {error}"))?;
    drop(file);

    fs::rename(&draft, &from_host).map_err(|error| format!("cannot rename draft: {error}"))?;
    fs::remove_dir(&work).map_err(|error| format!("cannot remove work directory: {error}"))?;

    let mut readback = Vec::new();
    File::open(&from_host)
        .and_then(|mut file| file.read_to_end(&mut readback))
        .map_err(|error| format!("cannot reopen host result: {error}"))?;
    if readback != b"host" {
        return Err(format!("{HOST_FILE} has unexpected contents: {readback:?}"));
    }

    // Until the Alpha-0 exposes xattrs, macOS may represent Finder metadata
    // as AppleDouble siblings. They are valid ordinary AFS+ files, but they
    // would add platform noise to the cross-host fixture and later benchmarks.
    for sidecar in ["._alpha0.host-work", "._alpha0.from-host"] {
        match fs::remove_file(path(root, sidecar)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot remove {sidecar}: {error}")),
        }
    }

    println!("[AFSPLUS-HOST-ALPHA0] PASS read-aros/create/write/truncate/rename/fsync/readback");
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let Some(root) = arguments.next().map(PathBuf::from) else {
        eprintln!("usage: afsplus-mounted-alpha0 <mounted-afsplus-directory>");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: afsplus-mounted-alpha0 <mounted-afsplus-directory>");
        return ExitCode::from(2);
    }
    match run(&root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-mounted-alpha0: {error}");
            ExitCode::FAILURE
        }
    }
}
