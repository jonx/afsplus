//! The read cache changes what the device is asked to read and nothing else:
//! one workload through the whole stack at several cache sizes sends the
//! device the same writes and barriers in the same order and leaves the same
//! image (docs/20-performance.md, "Cache independence").

use afsplus_block::{BlockDevice, CachedDevice, MemoryBackend, TraceBackend, TraceEvent};
use afsplus_check::check_device;
use afsplus_core::{mkfs, MkfsParams, MountOptions};
use afsplus_format::{Timespec, OBJECT_ROOT};
use afsplus_vfs::{AccessMode, Vfs};

fn at(seconds: i64) -> Timespec {
    Timespec {
        seconds,
        nanoseconds: 0,
    }
}

fn formatted() -> MemoryBackend {
    let mut device = MemoryBackend::new(4096, 8192);
    mkfs(
        &mut device,
        &MkfsParams {
            uuid: [0x7C; 16],
            label: "ReadCache".into(),
            region_size: 4096,
            reclaim_caps: Default::default(),
            log_slots: 8,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Insensitive,
            timestamp: at(0),
        },
    )
    .unwrap();
    device
}

type Stack = CachedDevice<TraceBackend<MemoryBackend>>;

/// Creates drawers of files, writes, renames, deletes, syncs; returns the
/// device-level writes and barriers, the reads, and the final image.
fn run(capacity: usize) -> (Vec<TraceEvent>, u64, Vec<u8>) {
    let device = CachedDevice::new(TraceBackend::new(formatted()), capacity);
    let mut vfs: Vfs<Stack> = Vfs::mount(device, MountOptions::default()).unwrap();
    let mut clock = 1;
    let mut tick = || {
        clock += 1;
        at(clock)
    };
    for drawer in 0..4 {
        let dir = vfs
            .create_directory(OBJECT_ROOT, &format!("d{drawer}"), tick())
            .unwrap();
        for file in 0..24 {
            let name = format!("f{file}");
            let id = vfs.create_file(dir, &name, tick()).unwrap();
            let handle = vfs.open_file(id, AccessMode::WriteOnly).unwrap();
            let bytes = vec![(drawer * 31 + file) as u8; 700 * (file + 1)];
            vfs.write(handle, 0, &bytes, tick()).unwrap();
            if file % 5 == 0 {
                vfs.fsync(handle).unwrap();
            }
            vfs.close(handle).unwrap();
        }
        for file in (0..24).step_by(3) {
            vfs.rename(
                dir,
                &format!("f{file}"),
                dir,
                &format!("r{file}"),
                false,
                tick(),
            )
            .unwrap();
        }
        for file in (1..24).step_by(3) {
            vfs.unlink_file(dir, &format!("f{file}"), tick()).unwrap();
        }
    }
    vfs.sync_filesystem().unwrap();
    let mut cached = vfs.into_volume().into_device();
    let reads = cached.inner().stats().reads;
    let order: Vec<TraceEvent> = cached
        .inner()
        .events()
        .iter()
        .copied()
        .filter(|event| !matches!(event, TraceEvent::Read { .. }))
        .collect();
    let mut image = Vec::new();
    let mut block = vec![0u8; 4096];
    let device = cached.inner_mut();
    for lba in 0..device.total_blocks() {
        device.read_block(lba, &mut block).unwrap();
        image.extend_from_slice(&block);
    }
    let mut memory = cached.into_inner().into_inner();
    assert!(check_device(&mut memory).is_clean());
    (order, reads, image)
}

#[test]
fn the_cache_size_changes_the_reads_and_nothing_the_device_keeps() {
    let (writes_none, reads_none, image_none) = run(0);
    let (writes_small, reads_small, image_small) = run(16);
    let (writes_large, reads_large, image_large) = run(100_000);
    assert_eq!(
        writes_small, writes_none,
        "writes and barriers, small cache"
    );
    assert_eq!(
        writes_large, writes_none,
        "writes and barriers, large cache"
    );
    assert!(image_small == image_none, "image, small cache");
    assert!(image_large == image_none, "image, large cache");
    assert!(
        reads_small < reads_none && reads_large < reads_small,
        "reads {reads_none} uncached, {reads_small} small, {reads_large} large"
    );
    eprintln!("device reads: {reads_none} uncached, {reads_small} with 16 blocks, {reads_large} unbounded");
}
