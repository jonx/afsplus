// SPDX-License-Identifier: BSD-2-Clause
use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use afsplus_block::BlockDevice;
use afsplus_core::mount::{SUPPORTED_INCOMPAT_FEATURES, SUPPORTED_RO_COMPAT_FEATURES};
use afsplus_core::{mount_with_options, MountMode, MountOptions, Volume};
use afsplus_format::ident::{COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG_DATA_UPDATES};
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::OBJECT_ROOT;

use crate::common::{
    core_failure, hex, json_string, read_header, report_failure, uuid_hex, Failure, EXIT_OK,
};

const TOOL: &str = "afsplus-extract";
const USAGE: &str =
    "usage: afsplus-extract [--max-entries N] [--max-bytes N] <image> <new-directory>";

struct Options {
    image: PathBuf,
    destination: PathBuf,
    max_entries: u64,
    max_bytes: u64,
}

pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> u8 {
    let options = match parse(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}\nExtract the selected checkpoint without writing to the source. Read the loss report in manifest.jsonl.");
            return EXIT_OK;
        }
        Err(error) => return report_failure(TOOL, error),
    };
    match execute(&options) {
        Ok(0) => EXIT_OK,
        Ok(errors) => report_failure(
            TOOL,
            Failure::media(
                "E_PARTIAL",
                format!(
                    "{errors} finding(s); read manifest.jsonl in {}",
                    options.destination.display()
                ),
            ),
        ),
        Err(error) => report_failure(TOOL, error),
    }
}

fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Option<Options>, Failure> {
    let mut args = args.into_iter();
    let mut paths = Vec::new();
    let mut max_entries = 100_000;
    let mut max_bytes = 1024 * 1024 * 1024;
    let mut positional = false;
    while let Some(arg) = args.next() {
        if !positional {
            match arg.to_str() {
                Some("--help" | "-h") => return Ok(None),
                Some("--") => {
                    positional = true;
                    continue;
                }
                Some(flag @ ("--max-entries" | "--max-bytes")) => {
                    let value = args
                        .next()
                        .and_then(|v| v.to_str().and_then(|s| s.parse::<u64>().ok()));
                    let value = value.filter(|&v| v > 0).ok_or_else(|| {
                        Failure::usage(format!("{flag} requires a positive integer"))
                    })?;
                    if flag == "--max-entries" {
                        max_entries = value;
                    } else {
                        max_bytes = value;
                    }
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(Failure::usage(format!("unknown option {value:?}; {USAGE}")))
                }
                _ => {}
            }
        }
        paths.push(PathBuf::from(arg));
    }
    if paths.len() != 2 {
        return Err(Failure::usage(USAGE));
    }
    Ok(Some(Options {
        image: paths.remove(0),
        destination: paths.remove(0),
        max_entries,
        max_bytes,
    }))
}

fn execute(options: &Options) -> Result<usize, Failure> {
    let (mut device, header) = read_header(&options.image)?;
    let features = header.ident.features;
    // An extractor must not silently drop metadata it cannot represent.
    if features.incompat & !SUPPORTED_INCOMPAT_FEATURES != 0
        || features.ro_compat & !SUPPORTED_RO_COMPAT_FEATURES != 0
        || features.compat & !COMPAT_DATA_POLICY != 0
    {
        return Err(Failure::media(
            "E_FEATURE",
            "extraction requires understood feature semantics",
        ));
    }
    let log = afsplus_core::intent_log::scan(
        &mut device,
        &header.ident.geometry(),
        header.ident.log_slots,
        &header.ident.uuid,
        header.selection.chosen.generation,
        features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0,
    )
    .map_err(|e| core_failure("E_INTENT_LOG", "cannot inspect log tail", e))?;
    let mut volume = mount_with_options(
        device,
        MountOptions {
            mode: MountMode::NoChanges,
        },
    )
    .map_err(|e| core_failure("E_MOUNT", "cannot inspect selected checkpoint", e))?;
    fs::create_dir(&options.destination)
        .map_err(|e| Failure::host_io(format!("destination must be new: {e}")))?;
    extract(
        &mut volume,
        &options.destination,
        options.max_entries,
        options.max_bytes,
        log.tail_note.as_deref(),
    )
}

fn host(error: std::io::Error) -> Failure {
    Failure::host_io(error.to_string())
}

fn new_file(path: &Path) -> Result<File, Failure> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(host)
}

fn line(out: &mut impl Write, body: &str) -> Result<(), Failure> {
    writeln!(out, "{{\"schema_version\":1,\"tool\":\"{TOOL}\",{body}}}").map_err(host)
}

fn finding(out: &mut impl Write, object: u64, id: &str, detail: &str) -> Result<(), Failure> {
    line(
        out,
        &format!(
            "\"record\":\"finding\",\"object_id\":{object},\"id\":{},\"detail\":{}",
            json_string(id),
            json_string(detail)
        ),
    )
}

fn object_record(out: &mut impl Write, record: ObjectRecord) -> Result<(), Failure> {
    let kind = match record.object_type {
        ObjectType::File => "file",
        ObjectType::Directory => "directory",
        ObjectType::Symlink => "symlink",
        ObjectType::Internal => "internal",
    };
    line(out, &format!("\"record\":\"object\",\"object_id\":{},\"type\":\"{kind}\",\"flags\":{},\"link_count\":{},\"size_bytes\":{},\"allocated_bytes\":{},\"created\":[{},{}],\"modified\":[{},{}],\"changed\":[{},{}],\"protection\":{},\"content_generation\":{},\"data_root\":{},\"data_blocks\":{}",
        record.object_id, record.flags, record.link_count, record.size_bytes, record.allocated_bytes,
        record.created.seconds, record.created.nanoseconds, record.modified.seconds, record.modified.nanoseconds,
        record.changed.seconds, record.changed.nanoseconds, record.protection, record.content_generation, record.data_root, record.data_blocks))
}

fn extract<D: BlockDevice>(
    volume: &mut Volume<D>,
    destination: &Path,
    max_entries: u64,
    max_bytes: u64,
    log_tail: Option<&str>,
) -> Result<usize, Failure> {
    let mut manifest = BufWriter::new(new_file(&destination.join("manifest.jsonl"))?);
    line(&mut manifest, &format!("\"record\":\"header\",\"uuid\":{},\"generation\":{},\"scope\":\"selected-checkpoint\",\"data_integrity\":\"unchecked-payload\",\"max_entries\":{max_entries},\"max_bytes\":{max_bytes}", json_string(&uuid_hex(&volume.ident().uuid)), volume.generation()))?;
    let mut errors = 0;
    if let Some(detail) = log_tail {
        finding(&mut manifest, OBJECT_ROOT, "E_LOG_TAIL", detail)?;
        errors += 1;
    }
    if volume.pending_intent_records() != 0 {
        finding(
            &mut manifest,
            OBJECT_ROOT,
            "E_PENDING_LOG",
            "durable intent records are excluded from this checkpoint-only extraction",
        )?;
        errors += 1;
    }
    let mut queue = VecDeque::from([OBJECT_ROOT]);
    let mut seen = BTreeSet::from([OBJECT_ROOT]);
    let mut entries = 0;
    let mut bytes = 0;
    let mut files = 0;
    let mut buffer = vec![0; 64 * 1024];
    while let Some(id) = queue.pop_front() {
        let record = match volume.stat(id) {
            Ok(Some(record)) => record,
            result => {
                finding(
                    &mut manifest,
                    id,
                    "E_OBJECT",
                    &format!("unreadable object: {result:?}"),
                )?;
                errors += 1;
                continue;
            }
        };
        object_record(&mut manifest, record)?;
        match record.object_type {
            ObjectType::Directory => {
                let mut cursor = None;
                loop {
                    let page = match volume.read_directory_page(id, cursor, 64) {
                        Ok(page) => page,
                        Err(e) => {
                            finding(&mut manifest, id, "E_DIRECTORY", &e.to_string())?;
                            errors += 1;
                            break;
                        }
                    };
                    let mut limited = false;
                    for entry in page.entries {
                        if entries == max_entries {
                            finding(
                                &mut manifest,
                                id,
                                "E_ENTRY_LIMIT",
                                "directory enumeration truncated by entry budget",
                            )?;
                            errors += 1;
                            limited = true;
                            break;
                        }
                        entries += 1;
                        line(&mut manifest, &format!("\"record\":\"link\",\"parent_id\":{id},\"object_id\":{},\"name_hex\":{}", entry.child_id, json_string(&hex(&entry.name))))?;
                        if seen.insert(entry.child_id) {
                            queue.push_back(entry.child_id);
                        }
                    }
                    if limited || page.eof {
                        break;
                    }
                    if cursor.is_some_and(|previous| previous == page.next) {
                        finding(
                            &mut manifest,
                            id,
                            "E_DIRECTORY",
                            "directory cursor made no progress",
                        )?;
                        errors += 1;
                        break;
                    }
                    cursor = Some(page.next);
                }
            }
            ObjectType::File => {
                if record.size_bytes > max_bytes - bytes {
                    finding(
                        &mut manifest,
                        id,
                        "E_BYTE_LIMIT",
                        "file skipped because it exceeds remaining byte budget",
                    )?;
                    errors += 1;
                    continue;
                }
                let partial_name = format!("object-{id}.partial");
                let partial = destination.join(&partial_name);
                let mut output = new_file(&partial)?;
                let mut offset = 0;
                let mut complete = true;
                while offset < record.size_bytes {
                    let count = (record.size_bytes - offset).min(buffer.len() as u64) as usize;
                    match volume.read_file_at(id, offset, &mut buffer[..count]) {
                        Ok(count) if count > 0 => {
                            output.write_all(&buffer[..count]).map_err(host)?;
                            offset += count as u64;
                            bytes += count as u64;
                        }
                        result => {
                            finding(
                                &mut manifest,
                                id,
                                "E_DATA",
                                &format!("read stopped at byte {offset}: {result:?}"),
                            )?;
                            errors += 1;
                            complete = false;
                            break;
                        }
                    }
                }
                output.sync_all().map_err(host)?;
                drop(output);
                let name = if complete {
                    let name = format!("object-{id}.bin");
                    // Hard-link publication refuses an existing output pathname.
                    fs::hard_link(&partial, destination.join(&name)).map_err(host)?;
                    fs::remove_file(&partial).map_err(host)?;
                    files += 1;
                    name
                } else {
                    partial_name
                };
                line(&mut manifest, &format!("\"record\":\"content\",\"object_id\":{id},\"file\":{},\"bytes\":{offset},\"complete\":{complete}", json_string(&name)))?;
            }
            _ => {
                finding(
                    &mut manifest,
                    id,
                    "E_OBJECT_TYPE",
                    "object content type has no extraction implementation",
                )?;
                errors += 1;
            }
        }
    }
    line(&mut manifest, &format!("\"record\":\"summary\",\"files\":{files},\"bytes\":{bytes},\"entries\":{entries},\"findings\":{errors},\"complete\":{}", errors == 0))?;
    manifest.flush().map_err(host)?;
    manifest.get_ref().sync_all().map_err(host)?;
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use afsplus_block::{BlockError, MemoryBackend};
    use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
    use afsplus_format::{Timespec, DEFAULT_BLOCK_SIZE};

    struct ReadSpy {
        inner: MemoryBackend,
        fail_lba: u64,
    }
    impl BlockDevice for ReadSpy {
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }
        fn total_blocks(&self) -> u64 {
            self.inner.total_blocks()
        }
        fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
            if lba == self.fail_lba {
                return Err(BlockError::Injected("salvage data read"));
            }
            self.inner.read_block(lba, buf)
        }
        fn write_block(&mut self, _: u64, _: &[u8]) -> Result<(), BlockError> {
            panic!("source write attempted")
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            panic!("source flush attempted")
        }
    }

    #[test]
    fn partial_read_and_pending_log_are_reported_without_write_or_flush_attempts() {
        let mut device = MemoryBackend::new(DEFAULT_BLOCK_SIZE, 1024);
        let now = Timespec {
            seconds: 1,
            nanoseconds: 0,
        };
        mkfs(
            &mut device,
            &MkfsParams {
                uuid: [19; 16],
                label: "extract".into(),
                region_size: 4096,
                reclaim_caps: Default::default(),
                log_slots: 8,
                shared_extents: true,
                data_policy: false,
                name_policy: NamePolicy::Sensitive,
                timestamp: now,
            },
        )
        .unwrap();
        let mut volume = mount(device).unwrap();
        let data = vec![17; DEFAULT_BLOCK_SIZE * 17];
        let id = volume.create_file_in_root("data", &data, now).unwrap();
        let record = volume.stat(id).unwrap().unwrap();
        let fail_lba = record.data_root + 16;
        volume.window_write_file_at(id, 0, b"logged", now).unwrap();
        volume.window_fsync().unwrap();
        let mut volume = mount_with_options(
            ReadSpy {
                inner: volume.into_device(),
                fail_lba,
            },
            MountOptions {
                mode: MountMode::NoChanges,
            },
        )
        .unwrap();
        let destination =
            std::env::temp_dir().join(format!("afsplus-extract-partial-{}", std::process::id()));
        fs::create_dir(&destination).unwrap();
        let errors = extract(&mut volume, &destination, 100, 1024 * 1024, None).unwrap();
        assert_eq!(errors, 2);
        assert_eq!(
            fs::read(destination.join(format!("object-{id}.partial"))).unwrap(),
            data[..65536]
        );
        assert!(!destination.join(format!("object-{id}.bin")).exists());
        let manifest = fs::read_to_string(destination.join("manifest.jsonl")).unwrap();
        assert!(manifest.contains("E_PENDING_LOG"));
        assert!(manifest.contains("E_DATA"));
        assert!(manifest.contains("\"bytes\":65536,\"complete\":false"));
        fs::remove_dir_all(destination).unwrap();
    }
}
