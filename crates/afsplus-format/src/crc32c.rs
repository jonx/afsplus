//! CRC32C (Castagnoli), the initial metadata checksum proposal
//! (`docs/03-on-disk-format.md` §6). On AArch64 built with the `crc`
//! feature, as every Apple Silicon target is, the CPU's CRC32C instructions
//! take eight bytes a step. Elsewhere it is table-driven and dependency-free:
//! eight bytes a step through eight tables (slicing-by-8), then byte by byte.
//! The words are read little-endian explicitly, so a big-endian CPU computes
//! the same value; the tables take 8 KiB.
//!
//! The checksum algorithm identifier remains an explicit format field so a
//! later epoch can negotiate alternatives.

/// Checksum algorithm identifier stored on disk.
pub const CHECKSUM_CRC32C: u8 = 1;

const POLY_REFLECTED: u32 = 0x82F6_3B78;

const TABLE: [u32; 256] = build_table();

/// `TABLES[k][i]` is the CRC of byte `i` followed by `k` zero bytes, so eight
/// lookups advance the state by eight bytes at once.
const TABLES: [[u32; 256]; 8] = build_tables();

const fn build_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0u32; 256]; 8];
    tables[0] = TABLE;
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let previous = tables[k - 1][i];
            tables[k][i] = (previous >> 8) ^ TABLE[(previous & 0xFF) as usize];
            i += 1;
        }
        k += 1;
    }
    tables
}

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
        #[cfg(all(target_arch = "aarch64", target_feature = "crc"))]
        {
            self.state = afsplus_crc_hw::crc32c_update(self.state, data);
        }
        #[cfg(not(all(target_arch = "aarch64", target_feature = "crc")))]
        {
            self.state = update_tables(self.state, data);
        }
    }

    pub fn finalize(self) -> u32 {
        !self.state
    }
}

/// Eight bytes a step through eight tables, then byte by byte.
#[cfg_attr(all(target_arch = "aarch64", target_feature = "crc"), allow(dead_code))]
fn update_tables(state: u32, data: &[u8]) -> u32 {
    {
        let mut crc = state;
        let (chunks, rest) = data.as_chunks::<8>();
        for chunk in chunks {
            let low = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) ^ crc;
            let high = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            crc = TABLES[7][(low & 0xFF) as usize]
                ^ TABLES[6][((low >> 8) & 0xFF) as usize]
                ^ TABLES[5][((low >> 16) & 0xFF) as usize]
                ^ TABLES[4][(low >> 24) as usize]
                ^ TABLES[3][(high & 0xFF) as usize]
                ^ TABLES[2][((high >> 8) & 0xFF) as usize]
                ^ TABLES[1][((high >> 16) & 0xFF) as usize]
                ^ TABLES[0][(high >> 24) as usize];
        }
        for &byte in rest {
            crc = (crc >> 8) ^ TABLE[((crc ^ byte as u32) & 0xFF) as usize];
        }
        crc
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{crc32c, update_tables, Hasher, TABLE};

    /// The byte-at-a-time definition the sliced form must equal.
    fn reference(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in data {
            crc = (crc >> 8) ^ TABLE[((crc ^ byte as u32) & 0xFF) as usize];
        }
        !crc
    }

    #[test]
    fn eight_bytes_a_step_equals_one_byte_a_step() {
        let mut state = 0x9E37_79B9u32;
        let data: Vec<u8> = (0..9000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        // Every length and start around the eight-byte step, and a block.
        for start in 0..9 {
            for length in (0..70).chain([4096, 4097, 8191]) {
                let slice = &data[start..start + length];
                assert_eq!(
                    crc32c(slice),
                    reference(slice),
                    "start {start} length {length}"
                );
            }
        }
        // Spans of any size streamed through one hasher.
        for split in [1, 3, 7, 8, 9, 100] {
            let mut hasher = Hasher::new();
            for piece in data[..4096].chunks(split) {
                hasher.update(piece);
            }
            assert_eq!(hasher.finalize(), reference(&data[..4096]), "split {split}");
        }
    }

    #[test]
    fn the_tables_equal_the_definition_whichever_path_is_built() {
        let data: Vec<u8> = (0..5000u32).map(|index| (index * 31 + 7) as u8).collect();
        for start in 0..9 {
            for length in (0..40).chain([4096, 4099]) {
                let slice = &data[start..start + length];
                assert_eq!(!update_tables(!0, slice), reference(slice));
            }
        }
    }

    #[test]
    fn known_vectors() {
        // RFC 3720 / common CRC32C test vectors.
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0);
        assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
        assert_eq!(crc32c(&[0xFFu8; 32]), 0x62A8_AB43);
    }
}
