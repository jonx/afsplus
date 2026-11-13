//! A bounded read cache in front of a block device.
//!
//! Every write goes to the device before the call returns and then replaces
//! the cached copy, so the cache never holds a block the device was not
//! given, and the order of writes and barriers the layer above chose is the
//! order the device sees. A write the device refused may have landed in part:
//! its block leaves the cache and the next read asks the device. Correctness
//! does not depend on the size; a capacity of zero passes every call through.
//!
//! There are two pools, each with its own capacity. Blocks read one at a
//! time ([`BlockDevice::read_block`]) are metadata and live in the first.
//! Runs of blocks ([`BlockDevice::read_blocks`]) are file data and live in
//! the second, so that reading a large file cannot push the trees and
//! records that every operation needs out of memory. A write through either
//! call updates its own pool and drops the block from the other, so neither
//! pool ever holds bytes the device does not.
//!
//! A run is asked of the device in one request for each stretch of it the
//! data pool does not hold, straight into the caller's buffer. A run that
//! continues where the previous one ended is read a little further than
//! asked ([`READ_AHEAD_BLOCKS`]), into the pool, so that a program reading
//! a file in small pieces makes one device request for many of its calls.
//! What is read ahead is ordinary cached content: the bytes of device
//! blocks, replaced or dropped by any write of them.
//!
//! Eviction takes the least recently used block of a pool. The index is a
//! map from LBA to a slot, and the slots form a doubly linked recency list
//! by index, so a hit, an insertion and an eviction each cost a constant
//! number of steps.
//!
//! A slot of the metadata pool may also carry the decoded form of its block
//! ([`BlockDevice::attach`]); it goes whenever the slot's bytes change or
//! the slot is reused, so it is never older than the bytes beside it.
//!
//! A [`CacheControl`] reaches a cache that sits deep inside a volume: it
//! reads the counters and asks for another capacity, which the cache takes at
//! its next device access.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

use crate::{check_run, BlockDevice, BlockError, Decoded};

const NONE: u32 = u32::MAX;

/// Blocks read beyond a run that continues the previous one. Sixteen 4 KiB
/// blocks are 64 KiB: enough to turn sixteen sequential 4 KiB reads into one
/// device request, small enough to cost little when the guess is wrong.
pub const READ_AHEAD_BLOCKS: usize = 16;

/// Hash of a block address: one multiplication, folded so that the low bits
/// the table indexes with depend on every bit of the address. The index is
/// asked on every cached access, where the general-purpose keyed hash cost
/// as much as the rest of a hit; the keys are block addresses the volume
/// chose, not input an attacker shapes.
#[derive(Default)]
struct LbaHasher(u64);

impl Hasher for LbaHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        let mixed = (self.0 ^ value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = mixed ^ (mixed >> 32);
    }
}

type Index = HashMap<u64, u32, BuildHasherDefault<LbaHasher>>;

/// What the cache has answered since it was created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Blocks served from the cache, of either pool.
    pub hits: u64,
    /// Blocks that went to the device.
    pub misses: u64,
    /// Blocks dropped to make room or to shrink.
    pub evictions: u64,
    /// Blocks dropped because the device refused their write.
    pub invalidations: u64,
    /// Uses of a cached block through its decoded form, which read no bytes.
    pub decoded_hits: u64,
    /// Blocks the device was asked for beyond a run, ahead of its reader.
    pub read_ahead: u64,
}

const NO_REQUEST: usize = usize::MAX;

#[derive(Default)]
struct Shared {
    requested: AtomicUsize,
    requested_data: AtomicUsize,
    capacity: AtomicUsize,
    data_capacity: AtomicUsize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    invalidations: AtomicU64,
    decoded_hits: AtomicU64,
    read_ahead: AtomicU64,
}

/// A handle on a [`CachedDevice`] that outlives borrowing it.
#[derive(Clone)]
pub struct CacheControl {
    shared: Arc<Shared>,
}

impl CacheControl {
    /// Asks for `capacity` metadata blocks; the cache takes it at its next
    /// access. The data pool keeps its size.
    pub fn request_capacity(&self, capacity: usize) {
        self.shared
            .requested
            .store(capacity.min(NO_REQUEST - 1), Relaxed);
    }

    /// Asks for `metadata` blocks of metadata and `data` blocks of file
    /// data; the cache takes both at its next access.
    pub fn request_capacities(&self, metadata: usize, data: usize) {
        self.request_capacity(metadata);
        self.shared
            .requested_data
            .store(data.min(NO_REQUEST - 1), Relaxed);
    }

    /// The metadata capacity in force, or the one requested and not yet
    /// taken.
    pub fn capacity(&self) -> usize {
        match self.shared.requested.load(Relaxed) {
            NO_REQUEST => self.shared.capacity.load(Relaxed),
            requested => requested,
        }
    }

    /// The file-data capacity in force, or the one requested and not yet
    /// taken.
    pub fn data_capacity(&self) -> usize {
        match self.shared.requested_data.load(Relaxed) {
            NO_REQUEST => self.shared.data_capacity.load(Relaxed),
            requested => requested,
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.shared.hits.load(Relaxed),
            misses: self.shared.misses.load(Relaxed),
            evictions: self.shared.evictions.load(Relaxed),
            invalidations: self.shared.invalidations.load(Relaxed),
            decoded_hits: self.shared.decoded_hits.load(Relaxed),
            read_ahead: self.shared.read_ahead.load(Relaxed),
        }
    }
}

struct Slot {
    lba: u64,
    previous: u32,
    next: u32,
    decoded: Option<Decoded>,
}

/// One bounded set of cached blocks, least recently used first to leave.
struct Pool {
    block_size: usize,
    capacity: usize,
    index: Index,
    slots: Vec<Slot>,
    /// Block contents, slot `i` at `i * block_size`.
    data: Vec<u8>,
    /// Slots of invalidated blocks, reused before any new slot is made.
    free: Vec<u32>,
    /// Most recently used slot, and least.
    newest: u32,
    oldest: u32,
}

impl Pool {
    fn new(block_size: usize) -> Self {
        Pool {
            block_size,
            capacity: 0,
            index: Index::default(),
            slots: Vec::new(),
            data: Vec::new(),
            free: Vec::new(),
            newest: NONE,
            oldest: NONE,
        }
    }

    /// Changes the capacity and takes or gives back its memory. Shrinking
    /// drops the least recently used blocks, and says how many. When the
    /// memory cannot be had the pool keeps its size and contents, and says
    /// `None`.
    fn set_capacity(&mut self, capacity: usize) -> Option<usize> {
        let block_size = self.block_size;
        let mut index = Index::default();
        let mut slots = Vec::new();
        let mut data = Vec::new();
        let mut free = Vec::new();
        if index.try_reserve(capacity).is_err()
            || slots.try_reserve_exact(capacity).is_err()
            || data
                .try_reserve_exact(capacity.saturating_mul(block_size))
                .is_err()
            || free.try_reserve_exact(capacity).is_err()
        {
            return None;
        }
        // Keep the `capacity` most recent blocks.
        let mut kept: Vec<(u64, Vec<u8>)> = Vec::with_capacity(capacity.min(self.index.len()));
        let mut slot = self.newest;
        while slot != NONE && kept.len() < capacity {
            let at = slot as usize * block_size;
            kept.push((
                self.slots[slot as usize].lba,
                self.data[at..at + block_size].to_vec(),
            ));
            slot = self.slots[slot as usize].next;
        }
        let dropped = self.index.len() - kept.len();
        self.capacity = capacity;
        self.index = index;
        self.slots = slots;
        self.data = data;
        self.free = free;
        self.newest = NONE;
        self.oldest = NONE;
        for (lba, block) in kept.into_iter().rev() {
            self.insert(lba, &block);
        }
        Some(dropped)
    }

    fn unlink(&mut self, slot: u32) {
        let (previous, next) = {
            let entry = &self.slots[slot as usize];
            (entry.previous, entry.next)
        };
        if previous == NONE {
            self.newest = next;
        } else {
            self.slots[previous as usize].next = next;
        }
        if next == NONE {
            self.oldest = previous;
        } else {
            self.slots[next as usize].previous = previous;
        }
    }

    fn push_newest(&mut self, slot: u32) {
        self.slots[slot as usize].previous = NONE;
        self.slots[slot as usize].next = self.newest;
        if self.newest != NONE {
            self.slots[self.newest as usize].previous = slot;
        }
        self.newest = slot;
        if self.oldest == NONE {
            self.oldest = slot;
        }
    }

    /// Counts a use of `slot`.
    fn touch(&mut self, slot: u32) {
        if self.newest != slot {
            self.unlink(slot);
            self.push_newest(slot);
        }
    }

    fn block_range(&self, slot: u32) -> std::ops::Range<usize> {
        let at = slot as usize * self.block_size;
        at..at + self.block_size
    }

    /// The bytes of `lba` when the pool holds it, counted as a use.
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> bool {
        let Some(&slot) = self.index.get(&lba) else {
            return false;
        };
        let range = self.block_range(slot);
        buf.copy_from_slice(&self.data[range]);
        self.touch(slot);
        true
    }

    /// Caches `block` under `lba`, replacing a cached copy, evicting the
    /// least recently used block when full. Says whether a block left.
    fn insert(&mut self, lba: u64, block: &[u8]) -> bool {
        if self.capacity == 0 {
            return false;
        }
        if let Some(&slot) = self.index.get(&lba) {
            let range = self.block_range(slot);
            self.data[range].copy_from_slice(block);
            self.slots[slot as usize].decoded = None;
            self.touch(slot);
            return false;
        }
        let mut evicted = false;
        let slot = if let Some(slot) = self.free.pop() {
            self.slots[slot as usize].lba = lba;
            self.slots[slot as usize].decoded = None;
            let range = self.block_range(slot);
            self.data[range].copy_from_slice(block);
            slot
        } else if self.slots.len() < self.capacity {
            let slot = self.slots.len() as u32;
            self.slots.push(Slot {
                lba,
                previous: NONE,
                next: NONE,
                decoded: None,
            });
            self.data.extend_from_slice(block);
            slot
        } else {
            let victim = self.oldest;
            self.unlink(victim);
            self.index.remove(&self.slots[victim as usize].lba);
            evicted = true;
            self.slots[victim as usize].lba = lba;
            self.slots[victim as usize].decoded = None;
            let range = self.block_range(victim);
            self.data[range].copy_from_slice(block);
            victim
        };
        self.index.insert(lba, slot);
        self.push_newest(slot);
        evicted
    }

    /// Forgets `lba`; its slot waits in the free list. Says whether the
    /// pool held it.
    fn invalidate(&mut self, lba: u64) -> bool {
        let Some(slot) = self.index.remove(&lba) else {
            return false;
        };
        self.slots[slot as usize].decoded = None;
        self.unlink(slot);
        self.free.push(slot);
        true
    }
}

pub struct CachedDevice<D: BlockDevice> {
    inner: D,
    /// Blocks read one at a time: trees, records, chains.
    metadata: Pool,
    /// Blocks read as runs: file data.
    file_data: Pool,
    /// The block after the run read last, which a sequential reader asks
    /// for next.
    next_run: u64,
    /// Where a run read ahead lands before it is shared out.
    ahead: Vec<u8>,
    shared: Arc<Shared>,
}

impl<D: BlockDevice> CachedDevice<D> {
    /// A cache of at most `capacity` metadata blocks in front of `inner`,
    /// and no file data. Its memory is taken here, all of it, as a classic
    /// buffer count is: what the cache holds then never moves with how
    /// full it is.
    pub fn new(inner: D, capacity: usize) -> Self {
        Self::with_capacities(inner, capacity, 0)
    }

    /// A cache of at most `metadata` blocks read one at a time and `data`
    /// blocks read as runs.
    pub fn with_capacities(inner: D, metadata: usize, data: usize) -> Self {
        let block_size = inner.block_size();
        let mut cache = CachedDevice {
            inner,
            metadata: Pool::new(block_size),
            file_data: Pool::new(block_size),
            next_run: u64::MAX,
            ahead: Vec::new(),
            shared: Arc::new(Shared {
                requested: AtomicUsize::new(NO_REQUEST),
                requested_data: AtomicUsize::new(NO_REQUEST),
                ..Shared::default()
            }),
        };
        cache.set_capacity(metadata);
        cache.set_data_capacity(data);
        cache
    }

    /// A handle for whoever does not hold the device.
    pub fn control(&self) -> CacheControl {
        CacheControl {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Takes the capacities asked for through a [`CacheControl`].
    fn take_request(&mut self) {
        let requested = self.shared.requested.swap(NO_REQUEST, Relaxed);
        if requested != NO_REQUEST {
            // Refused for want of memory: the size in force stays, and the
            // control reports it.
            let _ = self.set_capacity(requested);
        }
        let requested = self.shared.requested_data.swap(NO_REQUEST, Relaxed);
        if requested != NO_REQUEST {
            let _ = self.set_data_capacity(requested);
        }
    }

    /// Metadata blocks the cache may hold.
    pub fn capacity(&self) -> usize {
        self.metadata.capacity
    }

    /// File-data blocks the cache may hold.
    pub fn data_capacity(&self) -> usize {
        self.file_data.capacity
    }

    /// Metadata blocks held now.
    pub fn cached_blocks(&self) -> usize {
        self.metadata.index.len()
    }

    /// File-data blocks held now.
    pub fn cached_data_blocks(&self) -> usize {
        self.file_data.index.len()
    }

    pub fn stats(&self) -> CacheStats {
        self.control().stats()
    }

    pub fn inner(&self) -> &D {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut D {
        &mut self.inner
    }

    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Changes the metadata capacity and takes or gives back its memory.
    /// Shrinking drops the least recently used blocks. When the memory
    /// cannot be had the cache keeps its size and contents, and says false.
    pub fn set_capacity(&mut self, capacity: usize) -> bool {
        let Some(dropped) = self.metadata.set_capacity(capacity) else {
            return false;
        };
        self.shared.evictions.fetch_add(dropped as u64, Relaxed);
        self.shared.capacity.store(capacity, Relaxed);
        true
    }

    /// [`Self::set_capacity`] for the file-data pool.
    pub fn set_data_capacity(&mut self, capacity: usize) -> bool {
        let Some(dropped) = self.file_data.set_capacity(capacity) else {
            return false;
        };
        self.shared.evictions.fetch_add(dropped as u64, Relaxed);
        self.shared.data_capacity.store(capacity, Relaxed);
        true
    }

    /// Reads the device blocks `lba..lba + buf.len() / block_size` into
    /// `buf` in one request and keeps them in the data pool.
    fn fill_run(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let block_size = self.inner.block_size();
        self.inner.read_blocks(lba, buf)?;
        let blocks = buf.len() / block_size;
        self.shared.misses.fetch_add(blocks as u64, Relaxed);
        for (index, block) in buf.chunks_exact(block_size).enumerate() {
            if self.file_data.insert(lba + index as u64, block) {
                self.shared.evictions.fetch_add(1, Relaxed);
            }
        }
        Ok(())
    }

    /// How many blocks to read past a missing stretch that ends the run
    /// `lba..end`, of `blocks` blocks: none unless the run continues the
    /// one before it, is shorter than the read-ahead itself, and the pool
    /// has room to spare for the guess.
    fn read_ahead_after(&self, lba: u64, blocks: usize, end: u64) -> usize {
        if lba != self.next_run
            || blocks >= READ_AHEAD_BLOCKS
            || self.file_data.capacity < 4 * READ_AHEAD_BLOCKS
        {
            return 0;
        }
        let mut ahead = 0;
        while ahead < READ_AHEAD_BLOCKS
            && end + (ahead as u64) < self.inner.total_blocks()
            && !self.file_data.index.contains_key(&(end + ahead as u64))
        {
            ahead += 1;
        }
        ahead
    }
}

impl<D: BlockDevice> BlockDevice for CachedDevice<D> {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }

    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.take_request();
        if buf.len() == self.inner.block_size() && self.metadata.read(lba, buf) {
            self.shared.hits.fetch_add(1, Relaxed);
            return Ok(());
        }
        self.inner.read_block(lba, buf)?;
        self.shared.misses.fetch_add(1, Relaxed);
        if self.metadata.insert(lba, buf) {
            self.shared.evictions.fetch_add(1, Relaxed);
        }
        Ok(())
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.take_request();
        // Whatever the device answers, the data pool's copy is no longer
        // the block's bytes.
        self.file_data.invalidate(lba);
        match self.inner.write_block(lba, data) {
            Ok(()) => {
                if self.metadata.insert(lba, data) {
                    self.shared.evictions.fetch_add(1, Relaxed);
                }
                Ok(())
            }
            Err(error) => {
                if self.metadata.invalidate(lba) {
                    self.shared.invalidations.fetch_add(1, Relaxed);
                }
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }

    fn read_blocks(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.take_request();
        let block_size = self.inner.block_size();
        check_run(buf.len(), block_size)?;
        let blocks = buf.len() / block_size;
        if self.file_data.capacity == 0 {
            // No pool: the run is the device's, whole.
            if blocks != 0 {
                self.inner.read_blocks(lba, buf)?;
                self.shared.misses.fetch_add(blocks as u64, Relaxed);
            }
            return Ok(());
        }
        let mut at = 0;
        while at < blocks {
            let range = at * block_size..(at + 1) * block_size;
            if self.file_data.read(lba + at as u64, &mut buf[range]) {
                self.shared.hits.fetch_add(1, Relaxed);
                at += 1;
                continue;
            }
            // The stretch the pool does not hold, in one request.
            let mut end = at + 1;
            while end < blocks && !self.file_data.index.contains_key(&(lba + end as u64)) {
                end += 1;
            }
            let ahead = if end == blocks {
                self.read_ahead_after(lba, blocks, lba + blocks as u64)
            } else {
                0
            };
            if ahead == 0 {
                self.fill_run(lba + at as u64, &mut buf[at * block_size..end * block_size])?;
            } else {
                // Further than asked, so not into the caller's buffer.
                let wanted = (end - at) * block_size;
                let mut scratch = std::mem::take(&mut self.ahead);
                scratch.resize(wanted + ahead * block_size, 0);
                let result = self.fill_run(lba + at as u64, &mut scratch);
                if result.is_ok() {
                    buf[at * block_size..end * block_size].copy_from_slice(&scratch[..wanted]);
                    self.shared.read_ahead.fetch_add(ahead as u64, Relaxed);
                }
                self.ahead = scratch;
                result?;
            }
            at = end;
        }
        self.next_run = lba + blocks as u64;
        Ok(())
    }

    fn write_blocks(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.take_request();
        let block_size = self.inner.block_size();
        check_run(data.len(), block_size)?;
        let blocks = data.len() / block_size;
        // The metadata pool's copies, and what was decoded from them, are
        // no longer these blocks' bytes whatever the device answers.
        for index in 0..blocks as u64 {
            self.metadata.invalidate(lba + index);
        }
        match self.inner.write_blocks(lba, data) {
            Ok(()) => {
                for (index, block) in data.chunks_exact(block_size).enumerate() {
                    if self.file_data.insert(lba + index as u64, block) {
                        self.shared.evictions.fetch_add(1, Relaxed);
                    }
                }
                Ok(())
            }
            Err(error) => {
                // Any part of the run may have landed.
                for index in 0..blocks as u64 {
                    if self.file_data.invalidate(lba + index) {
                        self.shared.invalidations.fetch_add(1, Relaxed);
                    }
                }
                Err(error)
            }
        }
    }

    fn attached(&self, lba: u64) -> Option<Decoded> {
        let slot = *self.metadata.index.get(&lba)?;
        self.metadata.slots[slot as usize].decoded.clone()
    }

    fn decoded_capacity(&self) -> usize {
        self.metadata.capacity
    }

    fn decoded(&mut self, lba: u64) -> Option<Decoded> {
        self.take_request();
        let slot = *self.metadata.index.get(&lba)?;
        let decoded = self.metadata.slots[slot as usize].decoded.clone()?;
        self.metadata.touch(slot);
        self.shared.decoded_hits.fetch_add(1, Relaxed);
        Some(decoded)
    }

    fn attach(&mut self, lba: u64, value: Decoded) {
        if let Some(&slot) = self.metadata.index.get(&lba) {
            self.metadata.slots[slot as usize].decoded = Some(value);
        }
    }
}
