//! ADR-072 fixed-record and complete AFST leaf conformance images.
use afsplus_format::snapshot::{
    decode_key, LedgerState, LifetimeRecord, RegistryState, SnapshotRecord,
};
use afsplus_format::tree::{key_u64, TreeItem, TreeKind, TreeNode};

#[test]
fn fixed_record_bytes_are_explicit_and_id_exhaustion_never_wraps() {
    let record = SnapshotRecord {
        generation: 0x0102030405060708,
        committed_tx_id: 1,
        object_map_root: 0x1122334455667788,
    };
    let bytes = record.encode(u64::MAX, u64::MAX).unwrap();
    assert_eq!(
        bytes,
        [
            8, 7, 6, 5, 4, 3, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33,
            0x22, 0x11, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    assert_eq!(
        SnapshotRecord::decode(&bytes, u64::MAX, u64::MAX).unwrap(),
        record
    );
    assert_eq!(
        decode_key(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap(),
        0x0102030405060708
    );
    let state = RegistryState {
        next_id: u64::MAX - 1,
    };
    let (id, exhausted) = state.allocate_id().unwrap();
    assert_eq!(id, u64::MAX - 1);
    assert_eq!(
        RegistryState::decode(&exhausted.encode().unwrap()).unwrap(),
        exhausted
    );
    assert!(exhausted.allocate_id().is_err());
    assert!(RegistryState { next_id: 0 }.allocate_id().is_err());
}

#[test]
fn every_short_or_long_record_and_nonzero_reserved_byte_is_rejected() {
    for length in 0..65 {
        if length == 32 {
            continue;
        }
        let bytes = vec![0; length];
        assert!(RegistryState::decode(&bytes).is_err());
        assert!(LedgerState::decode(&bytes, 100).is_err());
        assert!(SnapshotRecord::decode(&bytes, 20, 100).is_err());
        assert!(LifetimeRecord::decode(&bytes, 10, 20, 100).is_err());
    }
    for length in 0..17 {
        assert_eq!(decode_key(&vec![0; length]).is_ok(), length == 8);
    }
    let registry = RegistryState { next_id: 2 }.encode().unwrap();
    for i in 8..32 {
        let mut bytes = registry;
        bytes[i] = 1;
        assert!(RegistryState::decode(&bytes).is_err());
    }
    let ledger = LedgerState {
        scan_position: 0,
        retained_blocks: 1,
    }
    .encode(100)
    .unwrap();
    for i in 16..32 {
        let mut bytes = ledger;
        bytes[i] = 1;
        assert!(LedgerState::decode(&bytes, 100).is_err());
    }
    let snapshot = SnapshotRecord {
        generation: 3,
        committed_tx_id: 3,
        object_map_root: 10,
    }
    .encode(20, 100)
    .unwrap();
    let lifetime = LifetimeRecord {
        blocks: 1,
        birth: 2,
        retirement: 4,
    }
    .encode(10, 20, 100)
    .unwrap();
    for i in 24..32 {
        let mut bytes = snapshot;
        bytes[i] = 1;
        assert!(SnapshotRecord::decode(&bytes, 20, 100).is_err());
        let mut bytes = lifetime;
        bytes[i] = 1;
        assert!(LifetimeRecord::decode(&bytes, 10, 20, 100).is_err());
    }
}

#[test]
fn lifetime_boundaries_and_context_reject_impossible_ownership() {
    let record = LifetimeRecord {
        blocks: 4,
        birth: 3,
        retirement: 7,
    };
    let bytes = record.encode(96, 7, 100).unwrap();
    assert_eq!(LifetimeRecord::decode(&bytes, 96, 7, 100).unwrap(), record);
    assert_eq!(
        (0..9).map(|g| record.contains(g)).collect::<Vec<_>>(),
        [false, false, false, true, true, true, true, false, false]
    );
    assert!(LifetimeRecord {
        retirement: 0,
        ..record
    }
    .contains(u64::MAX));
    for start in [0, 97, u64::MAX - 1] {
        assert!(record.encode(start, 7, 100).is_err());
    }
    for bad in [
        LifetimeRecord {
            blocks: 0,
            ..record
        },
        LifetimeRecord { birth: 0, ..record },
        LifetimeRecord { birth: 8, ..record },
        LifetimeRecord {
            retirement: 3,
            ..record
        },
        LifetimeRecord {
            retirement: 8,
            ..record
        },
    ] {
        assert!(bad.encode(10, 7, 100).is_err());
    }
    assert!(LifetimeRecord {
        blocks: 4,
        birth: 1,
        retirement: 2
    }
    .encode(u64::MAX - 1, 2, u64::MAX)
    .is_err());
    for state in [
        LedgerState {
            scan_position: 100,
            retained_blocks: 0,
        },
        LedgerState {
            scan_position: 0,
            retained_blocks: 101,
        },
    ] {
        assert!(state.encode(100).is_err());
    }
    let valid = SnapshotRecord {
        generation: 3,
        committed_tx_id: 3,
        object_map_root: 10,
    };
    for bad in [
        SnapshotRecord {
            generation: 0,
            ..valid
        },
        SnapshotRecord {
            generation: 4,
            ..valid
        },
        SnapshotRecord {
            committed_tx_id: 0,
            ..valid
        },
        SnapshotRecord {
            committed_tx_id: 4,
            ..valid
        },
        SnapshotRecord {
            object_map_root: 0,
            ..valid
        },
        SnapshotRecord {
            object_map_root: 100,
            ..valid
        },
    ] {
        assert!(bad.encode(3, 100).is_err());
    }
}

#[test]
fn complete_registry_and_ledger_leaf_images_round_trip_and_detect_damage() {
    let registry = [
        RegistryState { next_id: 2 }.encode().unwrap(),
        SnapshotRecord {
            generation: 3,
            committed_tx_id: 3,
            object_map_root: 10,
        }
        .encode(7, 100)
        .unwrap(),
    ];
    let ledger = [
        LedgerState {
            scan_position: 16,
            retained_blocks: 4,
        }
        .encode(100)
        .unwrap(),
        LifetimeRecord {
            blocks: 4,
            birth: 3,
            retirement: 7,
        }
        .encode(16, 7, 100)
        .unwrap(),
    ];
    for (kind, key, values) in [
        (TreeKind::SnapshotRegistry, 1, registry),
        (TreeKind::SnapshotLifetimes, 16, ledger),
    ] {
        let mut node = TreeNode::leaf(kind, 0);
        node.items = [0, key]
            .into_iter()
            .zip(values)
            .map(|(id, value)| TreeItem {
                key: key_u64(id).to_vec(),
                value: value.to_vec(),
            })
            .collect();
        node.subtree_items = 2;
        let image = node.encode(4096, 7).unwrap();
        let (decoded, _) = TreeNode::decode(&image).unwrap();
        assert_eq!(decoded, node);
        assert_eq!(TreeNode::fixed_item_capacity(4096, 8, 32).unwrap(), 84);
        for offset in [0, 4, 8, 16, 24, 28, 32, 64, 80, 112, 128, 4095] {
            let mut damaged = image.clone();
            damaged[offset] ^= 1;
            assert!(TreeNode::decode(&damaged).is_err());
        }
        // Revalidate typed values after the enclosing tree decoder.
        match kind {
            TreeKind::SnapshotRegistry => {
                assert_eq!(
                    RegistryState::decode(&decoded.items[0].value)
                        .unwrap()
                        .next_id,
                    2
                );
                assert_eq!(
                    SnapshotRecord::decode(&decoded.items[1].value, 7, 100)
                        .unwrap()
                        .object_map_root,
                    10
                );
            }
            TreeKind::SnapshotLifetimes => {
                assert_eq!(
                    LedgerState::decode(&decoded.items[0].value, 100)
                        .unwrap()
                        .retained_blocks,
                    4
                );
                assert_eq!(
                    LifetimeRecord::decode(&decoded.items[1].value, 16, 7, 100)
                        .unwrap()
                        .birth,
                    3
                );
            }
            _ => unreachable!(),
        }
    }
}
