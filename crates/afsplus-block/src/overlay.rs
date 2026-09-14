//! Experimental ADR-097 bounded memory branches over an immutable shared base.
use crate::{check_access, BlockDevice, BlockError};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Copy, Debug)]
pub struct OverlayLimits {
    pub branches: usize,
    pub entries: usize,
}

fn refused(message: &'static str) -> BlockError {
    BlockError::Io(std::io::Error::other(message))
}

struct Charge {
    counter: Arc<AtomicUsize>,
    amount: usize,
}
impl Charge {
    fn claim(counter: &Arc<AtomicUsize>, limit: usize, amount: usize) -> Result<Self, BlockError> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(amount).filter(|next| *next <= limit)
            })
            .map_err(|_| refused("overlay family resource limit"))?;
        Ok(Self {
            counter: counter.clone(),
            amount,
        })
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.amount, Ordering::AcqRel);
    }
}
struct Entry {
    lba: u64,
    bytes: Arc<Vec<u8>>,
    _charge: Charge,
}

/// Memory-only writable branches over a stable shared base.
/// Limits apply across the entire family, including retained fork entries.
pub struct OverlayBackend<D: BlockDevice> {
    base: Arc<Mutex<D>>,
    block_size: usize,
    blocks: u64,
    entries: Vec<Entry>,
    entry_count: Arc<AtomicUsize>,
    branch_count: Arc<AtomicUsize>,
    limits: OverlayLimits,
    _branch: Charge,
}
impl<D: BlockDevice> OverlayBackend<D> {
    pub fn new(base: D, limits: OverlayLimits) -> Result<Self, BlockError> {
        let branch_count = Arc::new(AtomicUsize::new(0));
        let branch = Charge::claim(&branch_count, limits.branches, 1)?;
        Ok(Self {
            block_size: base.block_size(),
            blocks: base.total_blocks(),
            base: Arc::new(Mutex::new(base)),
            entries: Vec::new(),
            entry_count: Arc::new(AtomicUsize::new(0)),
            branch_count,
            limits,
            _branch: branch,
        })
    }

    pub fn fork(&self) -> Result<Self, BlockError> {
        let branch = Charge::claim(&self.branch_count, self.limits.branches, 1)?;
        // Admit the complete copied index before allocating it. The remaining
        // reservation rolls back automatically if index allocation fails.
        let mut reserved =
            Charge::claim(&self.entry_count, self.limits.entries, self.entries.len())?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(self.entries.len())
            .map_err(|_| refused("overlay index allocation"))?;
        for entry in &self.entries {
            reserved.amount -= 1;
            let charge = Charge {
                counter: self.entry_count.clone(),
                amount: 1,
            };
            entries.push(Entry {
                lba: entry.lba,
                bytes: entry.bytes.clone(),
                _charge: charge,
            });
        }
        Ok(Self {
            base: self.base.clone(),
            block_size: self.block_size,
            blocks: self.blocks,
            entries,
            entry_count: self.entry_count.clone(),
            branch_count: self.branch_count.clone(),
            limits: self.limits,
            _branch: branch,
        })
    }

    pub fn live_entries(&self) -> usize {
        self.entry_count.load(Ordering::Acquire)
    }
    pub fn live_branches(&self) -> usize {
        self.branch_count.load(Ordering::Acquire)
    }
}
impl<D: BlockDevice> BlockDevice for OverlayBackend<D> {
    fn block_size(&self) -> usize {
        self.block_size
    }
    fn total_blocks(&self) -> u64 {
        self.blocks
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(lba, self.blocks, buf.len(), self.block_size)?;
        if let Ok(index) = self.entries.binary_search_by_key(&lba, |entry| entry.lba) {
            buf.copy_from_slice(&self.entries[index].bytes);
            Ok(())
        } else {
            self.base
                .lock()
                .map_err(|_| refused("overlay base lock poisoned"))?
                .read_block(lba, buf)
        }
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        check_access(lba, self.blocks, data.len(), self.block_size)?;
        let position = self.entries.binary_search_by_key(&lba, |entry| entry.lba);
        let charge = if position.is_err() {
            let charge = Charge::claim(&self.entry_count, self.limits.entries, 1)?;
            self.entries
                .try_reserve_exact(1)
                .map_err(|_| refused("overlay index allocation"))?;
            Some(charge)
        } else {
            None
        };
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(data.len())
            .map_err(|_| refused("overlay block allocation"))?;
        bytes.extend_from_slice(data);
        let bytes = Arc::new(bytes);
        match position {
            Ok(index) => self.entries[index].bytes = bytes,
            Err(index) => self.entries.insert(
                index,
                Entry {
                    lba,
                    bytes,
                    _charge: charge.expect("new entry has admission charge"),
                },
            ),
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        // Like MemoryBackend, this is process-lifetime simulation storage.
        // The base is immutable and must never receive a flush or write.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryBackend;

    #[test]
    fn forks_share_payloads_but_diverging_writes_are_isolated() {
        let mut base = MemoryBackend::new(512, 1 << 30);
        base.write_block(100, &[7; 512]).unwrap();
        let mut parent = OverlayBackend::new(
            base,
            OverlayLimits {
                branches: 3,
                entries: 4,
            },
        )
        .unwrap();
        parent.write_block(100, &[8; 512]).unwrap();
        let mut child = parent.fork().unwrap();
        assert!(Arc::ptr_eq(
            &parent.entries[0].bytes,
            &child.entries[0].bytes
        ));
        assert_eq!((child.live_entries(), child.live_branches()), (2, 2));
        child.write_block(100, &[0; 512]).unwrap();
        parent.write_block(200, &[9; 512]).unwrap();
        let mut out = [255; 512];
        parent.read_block(100, &mut out).unwrap();
        assert_eq!(out, [8; 512]);
        child.read_block(100, &mut out).unwrap();
        assert_eq!(out, [0; 512]);
        child.read_block(200, &mut out).unwrap();
        assert_eq!(out, [0; 512]);
        assert_eq!(parent.base.lock().unwrap().peek(100), vec![7; 512]);
        assert_eq!(parent.base.lock().unwrap().peek(200), vec![0; 512]);
        assert_eq!(parent.entries.len() + child.entries.len(), 3);
    }

    #[test]
    fn family_limit_rolls_back_partial_forks_and_releases_dropped_entries() {
        let mut parent = OverlayBackend::new(
            MemoryBackend::new(512, 10),
            OverlayLimits {
                branches: 2,
                entries: 3,
            },
        )
        .unwrap();
        parent.write_block(0, &[1; 512]).unwrap();
        parent.write_block(1, &[2; 512]).unwrap();
        // Only one entry slot remains; a two-entry fork must refuse before allocation.
        assert!(parent.fork().is_err());
        assert_eq!((parent.live_entries(), parent.live_branches()), (2, 1));
        parent.write_block(2, &[3; 512]).unwrap();
        assert!(parent.write_block(3, &[4; 512]).is_err());
        parent.write_block(0, &[9; 512]).unwrap();
        let mut out = [0; 512];
        parent.read_block(0, &mut out).unwrap();
        assert_eq!(out, [9; 512]);
        parent.read_block(3, &mut out).unwrap();
        assert_eq!(out, [0; 512]);

        let mut parent = OverlayBackend::new(
            MemoryBackend::new(512, 10),
            OverlayLimits {
                branches: 2,
                entries: 2,
            },
        )
        .unwrap();
        parent.write_block(0, &[1; 512]).unwrap();
        let child = parent.fork().unwrap();
        assert!(child.fork().is_err());
        assert!(parent.write_block(1, &[2; 512]).is_err());
        drop(child);
        assert_eq!((parent.live_entries(), parent.live_branches()), (1, 1));
        parent.write_block(1, &[2; 512]).unwrap();
    }

    #[test]
    fn fork_storage_scales_with_edits_and_shares_base_and_payloads() {
        let mut measurements = Vec::new();
        for blocks in [1024, 1 << 30] {
            let mut parent = OverlayBackend::new(
                MemoryBackend::new(512, blocks),
                OverlayLimits {
                    branches: 2,
                    entries: 4,
                },
            )
            .unwrap();
            parent.write_block(7, &[42; 512]).unwrap();
            let child = parent.fork().unwrap();
            assert!(Arc::ptr_eq(&parent.base, &child.base));
            assert!(Arc::ptr_eq(
                &parent.entries[0].bytes,
                &child.entries[0].bytes
            ));
            let copied_index_bytes = child.entries.capacity() * std::mem::size_of::<Entry>();
            let shared_payload_bytes = child.entries[0].bytes.capacity();
            assert_eq!(child.entries.len(), 1);
            assert_eq!(child.entries[0].bytes.len(), 512);
            eprintln!("logical_blocks={blocks} copied_index_bytes={copied_index_bytes} shared_payload_bytes={shared_payload_bytes} copied_payload_bytes=0");
            measurements.push((copied_index_bytes, shared_payload_bytes));
        }
        assert_eq!(measurements[0], measurements[1]);
    }

    struct ReadOnlyProbe {
        reads: usize,
    }
    impl BlockDevice for ReadOnlyProbe {
        fn block_size(&self) -> usize {
            512
        }
        fn total_blocks(&self) -> u64 {
            2
        }
        fn read_block(&mut self, _: u64, _: &mut [u8]) -> Result<(), BlockError> {
            self.reads += 1;
            Err(BlockError::Injected("base read"))
        }
        fn write_block(&mut self, _: u64, _: &[u8]) -> Result<(), BlockError> {
            panic!("base write")
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            panic!("base flush")
        }
    }

    #[test]
    fn admission_precedes_base_io_and_flush_never_mutates_base() {
        let mut view = OverlayBackend::new(
            ReadOnlyProbe { reads: 0 },
            OverlayLimits {
                branches: 1,
                entries: 1,
            },
        )
        .unwrap();
        assert!(view.read_block(2, &mut [0; 512]).is_err());
        assert!(view.read_block(0, &mut [0; 511]).is_err());
        assert!(view.write_block(2, &[0; 512]).is_err());
        assert!(view.write_block(0, &[0; 513]).is_err());
        assert_eq!(view.base.lock().unwrap().reads, 0);
        assert!(matches!(
            view.read_block(0, &mut [0; 512]),
            Err(BlockError::Injected("base read"))
        ));
        assert_eq!(view.live_entries(), 0);
        view.write_block(0, &[42; 512]).unwrap();
        let mut out = [0; 512];
        view.read_block(0, &mut out).unwrap();
        assert_eq!(out, [42; 512]);
        view.flush().unwrap();
        assert_eq!(view.base.lock().unwrap().reads, 1);
    }
}
