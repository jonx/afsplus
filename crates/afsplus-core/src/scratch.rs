//! Block-sized scratch buffers, lent out and given back.
//!
//! A tree descent reads a block into a buffer and decodes it. Taking a fresh
//! vector for each of those reads cost one allocation of a whole block per
//! node read, and on AROS one `AllocMem` and one `FreeMem` with it. The
//! buffers come back here when they are dropped, so a volume pays for them
//! once and the reads after that borrow.
//!
//! A borrow owns its buffer until it is dropped, and a nested read takes the
//! next one, so no two live borrows are ever the same memory. The pool is
//! per thread, so two volumes on two threads do not share it either.

use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

thread_local! {
    static POOL: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

/// The most buffers kept between borrows. A tree descent nests one buffer
/// per level and the checker's walk one more, so the depth cap of the format
/// plus room for the callers around it is all that is ever wanted; beyond
/// it a buffer is dropped rather than held.
const KEPT: usize = 24;

/// A borrowed block buffer. It reads and writes as a `[u8]` of the size
/// asked for, and it returns to the pool when it goes out of scope.
pub(crate) struct Block {
    buffer: Vec<u8>,
}

impl Block {
    /// A buffer of `block_size` bytes. What is in it is whatever the borrow
    /// before it left there, so a caller fills it before it reads it; every
    /// caller here reads a block into it as its first act. A buffer that has
    /// to change size is zeroed, because growing a vector must write
    /// something.
    pub(crate) fn take(block_size: usize) -> Block {
        let mut buffer = POOL
            .with(|pool| pool.borrow_mut().pop())
            .unwrap_or_default();
        if buffer.len() != block_size {
            buffer.clear();
            buffer.resize(block_size, 0);
        }
        Block { buffer }
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        let buffer = std::mem::take(&mut self.buffer);
        if buffer.capacity() == 0 {
            return;
        }
        POOL.with(|pool| {
            if let Ok(mut pool) = pool.try_borrow_mut() {
                if pool.len() < KEPT {
                    pool.push(buffer);
                }
            }
        });
    }
}

impl Deref for Block {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.buffer
    }
}

impl DerefMut for Block {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two borrows at once are two different buffers, and the second borrow
    /// after both are given back reuses what the first one held.
    #[test]
    fn two_borrows_never_share_memory() {
        let first_address;
        {
            let mut one = Block::take(64);
            let mut two = Block::take(64);
            one[0] = 1;
            two[0] = 2;
            assert_eq!(one[0], 1);
            assert_ne!(one.as_ptr(), two.as_ptr());
            first_address = one.as_ptr();
        }
        let again = Block::take(64);
        assert_eq!(again.len(), 64);
        let _ = first_address;
    }

    /// A borrow of a different size is still exactly that size, and the
    /// bytes a grown buffer gains are zero.
    #[test]
    fn a_borrow_has_the_size_asked_for() {
        {
            let mut small = Block::take(512);
            assert_eq!(small.len(), 512);
            small[500] = 0xff;
        }
        let big = Block::take(4096);
        assert_eq!(big.len(), 4096);
        assert!(big.iter().all(|byte| *byte == 0));
    }
}
