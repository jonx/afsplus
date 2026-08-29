//! In-memory sparse block device for tests and deterministic harnesses.

use std::collections::BTreeMap;

use crate::{check_access, BlockDevice, BlockError};

#[derive(Debug, Clone)]
pub struct MemoryBackend {
    block_size: usize,
    total_blocks: u64,
    blocks: BTreeMap<u64, Box<[u8]>>,
}

impl MemoryBackend {
    pub fn new(block_size: usize, total_blocks: u64) -> Self {
        assert!(block_size.is_power_of_two() && block_size >= 512);
        MemoryBackend { block_size, total_blocks, blocks: BTreeMap::new() }
    }

    /// Applies raw bytes to a block without going through the device
    /// interface. Used by the power-cut harness to build torn-write states.
    pub fn apply_raw(&mut self, lba: u64, data: &[u8]) {
        assert_eq!(data.len(), self.block_size);
        self.blocks.insert(lba, data.to_vec().into_boxed_slice());
    }

    /// Reads a block without mutable access (test helper).
    pub fn peek(&self, lba: u64) -> Vec<u8> {
        match self.blocks.get(&lba) {
            Some(data) => data.to_vec(),
            None => vec![0u8; self.block_size],
        }
    }
}

impl BlockDevice for MemoryBackend {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(lba, self.total_blocks, buf.len(), self.block_size)?;
        match self.blocks.get(&lba) {
            Some(data) => buf.copy_from_slice(data),
            None => buf.fill(0),
        }
        Ok(())
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        check_access(lba, self.total_blocks, data.len(), self.block_size)?;
        self.blocks.insert(lba, data.to_vec().into_boxed_slice());
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}
