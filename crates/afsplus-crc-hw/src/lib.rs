//! The CRC32C instructions of AArch64, for `afsplus-format`.
//!
//! Built only where the target has them at compile time, as every Apple
//! Silicon target does: without the `crc` feature the crate is empty and the
//! format crate keeps its tables. The CRC state goes in and out unfinalised,
//! as the format crate's streaming hasher holds it.
#![no_std]

/// Advances a CRC32C state over `data`, eight bytes a step.
#[cfg(all(target_arch = "aarch64", target_feature = "crc"))]
pub fn crc32c_update(mut crc: u32, data: &[u8]) -> u32 {
    use core::arch::aarch64::{__crc32cb, __crc32cd};

    let (chunks, rest) = data.as_chunks::<8>();
    for chunk in chunks {
        // SAFETY: this function is only built for AArch64 with the `crc`
        // feature enabled, so every CPU the binary runs on has the
        // instruction.
        crc = unsafe { __crc32cd(crc, u64::from_le_bytes(*chunk)) };
    }
    for &byte in rest {
        // SAFETY: as above.
        crc = unsafe { __crc32cb(crc, byte) };
    }
    crc
}
