//! Modern 64-bit operations of the AROS adapter: positioned I/O, clone,
//! preallocation and atomic replace.

use afsplus_aros::{ArosAdapter, ArosConfig, ArosError, LockAccess, OpenMode, SeekMode};
use afsplus_block::MemoryBackend;
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::Timespec;
use afsplus_vfs::Vfs;

fn timestamp(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted(shared_extents: bool) -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0xC5; 16],
            label: "ApiV2".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: timestamp(0),
        },
    )
    .unwrap();
    device
}

fn adapter(device: MemoryBackend) -> ArosAdapter<MemoryBackend> {
    ArosAdapter::new(
        Vfs::mount(device, MountOptions::default()).unwrap(),
        ArosConfig::default(),
    )
}

fn checked(adapter: ArosAdapter<MemoryBackend>) -> ArosAdapter<MemoryBackend> {
    let mut device = adapter.into_vfs().unwrap().into_volume().into_device();
    let report = check_device(&mut device);
    assert!(report.is_clean(), "{:?}", report.errors);
    self::adapter(device)
}

fn read_all(adapter: &mut ArosAdapter<MemoryBackend>, name: &[u8]) -> Vec<u8> {
    let file = adapter
        .open(None, name, OpenMode::OldFile, timestamp(900))
        .unwrap();
    let size = adapter.file_size(file).unwrap() as usize;
    let mut content = vec![0u8; size];
    assert_eq!(adapter.read_at(file, 0, &mut content).unwrap(), size);
    adapter.close(file).unwrap();
    content
}

#[test]
fn positioned_io_is_64_bit_and_leaves_the_dos_position_alone() {
    let mut adapter = adapter(formatted(true));
    let file = adapter
        .open(None, b"big", OpenMode::NewFile, timestamp(1))
        .unwrap();
    assert_eq!(adapter.write(file, b"hello", timestamp(2)).unwrap(), 5);
    // 4 GiB + 3: beyond every signed and unsigned 32-bit offset.
    assert_eq!(
        adapter
            .write_at(file, 0x1_0000_0003, b"far", timestamp(3))
            .unwrap(),
        3
    );
    assert_eq!(adapter.file_position(file).unwrap(), 5);
    assert_eq!(adapter.file_size(file).unwrap(), 0x1_0000_0006);

    let mut far = [0xEEu8; 8];
    assert_eq!(adapter.read_at(file, 0x1_0000_0001, &mut far).unwrap(), 5);
    assert_eq!(&far[..5], b"\0\0far");
    assert_eq!(adapter.file_position(file).unwrap(), 5);
    // The classic call continues from its own position: the hole after
    // "hello".
    let mut next = [0xEEu8; 2];
    assert_eq!(adapter.read(file, &mut next).unwrap(), 2);
    assert_eq!(next, [0, 0]);
    assert_eq!(adapter.file_position(file).unwrap(), 7);
    // The sparse 4 GiB file occupies two data blocks, never four gigabytes.
    assert_eq!(adapter.examine_file(file).unwrap().blocks, 2);
    assert_eq!(
        adapter.read_at(999, 0, &mut next),
        Err(ArosError::InvalidLock)
    );
    adapter.close(file).unwrap();
    checked(adapter);
}

#[test]
fn clone_file_shares_storage_and_diverges_on_write() {
    let mut adapter = adapter(formatted(true));
    let payload: Vec<u8> = (0..64 * 4096u32).map(|index| (index % 251) as u8).collect();
    let file = adapter
        .open(None, b"source", OpenMode::NewFile, timestamp(1))
        .unwrap();
    assert_eq!(
        adapter.write(file, &payload, timestamp(2)).unwrap(),
        payload.len()
    );
    adapter.close(file).unwrap();
    adapter.flush().unwrap();
    let used_before = adapter.disk_info().used_blocks;

    let source = adapter.locate(None, b"source", LockAccess::Shared).unwrap();
    adapter
        .clone_file(source, None, b"copy", timestamp(3))
        .unwrap();
    assert_eq!(
        adapter.clone_file(source, None, b"copy", timestamp(4)),
        Err(ArosError::ObjectExists)
    );
    adapter.free_lock(source).unwrap();
    adapter.flush().unwrap();
    // A byte copy needs 64 more data blocks; the clone shares them.
    let grown = adapter.disk_info().used_blocks - used_before;
    assert!(grown < 16, "clone consumed {grown} blocks");
    assert_eq!(read_all(&mut adapter, b"copy"), payload);

    // Writes to the clone stay private.
    let copy = adapter
        .open(None, b"copy", OpenMode::ReadWrite, timestamp(5))
        .unwrap();
    assert_eq!(
        adapter
            .write_at(copy, 4096, b"PRIVATE", timestamp(6))
            .unwrap(),
        7
    );
    adapter.close(copy).unwrap();
    let mut adapter = checked(adapter);
    assert_eq!(read_all(&mut adapter, b"source"), payload);
    let changed = read_all(&mut adapter, b"copy");
    assert_eq!(&changed[4096..4103], b"PRIVATE");
    assert_eq!(changed[..4096], payload[..4096]);
    assert_eq!(changed[4103..], payload[4103..]);

    // Control: a volume without shared extents answers
    // ERROR_ACTION_NOT_KNOWN and creates nothing, so the caller copies.
    let mut plain = self::adapter(formatted(false));
    let file = plain
        .open(None, b"source", OpenMode::NewFile, timestamp(1))
        .unwrap();
    plain.close(file).unwrap();
    let source = plain.locate(None, b"source", LockAccess::Shared).unwrap();
    assert_eq!(
        plain.clone_file(source, None, b"copy", timestamp(2)),
        Err(ArosError::ActionNotKnown)
    );
    assert_eq!(
        plain.locate(None, b"copy", LockAccess::Shared),
        Err(ArosError::ObjectNotFound)
    );
}

#[test]
fn preallocation_reserves_without_growing_and_is_bounded_per_request() {
    let vfs = Vfs::mount(formatted(true), MountOptions::default()).unwrap();
    let mut adapter = ArosAdapter::new(
        vfs,
        ArosConfig {
            max_preallocate_blocks: 32,
            ..ArosConfig::default()
        },
    );
    let file = adapter
        .open(None, b"reserved", OpenMode::NewFile, timestamp(1))
        .unwrap();
    // 100 bytes into block 1 through 100 bytes into block 16: 16 blocks.
    adapter
        .preallocate(file, 4096 + 100, 15 * 4096, timestamp(2))
        .unwrap();
    assert_eq!(adapter.file_size(file).unwrap(), 0);
    assert_eq!(adapter.examine_file(file).unwrap().blocks, 16);

    // One block over the per-request budget: refused, nothing reserved.
    assert_eq!(
        adapter.preallocate(file, 1 << 30, 33 * 4096, timestamp(3)),
        Err(ArosError::ObjectTooLarge)
    );
    assert_eq!(adapter.examine_file(file).unwrap().blocks, 16);
    assert_eq!(
        adapter.preallocate(file, 0, 0, timestamp(4)),
        Err(ArosError::InvalidComponentName)
    );
    assert_eq!(
        adapter.preallocate(file, u64::MAX, 2, timestamp(4)),
        Err(ArosError::InvalidComponentName)
    );

    // Reserved space reads as zeros once the size covers it, and a write
    // into it allocates nothing further.
    adapter
        .set_file_size(file, 8 * 4096, SeekMode::Beginning, timestamp(5))
        .unwrap();
    let mut zeros = [0xEEu8; 64];
    assert_eq!(adapter.read_at(file, 4096, &mut zeros).unwrap(), 64);
    assert_eq!(zeros, [0u8; 64]);
    assert_eq!(
        adapter.write_at(file, 4096, b"data", timestamp(6)).unwrap(),
        4
    );
    adapter.fsync(file).unwrap();
    assert_eq!(adapter.examine_file(file).unwrap().blocks, 16);
    adapter.close(file).unwrap();

    let reader = adapter
        .open(None, b"reserved", OpenMode::OldFile, timestamp(7))
        .unwrap();
    assert_eq!(
        adapter.preallocate(reader, 0, 4096, timestamp(8)),
        Err(ArosError::DiskWriteProtected)
    );
    adapter.close(reader).unwrap();
    checked(adapter);
}

#[test]
fn replace_is_atomic_rename_over_an_unheld_target() {
    let mut adapter = adapter(formatted(true));
    for (name, content) in [(&b"config"[..], &b"old"[..]), (b"config.tmp", b"new!")] {
        let file = adapter
            .open(None, name, OpenMode::NewFile, timestamp(1))
            .unwrap();
        adapter.write(file, content, timestamp(2)).unwrap();
        adapter.close(file).unwrap();
    }

    // The classic rename refuses an existing target...
    assert_eq!(
        adapter.rename(None, b"config.tmp", None, b"config", timestamp(3)),
        Err(ArosError::ObjectExists)
    );
    // ...and replace refuses a held one without touching it.
    let held = adapter.locate(None, b"config", LockAccess::Shared).unwrap();
    assert_eq!(
        adapter.replace(None, b"config.tmp", None, b"config", timestamp(4)),
        Err(ArosError::ObjectInUse)
    );
    adapter.free_lock(held).unwrap();
    assert_eq!(read_all(&mut adapter, b"config"), b"old");

    adapter
        .replace(None, b"config.tmp", None, b"config", timestamp(5))
        .unwrap();
    let mut adapter = checked(adapter);
    assert_eq!(read_all(&mut adapter, b"config"), b"new!");
    assert_eq!(
        adapter.locate(None, b"config.tmp", LockAccess::Shared),
        Err(ArosError::ObjectNotFound)
    );
    assert_eq!(
        adapter.replace(None, b"absent", None, b"config", timestamp(6)),
        Err(ArosError::ObjectNotFound)
    );
}
