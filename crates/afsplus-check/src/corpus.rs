//! Deterministic checker-corruption corpus.
//!
//! The corpus is generated from one valid, Rust-produced image, then applies
//! one localized integrity or semantic mutation per case.  Keeping the case
//! IDs, mutation coordinates and expected checker finding in a versioned
//! manifest gives filesystem implementers small artifacts they can replay
//! without reverse-engineering the test suite.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_core::volume::BatchOp;
use afsplus_core::{allocation_root, intent_log, mkfs, mount, object_map, MkfsParams};
use afsplus_format::bitmap::{BitmapPage, BITMAP_PAGE_BLOCKS};
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::ident::Identification;
use afsplus_format::intent_log::LogRecord;
use afsplus_format::region::RegionDescriptor;
use afsplus_format::{le, Timespec, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

use crate::{check_device, CheckReport, REPORT_SCHEMA_VERSION};

/// Version of the generated corpus manifest, independent of checker JSON.
pub const CORPUS_MANIFEST_VERSION: u32 = 1;

const TOTAL_BLOCKS: u64 = 256;
const REGION_BLOCKS: u32 = 64;
const CHECKPOINT_LBAS: [u64; 2] = [1, 2];

/// Where a checker finding is expected to appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingChannel {
    Error,
    Warning,
}

impl FindingChannel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// Stable oracle for one corrupted image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedFinding {
    pub channel: FindingChannel,
    pub message: &'static str,
}

/// One exact byte range changed from the reference image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusMutation {
    pub lba: u64,
    pub byte_offset: usize,
    pub byte_length: usize,
    pub operation: &'static str,
}

/// A generated image and its stable checker oracle.
#[derive(Debug, Clone)]
pub struct CorruptionCase {
    pub id: &'static str,
    pub surface: &'static str,
    pub description: &'static str,
    pub image: MemoryBackend,
    pub mutations: Vec<CorpusMutation>,
    pub expected_clean: bool,
    pub expected: ExpectedFinding,
}

#[derive(Debug, Clone, Copy)]
struct Layout {
    object_map_lba: u64,
    object_lba: u64,
    bitmap_lba: u64,
    bitmap_local_index: u32,
    log_lba: u64,
}

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn err(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

fn build_reference() -> Result<(MemoryBackend, Layout), String> {
    let mut image = MemoryBackend::new(DEFAULT_BLOCK_SIZE, TOTAL_BLOCKS);
    mkfs(
        &mut image,
        &MkfsParams {
            uuid: [0xC3; 16],
            label: "AFSPlusCorruptionCorpus".into(),
            region_size: REGION_BLOCKS,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: true,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: timestamp(1_800_000_000),
        },
    )
    .map_err(|error| err("mkfs reference image", error))?;

    let mut volume = mount(image).map_err(|error| err("mount reference image", error))?;
    let object_id = volume
        .create_file_in_root("anchor.bin", b"anchor", timestamp(1_800_000_001))
        .map_err(|error| err("create reference file", error))?;
    image = volume.into_device();

    let ident = Identification::decode(&image.peek(0))
        .map_err(|error| err("decode reference identification", error))?;
    let checkpoint = CHECKPOINT_LBAS
        .iter()
        .filter_map(|lba| {
            Checkpoint::decode(&image.peek(*lba), &ident.uuid)
                .ok()
                .map(|checkpoint| (*lba, checkpoint))
        })
        .max_by_key(|(_, checkpoint)| checkpoint.generation)
        .ok_or_else(|| "reference image has no checkpoint".to_owned())?
        .1;
    let object_lba = object_map::lookup_lba(
        &mut image,
        &ident.geometry(),
        checkpoint.object_map_block,
        checkpoint.generation,
        object_id,
    )
    .map_err(|error| err("locate reference object", error))?
    .ok_or_else(|| "reference object is absent from object map".to_owned())?;

    let geo = ident.geometry();
    let region = (object_lba / u64::from(geo.region_size)) as u32;
    let region_local = (object_lba % u64::from(geo.region_size)) as u32;
    let page_index = region_local / BITMAP_PAGE_BLOCKS;
    let allocation = allocation_root::lookup_record(
        &mut image,
        &geo,
        checkpoint.allocation_root_block,
        checkpoint.generation,
        region,
    )
    .map_err(|error| err("locate allocation record", error))?
    .ok_or_else(|| "reference allocation record is absent".to_owned())?;
    let descriptor_lba = geo.descriptor_slot_lba(region, allocation.descriptor_slot);
    let (descriptor, _) = RegionDescriptor::decode(&image.peek(descriptor_lba))
        .map_err(|error| err("decode reference region descriptor", error))?;
    let binding = descriptor
        .pages
        .get(page_index as usize)
        .ok_or_else(|| "reference bitmap binding is absent".to_owned())?;
    let bitmap_lba = geo.bitmap_slot_lba(region, page_index, binding.slot);
    let (bitmap, _) = BitmapPage::decode(&image.peek(bitmap_lba))
        .map_err(|error| err("decode reference bitmap", error))?;
    let bitmap_local_index = region_local
        .checked_sub(bitmap.first_block)
        .ok_or_else(|| "reference object precedes selected bitmap page".to_owned())?;
    if bitmap_local_index >= bitmap.valid_blocks || !bitmap.is_allocated(bitmap_local_index) {
        return Err("reference object is not allocated in selected bitmap".into());
    }

    let mut volume = mount(image).map_err(|error| err("remount reference image", error))?;
    volume
        .window_op(
            &BatchOp::CreateFile {
                parent_id: OBJECT_ROOT,
                name: "pending.bin",
                content: b"pending",
            },
            timestamp(1_800_000_002),
        )
        .map_err(|error| err("stage reference intent", error))?;
    volume
        .window_fsync()
        .map_err(|error| err("fsync reference intent", error))?;
    image = volume.into_device();
    let log_lba = *intent_log::log_slot_lbas(&geo, ident.log_slots)
        .map_err(|error| err("derive reference intent-log layout", error))?
        .first()
        .ok_or_else(|| "reference image has no intent-log slot".to_owned())?;

    let report = check_device(&mut image.clone());
    if !report.is_clean()
        || report
            .volume
            .as_ref()
            .map(|volume| volume.log_records_pending)
            != Some(1)
    {
        return Err(format!(
            "reference image is not checker-clean with one pending record: {:?}",
            report.errors
        ));
    }

    Ok((
        image,
        Layout {
            object_map_lba: checkpoint.object_map_block,
            object_lba,
            bitmap_lba,
            bitmap_local_index,
            log_lba,
        },
    ))
}

fn flip_byte(image: &mut MemoryBackend, lba: u64, offset: usize) -> Result<(), String> {
    let mut block = image.peek(lba);
    let byte = block
        .get_mut(offset)
        .ok_or_else(|| format!("mutation offset {offset} is outside block {lba}"))?;
    *byte ^= 0x80;
    image.apply_raw(lba, &block);
    Ok(())
}

fn mutate_sealed(
    image: &mut MemoryBackend,
    lba: u64,
    expected_type: u32,
    mutate: impl FnOnce(&mut [u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut block = image.peek(lba);
    let header = BlockHeader::verify(&block, expected_type)
        .map_err(|error| err("verify block before semantic mutation", error))?;
    mutate(&mut block)?;
    header.seal(&mut block);
    image.apply_raw(lba, &block);
    Ok(())
}

fn integrity_case(
    reference: &MemoryBackend,
    id: &'static str,
    surface: &'static str,
    description: &'static str,
    lbas: &[u64],
    expected_clean: bool,
    expected: ExpectedFinding,
) -> Result<CorruptionCase, String> {
    let mut image = reference.clone();
    let offset = HEADER_SIZE + 1;
    let mut mutations = Vec::new();
    for lba in lbas {
        flip_byte(&mut image, *lba, offset)?;
        mutations.push(CorpusMutation {
            lba: *lba,
            byte_offset: offset,
            byte_length: 1,
            operation: "xor-0x80-without-resealing",
        });
    }
    Ok(CorruptionCase {
        id,
        surface,
        description,
        image,
        mutations,
        expected_clean,
        expected,
    })
}

/// Builds the full deterministic corpus in memory.
pub fn build_corruption_corpus() -> Result<Vec<CorruptionCase>, String> {
    let (reference, layout) = build_reference()?;
    let error = |message| ExpectedFinding {
        channel: FindingChannel::Error,
        message,
    };
    let warning = |message| ExpectedFinding {
        channel: FindingChannel::Warning,
        message,
    };
    let mut cases = Vec::new();

    cases.push(integrity_case(
        &reference,
        "ident-checksum",
        "identification",
        "Identification payload changed without updating its block checksum.",
        &[0],
        false,
        error(
            "identification block invalid: checksum mismatch: stored 0x4e2d7dc5, computed 0xdc924a69",
        ),
    )?);
    {
        let mut image = reference.clone();
        let mut ident = Identification::decode(&image.peek(0))
            .map_err(|error| err("decode identification for feature mutation", error))?;
        ident.features.incompat |= 1u64 << 63;
        image.apply_raw(
            0,
            &ident
                .encode(DEFAULT_BLOCK_SIZE)
                .map_err(|error| err("encode identification feature mutation", error))?,
        );
        cases.push(CorruptionCase {
            id: "ident-unknown-incompat",
            surface: "identification",
            description: "Valid identification record advertises an unknown INCOMPAT bit.",
            image,
            mutations: vec![CorpusMutation {
                lba: 0,
                byte_offset: HEADER_SIZE + 153,
                byte_length: 8,
                operation: "set-unknown-incompat-and-reseal",
            }],
            expected_clean: false,
            expected: error("unsupported incompatible filesystem features: 0x8000000000000000"),
        });
    }

    cases.push(integrity_case(
        &reference,
        "checkpoint-checksums",
        "checkpoint",
        "Both retained checkpoint payloads changed without resealing.",
        &CHECKPOINT_LBAS,
        false,
        error(
            "no valid checkpoint (slot A: invalid: checksum mismatch: stored 0xcbbe4fcb, computed 0x59017867; slot B: invalid: checksum mismatch: stored 0xb8928542, computed 0x2a2db2ee)",
        ),
    )?);
    {
        let mut image = reference.clone();
        for lba in CHECKPOINT_LBAS {
            mutate_sealed(&mut image, lba, block_type::CHECKPOINT, |block| {
                le::put_u64(&mut block[HEADER_SIZE + 80..HEADER_SIZE + 88], 1);
                Ok(())
            })?;
        }
        cases.push(CorruptionCase {
            id: "checkpoint-reserved-flags",
            surface: "checkpoint",
            description: "Both valid-CRC checkpoints set a reserved payload flag.",
            image,
            mutations: CHECKPOINT_LBAS
                .iter()
                .map(|lba| CorpusMutation {
                    lba: *lba,
                    byte_offset: HEADER_SIZE + 80,
                    byte_length: 8,
                    operation: "set-reserved-payload-flag-and-reseal",
                })
                .collect(),
            expected_clean: false,
            expected: error(
                "no valid checkpoint (slot A: invalid: invalid structure: checkpoint payload reserved flags are nonzero; slot B: invalid: invalid structure: checkpoint payload reserved flags are nonzero)",
            ),
        });
    }

    cases.push(integrity_case(
        &reference,
        "tree-checksum",
        "tree",
        "Chosen object-map tree payload changed without resealing.",
        &[layout.object_map_lba],
        false,
        error(
            "chosen checkpoint generation 2 references invalid state: corrupt volume: tree node 28: checksum mismatch: stored 0x0614b8b3, computed 0x94ab8f1f",
        ),
    )?);
    {
        let mut image = reference.clone();
        mutate_sealed(
            &mut image,
            layout.object_map_lba,
            block_type::TREE_NODE,
            |block| {
                let range = HEADER_SIZE + 8..HEADER_SIZE + 16;
                let count = le::get_u64(&block[range.clone()]);
                le::put_u64(&mut block[range], count.saturating_add(1));
                Ok(())
            },
        )?;
        cases.push(CorruptionCase {
            id: "tree-leaf-accounting",
            surface: "tree",
            description: "Valid-CRC object-map leaf overstates its subtree item count.",
            image,
            mutations: vec![CorpusMutation {
                lba: layout.object_map_lba,
                byte_offset: HEADER_SIZE + 8,
                byte_length: 8,
                operation: "increment-subtree-items-and-reseal",
            }],
            expected_clean: false,
            expected: error(
                "chosen checkpoint generation 2 references invalid state: corrupt volume: tree node 28: invalid structure: leaf tree accounting is inconsistent",
            ),
        });
    }

    cases.push(integrity_case(
        &reference,
        "object-checksum",
        "object",
        "Mapped file-object payload changed without resealing.",
        &[layout.object_lba],
        false,
        error(
            "chosen checkpoint generation 2 references invalid state: format error: checksum mismatch: stored 0x8b28bb78, computed 0x19978cd4",
        ),
    )?);
    {
        let mut image = reference.clone();
        mutate_sealed(&mut image, layout.object_lba, block_type::OBJECT, |block| {
            le::put_u32(&mut block[HEADER_SIZE + 12..HEADER_SIZE + 16], 0);
            Ok(())
        })?;
        cases.push(CorruptionCase {
            id: "object-zero-link-count",
            surface: "object",
            description: "Valid-CRC mapped file object carries a zero link count.",
            image,
            mutations: vec![CorpusMutation {
                lba: layout.object_lba,
                byte_offset: HEADER_SIZE + 12,
                byte_length: 4,
                operation: "zero-link-count-and-reseal",
            }],
            expected_clean: false,
            expected: error(
                "chosen checkpoint generation 2 references invalid state: format error: invalid structure: link count zero without orphan support",
            ),
        });
    }

    cases.push(integrity_case(
        &reference,
        "bitmap-checksum",
        "bitmap",
        "Selected allocation-bitmap payload changed without resealing.",
        &[layout.bitmap_lba],
        false,
        error(
            "chosen checkpoint generation 2 references invalid state: corrupt volume: region 0 bitmap page 0: checksum mismatch: stored 0xabcc6fec, computed 0x39735840",
        ),
    )?);
    {
        let mut image = reference.clone();
        let (mut bitmap, generation) = BitmapPage::decode(&image.peek(layout.bitmap_lba))
            .map_err(|error| err("decode bitmap for ownership mutation", error))?;
        if !bitmap.set_allocated(layout.bitmap_local_index, false) {
            return Err("bitmap ownership mutation did not change its target bit".into());
        }
        image.apply_raw(
            layout.bitmap_lba,
            &bitmap
                .encode(DEFAULT_BLOCK_SIZE, generation)
                .map_err(|error| err("encode bitmap ownership mutation", error))?,
        );
        cases.push(CorruptionCase {
            id: "bitmap-reachable-block-free",
            surface: "bitmap",
            description: "Valid-CRC bitmap marks a mapped object-record block free.",
            image,
            mutations: vec![CorpusMutation {
                lba: layout.bitmap_lba,
                byte_offset: HEADER_SIZE
                    + afsplus_format::bitmap::BITMAP_FIXED_PAYLOAD
                    + layout.bitmap_local_index as usize / 8,
                byte_length: 1,
                operation: "clear-reachable-object-bit-and-reseal",
            }],
            expected_clean: false,
            expected: error(
                "chosen checkpoint generation 2 references invalid state: corrupt volume: region 0 bitmap page 0 free count mismatch: bitmap 35, descriptor 34",
            ),
        });
    }

    cases.push(integrity_case(
        &reference,
        "intent-checksum-tail",
        "intent-log",
        "First nonzero intent record changed without resealing; it is a crash boundary and a forensic warning.",
        &[layout.log_lba],
        true,
        warning(
            "intent log tail: log slot 0 contains an invalid nonzero record: checksum mismatch: stored 0x21895426, computed 0xb336638a",
        ),
    )?);
    {
        let mut image = reference.clone();
        let mut record = LogRecord::decode(&image.peek(layout.log_lba))
            .map_err(|error| err("decode intent record for sequence mutation", error))?;
        record.sequence = 2;
        image.apply_raw(
            layout.log_lba,
            &record
                .encode(DEFAULT_BLOCK_SIZE)
                .map_err(|error| err("encode intent sequence mutation", error))?,
        );
        cases.push(CorruptionCase {
            id: "intent-sequence-gap",
            surface: "intent-log",
            description: "Valid-CRC first intent record starts at sequence two.",
            image,
            mutations: vec![CorpusMutation {
                lba: layout.log_lba,
                byte_offset: HEADER_SIZE + 24,
                byte_length: 4,
                operation: "set-sequence-two-and-reseal",
            }],
            expected_clean: true,
            expected: warning("intent log tail: log slot 0 carries sequence 2, expected 1"),
        });
    }

    for case in &cases {
        validate_corruption_case(case)?;
    }
    Ok(cases)
}

/// Runs the checker and verifies one case's stable oracle.
pub fn validate_corruption_case(case: &CorruptionCase) -> Result<CheckReport, String> {
    let mut image = case.image.clone();
    let report = check_device(&mut image);
    if report.is_clean() != case.expected_clean {
        return Err(format!(
            "{}: expected clean={}, got errors {:?}",
            case.id, case.expected_clean, report.errors
        ));
    }
    let findings = match case.expected.channel {
        FindingChannel::Error => &report.errors,
        FindingChannel::Warning => &report.warnings,
    };
    if findings.as_slice() != [case.expected.message] {
        return Err(format!(
            "{}: expected exact {} finding {:?}; errors={:?}, warnings={:?}",
            case.id,
            case.expected.channel.as_str(),
            case.expected.message,
            report.errors,
            report.warnings
        ));
    }
    Ok(report)
}

fn write_sparse_image(path: &Path, image: &MemoryBackend) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| err(&format!("create {}", path.display()), error))?;
    for lba in 0..image.total_blocks() {
        let block = image.peek(lba);
        if block.iter().any(|byte| *byte != 0) {
            file.seek(SeekFrom::Start(lba * image.block_size() as u64))
                .and_then(|_| file.write_all(&block))
                .map_err(|error| err(&format!("write {}", path.display()), error))?;
        }
    }
    file.set_len(image.total_blocks() * image.block_size() as u64)
        .and_then(|()| file.sync_all())
        .map_err(|error| err(&format!("finish {}", path.display()), error))
}

fn json_string(value: &str) -> String {
    let mut encoded = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                encoded.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

fn render_manifest(cases: &[(CorruptionCase, CheckReport)]) -> String {
    let mut output = format!(
        "{{\"corpus_schema_version\":{CORPUS_MANIFEST_VERSION},\"checker_schema_version\":{REPORT_SCHEMA_VERSION},\"block_size\":{DEFAULT_BLOCK_SIZE},\"total_blocks\":{TOTAL_BLOCKS},\"cases\":["
    );
    for (index, (case, report)) in cases.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":{},\"surface\":{},\"description\":{},\"expected_clean\":{},\"expected_channel\":{},\"expected_message\":{},\"image\":{},\"report\":{},\"mutations\":[",
            json_string(case.id),
            json_string(case.surface),
            json_string(case.description),
            case.expected_clean,
            json_string(case.expected.channel.as_str()),
            json_string(case.expected.message),
            json_string(&format!("{}.img", case.id)),
            report.render_json(),
        ));
        for (mutation_index, mutation) in case.mutations.iter().enumerate() {
            if mutation_index != 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"lba\":{},\"byte_offset\":{},\"byte_length\":{},\"operation\":{}}}",
                mutation.lba,
                mutation.byte_offset,
                mutation.byte_length,
                json_string(mutation.operation),
            ));
        }
        output.push_str("]}");
    }
    output.push_str("]}");
    output.push('\n');
    output
}

/// Writes sparse images, exact checker reports and a versioned manifest.
/// Existing output is never replaced.
pub fn write_corruption_corpus(output: &Path) -> Result<usize, String> {
    if output.exists() {
        return Err(format!(
            "refusing to replace existing output {}",
            output.display()
        ));
    }
    let cases = build_corruption_corpus()?
        .into_iter()
        .map(|case| {
            let report = validate_corruption_case(&case)?;
            Ok((case, report))
        })
        .collect::<Result<Vec<_>, String>>()?;
    fs::create_dir(output).map_err(|error| err(&format!("create {}", output.display()), error))?;
    for (case, report) in &cases {
        write_sparse_image(&output.join(format!("{}.img", case.id)), &case.image)?;
        fs::write(
            output.join(format!("{}.report.json", case.id)),
            format!("{}\n", report.render_json()),
        )
        .map_err(|error| err("write checker report", error))?;
    }
    fs::write(output.join("manifest.json"), render_manifest(&cases))
        .map_err(|error| err("write corpus manifest", error))?;
    fs::write(
        output.join("README.txt"),
        "Deterministic AFS+ checker corruption corpus.\n\
         Each sparse image is generated from the same valid reference volume.\n\
         Replay with: cargo run -p afsplus-check --bin afsplus-check -- <case>.img --json\n\
         Compare the result with <case>.report.json and manifest.json.\n",
    )
    .map_err(|error| err("write corpus README", error))?;
    Ok(cases.len())
}
