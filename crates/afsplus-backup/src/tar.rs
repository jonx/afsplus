//! Ustar framing with checked positive GNU size fields and explicit PAX overrides.
//! Names, metadata semantics, integrity and backup completion belong to the profile.
use std::io::{Read, Write};

pub const BLOCK: usize = 512;
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Invalid(&'static str),
    Limit,
    Busy,
    Poisoned,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "tar I/O: {e}"),
            Self::Invalid(e) => write!(f, "invalid tar: {e}"),
            Self::Limit => f.write_str("tar resource limit exceeded"),
            Self::Busy => f.write_str("tar payload requires completion"),
            Self::Poisoned => f.write_str("tar stream is poisoned"),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    HardLink,
    Symlink,
    Directory,
    PaxLocal,
    PaxGlobal,
}
impl Kind {
    fn byte(self) -> u8 {
        match self {
            Self::File => b'0',
            Self::HardLink => b'1',
            Self::Symlink => b'2',
            Self::Directory => b'5',
            Self::PaxLocal => b'x',
            Self::PaxGlobal => b'g',
        }
    }
    fn decode(b: u8) -> Result<Self, Error> {
        Ok(match b {
            0 | b'0' => Self::File,
            b'1' => Self::HardLink,
            b'2' => Self::Symlink,
            b'5' => Self::Directory,
            b'x' => Self::PaxLocal,
            b'g' => Self::PaxGlobal,
            _ => return Err(Error::Invalid("unsupported member type")),
        })
    }
    fn has_payload(self) -> bool {
        matches!(self, Self::File | Self::PaxLocal | Self::PaxGlobal)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub path: String,
    pub link: String,
    pub kind: Kind,
    pub mode: u32,
    pub uid: u64,
    pub gid: u64,
    /// Raw octal/positive-GNU size. Validated ordinary PAX may override it.
    pub size: u64,
    pub mtime: u64,
    pub uname: String,
    pub gname: String,
}
fn number(bytes: &[u8]) -> Result<u64, Error> {
    let start = bytes
        .iter()
        .position(|c| !matches!(c, 0 | b' '))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|c| !matches!(c, 0 | b' '))
        .map_or(start, |n| n + 1);
    bytes[start..end].iter().try_fold(0u64, |n, &c| {
        if !(b'0'..=b'7').contains(&c) {
            return Err(Error::Invalid("numeric field is not octal"));
        }
        n.checked_mul(8)
            .and_then(|n| n.checked_add((c - b'0') as u64))
            .ok_or(Error::Invalid("numeric overflow"))
    })
}
fn size_number(bytes: &[u8]) -> Result<u64, Error> {
    if bytes[0] & 0x80 == 0 {
        return number(bytes);
    }
    if bytes[0] != 0x80 {
        return Err(Error::Invalid("negative or overflowing binary size"));
    }
    bytes[1..].iter().try_fold(0u64, |n, &b| {
        n.checked_mul(256)
            .and_then(|n| n.checked_add(b as u64))
            .ok_or(Error::Invalid("binary size overflow"))
    })
}
fn put_size(out: &mut [u8], value: u64) -> Result<(), Error> {
    if value < (1u64 << 33) {
        return put_number(out, value);
    }
    out.fill(0);
    out[0] = 0x80;
    let end = out.len();
    out[end - 8..].copy_from_slice(&value.to_be_bytes());
    Ok(())
}
fn text(bytes: &[u8]) -> Result<&str, Error> {
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    if bytes[end..].iter().any(|&c| c != 0) {
        return Err(Error::Invalid("nonzero string padding"));
    }
    std::str::from_utf8(&bytes[..end]).map_err(|_| Error::Invalid("non-UTF-8 header text"))
}
fn put_text(out: &mut [u8], value: &str) -> Result<(), Error> {
    if value.len() > out.len() {
        return Err(Error::Limit);
    }
    if value.contains('\0') {
        return Err(Error::Invalid("NUL in header text"));
    }
    out[..value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}
fn put_number(out: &mut [u8], value: u64) -> Result<(), Error> {
    let octal = format!("{value:o}");
    if octal.len() >= out.len() {
        return Err(Error::Limit);
    }
    out.fill(b'0');
    let end = out.len() - 1;
    out[end] = 0;
    out[end - octal.len()..end].copy_from_slice(octal.as_bytes());
    Ok(())
}
fn checksum(block: &[u8; BLOCK]) -> u64 {
    block
        .iter()
        .enumerate()
        .map(|(n, &b)| {
            if (148..156).contains(&n) {
                b' ' as u64
            } else {
                b as u64
            }
        })
        .sum()
}
impl Header {
    pub fn decode(block: &[u8; BLOCK]) -> Result<Self, Error> {
        if number(&block[148..156])? != checksum(block) {
            return Err(Error::Invalid("header checksum"));
        }
        if &block[257..263] != b"ustar\0" || &block[263..265] != b"00" {
            return Err(Error::Invalid("unsupported header format"));
        }
        if block[500..].iter().any(|&c| c != 0) {
            return Err(Error::Invalid("reserved header bytes"));
        }
        let name = text(&block[..100])?;
        if name.is_empty() {
            return Err(Error::Invalid("empty member name"));
        }
        let prefix = text(&block[345..500])?;
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let kind = Kind::decode(block[156])?;
        let size = size_number(&block[124..136])?;
        if !kind.has_payload() && size != 0 {
            return Err(Error::Invalid("non-data member size"));
        }
        if number(&block[329..337])? != 0 || number(&block[337..345])? != 0 {
            return Err(Error::Invalid("device numbers on supported member"));
        }
        Ok(Self {
            path,
            link: text(&block[157..257])?.to_owned(),
            kind,
            mode: u32::try_from(number(&block[100..108])?)
                .map_err(|_| Error::Invalid("mode overflow"))?,
            uid: number(&block[108..116])?,
            gid: number(&block[116..124])?,
            size,
            mtime: number(&block[136..148])?,
            uname: text(&block[265..297])?.to_owned(),
            gname: text(&block[297..329])?.to_owned(),
        })
    }
    pub fn encode(&self) -> Result<[u8; BLOCK], Error> {
        if self.path.is_empty() {
            return Err(Error::Invalid("empty member name"));
        }
        if self.path.len() > 256 {
            return Err(Error::Limit);
        }
        if !self.kind.has_payload() && self.size != 0 {
            return Err(Error::Invalid("non-data member size"));
        }
        let mut block = [0; BLOCK];
        let (prefix, name) = if self.path.len() <= 100 {
            ("", self.path.as_str())
        } else {
            let split = self
                .path
                .rmatch_indices('/')
                .find(|(at, _)| {
                    *at <= 155 && self.path.len() - at - 1 <= 100 && *at + 1 < self.path.len()
                })
                .ok_or(Error::Limit)?
                .0;
            (&self.path[..split], &self.path[split + 1..])
        };
        put_text(&mut block[..100], name)?;
        put_text(&mut block[345..500], prefix)?;
        put_number(&mut block[100..108], self.mode as u64)?;
        put_number(&mut block[108..116], self.uid)?;
        put_number(&mut block[116..124], self.gid)?;
        put_size(&mut block[124..136], self.size)?;
        put_number(&mut block[136..148], self.mtime)?;
        block[156] = self.kind.byte();
        put_text(&mut block[157..257], &self.link)?;
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        put_text(&mut block[265..297], &self.uname)?;
        put_text(&mut block[297..329], &self.gname)?;
        put_number(&mut block[329..337], 0)?;
        put_number(&mut block[337..345], 0)?;
        let check = format!("{:06o}\0 ", checksum(&block));
        block[148..156].copy_from_slice(check.as_bytes());
        Ok(block)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub members: u64,
    pub member_bytes: u64,
    pub trailing_zero_blocks: usize,
}
#[derive(Debug, Clone, Copy)]
enum State {
    Header,
    Size(Kind, u64),
    Body(u64, usize),
    End,
    Poisoned,
}
fn padding(size: u64) -> usize {
    ((BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64) as usize
}
fn effective(kind: Kind, raw: u64, override_size: Option<u64>, limit: u64) -> Result<u64, Error> {
    if override_size.is_some() && kind != Kind::File {
        return Err(Error::Invalid("size override requires regular file"));
    }
    let size = override_size.unwrap_or(raw);
    if size > limit {
        return Err(Error::Limit);
    }
    Ok(size)
}
/// The profile must validate a PAX size override before passing it to begin_payload.
/// Errors after reading stream bytes poison the reader; Busy/limit admission can be retried.
pub struct Reader<R> {
    inner: R,
    limits: Limits,
    count: u64,
    state: State,
}
impl<R: Read> Reader<R> {
    pub(crate) fn stream(&self) -> &R {
        &self.inner
    }
    pub fn new(inner: R, limits: Limits) -> Self {
        Self {
            inner,
            limits,
            count: 0,
            state: State::Header,
        }
    }
    pub fn next_header(&mut self) -> Result<Option<Header>, Error> {
        match self.state {
            State::End => return Ok(None),
            State::Poisoned => return Err(Error::Poisoned),
            State::Header => {}
            _ => return Err(Error::Busy),
        }
        self.state = State::Poisoned;
        let mut block = [0; BLOCK];
        self.inner.read_exact(&mut block)?;
        if block.iter().all(|&c| c == 0) {
            self.inner.read_exact(&mut block)?;
            if block.iter().any(|&c| c != 0) {
                return Err(Error::Invalid("missing second end block"));
            }
            let mut extra = 0usize;
            loop {
                let count = loop {
                    match self.inner.read(&mut block[..1]) {
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        other => break other?,
                    }
                };
                if count == 0 {
                    break;
                }
                if extra == self.limits.trailing_zero_blocks {
                    return Err(Error::Limit);
                }
                self.inner.read_exact(&mut block[1..])?;
                if block.iter().any(|&c| c != 0) {
                    return Err(Error::Invalid("data after end markers"));
                }
                extra += 1;
            }
            self.state = State::End;
            return Ok(None);
        }
        if self.count == self.limits.members {
            return Err(Error::Limit);
        }
        let header = Header::decode(&block)?;
        self.count += 1;
        self.state = State::Size(header.kind, header.size);
        Ok(Some(header))
    }
    /// Select raw header size or an independently validated PAX override exactly once.
    pub fn begin_payload(&mut self, override_size: Option<u64>) -> Result<(), Error> {
        let State::Size(kind, raw) = self.state else {
            return Err(if matches!(self.state, State::Poisoned) {
                Error::Poisoned
            } else {
                Error::Busy
            });
        };
        let size = effective(kind, raw, override_size, self.limits.member_bytes)?;
        self.state = if size == 0 {
            State::Header
        } else {
            State::Body(size, padding(size))
        };
        Ok(())
    }
    pub fn read_payload(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        let State::Body(left, pad) = self.state else {
            return Err(if matches!(self.state, State::Poisoned) {
                Error::Poisoned
            } else {
                Error::Busy
            });
        };
        let count = out.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        if count == 0 {
            return Ok(0);
        }
        self.state = State::Poisoned;
        self.inner.read_exact(&mut out[..count])?;
        let remaining = left - count as u64;
        if remaining == 0 {
            let mut zeros = [0; BLOCK];
            self.inner.read_exact(&mut zeros[..pad])?;
            if zeros[..pad].iter().any(|&c| c != 0) {
                return Err(Error::Invalid("nonzero payload padding"));
            }
            self.state = State::Header;
        } else {
            self.state = State::Body(remaining, pad);
        }
        Ok(count)
    }
}
/// Streaming writer. Flush completes framing only; the destination owns durability.
pub struct Writer<W> {
    inner: W,
    limits: Limits,
    count: u64,
    remaining: u64,
    pad: usize,
    poisoned: bool,
}
impl<W: Write> Writer<W> {
    pub(crate) fn stream(&self) -> &W {
        &self.inner
    }
    pub fn new(inner: W, limits: Limits) -> Self {
        Self {
            inner,
            limits,
            count: 0,
            remaining: 0,
            pad: 0,
            poisoned: false,
        }
    }
    pub fn start(&mut self, header: &Header, override_size: Option<u64>) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.remaining != 0 {
            return Err(Error::Busy);
        }
        if self.count == self.limits.members {
            return Err(Error::Limit);
        }
        let size = effective(
            header.kind,
            header.size,
            override_size,
            self.limits.member_bytes,
        )?;
        let block = header.encode()?;
        self.poisoned = true;
        self.inner.write_all(&block)?;
        self.count += 1;
        self.remaining = size;
        self.pad = padding(size);
        self.poisoned = false;
        Ok(())
    }
    pub fn write_payload(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if bytes.len() as u128 > self.remaining as u128 {
            return Err(Error::Limit);
        }
        self.poisoned = true;
        self.inner.write_all(bytes)?;
        self.remaining -= bytes.len() as u64;
        if self.remaining == 0 {
            self.inner.write_all(&[0; BLOCK][..self.pad])?;
            self.pad = 0;
        }
        self.poisoned = false;
        Ok(())
    }
    pub fn finish(mut self) -> Result<W, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.remaining != 0 {
            return Err(Error::Busy);
        }
        self.inner.write_all(&[0; BLOCK * 2])?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
mod tests;
