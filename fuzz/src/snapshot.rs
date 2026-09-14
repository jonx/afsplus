//! Headerless snapshot leaf values: independent wire-field admission oracles.
use super::{splitmix64, CodecTarget, SEED_SCHEMA_VERSION};
use afsplus_format::snapshot::{
    decode_key, LedgerState, LifetimeRecord, RegistryState, SnapshotRecord,
};

const MAX_GENERATION: u64 = 17;
const TOTAL: u64 = 8192;
const START: u64 = 101;

pub(super) fn handles(target: CodecTarget) -> bool {
    matches!(
        target,
        CodecTarget::SnapshotRegistry
            | CodecTarget::SnapshotRecord
            | CodecTarget::SnapshotLifetime
            | CodecTarget::SnapshotLedger
            | CodecTarget::SnapshotKey
    )
}

pub(super) fn seed(target: CodecTarget) -> Result<Vec<u8>, String> {
    let value = match target {
        CodecTarget::SnapshotRegistry => RegistryState { next_id: 42 }.encode(),
        CodecTarget::SnapshotRecord => SnapshotRecord {
            generation: 11,
            committed_tx_id: 9,
            object_map_root: 123,
        }
        .encode(MAX_GENERATION, TOTAL),
        CodecTarget::SnapshotLifetime => LifetimeRecord {
            blocks: 7,
            birth: 3,
            retirement: 13,
        }
        .encode(START, MAX_GENERATION, TOTAL),
        CodecTarget::SnapshotLedger => LedgerState {
            scan_position: 101,
            retained_blocks: 23,
        }
        .encode(TOTAL),
        CodecTarget::SnapshotKey => return Ok(42u64.to_be_bytes().to_vec()),
        _ => unreachable!(),
    };
    value.map(|v| v.to_vec()).map_err(|e| e.to_string())
}

fn word(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().unwrap())
}

// Deliberately read the wire directly rather than calling codec validation.
fn expected(target: CodecTarget, input: &[u8]) -> bool {
    if target == CodecTarget::SnapshotKey {
        return input.len() == 8;
    }
    if input.len() != 32 {
        return false;
    }
    let used = match target {
        CodecTarget::SnapshotRegistry => 8,
        CodecTarget::SnapshotLedger => 16,
        _ => 24,
    };
    if input[used..].iter().any(|b| *b != 0) {
        return false;
    }
    let a = word(input, 0);
    let b = word(input, 8);
    let c = word(input, 16);
    match target {
        CodecTarget::SnapshotRegistry => a != 0,
        CodecTarget::SnapshotRecord => {
            (1..=MAX_GENERATION).contains(&a) && b != 0 && b <= a && c != 0 && c < TOTAL
        }
        CodecTarget::SnapshotLifetime => {
            a != 0
                && a <= TOTAL - START
                && (1..=MAX_GENERATION).contains(&b)
                && (c == 0 || (c > b && c <= MAX_GENERATION))
        }
        CodecTarget::SnapshotLedger => a < TOTAL && b <= TOTAL,
        _ => unreachable!(),
    }
}

pub(super) fn accepts(target: CodecTarget, input: &[u8]) -> bool {
    match target {
        CodecTarget::SnapshotRegistry => RegistryState::decode(input).is_ok(),
        CodecTarget::SnapshotRecord => SnapshotRecord::decode(input, MAX_GENERATION, TOTAL).is_ok(),
        CodecTarget::SnapshotLifetime => {
            LifetimeRecord::decode(input, START, MAX_GENERATION, TOTAL).is_ok()
        }
        CodecTarget::SnapshotLedger => LedgerState::decode(input, TOTAL).is_ok(),
        CodecTarget::SnapshotKey => decode_key(input).is_ok(),
        _ => unreachable!(),
    }
}

pub(super) fn exercise(target: CodecTarget, input: &[u8]) -> Result<(), String> {
    if accepts(target, input) != expected(target, input) {
        return Err(format!("{target}: independent admission mismatch"));
    }
    if !expected(target, input) {
        return Ok(());
    }
    let encoded = match target {
        CodecTarget::SnapshotRegistry => {
            let state = RegistryState::decode(input).unwrap();
            if state.next_id != word(input, 0) {
                return Err("registry endian mismatch".into());
            }
            let allocation = state.allocate_id();
            if state.next_id == u64::MAX {
                if allocation.is_ok() {
                    return Err("snapshot ID wrapped".into());
                }
            } else if allocation.map_err(|e| e.to_string())?
                != (
                    state.next_id,
                    RegistryState {
                        next_id: state.next_id + 1,
                    },
                )
            {
                return Err("snapshot ID allocation mismatch".into());
            }
            state.encode()
        }
        CodecTarget::SnapshotRecord => {
            let record = SnapshotRecord::decode(input, MAX_GENERATION, TOTAL).unwrap();
            if (
                record.generation,
                record.committed_tx_id,
                record.object_map_root,
            ) != (word(input, 0), word(input, 8), word(input, 16))
            {
                return Err("snapshot field mismatch".into());
            }
            record.encode(MAX_GENERATION, TOTAL)
        }
        CodecTarget::SnapshotLifetime => {
            let record = LifetimeRecord::decode(input, START, MAX_GENERATION, TOTAL).unwrap();
            if (record.blocks, record.birth, record.retirement)
                != (word(input, 0), word(input, 8), word(input, 16))
            {
                return Err("lifetime field mismatch".into());
            }
            for generation in [0, word(input, 8), word(input, 16), MAX_GENERATION, u64::MAX] {
                let wanted = generation >= word(input, 8)
                    && (word(input, 16) == 0 || generation < word(input, 16));
                if record.contains(generation) != wanted {
                    return Err("lifetime interval mismatch".into());
                }
            }
            record.encode(START, MAX_GENERATION, TOTAL)
        }
        CodecTarget::SnapshotLedger => {
            let state = LedgerState::decode(input, TOTAL).unwrap();
            if (state.scan_position, state.retained_blocks) != (word(input, 0), word(input, 8)) {
                return Err("ledger field mismatch".into());
            }
            state.encode(TOTAL)
        }
        CodecTarget::SnapshotKey => {
            let decoded = decode_key(input).unwrap();
            let independent = input.iter().fold(0u64, |a, b| (a << 8) | u64::from(*b));
            if decoded != independent {
                return Err("snapshot key endian mismatch".into());
            }
            return Ok(());
        }
        _ => unreachable!(),
    }
    .map_err(|e| e.to_string())?;
    if encoded.as_slice() != input {
        return Err("snapshot exact canonical roundtrip mismatch".into());
    }
    Ok(())
}

pub(super) fn mutate(target: CodecTarget, case: u64, mut input: Vec<u8>) -> Vec<u8> {
    let random = splitmix64(case ^ ((target as u64) << 56) ^ u64::from(SEED_SCHEMA_VERSION));
    match case % 6 {
        0 | 1 => {
            let at = random as usize % input.len();
            input[at] ^= 1 << ((random >> 32) & 7);
        }
        2 => input.truncate(random as usize % (input.len() + 1)),
        3 => {
            let at = random as usize % (input.len() / 8) * 8;
            let values = [0, 1, 17, 18, 8191, 8192, u64::MAX];
            input[at..at + 8]
                .copy_from_slice(&values[(random >> 32) as usize % values.len()].to_le_bytes());
        }
        4 => input.fill((random >> 24) as u8),
        5 => input.extend(
            (0..((random >> 40) as usize % 64 + 1)).map(|i| splitmix64(random ^ i as u64) as u8),
        ),
        _ => unreachable!(),
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_length_reserved_byte_and_numeric_boundary_has_an_oracle() {
        for target in super::super::CodecTarget::ALL
            .into_iter()
            .filter(|t| handles(*t))
        {
            let seed = seed(target).unwrap();
            for len in 0..=96 {
                let mut bytes = seed.clone();
                bytes.resize(len, 0);
                exercise(target, &bytes).unwrap();
            }
            for offset in 0..seed.len() {
                let mut bytes = seed.clone();
                bytes[offset] ^= 0xff;
                exercise(target, &bytes).unwrap();
            }
            for offset in (0..seed.len()).step_by(8) {
                for value in [
                    0u64,
                    1,
                    2,
                    3,
                    11,
                    13,
                    17,
                    18,
                    TOTAL - START,
                    TOTAL - START + 1,
                    TOTAL - 1,
                    TOTAL,
                    u64::MAX,
                ] {
                    let mut bytes = seed.clone();
                    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
                    exercise(target, &bytes).unwrap();
                }
            }
        }
    }
}
