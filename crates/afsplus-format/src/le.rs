//! Explicit little-endian load/store helpers (`docs/03-on-disk-format.md` §1).
//!
//! All slices must be exactly the size of the integer; callers slice the
//! buffer explicitly so every field access names its offset and width.

pub fn get_u16(buf: &[u8]) -> u16 {
    u16::from_le_bytes(buf.try_into().expect("get_u16 needs exactly 2 bytes"))
}

pub fn get_u32(buf: &[u8]) -> u32 {
    u32::from_le_bytes(buf.try_into().expect("get_u32 needs exactly 4 bytes"))
}

pub fn get_u64(buf: &[u8]) -> u64 {
    u64::from_le_bytes(buf.try_into().expect("get_u64 needs exactly 8 bytes"))
}

pub fn get_i64(buf: &[u8]) -> i64 {
    i64::from_le_bytes(buf.try_into().expect("get_i64 needs exactly 8 bytes"))
}

pub fn put_u16(buf: &mut [u8], value: u16) {
    buf.copy_from_slice(&value.to_le_bytes());
}

pub fn put_u32(buf: &mut [u8], value: u32) {
    buf.copy_from_slice(&value.to_le_bytes());
}

pub fn put_u64(buf: &mut [u8], value: u64) {
    buf.copy_from_slice(&value.to_le_bytes());
}

pub fn put_i64(buf: &mut [u8], value: i64) {
    buf.copy_from_slice(&value.to_le_bytes());
}
