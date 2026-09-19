//! Populate a formatted AFS+ image from a host directory tree.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
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
    symlinks: u64,
    hard_links: u64,
    bytes: u64,
    holes: u64,
}

/// One write per chunk; a chunk of zeros is left unwritten, so a host hole
/// stays a hole on the volume and a large file never needs one extent.
const CHUNK: usize = 1 << 20;

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

/// Host inode to object, for a hard link met a second time.
type Seen = HashMap<(u64, u64), u64>;

fn import_file(
    volume: &mut Volume<FileBackend>,
    parent_id: u64,
    name: &str,
    path: &Path,
    stats: &mut ImportStats,
) -> Result<u64, String> {
    let mut source =
        fs::File::open(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let size = source
        .metadata()
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
        .len();
    let object_id = volume
        .create_file_in_directory(parent_id, name, &[], Timespec::default())
        .map_err(|error| format!("cannot create file {name:?}: {error}"))?;
    let mut buffer = vec![0u8; CHUNK];
    let mut offset = 0u64;
    while offset < size {
        let want = usize::try_from((size - offset).min(CHUNK as u64)).expect("chunk fits");
        source
            .read_exact(&mut buffer[..want])
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if buffer[..want].iter().any(|&byte| byte != 0) {
            volume
                .write_file_at(object_id, offset, &buffer[..want], Timespec::default())
                .map_err(|error| format!("cannot write {name:?} at {offset}: {error}"))?;
        } else {
            stats.holes += 1;
        }
        offset += want as u64;
    }
    // A file that ends in zeros keeps its length without a written tail.
    volume
        .truncate_file(object_id, size, Timespec::default())
        .map_err(|error| format!("cannot set the size of {name:?}: {error}"))?;
    stats.files += 1;
    stats.bytes = stats
        .bytes
        .checked_add(size)
        .ok_or_else(|| "import byte count overflow".to_owned())?;
    Ok(object_id)
}

fn import_directory(
    volume: &mut Volume<FileBackend>,
    parent_id: u64,
    source: &Path,
    stats: &mut ImportStats,
    seen: &mut Seen,
) -> Result<(), String> {
    for entry in sorted_entries(source)? {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        let name = utf8_name(&entry.file_name(), &path)?.to_owned();
        if file_type.is_symlink() {
            let target = fs::read_link(&path)
                .map_err(|error| format!("cannot read the link {}: {error}", path.display()))?;
            let target = target
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 link target: {}", path.display()))?
                .to_owned();
            volume
                .create_symlink(parent_id, &name, &target, Timespec::default())
                .map_err(|error| format!("cannot create symlink {name:?}: {error}"))?;
            stats.symlinks += 1;
        } else if file_type.is_dir() {
            let object_id = volume
                .create_directory(parent_id, &name, Timespec::default())
                .map_err(|error| format!("cannot create directory {name:?}: {error}"))?;
            stats.directories += 1;
            import_directory(volume, object_id, &path, stats, seen)?;
        } else if file_type.is_file() {
            let meta = fs::metadata(&path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            let key = (meta.dev(), meta.ino());
            if meta.nlink() > 1 {
                if let Some(&object_id) = seen.get(&key) {
                    volume
                        .link_file(object_id, parent_id, &name, Timespec::default())
                        .map_err(|error| format!("cannot link {name:?}: {error}"))?;
                    stats.hard_links += 1;
                    continue;
                }
            }
            let object_id = import_file(volume, parent_id, &name, &path, stats)?;
            if meta.nlink() > 1 {
                seen.insert(key, object_id);
            }
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
    let mut seen = Seen::new();
    import_directory(&mut volume, OBJECT_ROOT, source, &mut stats, &mut seen)?;
    volume
        .sync()
        .map_err(|error| format!("cannot sync populated image: {error}"))?;
    println!(
        "populated {} from {}: {} directories, {} files, {} symlinks, {} hard links, {} bytes ({} MiB of zeros left as holes), generation {}",
        image.display(),
        source.display(),
        stats.directories,
        stats.files,
        stats.symlinks,
        stats.hard_links,
        stats.bytes,
        stats.holes,
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
