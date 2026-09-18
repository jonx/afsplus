//! The read cache against the device it fronts: what a read returns, what
//! the device receives, and what happens when the device refuses a write.

use afsplus_block::{BlockDevice, BlockError, CachedDevice, MemoryBackend};

const BLOCK: usize = 512;

fn block(value: u8) -> Vec<u8> {
    vec![value; BLOCK]
}

fn read(device: &mut impl BlockDevice, lba: u64) -> Vec<u8> {
    let mut buffer = vec![0u8; BLOCK];
    device.read_block(lba, &mut buffer).unwrap();
    buffer
}

/// A seeded sequence of reads, writes, barriers and resizes gives the cached
/// device exactly the contents of an uncached one, and the device behind the
/// cache the same writes in the same order.
#[test]
fn a_cached_device_reads_what_an_uncached_one_reads() {
    let mut state = 0x2545_f491_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let mut plain = MemoryBackend::new(BLOCK, 64);
    let mut cached = CachedDevice::new(MemoryBackend::new(BLOCK, 64), 5);
    for step in 0..20_000u32 {
        let lba = u64::from(next() % 64);
        match next() % 10 {
            0..=5 => assert_eq!(read(&mut cached, lba), read(&mut plain, lba), "step {step}"),
            6..=8 => {
                let value = block((next() & 0xff) as u8);
                plain.write_block(lba, &value).unwrap();
                cached.write_block(lba, &value).unwrap();
            }
            _ => {
                if next() % 50 == 0 {
                    assert!(cached.set_capacity((next() % 9) as usize));
                } else {
                    cached.flush().unwrap();
                }
            }
        }
        assert!(cached.cached_blocks() <= cached.capacity());
    }
    for lba in 0..64 {
        assert_eq!(
            read(cached.inner_mut(), lba),
            read(&mut plain, lba),
            "device {lba}"
        );
    }
    let stats = cached.stats();
    assert!(stats.hits > 0 && stats.misses > 0 && stats.evictions > 0);
}

#[test]
fn the_least_recently_used_block_leaves_first() {
    let mut cached = CachedDevice::new(MemoryBackend::new(BLOCK, 8), 2);
    read(&mut cached, 1);
    read(&mut cached, 2);
    read(&mut cached, 1); // 2 is now the oldest
    read(&mut cached, 3); // evicts 2
    let before = cached.stats();
    read(&mut cached, 1);
    read(&mut cached, 3);
    assert_eq!(cached.stats().hits, before.hits + 2);
    read(&mut cached, 2);
    assert_eq!(cached.stats().misses, before.misses + 1);
    assert_eq!(cached.stats().evictions, 2);
}

#[test]
fn a_write_is_cached_and_reaches_the_device_at_once() {
    let mut cached = CachedDevice::new(MemoryBackend::new(BLOCK, 8), 4);
    cached.write_block(3, &block(7)).unwrap();
    assert_eq!(read(cached.inner_mut(), 3), block(7), "write-through");
    let misses = cached.stats().misses;
    assert_eq!(read(&mut cached, 3), block(7));
    assert_eq!(cached.stats().misses, misses, "the written block is served");
}

#[test]
fn a_capacity_of_zero_passes_everything_through() {
    let mut cached = CachedDevice::new(MemoryBackend::new(BLOCK, 8), 0);
    cached.write_block(1, &block(9)).unwrap();
    assert_eq!(read(&mut cached, 1), block(9));
    assert_eq!(read(&mut cached, 1), block(9));
    assert_eq!(cached.cached_blocks(), 0);
    assert_eq!(cached.stats().hits, 0);
    assert_eq!(cached.stats().misses, 2);
}

/// A device that stores a write and then reports it failed, as a device can
/// that dies mid-request: the bytes may be there or not.
struct LandsThenFails {
    inner: MemoryBackend,
    fail_next_write: bool,
}

impl BlockDevice for LandsThenFails {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.inner.write_block(lba, data)?;
        if std::mem::take(&mut self.fail_next_write) {
            return Err(BlockError::Injected("write reported failed"));
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

#[test]
fn a_refused_write_leaves_the_block_to_the_device() {
    let device = LandsThenFails {
        inner: MemoryBackend::new(BLOCK, 8),
        fail_next_write: false,
    };
    let mut cached = CachedDevice::new(device, 4);
    cached.write_block(2, &block(1)).unwrap();
    assert_eq!(read(&mut cached, 2), block(1));
    cached.inner_mut().fail_next_write = true;
    assert!(cached.write_block(2, &block(2)).is_err());
    // Neither the old cached copy nor the refused bytes: whatever the device
    // holds, which here is the new block.
    assert_eq!(read(&mut cached, 2), block(2));
    assert_eq!(cached.stats().invalidations, 1);
    // The slot is reused.
    for lba in 3..8 {
        read(&mut cached, lba);
    }
    assert!(cached.cached_blocks() <= 4);
}

#[test]
fn a_control_resizes_the_cache_at_its_next_access_and_reads_its_counters() {
    let mut cached = CachedDevice::new(MemoryBackend::new(BLOCK, 16), 8);
    let control = cached.control();
    for lba in 0..8 {
        read(&mut cached, lba);
    }
    assert_eq!(cached.cached_blocks(), 8);
    control.request_capacity(2);
    assert_eq!(control.capacity(), 2);
    assert_eq!(cached.cached_blocks(), 8, "taken at the next access");
    read(&mut cached, 7);
    assert_eq!(cached.cached_blocks(), 2);
    assert_eq!(cached.capacity(), 2);
    // The two most recent survive: 7 was just read, 6 before it.
    let hits = control.stats().hits;
    read(&mut cached, 6);
    assert_eq!(control.stats().hits, hits + 1);
    assert_eq!(control.stats(), cached.stats());
}

#[test]
fn a_capacity_that_cannot_be_had_leaves_the_cache_as_it_was() {
    let mut cached = CachedDevice::new(MemoryBackend::new(BLOCK, 16), 4);
    for lba in 0..4 {
        read(&mut cached, lba);
    }
    assert!(!cached.set_capacity(usize::MAX / 2));
    assert_eq!(cached.capacity(), 4);
    assert_eq!(cached.cached_blocks(), 4);
    let hits = cached.stats().hits;
    read(&mut cached, 3);
    assert_eq!(cached.stats().hits, hits + 1);
    // Through the control, the same request is refused at the next access
    // and the control then reports the size in force.
    let control = cached.control();
    control.request_capacity(usize::MAX / 2);
    read(&mut cached, 3);
    assert_eq!(control.capacity(), 4);
}
