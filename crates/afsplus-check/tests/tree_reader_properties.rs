//! Finite caller properties for generic typed-tree traversal. Values are opaque:
//! this suite qualifies tree ownership/ranges, not each adapter's leaf payload.
use std::collections::BTreeMap;

use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_core::tree::{self, TreeSpec};
use afsplus_core::CoreError;
use afsplus_format::geometry::Geometry;
use afsplus_format::header::{block_type, BlockHeader, HEADER_SIZE};
use afsplus_format::tree::{child_value, ChildRef, TreeItem, TreeKind, TreeNode};

const ROOT: u64 = 16;
const GEO: Geometry = Geometry {
    block_size: 4096,
    total_blocks: 128,
    region_size: 64,
};
const KINDS: [TreeKind; 7] = [
    TreeKind::ObjectMap,
    TreeKind::Directory,
    TreeKind::ExtentMap,
    TreeKind::AllocationRoot,
    TreeKind::SharedExtents,
    TreeKind::SnapshotRegistry,
    TreeKind::SnapshotLifetimes,
];
type Map = BTreeMap<Vec<u8>, Vec<u8>>;

struct Fixture {
    nodes: BTreeMap<u64, TreeNode>,
    expected: Map,
    spec: TreeSpec,
}

fn key(value: u16) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn parent(
    kind: TreeKind,
    owner: u64,
    level: u8,
    left: u64,
    right: u64,
    (left_count, right_count): (u64, u64),
    separator: Vec<u8>,
) -> TreeNode {
    TreeNode {
        kind,
        owner,
        level,
        subtree_items: left_count + right_count,
        leftmost_child: left,
        leftmost_items: left_count,
        items: vec![TreeItem {
            key: separator,
            value: child_value(ChildRef {
                lba: right,
                subtree_items: right_count,
            })
            .unwrap(),
        }],
    }
}

fn fixture(kind: TreeKind, variant: usize) -> Fixture {
    let owner = if matches!(kind, TreeKind::Directory | TreeKind::ExtentMap) {
        42
    } else {
        0
    };
    let mut nodes = BTreeMap::new();
    let mut expected = Map::new();
    let mut sizes = [0; 4];
    for (leaf, size) in sizes.iter_mut().enumerate() {
        *size = 1 + ((variant + leaf) % 4) as u64;
        let mut node = TreeNode::leaf(kind, owner);
        for position in 0..*size {
            let number = 1 + leaf as u16 * 32 + position as u16 * (2 + variant as u16);
            let k = key(number);
            let value = vec![leaf as u8, position as u8, variant as u8, 0xa5];
            expected.insert(k.clone(), value.clone());
            node.items.push(TreeItem { key: k, value });
        }
        node.subtree_items = *size;
        nodes.insert(19 + leaf as u64, node);
    }
    nodes.insert(
        17,
        parent(kind, owner, 1, 19, 20, (sizes[0], sizes[1]), key(33)),
    );
    nodes.insert(
        18,
        parent(kind, owner, 1, 21, 22, (sizes[2], sizes[3]), key(97)),
    );
    nodes.insert(
        ROOT,
        parent(
            kind,
            owner,
            2,
            17,
            18,
            (sizes[0] + sizes[1], sizes[2] + sizes[3]),
            key(65),
        ),
    );
    Fixture {
        nodes,
        expected,
        spec: TreeSpec {
            kind,
            owner,
            max_generation: 7,
        },
    }
}

impl Fixture {
    fn device(&self) -> MemoryBackend {
        let mut dev = MemoryBackend::new(GEO.block_size, GEO.total_blocks);
        for (&lba, node) in &self.nodes {
            dev.apply_raw(lba, &node.encode(GEO.block_size, 7).unwrap());
        }
        dev
    }
    fn items(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.expected
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[test]
fn every_kind_and_small_shape_matches_ordered_map_with_bounded_point_reads() {
    for kind in KINDS {
        for variant in 0..8 {
            let fixture = fixture(kind, variant);
            let mut dev = TraceBackend::new(fixture.device());
            let summary = tree::validate_tree(&mut dev, &GEO, ROOT, fixture.spec).unwrap();
            assert_eq!(
                (summary.items, summary.nodes, summary.height),
                (fixture.expected.len() as u64, 7, 3)
            );
            assert_eq!(dev.stats().reads, 7);
            for number in 0..=128 {
                let k = key(number);
                dev.reset();
                let (found, stats) = tree::lookup(&mut dev, &GEO, ROOT, fixture.spec, &k).unwrap();
                assert_eq!(
                    found,
                    fixture.expected.get(&k).cloned(),
                    "{kind:?}/{variant}/{number}"
                );
                assert_eq!(
                    (dev.stats().reads, stats.pages_read, stats.peak_page_buffers),
                    (3, 3, 2)
                );
                dev.reset();
                let (floor, stats) =
                    tree::lookup_floor(&mut dev, &GEO, ROOT, fixture.spec, &k).unwrap();
                let expected = fixture
                    .expected
                    .range(..=k)
                    .next_back()
                    .map(|(k, v)| (k.clone(), v.clone()));
                assert_eq!(floor, expected, "{kind:?}/{variant}/{number}");
                assert_eq!(
                    (dev.stats().reads, stats.pages_read, stats.peak_page_buffers),
                    (3, 3, 2)
                );
                assert_eq!((dev.stats().writes, dev.stats().flushes), (0, 0));
            }
        }
    }
}

#[test]
fn ordinal_and_key_pages_concatenate_without_gaps_or_duplicates() {
    for variant in 0..8 {
        let fixture = fixture(TreeKind::Directory, variant);
        let expected = fixture.items();
        let mut dev = TraceBackend::new(fixture.device());
        for limit in 0..=5 {
            for start in 0..=expected.len() + 1 {
                dev.reset();
                let page =
                    tree::read_range(&mut dev, &GEO, ROOT, fixture.spec, start as u64, limit)
                        .unwrap();
                let wanted: Vec<_> = expected.iter().skip(start).take(limit).cloned().collect();
                assert_eq!(page.items, wanted);
                assert_eq!(page.total_items, expected.len() as u64);
                assert!(dev.stats().reads <= 7);
                if limit == 0 || start >= expected.len() {
                    assert_eq!(dev.stats().reads, 1);
                }
            }
            for number in 0..=128 {
                dev.reset();
                let low = key(number);
                let (page, stats) =
                    tree::read_key_page(&mut dev, &GEO, ROOT, fixture.spec, &low, limit).unwrap();
                let wanted: Vec<_> = fixture
                    .expected
                    .range(low..)
                    .take(limit)
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                assert_eq!(page.items, wanted);
                assert_eq!(page.total_items, expected.len() as u64);
                assert_eq!(dev.stats().reads, stats.pages_read);
                assert!(stats.pages_read <= 7);
                assert!(stats.peak_page_buffers <= 6);
                if limit == 0 {
                    assert_eq!(stats.pages_read, 1);
                }
            }
            if limit == 0 {
                continue;
            }
            let mut ordinal = Vec::new();
            while ordinal.len() < expected.len() {
                let page = tree::read_range(
                    &mut dev,
                    &GEO,
                    ROOT,
                    fixture.spec,
                    ordinal.len() as u64,
                    limit,
                )
                .unwrap();
                assert!(!page.items.is_empty());
                ordinal.extend(page.items);
            }
            assert_eq!(ordinal, expected);
            let mut by_key = Vec::new();
            let mut low = key(0);
            loop {
                let (page, _) =
                    tree::read_key_page(&mut dev, &GEO, ROOT, fixture.spec, &low, limit).unwrap();
                let Some((last, _)) = page.items.last() else {
                    break;
                };
                low = key(u16::from_be_bytes(last.as_slice().try_into().unwrap()) + 1);
                by_key.extend(page.items);
                assert!(by_key.len() <= expected.len());
            }
            assert_eq!(by_key, expected);
        }
        let mut visited = Vec::new();
        dev.reset();
        let summary =
            tree::visit_tree_nodes_bounded(&mut dev, &GEO, ROOT, fixture.spec, |lba, _| {
                visited.push(lba);
                Ok(())
            })
            .unwrap();
        visited.sort_unstable();
        assert_eq!(visited, (16..=22).collect::<Vec<_>>());
        assert_eq!(
            (summary.nodes, summary.items, dev.stats().reads),
            (7, expected.len() as u64, 7)
        );
    }
}

fn reject_traversed(dev: MemoryBackend, spec: TreeSpec, query: u16, bound: u64) {
    let mut dev = TraceBackend::new(dev);
    let result = tree::lookup(&mut dev, &GEO, ROOT, spec, &key(query));
    assert!(matches!(result, Err(CoreError::Corrupt(_))));
    assert!(dev.stats().reads <= bound);
    dev.reset();
    assert!(tree::lookup_floor(&mut dev, &GEO, ROOT, spec, &key(query)).is_err());
    assert!(dev.stats().reads <= bound);
    dev.reset();
    assert!(tree::read_key_page(&mut dev, &GEO, ROOT, spec, &key(query), 1).is_err());
    assert!(dev.stats().reads <= bound);
    dev.reset();
    assert!(tree::validate_tree(&mut dev, &GEO, ROOT, spec).is_err());
    assert!(dev.stats().reads <= 7);
    assert_eq!((dev.stats().writes, dev.stats().flushes), (0, 0));
}

#[test]
fn traversed_identity_child_range_and_cycle_corruptions_fail_boundedly() {
    for at in [ROOT, 17, 19] {
        for identity in 0..3 {
            let mut fixture = fixture(TreeKind::Directory, 2);
            let node = fixture.nodes.get_mut(&at).unwrap();
            if identity == 0 {
                node.owner += 1;
            }
            if identity == 1 {
                node.kind = TreeKind::ExtentMap;
            }
            let mut dev = fixture.device();
            if identity == 2 {
                dev.apply_raw(at, &fixture.nodes[&at].encode(GEO.block_size, 8).unwrap());
            }
            reject_traversed(dev, fixture.spec, 1, 3);
        }
    }
    for bad_lba in [1, 8, 64, 69, 128, u64::MAX, ROOT] {
        let mut fixture = fixture(TreeKind::Directory, 2);
        fixture.nodes.get_mut(&ROOT).unwrap().leftmost_child = bad_lba;
        reject_traversed(fixture.device(), fixture.spec, 1, 1);
    }
    let mut fixture = fixture(TreeKind::Directory, 2);
    fixture.nodes.get_mut(&17).unwrap().level = 2;
    reject_traversed(fixture.device(), fixture.spec, 1, 2);

    let mut fixture = self::fixture(TreeKind::Directory, 2);
    fixture
        .nodes
        .get_mut(&19)
        .unwrap()
        .items
        .last_mut()
        .unwrap()
        .key = key(40);
    reject_traversed(fixture.device(), fixture.spec, 1, 3);

    let mut fixture = self::fixture(TreeKind::Directory, 2);
    fixture.nodes.get_mut(&ROOT).unwrap().items[0].key = key(64);
    reject_traversed(fixture.device(), fixture.spec, 65, 3);

    let mut fixture = self::fixture(TreeKind::Directory, 2);
    fixture
        .nodes
        .insert(19, TreeNode::leaf(TreeKind::Directory, fixture.spec.owner));
    reject_traversed(fixture.device(), fixture.spec, 1, 3);
}

#[test]
fn global_counts_and_unvisited_damage_belong_to_checker_not_point_lookup() {
    let mut fixture = fixture(TreeKind::Directory, 2);
    let root = fixture.nodes.get_mut(&ROOT).unwrap();
    root.leftmost_items += 1;
    root.subtree_items += 1;
    let mut dev = TraceBackend::new(fixture.device());
    assert!(tree::lookup(&mut dev, &GEO, ROOT, fixture.spec, &key(1))
        .unwrap()
        .0
        .is_some());
    assert_eq!(dev.stats().reads, 3);
    assert!(tree::validate_tree(&mut dev, &GEO, ROOT, fixture.spec).is_err());
    assert!(tree::read_range(&mut dev, &GEO, ROOT, fixture.spec, 0, 1).is_err());
    assert!(tree::read_key_page(&mut dev, &GEO, ROOT, fixture.spec, &key(1), 1).is_err());

    let mut fixture = self::fixture(TreeKind::Directory, 2);
    fixture.nodes.get_mut(&22).unwrap().owner += 1;
    let mut dev = TraceBackend::new(fixture.device());
    assert_eq!(
        tree::lookup(&mut dev, &GEO, ROOT, fixture.spec, &key(1))
            .unwrap()
            .0,
        fixture.expected.get(&key(1)).cloned()
    );
    assert_eq!(dev.stats().reads, 3);
    assert!(tree::validate_tree(&mut dev, &GEO, ROOT, fixture.spec).is_err());

    // Duplicate ownership is an exhaustive-validator obligation; do not require
    // an unrelated point lookup to visit the duplicate right-hand branch.
    let mut fixture = self::fixture(TreeKind::Directory, 2);
    fixture.nodes.get_mut(&ROOT).unwrap().items[0].value = child_value(ChildRef {
        lba: 17,
        subtree_items: fixture.nodes[&18].subtree_items,
    })
    .unwrap();
    let mut dev = TraceBackend::new(fixture.device());
    assert!(tree::validate_tree(&mut dev, &GEO, ROOT, fixture.spec).is_err());
    assert!(dev.stats().reads <= 7);
}

#[test]
fn resealed_malformed_child_fields_and_query_admission_reach_core_readers() {
    let fixture = fixture(TreeKind::Directory, 2);
    // Payload offsets: level=1, leftmost LBA=16, first item value length=34.
    // Header resealing is shared infrastructure, not an independent CRC oracle.
    for (offset, bytes) in [
        (1, vec![16]),
        (16, vec![0; 8]),
        (34, 15u16.to_le_bytes().to_vec()),
    ] {
        let mut dev = fixture.device();
        let mut raw = dev.peek(ROOT);
        let header = BlockHeader::verify(&raw, block_type::TREE_NODE).unwrap();
        raw[HEADER_SIZE + offset..HEADER_SIZE + offset + bytes.len()].copy_from_slice(&bytes);
        header.seal(&mut raw);
        dev.apply_raw(ROOT, &raw);
        reject_traversed(dev, fixture.spec, 1, 1);
    }
    let mut dev = TraceBackend::new(fixture.device());
    for invalid in [Vec::new(), vec![1; 1021]] {
        assert!(tree::lookup(&mut dev, &GEO, ROOT, fixture.spec, &invalid).is_err());
        assert!(tree::lookup_floor(&mut dev, &GEO, ROOT, fixture.spec, &invalid).is_err());
        assert!(tree::read_key_page(&mut dev, &GEO, ROOT, fixture.spec, &invalid, 1).is_err());
    }
    for root in [0, 8, 64, 128, u64::MAX] {
        assert!(tree::lookup(&mut dev, &GEO, root, fixture.spec, &key(1)).is_err());
        assert!(tree::validate_tree(&mut dev, &GEO, root, fixture.spec).is_err());
    }
    assert_eq!(dev.stats().reads, 0);
}
