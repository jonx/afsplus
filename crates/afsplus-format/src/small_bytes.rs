//! A byte string that keeps a short value inside itself.
//!
//! Tree keys and the values of internal nodes are small: a comparison key is
//! a handful of bytes, an object-map key is eight and a child reference is
//! sixteen. Decoding a node into items that each own a `Vec<u8>` key and a
//! `Vec<u8>` value therefore cost two heap allocations per item, for every
//! item of every node an operation touched, where a lookup wants one item
//! and a descent one separator. Measured on the host that was 78 % of the
//! allocations of a create and 81 % of a delete's.
//!
//! [`SmallBytes`] holds up to [`INLINE`] bytes in its own storage and falls
//! back to a vector above that. It is memory, not format: what reaches the
//! disk is unchanged, and a value is the same bytes either way.

use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use core::ops::{Deref, DerefMut};

/// Bytes kept without a heap block. Twenty-two covers every key and child
/// reference the trees use and leaves the type the size of two words plus a
/// vector, so a tree item grows from 48 bytes to 64 rather than doubling.
pub const INLINE: usize = 22;

#[derive(Clone)]
pub enum SmallBytes {
    Inline { len: u8, bytes: [u8; INLINE] },
    Heap(Vec<u8>),
}

impl SmallBytes {
    pub const fn new() -> Self {
        SmallBytes::Inline {
            len: 0,
            bytes: [0; INLINE],
        }
    }

    pub fn from_slice(source: &[u8]) -> Self {
        if source.len() <= INLINE {
            let mut bytes = [0u8; INLINE];
            bytes[..source.len()].copy_from_slice(source);
            SmallBytes::Inline {
                len: source.len() as u8,
                bytes,
            }
        } else {
            SmallBytes::Heap(source.to_vec())
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            SmallBytes::Inline { len, bytes } => &bytes[..*len as usize],
            SmallBytes::Heap(vector) => vector.as_slice(),
        }
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }

    /// The bytes as a vector, without copying them when they are already in
    /// one.
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            SmallBytes::Inline { len, bytes } => bytes[..len as usize].to_vec(),
            SmallBytes::Heap(vector) => vector,
        }
    }
}

impl Default for SmallBytes {
    fn default() -> Self {
        SmallBytes::new()
    }
}

impl Deref for SmallBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl DerefMut for SmallBytes {
    fn deref_mut(&mut self) -> &mut [u8] {
        match self {
            SmallBytes::Inline { len, bytes } => &mut bytes[..*len as usize],
            SmallBytes::Heap(vector) => vector.as_mut_slice(),
        }
    }
}

impl AsRef<[u8]> for SmallBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<Vec<u8>> for SmallBytes {
    fn from(source: Vec<u8>) -> Self {
        if source.len() <= INLINE {
            SmallBytes::from_slice(&source)
        } else {
            SmallBytes::Heap(source)
        }
    }
}

impl From<&[u8]> for SmallBytes {
    fn from(source: &[u8]) -> Self {
        SmallBytes::from_slice(source)
    }
}

impl<const N: usize> From<[u8; N]> for SmallBytes {
    fn from(source: [u8; N]) -> Self {
        SmallBytes::from_slice(&source)
    }
}

impl<const N: usize> From<&[u8; N]> for SmallBytes {
    fn from(source: &[u8; N]) -> Self {
        SmallBytes::from_slice(source)
    }
}

impl From<&Vec<u8>> for SmallBytes {
    fn from(source: &Vec<u8>) -> Self {
        SmallBytes::from_slice(source)
    }
}

impl PartialEq for SmallBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for SmallBytes {}

impl PartialEq<[u8]> for SmallBytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}

impl PartialEq<&[u8]> for SmallBytes {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_slice() == *other
    }
}

impl PartialEq<Vec<u8>> for SmallBytes {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> PartialEq<[u8; N]> for SmallBytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl PartialOrd for SmallBytes {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SmallBytes {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl fmt::Debug for SmallBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_slice(), formatter)
    }
}

impl core::hash::Hash for SmallBytes {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The type stays small enough that a tree item is 64 bytes: the
    /// decoded form of a node must not double because of it.
    #[test]
    fn the_type_is_thirty_two_bytes() {
        assert_eq!(core::mem::size_of::<SmallBytes>(), 32);
    }

    /// The same bytes compare, order and read the same whether they are
    /// inside the value or in a vector behind it.
    #[test]
    fn inline_and_heap_behave_alike() {
        let short = SmallBytes::from_slice(b"abc");
        let long = SmallBytes::from_slice(&[0x41u8; INLINE + 9]);
        assert!(matches!(short, SmallBytes::Inline { .. }));
        assert!(matches!(long, SmallBytes::Heap(_)));
        assert_eq!(short.as_slice(), b"abc");
        assert_eq!(long.len(), INLINE + 9);
        assert_eq!(short.to_vec(), b"abc".to_vec());
        assert_eq!(long.clone().into_vec(), vec![0x41u8; INLINE + 9]);

        // A vector of inline length is copied in, and one above it is kept.
        let borrowed = SmallBytes::from(vec![7u8; INLINE]);
        assert!(matches!(borrowed, SmallBytes::Inline { .. }));
        let kept = SmallBytes::from(vec![7u8; INLINE + 1]);
        assert!(matches!(kept, SmallBytes::Heap(_)));

        // Ordering is the ordering of the bytes, across the boundary.
        let a = SmallBytes::from_slice(b"b");
        let b = SmallBytes::from_slice(&[b'a'; INLINE + 1]);
        assert!(a > b);
        assert_eq!(SmallBytes::from_slice(b"x"), SmallBytes::from(vec![b'x']));
    }
}
