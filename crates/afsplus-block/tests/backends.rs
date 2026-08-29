//! Backend semantics: zero-fill reads, bounds checks, trace accounting,
//! fault injection, and the power-cut crash-state generator itself.

use afsplus_block::{
    crash_states, ActivityBackend, ActivityEvent, ActivityOperation, ActivityPhase, ActivitySink,
    BlockDevice, BlockError, FaultBackend, FaultPlan, FileBackend, MemoryBackend, RecordedOp,
    RecordingBackend, TraceBackend,
};

const BS: usize = 4096;

fn block(fill: u8) -> Vec<u8> {
    vec![fill; BS]
}

#[test]
fn memory_backend_zero_fill_and_bounds() {
    let mut dev = MemoryBackend::new(BS, 8);
    let mut buf = block(0xAA);
    dev.read_block(3, &mut buf).unwrap();
    assert_eq!(buf, block(0));
    dev.write_block(3, &block(0x55)).unwrap();
    dev.read_block(3, &mut buf).unwrap();
    assert_eq!(buf, block(0x55));
    assert!(matches!(dev.read_block(8, &mut buf), Err(BlockError::OutOfBounds { .. })));
    assert!(matches!(dev.write_block(2, &[0u8; 100]), Err(BlockError::WrongBufferSize { .. })));
}

#[test]
fn file_backend_sparse_tail_reads_zero() {
    let dir = std::env::temp_dir().join(format!("afsplus-block-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sparse.img");
    {
        let mut dev = FileBackend::create(&path, BS, 64).unwrap();
        dev.write_block(2, &block(0x11)).unwrap();
        dev.flush().unwrap();
    }
    let mut dev = FileBackend::open(&path, BS, 64).unwrap();
    let mut buf = block(0xFF);
    dev.read_block(2, &mut buf).unwrap();
    assert_eq!(buf, block(0x11));
    // Block 40 is beyond the file's written extent: must read as zeros.
    dev.read_block(40, &mut buf).unwrap();
    assert_eq!(buf, block(0));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn trace_backend_accounts_operations() {
    let mut dev = TraceBackend::new(MemoryBackend::new(BS, 8));
    let mut buf = block(0);
    dev.write_block(1, &block(1)).unwrap();
    dev.write_block(2, &block(2)).unwrap();
    dev.read_block(1, &mut buf).unwrap();
    dev.flush().unwrap();
    let stats = dev.stats();
    assert_eq!(stats.writes, 2);
    assert_eq!(stats.reads, 1);
    assert_eq!(stats.flushes, 1);
    assert_eq!(stats.bytes_written, 2 * BS as u64);
}

#[derive(Debug, Default)]
struct ActivityLog(Vec<ActivityEvent>);

impl ActivitySink for ActivityLog {
    fn on_activity(&mut self, event: ActivityEvent) {
        self.0.push(event);
    }
}

#[derive(Debug, Default)]
struct WriteActivityLog(Vec<ActivityEvent>);

impl ActivitySink for WriteActivityLog {
    fn is_enabled(&self, operation: ActivityOperation) -> bool {
        matches!(
            operation,
            ActivityOperation::Write | ActivityOperation::Flush
        )
    }

    fn on_activity(&mut self, event: ActivityEvent) {
        self.0.push(event);
    }
}

#[test]
fn activity_backend_brackets_io_without_copying_payloads() {
    let mut dev = ActivityBackend::new(MemoryBackend::new(BS, 8), ActivityLog::default());
    let mut buf = block(0);
    dev.write_block(3, &block(7)).unwrap();
    dev.read_block(3, &mut buf).unwrap();
    dev.flush().unwrap();

    let (_, log) = dev.into_parts();
    assert_eq!(
        log.0,
        vec![
            ActivityEvent {
                operation: ActivityOperation::Write,
                phase: ActivityPhase::Begin,
                lba: Some(3),
                block_count: 1,
            },
            ActivityEvent {
                operation: ActivityOperation::Write,
                phase: ActivityPhase::End { success: true },
                lba: Some(3),
                block_count: 1,
            },
            ActivityEvent {
                operation: ActivityOperation::Read,
                phase: ActivityPhase::Begin,
                lba: Some(3),
                block_count: 1,
            },
            ActivityEvent {
                operation: ActivityOperation::Read,
                phase: ActivityPhase::End { success: true },
                lba: Some(3),
                block_count: 1,
            },
            ActivityEvent {
                operation: ActivityOperation::Flush,
                phase: ActivityPhase::Begin,
                lba: None,
                block_count: 0,
            },
            ActivityEvent {
                operation: ActivityOperation::Flush,
                phase: ActivityPhase::End { success: true },
                lba: None,
                block_count: 0,
            },
        ]
    );
}

#[test]
fn activity_backend_reports_failed_completion() {
    let plan = FaultPlan {
        fail_write_index: Some(0),
        fail_flush_index: None,
        fail_hard: false,
    };
    let inner = FaultBackend::new(MemoryBackend::new(BS, 8), plan);
    let mut dev = ActivityBackend::new(inner, ActivityLog::default());
    assert!(matches!(
        dev.write_block(2, &block(4)),
        Err(BlockError::Injected(_))
    ));

    let (_, log) = dev.into_parts();
    assert_eq!(log.0.len(), 2);
    assert_eq!(log.0[0].phase, ActivityPhase::Begin);
    assert_eq!(log.0[1].phase, ActivityPhase::End { success: false });
}

#[test]
fn write_led_sink_can_filter_reads_before_emission() {
    let mut dev = ActivityBackend::new(MemoryBackend::new(BS, 8), WriteActivityLog::default());
    let mut buf = block(0);
    dev.read_block(1, &mut buf).unwrap();
    dev.write_block(1, &block(1)).unwrap();
    dev.flush().unwrap();

    let (_, log) = dev.into_parts();
    assert_eq!(log.0.len(), 4);
    assert!(log.0.iter().all(|event| matches!(
        event.operation,
        ActivityOperation::Write | ActivityOperation::Flush
    )));
}

#[test]
fn fault_backend_fails_exactly_the_planned_write() {
    let plan = FaultPlan { fail_write_index: Some(1), fail_flush_index: None, fail_hard: true };
    let mut dev = FaultBackend::new(MemoryBackend::new(BS, 8), plan);
    dev.write_block(1, &block(1)).unwrap();
    assert!(matches!(dev.write_block(2, &block(2)), Err(BlockError::Injected(_))));
    assert!(dev.tripped());
    // fail_hard: the device is gone afterwards.
    assert!(matches!(dev.flush(), Err(BlockError::Injected(_))));
}

#[test]
fn crash_states_respect_flush_barrier() {
    let base = MemoryBackend::new(BS, 8);
    let mut dev = RecordingBackend::new(base.clone());
    dev.write_block(1, &block(1)).unwrap();
    dev.flush().unwrap();
    dev.write_block(2, &block(2)).unwrap();
    dev.write_block(3, &block(3)).unwrap();
    let (_, log) = dev.into_parts();
    assert_eq!(log.len(), 4);

    // Crash at the very end: block 1 is durable in every state (it precedes
    // the completed flush); blocks 2 and 3 vary across subsets and tears.
    let states = crash_states(&base, &log, 4);
    // 2^2 subsets + tears (3 offsets × 2 writes) = 10 states.
    assert_eq!(states.len(), 10);
    let mut saw_2_without_3 = false;
    let mut saw_3_without_2 = false;
    for state in &states {
        assert_eq!(state.image.peek(1), block(1), "{}", state.description);
        let b2 = state.image.peek(2);
        let b3 = state.image.peek(3);
        if b2 == block(2) && b3 == block(0) {
            saw_2_without_3 = true;
        }
        if b3 == block(3) && b2 == block(0) {
            saw_3_without_2 = true;
        }
    }
    assert!(saw_2_without_3, "subset enumeration must cover in-order loss");
    assert!(saw_3_without_2, "subset enumeration must cover reordering");

    // Crash before anything: exactly the base image.
    let states = crash_states(&base, &log, 0);
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].image.peek(1), block(0));

    // A torn state must mix new and old bytes in one block.
    let states = crash_states(&base, &log, 4);
    let torn = states.iter().find(|s| s.description.contains("torn")).unwrap();
    let torn_block = [torn.image.peek(2), torn.image.peek(3)]
        .into_iter()
        .find(|b| *b != block(0) && *b != block(2) && *b != block(3));
    assert!(torn_block.is_some(), "expected a partially applied block");
}

#[test]
fn recorded_ops_capture_payloads() {
    let mut dev = RecordingBackend::new(MemoryBackend::new(BS, 8));
    dev.write_block(5, &block(9)).unwrap();
    let (_, log) = dev.into_parts();
    match &log[0] {
        RecordedOp::Write { lba, data } => {
            assert_eq!(*lba, 5);
            assert_eq!(*data, block(9));
        }
        RecordedOp::Flush => panic!("expected a write"),
    }
}
