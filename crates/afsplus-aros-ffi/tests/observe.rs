//! Notification, health and trace-sink entry points through the C boundary.

mod common;

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::slice;

use afsplus_aros_ffi::*;
use afsplus_block::{BlockDevice, MemoryBackend};
use common::{formatted, mount};

fn mkdir(filesystem: *mut AfsplusAros, name: &[u8]) -> i32 {
    let mut lock = 0;
    let status = afsplus_aros_create_directory(
        filesystem,
        0,
        name.as_ptr(),
        name.len() as u32,
        1,
        0,
        &mut lock,
    );
    if status == 0 {
        assert_eq!(afsplus_aros_free_lock(filesystem, lock), 0);
    }
    status
}

#[test]
fn watches_cross_the_boundary_as_coalesced_identifiers() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    let (mut first, mut second) = (0, 0);
    assert_eq!(
        afsplus_aros_watch_add(filesystem, 0, b"Drawer".as_ptr(), 6, &mut first),
        0
    );
    assert_eq!(
        afsplus_aros_watch_add(filesystem, 0, b"other".as_ptr(), 5, &mut second),
        0
    );
    assert_eq!((first, second), (1, 2));

    let mut ids = [0u64; 4];
    let mut count = 9;
    assert_eq!(
        afsplus_aros_watch_drain(filesystem, ids.as_mut_ptr(), 4, &mut count),
        0
    );
    assert_eq!(count, 0);
    assert_eq!(mkdir(filesystem, b"drawer"), 0);
    assert_eq!(
        afsplus_aros_watch_drain(filesystem, ids.as_mut_ptr(), 4, &mut count),
        0
    );
    assert_eq!((count, ids[0]), (1, 1));
    assert_eq!(afsplus_aros_watch_remove(filesystem, 1), 0);
    assert_eq!(afsplus_aros_watch_remove(filesystem, 1), 205);
    assert_eq!(
        afsplus_aros_watch_drain(filesystem, ptr::null_mut(), 4, &mut count),
        210
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

struct Collected {
    events: Vec<AfspTraceEvent>,
}

unsafe extern "C" fn collect(context: *mut c_void, event: *const AfspTraceEvent) {
    // SAFETY: the test keeps the collector alive while the sink is attached.
    let collected = unsafe { &mut *context.cast::<Collected>() };
    // SAFETY: the bridge passes one valid event for the duration of the call.
    collected.events.push(unsafe { *event });
}

fn counters(filesystem: *mut AfsplusAros) -> AfsplusArosTraceCounters {
    let mut output = AfsplusArosTraceCounters {
        struct_size: size_of::<AfsplusArosTraceCounters>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_trace_counters(filesystem, &mut output), 0);
    output
}

#[test]
fn trace_sink_streams_selected_categories_until_detached() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    assert_eq!(counters(filesystem).attached, 0);

    let mut collected = Collected { events: Vec::new() };
    let sink = AfspTraceSink {
        emit: Some(collect),
        ctx: ptr::from_mut(&mut collected).cast::<c_void>(),
        category_mask: AFSP_TRACE_API | AFSP_TRACE_TX,
    };
    assert_eq!(afsplus_aros_set_trace_sink(filesystem, &sink), 0);
    assert_eq!(mkdir(filesystem, b"traced"), 0);

    assert!(!collected.events.is_empty());
    // The stream opens with the API entry (AFSP_EVENT_API_BEGIN, 8) and only
    // carries the two selected categories, in strictly increasing sequence.
    assert_eq!(
        (collected.events[0].category, collected.events[0].event),
        (0x2000, 8)
    );
    assert!(collected
        .events
        .iter()
        .all(|event| event.category == 0x2000 || event.category == 0x0001));
    assert!(collected
        .events
        .iter()
        .any(|event| event.category == 0x0001));
    assert!(collected
        .events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    let seen = counters(filesystem);
    assert_eq!(seen.attached, 1);
    assert_eq!(seen.delivered, collected.events.len() as u64);
    assert_eq!(seen.missed, 0);

    // Control: a mask that selects only errors delivers nothing for the same
    // successful operation and counts what it filtered.
    let delivered_before = collected.events.len();
    let errors_only = AfspTraceSink {
        emit: Some(collect),
        ctx: ptr::from_mut(&mut collected).cast::<c_void>(),
        category_mask: AFSP_TRACE_ERROR,
    };
    assert_eq!(afsplus_aros_set_trace_sink(filesystem, &errors_only), 0);
    assert_eq!(mkdir(filesystem, b"filtered"), 0);
    assert_eq!(collected.events.len(), delivered_before);
    let filtered = counters(filesystem);
    assert_eq!(filtered.delivered, 0);
    assert!(filtered.filtered > 0);

    // Detached: no recorder, no callbacks.
    assert_eq!(afsplus_aros_set_trace_sink(filesystem, ptr::null()), 0);
    assert_eq!(mkdir(filesystem, b"silent"), 0);
    assert_eq!(collected.events.len(), delivered_before);
    assert_eq!(counters(filesystem).attached, 0);

    let missing_emit = AfspTraceSink {
        emit: None,
        ctx: ptr::null_mut(),
        category_mask: AFSP_TRACE_API,
    };
    assert_eq!(afsplus_aros_set_trace_sink(filesystem, &missing_emit), 210);
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

struct Faulty {
    device: MemoryBackend,
    fail_writes: bool,
}

unsafe extern "C" fn faulty_read(
    context: *mut c_void,
    lba: u64,
    destination: *mut u8,
    length: u32,
) -> i32 {
    // SAFETY: the test keeps the context and destination alive for the call.
    let faulty = unsafe { &mut *context.cast::<Faulty>() };
    // SAFETY: the bridge supplies exactly one writable block.
    let destination = unsafe { slice::from_raw_parts_mut(destination, length as usize) };
    faulty.device.read_block(lba, destination).map_or(5, |()| 0)
}

unsafe extern "C" fn faulty_write(
    context: *mut c_void,
    lba: u64,
    source: *const u8,
    length: u32,
) -> i32 {
    // SAFETY: the test keeps the context and source alive for the call.
    let faulty = unsafe { &mut *context.cast::<Faulty>() };
    if faulty.fail_writes {
        return 5;
    }
    // SAFETY: the bridge supplies exactly one readable block.
    let source = unsafe { slice::from_raw_parts(source, length as usize) };
    faulty.device.write_block(lba, source).map_or(5, |()| 0)
}

unsafe extern "C" fn faulty_flush(context: *mut c_void) -> i32 {
    // SAFETY: the test keeps the context alive for the entire mount.
    let faulty = unsafe { &mut *context.cast::<Faulty>() };
    faulty.device.flush().map_or(5, |()| 0)
}

fn health(filesystem: *mut AfsplusAros) -> AfsplusArosHealth {
    let mut output = AfsplusArosHealth {
        struct_size: size_of::<AfsplusArosHealth>() as u32,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_health(filesystem, &mut output), 0);
    output
}

#[test]
fn a_failing_device_becomes_a_queryable_health_event() {
    let mut faulty = Faulty {
        device: formatted(true),
        fail_writes: false,
    };
    let callbacks = AfsplusArosDevice {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosDevice>() as u32,
        context: ptr::from_mut(&mut faulty).cast::<c_void>(),
        block_size: 4096,
        reserved: 0,
        total_blocks: 8192,
        read_block: Some(faulty_read),
        write_block: Some(faulty_write),
        flush: Some(faulty_flush),
    };
    let config = AfsplusArosMountConfig {
        abi_version: AFSPLUS_AROS_ABI_VERSION,
        struct_size: size_of::<AfsplusArosMountConfig>() as u32,
        mount_mode: AFSPLUS_AROS_MOUNT_READ_WRITE,
        name_encoding: AFSPLUS_AROS_ENCODING_UTF8,
        volume_name: b"AFS+".as_ptr(),
        volume_name_length: 4,
        max_file_handles: 8,
        max_locks: 8,
        max_file_info_name_bytes: 107,
        flags: 0,
    };
    let mut filesystem = ptr::null_mut();
    assert_eq!(afsplus_aros_mount(&callbacks, &config, &mut filesystem), 0);

    let clean = health(filesystem);
    // The current layout, which grew past the 112 bytes first published.
    assert_eq!(clean.struct_size, 136);
    assert_eq!(
        (clean.flags, clean.generation, clean.events_recorded),
        (0, 1, 0)
    );
    assert_eq!(clean.total_blocks, 8192);
    // An ordinary failure is a result, not a health event.
    let mut lock = 0;
    assert_eq!(
        afsplus_aros_locate(
            filesystem,
            0,
            b"absent".as_ptr(),
            6,
            AFSPLUS_AROS_LOCK_SHARED,
            &mut lock
        ),
        205
    );
    assert_eq!(health(filesystem).events_recorded, 0);

    faulty.fail_writes = true;
    assert_eq!(mkdir(filesystem, b"doomed"), 100);
    let degraded = health(filesystem);
    assert_eq!(degraded.flags & AFSPLUS_AROS_HEALTH_DEVICE_ERROR, 1);
    assert_eq!(degraded.device_errors, 1);
    assert_eq!(degraded.last_error, 100);
    let mut events = [AfsplusArosHealthEvent::default(); 4];
    let mut count = 0;
    assert_eq!(
        afsplus_aros_health_events(filesystem, events.as_mut_ptr(), 4, &mut count),
        0
    );
    assert_eq!(count, 1);
    assert_eq!(
        events[0],
        AfsplusArosHealthEvent {
            sequence: 1,
            kind: 1,
            dos_error: 100,
        }
    );
    // Draining empties the ring and keeps the counters.
    assert_eq!(
        afsplus_aros_health_events(filesystem, events.as_mut_ptr(), 4, &mut count),
        0
    );
    assert_eq!(count, 0);
    assert_eq!(health(filesystem).device_errors, 1);

    // A client of the first layout gets the first layout: the counters
    // appended after it are not written past the size it declared.
    let mut old = AfsplusArosHealth {
        struct_size: AFSPLUS_AROS_HEALTH_FIRST_LAYOUT as u32,
        checkpoint_fallbacks: 0x5a5a,
        reclaim_backlog_highs: 0x5a5a,
        free_count_mismatches: 0x5a5a,
        ..Default::default()
    };
    assert_eq!(afsplus_aros_health(filesystem, &mut old), 0);
    assert_eq!(old.struct_size, 112);
    assert_eq!(old.device_errors, 1);
    assert_eq!(
        (
            old.checkpoint_fallbacks,
            old.reclaim_backlog_highs,
            old.free_count_mismatches
        ),
        (0x5a5a, 0x5a5a, 0x5a5a)
    );

    faulty.fail_writes = false;
    let _ = afsplus_aros_unmount(filesystem);
}

#[test]
fn info_document_crosses_the_boundary_with_the_short_buffer_rule() {
    let mut device = formatted(true);
    let filesystem = mount(&mut device);
    let mut required = 0;
    let mut small = [0xEEu8; 16];
    assert_eq!(
        afsplus_aros_info_json(filesystem, small.as_mut_ptr(), 16, &mut required),
        0
    );
    assert!(required > 16);
    assert_eq!(small, [0xEE; 16]);

    let mut buffer = vec![0xEEu8; required as usize + 4];
    let mut written = 0;
    assert_eq!(
        afsplus_aros_info_json(
            filesystem,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut written
        ),
        0
    );
    assert_eq!(written, required);
    let document = std::str::from_utf8(&buffer[..written as usize]).unwrap();
    assert!(document.starts_with(
        "{\"schema\":\"afsplus-handler-info\",\"schema_version\":1,\"volume\":{\"uuid\":\"c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6c6\",\"label\":\"FfiCommon\","
    ));
    assert!(document.ends_with("\"handles\":{\"locks\":0,\"files\":0,\"watches\":0}}"));
    assert_eq!(buffer[written as usize..], [0xEE; 4]);
    assert_eq!(
        afsplus_aros_info_json(filesystem, buffer.as_mut_ptr(), 4, ptr::null_mut()),
        210
    );
    assert_eq!(afsplus_aros_unmount(filesystem), 0);
}

/// The published codes of `api/debug_observability.h` are the core's: every
/// kind has one under its own name, each code once, and nothing else.
#[test]
fn trace_event_codes_are_the_ones_the_header_publishes() {
    use afsplus_core::flight::EventKind;
    let header = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../api/debug_observability.h"
    ))
    .unwrap();
    let published: Vec<(String, u16)> = header
        .lines()
        .filter_map(|line| {
            let (name, code) = line.trim().strip_prefix("AFSP_EVENT_")?.split_once(" = ")?;
            Some((name.to_owned(), code.trim_end_matches(',').parse().ok()?))
        })
        .collect();
    let screaming = |kind: EventKind| {
        let mut name = String::new();
        for (index, character) in format!("{kind:?}").chars().enumerate() {
            if character.is_ascii_uppercase() && index > 0 {
                name.push('_');
            }
            name.push(character.to_ascii_uppercase());
        }
        name
    };
    assert_eq!(published.len(), EventKind::ALL.len());
    for kind in EventKind::ALL {
        assert!(
            published.contains(&(screaming(kind), kind.code())),
            "{kind:?} is not published as code {}",
            kind.code()
        );
        assert_eq!(EventKind::from_code(kind.code()), Some(kind));
    }
    let mut codes: Vec<u16> = published.iter().map(|(_, code)| *code).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), published.len(), "a code is published twice");
    assert!(!codes.contains(&0), "zero means no event");
    assert_eq!(EventKind::from_code(0), None);
}
