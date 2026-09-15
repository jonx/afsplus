//! Resealed cross-block errors must reach the reclaim caller, not fail CRC admission.
use afsplus_block::{BlockDevice, MemoryBackend, TraceBackend};
use afsplus_core::reclaim::load_all;
use afsplus_format::{
    geometry::Geometry,
    reclaim::{
        ReclaimCaps, ReclaimEntry, ReclaimRoot, ReclaimSegment, ReclaimTable, SegmentRef, TableRef,
    },
};

#[test]
fn independently_decodable_queue_blocks_reject_inconsistent_relations() {
    let geo = Geometry {
        block_size: 4096,
        total_blocks: 512,
        region_size: 256,
    };
    for fault in 0..11 {
        let mut root = ReclaimRoot::empty(ReclaimCaps::default());
        root.pending_blocks = 2;
        root.appended_blocks_total = 2;
        root.table_refs.push(TableRef {
            lba: 22,
            ref_count: 1,
        });
        let mut table = ReclaimTable {
            refs: vec![SegmentRef {
                lba: 21,
                entry_count: 1,
            }],
        };
        let mut segment = ReclaimSegment {
            entries: vec![ReclaimEntry {
                start: 100,
                blocks: 2,
                retire_generation: 7,
            }],
        };
        let mut generations = [8, 8, 8];
        match fault {
            0 => {}
            1 => generations[0] = 9,
            2 => generations[1] = 9,
            3 => generations[2] = 9,
            4 => root.table_refs[0].ref_count = 2,
            5 => table.refs[0].entry_count = 2,
            6 => segment.entries[0].start = 255,
            7 => segment.entries[0].start = 511,
            8 => {
                root.pending_blocks = 3;
                root.appended_blocks_total = 3;
            }
            9 => root.head_block_offset = 2,
            10 => segment.entries[0].retire_generation = 9,
            _ => unreachable!(),
        }
        let rb = root.encode(4096, generations[0]).unwrap();
        let tb = table.encode(4096, generations[1]).unwrap();
        let sb = segment.encode(4096, generations[2]).unwrap();
        assert!(ReclaimRoot::decode(&rb).is_ok(), "fault {fault}");
        assert!(ReclaimTable::decode(&tb).is_ok(), "fault {fault}");
        assert!(ReclaimSegment::decode(&sb).is_ok(), "fault {fault}");
        let mut image = MemoryBackend::new(4096, 512);
        for (lba, bytes) in [(20, rb), (22, tb), (21, sb)] {
            image.write_block(lba, &bytes).unwrap();
        }
        let mut traced = TraceBackend::new(image);
        let result = load_all(&mut traced, &geo, 20, 8);
        if fault == 0 {
            let loaded = result.unwrap();
            assert_eq!(loaded.runs, segment.entries);
            assert_eq!(loaded.structure_blocks, [20, 22, 21]);
            assert_eq!(loaded.pending_blocks, 2);
        } else {
            let error = result
                .err()
                .expect("malformed relation accepted")
                .to_string();
            let expected = [
                "",
                "root generation",
                "table 22",
                "segment 21",
                "table 22",
                "segment 21",
                "crosses a region",
                "outside allocatable",
                "pending blocks",
                "cursor beyond",
                "entry generation",
            ][fault];
            assert!(error.contains(expected), "fault {fault}: {error}");
        }
        assert!(traced.stats().reads <= 3);
        assert_eq!(traced.stats().writes, 0);
        assert_eq!(traced.stats().flushes, 0);
    }
}
