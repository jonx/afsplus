//! Bounded, unique-key UTF-8 PAX extended-header record blocks.
//!
//! The profile layer decides supported keys, values and preservation policy.
//! This codec rejects duplicate keys instead of applying global/local override
//! semantics, and accepts only canonical decimal byte lengths.
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub bytes: usize,
    pub records: usize,
    pub key_bytes: usize,
    pub value_bytes: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Limit,
    InvalidLength,
    InvalidRecord,
    InvalidUtf8,
    DuplicateKey,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Limit => "PAX resource limit exceeded",
            Self::InvalidLength => "invalid PAX byte length",
            Self::InvalidRecord => "invalid PAX record",
            Self::InvalidUtf8 => "invalid PAX UTF-8",
            Self::DuplicateKey => "duplicate PAX keyword",
        })
    }
}
impl std::error::Error for Error {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    pub key: &'a str,
    pub value: &'a str,
}
impl Limits {
    fn validate(self) -> Result<(), Error> {
        if self.bytes == 0 || self.records == 0 || self.key_bytes == 0 {
            Err(Error::Limit)
        } else {
            Ok(())
        }
    }
    fn record(self, record: Record<'_>) -> Result<(), Error> {
        if record.key.len() > self.key_bytes || record.value.len() > self.value_bytes {
            return Err(Error::Limit);
        }
        if record.key.is_empty()
            || record.key.bytes().any(|c| matches!(c, b'=' | b'\n' | 0))
            || record.value.contains('\0')
        {
            return Err(Error::InvalidRecord);
        }
        Ok(())
    }
}

/// Borrows keys/values from the admitted input. Output bookkeeping scales with
/// the explicit record count; no allocation follows an unchecked wire length.
pub fn decode(input: &[u8], limits: Limits) -> Result<Vec<Record<'_>>, Error> {
    limits.validate()?;
    if input.len() > limits.bytes {
        return Err(Error::Limit);
    }
    let mut rest = input;
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    while !rest.is_empty() {
        if records.len() == limits.records {
            return Err(Error::Limit);
        }
        let mut length = 0usize;
        let mut digits = 0usize;
        for &c in rest {
            if c == b' ' {
                break;
            }
            if !c.is_ascii_digit() || (digits == 0 && c == b'0') {
                return Err(Error::InvalidLength);
            }
            length = length
                .checked_mul(10)
                .and_then(|n| n.checked_add((c - b'0') as usize))
                .ok_or(Error::InvalidLength)?;
            digits += 1;
        }
        if digits == 0
            || rest.get(digits) != Some(&b' ')
            || length > rest.len()
            || length < digits + 4
        {
            return Err(Error::InvalidLength);
        }
        let (wire, tail) = rest.split_at(length);
        if wire.last() != Some(&b'\n') {
            return Err(Error::InvalidRecord);
        }
        let body = &wire[digits + 1..length - 1];
        let equal = body
            .iter()
            .position(|&c| c == b'=')
            .ok_or(Error::InvalidRecord)?;
        let record = Record {
            key: std::str::from_utf8(&body[..equal]).map_err(|_| Error::InvalidUtf8)?,
            value: std::str::from_utf8(&body[equal + 1..]).map_err(|_| Error::InvalidUtf8)?,
        };
        limits.record(record)?;
        if !seen.insert(record.key) {
            return Err(Error::DuplicateKey);
        }
        records.push(record);
        rest = tail;
    }
    Ok(records)
}

fn wire_length(record: Record<'_>) -> Result<usize, Error> {
    let base = record
        .key
        .len()
        .checked_add(record.value.len())
        .and_then(|n| n.checked_add(3))
        .ok_or(Error::Limit)?;
    let mut length = base.checked_add(1).ok_or(Error::Limit)?;
    loop {
        let next = base
            .checked_add(length.ilog10() as usize + 1)
            .ok_or(Error::Limit)?;
        if next == length {
            return Ok(length);
        }
        length = next;
    }
}

/// Validates fields and budgets without allocating a serialized record block.
/// Duplicate-key bookkeeping is bounded by the admitted record count.
pub fn encoded_len(records: &[Record<'_>], limits: Limits) -> Result<usize, Error> {
    limits.validate()?;
    if records.len() > limits.records {
        return Err(Error::Limit);
    }
    let mut total = 0usize;
    let mut seen = BTreeSet::new();
    for &record in records {
        limits.record(record)?;
        if !seen.insert(record.key) {
            return Err(Error::DuplicateKey);
        }
        total = total
            .checked_add(wire_length(record)?)
            .ok_or(Error::Limit)?;
        if total > limits.bytes {
            return Err(Error::Limit);
        }
    }
    Ok(total)
}

/// Preflights every record and total byte count before allocating output.
pub fn encode(records: &[Record<'_>], limits: Limits) -> Result<Vec<u8>, Error> {
    let total = encoded_len(records, limits)?;
    let mut output = Vec::with_capacity(total);
    for &record in records {
        output.extend_from_slice(wire_length(record)?.to_string().as_bytes());
        output.push(b' ');
        output.extend_from_slice(record.key.as_bytes());
        output.push(b'=');
        output.extend_from_slice(record.value.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> Limits {
        Limits {
            bytes: 65536,
            records: 64,
            key_bytes: 128,
            value_bytes: 16384,
        }
    }
    #[test]
    fn python_tarfile_golden_and_utf8_byte_lengths() {
        // Python 3.9.6 tarfile._create_pax_generic_header independent fixture.
        let golden = b"27 path=caf\xc3\xa9/notes\npart=1\n22 mtime=-1.000000001\n";
        let records = [
            Record {
                key: "path",
                value: "café/notes\npart=1",
            },
            Record {
                key: "mtime",
                value: "-1.000000001",
            },
        ];
        assert_eq!(decode(golden, limits()).unwrap(), records);
        assert_eq!(encode(&records, limits()).unwrap(), golden);
    }
    #[test]
    fn decimal_boundaries_and_empty_values_round_trip() {
        for size in [
            0, 1, 3, 4, 5, 8, 9, 90, 91, 92, 93, 94, 95, 990, 999, 1000, 9999,
        ] {
            let value = "x".repeat(size);
            let records = [Record {
                key: "k",
                value: &value,
            }];
            let wire = encode(&records, limits()).unwrap();
            assert_eq!(decode(&wire, limits()).unwrap(), records);
            for end in 1..wire.len() {
                assert!(decode(&wire[..end], limits()).is_err());
            }
        }
    }
    #[test]
    fn malformed_lengths_duplicates_and_utf8_are_rejected() {
        for wire in [
            b"0 k=v\n".as_slice(),
            b"07 k=v\n",
            b"-1 k=v\n",
            b"7 k=v",
            b"7 k=vX",
            b"999999999999999999999999999999 k=v\n",
            b"7 k=v\n7 k=w\n",
            b"6 =xx\n",
            b"7 k=\xff\n",
            b"7 k=\0\n",
            b"7 k=v\ntrash",
        ] {
            assert!(decode(wire, limits()).is_err(), "{wire:?}");
        }
        assert_eq!(
            decode(b"6 k=v\n6 k=w\n", limits()),
            Err(Error::DuplicateKey)
        );
        assert_eq!(decode(b"6 k=\xff\n", limits()), Err(Error::InvalidUtf8));
        assert_eq!(decode(b"6 k=\0\n", limits()), Err(Error::InvalidRecord));
        let duplicate = [
            Record {
                key: "path",
                value: "a",
            },
            Record {
                key: "path",
                value: "b",
            },
        ];
        assert_eq!(encode(&duplicate, limits()), Err(Error::DuplicateKey));
    }
    #[test]
    fn every_budget_is_enforced_in_both_directions() {
        let records = [
            Record {
                key: "path",
                value: "abc",
            },
            Record {
                key: "mtime",
                value: "123",
            },
        ];
        let wire = encode(&records, limits()).unwrap();
        for bound in [
            Limits {
                bytes: wire.len() - 1,
                ..limits()
            },
            Limits {
                records: 1,
                ..limits()
            },
            Limits {
                key_bytes: 3,
                ..limits()
            },
            Limits {
                value_bytes: 2,
                ..limits()
            },
            Limits {
                bytes: 0,
                ..limits()
            },
            Limits {
                records: 0,
                ..limits()
            },
        ] {
            assert_eq!(decode(&wire, bound), Err(Error::Limit));
            assert_eq!(encode(&records, bound), Err(Error::Limit));
        }
        assert_eq!(
            encode(
                &records,
                Limits {
                    bytes: wire.len(),
                    ..limits()
                }
            )
            .unwrap(),
            wire
        );
    }
    #[test]
    fn deterministic_arbitrary_input_never_panics() {
        let mut seed = 0x913a_478cu32;
        for size in 0..1024 {
            let mut wire = vec![0; size];
            for c in &mut wire {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *c = seed as u8;
            }
            let _ = decode(&wire, limits());
        }
    }
}
