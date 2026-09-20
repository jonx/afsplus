//! Priority-0 hardening behaviors: ambiguous same-generation checkpoints,
//! no silent fallback over corrupt reachable state, comparison-key
//! consistency, and counter-overflow handling.

use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{allocation_root, mkfs, mount, object_map, CoreError, MkfsParams};
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::ident::Identification;
use afsplus_format::region::RegionDescriptor;
use afsplus_format::tree::TreeNode;
use afsplus_format::Timespec;

const BS: usize = 4096;

fn formatted() -> MemoryBackend {
    let mut dev = MemoryBackend::new(BS, 64);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [42u8; 16],
            label: "HardVol".into(),
            region_size: 64,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: Timespec {
                seconds: 1_780_000_000,
                nanoseconds: 0,
            },
        },
    )
    .unwrap();
    dev
}

fn ts(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn read_ident(dev: &MemoryBackend) -> Identification {
    Identification::decode(&dev.peek(0)).unwrap()
}

#[test]
fn allocator_descriptor_and_page_corruption_are_deferred_but_never_accepted() {
    let mut base = formatted();
    let ident = read_ident(&base);
    let checkpoint = Checkpoint::decode(&base.peek(1), &ident.uuid).unwrap();
    let geo = ident.geometry();
    let record = allocation_root::lookup_record(
        &mut base,
        &geo,
        checkpoint.allocation_root_block,
        checkpoint.generation,
        0,
    )
    .unwrap()
    .unwrap();
    let descriptor_lba = geo.descriptor_slot_lba(0, record.descriptor_slot);
    let (descriptor, _) = RegionDescriptor::decode(&base.peek(descriptor_lba)).unwrap();
    let bitmap_lba = geo.bitmap_slot_lba(0, 0, descriptor.pages[0].slot);

    for (kind, lba) in [("descriptor", descriptor_lba), ("bitmap", bitmap_lba)] {
        let mut dev = base.clone();
        let mut block = dev.peek(lba);
        block[100] ^= 0x80;
        dev.apply_raw(lba, &block);

        // Mount reads namespace roots only, so unrelated allocator damage is
        // discovered by the exhaustive checker or when allocation begins.
        let report = check_device(&mut dev);
        assert!(!report.is_clean(), "{kind} corruption escaped the checker");
        let mut vol = mount(dev).expect("bounded mount must not scan allocator pages");
        assert!(matches!(
            vol.create_file_in_root("probe", b"", ts(1)),
            Err(CoreError::Corrupt(_))
        ));
        assert_eq!(
            vol.generation(),
            1,
            "failed mutation changed committed state"
        );
    }
}

#[test]
fn same_generation_checkpoints_are_ambiguous_for_mount_and_checker() {
    let mut dev = formatted();
    // Duplicate slot A into slot B: two structurally valid checkpoints with
    // the same generation — a state no correct commit sequence can produce.
    let slot_a = dev.peek(1);
    dev.apply_raw(2, &slot_a);

    let report = check_device(&mut dev);
    assert!(!report.is_clean());
    assert!(
        report.errors.iter().any(|e| e.contains("ambiguous")),
        "checker must flag ambiguity: {:?}",
        report.errors
    );
    match mount(dev) {
        Err(CoreError::AmbiguousCheckpoints(1)) => {}
        Err(e) => panic!("mount must report ambiguity, got: {e}"),
        Ok(_) => panic!("mount must refuse the ambiguous volume"),
    }
}

#[test]
fn corrupt_state_under_the_chosen_checkpoint_is_an_error_not_a_fallback() {
    // The newest checkpoint is structurally valid but its object map is
    // corrupt. The old pre-hardening behavior silently fell back to
    // generation 1, masking the violation; mount must now report corruption
    // and the checker must flag it as an error.
    let mut dev = formatted();
    let mut vol = mount(dev).unwrap();
    vol.create_file_in_root("hello.txt", b"x", ts(1)).unwrap();
    dev = vol.into_device();

    let ident = read_ident(&dev);
    let newest = Checkpoint::decode(&dev.peek(2), &ident.uuid).expect("generation 2 in slot B");
    assert_eq!(newest.generation, 2);
    let mut omap_block = dev.peek(newest.object_map_block);
    omap_block[100] ^= 0xFF;
    dev.apply_raw(newest.object_map_block, &omap_block);

    let report = check_device(&mut dev);
    assert!(!report.is_clean(), "checker must not accept the volume");
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("references invalid state")),
        "unexpected findings: {:?}",
        report.errors
    );
    match mount(dev) {
        Err(CoreError::Corrupt(message)) => {
            assert!(
                message.contains("generation 2"),
                "unhelpful diagnostics: {message}"
            );
        }
        Err(e) => panic!("mount must report corruption, got: {e}"),
        Ok(vol) => panic!(
            "mount must report corruption, not fall back to generation {}",
            vol.generation()
        ),
    }
}

#[test]
fn corrupt_descendant_is_reported_on_access_without_a_mount_scan() {
    let mut dev = formatted();
    let mut vol = mount(dev).unwrap();
    let id = vol.create_file_in_root("hello.txt", b"x", ts(1)).unwrap();
    dev = vol.into_device();

    let ident = read_ident(&dev);
    let newest = Checkpoint::decode(&dev.peek(2), &ident.uuid).unwrap();
    let record_lba = object_map::lookup_lba(
        &mut dev,
        &ident.geometry(),
        newest.object_map_block,
        newest.generation,
        id,
    )
    .unwrap()
    .unwrap();
    let mut record = dev.peek(record_lba);
    record[100] ^= 0xFF;
    dev.apply_raw(record_lba, &record);

    // Roots remain valid, so bounded mount succeeds and namespace lookup is
    // available. The first access to the damaged descendant reports it.
    let mut vol = mount(dev.clone()).unwrap();
    assert_eq!(vol.lookup_root("hello.txt").unwrap(), Some(id));
    assert!(matches!(vol.stat(id), Err(CoreError::Corrupt(_))));

    let report = check_device(&mut dev);
    assert!(
        !report.is_clean(),
        "full checker must find the damaged descendant"
    );
}

#[test]
fn stored_comparison_key_must_match_the_name() {
    let mut dev = formatted();
    let mut vol = mount(dev).unwrap();
    let _id = vol.create_file_in_root("aaa.txt", b"", ts(1)).unwrap();
    let dir_lba = vol
        .stat(afsplus_format::OBJECT_ROOT)
        .unwrap()
        .unwrap()
        .data_root;
    dev = vol.into_device();

    // Keep the AFST node structurally valid but change the stored key without
    // changing the original name inside its typed value.
    let (mut forged, generation) = TreeNode::decode(&dev.peek(dir_lba)).unwrap();
    assert_eq!(forged.items.len(), 1);
    forged.items[0].key = b"zzz-not-the-name".into();
    dev.apply_raw(dir_lba, &forged.encode(BS, generation).unwrap());

    // Bounded mount validates only the root structure. The first lookup that
    // reaches the forged typed record reports the semantic mismatch.
    let mut vol = mount(dev.clone()).unwrap();
    assert!(matches!(
        vol.lookup_root("zzz-not-the-name"),
        Err(CoreError::Corrupt(message)) if message.contains("key")
    ));
    let report = check_device(&mut dev);
    assert!(!report.is_clean());
}

#[test]
fn generation_counter_overflow_is_reported_not_wrapped() {
    let mut dev = formatted();
    let ident = read_ident(&dev);
    // Rewrite checkpoint A at generation u64::MAX (bitmap records keep their
    // real generation, which stays ≤ the checkpoint's).
    let mut ckpt = Checkpoint::decode(&dev.peek(1), &ident.uuid).unwrap();
    ckpt.generation = u64::MAX;
    ckpt.committed_tx_id = u64::MAX;
    dev.apply_raw(1, &ckpt.encode(BS).unwrap());

    let mut vol = mount(dev).unwrap();
    assert_eq!(vol.generation(), u64::MAX);
    match vol.create_file_in_root("overflow.txt", b"", ts(1)) {
        Err(CoreError::PrototypeLimit(message)) => {
            assert!(message.contains("generation"), "{message}");
        }
        other => panic!("expected a reported overflow, got {other:?}"),
    }
    assert_eq!(
        vol.generation(),
        u64::MAX,
        "failed commit must not change state"
    );
}

#[test]
fn object_id_counter_overflow_is_reported_not_wrapped() {
    let mut dev = formatted();
    let ident = read_ident(&dev);
    let mut ckpt = Checkpoint::decode(&dev.peek(1), &ident.uuid).unwrap();
    ckpt.next_object_id = u64::MAX;
    dev.apply_raw(1, &ckpt.encode(BS).unwrap());

    let mut vol = mount(dev).unwrap();
    match vol.create_file_in_root("overflow.txt", b"", ts(1)) {
        Err(CoreError::PrototypeLimit(message)) => {
            assert!(message.contains("object ID"), "{message}");
        }
        other => panic!("expected a reported overflow, got {other:?}"),
    }
}
