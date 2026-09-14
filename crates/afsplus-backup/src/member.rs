//! Effective ordinary member fields after a single local PAX record block.
//! This admission layer does not validate the complete preservation profile.
use crate::{envelope, pax, tar};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Pax(pax::Error),
    Unsupported,
    Invalid,
    Limit,
}

/// Seconds use floor normalization; nanoseconds are always less than 10^9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: u32,
}

pub(crate) fn unsigned(text: &str) -> Result<u64, Error> {
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
        return Err(Error::Invalid);
    }
    text.bytes().try_fold(0u64, |n, c| {
        if !c.is_ascii_digit() {
            return Err(Error::Invalid);
        }
        n.checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(c - b'0')))
            .ok_or(Error::Invalid)
    })
}

impl Timestamp {
    /// Parses exact nanoseconds without floating-point conversion or rounding.
    pub fn parse(text: &str) -> Result<Self, Error> {
        // i64 magnitude, sign, decimal point and nine fractional digits.
        if text.len() > 30 {
            return Err(Error::Invalid);
        }
        let negative = text.starts_with('-');
        let magnitude = text.strip_prefix('-').unwrap_or(text);
        let (whole, fraction) = magnitude.split_once('.').unwrap_or((magnitude, ""));
        if magnitude.contains('.') && fraction.is_empty() {
            return Err(Error::Invalid);
        }
        let whole = unsigned(whole)?;
        if fraction.len() > 9 || !fraction.bytes().all(|c| c.is_ascii_digit()) {
            return Err(Error::Invalid);
        }
        let mut nanos = 0u32;
        for c in fraction.bytes() {
            nanos = nanos * 10 + u32::from(c - b'0');
        }
        nanos *= 10u32.pow(9 - fraction.len() as u32);
        if negative && whole == 0 && nanos == 0 {
            return Err(Error::Invalid);
        }
        let seconds = if negative {
            -i128::from(whole) - i128::from(nanos != 0)
        } else {
            i128::from(whole)
        };
        Ok(Self {
            seconds: i64::try_from(seconds).map_err(|_| Error::Invalid)?,
            nanos: if negative && nanos != 0 {
                1_000_000_000 - nanos
            } else {
                nanos
            },
        })
    }

    pub fn decimal(self) -> Result<String, Error> {
        if self.nanos >= 1_000_000_000 {
            return Err(Error::Invalid);
        }
        if self.nanos == 0 {
            return Ok(self.seconds.to_string());
        }
        let (sign, whole, fraction) = if self.seconds < 0 {
            (
                "-",
                -(i128::from(self.seconds) + 1),
                1_000_000_000 - self.nanos,
            )
        } else {
            ("", i128::from(self.seconds), self.nanos)
        };
        Ok(format!("{sign}{whole}.{fraction:09}")
            .trim_end_matches('0')
            .to_owned())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Member<'a> {
    pub path: &'a str,
    pub link: &'a str,
    pub kind: tar::Kind,
    pub mode: u32,
    pub uid: u64,
    pub gid: u64,
    pub size: u64,
    pub mtime: Timestamp,
    pub uname: &'a str,
    pub gname: &'a str,
}

/// Borrows admitted strings. Unknown records require a specific profile handler;
/// they are never silently discarded. Pass the resulting file size to framing.
pub fn resolve<'a>(
    header: &'a tar::Header,
    records: &[pax::Record<'a>],
    limits: pax::Limits,
) -> Result<Member<'a>, Error> {
    pax::encoded_len(records, limits).map_err(Error::Pax)?;
    if matches!(header.kind, tar::Kind::PaxLocal | tar::Kind::PaxGlobal) {
        return Err(Error::Unsupported);
    }
    let mut result = Member {
        path: &header.path,
        link: &header.link,
        kind: header.kind,
        mode: header.mode,
        uid: header.uid,
        gid: header.gid,
        size: header.size,
        mtime: Timestamp {
            seconds: 0,
            nanos: 0,
        },
        uname: &header.uname,
        gname: &header.gname,
    };
    let mut time = None;
    for record in records {
        match record.key {
            "path" => result.path = record.value,
            "linkpath" => result.link = record.value,
            "size" => result.size = unsigned(record.value)?,
            "uid" => result.uid = unsigned(record.value)?,
            "gid" => result.gid = unsigned(record.value)?,
            "mtime" => time = Some(Timestamp::parse(record.value)?),
            "uname" => result.uname = record.value,
            "gname" => result.gname = record.value,
            _ => return Err(Error::Unsupported),
        }
    }
    result.mtime = match time {
        Some(time) => time,
        None => Timestamp {
            seconds: i64::try_from(header.mtime).map_err(|_| Error::Invalid)?,
            nanos: 0,
        },
    };
    for text in [result.path, result.link, result.uname, result.gname] {
        if text.len() > limits.value_bytes {
            return Err(Error::Limit);
        }
        if text.contains('\0') {
            return Err(Error::Invalid);
        }
    }
    if !envelope::canonical(result.path, result.kind == tar::Kind::Directory, true)
        || (result.kind != tar::Kind::File && result.size != 0)
    {
        return Err(Error::Invalid);
    }
    match result.kind {
        tar::Kind::HardLink if !envelope::canonical(result.link, false, false) => {
            return Err(Error::Invalid)
        }
        tar::Kind::Symlink if result.link.is_empty() => return Err(Error::Invalid),
        tar::Kind::File | tar::Kind::Directory if !result.link.is_empty() => {
            return Err(Error::Invalid)
        }
        _ => (),
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> pax::Limits {
        pax::Limits {
            bytes: 4096,
            records: 16,
            key_bytes: 64,
            value_bytes: 512,
        }
    }
    fn header() -> tar::Header {
        tar::Header {
            path: "files/safe".into(),
            link: String::new(),
            kind: tar::Kind::File,
            mode: 0o640,
            uid: 1,
            gid: 2,
            size: 3,
            mtime: 4,
            uname: String::new(),
            gname: String::new(),
        }
    }
    #[test]
    fn exact_signed_timestamps_and_extremes() {
        for (text, seconds, nanos) in [
            ("0", 0, 0),
            ("1.25", 1, 250_000_000),
            ("-1.25", -2, 750_000_000),
            ("-0.000000001", -1, 999_999_999),
            ("9223372036854775807.999999999", i64::MAX, 999_999_999),
            ("-9223372036854775808", i64::MIN, 0),
            ("-9223372036854775807.999999999", i64::MIN, 1),
        ] {
            let value = Timestamp { seconds, nanos };
            assert_eq!(Timestamp::parse(text), Ok(value));
            assert_eq!(value.decimal().unwrap(), text);
        }
        for text in [
            "",
            "-0",
            "-0.000",
            "+1",
            "01",
            "1.",
            ".1",
            "1.0000000001",
            "1.2.3",
            "1e2",
            "9223372036854775808",
            "-9223372036854775808.1",
        ] {
            assert_eq!(Timestamp::parse(text), Err(Error::Invalid), "{text}");
        }
        assert_eq!(
            Timestamp::parse("1.250000000").unwrap().decimal().unwrap(),
            "1.25"
        );
        assert_eq!(
            Timestamp {
                seconds: 0,
                nanos: 1_000_000_000
            }
            .decimal(),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn resolved_names_cannot_escape_safe_raw_header() {
        let h = header();
        for path in [
            "../escape",
            "/files/a",
            "files/../a",
            "files//a",
            "files/./a",
            "_AROS_BACKUP/complete.pax",
            "files/a/",
        ] {
            assert_eq!(
                resolve(
                    &h,
                    &[pax::Record {
                        key: "path",
                        value: path
                    }],
                    limits()
                ),
                Err(Error::Invalid)
            );
        }
        let records = [
            pax::Record {
                key: "path",
                value: "files/café",
            },
            pax::Record {
                key: "size",
                value: "18446744073709551615",
            },
            pax::Record {
                key: "mtime",
                value: "-0.25",
            },
        ];
        let member = resolve(&h, &records, limits()).unwrap();
        assert_eq!(member.path, "files/café");
        assert_eq!(member.size, u64::MAX);
        assert_eq!(
            member.mtime,
            Timestamp {
                seconds: -1,
                nanos: 750_000_000
            }
        );
        assert!(std::ptr::eq(
            member.path.as_ptr(),
            records[0].value.as_ptr()
        ));
    }
    #[test]
    fn link_semantics_and_unsupported_fields_are_explicit() {
        let mut h = header();
        for key in [
            "GNU.sparse.map",
            "SCHILY.xattr.user.test",
            "atime",
            "unknown",
        ] {
            assert_eq!(
                resolve(&h, &[pax::Record { key, value: "1" }], limits()),
                Err(Error::Unsupported)
            );
        }
        h.kind = tar::Kind::HardLink;
        h.size = 0;
        h.link = "files/target".into();
        assert!(resolve(&h, &[], limits()).is_ok());
        assert_eq!(
            resolve(
                &h,
                &[pax::Record {
                    key: "linkpath",
                    value: "../target"
                }],
                limits()
            ),
            Err(Error::Invalid)
        );
        h.kind = tar::Kind::Symlink;
        h.link = "../../target".into();
        assert!(resolve(&h, &[], limits()).is_ok()); // Target is data, not traversal authority.
        h.kind = tar::Kind::Directory;
        assert_eq!(resolve(&h, &[], limits()), Err(Error::Invalid));
        h.link.clear();
        assert_eq!(
            resolve(
                &h,
                &[pax::Record {
                    key: "size",
                    value: "1"
                }],
                limits()
            ),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn direct_records_and_raw_fields_obey_admission() {
        let mut h = header();
        let record = pax::Record {
            key: "path",
            value: "files/a",
        };
        assert_eq!(
            resolve(&h, &[record, record], limits()),
            Err(Error::Pax(pax::Error::DuplicateKey))
        );
        assert_eq!(
            resolve(
                &h,
                &[record],
                pax::Limits {
                    bytes: 1,
                    ..limits()
                }
            ),
            Err(Error::Pax(pax::Error::Limit))
        );
        for value in ["", "01", "-1", "18446744073709551616"] {
            assert_eq!(
                resolve(&h, &[pax::Record { key: "size", value }], limits()),
                Err(Error::Invalid)
            );
        }
        h.path = "x".repeat(513);
        assert_eq!(resolve(&h, &[], limits()), Err(Error::Limit));
        h.path = "files/a\0b".into();
        assert_eq!(resolve(&h, &[], limits()), Err(Error::Invalid));
    }
}
