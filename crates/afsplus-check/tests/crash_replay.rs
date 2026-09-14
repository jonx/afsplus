use afsplus_block::{for_each_crash_state, BlockDevice, MemoryBackend, RecordedOp};
use afsplus_check::crash_replay::select;

#[test]
fn every_selected_variant_matches_existing_oracle_including_repeated_lbas() {
    for bs in [512, 4096] {
        let mut base = MemoryBackend::new(bs, 8);
        base.write_block(1, &vec![9; bs]).unwrap();
        let log = vec![
            RecordedOp::Write {
                lba: 1,
                data: vec![1; bs],
            },
            RecordedOp::Flush,
            RecordedOp::Write {
                lba: 1,
                data: vec![2; bs],
            },
            RecordedOp::Write {
                lba: 2,
                data: vec![3; bs],
            },
            RecordedOp::Write {
                lba: 1,
                data: vec![4; bs],
            },
        ];
        for cut in 0..=log.len() {
            let mut count = 0;
            for_each_crash_state(&base, &log, cut, |state| {
                let selected = select(&base, &log, cut, count).unwrap();
                for lba in 0..base.total_blocks() {
                    assert_eq!(
                        selected.peek(lba),
                        state.image.peek(lba),
                        "bs={bs} cut={cut} variant={count}"
                    );
                }
                count += 1;
            });
            assert!(select(&base, &log, cut, count).is_err());
        }
        assert_eq!(base.peek(1), vec![9; bs]);
        assert_eq!(base.peek(2), vec![0; bs]);
    }
}

#[test]
fn invalid_and_excessive_selections_refuse() {
    let base = MemoryBackend::new(4096, 8);
    assert!(select(&base, &[], 1, 0).is_err());
    assert!(select(
        &base,
        &[RecordedOp::Write {
            lba: 8,
            data: vec![0; 4096]
        }],
        1,
        0
    )
    .is_err());
    assert!(select(
        &base,
        &[RecordedOp::Write {
            lba: 0,
            data: vec![0; 512]
        }],
        1,
        0
    )
    .is_err());
    let log = vec![
        RecordedOp::Write {
            lba: 0,
            data: vec![1; 4096]
        };
        13
    ];
    assert!(select(&base, &log, 13, 0).is_err());
}
