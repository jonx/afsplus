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

#[test]
fn a_decoded_form_lives_until_its_block_is_written_or_leaves() {
    use std::sync::Arc;
    let mut cached = CachedDevice::new(MemoryBackend::new(BLOCK, 16), 2);
    let decoded = |value: u32| -> afsplus_block::Decoded { Arc::new(value) };
    let of = |cached: &CachedDevice<MemoryBackend>, lba| {
        cached
            .attached(lba)
            .and_then(|value| value.downcast::<u32>().ok())
            .map(|value| *value)
    };
    // Nothing is kept for a block the cache does not hold.
    cached.attach(1, decoded(1));
    assert_eq!(of(&cached, 1), None);
    read(&mut cached, 1);
    cached.attach(1, decoded(11));
    assert_eq!(of(&cached, 1), Some(11));
    // A read leaves it; a write of the block drops it.
    read(&mut cached, 1);
    assert_eq!(of(&cached, 1), Some(11));
    cached.write_block(1, &block(5)).unwrap();
    assert_eq!(of(&cached, 1), None);
    // Leaving the cache drops it too, even when the block comes back.
    cached.attach(1, decoded(12));
    read(&mut cached, 2);
    read(&mut cached, 3);
    read(&mut cached, 1);
    assert_eq!(of(&cached, 1), None);
    // A refused write drops it with the bytes.
    let device = LandsThenFails {
        inner: MemoryBackend::new(BLOCK, 8),
        fail_next_write: false,
    };
    let mut cached = CachedDevice::new(device, 4);
    read(&mut cached, 4);
    cached.attach(4, decoded(4));
    cached.inner_mut().fail_next_write = true;
    assert!(cached.write_block(4, &block(9)).is_err());
    assert!(cached.attached(4).is_none());
    // A device without a cache keeps nothing.
    let mut plain = MemoryBackend::new(BLOCK, 4);
    plain.attach(0, decoded(0));
    assert!(plain.attached(0).is_none());
}

/// A device that counts the requests it is given, a run being one.
struct Requests {
    inner: MemoryBackend,
    reads: Vec<(u64, usize)>,
    writes: Vec<(u64, usize)>,
    refuse_writes: bool,
}

impl Requests {
    fn new(blocks: u64) -> Self {
        Requests {
            inner: MemoryBackend::new(BLOCK, blocks),
            reads: Vec::new(),
            writes: Vec::new(),
            refuse_writes: false,
        }
    }
}

impl BlockDevice for Requests {
    fn block_size(&self) -> usize {
        BLOCK
    }
    fn total_blocks(&self) -> u64 {
        self.inner.total_blocks()
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.reads.push((lba, 1));
        self.inner.read_block(lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.writes.push((lba, 1));
        if self.refuse_writes {
            return Err(BlockError::Injected("write"));
        }
        self.inner.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
    fn read_blocks(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.reads.push((lba, buf.len() / BLOCK));
        for (index, block) in buf.chunks_exact_mut(BLOCK).enumerate() {
            self.inner.read_block(lba + index as u64, block)?;
        }
        Ok(())
    }
    fn write_blocks(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.writes.push((lba, data.len() / BLOCK));
        for (index, block) in data.chunks_exact(BLOCK).enumerate() {
            // A refused run has landed in part: its first block is written.
            if self.refuse_writes && index == 1 {
                return Err(BlockError::Injected("write"));
            }
            self.inner.write_block(lba + index as u64, block)?;
        }
        if self.refuse_writes {
            return Err(BlockError::Injected("write"));
        }
        Ok(())
    }
}

fn read_run(device: &mut impl BlockDevice, lba: u64, blocks: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; blocks * BLOCK];
    device.read_blocks(lba, &mut buffer).unwrap();
    buffer
}

/// Single blocks and runs, read and written in any mix through both pools
/// at any size, read ahead or not, with writes the device refuses in part:
/// the cached device always answers what an uncached one answers, and the
/// device behind it holds the same bytes.
#[test]
fn runs_and_blocks_through_two_pools_read_what_the_device_holds() {
    for seed in [0x2545_f491_u32, 0x9e37_79b9, 0x1234_5678, 7] {
        let mut state = seed;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        const BLOCKS: u64 = 160;
        let mut plain = MemoryBackend::new(BLOCK, BLOCKS);
        let mut cached = CachedDevice::with_capacities(Requests::new(BLOCKS), 7, 70);
        let mut sequential = 0u64;
        for step in 0..30_000u32 {
            let lba = u64::from(next() % BLOCKS as u32);
            let run = 1 + (next() % 24) as usize;
            let run = run.min((BLOCKS - lba) as usize);
            match next() % 16 {
                0..=3 => assert_eq!(read(&mut cached, lba), read(&mut plain, lba), "step {step}"),
                4..=6 => assert_eq!(
                    read_run(&mut cached, lba, run),
                    read_run(&mut plain, lba, run),
                    "step {step}"
                ),
                // A reader that goes on where it stopped, in small runs:
                // what the read-ahead is for.
                7..=9 => {
                    let run = 1 + (next() % 5) as usize;
                    if sequential + run as u64 > BLOCKS {
                        sequential = 0;
                    }
                    assert_eq!(
                        read_run(&mut cached, sequential, run),
                        read_run(&mut plain, sequential, run),
                        "step {step}"
                    );
                    sequential += run as u64;
                }
                10..=11 => {
                    let value = block((next() & 0xff) as u8);
                    plain.write_block(lba, &value).unwrap();
                    cached.write_block(lba, &value).unwrap();
                }
                12..=13 => {
                    let value = vec![(next() & 0xff) as u8; run * BLOCK];
                    plain.write_blocks(lba, &value).unwrap();
                    cached.write_blocks(lba, &value).unwrap();
                }
                14 => {
                    // Refused: one block is not written, a run is written
                    // in its first block only. The plain device is given
                    // exactly what landed.
                    let value = vec![(next() & 0xff) as u8; run * BLOCK];
                    cached.inner_mut().refuse_writes = true;
                    if next() % 2 == 0 || run == 1 {
                        assert!(cached.write_block(lba, &value[..BLOCK]).is_err());
                    } else {
                        assert!(cached.write_blocks(lba, &value).is_err());
                        plain.write_block(lba, &value[..BLOCK]).unwrap();
                    }
                    cached.inner_mut().refuse_writes = false;
                }
                _ => {
                    if next() % 3 == 0 {
                        assert!(cached.set_capacity((next() % 9) as usize));
                    } else {
                        assert!(cached.set_data_capacity((next() % 90) as usize));
                    }
                }
            }
            assert!(cached.cached_blocks() <= cached.capacity());
            assert!(cached.cached_data_blocks() <= cached.data_capacity());
        }
        for lba in 0..BLOCKS {
            assert_eq!(
                read(&mut cached.inner_mut().inner, lba),
                read(&mut plain, lba),
                "device {lba}"
            );
            assert_eq!(read(&mut cached, lba), read(&mut plain, lba), "block {lba}");
            assert_eq!(
                read_run(&mut cached, lba, 1),
                read(&mut plain, lba),
                "run {lba}"
            );
        }
        let stats = cached.stats();
        assert!(stats.hits > 0 && stats.misses > 0 && stats.evictions > 0);
        assert!(stats.read_ahead > 0, "seed {seed:#x} never read ahead");
    }
}

/// A run is one device request for each stretch of it the cache does not
/// hold, a repeat is none, and what it reads never takes a metadata slot.
#[test]
fn a_run_is_one_request_and_leaves_the_metadata_pool_alone() {
    let mut cached = CachedDevice::with_capacities(Requests::new(256), 4, 128);
    for lba in 0..4 {
        read(&mut cached, lba);
    }
    cached.inner_mut().reads.clear();
    // 40 blocks of file data, ten times the metadata pool.
    read_run(&mut cached, 100, 40);
    assert_eq!(cached.inner().reads, [(100, 40)]);
    read_run(&mut cached, 100, 40);
    read_run(&mut cached, 110, 5);
    assert_eq!(cached.inner().reads, [(100, 40)], "a repeat asks nothing");
    // The four metadata blocks are still held.
    for lba in 0..4 {
        read(&mut cached, lba);
    }
    assert_eq!(cached.inner().reads, [(100, 40)]);
    assert_eq!(cached.cached_blocks(), 4);
    // A run that holes the cached stretch asks for the two ends only.
    read_run(&mut cached, 95, 50);
    assert_eq!(cached.inner().reads[1..], [(95, 5), (140, 5)]);
    // Written data is held too, as one request.
    cached.write_blocks(200, &vec![9u8; 8 * BLOCK]).unwrap();
    assert_eq!(cached.inner().writes, [(200, 8)]);
    assert_eq!(read_run(&mut cached, 200, 8), vec![9u8; 8 * BLOCK]);
    assert_eq!(cached.inner().reads.len(), 3);

    // Without a data pool a run is still one request, every time.
    let mut bare = CachedDevice::new(Requests::new(64), 4);
    read_run(&mut bare, 8, 16);
    read_run(&mut bare, 8, 16);
    assert_eq!(bare.inner().reads, [(8, 16), (8, 16)]);
}

/// A reader that continues where it stopped is read ahead of; one that
/// jumps is not, and neither is a run as long as the read-ahead itself.
#[test]
fn only_a_continued_short_run_is_read_ahead() {
    use afsplus_block::cache::READ_AHEAD_BLOCKS as AHEAD;
    let mut cached = CachedDevice::with_capacities(Requests::new(1024), 0, 256);
    read_run(&mut cached, 10, 2);
    assert_eq!(cached.inner().reads, [(10, 2)], "the first run is not");
    read_run(&mut cached, 12, 2);
    assert_eq!(cached.inner().reads[1], (12, 2 + AHEAD));
    // The blocks read ahead answer the calls that follow.
    for lba in (14..14 + AHEAD as u64).step_by(2) {
        read_run(&mut cached, lba, 2);
    }
    assert_eq!(cached.inner().reads.len(), 2);
    // A jump: exactly what was asked.
    read_run(&mut cached, 500, 3);
    assert_eq!(cached.inner().reads[2], (500, 3));
    // A long continued run: exactly what was asked.
    read_run(&mut cached, 503, AHEAD);
    assert_eq!(cached.inner().reads[3], (503, AHEAD));
    // The end of the device bounds it.
    read_run(&mut cached, 1018, 2);
    read_run(&mut cached, 1020, 2);
    assert_eq!(cached.inner().reads[5], (1020, 4));
    // What was read ahead is dropped by a write of it, like any block.
    read_run(&mut cached, 700, 1);
    read_run(&mut cached, 701, 1);
    cached.write_block(705, &block(0x77)).unwrap();
    assert_eq!(read_run(&mut cached, 705, 1), block(0x77));
}

/// A decoded form goes when its block is written as part of a run, also
/// when the device refuses the run.
#[test]
fn a_run_written_over_a_block_drops_its_decoded_form() {
    let decoded = |value: u32| -> afsplus_block::Decoded { std::sync::Arc::new(value) };
    for refuse in [false, true] {
        let mut cached = CachedDevice::with_capacities(Requests::new(64), 8, 8);
        read(&mut cached, 5);
        cached.attach(5, decoded(5));
        assert!(cached.decoded(5).is_some());
        cached.inner_mut().refuse_writes = refuse;
        assert_eq!(
            cached.write_blocks(4, &vec![1u8; 3 * BLOCK]).is_err(),
            refuse
        );
        cached.inner_mut().refuse_writes = false;
        assert!(cached.attached(5).is_none() && cached.decoded(5).is_none());
        let on_device = read(&mut cached.inner_mut().inner, 5);
        assert_eq!(read(&mut cached, 5), on_device);
    }
}
