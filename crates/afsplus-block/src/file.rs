//! Sparse host-file block device.
//!
//! Reads beyond the current file extent return zeros, so a freshly created
//! (or sparse) image behaves exactly like [`crate::MemoryBackend`]. `flush`
//! maps to `File::sync_data`, matching the durability contract of the trait.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::{check_access, BlockDevice, BlockError};

#[derive(Debug)]
pub struct FileBackend {
    file: File,
    block_size: usize,
    total_blocks: u64,
}

impl FileBackend {
    /// Creates or truncates an image file with the given geometry.
    pub fn create(path: &Path, block_size: usize, total_blocks: u64) -> Result<Self, BlockError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(FileBackend {
            file,
            block_size,
            total_blocks,
        })
    }

    /// Opens an existing image read/write with an explicit geometry.
    ///
    /// The declared `total_blocks` may exceed the current file length: the
    /// tail of a sparse image reads as zeros.
    pub fn open(path: &Path, block_size: usize, total_blocks: u64) -> Result<Self, BlockError> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(FileBackend {
            file,
            block_size,
            total_blocks,
        })
    }

    /// Opens an existing image, deriving a provisional geometry from the file
    /// length. Callers that learn the authoritative geometry from the
    /// identification block can widen it with [`FileBackend::set_total_blocks`].
    pub fn open_sized_by_file(path: &Path, block_size: usize) -> Result<Self, BlockError> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len();
        let total_blocks = len.div_ceil(block_size as u64).max(1);
        Ok(FileBackend {
            file,
            block_size,
            total_blocks,
        })
    }

    pub fn set_total_blocks(&mut self, total_blocks: u64) {
        self.total_blocks = total_blocks;
    }
}

impl BlockDevice for FileBackend {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    fn read_block(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(lba, self.total_blocks, buf.len(), self.block_size)?;
        self.file
            .seek(SeekFrom::Start(lba * self.block_size as u64))?;
        // Short reads at EOF are zero-filled: sparse tail semantics.
        let mut filled = 0;
        while filled < buf.len() {
            let n = self.file.read(&mut buf[filled..])?;
            if n == 0 {
                buf[filled..].fill(0);
                break;
            }
            filled += n;
        }
        Ok(())
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        check_access(lba, self.total_blocks, data.len(), self.block_size)?;
        self.file
            .seek(SeekFrom::Start(lba * self.block_size as u64))?;
        self.file.write_all(data)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.file.sync_data()?;
        Ok(())
    }
}
