// SPDX-License-Identifier: BSD-2-Clause
//! `afsplus-disk`: an AFS+ file system inside a GPT disk image, the way an
//! AROS machine boots from it.
//!
//! AROS gives its partitions GPT type GUIDs of the form
//! `{DosType}-BB67-46C5-AA4A-F502CA018E5E` and keeps the boot priority in the
//! low byte of the upper attribute word, beside its bootable bit
//! (`rom/partition/partitiongpt.c`). An AFS+ partition is therefore
//! `4146532B-BB67-46C5-AA4A-F502CA018E5E`, DosType 'AFS+'. `wrap` places a
//! formatted AFS+ image in such a partition; `extract` finds the partition by
//! that type and copies it out, for the checker and the other host tools.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::common::{report_failure, Failure, EXIT_OK};

const TOOL: &str = "afsplus-disk";
const USAGE: &str = "usage: afsplus-disk wrap [--bootpri N] [--name NAME] [--disk-guid HEX32] [--partition-guid HEX32] [--force] <disk-image> <afsplus-image>\n       afsplus-disk extract <disk-image> <new-afsplus-image>";

/// The DosType of AFS+ partitions and volumes, 'AFS+'.
pub const AFSPLUS_DOSTYPE: u32 = 0x4146_532B;
/// The AROS type GUID family, less its first field, in on-disk order.
const AROS_TYPE_TAIL: [u8; 12] = [
    0x67, 0xBB, 0xC5, 0x46, 0xAA, 0x4A, 0xF5, 0x02, 0xCA, 0x01, 0x8E, 0x5E,
];
const SECTOR: u64 = 512;
const ENTRY_SIZE: usize = 128;
const ENTRIES: usize = 128;
const ENTRY_SECTORS: u64 = (ENTRY_SIZE * ENTRIES) as u64 / SECTOR;
/// Partitions start and end on 1 MiB, as every current partitioning tool does.
const ALIGN: u64 = 1024 * 1024 / SECTOR;
const AROS_BOOTABLE: u64 = 1 << 60;
const HEADER_SIZE: u32 = 92;

/// The type GUID of an AROS partition with `dostype`, in on-disk order.
pub fn aros_type_guid(dostype: u32) -> [u8; 16] {
    let mut guid = [0u8; 16];
    guid[..4].copy_from_slice(&dostype.to_le_bytes());
    guid[4..].copy_from_slice(&AROS_TYPE_TAIL);
    guid
}

/// CRC-32 as GPT uses it (IEEE 802.3, reflected, 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// Where the partition of a wrapped disk lies, in sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub disk_sectors: u64,
    pub first: u64,
    pub last: u64,
}

impl Layout {
    /// A disk with one partition of `bytes`, aligned at both ends, and room
    /// after it for the backup table.
    pub fn for_partition(bytes: u64) -> Result<Self, Failure> {
        if bytes == 0 || !bytes.is_multiple_of(SECTOR) {
            return Err(Failure::usage(format!(
                "the AFS+ image is {bytes} bytes, not a positive multiple of {SECTOR}"
            )));
        }
        let sectors = bytes / SECTOR;
        let first = ALIGN;
        let last = first + sectors - 1;
        let disk_sectors = (last + 1).div_ceil(ALIGN) * ALIGN + ALIGN;
        Ok(Self {
            disk_sectors,
            first,
            last,
        })
    }
}

/// What `wrap` writes besides the partition content.
pub struct Wrap {
    pub bootpri: i8,
    pub name: String,
    pub disk_guid: [u8; 16],
    pub partition_guid: [u8; 16],
}

fn entry(layout: &Layout, wrap: &Wrap) -> [u8; ENTRY_SIZE] {
    let mut entry = [0u8; ENTRY_SIZE];
    entry[..16].copy_from_slice(&aros_type_guid(AFSPLUS_DOSTYPE));
    entry[16..32].copy_from_slice(&wrap.partition_guid);
    entry[32..40].copy_from_slice(&layout.first.to_le_bytes());
    entry[40..48].copy_from_slice(&layout.last.to_le_bytes());
    let attributes = AROS_BOOTABLE | (u64::from(wrap.bootpri as u8) << 32);
    entry[48..56].copy_from_slice(&attributes.to_le_bytes());
    for (index, unit) in wrap.name.encode_utf16().take(36).enumerate() {
        entry[56 + 2 * index..58 + 2 * index].copy_from_slice(&unit.to_le_bytes());
    }
    entry
}

fn header(layout: &Layout, disk_guid: &[u8; 16], backup: bool, entries_crc: u32) -> [u8; 512] {
    let last_sector = layout.disk_sectors - 1;
    let (current, other, table) = if backup {
        (last_sector, 1, last_sector - ENTRY_SECTORS)
    } else {
        (1, last_sector, 2)
    };
    let mut block = [0u8; 512];
    block[..8].copy_from_slice(b"EFI PART");
    block[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    block[12..16].copy_from_slice(&HEADER_SIZE.to_le_bytes());
    block[24..32].copy_from_slice(&current.to_le_bytes());
    block[32..40].copy_from_slice(&other.to_le_bytes());
    block[40..48].copy_from_slice(&(2 + ENTRY_SECTORS).to_le_bytes());
    block[48..56].copy_from_slice(&(last_sector - ENTRY_SECTORS - 1).to_le_bytes());
    block[56..72].copy_from_slice(disk_guid);
    block[72..80].copy_from_slice(&table.to_le_bytes());
    block[80..84].copy_from_slice(&(ENTRIES as u32).to_le_bytes());
    block[84..88].copy_from_slice(&(ENTRY_SIZE as u32).to_le_bytes());
    block[88..92].copy_from_slice(&entries_crc.to_le_bytes());
    let crc = crc32(&block[..HEADER_SIZE as usize]);
    block[16..20].copy_from_slice(&crc.to_le_bytes());
    block
}

fn protective_mbr(layout: &Layout) -> [u8; 512] {
    let mut block = [0u8; 512];
    let record = &mut block[446..462];
    record[1..4].copy_from_slice(&[0x00, 0x02, 0x00]);
    record[4] = 0xEE;
    record[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
    record[8..12].copy_from_slice(&1u32.to_le_bytes());
    let covered = (layout.disk_sectors - 1).min(u64::from(u32::MAX)) as u32;
    record[12..16].copy_from_slice(&covered.to_le_bytes());
    block[510] = 0x55;
    block[511] = 0xAA;
    block
}

/// The sectors `wrap` writes around the partition: the protective MBR and the
/// primary header and table at the start, the backup table and header at
/// the end, as (sector, bytes).
pub fn tables(layout: &Layout, wrap: &Wrap) -> Vec<(u64, Vec<u8>)> {
    let mut entries = vec![0u8; ENTRY_SIZE * ENTRIES];
    entries[..ENTRY_SIZE].copy_from_slice(&entry(layout, wrap));
    let entries_crc = crc32(&entries);
    let last_sector = layout.disk_sectors - 1;
    vec![
        (0, protective_mbr(layout).to_vec()),
        (
            1,
            header(layout, &wrap.disk_guid, false, entries_crc).to_vec(),
        ),
        (2, entries.clone()),
        (last_sector - ENTRY_SECTORS, entries),
        (
            last_sector,
            header(layout, &wrap.disk_guid, true, entries_crc).to_vec(),
        ),
    ]
}

/// Finds the AFS+ partition in a GPT disk: validates the primary header and
/// its table by their CRCs and returns the partition's first and last
/// sector. Exactly one AFS+ partition is accepted.
pub fn find_partition(disk: &mut File) -> Result<(u64, u64), Failure> {
    let mut block = [0u8; 512];
    read_at(disk, SECTOR, &mut block)?;
    if &block[..8] != b"EFI PART" {
        return Err(Failure::media("E_GPT", "no GPT header in sector 1"));
    }
    let size = u32::from_le_bytes(block[12..16].try_into().unwrap()) as usize;
    if !(HEADER_SIZE as usize..=512).contains(&size) {
        return Err(Failure::media("E_GPT", format!("GPT header size {size}")));
    }
    let stored = u32::from_le_bytes(block[16..20].try_into().unwrap());
    let mut copy = block;
    copy[16..20].fill(0);
    if crc32(&copy[..size]) != stored {
        return Err(Failure::media("E_GPT", "GPT header CRC mismatch"));
    }
    let table = u64::from_le_bytes(block[72..80].try_into().unwrap());
    let count = u32::from_le_bytes(block[80..84].try_into().unwrap()) as usize;
    let entry_size = u32::from_le_bytes(block[84..88].try_into().unwrap()) as usize;
    let entries_crc = u32::from_le_bytes(block[88..92].try_into().unwrap());
    if entry_size < ENTRY_SIZE || count == 0 || count * entry_size > 1 << 20 {
        return Err(Failure::media(
            "E_GPT",
            format!("{count} GPT entries of {entry_size} bytes"),
        ));
    }
    let mut entries = vec![0u8; count * entry_size];
    read_at(disk, table * SECTOR, &mut entries)?;
    if crc32(&entries) != entries_crc {
        return Err(Failure::media("E_GPT", "GPT partition table CRC mismatch"));
    }
    let wanted = aros_type_guid(AFSPLUS_DOSTYPE);
    let found: Vec<(u64, u64)> = entries
        .chunks(entry_size)
        .filter(|entry| entry[..16] == wanted)
        .map(|entry| {
            (
                u64::from_le_bytes(entry[32..40].try_into().unwrap()),
                u64::from_le_bytes(entry[40..48].try_into().unwrap()),
            )
        })
        .collect();
    match found.as_slice() {
        [(first, last)] if first <= last => Ok((*first, *last)),
        [_] => Err(Failure::media(
            "E_GPT",
            "the AFS+ partition ends before it starts",
        )),
        [] => Err(Failure::media("E_GPT", "no AFS+ partition")),
        _ => Err(Failure::media(
            "E_GPT",
            format!("{} AFS+ partitions; extract takes one", found.len()),
        )),
    }
}

fn read_at(file: &mut File, offset: u64, buf: &mut [u8]) -> Result<(), Failure> {
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(buf))
        .map_err(|error| Failure::host_io(format!("read at byte {offset}: {error}")))
}

fn write_at(file: &mut File, offset: u64, buf: &[u8]) -> Result<(), Failure> {
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.write_all(buf))
        .map_err(|error| Failure::host_io(format!("write at byte {offset}: {error}")))
}

fn copy(
    from: &mut File,
    from_offset: u64,
    to: &mut File,
    to_offset: u64,
    bytes: u64,
) -> Result<(), Failure> {
    let mut buffer = vec![0u8; 1 << 20];
    let mut done = 0;
    while done < bytes {
        let chunk = (bytes - done).min(buffer.len() as u64) as usize;
        read_at(from, from_offset + done, &mut buffer[..chunk])?;
        // Leave runs of zeros as holes: images are mostly empty.
        if buffer[..chunk].iter().any(|&byte| byte != 0) {
            write_at(to, to_offset + done, &buffer[..chunk])?;
        }
        done += chunk as u64;
    }
    Ok(())
}

fn random_guid() -> Result<[u8; 16], Failure> {
    let mut guid = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut guid))
        .map_err(|error| Failure::host_io(format!("/dev/urandom: {error}")))?;
    // Version 4, RFC 4122 variant; the version sits in the little-endian
    // third field.
    guid[7] = (guid[7] & 0x0F) | 0x40;
    guid[8] = (guid[8] & 0x3F) | 0x80;
    Ok(guid)
}

fn parse_guid(text: &str) -> Result<[u8; 16], Failure> {
    let digits: String = text.chars().filter(|c| *c != '-').collect();
    if digits.len() != 32 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Failure::usage(format!(
            "{text:?} is not 32 hexadecimal digits"
        )));
    }
    let mut guid = [0u8; 16];
    for (index, byte) in guid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[2 * index..2 * index + 2], 16).unwrap();
    }
    Ok(guid)
}

enum Command {
    Wrap {
        disk: PathBuf,
        image: PathBuf,
        wrap: Wrap,
        force: bool,
    },
    Extract {
        disk: PathBuf,
        image: PathBuf,
    },
}

fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Option<Command>, Failure> {
    let mut args = args.into_iter();
    let verb = match args.next() {
        None => return Ok(None),
        Some(verb) => verb,
    };
    let text = |value: Option<OsString>, flag: &str| -> Result<String, Failure> {
        value
            .ok_or_else(|| Failure::usage(format!("{flag} needs a value")))?
            .into_string()
            .map_err(|_| Failure::usage(format!("{flag} needs UTF-8")))
    };
    let mut paths = Vec::new();
    let mut bootpri: i8 = 0;
    let mut name = String::from("AFS+");
    let mut disk_guid = None;
    let mut partition_guid = None;
    let mut force = false;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--help" | "-h") => return Ok(None),
            Some("--bootpri") => {
                let value = text(args.next(), "--bootpri")?;
                bootpri = value
                    .parse()
                    .map_err(|_| Failure::usage("--bootpri takes -128 to 127"))?;
            }
            Some("--name") => {
                name = text(args.next(), "--name")?;
                if name.encode_utf16().count() > 36 {
                    return Err(Failure::usage("--name takes at most 36 UTF-16 units"));
                }
            }
            Some("--disk-guid") => {
                disk_guid = Some(parse_guid(&text(args.next(), "--disk-guid")?)?)
            }
            Some("--partition-guid") => {
                partition_guid = Some(parse_guid(&text(args.next(), "--partition-guid")?)?)
            }
            Some("--force") => force = true,
            Some(flag) if flag.starts_with("--") => {
                return Err(Failure::usage(format!("unknown option {flag}")))
            }
            _ => paths.push(PathBuf::from(argument)),
        }
    }
    let [disk, image]: [PathBuf; 2] = paths.try_into().map_err(|_| Failure::usage(USAGE))?;
    match verb.to_str() {
        Some("wrap") => Ok(Some(Command::Wrap {
            disk,
            image,
            wrap: Wrap {
                bootpri,
                name,
                disk_guid: match disk_guid {
                    Some(guid) => guid,
                    None => random_guid()?,
                },
                partition_guid: match partition_guid {
                    Some(guid) => guid,
                    None => random_guid()?,
                },
            },
            force,
        })),
        Some("extract") => Ok(Some(Command::Extract { disk, image })),
        _ => Err(Failure::usage(USAGE)),
    }
}

fn create(path: &Path, force: bool) -> Result<File, Failure> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options
        .open(path)
        .map_err(|error| Failure::host_io(format!("cannot create {}: {error}", path.display())))
}

fn execute(command: Command) -> Result<(), Failure> {
    match command {
        Command::Wrap {
            disk,
            image,
            wrap,
            force,
        } => {
            let mut source = File::open(&image).map_err(|error| {
                Failure::host_io(format!("cannot open {}: {error}", image.display()))
            })?;
            let bytes = source
                .metadata()
                .map_err(|error| Failure::host_io(format!("{}: {error}", image.display())))?
                .len();
            let layout = Layout::for_partition(bytes)?;
            let mut target = create(&disk, force)?;
            target
                .set_len(layout.disk_sectors * SECTOR)
                .map_err(|error| Failure::host_io(format!("{}: {error}", disk.display())))?;
            copy(&mut source, 0, &mut target, layout.first * SECTOR, bytes)?;
            for (sector, content) in tables(&layout, &wrap) {
                write_at(&mut target, sector * SECTOR, &content)?;
            }
            target
                .sync_all()
                .map_err(|error| Failure::host_io(format!("{}: {error}", disk.display())))?;
            println!(
                "wrapped {} in {}: AFS+ partition sectors {}..={} of {}, boot priority {}",
                image.display(),
                disk.display(),
                layout.first,
                layout.last,
                layout.disk_sectors,
                wrap.bootpri
            );
            Ok(())
        }
        Command::Extract { disk, image } => {
            let mut source = File::open(&disk).map_err(|error| {
                Failure::host_io(format!("cannot open {}: {error}", disk.display()))
            })?;
            let (first, last) = find_partition(&mut source)?;
            let bytes = (last - first + 1) * SECTOR;
            let mut target = create(&image, false)?;
            target
                .set_len(bytes)
                .map_err(|error| Failure::host_io(format!("{}: {error}", image.display())))?;
            copy(&mut source, first * SECTOR, &mut target, 0, bytes)?;
            println!(
                "extracted the AFS+ partition, sectors {first}..={last}, to {}",
                image.display()
            );
            Ok(())
        }
    }
}

pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> u8 {
    match parse(args) {
        Ok(None) => {
            println!("{USAGE}\nPlace an AFS+ image in a bootable AROS GPT partition, or take it out again.");
            EXIT_OK
        }
        Ok(Some(command)) => match execute(command) {
            Ok(()) => EXIT_OK,
            Err(error) => report_failure(TOOL, error),
        },
        Err(error) => report_failure(TOOL, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_the_ieee_one() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn the_afsplus_type_guid_carries_its_dostype_first() {
        // 4146532B-BB67-46C5-AA4A-F502CA018E5E in GPT's mixed-endian order.
        assert_eq!(
            aros_type_guid(AFSPLUS_DOSTYPE),
            [
                0x2B, 0x53, 0x46, 0x41, 0x67, 0xBB, 0xC5, 0x46, 0xAA, 0x4A, 0xF5, 0x02, 0xCA, 0x01,
                0x8E, 0x5E
            ]
        );
    }

    #[test]
    fn the_partition_is_aligned_and_leaves_room_for_the_backup_table() {
        let layout = Layout::for_partition(64 * 1024 * 1024).unwrap();
        assert_eq!(layout.first, 2048);
        assert_eq!(layout.last, 2048 + 131_072 - 1);
        assert_eq!(layout.disk_sectors % ALIGN, 0);
        assert!(layout.disk_sectors - 1 - ENTRY_SECTORS > layout.last);
    }
}
