use afsplus_block::powercut::for_each_overlay_crash_state;
use afsplus_block::{
    crash_states, BlockDevice, MemoryBackend, OverlayBackend, OverlayLimits, RecordedOp,
};

#[test]
fn overlay_cuts_match_independent_memory_images_byte_for_byte() {
    for bs in [512, 4096] {
        let mut base = MemoryBackend::new(bs, 8);
        base.write_block(1, &vec![17; bs]).unwrap();
        let log = vec![
            RecordedOp::Write {
                lba: 1,
                data: vec![33; bs],
            },
            RecordedOp::Flush,
            RecordedOp::Write {
                lba: 2,
                data: vec![44; bs],
            },
            RecordedOp::Write {
                lba: 1,
                data: vec![55; bs],
            },
            RecordedOp::Write {
                lba: 1,
                data: vec![66; bs],
            },
            RecordedOp::Flush,
        ];
        let overlay = OverlayBackend::new(
            base.clone(),
            OverlayLimits {
                branches: 3,
                entries: 8,
            },
        )
        .unwrap();
        for cut in 0..=log.len() {
            let expected = crash_states(&base, &log, cut);
            let mut count = 0;
            for_each_overlay_crash_state(&overlay, &log, cut, |mut image, description| {
                let reference = &expected[count];
                assert_eq!(description, reference.description);
                for lba in 0..8 {
                    let mut out = vec![0; bs];
                    image.read_block(lba, &mut out)?;
                    assert_eq!(out, reference.image.peek(lba), "{description}, lba {lba}");
                }
                count += 1;
                Ok(())
            })
            .unwrap();
            assert_eq!(count, expected.len());
            assert_eq!((overlay.live_branches(), overlay.live_entries()), (1, 0));
        }
    }
}

#[test]
fn incomplete_enumeration_is_explicit_and_releases_branch_charges() {
    let base = OverlayBackend::new(
        MemoryBackend::new(512, 8),
        OverlayLimits {
            branches: 3,
            entries: 0,
        },
    )
    .unwrap();
    let log = vec![RecordedOp::Write {
        lba: 1,
        data: vec![7; 512],
    }];
    let mut visits = 0;
    assert!(for_each_overlay_crash_state(&base, &log, 1, |_, _| {
        visits += 1;
        Ok(())
    })
    .is_err());
    assert_eq!(visits, 1); // The lost-write state succeeds; applied write exceeds the budget.
    assert_eq!((base.live_branches(), base.live_entries()), (1, 0));
    assert!(
        for_each_overlay_crash_state(&base, &log, 2, |_, _| panic!("invalid point callback"))
            .is_err()
    );
    let long = vec![
        RecordedOp::Write {
            lba: 0,
            data: vec![0; 512]
        };
        13
    ];
    assert!(
        for_each_overlay_crash_state(&base, &long, 13, |_, _| panic!("overbudget callback"))
            .is_err()
    );
    assert!(for_each_overlay_crash_state(&base, &[], 0, |_, _| Err(
        afsplus_block::BlockError::Injected("consumer")
    ))
    .is_err());
    assert_eq!((base.live_branches(), base.live_entries()), (1, 0));
}
