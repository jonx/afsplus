//! Cross-record admission: exact prefix and exact read termination.
use afsplus_block::{BlockDevice, MemoryBackend, TraceBackend, TraceEvent};
use afsplus_core::intent_log::{log_slot_lbas, scan};
use afsplus_format::{
    crc32c,
    geometry::Geometry,
    intent_log::{LogOp, LogRecord},
    Timespec,
};

const GEO: Geometry = Geometry {
    block_size: 4096,
    total_blocks: 512,
    region_size: 256,
};
const UUID: [u8; 16] = [72; 16];

fn record(sequence: u32, start: u64) -> LogRecord {
    LogRecord {
        uuid: UUID,
        base_generation: 7,
        sequence,
        ops: vec![LogOp::Create {
            parent_id: 1,
            name: format!("file{sequence}").into_bytes(),
            expected_object_id: 10 + u64::from(sequence),
            size_bytes: 4096,
            content_crc: crc32c::crc32c(&vec![sequence as u8; 4096]),
            timestamp: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            extents: vec![(start, 1)],
        }],
    }
}

#[test]
fn middle_record_admission_preserves_exact_prefix_and_stops_reads() {
    let slots = log_slot_lbas(&GEO, 3).unwrap();
    for fault in 0..8 {
        let mut dev = MemoryBackend::new(4096, 512);
        let mut records = vec![record(1, 100), record(2, 101), record(3, 102)];
        for (i, lba) in [100, 101, 102].into_iter().enumerate() {
            dev.write_block(lba, &vec![(i + 1) as u8; 4096]).unwrap();
        }
        match fault {
            0 => {}
            1 => records[1].uuid[0] ^= 1,
            2 => records[1].base_generation += 1,
            3 => records[1].sequence = 3,
            4..=7 => {
                if let LogOp::Create {
                    extents,
                    content_crc,
                    ..
                } = &mut records[1].ops[0]
                {
                    match fault {
                        4 => extents[0].0 = 100,
                        5 => extents[0].0 = 512,
                        6 => extents[0].0 = 0,
                        7 => *content_crc ^= 1,
                        _ => unreachable!(),
                    }
                }
            }
            _ => unreachable!(),
        }
        for (lba, record) in slots.iter().zip(&records) {
            let bytes = record.encode(4096).unwrap();
            assert_eq!(LogRecord::decode(&bytes).unwrap(), *record);
            dev.write_block(*lba, &bytes).unwrap();
        }
        let mut traced = TraceBackend::new(dev);
        let found = scan(&mut traced, &GEO, 3, &UUID, 7, true).unwrap();
        assert_eq!(
            found.records,
            records[..if fault == 0 { 3 } else { 1 }],
            "fault {fault}"
        );
        assert_eq!(found.tail_note.is_some(), fault >= 3, "fault {fault}");
        let mut reads = vec![slots[0], 100, slots[1]];
        if fault == 0 {
            reads.extend([101, slots[2], 102]);
        }
        if fault == 7 {
            reads.push(101);
        }
        assert_eq!(
            traced.events(),
            reads
                .into_iter()
                .map(|lba| TraceEvent::Read { lba })
                .collect::<Vec<_>>(),
            "fault {fault}"
        );
        assert_eq!(traced.stats().writes, 0);
        assert_eq!(traced.stats().flushes, 0);
    }
}
