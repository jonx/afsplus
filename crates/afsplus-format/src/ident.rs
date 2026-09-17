//! Identification block (`docs/03-on-disk-format.md` §5).
//!
//! Fixed, checksummed record at a deterministic location (prototype: LBA 0)
//! that lets external tools recognize AFS+ without parsing arbitrary
//! metadata. It is written once by the formatter and is immutable afterwards,
//! so it is never exposed to torn rewrites; mutable committed state
//! (clean/dirty, generations) lives in the checkpoints.
//!
//! Payload layout after the 32-byte common header:
//!
//! ```text
//! offset size field
//! 0      8    filesystem magic ("AFSPLUS1")
//! 8      4    format epoch
//! 12     4    identification record version
//! 16     16   filesystem UUID
//! 32     1    logical block shift
//! 33     1    checksum algorithm identifier
//! 34     2    intent-log slot count (0 = no log area; ADR-037)
//! 36     4    allocation region size in blocks
//! 40     8    total logical blocks
//! 48     8    checkpoint slot A LBA
//! 56     8    checkpoint slot B LBA
//! 64     8    first general-allocation LBA (after region 0's reserved head)
//! 72     1    label length in bytes
//! 73     64   label (UTF-8, zero padded)
//! 137    8    COMPAT feature bits
//! 145    8    RO_COMPAT feature bits
//! 153    8    INCOMPAT feature bits
//! 161    1    directory comparison-key algorithm
//! 162    3    Unicode table version (major, minor, patch)
//! ```

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::crc32c::CHECKSUM_CRC32C;
use crate::geometry::Geometry;
use crate::header::{block_type, BlockHeader, HEADER_SIZE};
use crate::{le, FormatError, DEFAULT_BLOCK_SHIFT, FORMAT_EPOCH, FS_MAGIC};

pub const IDENT_VERSION: u32 = 3;
pub const IDENT_VERSION_FEATURES: u32 = 2;
pub const IDENT_VERSION_LEGACY: u32 = 1;
pub const LABEL_MAX_BYTES: usize = 64;

/// The label rule shared by the formatter, the relabel operation and both
/// label carriers (this block at format time, the checkpoint afterwards): at
/// most 64 bytes of UTF-8 and no NUL byte. Naming syntax of a host (a DOS
/// colon, a path separator) is host policy and absent from the format.
pub fn validate_label(label: &str) -> Result<(), FormatError> {
    if label.len() > LABEL_MAX_BYTES {
        return Err(FormatError::Overflow("volume label"));
    }
    if label.as_bytes().contains(&0) {
        return Err(FormatError::Invalid("volume label contains NUL"));
    }
    Ok(())
}
/// The intent-log area and its replay semantics must be understood by every
/// implementation that opens the volume (ADR-037).
pub const INCOMPAT_INTENT_LOG: u64 = 1 << 0;
/// Intent-log record version 3 may reference replacement data for an
/// existing file. A separate INCOMPAT identity keeps older version-2
/// implementations from treating an unknown valid record as an empty/torn
/// tail and silently losing a completed fsync (ADR-064).
pub const INCOMPAT_INTENT_LOG_DATA_UPDATES: u64 = 1 << 1;
/// Persistent snapshot ownership (ADR-072). Codecs alone do not authorize
/// mounting this feature; the core must qualify the complete lifetime protocol.
pub const INCOMPAT_PERSISTENT_SNAPSHOTS: u64 = 1 << 2;
/// Object records may carry `OBJECT_FLAG_SECURITY_REF` and reference
/// `"AFSX"` descriptor segments. An implementation without the container
/// cannot decode such a record and would drop the reference on rewrite, so
/// the identity is INCOMPAT.
pub const INCOMPAT_SECURITY_DESCRIPTORS: u64 = 1 << 3;

/// Shared data extents (ADR-061): reference counts must be honoured on every
/// write and free, so an implementation without support mounts read-only.
/// Set by mkfs according to the compatibility profile; identification is
/// immutable, so the first clone cannot set it.
pub const RO_COMPAT_SHARED_EXTENTS: u64 = 1 << 0;
/// A reserved object-map entry at object ID 2 may contain the internal
/// orphan directory (ADR-066). Unaware writers must mount read-only so they
/// cannot lose or expose its lifecycle state.
pub const RO_COMPAT_ORPHAN_DIRECTORY: u64 = 1 << 1;

/// Per-file persistent data-update policy (ADR-065): object records on this
/// volume may carry `OBJECT_FLAG_DATA_IN_PLACE`. An implementation that
/// ignores the policy always uses full data COW, which is strictly stronger,
/// so the bit is COMPAT; the validated object-flags namespace keeps unaware
/// writers from silently dropping the per-file choice. Set by mkfs per
/// profile; identification is immutable.
pub const COMPAT_DATA_POLICY: u64 = 1 << 0;

const LEGACY_PAYLOAD_LEN: usize = 73 + LABEL_MAX_BYTES;
const FEATURE_PAYLOAD_LEN: usize = LEGACY_PAYLOAD_LEN + 3 * 8;
const PAYLOAD_LEN: usize = FEATURE_PAYLOAD_LEN + 4;

/// Unicode data frozen by the first executable comparison-key algorithm.
pub const UNICODE_VERSION_16_0_0: [u8; 3] = [16, 0, 0];

/// Byte-level directory-key derivation recorded in the immutable identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NameKeyAlgorithm {
    /// Prototype v1/v2 byte identity. Compatibility mode; never emitted by new mkfs.
    LegacyIdentity = 0,
    /// Unicode 16 NFC; comparisons remain case-sensitive.
    UnicodeNfc = 1,
    /// Unicode 16 canonical normalization plus full default case folding.
    UnicodeNfcCasefold = 2,
}

impl NameKeyAlgorithm {
    fn decode(value: u8) -> Result<Self, FormatError> {
        match value {
            0 => Ok(Self::LegacyIdentity),
            1 => Ok(Self::UnicodeNfc),
            2 => Ok(Self::UnicodeNfcCasefold),
            _ => Err(FormatError::Invalid(
                "unsupported directory comparison-key algorithm",
            )),
        }
    }
}

/// Compact feature summary carried by the immutable identification block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeatureFlags {
    pub compat: u64,
    pub ro_compat: u64,
    pub incompat: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identification {
    pub uuid: [u8; 16],
    pub block_shift: u8,
    pub checksum_algorithm: u8,
    pub region_size: u32,
    /// Reserved intent-log slots after the allocation-root pool (ADR-037).
    pub log_slots: u16,
    pub features: FeatureFlags,
    pub name_key_algorithm: NameKeyAlgorithm,
    pub unicode_version: [u8; 3],
    pub total_blocks: u64,
    pub checkpoint_slots: [u64; 2],
    pub metadata_start: u64,
    pub label: String,
}

impl Identification {
    pub fn block_size(&self) -> usize {
        1usize << self.block_shift
    }

    pub fn geometry(&self) -> Geometry {
        Geometry {
            block_size: self.block_size(),
            total_blocks: self.total_blocks,
            region_size: self.region_size,
        }
    }

    pub fn encode(&self, block_size: usize) -> Result<Vec<u8>, FormatError> {
        if self.block_shift != DEFAULT_BLOCK_SHIFT {
            return Err(FormatError::Invalid("prototype supports only 4 KiB blocks"));
        }
        if block_size != self.block_size() {
            return Err(FormatError::WrongBufferSize {
                expected: self.block_size(),
                actual: block_size,
            });
        }
        validate_label(&self.label)?;
        let label = self.label.as_bytes();
        self.validate_geometry()?;

        let mut block = vec![0u8; block_size];
        let p = &mut block[HEADER_SIZE..];
        le::put_u64(&mut p[0..8], FS_MAGIC);
        le::put_u32(&mut p[8..12], FORMAT_EPOCH);
        le::put_u32(&mut p[12..16], IDENT_VERSION);
        p[16..32].copy_from_slice(&self.uuid);
        p[32] = self.block_shift;
        p[33] = self.checksum_algorithm;
        le::put_u16(&mut p[34..36], self.log_slots);
        le::put_u32(&mut p[36..40], self.region_size);
        le::put_u64(&mut p[40..48], self.total_blocks);
        le::put_u64(&mut p[48..56], self.checkpoint_slots[0]);
        le::put_u64(&mut p[56..64], self.checkpoint_slots[1]);
        le::put_u64(&mut p[64..72], self.metadata_start);
        p[72] = label.len() as u8;
        p[73..73 + label.len()].copy_from_slice(label);
        le::put_u64(&mut p[137..145], self.features.compat);
        le::put_u64(&mut p[145..153], self.features.ro_compat);
        le::put_u64(&mut p[153..161], self.features.incompat);
        p[161] = self.name_key_algorithm as u8;
        p[162..165].copy_from_slice(&self.unicode_version);

        BlockHeader {
            block_type: block_type::IDENTIFICATION,
            flags: 0,
            owner: 0,
            generation: 0,
            payload_len: PAYLOAD_LEN as u32,
        }
        .seal(&mut block);
        Ok(block)
    }

    pub fn decode(block: &[u8]) -> Result<Identification, FormatError> {
        let header = BlockHeader::verify(block, block_type::IDENTIFICATION)?;
        let p = header.payload(block);
        // The block belongs to the volume, not to an object, and it has no
        // flag namespace (ADR-114).
        if header.flags != 0 || header.owner != 0 {
            return Err(FormatError::Invalid(
                "identification header flags or owner are nonzero",
            ));
        }
        if p.len() < LEGACY_PAYLOAD_LEN {
            return Err(FormatError::Invalid("identification payload too short"));
        }
        if le::get_u64(&p[0..8]) != FS_MAGIC {
            return Err(FormatError::Invalid("filesystem magic mismatch"));
        }
        let epoch = le::get_u32(&p[8..12]);
        if epoch != FORMAT_EPOCH {
            return Err(FormatError::Invalid("unsupported format epoch"));
        }
        let version = le::get_u32(&p[12..16]);
        if !matches!(
            version,
            IDENT_VERSION | IDENT_VERSION_FEATURES | IDENT_VERSION_LEGACY
        ) {
            return Err(FormatError::Invalid("unsupported identification version"));
        }
        let minimum_payload = match version {
            IDENT_VERSION => PAYLOAD_LEN,
            IDENT_VERSION_FEATURES => FEATURE_PAYLOAD_LEN,
            _ => LEGACY_PAYLOAD_LEN,
        };
        // Exact for its version: a byte past the last field belongs to no
        // field, and the formatter writes none (ADR-114).
        if p.len() != minimum_payload {
            return Err(FormatError::Invalid(
                "identification payload length is not exact for its version",
            ));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&p[16..32]);
        let block_shift = p[32];
        if block_shift != DEFAULT_BLOCK_SHIFT {
            return Err(FormatError::Invalid("prototype supports only 4 KiB blocks"));
        }
        let checksum_algorithm = p[33];
        if checksum_algorithm != CHECKSUM_CRC32C {
            return Err(FormatError::Invalid("unsupported checksum algorithm"));
        }
        let log_slots = le::get_u16(&p[34..36]);
        let features = if version >= IDENT_VERSION_FEATURES {
            FeatureFlags {
                compat: le::get_u64(&p[137..145]),
                ro_compat: le::get_u64(&p[145..153]),
                incompat: le::get_u64(&p[153..161]),
            }
        } else {
            FeatureFlags {
                incompat: if log_slots > 0 {
                    INCOMPAT_INTENT_LOG
                } else {
                    0
                },
                ..FeatureFlags::default()
            }
        };
        let (name_key_algorithm, unicode_version) = if version == IDENT_VERSION {
            (NameKeyAlgorithm::decode(p[161])?, [p[162], p[163], p[164]])
        } else {
            (NameKeyAlgorithm::LegacyIdentity, [0, 0, 0])
        };
        let region_size = le::get_u32(&p[36..40]);
        let label_len = p[72] as usize;
        if label_len > LABEL_MAX_BYTES {
            return Err(FormatError::Invalid("label length out of range"));
        }
        let label =
            core::str::from_utf8(&p[73..73 + label_len]).map_err(|_| FormatError::InvalidUtf8)?;
        // The reader applies the rule the writer applies, so a decoded
        // identification always encodes again.
        validate_label(label)?;

        let ident = Identification {
            uuid,
            block_shift,
            checksum_algorithm,
            region_size,
            log_slots,
            features,
            name_key_algorithm,
            unicode_version,
            total_blocks: le::get_u64(&p[40..48]),
            checkpoint_slots: [le::get_u64(&p[48..56]), le::get_u64(&p[56..64])],
            metadata_start: le::get_u64(&p[64..72]),
            label: String::from(label),
        };
        ident.validate_geometry()?;
        Ok(ident)
    }

    /// Bounds-first geometry validation shared by encode and decode.
    fn validate_geometry(&self) -> Result<(), FormatError> {
        self.geometry().validate()?;
        // Each compatibility class is its own bit namespace (ADR-061 assigns
        // RO_COMPAT bit 0 while INCOMPAT bit 0 is the intent log): a bit's
        // meaning is the pair (class, position), never the position alone.
        if (self.log_slots > 0) != (self.features.incompat & INCOMPAT_INTENT_LOG != 0) {
            return Err(FormatError::Invalid(
                "intent-log slots and incompatible feature bit disagree",
            ));
        }
        if self.features.incompat & INCOMPAT_INTENT_LOG_DATA_UPDATES != 0
            && self.features.incompat & INCOMPAT_INTENT_LOG == 0
        {
            return Err(FormatError::Invalid(
                "intent-log data updates require the base intent-log feature",
            ));
        }
        match self.name_key_algorithm {
            NameKeyAlgorithm::LegacyIdentity if self.unicode_version == [0, 0, 0] => {}
            NameKeyAlgorithm::UnicodeNfc | NameKeyAlgorithm::UnicodeNfcCasefold
                if self.unicode_version == UNICODE_VERSION_16_0_0 => {}
            NameKeyAlgorithm::LegacyIdentity => {
                return Err(FormatError::Invalid(
                    "legacy identity key must not declare Unicode tables",
                ))
            }
            NameKeyAlgorithm::UnicodeNfc | NameKeyAlgorithm::UnicodeNfcCasefold => {
                return Err(FormatError::Invalid(
                    "unsupported directory Unicode table version",
                ))
            }
        }
        // The prototype pins the reserved layout: ident at 0, checkpoint
        // slots at 1 and 2, general allocation from the end of region 0's
        // reserved head.
        if self.checkpoint_slots != [1, 2] {
            return Err(FormatError::Invalid(
                "prototype requires checkpoint slots at LBA 1 and 2",
            ));
        }
        if self.metadata_start != self.geometry().region0_reserved_blocks() {
            return Err(FormatError::Invalid(
                "metadata start does not match reserved layout",
            ));
        }
        Ok(())
    }
}
