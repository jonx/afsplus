//! Fixed partition-sized views of an owned block provider (ADR-096).
use crate::{check_access, BlockDevice, BlockError};

#[derive(Debug)]
pub struct SliceBackend<D: BlockDevice> {
    inner: D,
    start: u64,
    blocks: u64,
}

impl<D: BlockDevice> SliceBackend<D> {
    /// Admit a nonempty interval without overflowing the parent's capacity.
    pub fn new(inner: D, start: u64, blocks: u64) -> Result<Self, BlockError> {
        let capacity = inner.total_blocks();
        if start >= capacity || blocks == 0 || blocks > capacity - start {
            return Err(BlockError::OutOfBounds {
                lba: start,
                total_blocks: capacity,
            });
        }
        Ok(Self {
            inner,
            start,
            blocks,
        })
    }

    pub fn into_inner(self) -> D {
        self.inner
    }
}

impl<D: BlockDevice> BlockDevice for SliceBackend<D> {
    fn block_size(&self) -> usize {
        self.inner.block_size()
    }
    fn total_blocks(&self) -> u64 {
        self.blocks
    }
    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(lba, self.blocks, buf.len(), self.block_size())?;
        self.inner.read_block(self.start + lba, buf)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        check_access(lba, self.blocks, data.len(), self.block_size())?;
        self.inner.write_block(self.start + lba, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FaultBackend, FaultPlan, MemoryBackend, TraceBackend, TraceEvent};

    #[test]
    fn nested_views_preserve_neighbors_and_translate_boundaries() {
        let mut base = MemoryBackend::new(512, 12);
        for lba in 0..12 {
            base.write_block(lba, &[lba as u8; 512]).unwrap();
        }
        let outer = SliceBackend::new(base, 2, 8).unwrap();
        let mut view = SliceBackend::new(outer, 1, 4).unwrap();
        assert_eq!((view.total_blocks(), view.block_size()), (4, 512));
        let mut data = [0; 512];
        view.read_block(0, &mut data).unwrap();
        assert_eq!(data, [3; 512]);
        view.read_block(3, &mut data).unwrap();
        assert_eq!(data, [6; 512]);
        view.write_block(0, &[99; 512]).unwrap();
        view.write_block(3, &[88; 512]).unwrap();
        view.flush().unwrap();
        let base = view.into_inner().into_inner();
        for lba in 0..12 {
            let expected = match lba {
                3 => 99,
                6 => 88,
                _ => lba as u8,
            };
            assert_eq!(base.peek(lba), vec![expected; 512]);
        }
    }

    #[test]
    fn rejected_access_never_reaches_parent() {
        let mut view =
            SliceBackend::new(TraceBackend::new(MemoryBackend::new(512, 10)), 2, 4).unwrap();
        for lba in [4, u64::MAX] {
            assert!(view.read_block(lba, &mut [0; 512]).is_err());
            assert!(view.write_block(lba, &[0; 512]).is_err());
        }
        for size in [0, 511, 513] {
            assert!(view.read_block(0, &mut vec![0; size]).is_err());
            assert!(view.write_block(0, &vec![0; size]).is_err());
        }
        view.flush().unwrap();
        assert_eq!(view.into_inner().events(), &[TraceEvent::Flush]);
        for (start, length) in [(0, 0), (10, 1), (9, 2), (1, u64::MAX), (u64::MAX, 1)] {
            assert!(SliceBackend::new(MemoryBackend::new(512, 10), start, length).is_err());
        }
        let mut last =
            SliceBackend::new(MemoryBackend::new(512, u64::MAX), u64::MAX - 1, 1).unwrap();
        last.write_block(0, &[7; 512]).unwrap();
        assert_eq!(last.into_inner().peek(u64::MAX - 1), vec![7; 512]);
    }

    #[test]
    fn parent_failure_is_not_converted_to_success() {
        let parent = FaultBackend::new(
            MemoryBackend::new(512, 10),
            FaultPlan {
                fail_write_index: Some(0),
                fail_flush_index: None,
                fail_hard: true,
            },
        );
        let mut view = SliceBackend::new(parent, 2, 4).unwrap();
        assert!(matches!(
            view.write_block(0, &[0; 512]),
            Err(BlockError::Injected("write fault"))
        ));
        assert!(matches!(
            view.read_block(0, &mut [0; 512]),
            Err(BlockError::Injected("device gone after fault"))
        ));
        assert!(view.flush().is_err());
        let parent = FaultBackend::new(
            MemoryBackend::new(512, 10),
            FaultPlan {
                fail_flush_index: Some(0),
                ..FaultPlan::default()
            },
        );
        let mut view = SliceBackend::new(parent, 0, 10).unwrap();
        assert!(matches!(
            view.flush(),
            Err(BlockError::Injected("flush fault"))
        ));
    }
}
