use afsplus_block::{
    for_each_crash_state, for_each_crash_state_with_budget, MemoryBackend, RecordedOp,
};

#[test]
fn explicit_budget_enumerates_every_thirteen_write_subset() {
    let base = MemoryBackend::new(4096, 16);
    let log: Vec<_> = (0..13)
        .map(|i| RecordedOp::Write {
            lba: i,
            data: vec![1; 4096],
        })
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut tears = 0;
    for_each_crash_state_with_budget(&base, &log, 13, 13, |state| {
        if state.description.contains("unflushed subset") {
            let mask = (0..13).fold(0u16, |mask, i| {
                mask | (u16::from(state.image.peek(i)[0]) << i)
            });
            assert!(seen.insert(mask));
        } else {
            tears += 1;
        }
    });
    assert_eq!(seen.len(), 8192);
    assert_eq!(tears, 39);
}

#[test]
fn default_budget_still_refuses_thirteen_writes_before_visiting() {
    let base = MemoryBackend::new(4096, 16);
    let log: Vec<_> = (0..13)
        .map(|i| RecordedOp::Write {
            lba: i,
            data: vec![1; 4096],
        })
        .collect();
    let mut visits = 0;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for_each_crash_state(&base, &log, 13, |_| visits += 1);
    }))
    .is_err());
    assert_eq!(visits, 0);
}
