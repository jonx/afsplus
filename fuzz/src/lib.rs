//! Deterministic, replayable fuzz targets for the `afsplus-format` codecs.

mod checkpoint_snapshot;
mod legacy;
mod object_payload;
mod reclaim;
mod snapshot;

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use afsplus_format::bitmap::{BitmapPage, BITMAP_PAGE_BLOCKS};
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::geometry::Geometry;
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::ident::{
    FeatureFlags, Identification, NameKeyAlgorithm, COMPAT_DATA_POLICY, INCOMPAT_INTENT_LOG,
    INCOMPAT_INTENT_LOG_DATA_UPDATES, RO_COMPAT_SHARED_EXTENTS, UNICODE_VERSION_16_0_0,
};
use afsplus_format::intent_log::{LogOp, LogRecord};
use afsplus_format::object::{ObjectRecord, ObjectType};
use afsplus_format::region::{BitmapBinding, RegionDescriptor};
use afsplus_format::tree::{TreeItem, TreeKind, TreeNode};
use afsplus_format::{Timespec, DEFAULT_BLOCK_SHIFT, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

pub const SEED_SCHEMA_VERSION: u16 = 1;
pub const ARTIFACT_SCHEMA_VERSION: u16 = 1;

const ARTIFACT_MAGIC: [u8; 4] = *b"AFRF";
const ARTIFACT_HEADER_SIZE: usize = 24;
const MAX_ARTIFACT_BYTES: usize = DEFAULT_BLOCK_SIZE + 64;
const UUID: [u8; 16] = [0x5A; 16];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum CodecTarget {
    Identification = 1,
    Checkpoint = 2,
    TreeNode = 3,
    ObjectRecord = 4,
    IntentLog = 5,
    BitmapPage = 6,
    RegionDescriptor = 7,
    SnapshotRegistry = 8,
    SnapshotRecord = 9,
    SnapshotLifetime = 10,
    SnapshotLedger = 11,
    SnapshotKey = 12,
    ReclaimRoot = 13,
    ReclaimSegment = 14,
    ReclaimTable = 15,
    SnapshotCheckpoint = 16,
    InlineSymlink = 17,
    ObjectMetadata = 18,
    LegacyDirectory = 19,
    LegacyObjectMap = 20,
    LegacyRetired = 21,
}

impl CodecTarget {
    pub const ALL: [Self; 21] = [
        Self::Identification,
        Self::Checkpoint,
        Self::TreeNode,
        Self::ObjectRecord,
        Self::IntentLog,
        Self::BitmapPage,
        Self::RegionDescriptor,
        Self::SnapshotRegistry,
        Self::SnapshotRecord,
        Self::SnapshotLifetime,
        Self::SnapshotLedger,
        Self::SnapshotKey,
        Self::ReclaimRoot,
        Self::ReclaimSegment,
        Self::ReclaimTable,
        Self::SnapshotCheckpoint,
        Self::InlineSymlink,
        Self::ObjectMetadata,
        Self::LegacyDirectory,
        Self::LegacyObjectMap,
        Self::LegacyRetired,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Identification => "identification",
            Self::Checkpoint => "checkpoint",
            Self::TreeNode => "tree-node",
            Self::ObjectRecord => "object-record",
            Self::IntentLog => "intent-log",
            Self::BitmapPage => "bitmap-page",
            Self::RegionDescriptor => "region-descriptor",
            Self::SnapshotRegistry => "snapshot-registry",
            Self::SnapshotRecord => "snapshot-record",
            Self::SnapshotLifetime => "snapshot-lifetime",
            Self::SnapshotLedger => "snapshot-ledger",
            Self::SnapshotKey => "snapshot-key",
            Self::ReclaimRoot => "reclaim-root",
            Self::ReclaimSegment => "reclaim-segment",
            Self::ReclaimTable => "reclaim-table",
            Self::SnapshotCheckpoint => "snapshot-checkpoint",
            Self::InlineSymlink => "inline-symlink",
            Self::ObjectMetadata => "object-metadata",
            Self::LegacyDirectory => "legacy-directory",
            Self::LegacyObjectMap => "legacy-object-map",
            Self::LegacyRetired => "legacy-retired",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|target| target.name() == name)
    }

    fn from_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|target| *target as u8 == id)
    }

    fn block_type(self) -> u32 {
        match self {
            Self::Identification => block_type::IDENTIFICATION,
            Self::Checkpoint | Self::SnapshotCheckpoint => block_type::CHECKPOINT,
            Self::TreeNode => block_type::TREE_NODE,
            Self::ObjectRecord | Self::InlineSymlink | Self::ObjectMetadata => block_type::OBJECT,
            Self::IntentLog => block_type::INTENT_LOG,
            Self::BitmapPage => block_type::BITMAP,
            Self::RegionDescriptor => block_type::REGION_DESCRIPTOR,
            Self::LegacyDirectory => block_type::DIRECTORY,
            Self::LegacyObjectMap => block_type::OBJECT_MAP,
            Self::LegacyRetired => block_type::RETIRED,
            Self::ReclaimRoot => block_type::RECLAIM_ROOT,
            Self::ReclaimSegment => block_type::RECLAIM_SEGMENT,
            Self::ReclaimTable => block_type::RECLAIM_TABLE,
            _ => unreachable!("snapshot leaf values have no block header"),
        }
    }
}

impl fmt::Display for CodecTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzArtifact {
    pub target: CodecTarget,
    pub case: u64,
    pub input: Vec<u8>,
}

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 123_456_789,
    }
}

fn identification_seed() -> Result<Vec<u8>, String> {
    Identification {
        uuid: UUID,
        block_shift: DEFAULT_BLOCK_SHIFT,
        checksum_algorithm: afsplus_format::crc32c::CHECKSUM_CRC32C,
        region_size: 4096,
        log_slots: 8,
        features: FeatureFlags {
            compat: COMPAT_DATA_POLICY,
            ro_compat: RO_COMPAT_SHARED_EXTENTS,
            incompat: INCOMPAT_INTENT_LOG | INCOMPAT_INTENT_LOG_DATA_UPDATES,
        },
        name_key_algorithm: NameKeyAlgorithm::UnicodeNfcCasefold,
        unicode_version: UNICODE_VERSION_16_0_0,
        total_blocks: 8192,
        checkpoint_slots: [1, 2],
        metadata_start: 9,
        label: "CodecFuzz".into(),
    }
    .encode(DEFAULT_BLOCK_SIZE)
    .map_err(|error| format!("identification seed: {error}"))
}

fn checkpoint_seed() -> Result<Vec<u8>, String> {
    Checkpoint {
        uuid: UUID,
        generation: 7,
        root_object_id: OBJECT_ROOT,
        object_map_block: 32,
        allocation_root_block: 33,
        reclaim_root_block: 34,
        next_object_id: 42,
        committed_tx_id: 6,
        free_blocks_total: 8000,
        flags: 0,
        shared_extent_root_block: 35,
        snapshot_roots: None,
    }
    .encode(DEFAULT_BLOCK_SIZE)
    .map_err(|error| format!("checkpoint seed: {error}"))
}

fn tree_seed() -> Result<Vec<u8>, String> {
    TreeNode {
        kind: TreeKind::ObjectMap,
        owner: 0,
        level: 0,
        subtree_items: 2,
        leftmost_child: 0,
        leftmost_items: 0,
        items: vec![
            TreeItem {
                key: 1u64.to_be_bytes().to_vec(),
                value: 100u64.to_le_bytes().to_vec(),
            },
            TreeItem {
                key: 16u64.to_be_bytes().to_vec(),
                value: 101u64.to_le_bytes().to_vec(),
            },
        ],
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .map_err(|error| format!("tree seed: {error}"))
}

fn object_seed() -> Result<Vec<u8>, String> {
    ObjectRecord {
        object_id: 16,
        object_type: ObjectType::File,
        flags: 0,
        link_count: 2,
        size_bytes: 3000,
        allocated_bytes: DEFAULT_BLOCK_SIZE as u64,
        created: timestamp(1),
        modified: timestamp(2),
        changed: timestamp(3),
        protection: 0x1020_3040,
        content_generation: 4,
        data_root: 128,
        data_blocks: 1,
        security: None,
    }
    .encode(DEFAULT_BLOCK_SIZE, 7)
    .map_err(|error| format!("object seed: {error}"))
}

fn intent_seed() -> Result<Vec<u8>, String> {
    LogRecord {
        uuid: UUID,
        base_generation: 7,
        sequence: 1,
        ops: vec![
            LogOp::Create {
                parent_id: OBJECT_ROOT,
                name: b"pending.bin".to_vec(),
                expected_object_id: 42,
                size_bytes: 5000,
                content_crc: 0x1122_3344,
                timestamp: timestamp(4),
                extents: vec![(500, 2)],
            },
            LogOp::Delete {
                parent_id: OBJECT_ROOT,
                name: b"old.bin".to_vec(),
                timestamp: timestamp(5),
            },
            LogOp::Rename {
                source_parent_id: OBJECT_ROOT,
                source_name: b"pending.bin".to_vec(),
                target_parent_id: OBJECT_ROOT,
                target_name: b"final.bin".to_vec(),
                replace: true,
                timestamp: timestamp(6),
            },
            LogOp::Write {
                object_id: 16,
                logical_start: 1,
                expected_size_bytes: 8192,
                new_size_bytes: 12_000,
                content_crc: 0x5566_7788,
                timestamp: timestamp(7),
                extents: vec![(600, 2)],
            },
            LogOp::Truncate {
                object_id: 16,
                logical_start: 1,
                expected_size_bytes: 12_000,
                new_size_bytes: 5000,
                content_crc: 0x99AA_BBCC,
                timestamp: timestamp(8),
                extents: vec![(700, 1)],
            },
        ],
    }
    .encode(DEFAULT_BLOCK_SIZE)
    .map_err(|error| format!("intent-log seed: {error}"))
}

fn allocation_geometry() -> Geometry {
    Geometry {
        block_size: DEFAULT_BLOCK_SIZE,
        total_blocks: 2 * 65536 - 3,
        region_size: 65536,
    }
}

fn bitmap_seed_page() -> BitmapPage {
    let geo = allocation_geometry();
    let page_index = geo.bitmap_page_count(1) - 1;
    let valid = geo.bitmap_page_valid_blocks(1, page_index);
    let mut page = BitmapPage::all_free(1, page_index, page_index * BITMAP_PAGE_BLOCKS, valid);
    page.set_allocated(0, true);
    page.set_allocated(valid - 1, true);
    page
}

fn region_seed_descriptor() -> RegionDescriptor {
    let geo = allocation_geometry();
    let pages: Vec<_> = (0..geo.bitmap_page_count(1))
        .map(|index| BitmapBinding {
            slot: (index % 3) as u8,
            free_blocks: if index == 0 {
                geo.bitmap_page_valid_blocks(1, index) - geo.region_reserved_blocks(1) as u32
            } else if index + 1 == geo.bitmap_page_count(1) {
                bitmap_seed_page().free_blocks()
            } else {
                geo.bitmap_page_valid_blocks(1, index)
            },
            generation: 5 + u64::from(index % 3),
        })
        .collect();
    RegionDescriptor {
        region: 1,
        valid_blocks: geo.region_valid_blocks(1),
        free_blocks: pages.iter().map(|p| p.free_blocks).sum(),
        pages,
    }
}

fn accepts(target: CodecTarget, input: &[u8]) -> bool {
    if legacy::handles(target) {
        return legacy::accepts(target, input);
    }
    if object_payload::handles(target) {
        return object_payload::accepts(target, input);
    }
    if target == CodecTarget::SnapshotCheckpoint {
        return checkpoint_snapshot::accepts(input);
    }
    if reclaim::handles(target) {
        return reclaim::accepts(target, input);
    }
    if snapshot::handles(target) {
        return snapshot::accepts(target, input);
    }
    match target {
        CodecTarget::Identification => Identification::decode(input).is_ok(),
        CodecTarget::Checkpoint => Checkpoint::decode(input, &UUID).is_ok_and(|value| {
            value
                .validate_structural(&Geometry {
                    block_size: DEFAULT_BLOCK_SIZE,
                    total_blocks: 8192,
                    region_size: 4096,
                })
                .is_ok()
        }),
        CodecTarget::TreeNode => TreeNode::decode(input).is_ok(),
        CodecTarget::ObjectRecord => ObjectRecord::decode_with_generation(input).is_ok(),
        CodecTarget::IntentLog => LogRecord::decode(input).is_ok(),
        CodecTarget::BitmapPage => BitmapPage::decode(input).is_ok(),
        CodecTarget::RegionDescriptor => RegionDescriptor::decode(input)
            .is_ok_and(|(d, generation)| d.validate(&allocation_geometry(), 1, generation).is_ok()),
        _ => unreachable!(),
    }
}

pub fn canonical_seed(target: CodecTarget) -> Result<Vec<u8>, String> {
    if legacy::handles(target) {
        return legacy::seed(target);
    }
    if object_payload::handles(target) {
        return object_payload::seed(target);
    }
    if target == CodecTarget::SnapshotCheckpoint {
        return checkpoint_snapshot::seed();
    }
    if reclaim::handles(target) {
        return reclaim::seed(target);
    }
    if snapshot::handles(target) {
        return snapshot::seed(target);
    }
    match target {
        CodecTarget::Identification => identification_seed(),
        CodecTarget::Checkpoint => checkpoint_seed(),
        CodecTarget::TreeNode => tree_seed(),
        CodecTarget::ObjectRecord => object_seed(),
        CodecTarget::IntentLog => intent_seed(),
        CodecTarget::BitmapPage => bitmap_seed_page()
            .encode(DEFAULT_BLOCK_SIZE, 7)
            .map_err(|e| e.to_string()),
        CodecTarget::RegionDescriptor => region_seed_descriptor()
            .encode(DEFAULT_BLOCK_SIZE, 7)
            .map_err(|e| e.to_string()),
        _ => unreachable!(),
    }
}

fn roundtrip(target: CodecTarget, input: &[u8]) -> Result<(), String> {
    if legacy::handles(target) {
        return legacy::exercise(target, input);
    }
    if object_payload::handles(target) {
        return object_payload::exercise(target, input);
    }
    if target == CodecTarget::SnapshotCheckpoint {
        return checkpoint_snapshot::exercise(input);
    }
    if reclaim::handles(target) {
        return reclaim::exercise(target, input);
    }
    if snapshot::handles(target) {
        return snapshot::exercise(target, input);
    }
    match target {
        CodecTarget::Identification => {
            if let Ok(decoded) = Identification::decode(input) {
                let canonical = decoded
                    .encode(DEFAULT_BLOCK_SIZE)
                    .map_err(|error| format!("decoded identification cannot encode: {error}"))?;
                let second = Identification::decode(&canonical)
                    .map_err(|error| format!("canonical identification cannot decode: {error}"))?;
                if second != decoded
                    || second
                        .encode(DEFAULT_BLOCK_SIZE)
                        .map_err(|error| format!("second identification encode: {error}"))?
                        != canonical
                {
                    return Err("identification canonical round-trip mismatch".into());
                }
            }
        }
        CodecTarget::Checkpoint => {
            if let Ok(decoded) = Checkpoint::decode(input, &UUID) {
                let geometry = Geometry {
                    block_size: DEFAULT_BLOCK_SIZE,
                    total_blocks: 8192,
                    region_size: 4096,
                };
                if decoded.validate_structural(&geometry).is_err() {
                    return Ok(());
                }
                let canonical = decoded
                    .encode(DEFAULT_BLOCK_SIZE)
                    .map_err(|error| format!("decoded checkpoint cannot encode: {error}"))?;
                let second = Checkpoint::decode(&canonical, &UUID)
                    .map_err(|error| format!("canonical checkpoint cannot decode: {error}"))?;
                if second != decoded
                    || second
                        .encode(DEFAULT_BLOCK_SIZE)
                        .map_err(|error| format!("second checkpoint encode: {error}"))?
                        != canonical
                {
                    return Err("checkpoint canonical round-trip mismatch".into());
                }
            }
        }
        CodecTarget::TreeNode => {
            if let Ok((decoded, generation)) = TreeNode::decode(input) {
                let canonical = decoded
                    .encode(DEFAULT_BLOCK_SIZE, generation)
                    .map_err(|error| format!("decoded tree cannot encode: {error}"))?;
                let second = TreeNode::decode(&canonical)
                    .map_err(|error| format!("canonical tree cannot decode: {error}"))?;
                if second != (decoded.clone(), generation)
                    || second
                        .0
                        .encode(DEFAULT_BLOCK_SIZE, second.1)
                        .map_err(|error| format!("second tree encode: {error}"))?
                        != canonical
                {
                    return Err("tree canonical round-trip mismatch".into());
                }
            }
        }
        CodecTarget::ObjectRecord => {
            if let Ok((decoded, generation)) = ObjectRecord::decode_with_generation(input) {
                let canonical = decoded
                    .encode(DEFAULT_BLOCK_SIZE, generation)
                    .map_err(|error| format!("decoded object cannot encode: {error}"))?;
                let second = ObjectRecord::decode_with_generation(&canonical)
                    .map_err(|error| format!("canonical object cannot decode: {error}"))?;
                if second != (decoded, generation)
                    || second
                        .0
                        .encode(DEFAULT_BLOCK_SIZE, second.1)
                        .map_err(|error| format!("second object encode: {error}"))?
                        != canonical
                {
                    return Err("object canonical round-trip mismatch".into());
                }
            }
        }
        CodecTarget::BitmapPage => {
            if let Ok((decoded, generation)) = BitmapPage::decode(input) {
                let free = (0..decoded.valid_blocks)
                    .filter(|index| decoded.bits[*index as usize / 8] & (1 << (*index % 8)) == 0)
                    .count() as u32;
                if free != decoded.free_blocks() {
                    return Err("bitmap free count mismatch".into());
                }
                let mut edited = decoded.clone();
                for index in [0, decoded.valid_blocks - 1] {
                    let was = edited.is_allocated(index);
                    let before = edited.free_blocks();
                    if !edited.set_allocated(index, !was)
                        || edited.is_allocated(index) == was
                        || edited.free_blocks() != if was { before + 1 } else { before - 1 }
                    {
                        return Err("bitmap state/count update mismatch".into());
                    }
                }
                let canonical = decoded
                    .encode(DEFAULT_BLOCK_SIZE, generation)
                    .map_err(|e| e.to_string())?;
                let second = BitmapPage::decode(&canonical).map_err(|e| e.to_string())?;
                if second != (decoded, generation)
                    || second
                        .0
                        .encode(DEFAULT_BLOCK_SIZE, second.1)
                        .map_err(|e| e.to_string())?
                        != canonical
                {
                    return Err("bitmap canonical round-trip mismatch".into());
                }
            }
        }
        CodecTarget::RegionDescriptor => {
            if let Ok((decoded, generation)) = RegionDescriptor::decode(input) {
                // Decode is structural; the allocator additionally validates
                // a descriptor against the selected region's trusted geometry.
                if decoded
                    .validate(&allocation_geometry(), 1, generation)
                    .is_err()
                {
                    return Ok(());
                }
                let canonical = decoded
                    .encode(DEFAULT_BLOCK_SIZE, generation)
                    .map_err(|e| e.to_string())?;
                let second = RegionDescriptor::decode(&canonical).map_err(|e| e.to_string())?;
                if second != (decoded, generation)
                    || second
                        .0
                        .encode(DEFAULT_BLOCK_SIZE, second.1)
                        .map_err(|e| e.to_string())?
                        != canonical
                {
                    return Err("region canonical round-trip mismatch".into());
                }
            }
        }
        CodecTarget::IntentLog => {
            if let Ok(decoded) = LogRecord::decode(input) {
                let canonical = decoded
                    .encode(DEFAULT_BLOCK_SIZE)
                    .map_err(|error| format!("decoded intent log cannot encode: {error}"))?;
                let second = LogRecord::decode(&canonical)
                    .map_err(|error| format!("canonical intent log cannot decode: {error}"))?;
                if second != decoded
                    || second
                        .encode(DEFAULT_BLOCK_SIZE)
                        .map_err(|error| format!("second intent-log encode: {error}"))?
                        != canonical
                {
                    return Err("intent-log canonical round-trip mismatch".into());
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

pub fn mutated_input(target: CodecTarget, case: u64) -> Result<Vec<u8>, String> {
    let seed = canonical_seed(target)?;
    if case == 0 {
        return Ok(seed);
    }
    if snapshot::handles(target) {
        return Ok(snapshot::mutate(target, case, seed));
    }
    let random = splitmix64(case ^ ((target as u64) << 56) ^ u64::from(SEED_SCHEMA_VERSION));
    let index = random as usize % seed.len();
    let bit = 1u8 << ((random >> 32) & 7);
    let mut input = seed.clone();
    match case % 6 {
        0 => {
            input[index] ^= bit;
        }
        1 => {
            let header = BlockHeader::verify(&seed, target.block_type())
                .map_err(|error| format!("verify canonical seed: {error}"))?;
            let payload_index = HEADER_SIZE + random as usize % header.payload_len as usize;
            input[payload_index] ^= bit;
            header.seal(&mut input);
        }
        2 => {
            input.truncate(random as usize % (seed.len() + 1));
        }
        3 => {
            let count = ((random >> 40) as usize % 16) + 1;
            for step in 0..count {
                let at = splitmix64(random ^ step as u64) as usize % input.len();
                input[at] = input[at].wrapping_add((step as u8).wrapping_add(1));
            }
        }
        4 => {
            let header = BlockHeader::verify(&seed, target.block_type())
                .map_err(|error| format!("verify canonical seed: {error}"))?;
            let payload_index = HEADER_SIZE + random as usize % header.payload_len as usize;
            let count = ((random >> 48) as usize % 16) + 1;
            let payload_end = HEADER_SIZE + header.payload_len as usize;
            let end = payload_index.saturating_add(count).min(payload_end);
            input[payload_index..end].fill((random >> 24) as u8);
            header.seal(&mut input);
        }
        5 => {
            let extra = ((random >> 40) as usize % 64) + 1;
            input.extend((0..extra).map(|offset| splitmix64(random ^ offset as u64) as u8));
        }
        _ => unreachable!(),
    }
    Ok(input)
}

/// Exercises one decoder. Rejected inputs are successful fuzz outcomes;
/// accepted inputs must encode and decode to one stable canonical block.
pub fn exercise(target: CodecTarget, input: &[u8]) -> Result<(), String> {
    match catch_unwind(AssertUnwindSafe(|| roundtrip(target, input))) {
        Ok(result) => result,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("non-string panic payload");
            Err(format!("{target} decoder panicked: {detail}"))
        }
    }
}

pub fn run_case(target: CodecTarget, case: u64) -> Result<FuzzArtifact, String> {
    let input = mutated_input(target, case)?;
    if case == 0 {
        match catch_unwind(AssertUnwindSafe(|| accepts(target, &input))) {
            Ok(true) => {}
            Ok(false) => return Err(format!("{target}: canonical seed was rejected")),
            Err(_) => return Err(format!("{target}: canonical seed decoder panicked")),
        }
    }
    exercise(target, &input)?;
    Ok(FuzzArtifact {
        target,
        case,
        input,
    })
}

pub fn write_artifact(path: &Path, artifact: &FuzzArtifact) -> Result<(), String> {
    if artifact.input.len() > MAX_ARTIFACT_BYTES || artifact.input.len() > u32::MAX as usize {
        return Err("fuzz artifact input exceeds the bounded format".into());
    }
    let mut encoded = Vec::with_capacity(ARTIFACT_HEADER_SIZE + artifact.input.len());
    encoded.extend_from_slice(&ARTIFACT_MAGIC);
    encoded.extend_from_slice(&ARTIFACT_SCHEMA_VERSION.to_le_bytes());
    encoded.extend_from_slice(&SEED_SCHEMA_VERSION.to_le_bytes());
    encoded.push(artifact.target as u8);
    encoded.extend_from_slice(&[0; 3]);
    encoded.extend_from_slice(&artifact.case.to_le_bytes());
    encoded.extend_from_slice(&(artifact.input.len() as u32).to_le_bytes());
    encoded.extend_from_slice(&artifact.input);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(&encoded)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("write/sync {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync artifact parent {}: {error}", parent.display()))
}

pub fn read_artifact(path: &Path) -> Result<FuzzArtifact, String> {
    let file = fs::File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("stat {}: {error}", path.display()))?;
    let limit = ARTIFACT_HEADER_SIZE + MAX_ARTIFACT_BYTES;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err("fuzz artifact exceeds the bounded regular-file format".into());
    }
    // Bound the read itself, not only an earlier length observation.
    let mut encoded = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut encoded)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if encoded.len() > limit {
        return Err("fuzz artifact exceeds the bounded format".into());
    }
    if encoded.len() < ARTIFACT_HEADER_SIZE || encoded[0..4] != ARTIFACT_MAGIC {
        return Err("fuzz artifact header is invalid".into());
    }
    let version = u16::from_le_bytes([encoded[4], encoded[5]]);
    let seed_version = u16::from_le_bytes([encoded[6], encoded[7]]);
    if version != ARTIFACT_SCHEMA_VERSION || seed_version != SEED_SCHEMA_VERSION {
        return Err("fuzz artifact version is unsupported".into());
    }
    if encoded[9..12] != [0, 0, 0] {
        return Err("fuzz artifact reserved bytes are nonzero".into());
    }
    let target = CodecTarget::from_id(encoded[8])
        .ok_or_else(|| "fuzz artifact target is unknown".to_owned())?;
    let case = u64::from_le_bytes(
        encoded[12..20]
            .try_into()
            .map_err(|_| "fuzz artifact case is truncated")?,
    );
    let length = u32::from_le_bytes(
        encoded[20..24]
            .try_into()
            .map_err(|_| "fuzz artifact length is truncated")?,
    ) as usize;
    if length > MAX_ARTIFACT_BYTES || encoded.len() != ARTIFACT_HEADER_SIZE + length {
        return Err("fuzz artifact length is inconsistent".into());
    }
    Ok(FuzzArtifact {
        target,
        case,
        input: encoded[ARTIFACT_HEADER_SIZE..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_seeds_decode_and_roundtrip() {
        for target in CodecTarget::ALL {
            let input = canonical_seed(target).unwrap();
            assert!(accepts(target, &input), "{target}: canonical seed rejected");
            exercise(target, &input).unwrap();
        }
    }

    #[test]
    fn deterministic_mutations_are_panic_free() {
        for target in CodecTarget::ALL {
            for case in 0..1024 {
                run_case(target, case)
                    .unwrap_or_else(|error| panic!("{target} case {case}: {error}"));
            }
        }
    }

    #[test]
    fn seed_and_case_identity_fingerprints_are_pinned() {
        let actual: Vec<_> = CodecTarget::ALL
            .into_iter()
            .map(|target| {
                (
                    afsplus_format::crc32c::crc32c(&canonical_seed(target).unwrap()),
                    afsplus_format::crc32c::crc32c(&mutated_input(target, 47).unwrap()),
                )
            })
            .collect();
        // Changing either column changes stable case identities. Bump
        // SEED_SCHEMA_VERSION and regenerate retained artifacts deliberately
        // before updating these fingerprints. New target IDs append rows;
        // the original five targets retain their exact version-1 bytes.
        assert_eq!(
            actual,
            vec![
                (2_222_231_502, 4_262_541_247),
                (871_648_849, 3_020_652_461),
                (2_376_739_319, 847_916_959),
                (3_397_820_429, 566_230_885),
                (1_397_445_837, 1_646_714_288),
                (2_056_605_176, 3_969_668_773),
                (3_251_275_802, 4_105_609_348),
                (3_025_747_490, 4_283_471_495),
                (2_787_334_268, 1_549_429_375),
                (1_278_262_154, 32_389_227),
                (533_851_047, 67_109_637),
                (3_346_469_996, 389_921_249),
                (3_977_748_295, 1_732_764_413),
                (1_589_963_715, 1_381_077_657),
                (1_491_793_277, 3_037_729_287),
                (1_979_058_185, 607_977_437),
                (4_116_635_091, 3_389_458_638),
                (1_571_146_886, 2_571_305_043),
                (2_107_079_709, 3_576_662_424),
                (1_972_104_977, 266_223_001),
                (1_111_250_663, 445_173_983),
            ]
        );
    }

    #[test]
    fn artifact_roundtrip_is_exact_and_bounded() {
        for target in CodecTarget::ALL {
            let artifact = run_case(target, 47).unwrap();
            let path = std::env::temp_dir().join(format!(
                "afsplus-rust-fuzz-artifact-{}-{}.afrf",
                std::process::id(),
                target as u8
            ));
            assert!(!path.exists());
            write_artifact(&path, &artifact).unwrap();
            let saved = fs::read(&path).unwrap();
            let decoded = read_artifact(&path).unwrap();
            assert_eq!(decoded, artifact);
            exercise(decoded.target, &decoded.input).unwrap();
            let other = run_case(target, 48).unwrap();
            assert!(
                write_artifact(&path, &other).is_err(),
                "retained artifact overwritten"
            );
            assert_eq!(fs::read(&path).unwrap(), saved);
            fs::remove_file(&path).unwrap();
        }
    }

    #[test]
    fn allocation_seeds_exercise_geometry_and_partial_bitmap_pages() {
        let geometry = allocation_geometry();
        geometry.validate().unwrap();
        let descriptor = region_seed_descriptor();
        descriptor.validate(&geometry, 1, 7).unwrap();
        assert_eq!(descriptor.pages.len(), 3);
        assert_eq!(
            descriptor.pages.iter().map(|p| p.slot).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        let page = bitmap_seed_page();
        assert_eq!(page.page_index, 2);
        assert_ne!(page.valid_blocks % 8, 0);
        assert_eq!(page.free_blocks(), page.valid_blocks - 2);
        assert_eq!(descriptor.pages[2].free_blocks, page.free_blocks());
    }

    #[test]
    fn resealed_allocation_length_binding_and_padding_errors_are_rejected() {
        for (target, offsets) in [
            (
                CodecTarget::BitmapPage,
                vec![(4, u32::MAX), (8, 0), (12, 0), (12, u32::MAX)],
            ),
            (
                CodecTarget::RegionDescriptor,
                vec![
                    (4, 0),
                    (8, u32::MAX),
                    (12, u32::MAX),
                    (16, 3),
                    (24, 0),
                    (24, 9),
                ],
            ),
        ] {
            let seed = canonical_seed(target).unwrap();
            let header = BlockHeader::verify(&seed, target.block_type()).unwrap();
            for (offset, value) in offsets {
                let mut input = seed.clone();
                input[HEADER_SIZE + offset..HEADER_SIZE + offset + 4]
                    .copy_from_slice(&value.to_le_bytes());
                header.seal(&mut input);
                BlockHeader::verify(&input, target.block_type()).unwrap();
                assert!(
                    !accepts(target, &input),
                    "{target} field {offset} value {value}"
                );
                exercise(target, &input).unwrap();
            }
        }
        let page = bitmap_seed_page();
        let mut input = canonical_seed(CodecTarget::BitmapPage).unwrap();
        let header = BlockHeader::verify(&input, block_type::BITMAP).unwrap();
        input[HEADER_SIZE + 16 + page.bits.len() - 1] &= !(1 << (page.valid_blocks % 8));
        header.seal(&mut input);
        assert!(!accepts(CodecTarget::BitmapPage, &input));
        exercise(CodecTarget::BitmapPage, &input).unwrap();
    }

    #[test]
    fn malformed_artifacts_fail_without_large_reads() {
        let path = std::env::temp_dir().join(format!(
            "afsplus-rust-fuzz-invalid-{}.afrf",
            std::process::id()
        ));
        assert!(!path.exists());
        fs::write(&path, b"not an artifact").unwrap();
        let error = read_artifact(&path).unwrap_err();
        fs::remove_file(&path).unwrap();
        assert!(error.contains("header is invalid"));
    }
}
