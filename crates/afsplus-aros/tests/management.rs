//! The handler's versioned info document (ADR-025).

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, LockAccess, OpenMode};
use afsplus_block::MemoryBackend;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

#[test]
fn info_document_is_versioned_exact_and_follows_the_live_state() {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ],
            label: "Work \"A\"".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: timestamp(0),
        },
    )
    .unwrap();
    let mut adapter = ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig::default(),
    );
    let policy = adapter.volume_policy();
    let expected = format!(
        concat!(
            "{{\"schema\":\"afsplus-handler-info\",\"schema_version\":1,",
            "\"volume\":{{\"uuid\":\"00112233445566778899aabbccddeeff\",",
            "\"label\":\"Work \\\"A\\\"\",\"block_size\":4096,\"total_blocks\":8192,",
            "\"free_blocks\":{free},\"available_blocks\":{available},",
            "\"max_name_bytes\":255,\"case_sensitive\":false,",
            "\"unicode_version\":\"16.0.0\",",
            "\"features\":{{\"compat\":0,\"ro_compat\":3,\"incompat\":3}}}},",
            "\"mount\":{{\"mode\":\"read-write\",\"generation\":1,",
            "\"pending_intent_records\":0,\"pending_orphans\":0}},",
            "\"capabilities\":[\"io_64bit\",\"utf8_names\",\"hard_links\",",
            "\"atomic_replace\",\"object_ids\",\"paged_directories\",\"sparse_files\",",
            "\"fsync\",\"clone_file\",\"clone_range\",\"logged_data_fsync\",",
            "\"open_unlinked\",\"symlinks\",\"preallocate\",\"extended_attributes\"],",
            "\"health\":{{\"flags\":[],\"device_errors\":0,\"corruption_errors\":0,",
            "\"no_space_errors\":0,\"internal_faults\":0,\"events_recorded\":0,",
            "\"events_dropped\":0,\"last_error\":0}},",
            "\"handles\":{{\"locks\":0,\"files\":0,\"watches\":0}}}}"
        ),
        free = policy.statfs.free_blocks,
        available = policy.statfs.available_blocks,
    );
    assert_eq!(adapter.info_json().unwrap(), expected);

    // The document follows the live state: handles, health and generation.
    let lock = adapter.locate(None, b"", LockAccess::Shared).unwrap();
    let file = adapter
        .open(None, b"note", OpenMode::NewFile, timestamp(1))
        .unwrap();
    adapter.add_watch(None, b"note").unwrap();
    adapter.health_log().record(ArosError::NotDosDisk);
    let live = adapter.info_json().unwrap();
    assert_ne!(live, expected);
    assert!(live.contains("\"handles\":{\"locks\":1,\"files\":1,\"watches\":1}"));
    assert!(live.contains(
        "\"health\":{\"flags\":[\"corruption\"],\"device_errors\":0,\"corruption_errors\":1,"
    ));
    assert!(live.contains("\"last_error\":225}"));
    assert!(live.contains("\"generation\":2,"));
    adapter.close(file).unwrap();
    adapter.free_lock(lock).unwrap();
}
