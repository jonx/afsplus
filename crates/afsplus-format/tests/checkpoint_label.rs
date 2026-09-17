//! Wire image of the checkpoint's volume label field.
use afsplus_format::checkpoint::{Checkpoint, SnapshotRoots};
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::{FormatError, DEFAULT_BLOCK_SIZE, OBJECT_ROOT};

fn checkpoint(label: &str) -> Checkpoint {
    Checkpoint {
        uuid: [9; 16],
        generation: 5,
        root_object_id: OBJECT_ROOT,
        object_map_block: 10,
        allocation_root_block: 13,
        reclaim_root_block: 12,
        next_object_id: 20,
        committed_tx_id: 5,
        free_blocks_total: 800,
        flags: 0,
        shared_extent_root_block: 0,
        label: label.into(),
        snapshot_roots: None,
    }
}

#[test]
fn the_label_field_has_the_documented_bytes_and_position() {
    let block = checkpoint("Dé").encode(DEFAULT_BLOCK_SIZE).unwrap();
    assert_eq!(&block[24..28], &168u32.to_le_bytes());
    let field = &block[HEADER_SIZE + 96..HEADER_SIZE + 168];
    assert_eq!(&field[..8], &[3, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&field[8..11], "Dé".as_bytes());
    assert!(field[11..].iter().all(|b| *b == 0));
    assert_eq!(
        Checkpoint::decode(&block, &[9; 16]).unwrap(),
        checkpoint("Dé")
    );

    // With snapshot roots the label keeps its place and the roots follow it.
    let mut with_roots = checkpoint("Dé");
    with_roots.snapshot_roots = Some(SnapshotRoots {
        registry: 0x0101,
        lifetimes: 0x0202,
    });
    let block = with_roots.encode(DEFAULT_BLOCK_SIZE).unwrap();
    assert_eq!(&block[24..28], &184u32.to_le_bytes());
    assert_eq!(&block[HEADER_SIZE + 96..HEADER_SIZE + 97], &[3]);
    assert_eq!(
        &block[HEADER_SIZE + 168..HEADER_SIZE + 184],
        &[1, 1, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(Checkpoint::decode(&block, &[9; 16]).unwrap(), with_roots);
}

#[test]
fn bounds_and_refusals() {
    let longest = "é".repeat(32);
    let block = checkpoint(&longest).encode(DEFAULT_BLOCK_SIZE).unwrap();
    assert_eq!(Checkpoint::decode(&block, &[9; 16]).unwrap().label, longest);
    assert_eq!(
        Checkpoint::decode(
            &checkpoint("").encode(DEFAULT_BLOCK_SIZE).unwrap(),
            &[9; 16]
        )
        .unwrap()
        .label,
        ""
    );
    assert_eq!(
        checkpoint(&"x".repeat(65)).encode(DEFAULT_BLOCK_SIZE),
        Err(FormatError::Overflow("volume label"))
    );
    assert_eq!(
        checkpoint("a\0b").encode(DEFAULT_BLOCK_SIZE),
        Err(FormatError::Invalid("volume label contains NUL"))
    );

    let clean = checkpoint("Label").encode(DEFAULT_BLOCK_SIZE).unwrap();
    let header = BlockHeader::verify(&clean, block_type::CHECKPOINT).unwrap();
    for (offset, value) in [
        (96usize, 65u8), // length above the bound
        (96, 6),         // length reaching into the padding: NUL inside
        (97, 1),         // reserved byte
        (103, 1),        // last reserved byte
        (104 + 5, b'x'), // byte after the label
        (104 + 63, 1),   // last padding byte
        (104, 0xff),     // invalid UTF-8
    ] {
        let mut block = clean.clone();
        block[HEADER_SIZE + offset] = value;
        header.seal(&mut block);
        assert!(
            Checkpoint::decode(&block, &[9; 16]).is_err(),
            "offset {offset} value {value}"
        );
    }
    // The old 96-byte and 112-byte payloads are no longer a checkpoint.
    for length in [96u32, 112, 167, 169, 183, 185] {
        let mut block = clean.clone();
        BlockHeader {
            payload_len: length,
            ..header
        }
        .seal(&mut block);
        assert_eq!(
            Checkpoint::decode(&block, &[9; 16]),
            Err(FormatError::Invalid("checkpoint payload length mismatch"))
        );
    }
}
