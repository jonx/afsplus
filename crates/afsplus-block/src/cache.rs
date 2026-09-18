//! A bounded read cache in front of a block device.
//!
//! Every write goes to the device before the call returns and then replaces
//! the cached copy, so the cache never holds a block the device was not
//! given, and the order of writes and barriers the layer above chose is the
//! order the device sees. A write the device refused may have landed in part:
//! its block leaves the cache and the next read asks the device. Correctness
//! does not depend on the size; a capacity of zero passes every call through.
//!
//! Eviction takes the least recently used block. The index is a map from LBA
//! to a slot, and the slots form a doubly linked recency list by index, so a
//! hit, an insertion and an eviction each cost a constant number of steps.
//!
//! A [`CacheControl`] reaches a cache that sits deep inside a volume: it
//! reads the counters and asks for another capacity, which the cache takes at
//! its next device access.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

use crate::{BlockDevice, BlockError};

const NONE: u32 = u32::MAX;

/// What the cache has answered since it was created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Reads served from the cache.
    pub hits: u64,
    /// Reads that went to the device.
    pub misses: u64,
    /// Blocks dropped to make room or to shrink.
    pub evictions: u64,
    /// Blocks dropped because the device refused their write.
    pub invalidations: u64,
}

const NO_REQUEST: usize = usize::MAX;

#[derive(Default)]
struct Shared {
    requested: AtomicUsize,
    capacity: AtomicUsize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    invalidations: AtomicU64,
}

/// A handle on a [`CachedDevice`] that outlives borrowing it.
#[derive(Clone)]
pub struct CacheControl {
    shared: Arc<Shared>,
}

impl CacheControl {
    /// Asks for `capacity` blocks; the cache takes it at its next access.
    pub fn request_capacity(&self, capacity: usize) {
        self.shared
            .requested
            .store(capacity.min(NO_REQUEST - 1), Relaxed);
    }

    /// The capacity in force, or the one requested and not yet taken.
    pub fn capacity(&self) -> usize {
        match self.shared.requested.load(Relaxed) {
            NO_REQUEST => self.shared.capacity.load(Relaxed),
            requested => requested,
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.shared.hits.load(Relaxed),
            misses: self.shared.misses.load(Relaxed),
            evictions: self.shared.evictions.load(Relaxed),
            invalidations: self.shared.invalidations.load(Relaxed),
        }
    }
}

struct Slot {
    lba: u64,
    previous: u32,
    next: u32,
}

pub struct CachedDevice<D: BlockDevice> {
    inner: D,
    capacity: usize,
    index: HashMap<u64, u32>,
    slots: Vec<Slot>,
    /// Block contents, slot `i` at `i * block_size`.
    data: Vec<u8>,
    /// Slots of invalidated blocks, reused before any new slot is made.
    free: Vec<u32>,
    /// Most recently used slot, and least.
    newest: u32,
    oldest: u32,
    shared: Arc<Shared>,
}

impl<D: BlockDevice> CachedDevice<D> {
    /// A cache of at most `capacity` blocks in front of `inner`. Its memory
    /// is taken here, all of it, as a classic buffer count is: what the
    /// cache holds then never moves with how full it is.
    pub fn new(inner: D, capacity: usize) -> Self {
        let mut cache = CachedDevice {
            inner,
            capacity: 0,
            index: HashMap::new(),
            slots: Vec::new(),
            data: Vec::new(),
            free: Vec::new(),
            newest: NONE,
            oldest: NONE,
            shared: Arc::new(Shared {
                requested: AtomicUsize::new(NO_REQUEST),
                ..Shared::default()
            }),
        };
        cache.set_capacity(capacity);
        cache
    }

    /// A handle for whoever does not hold the device.
    pub fn control(&self) -> CacheControl {
        CacheControl {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Takes a capacity asked for through a [`CacheControl`].
    fn take_request(&mut self) {
        let requested = self.shared.requested.swap(NO_REQUEST, Relaxed);
        if requested != NO_REQUEST {
            // Refused for want of memory: the size in force stays, and the
            // control reports it.
            let _ = self.set_capacity(requested);
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Blocks held now.
    pub fn cached_blocks(&self) -> usize {
        self.index.len()
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

    /// Changes the capacity and takes or gives back its memory. Shrinking
    /// drops the least recently used blocks. When the memory cannot be had
    /// the cache keeps its size and contents, and says false.
    pub fn set_capacity(&mut self, capacity: usize) -> bool {
        let block_size = self.inner.block_size();
        let mut index = HashMap::new();
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
            return false;
        }
        // Keep the `capacity` most recent blocks.
        let mut kept: Vec<(u64, Vec<u8>)> = Vec::with_capacity(capacity);
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
        self.shared.evictions.fetch_add(dropped as u64, Relaxed);
        self.capacity = capacity;
        self.shared.capacity.store(capacity, Relaxed);
        self.index = index;
        self.slots = slots;
        self.data = data;
        self.free = free;
        self.newest = NONE;
        self.oldest = NONE;
        for (lba, block) in kept.into_iter().rev() {
            self.insert(lba, &block);
        }
        true
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

    fn block_range(&self, slot: u32) -> std::ops::Range<usize> {
        let block_size = self.inner.block_size();
        let at = slot as usize * block_size;
        at..at + block_size
    }

    /// Caches `block` under `lba`, replacing a cached copy, evicting the
    /// least recently used block when full.
    fn insert(&mut self, lba: u64, block: &[u8]) {
        if self.capacity == 0 {
            return;
        }
        if let Some(&slot) = self.index.get(&lba) {
            let range = self.block_range(slot);
            self.data[range].copy_from_slice(block);
            self.unlink(slot);
            self.push_newest(slot);
            return;
        }
        let slot = if let Some(slot) = self.free.pop() {
            self.slots[slot as usize].lba = lba;
            let range = self.block_range(slot);
            self.data[range].copy_from_slice(block);
            slot
        } else if self.slots.len() < self.capacity {
            let slot = self.slots.len() as u32;
            self.slots.push(Slot {
                lba,
                previous: NONE,
                next: NONE,
            });
            self.data.extend_from_slice(block);
            slot
        } else {
            let victim = self.oldest;
            self.unlink(victim);
            self.index.remove(&self.slots[victim as usize].lba);
            self.shared.evictions.fetch_add(1, Relaxed);
            self.slots[victim as usize].lba = lba;
            let range = self.block_range(victim);
            self.data[range].copy_from_slice(block);
            victim
        };
        self.index.insert(lba, slot);
        self.push_newest(slot);
    }

    /// Forgets `lba`; its slot waits in the free list.
    fn invalidate(&mut self, lba: u64) {
        if let Some(slot) = self.index.remove(&lba) {
            self.unlink(slot);
            self.free.push(slot);
            self.shared.invalidations.fetch_add(1, Relaxed);
        }
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
        if let Some(&slot) = self.index.get(&lba) {
            if buf.len() == self.inner.block_size() {
                let range = self.block_range(slot);
                buf.copy_from_slice(&self.data[range]);
                self.unlink(slot);
                self.push_newest(slot);
                self.shared.hits.fetch_add(1, Relaxed);
                return Ok(());
            }
        }
        self.inner.read_block(lba, buf)?;
        self.shared.misses.fetch_add(1, Relaxed);
        self.insert(lba, buf);
        Ok(())
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.take_request();
        match self.inner.write_block(lba, data) {
            Ok(()) => {
                self.insert(lba, data);
                Ok(())
            }
            Err(error) => {
                self.invalidate(lba);
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}
