//! ADR-082 exact object metadata. Decode success is not preservation completion.
use crate::{envelope, member, pax, tar};
use member::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inventory {
    Empty,
    Present,
    Uninspected,
}
impl Inventory {
    fn text(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Present => "present",
            Self::Uninspected => "uninspected",
        }
    }
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "empty" => Ok(Self::Empty),
            "present" => Ok(Self::Present),
            "uninspected" => Ok(Self::Uninspected),
            _ => Err(Error::Invalid),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Pax(pax::Error),
    Invalid,
    Unsupported,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Object<'a> {
    pub path: &'a str,
    pub kind: tar::Kind,
    pub protection: u64,
    /// The AROS comment, empty when the object has none (ADR-106, ADR-119).
    pub comment: &'a str,
    /// The owner, which tar headers of this archive never carry (ADR-118, ADR-119).
    pub uid: u32,
    pub gid: u32,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub changed: Timestamp,
    pub attributes: Inventory,
    pub security: Inventory,
}
fn kind_text(kind: tar::Kind) -> Result<&'static str, Error> {
    match kind {
        tar::Kind::File => Ok("file"),
        tar::Kind::Directory => Ok("directory"),
        tar::Kind::Symlink => Ok("symlink"),
        tar::Kind::HardLink => Ok("hardlink"),
        _ => Err(Error::Unsupported),
    }
}
impl Object<'_> {
    fn validate(&self) -> Result<(), Error> {
        kind_text(self.kind)?;
        if !envelope::canonical(self.path, self.kind == tar::Kind::Directory, false)
            || self.path.contains('\0')
            || self.comment.contains('\0')
            || [self.created, self.modified, self.changed]
                .iter()
                .any(|t| t.nanos >= 1_000_000_000)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
/// The payload version this build writes and the only one it reads: version
/// 1 carried no comment and no owner, and there is no legacy (ADR-119).
const VERSION: &str = "2";
const KEYS: [&str; 12] = [
    "AROS.object.version",
    "AROS.object.path",
    "AROS.object.kind",
    "AROS.object.protection",
    "AROS.object.created",
    "AROS.object.modified",
    "AROS.object.changed",
    "AROS.object.attributes",
    "AROS.object.security",
    "AROS.object.comment",
    "AROS.object.uid",
    "AROS.object.gid",
];

pub fn decode(input: &[u8], limits: pax::Limits) -> Result<Object<'_>, Error> {
    let records = pax::decode(input, limits).map_err(Error::Pax)?;
    if records.iter().any(|r| !KEYS.contains(&r.key)) {
        return Err(Error::Unsupported);
    }
    if records.len() != KEYS.len() {
        return Err(Error::Invalid);
    }
    let field = |index: usize| {
        records
            .iter()
            .find(|r| r.key == KEYS[index])
            .map(|r| r.value)
            .ok_or(Error::Invalid)
    };
    if field(0)? != VERSION {
        return Err(Error::Unsupported);
    }
    let kind = match field(2)? {
        "file" => tar::Kind::File,
        "directory" => tar::Kind::Directory,
        "symlink" => tar::Kind::Symlink,
        "hardlink" => tar::Kind::HardLink,
        _ => return Err(Error::Unsupported),
    };
    let object = Object {
        path: field(1)?,
        kind,
        protection: member::unsigned(field(3)?).map_err(|_| Error::Invalid)?,
        created: Timestamp::parse(field(4)?).map_err(|_| Error::Invalid)?,
        modified: Timestamp::parse(field(5)?).map_err(|_| Error::Invalid)?,
        changed: Timestamp::parse(field(6)?).map_err(|_| Error::Invalid)?,
        attributes: Inventory::parse(field(7)?)?,
        security: Inventory::parse(field(8)?)?,
        comment: field(9)?,
        uid: identity(field(10)?)?,
        gid: identity(field(11)?)?,
    };
    object.validate()?;
    Ok(object)
}

/// A canonical decimal owner identity: 0 to 4294967295, no sign, no leading zero.
fn identity(text: &str) -> Result<u32, Error> {
    let value = member::unsigned(text).map_err(|_| Error::Invalid)?;
    u32::try_from(value).map_err(|_| Error::Invalid)
}

pub fn encode(object: &Object<'_>, limits: pax::Limits) -> Result<Vec<u8>, Error> {
    object.validate()?;
    let protection = object.protection.to_string();
    let created = object.created.decimal().map_err(|_| Error::Invalid)?;
    let modified = object.modified.decimal().map_err(|_| Error::Invalid)?;
    let changed = object.changed.decimal().map_err(|_| Error::Invalid)?;
    let uid = object.uid.to_string();
    let gid = object.gid.to_string();
    let values = [
        VERSION,
        object.path,
        kind_text(object.kind)?,
        &protection,
        &created,
        &modified,
        &changed,
        object.attributes.text(),
        object.security.text(),
        object.comment,
        &uid,
        &gid,
    ];
    let records: [pax::Record<'_>; 12] = std::array::from_fn(|i| pax::Record {
        key: KEYS[i],
        value: values[i],
    });
    pax::encode(&records, limits).map_err(Error::Pax)
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
    fn object() -> Object<'static> {
        Object {
            path: "files/café",
            kind: tar::Kind::File,
            protection: u64::MAX,
            created: Timestamp {
                seconds: i64::MIN,
                nanos: 1,
            },
            modified: Timestamp {
                seconds: -1,
                nanos: 750_000_000,
            },
            changed: Timestamp {
                seconds: i64::MAX,
                nanos: 999_999_999,
            },
            attributes: Inventory::Uninspected,
            security: Inventory::Present,
            comment: "Réglé à 100%, voir la note",
            uid: u32::MAX,
            gid: 20,
        }
    }
    #[test]
    fn exact_fields_and_inventory_states_round_trip() {
        for attributes in [Inventory::Empty, Inventory::Present, Inventory::Uninspected] {
            for security in [Inventory::Empty, Inventory::Present, Inventory::Uninspected] {
                let o = Object {
                    attributes,
                    security,
                    ..object()
                };
                let wire = encode(&o, limits()).unwrap();
                let got = decode(&wire, limits()).unwrap();
                assert_eq!(got, o);
                assert!(got.path.as_ptr() >= wire.as_ptr());
                assert!((got.path.as_ptr() as usize) < wire.as_ptr() as usize + wire.len());
                let mut records = pax::decode(&wire, limits()).unwrap();
                records.reverse();
                assert_eq!(
                    decode(&pax::encode(&records, limits()).unwrap(), limits()).unwrap(),
                    o
                );
            }
        }
    }
    #[test]
    fn required_fields_never_default_and_unknowns_fail() {
        let wire = encode(&object(), limits()).unwrap();
        let records = pax::decode(&wire, limits()).unwrap();
        for index in 0..records.len() {
            let mut changed = records.clone();
            changed.remove(index);
            assert_eq!(
                decode(&pax::encode(&changed, limits()).unwrap(), limits()),
                Err(Error::Invalid)
            );
        }
        for (index, value, expected) in [
            (0, "1", Error::Unsupported),
            (0, "3", Error::Unsupported),
            (10, "4294967296", Error::Invalid),
            (11, "-1", Error::Invalid),
            (10, "007", Error::Invalid),
            (2, "device", Error::Unsupported),
            (3, "01", Error::Invalid),
            (7, "", Error::Invalid),
            (8, "unknown", Error::Invalid),
            (1, "../escape", Error::Invalid),
        ] {
            let mut changed = records.clone();
            changed[index].value = value;
            assert_eq!(
                decode(&pax::encode(&changed, limits()).unwrap(), limits()),
                Err(expected)
            );
        }
        let mut changed = records.clone();
        changed[0].key = "AROS.object.future";
        assert_eq!(
            decode(&pax::encode(&changed, limits()).unwrap(), limits()),
            Err(Error::Unsupported)
        );
        let mut duplicate = wire.clone();
        duplicate.extend_from_slice(&wire);
        assert!(matches!(
            decode(
                &duplicate,
                pax::Limits {
                    records: 32,
                    ..limits()
                }
            ),
            Err(Error::Pax(pax::Error::DuplicateKey))
        ));
    }
    #[test]
    fn truncation_namespace_and_budgets_are_strict() {
        let wire = encode(&object(), limits()).unwrap();
        for end in 0..wire.len() {
            assert!(decode(&wire[..end], limits()).is_err());
        }
        let small = pax::Limits {
            bytes: wire.len() - 1,
            ..limits()
        };
        assert_eq!(encode(&object(), small), Err(Error::Pax(pax::Error::Limit)));
        assert_eq!(decode(&wire, small), Err(Error::Pax(pax::Error::Limit)));
        for path in [
            "_AROS_BACKUP/metadata/x",
            "files/../x",
            "files//x",
            "files/x/",
        ] {
            assert_eq!(
                encode(&Object { path, ..object() }, limits()),
                Err(Error::Invalid)
            );
        }
        let root = Object {
            path: "files",
            kind: tar::Kind::Directory,
            ..object()
        };
        assert_eq!(
            decode(&encode(&root, limits()).unwrap(), limits()).unwrap(),
            root
        );
        assert_eq!(
            encode(
                &Object {
                    created: Timestamp {
                        seconds: 0,
                        nanos: 1_000_000_000
                    },
                    ..object()
                },
                limits()
            ),
            Err(Error::Invalid)
        );
    }
}
