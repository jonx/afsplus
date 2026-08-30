//! CRC32C (Castagnoli), the initial metadata checksum proposal
//! (`docs/03-on-disk-format.md` §6). Table-driven, dependency-free.
//!
//! The checksum algorithm identifier remains an explicit format field so a
//! later epoch can negotiate alternatives.

/// Checksum algorithm identifier stored on disk.
pub const CHECKSUM_CRC32C: u8 = 1;

const POLY_REFLECTED: u32 = 0x82F6_3B78;

const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY_REFLECTED
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// Computes the CRC32C of `data`.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Streaming CRC32C, used to checksum a block in disjoint spans without
/// copying it (for example, everything around a zeroed checksum field).
pub struct Hasher {
    state: u32,
}

impl Hasher {
    pub fn new() -> Self {
        Hasher { state: !0 }
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.state;
        for &byte in data {
            crc = (crc >> 8) ^ TABLE[((crc ^ byte as u32) & 0xFF) as usize];
        }
        self.state = crc;
    }

    pub fn finalize(self) -> u32 {
        !self.state
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::crc32c;

    #[test]
    fn known_vectors() {
        // RFC 3720 / common CRC32C test vectors.
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0);
        assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
        assert_eq!(crc32c(&[0xFFu8; 32]), 0x62A8_AB43);
    }
}
