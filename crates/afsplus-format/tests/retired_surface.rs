//! A retired version or block kind is refused, not ignored (ADR-115), and its
//! number is never reused. Each probe is a well-formed block of the current
//! format edited into the retired shape and resealed, so only the retired
//! field decides the verdict.
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::ident::{FeatureFlags, Identification, NameKeyAlgorithm, IDENT_VERSION};
use afsplus_format::intent_log::{LogOp, LogRecord};
use afsplus_format::{le, FormatError, Timespec, DEFAULT_BLOCK_SIZE as BS};

fn ident() -> Identification {
    Identification {
        uuid: [0x5a; 16],
        block_shift: 12,
        checksum_algorithm: afsplus_format::crc32c::CHECKSUM_CRC32C,
        region_size: 512,
        log_slots: 4,
        features: FeatureFlags {
            compat: 1,
            ro_compat: 3,
            incompat: 3,
        },
        name_key_algorithm: NameKeyAlgorithm::UnicodeNfc,
        unicode_version: [16, 0, 0],
        total_blocks: 2048,
        checkpoint_slots: [1, 2],
        metadata_start: 9,
        label: "Retired".into(),
    }
}

fn log() -> LogRecord {
    LogRecord {
        uuid: [0x5a; 16],
        base_generation: 9,
        sequence: 2,
        ops: vec![LogOp::Delete {
            parent_id: 1,
            name: b"old".to_vec(),
            timestamp: Timespec {
                seconds: 5,
                nanoseconds: 0,
            },
        }],
    }
}

fn reseal(block: &mut [u8], payload_len: Option<u32>) {
    BlockHeader {
        block_type: u32::from_le_bytes(block[0..4].try_into().unwrap()),
        flags: 0,
        owner: u64::from_le_bytes(block[8..16].try_into().unwrap()),
        generation: u64::from_le_bytes(block[16..24].try_into().unwrap()),
        payload_len: payload_len
            .unwrap_or_else(|| u32::from_le_bytes(block[24..28].try_into().unwrap())),
    }
    .seal(block);
}

#[test]
fn the_two_retired_identification_versions_are_refused() {
    let clean = ident().encode(BS).unwrap();
    assert_eq!(Identification::decode(&clean).unwrap(), ident());
    assert_eq!(IDENT_VERSION, 3);

    // Version 1 with its own payload length of 137, version 2 with 161: the
    // shapes a reader of those prototypes would have accepted.
    for (version, payload) in [(1u32, 137u32), (2, 161)] {
        let mut block = clean.clone();
        le::put_u32(&mut block[HEADER_SIZE + 12..HEADER_SIZE + 16], version);
        block[HEADER_SIZE + payload as usize..].fill(0);
        reseal(&mut block, Some(payload));
        assert_eq!(
            Identification::decode(&block),
            Err(FormatError::Invalid("unsupported identification version")),
            "version {version}"
        );
        // And with the current length, so it is the version that decides.
        let mut same_length = clean.clone();
        le::put_u32(
            &mut same_length[HEADER_SIZE + 12..HEADER_SIZE + 16],
            version,
        );
        reseal(&mut same_length, None);
        assert_eq!(
            Identification::decode(&same_length),
            Err(FormatError::Invalid("unsupported identification version")),
            "version {version} at the current length"
        );
    }
    // Version 0 and the first free number are refused the same way, so a
    // retired number is never reused.
    for version in [0u32, 4] {
        let mut block = clean.clone();
        le::put_u32(&mut block[HEADER_SIZE + 12..HEADER_SIZE + 16], version);
        reseal(&mut block, None);
        assert!(Identification::decode(&block).is_err(), "version {version}");
    }
}

#[test]
fn the_retired_intent_log_record_version_is_refused() {
    let clean = log().encode(BS).unwrap();
    assert_eq!(LogRecord::decode(&clean).unwrap(), log());
    // The writer states version 2 for a namespace record; both 2 and 3 are
    // current, 0 is the retired prototype without an operation timestamp.
    assert_eq!(le::get_u16(&clean[HEADER_SIZE + 30..HEADER_SIZE + 32]), 2);
    for version in [0u16, 1, 4] {
        let mut block = clean.clone();
        le::put_u16(&mut block[HEADER_SIZE + 30..HEADER_SIZE + 32], version);
        reseal(&mut block, None);
        assert_eq!(
            LogRecord::decode(&block),
            Err(FormatError::Invalid(
                "unsupported intent-log record version"
            )),
            "version {version}"
        );
    }
}

#[test]
fn the_three_retired_block_magics_are_unknown_to_every_decoder() {
    // "AFSD", "AFSM" and "AFSR" named the one-block directory, the flat
    // object map and the retired list. No decoder answers for them now, and
    // no current kind may claim one of those magics.
    let retired = [*b"AFSD", *b"AFSM", *b"AFSR"];
    let current = [
        block_type::IDENTIFICATION,
        block_type::CHECKPOINT,
        block_type::OBJECT,
        block_type::BITMAP,
        block_type::REGION_DESCRIPTOR,
        block_type::TREE_NODE,
        block_type::RECLAIM_ROOT,
        block_type::RECLAIM_SEGMENT,
        block_type::RECLAIM_TABLE,
        block_type::SECURITY_DESCRIPTOR,
        block_type::ATTRIBUTE_SET,
        block_type::INTENT_LOG,
    ];
    for magic in retired {
        let value = u32::from_le_bytes(magic);
        assert!(
            !current.contains(&value),
            "a current kind reuses the retired magic {}",
            String::from_utf8_lossy(&magic)
        );
        // A block carrying a retired magic and a valid checksum is not a
        // block of any kind this format defines.
        let mut block = vec![0u8; BS];
        block[0..4].copy_from_slice(&magic);
        BlockHeader {
            block_type: value,
            flags: 0,
            owner: 1,
            generation: 7,
            payload_len: 8,
        }
        .seal(&mut block);
        assert!(BlockHeader::verify(&block, value).is_ok());
        for kind in current {
            assert!(
                BlockHeader::verify(&block, kind).is_err(),
                "{} verified as a current kind",
                String::from_utf8_lossy(&magic)
            );
        }
    }
}
