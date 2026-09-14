//! ADR-085 bounded, Merkle-checked scratch capture and replay.
use crate::{pax, stream, tar};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Seek, SeekFrom, Write};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Stream(stream::Error),
    Limit,
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub chunk_bytes: usize,
    pub archive_bytes: u64,
    pub store_bytes: u64,
}
#[derive(Debug, Clone, Copy)]
pub struct Stats {
    pub archive_bytes: u64,
    pub store_bytes: u64,
    pub chunks: u64,
    pub tree_levels: usize,
    pub chunk_bytes: usize,
}
#[derive(Clone, Copy)]
struct Level {
    start: u64,
    count: u64,
    stride: u64,
}
impl Level {
    fn position(self, index: u64) -> io::Result<u64> {
        if index >= self.count {
            return Err(invalid("spool tree index"));
        }
        index
            .checked_mul(self.stride)
            .and_then(|n| n.checked_add(self.start))
            .ok_or_else(|| invalid("spool offset overflow"))
    }
}
fn invalid(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}
fn leaf(index: u64, data: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update([0]);
    hash.update(index.to_le_bytes());
    hash.update((data.len() as u64).to_le_bytes());
    hash.update(data);
    hash.finalize().into()
}
fn parent(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update([1]);
    hash.update(left);
    hash.update(right);
    hash.finalize().into()
}
fn read_retry<R: Read>(input: &mut R, out: &mut [u8]) -> io::Result<usize> {
    loop {
        match input.read(out) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => (),
            result => return result,
        }
    }
}
fn read_hash<S: Read + Seek>(store: &mut S, level: Level, index: u64) -> io::Result<[u8; 32]> {
    store.seek(SeekFrom::Start(level.position(index)?))?;
    let mut hash = [0; 32];
    store.read_exact(&mut hash)?;
    Ok(hash)
}
/// Owns scratch storage and its privately retained integrity root. Construction
/// succeeds only after the complete captured archive verifies through that root.
pub struct Verified<S> {
    store: S,
    levels: Vec<Level>,
    root: [u8; 32],
    stats: Stats,
    sparse: bool,
}
impl<S: Read + Write + Seek> Verified<S> {
    pub fn capture<R: Read>(
        input: R,
        store: S,
        limits: Limits,
        framing: tar::Limits,
        records: pax::Limits,
    ) -> Result<Self, Error> {
        Self::capture_mode(input, store, limits, framing, records, false)
    }
    /// Explicit sparse-header admission; content maps require semantic validation.
    pub fn capture_sparse<R: Read>(
        input: R,
        store: S,
        limits: Limits,
        framing: tar::Limits,
        records: pax::Limits,
    ) -> Result<Self, Error> {
        Self::capture_mode(input, store, limits, framing, records, true)
    }
    fn capture_mode<R: Read>(
        mut input: R,
        store: S,
        limits: Limits,
        framing: tar::Limits,
        records: pax::Limits,
        sparse: bool,
    ) -> Result<Self, Error> {
        let mut spool = Self::copy(&mut input, store, limits)?;
        spool.sparse = sparse;
        {
            // This first pass deliberately has no early-publication capability.
            let cursor = Replay::new(&mut spool);
            let mut reader = stream::Reader::new_mode(cursor, framing, records, sparse)
                .map_err(Error::Stream)?;
            let mut discard = [0; 512];
            while let Some(member) = reader.next_member().map_err(Error::Stream)? {
                let mut remaining = member.size;
                while remaining != 0 {
                    let count = reader.read_payload(&mut discard).map_err(Error::Stream)?;
                    if count == 0 {
                        return Err(invalid("spool verification made no progress").into());
                    }
                    remaining -= count as u64;
                }
            }
            if reader.receipt().is_none() {
                return Err(invalid("spool lacks completion").into());
            }
        }
        Ok(spool)
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }
    pub fn reader(
        &mut self,
        framing: tar::Limits,
        records: pax::Limits,
    ) -> Result<stream::Reader<Replay<'_, S>>, Error> {
        let sparse = self.sparse;
        let mut reader = stream::Reader::new_mode(Replay::new(self), framing, records, sparse)
            .map_err(Error::Stream)?;
        reader.admit_verified_source();
        Ok(reader)
    }
    fn copy<R: Read>(input: &mut R, mut store: S, limits: Limits) -> Result<Self, Error> {
        if !(512..=1_048_576).contains(&limits.chunk_bytes)
            || !limits.chunk_bytes.is_power_of_two()
            || limits.archive_bytes == 0
            || limits.store_bytes == 0
        {
            return Err(Error::Limit);
        }
        let chunk = limits.chunk_bytes as u64;
        let stride = chunk + 32;
        let mut buffer = vec![0; limits.chunk_bytes];
        let mut bytes = 0u64;
        let mut chunks = 0u64;
        let mut end = 0u64;
        loop {
            buffer.fill(0);
            let mut used = 0;
            let mut eof = false;
            while used < buffer.len() {
                let room = limits.archive_bytes - bytes - used as u64;
                if room == 0 {
                    if read_retry(input, &mut [0; 1])? != 0 {
                        return Err(Error::Limit);
                    }
                    eof = true;
                    break;
                }
                let want = (buffer.len() - used).min(room.min(usize::MAX as u64) as usize);
                let count = read_retry(input, &mut buffer[used..used + want])?;
                if count == 0 {
                    eof = true;
                    break;
                }
                used += count;
            }
            if used != 0 {
                let next = end
                    .checked_add(stride)
                    .filter(|&n| n <= limits.store_bytes)
                    .ok_or(Error::Limit)?;
                store.seek(SeekFrom::Start(end))?;
                store.write_all(&buffer)?;
                store.write_all(&leaf(chunks, &buffer[..used]))?;
                bytes += used as u64;
                chunks += 1;
                end = next;
            }
            if eof {
                break;
            }
        }
        let mut levels = vec![Level {
            start: chunk,
            count: chunks,
            stride,
        }];
        while levels.last().unwrap().count > 1 {
            let previous = *levels.last().unwrap();
            let count = previous.count.div_ceil(2);
            let next = count
                .checked_mul(32)
                .and_then(|n| n.checked_add(end))
                .filter(|&n| n <= limits.store_bytes)
                .ok_or(Error::Limit)?;
            let level = Level {
                start: end,
                count,
                stride: 32,
            };
            for index in 0..count {
                let left = read_hash(&mut store, previous, index * 2)?;
                let hash = if index * 2 + 1 < previous.count {
                    parent(&left, &read_hash(&mut store, previous, index * 2 + 1)?)
                } else {
                    left
                };
                store.seek(SeekFrom::Start(level.position(index)?))?;
                store.write_all(&hash)?;
            }
            levels.push(level);
            end = next;
        }
        let root = if chunks == 0 {
            Sha256::digest([]).into()
        } else {
            read_hash(&mut store, *levels.last().unwrap(), 0)?
        };
        store.flush()?;
        let stats = Stats {
            archive_bytes: bytes,
            store_bytes: end,
            chunks,
            tree_levels: levels.len(),
            chunk_bytes: limits.chunk_bytes,
        };
        Ok(Self {
            store,
            levels,
            root,
            stats,
            sparse: false,
        })
    }
}
/// Sequential view whose bytes are checked before they reach archive parsing.
pub struct Replay<'a, S> {
    spool: &'a mut Verified<S>,
    position: u64,
    cached: Option<u64>,
    buffer: Vec<u8>,
    failed: bool,
}
impl<'a, S: Read + Write + Seek> Replay<'a, S> {
    fn new(spool: &'a mut Verified<S>) -> Self {
        let buffer = vec![0; spool.stats.chunk_bytes];
        Self {
            spool,
            position: 0,
            cached: None,
            buffer,
            failed: false,
        }
    }
    fn load(&mut self, index: u64) -> io::Result<()> {
        let chunk = self.spool.stats.chunk_bytes as u64;
        let start = index
            .checked_mul(chunk + 32)
            .ok_or_else(|| invalid("spool data offset"))?;
        self.spool.store.seek(SeekFrom::Start(start))?;
        self.spool.store.read_exact(&mut self.buffer)?;
        let length = (self.spool.stats.archive_bytes - index * chunk).min(chunk) as usize;
        if self.buffer[length..].iter().any(|&b| b != 0) {
            return Err(invalid("spool chunk padding"));
        }
        let mut hash = leaf(index, &self.buffer[..length]);
        let mut node = index;
        for &level in &self.spool.levels {
            if level.count == 1 {
                break;
            }
            if node & 1 != 0 {
                hash = parent(&read_hash(&mut self.spool.store, level, node - 1)?, &hash);
            } else if node + 1 < level.count {
                hash = parent(&hash, &read_hash(&mut self.spool.store, level, node + 1)?);
            }
            node /= 2;
        }
        if hash != self.spool.root {
            return Err(invalid("spool Merkle proof mismatch"));
        }
        self.cached = Some(index);
        Ok(())
    }
}
impl<S: Read + Write + Seek> Read for Replay<'_, S> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.failed {
            return Err(invalid("failed spool replay"));
        }
        if output.is_empty() || self.position == self.spool.stats.archive_bytes {
            return Ok(0);
        }
        self.failed = true;
        let chunk = self.spool.stats.chunk_bytes as u64;
        let index = self.position / chunk;
        if self.cached != Some(index) {
            self.load(index)?;
        }
        let offset = (self.position % chunk) as usize;
        let count = output
            .len()
            .min(self.buffer.len() - offset)
            .min((self.spool.stats.archive_bytes - self.position).min(usize::MAX as u64) as usize);
        output[..count].copy_from_slice(&self.buffer[offset..offset + count]);
        self.position += count as u64;
        self.failed = false;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope;
    use std::io::Cursor;
    fn limits() -> Limits {
        Limits {
            chunk_bytes: 512,
            archive_bytes: 65536,
            store_bytes: 131072,
        }
    }
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
            value_bytes: 2048,
        }
    }
    fn archive() -> Vec<u8> {
        let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
        writer
            .start(
                &tar::Header {
                    path: "files".into(),
                    link: String::new(),
                    kind: tar::Kind::Directory,
                    mode: 0o700,
                    uid: 0,
                    gid: 0,
                    size: 0,
                    mtime: 0,
                    uname: String::new(),
                    gname: String::new(),
                },
                None,
            )
            .unwrap();
        writer.finish().unwrap().0
    }
    #[test]
    fn independent_merkle_root_odd_trees_and_partial_chunks() {
        let bytes: Vec<u8> = (0..1280).map(|i| (i % 251) as u8).collect();
        let mut spool =
            Verified::copy(&mut bytes.as_slice(), Cursor::new(Vec::new()), limits()).unwrap();
        // Python hashlib + independent recursive RFC-6962 tree split at the
        // largest power of two smaller than the number of leaves.
        let expected = "1b2cb20a796b6b6cdabbe36268922cac3156f6e443d4839ab8d5534227f994f1";
        assert_eq!(
            spool
                .root
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            expected
        );
        for length in [1, 511, 512, 513, 1024, 1280, 1536, 2049, 3584, 4097] {
            let input: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
            let mut spool =
                Verified::copy(&mut input.as_slice(), Cursor::new(Vec::new()), limits()).unwrap();
            let mut result = vec![];
            Replay::new(&mut spool).read_to_end(&mut result).unwrap();
            assert_eq!(result, input);
            let stats = spool.stats();
            assert_eq!(stats.chunks, (length as u64).div_ceil(512));
            assert_eq!(stats.store_bytes, spool.store.get_ref().len() as u64);
            assert!(
                stats.store_bytes
                    <= stats.archive_bytes
                        + 511
                        + stats.chunks * 64
                        + stats.tree_levels as u64 * 32
            );
        }
        let mut result = vec![];
        Replay::new(&mut spool).read_to_end(&mut result).unwrap();
        assert_eq!(result, bytes);
    }
    #[test]
    fn changed_resealed_reordered_or_truncated_scratch_never_returns_bad_bytes() {
        let bytes: Vec<u8> = (0..1280).map(|i| (i % 251) as u8).collect();
        for fault in 0..5 {
            let mut spool =
                Verified::copy(&mut bytes.as_slice(), Cursor::new(Vec::new()), limits()).unwrap();
            match fault {
                0 => spool.store.get_mut()[0] ^= 1,
                1 => {
                    spool.store.get_mut()[0] ^= 1;
                    let resealed = leaf(0, &spool.store.get_ref()[..512]);
                    spool.store.get_mut()[512..544].copy_from_slice(&resealed);
                }
                2 => {
                    let position = spool.levels[1].position(1).unwrap() as usize;
                    spool.store.get_mut()[position] ^= 1;
                }
                3 => {
                    let old = spool.store.get_ref()[..1088].to_vec();
                    spool.store.get_mut()[..544].copy_from_slice(&old[544..]);
                    spool.store.get_mut()[544..1088].copy_from_slice(&old[..544]);
                }
                _ => spool.store.get_mut().truncate(600),
            }
            let mut replay = Replay::new(&mut spool);
            let mut output = [99; 7];
            assert!(replay.read(&mut output).is_err());
            assert_eq!(output, [99; 7]);
            assert!(replay.read(&mut output).is_err());
        }
        let mut spool =
            Verified::copy(&mut bytes.as_slice(), Cursor::new(Vec::new()), limits()).unwrap();
        spool.store.get_mut()[2 * 544 + 300] = 1; // Final chunk's unused padding.
        let mut replay = Replay::new(&mut spool);
        replay.position = 1024;
        let mut output = [99; 1];
        assert!(replay.read(&mut output).is_err());
        assert_eq!(output, [99]);
    }
    #[derive(Default)]
    struct FaultStore {
        inner: Cursor<Vec<u8>>,
        read: bool,
        write: bool,
        seek: bool,
        flush: bool,
        max_read: usize,
        read_calls: u64,
        read_bytes: u64,
        write_calls: u64,
        write_bytes: u64,
    }
    impl Read for FaultStore {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.max_read = self.max_read.max(out.len());
            self.read_calls += 1;
            if self.read {
                Err(io::Error::other("read"))
            } else {
                let count = self.inner.read(out)?;
                self.read_bytes += count as u64;
                Ok(count)
            }
        }
    }
    impl Write for FaultStore {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.write_calls += 1;
            if self.write {
                Err(io::Error::other("write"))
            } else {
                let count = self.inner.write(bytes)?;
                self.write_bytes += count as u64;
                Ok(count)
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.flush {
                Err(io::Error::other("flush"))
            } else {
                Ok(())
            }
        }
    }
    impl Seek for FaultStore {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            if self.seek {
                Err(io::Error::other("seek"))
            } else {
                self.inner.seek(position)
            }
        }
    }
    #[test]
    fn capture_requires_complete_archive_and_admitted_scratch_io() {
        let wire = archive();
        for fault in 0..4 {
            let store = FaultStore {
                read: fault == 0,
                write: fault == 1,
                seek: fault == 2,
                flush: fault == 3,
                ..Default::default()
            };
            assert!(matches!(
                Verified::capture(wire.as_slice(), store, limits(), framing(), records()),
                Err(Error::Io(_))
            ));
        }
        for bound in [
            Limits {
                chunk_bytes: 0,
                ..limits()
            },
            Limits {
                chunk_bytes: 513,
                ..limits()
            },
            Limits {
                archive_bytes: wire.len() as u64 - 1,
                ..limits()
            },
            Limits {
                store_bytes: 1,
                ..limits()
            },
        ] {
            assert!(matches!(
                Verified::capture(
                    wire.as_slice(),
                    Cursor::new(Vec::new()),
                    bound,
                    framing(),
                    records()
                ),
                Err(Error::Limit)
            ));
        }
        for end in [0, 512, wire.len() - 1] {
            assert!(Verified::capture(
                &wire[..end],
                Cursor::new(Vec::new()),
                limits(),
                framing(),
                records()
            )
            .is_err());
        }
        let mut spool = Verified::capture(
            wire.as_slice(),
            Cursor::new(Vec::new()),
            limits(),
            framing(),
            records(),
        )
        .unwrap();
        let stats = spool.stats();
        let mut reader = spool.reader(framing(), records()).unwrap();
        assert!(reader.can_publish());
        assert!(reader.receipt().is_none());
        assert_eq!(reader.next_member().unwrap().unwrap().path, "files");
        assert!(reader.next_member().unwrap().is_none());
        assert!(reader.receipt().is_some());
        drop(reader);
        assert!(Verified::capture(
            wire.as_slice(),
            Cursor::new(Vec::new()),
            Limits {
                archive_bytes: wire.len() as u64,
                store_bytes: stats.store_bytes,
                ..limits()
            },
            framing(),
            records()
        )
        .is_ok());
        assert!(matches!(
            Verified::capture(
                wire.as_slice(),
                Cursor::new(Vec::new()),
                Limits {
                    store_bytes: stats.store_bytes - 1,
                    ..limits()
                },
                framing(),
                records()
            ),
            Err(Error::Limit)
        ));
    }
    #[test]
    fn replay_failure_revokes_early_publication_admission() {
        let wire = archive();
        let mut spool = Verified::capture(
            wire.as_slice(),
            FaultStore::default(),
            limits(),
            framing(),
            records(),
        )
        .unwrap();
        // A later chunk is corrupt, after the beginning control consumed by new.
        spool.store.inner.get_mut()[2 * 544] ^= 1;
        let mut reader = spool.reader(framing(), records()).unwrap();
        assert!(reader.can_publish());
        assert!(reader.next_member().is_err());
        assert!(!reader.can_publish());
        assert!(reader.receipt().is_none());
    }
    #[test]
    fn scratch_overhead_and_read_requests_are_measured() {
        for chunk_bytes in [512, 4096] {
            let wire = archive();
            let mut spool = Verified::capture(
                wire.as_slice(),
                FaultStore::default(),
                Limits {
                    chunk_bytes,
                    ..limits()
                },
                framing(),
                records(),
            )
            .unwrap();
            let stats = spool.stats();
            let mut reader = spool.reader(framing(), records()).unwrap();
            assert!(reader.next_member().unwrap().is_some());
            assert!(reader.next_member().unwrap().is_none());
            drop(reader);
            assert_eq!(spool.store.max_read, chunk_bytes);
            assert_eq!(spool.store.write_bytes, stats.store_bytes);
            assert!(spool.levels.capacity() <= 64);
            println!("spool chunk={} archive={} scratch={} levels={} max_read={} reads={} read_bytes={} writes={} write_bytes={}", chunk_bytes, stats.archive_bytes, stats.store_bytes, stats.tree_levels, spool.store.max_read, spool.store.read_calls, spool.store.read_bytes, spool.store.write_calls, spool.store.write_bytes);
        }
    }
    #[cfg(unix)]
    #[test]
    fn temporary_regular_file_replays_without_a_value_sized_memory_store() {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "afsplus-spool-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        let wire = archive();
        let mut spool =
            Verified::capture(wire.as_slice(), file, limits(), framing(), records()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            spool.stats().store_bytes
        );
        let mut reader = spool.reader(framing(), records()).unwrap();
        assert!(reader.next_member().unwrap().is_some());
        assert!(reader.next_member().unwrap().is_none());
        assert!(reader.receipt().is_some());
        drop(reader);
        drop(spool);
        std::fs::remove_file(path).unwrap();
    }
}
