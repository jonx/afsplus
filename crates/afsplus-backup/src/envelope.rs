//! ADR-081 streamed integrity envelope. Receipts do not certify preservation semantics.
use crate::{pax, tar};
use sha2::{Digest, Sha512_256};
use std::io::{Read, Write};
const BEGIN: &str = "_AROS_BACKUP/begin";
const END: &str = "_AROS_BACKUP/complete.pax";
const MAX_BYTES: u128 = u128::MAX / 8;
const CONTROL: pax::Limits = pax::Limits {
    bytes: 4096,
    records: 4,
    key_bytes: 64,
    value_bytes: 128,
};
#[derive(Debug)]
pub enum Error {
    Tar(tar::Error),
    Pax(pax::Error),
    Invalid(&'static str),
    Limit,
    Poisoned,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backup envelope: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<tar::Error> for Error {
    fn from(e: tar::Error) -> Self {
        Self::Tar(e)
    }
}
impl From<pax::Error> for Error {
    fn from(e: pax::Error) -> Self {
        Self::Pax(e)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub bytes: u128,
    pub members: u64,
    pub hash: [u8; 32],
}
struct HashIo<T> {
    inner: T,
    bytes: u128,
    hash: Sha512_256,
}
impl<T> HashIo<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            bytes: 0,
            hash: Sha512_256::new(),
        }
    }
    fn admit(&self, count: usize) -> std::io::Result<()> {
        if count as u128 > MAX_BYTES - self.bytes {
            return Err(std::io::Error::other("hash message domain exhausted"));
        }
        Ok(())
    }
    fn update(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len() as u128;
        self.hash.update(bytes);
    }
    fn receipt(&self, members: u64) -> Receipt {
        Receipt {
            bytes: self.bytes,
            members,
            hash: self.hash.clone().finalize().into(),
        }
    }
}
impl<R: Read> Read for HashIo<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.admit(out.len())?;
        let count = self.inner.read(out)?;
        self.update(&out[..count]);
        Ok(count)
    }
}
impl<W: Write> Write for HashIo<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.admit(bytes.len())?;
        let count = self.inner.write(bytes)?;
        self.update(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
fn framing_limits(limits: tar::Limits) -> Result<tar::Limits, Error> {
    Ok(tar::Limits {
        members: limits.members.checked_add(2).ok_or(Error::Limit)?,
        member_bytes: limits.member_bytes.max(CONTROL.bytes as u64),
        ..limits
    })
}
fn control_header(path: &str, size: u64) -> tar::Header {
    tar::Header {
        path: path.into(),
        link: String::new(),
        kind: if path == END {
            tar::Kind::File
        } else {
            tar::Kind::PaxGlobal
        },
        mode: if path == END { 0o600 } else { 0 },
        uid: 0,
        gid: 0,
        size,
        mtime: 0,
        uname: String::new(),
        gname: String::new(),
    }
}
fn reserved(header: &tar::Header) -> bool {
    matches!(header.path.as_str(), BEGIN | END)
}
pub(crate) fn canonical(path: &str, directory: bool, metadata: bool) -> bool {
    if path.ends_with('/') && !directory {
        return false;
    }
    let clean = if directory {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    let namespace = clean.starts_with("files/")
        || (directory && clean == "files")
        || (metadata
            && (clean.starts_with("_AROS_BACKUP/metadata/")
                || (directory && clean == "_AROS_BACKUP/metadata")));
    namespace
        && clean
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}
fn body_header(header: &tar::Header) -> bool {
    if reserved(header) || header.kind == tar::Kind::PaxGlobal {
        return false;
    }
    canonical(&header.path, header.kind == tar::Kind::Directory, true)
        && (header.kind != tar::Kind::HardLink || canonical(&header.link, false, false))
}
fn begin_records() -> [pax::Record<'static>; 2] {
    [
        pax::Record {
            key: "AROS.backup.envelope",
            value: "2",
        },
        pax::Record {
            key: "AROS.backup.algorithm",
            value: "sha512-256",
        },
    ]
}
fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
fn field<'a>(records: &'a [pax::Record<'a>], key: &str) -> Result<&'a str, Error> {
    records
        .iter()
        .find(|r| r.key == key)
        .map(|r| r.value)
        .ok_or(Error::Invalid("missing control field"))
}
fn decimal(value: &str) -> Result<u128, Error> {
    let n: u128 = value.parse().map_err(|_| Error::Invalid("invalid count"))?;
    if n.to_string() != value {
        return Err(Error::Invalid("noncanonical count"));
    }
    Ok(n)
}
fn read_control<R: Read>(
    reader: &mut tar::Reader<HashIo<R>>,
    header: &tar::Header,
    path: &str,
) -> Result<Vec<u8>, Error> {
    if header.size > CONTROL.bytes as u64 {
        return Err(Error::Limit);
    }
    if *header != control_header(path, header.size) {
        return Err(Error::Invalid("control header fields"));
    }
    reader.begin_payload(None)?;
    let mut bytes = vec![0; header.size as usize];
    if !bytes.is_empty() {
        reader.read_payload(&mut bytes)?;
    }
    Ok(bytes)
}
/// Emits control records around caller-supplied body members. The caller owns
/// source enumeration, PAX interpretation, preservation checks and authorization.
pub struct Writer<W> {
    tar: tar::Writer<HashIo<W>>,
    limits: tar::Limits,
    members: u64,
}
impl<W: Write> Writer<W> {
    pub fn new(inner: W, limits: tar::Limits) -> Result<Self, Error> {
        let mut writer = Self {
            tar: tar::Writer::new(HashIo::new(inner), framing_limits(limits)?),
            limits,
            members: 0,
        };
        writer.control(BEGIN, &begin_records())?;
        Ok(writer)
    }
    fn control(&mut self, path: &str, records: &[pax::Record<'_>]) -> Result<(), Error> {
        let bytes = pax::encode(records, CONTROL)?;
        self.tar
            .start(&control_header(path, bytes.len() as u64), None)?;
        self.tar.write_payload(&bytes)?;
        Ok(())
    }
    pub fn start(&mut self, header: &tar::Header, override_size: Option<u64>) -> Result<(), Error> {
        if !body_header(header) {
            return Err(Error::Invalid("reserved envelope member"));
        }
        if self.members == self.limits.members
            || override_size.unwrap_or(header.size) > self.limits.member_bytes
        {
            return Err(Error::Limit);
        }
        self.tar.start(header, override_size)?;
        self.members += 1;
        Ok(())
    }
    pub fn write_payload(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.tar.write_payload(bytes)?;
        Ok(())
    }
    pub fn finish(mut self) -> Result<(W, Receipt), Error> {
        let receipt = self.tar.stream().receipt(self.members);
        let count = receipt.bytes.to_string();
        let members = receipt.members.to_string();
        let hash = hex(&receipt.hash);
        self.control(
            END,
            &[
                pax::Record {
                    key: "AROS.backup.end",
                    value: "2",
                },
                pax::Record {
                    key: "AROS.backup.bytes",
                    value: &count,
                },
                pax::Record {
                    key: "AROS.backup.members",
                    value: &members,
                },
                pax::Record {
                    key: "AROS.backup.hash",
                    value: &hash,
                },
            ],
        )?;
        let output = self.tar.finish()?;
        Ok((output.inner, receipt))
    }
}
/// The receipt appears only after terminal counts/digest and actual EOF verify.
/// Consuming a body member does not produce an integrity or preservation receipt.
pub struct Reader<R> {
    tar: tar::Reader<HashIo<R>>,
    limits: tar::Limits,
    members: u64,
    receipt: Option<Receipt>,
    poisoned: bool,
    pending_size: u64,
}
impl<R: Read> Reader<R> {
    pub fn new(inner: R, limits: tar::Limits) -> Result<Self, Error> {
        let mut reader = tar::Reader::new(HashIo::new(inner), framing_limits(limits)?);
        let header = reader
            .next_header()?
            .ok_or(Error::Invalid("missing beginning control"))?;
        let bytes = read_control(&mut reader, &header, BEGIN)?;
        let records = pax::decode(&bytes, CONTROL)?;
        if records.len() != 2
            || field(&records, "AROS.backup.envelope")? != "2"
            || field(&records, "AROS.backup.algorithm")? != "sha512-256"
        {
            return Err(Error::Invalid("unsupported envelope"));
        }
        Ok(Self {
            tar: reader,
            limits,
            members: 0,
            receipt: None,
            poisoned: false,
            pending_size: 0,
        })
    }
    pub fn receipt(&self) -> Option<&Receipt> {
        self.receipt.as_ref()
    }
    pub fn next_header(&mut self) -> Result<Option<tar::Header>, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.receipt.is_some() {
            return Ok(None);
        }
        let expected = self.tar.stream().receipt(self.members);
        let Some(header) = self.tar.next_header()? else {
            self.poisoned = true;
            return Err(Error::Invalid("missing completion control"));
        };
        if header.kind == tar::Kind::File && header.path == END {
            self.poisoned = true;
            let bytes = read_control(&mut self.tar, &header, END)?;
            let records = pax::decode(&bytes, CONTROL)?;
            if records.len() != 4
                || field(&records, "AROS.backup.end")? != "2"
                || decimal(field(&records, "AROS.backup.bytes")?)? != expected.bytes
                || decimal(field(&records, "AROS.backup.members")?)? != expected.members as u128
                || field(&records, "AROS.backup.hash")? != hex(&expected.hash)
            {
                return Err(Error::Invalid("completion mismatch"));
            }
            if self.tar.next_header()?.is_some() {
                return Err(Error::Invalid("member after completion"));
            }
            self.receipt = Some(expected);
            self.poisoned = false;
            return Ok(None);
        }
        if !body_header(&header) {
            self.poisoned = true;
            return Err(Error::Invalid("invalid body namespace or control"));
        }
        if self.members == self.limits.members {
            self.poisoned = true;
            return Err(Error::Limit);
        }
        self.members += 1;
        self.pending_size = header.size;
        Ok(Some(header))
    }
    pub fn begin_payload(&mut self, override_size: Option<u64>) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        // Framing admits larger controls; the body limit is checked separately.
        if override_size.unwrap_or(self.pending_size) > self.limits.member_bytes {
            return Err(Error::Limit);
        }
        self.tar.begin_payload(override_size)?;
        Ok(())
    }
    pub fn read_payload(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        Ok(self.tar.read_payload(out)?)
    }
}

#[cfg(test)]
mod tests;
