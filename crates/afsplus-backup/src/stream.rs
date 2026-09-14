//! Streaming ordinary-member admission over the verified envelope framing.
//! Completion certifies framing and admitted fields, not preservation completeness.
use crate::{envelope, member, pax, tar};
use std::io::Read;
#[cfg(feature = "consumer")]
use std::sync::Arc;

#[derive(Debug)]
pub enum Error {
    Envelope(envelope::Error),
    Pax(pax::Error),
    Member(member::Error),
    Invalid(&'static str),
    Limit,
    Busy,
    Poisoned,
}

pub struct Reader<R> {
    inner: envelope::Reader<R>,
    limits: pax::Limits,
    header: Option<tar::Header>,
    records: Vec<u8>,
    remaining: u64,
    poisoned: bool,
    complete: bool,
    #[cfg(feature = "consumer")]
    identity: Arc<()>,
}
impl<R: Read> Reader<R> {
    pub fn new(input: R, framing: tar::Limits, records: pax::Limits) -> Result<Self, Error> {
        // Validate even an empty archive's configured metadata admission.
        pax::encoded_len(&[], records).map_err(Error::Pax)?;
        Ok(Self {
            inner: envelope::Reader::new(input, framing).map_err(Error::Envelope)?,
            limits: records,
            header: None,
            records: Vec::new(),
            remaining: 0,
            poisoned: false,
            complete: false,
            #[cfg(feature = "consumer")]
            identity: Arc::new(()),
        })
    }

    /// Advance only after consuming the current payload. Local records apply to
    /// exactly the following ordinary member; stacked or dangling blocks fail.
    pub fn next_member(&mut self) -> Result<Option<member::Member<'_>>, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.remaining != 0 {
            return Err(Error::Busy);
        }
        if self.complete {
            return Ok(None);
        }
        self.poisoned = true;
        self.header = None;
        self.records.clear();
        let Some(mut header) = self.inner.next_header().map_err(Error::Envelope)? else {
            self.complete = true;
            self.poisoned = false;
            return Ok(None);
        };
        if header.kind == tar::Kind::PaxLocal {
            let size = usize::try_from(header.size).map_err(|_| Error::Limit)?;
            if size > self.limits.bytes {
                return Err(Error::Limit);
            }
            self.inner.begin_payload(None).map_err(Error::Envelope)?;
            self.records.resize(size, 0);
            if size != 0 {
                self.inner
                    .read_payload(&mut self.records)
                    .map_err(Error::Envelope)?;
            }
            // Validate before reading the following header, with bounded work.
            pax::decode(&self.records, self.limits).map_err(Error::Pax)?;
            header = self
                .inner
                .next_header()
                .map_err(Error::Envelope)?
                .ok_or(Error::Invalid("dangling local PAX block"))?;
            if header.kind == tar::Kind::PaxLocal {
                return Err(Error::Invalid("stacked local PAX blocks"));
            }
        }
        self.header = Some(header);
        let records = pax::decode(&self.records, self.limits).map_err(Error::Pax)?;
        let effective = member::resolve(self.header.as_ref().unwrap(), &records, self.limits)
            .map_err(Error::Member)?;
        self.inner
            .begin_payload(if effective.kind == tar::Kind::File {
                Some(effective.size)
            } else {
                None
            })
            .map_err(Error::Envelope)?;
        self.remaining = effective.size;
        self.poisoned = false;
        Ok(Some(effective))
    }

    /// Reads into caller storage. Empty/finished payload reads return zero;
    /// uncertain I/O poisons the reader and withholds its completion receipt.
    pub fn read_payload(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.remaining == 0 || output.is_empty() {
            return Ok(0);
        }
        self.poisoned = true;
        let count = self.inner.read_payload(output).map_err(Error::Envelope)?;
        self.remaining -= count as u64;
        self.poisoned = false;
        Ok(count)
    }

    #[cfg(feature = "consumer")]
    pub(crate) fn identity(&self) -> Arc<()> {
        self.identity.clone()
    }

    #[cfg(feature = "consumer")]
    pub(crate) fn invalidate(&mut self) {
        self.poisoned = true;
    }

    pub fn receipt(&self) -> Option<&envelope::Receipt> {
        if self.complete && !self.poisoned {
            self.inner.receipt()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn framing() -> tar::Limits {
        tar::Limits {
            members: 16,
            member_bytes: 65536,
            trailing_zero_blocks: 0,
        }
    }
    fn records() -> pax::Limits {
        pax::Limits {
            bytes: 4096,
            records: 16,
            key_bytes: 64,
            value_bytes: 512,
        }
    }
    fn header(path: &str, kind: tar::Kind, size: u64) -> tar::Header {
        tar::Header {
            path: path.into(),
            link: String::new(),
            kind,
            mode: 0o600,
            uid: 0,
            gid: 0,
            size,
            mtime: 0,
            uname: String::new(),
            gname: String::new(),
        }
    }
    fn archive(local: &[pax::Record<'_>], following: bool, stacked: bool) -> Vec<u8> {
        let mut w = envelope::Writer::new(Vec::new(), framing()).unwrap();
        let wire = pax::encode(local, records()).unwrap();
        for _ in 0..if stacked { 2 } else { 1 } {
            w.start(
                &header(
                    "_AROS_BACKUP/metadata/local",
                    tar::Kind::PaxLocal,
                    wire.len() as u64,
                ),
                None,
            )
            .unwrap();
            w.write_payload(&wire).unwrap();
        }
        if following {
            w.start(&header("files/placeholder", tar::Kind::File, 0), Some(3))
                .unwrap();
            w.write_payload(b"abc").unwrap();
            w.start(&header("files/plain", tar::Kind::File, 1), None)
                .unwrap();
            w.write_payload(b"z").unwrap();
        }
        w.finish().unwrap().0
    }
    fn overrides() -> [pax::Record<'static>; 3] {
        [
            pax::Record {
                key: "path",
                value: "files/actual",
            },
            pax::Record {
                key: "size",
                value: "3",
            },
            pax::Record {
                key: "mtime",
                value: "-0.25",
            },
        ]
    }
    #[test]
    fn local_overrides_apply_once_and_payload_is_streamed() {
        let bytes = archive(&overrides(), true, false);
        let mut r = Reader::new(bytes.as_slice(), framing(), records()).unwrap();
        let m = r.next_member().unwrap().unwrap();
        assert_eq!(m.path, "files/actual");
        assert_eq!(m.size, 3);
        assert_eq!(
            m.mtime,
            member::Timestamp {
                seconds: -1,
                nanos: 750_000_000
            }
        );
        assert!(matches!(r.next_member(), Err(Error::Busy)));
        let mut out = [0; 2];
        assert_eq!(r.read_payload(&mut out).unwrap(), 2);
        assert_eq!(&out, b"ab");
        assert_eq!(r.read_payload(&mut out).unwrap(), 1);
        assert_eq!(out[0], b'c');
        assert_eq!(r.read_payload(&mut out).unwrap(), 0);
        let m = r.next_member().unwrap().unwrap();
        assert_eq!(m.path, "files/plain");
        assert_eq!(m.size, 1);
        assert_eq!(m.mtime.seconds, 0);
        assert!(r.receipt().is_none());
        assert_eq!(r.read_payload(&mut out).unwrap(), 1);
        assert_eq!(out[0], b'z');
        assert!(r.next_member().unwrap().is_none());
        assert!(r.receipt().is_some());
        assert!(r.next_member().unwrap().is_none());
    }
    #[test]
    fn local_errors_poison_and_never_expose_completion() {
        for bytes in [
            archive(&overrides(), false, false),
            archive(&overrides(), true, true),
            archive(
                &[pax::Record {
                    key: "path",
                    value: "../escape",
                }],
                true,
                false,
            ),
        ] {
            let mut r = Reader::new(bytes.as_slice(), framing(), records()).unwrap();
            assert!(r.next_member().is_err());
            assert!(matches!(r.next_member(), Err(Error::Poisoned)));
            assert!(matches!(r.read_payload(&mut [0; 1]), Err(Error::Poisoned)));
            assert!(r.receipt().is_none());
        }
        let bytes = archive(&overrides(), true, false);
        let mut r = Reader::new(
            bytes.as_slice(),
            framing(),
            pax::Limits {
                bytes: 1,
                ..records()
            },
        )
        .unwrap();
        assert!(matches!(r.next_member(), Err(Error::Limit)));
        assert!(r.records.is_empty());
    }
    #[test]
    fn every_truncation_withholds_receipt() {
        let bytes = archive(&overrides(), true, false);
        for end in 0..bytes.len() {
            let Ok(mut r) = Reader::new(&bytes[..end], framing(), records()) else {
                continue;
            };
            while let Ok(Some(_)) = r.next_member() {
                let mut out = [0; 7];
                while matches!(r.read_payload(&mut out), Ok(n) if n != 0) {}
            }
            assert!(r.receipt().is_none(), "prefix {end}");
        }
    }
}
