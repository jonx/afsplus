//! The owned-chain codec is the security segment codec with the kind as a
//! parameter: same bytes for the security kind, and one kind never reads as
//! another.
use afsplus_format::chain::{ChainKind, ChainSegment};
use afsplus_format::security::{SecuritySegment, SECURITY_CHAIN};
use afsplus_format::FormatError;

const BLOCK: usize = 4096;

const OTHER: ChainKind = ChainKind {
    block_type: u32::from_le_bytes(*b"AFSt"),
    max_bytes: 8192,
    label: "other",
    length_out_of_range: "other length out of range",
    ..SECURITY_CHAIN
};

fn segment(bytes: &[u8]) -> ChainSegment<'_> {
    ChainSegment {
        object_id: 42,
        format: 7,
        version: 3,
        total_len: bytes.len() as u32,
        index: 0,
        count: 1,
        next: 0,
        bytes,
    }
}

#[test]
fn security_kind_bytes_equal_the_security_segment_codec() {
    let bytes = [0x5au8; 300];
    let s = segment(&bytes);
    let generic = s.encode(&SECURITY_CHAIN, BLOCK, 9).unwrap();
    let specific = SecuritySegment {
        object_id: s.object_id,
        format: s.format,
        version: s.version,
        total_len: s.total_len,
        index: s.index,
        count: s.count,
        next: s.next,
        bytes: s.bytes,
    }
    .encode(BLOCK, 9)
    .unwrap();
    assert!(generic == specific, "the two encoders disagree");
    assert_eq!(&generic[0..4], b"AFSX");
    let (back, generation) = ChainSegment::decode(&SECURITY_CHAIN, &generic).unwrap();
    assert_eq!((back, generation), (s, 9));
}

#[test]
fn a_kind_refuses_the_blocks_of_another_kind() {
    let bytes = [1u8; 64];
    let security = segment(&bytes).encode(&SECURITY_CHAIN, BLOCK, 9).unwrap();
    let other = segment(&bytes).encode(&OTHER, BLOCK, 9).unwrap();
    assert_eq!(&other[0..4], b"AFSt");
    assert!(ChainSegment::decode(&OTHER, &security).is_err());
    assert!(ChainSegment::decode(&SECURITY_CHAIN, &other).is_err());
    assert_eq!(
        ChainSegment::decode(&OTHER, &other).unwrap().0,
        segment(&bytes)
    );
}

#[test]
fn the_bound_and_the_messages_belong_to_the_kind() {
    let bytes = vec![2u8; 8193];
    let mut s = segment(&bytes);
    s.count = 3;
    assert_eq!(
        s.encode(&OTHER, BLOCK, 9),
        Err(FormatError::Invalid("other length out of range"))
    );
    // The same content fits the security bound; only the position is wrong.
    s.count = 9;
    assert_eq!(
        s.encode(&SECURITY_CHAIN, BLOCK, 9),
        Err(FormatError::Invalid(
            "security segment position inconsistent"
        ))
    );
}
