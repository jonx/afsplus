//! Populate a formatted AFS+ image from a host directory tree.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use afsplus_block::{BlockDevice, FileBackend};
use afsplus_core::{mount, Volume};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

#[derive(Default)]
struct ImportStats {
    directories: u64,
    files: u64,
    bytes: u64,
}

fn utf8_name<'a>(name: &'a OsStr, path: &Path) -> Result<&'a str, String> {
    name.to_str()
        .ok_or_else(|| format!("non-UTF-8 path is not supported: {}", path.display()))
}

fn sorted_entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate {}: {error}", path.display()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn import_directory(
    volume: &mut Volume<FileBackend>,
    parent_id: u64,
    source: &Path,
    stats: &mut ImportStats,
) -> Result<(), String> {
    for entry in sorted_entries(source)? {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        let name = utf8_name(&entry.file_name(), &path)?.to_owned();
        if file_type.is_symlink() {
            return Err(format!(
                "symbolic links are not representable in Alpha-0: {}",
                path.display()
            ));
        }
        if file_type.is_dir() {
            let object_id = volume
                .create_directory(parent_id, &name, Timespec::default())
                .map_err(|error| format!("cannot create directory {name:?}: {error}"))?;
            stats.directories += 1;
            import_directory(volume, object_id, &path, stats)?;
        } else if file_type.is_file() {
            let content = fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            volume
                .create_file_in_directory(parent_id, &name, &content, Timespec::default())
                .map_err(|error| format!("cannot create file {name:?}: {error}"))?;
            stats.files += 1;
            stats.bytes = stats
                .bytes
                .checked_add(content.len() as u64)
                .ok_or_else(|| "import byte count overflow".to_owned())?;
        } else {
            return Err(format!("unsupported host object: {}", path.display()));
        }
    }
    Ok(())
}

fn open_image(path: &Path) -> Result<FileBackend, String> {
    let mut device = FileBackend::open_sized_by_file(path, DEFAULT_BLOCK_SIZE)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut block = vec![0; DEFAULT_BLOCK_SIZE];
    device
        .read_block(0, &mut block)
        .map_err(|error| format!("cannot read {} identification: {error}", path.display()))?;
    let ident = Identification::decode(&block)
        .map_err(|error| format!("cannot decode {} identification: {error}", path.display()))?;
    device.set_total_blocks(ident.total_blocks);
    Ok(device)
}

fn run(image: &Path, source: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err(format!("source is not a directory: {}", source.display()));
    }
    let device = open_image(image)?;
    let mut volume = mount(device).map_err(|error| format!("cannot mount image: {error}"))?;
    if !volume
        .list_root()
        .map_err(|error| format!("cannot inspect image root: {error}"))?
        .is_empty()
    {
        return Err("refusing to populate a non-empty image".to_owned());
    }
    let mut stats = ImportStats::default();
    import_directory(&mut volume, OBJECT_ROOT, source, &mut stats)?;
    volume
        .sync()
        .map_err(|error| format!("cannot sync populated image: {error}"))?;
    println!(
        "populated {} from {}: {} directories, {} files, {} bytes, generation {}",
        image.display(),
        source.display(),
        stats.directories,
        stats.files,
        stats.bytes,
        volume.generation()
    );
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(image), Some(source)) = (arguments.next(), arguments.next()) else {
        eprintln!("usage: afsplus-populate <image> <source-directory>");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: afsplus-populate <image> <source-directory>");
        return ExitCode::from(2);
    }
    match run(&PathBuf::from(image), &PathBuf::from(source)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("afsplus-populate: {error}");
            ExitCode::FAILURE
        }
    }
}
